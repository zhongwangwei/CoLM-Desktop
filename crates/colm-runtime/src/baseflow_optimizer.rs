//! `DEF_Optimize_Baseflow`：预热期逐年反解 `scale_baseflow`（`MOD_Opt_Baseflow.F90`）。
//!
//! 上游在每一步 `CoLMDRIVER`、`hist_out`、写续跑**之后**调 `ParameterOptimization`
//! （`CoLM.F90:709`），只在 `is_spinup` 时动手：
//!
//! * 每步把 `recharge = forc_prc + forc_prl - fevpa - rsur` 与 `rsub` 乘 `deltim`
//!   累加进年总量（`add_spv`：累加器初值是 `spval`，第一笔直接赋值、之后才相加）；
//! * 跨年那一步（`isendofyear(idate, deltim)`）先把本年的 `zwt`/`zwt_init`/
//!   `scale_baseflow`/两个年总量写进 `ParaOpt/cNNNN/<case>_baseflow.nc`，
//!   再按"地下水位变深而补给不及基流、或变浅而补给多于基流"两种情形把
//!   `scale_baseflow` 乘上 `rchg/rsub`，下限 `1e-8`，改完写回
//!   `ParaOpt/<case>_baseflow.nc` 并把两个累加器复位成 `spval`。
//!
//! 新的 `scale_baseflow` 当步就留在内存里，**下一步**的 `rsubst` 就乘它
//! （`MOD_SoilSnowHydrology.F90:1041`）。所以这不是事后的离线标定，而是会改写
//! 预热轨迹的一环，必须在主循环里做。

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

/// 上游的 `spval`：累加器"还没有值"的标记，也是写进文件的值。
const SPVAL: f64 = -1.0e36;

/// 一个 patch 的优化器状态（上游各向量在 `ipatch` 处的元素）。
#[derive(Debug, Clone, PartialEq)]
struct PatchBaseflow {
    /// `scale_baseflow(ipatch)`
    scale: f64,
    /// `zwt_init(ipatch)`
    initial_water_table_depth_m: f64,
    /// `rchg_year(ipatch)`：`None` 即上游的 `spval`。
    recharge_mm: Option<f64>,
    /// `rsub_year(ipatch)`，同上。
    subsurface_runoff_mm: Option<f64>,
    /// `patchtype <= 1`：只有土壤与城市 patch 会被改写 `scale_baseflow`。
    adjustable: bool,
}

/// 整个站点的优化器（`scale_baseflow`/`zwt_init`/`rchg_year`/`rsub_year` 四个长度 `numpatch`
/// 的向量加一个 `iter_bf_opt`）。上游逐 patch 独立累加、独立更新，跨年时整向量写文件；
/// 单点的多 patch 就是多作物站点，向量顺序即 patch 下标。
#[derive(Debug, Clone, PartialEq)]
pub struct BaseflowOptimizer {
    patches: Vec<PatchBaseflow>,
    iteration: u32,
    /// `DEF_dir_restart/ParaOpt`。
    directory: PathBuf,
    /// `<case>_baseflow_<block>.nc`：`ncio_create_file_vector` 与读取同一套块后缀（单点是 `w180_s90`）。
    file_name: String,
}

/// `Opt_Baseflow_init` 对一个 patch 的输入。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseflowPatchInit {
    /// 刚从 `ParaOpt/<case>_baseflow.nc` 读到的 `scale_baseflow`（缺省 1）。
    pub scale: f64,
    /// 启动时刻的 `zwt`。
    pub water_table_depth_m: f64,
    pub patch_type: i32,
}

/// 一步里优化器要的量，全部取**步末**的值（与上游在 `CoLMDRIVER` 之后读全局数组一致）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BaseflowStep {
    pub convective_precipitation_kg_m2_s: f64,
    pub large_scale_precipitation_kg_m2_s: f64,
    pub total_evaporation_kg_m2_s: f64,
    pub surface_runoff_mm_s: f64,
    pub subsurface_runoff_mm_s: f64,
    pub time_step_seconds: f64,
}

impl BaseflowOptimizer {
    /// `Opt_Baseflow_init`，`patches` 按 patch 下标排列；`block` 是块后缀（单点 `w180_s90`）。
    pub fn new(
        patches: &[BaseflowPatchInit],
        directory: impl Into<PathBuf>,
        case_name: &str,
        block: &str,
    ) -> Self {
        Self {
            patches: patches
                .iter()
                .map(|patch| PatchBaseflow {
                    scale: patch.scale,
                    initial_water_table_depth_m: patch.water_table_depth_m,
                    recharge_mm: None,
                    subsurface_runoff_mm: None,
                    adjustable: patch.patch_type <= 1,
                })
                .collect(),
            iteration: 0,
            directory: directory.into(),
            file_name: format!("{case_name}_baseflow_{block}.nc"),
        }
    }

    /// LULCC 换年（`REST_LulccTimeVariables` 与 `LulccDriver`，upstream-bugs 第 43 条 vendor 已修）：
    /// `previous[np]` 是新 patch 配上的旧 patch。配上的沿用四个量（年末结算刚复位过两个累加器），
    /// 新出现的按 `Opt_Baseflow_init` 的缺省：`scale = 1`、`zwt_init` 取换年后的 `zwt`。迭代计数延续。
    pub fn carried_over(
        &self,
        previous: &[Option<usize>],
        patches: &[BaseflowPatchInit],
    ) -> Result<Self> {
        ensure!(
            previous.len() == patches.len(),
            "the LULCC pairing has {} entries for {} patches",
            previous.len(),
            patches.len()
        );
        let patches = previous
            .iter()
            .zip(patches)
            .map(|(old, init)| -> Result<PatchBaseflow> {
                let adjustable = init.patch_type <= 1;
                Ok(match old {
                    Some(old) => {
                        let state = self
                            .patches
                            .get(*old)
                            .with_context(|| format!("old patch {old} is out of range"))?;
                        PatchBaseflow {
                            adjustable,
                            ..state.clone()
                        }
                    }
                    None => PatchBaseflow {
                        scale: 1.0,
                        initial_water_table_depth_m: init.water_table_depth_m,
                        recharge_mm: None,
                        subsurface_runoff_mm: None,
                        adjustable,
                    },
                })
            })
            .collect::<Result<_>>()?;
        Ok(Self {
            patches,
            iteration: self.iteration,
            directory: self.directory.clone(),
            file_name: self.file_name.clone(),
        })
    }

    /// 当前的 `scale_baseflow` 整向量。
    pub fn scales(&self) -> Vec<f64> {
        self.patches.iter().map(|patch| patch.scale).collect()
    }

    pub fn patch_count(&self) -> usize {
        self.patches.len()
    }

    /// 下一步 `rsubst` 要乘的 `scale_baseflow(patch)`。
    pub fn scale(&self, patch: usize) -> f64 {
        self.patches[patch].scale
    }

    /// 已经完成的年数（上游 `iter_bf_opt`）。
    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    /// `BaseFlow_Optimize` 里每步的两次 `add_spv`。
    ///
    /// `recharge` 带 `WHERE` 掩码：四个输入任一是 `spval` 就保持 `spval`、不累加。
    /// 累加写成 `s + var*dt`：GIMPLE 里乘积被 `s = var*dt` 那一支共用，没有收缩成 FMA。
    pub fn accumulate(&mut self, patch: usize, step: BaseflowStep) {
        let state = &mut self.patches[patch];
        let inputs = [
            step.convective_precipitation_kg_m2_s,
            step.large_scale_precipitation_kg_m2_s,
            step.total_evaporation_kg_m2_s,
            step.surface_runoff_mm_s,
        ];
        let recharge = if inputs.contains(&colm_core::MISSING) {
            colm_core::MISSING
        } else {
            step.convective_precipitation_kg_m2_s + step.large_scale_precipitation_kg_m2_s
                - step.total_evaporation_kg_m2_s
                - step.surface_runoff_mm_s
        };
        add_spv(&mut state.recharge_mm, recharge, step.time_step_seconds);
        add_spv(
            &mut state.subsurface_runoff_mm,
            step.subsurface_runoff_mm_s,
            step.time_step_seconds,
        );
    }

    /// 跨年那一步：写本轮记录、逐 patch 更新 `scale_baseflow`、整向量写回、复位累加器。
    /// `water_table_depth_m` 是各 patch 步末的 `zwt`。
    pub fn close_year(&mut self, water_table_depth_m: &[f64]) -> Result<()> {
        ensure!(
            water_table_depth_m.len() == self.patches.len(),
            "the baseflow optimizer has {} patches but got {} water tables",
            self.patches.len(),
            water_table_depth_m.len()
        );
        self.iteration += 1;
        let cycle_directory = self.directory.join(format!("c{:04}", self.iteration));
        std::fs::create_dir_all(&cycle_directory)
            .with_context(|| format!("cannot create {}", cycle_directory.display()))?;
        let column = |patches: &[PatchBaseflow], value: fn(&PatchBaseflow) -> f64| -> Vec<f64> {
            patches.iter().map(value).collect()
        };
        let patches = &self.patches;
        write_vector_file(
            &cycle_directory.join(&self.file_name),
            &[
                ("zwt", water_table_depth_m.to_vec()),
                (
                    "zwt_init",
                    column(patches, |p| p.initial_water_table_depth_m),
                ),
                ("scale_baseflow", column(patches, |p| p.scale)),
                (
                    "total_recharge",
                    column(patches, |p| p.recharge_mm.unwrap_or(SPVAL)),
                ),
                (
                    "total_subsurface_runoff",
                    column(patches, |p| p.subsurface_runoff_mm.unwrap_or(SPVAL)),
                ),
            ],
        )?;

        for (state, &zwt) in self.patches.iter_mut().zip(water_table_depth_m) {
            state.scale = adjusted_scale(
                state.scale,
                state.adjustable,
                zwt,
                state.initial_water_table_depth_m,
                state.recharge_mm.unwrap_or(SPVAL),
                state.subsurface_runoff_mm.unwrap_or(SPVAL),
            );
        }

        write_vector_file(
            &self.directory.join(&self.file_name),
            &[("scale_baseflow", column(&self.patches, |p| p.scale))],
        )?;
        for state in &mut self.patches {
            state.recharge_mm = None;
            state.subsurface_runoff_mm = None;
        }
        Ok(())
    }
}

/// `add_spv (var, s, dt)`：`s` 是 `spval` 时直接取 `var*dt`，否则累加。
fn add_spv(sum: &mut Option<f64>, value: f64, time_step_seconds: f64) {
    // `IF (var(i) /= spval)`：湿地上的 `rsub` 就是 `spval`。
    if value == colm_core::MISSING {
        return;
    }
    let increment = value * time_step_seconds;
    *sum = Some(match *sum {
        Some(previous) => previous + increment,
        None => increment,
    });
}

/// `MOD_Opt_Baseflow.F90:118-140`。两个条件不互斥地先后判（上游就是两个独立的 `IF`），
/// 但它们对 `rchg` 与 `rsub` 的大小要求相反，至多一个成立。
/// 两个累加器都是 `spval`（负数）时 `> 0` 不成立，只剩下限那一句。
fn adjusted_scale(
    scale: f64,
    adjustable: bool,
    water_table_depth_m: f64,
    initial_water_table_depth_m: f64,
    recharge_mm: f64,
    subsurface_runoff_mm: f64,
) -> f64 {
    if !adjustable {
        return scale;
    }
    let mut scale = scale;
    if recharge_mm > 0.0 && subsurface_runoff_mm > 0.0 {
        if water_table_depth_m > initial_water_table_depth_m && recharge_mm < subsurface_runoff_mm {
            scale *= recharge_mm / subsurface_runoff_mm;
        }
        if water_table_depth_m < initial_water_table_depth_m && recharge_mm > subsurface_runoff_mm {
            scale *= recharge_mm / subsurface_runoff_mm;
        }
    }
    1.0e-8_f64.max(scale)
}

/// `ncio_create_file_vector` + `ncio_define_dimension_vector(..., 'patch')` +
/// 若干 `ncio_write_vector`：维度 `patch` 的长度就是站点的 patch 数。
fn write_vector_file(path: &Path, variables: &[(&str, Vec<f64>)]) -> Result<()> {
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    let length = variables.first().map_or(0, |(_, values)| values.len());
    file.add_dimension("patch", length)?;
    for (name, values) in variables {
        let mut variable = file.add_variable::<f64>(name, &["patch"])?;
        variable
            .put_values(values, netcdf::Extents::All)
            .with_context(|| format!("cannot write {name} to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "baseflow_optimizer_tests.rs"]
mod baseflow_optimizer_tests;
