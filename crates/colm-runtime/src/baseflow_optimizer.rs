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

use anyhow::{Context, Result};

/// 上游的 `spval`：累加器"还没有值"的标记，也是写进文件的值。
const SPVAL: f64 = -1.0e36;

/// 单个 patch 的优化器状态（`scale_baseflow`/`zwt_init`/`rchg_year`/`rsub_year`/`iter_bf_opt`）。
#[derive(Debug, Clone, PartialEq)]
pub struct BaseflowOptimizer {
    scale: f64,
    initial_water_table_depth_m: f64,
    /// `rchg_year`：`None` 即上游的 `spval`。
    recharge_mm: Option<f64>,
    /// `rsub_year`，同上。
    subsurface_runoff_mm: Option<f64>,
    iteration: u32,
    /// `patchtype <= 1`：只有土壤与城市 patch 会被改写 `scale_baseflow`。
    adjustable: bool,
    /// `DEF_dir_restart/ParaOpt`。
    directory: PathBuf,
    /// `<case>_baseflow_w180_s90.nc`：`ncio_create_file_vector` 与读取同一套块后缀。
    file_name: String,
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
    /// `Opt_Baseflow_init`：`scale` 是刚从 `ParaOpt/<case>_baseflow.nc` 读到的值（缺省 1），
    /// `zwt_init` 取启动时刻的 `zwt`。
    pub fn new(
        scale: f64,
        initial_water_table_depth_m: f64,
        patch_type: i32,
        directory: impl Into<PathBuf>,
        case_name: &str,
    ) -> Self {
        Self {
            scale,
            initial_water_table_depth_m,
            recharge_mm: None,
            subsurface_runoff_mm: None,
            iteration: 0,
            adjustable: patch_type <= 1,
            directory: directory.into(),
            file_name: format!("{case_name}_baseflow_w180_s90.nc"),
        }
    }

    /// 下一步 `rsubst` 要乘的 `scale_baseflow(ipatch)`。
    pub fn scale(&self) -> f64 {
        self.scale
    }

    /// 已经完成的年数（上游 `iter_bf_opt`）。
    pub fn iteration(&self) -> u32 {
        self.iteration
    }

    /// `BaseFlow_Optimize` 里每步的两次 `add_spv`。
    ///
    /// 单点强迫与通量都不会是 `spval`，所以上游的 `WHERE` 掩码在这里恒真。
    /// 累加写成 `s + var*dt`：GIMPLE 里乘积被 `s = var*dt` 那一支共用，没有收缩成 FMA。
    pub fn accumulate(&mut self, step: BaseflowStep) {
        let recharge = step.convective_precipitation_kg_m2_s
            + step.large_scale_precipitation_kg_m2_s
            - step.total_evaporation_kg_m2_s
            - step.surface_runoff_mm_s;
        add_spv(&mut self.recharge_mm, recharge, step.time_step_seconds);
        add_spv(
            &mut self.subsurface_runoff_mm,
            step.subsurface_runoff_mm_s,
            step.time_step_seconds,
        );
    }

    /// 跨年那一步：写本轮记录、更新 `scale_baseflow`、写回、复位累加器。
    pub fn close_year(&mut self, water_table_depth_m: f64) -> Result<()> {
        self.iteration += 1;
        let cycle_directory = self.directory.join(format!("c{:04}", self.iteration));
        std::fs::create_dir_all(&cycle_directory)
            .with_context(|| format!("cannot create {}", cycle_directory.display()))?;
        let recharge = self.recharge_mm.unwrap_or(SPVAL);
        let subsurface = self.subsurface_runoff_mm.unwrap_or(SPVAL);
        write_vector_file(
            &cycle_directory.join(&self.file_name),
            &[
                ("zwt", water_table_depth_m),
                ("zwt_init", self.initial_water_table_depth_m),
                ("scale_baseflow", self.scale),
                ("total_recharge", recharge),
                ("total_subsurface_runoff", subsurface),
            ],
        )?;

        self.scale = adjusted_scale(
            self.scale,
            self.adjustable,
            water_table_depth_m,
            self.initial_water_table_depth_m,
            recharge,
            subsurface,
        );

        write_vector_file(
            &self.directory.join(&self.file_name),
            &[("scale_baseflow", self.scale)],
        )?;
        self.recharge_mm = None;
        self.subsurface_runoff_mm = None;
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
/// 若干 `ncio_write_vector`：单点只有一个 patch。
fn write_vector_file(path: &Path, variables: &[(&str, f64)]) -> Result<()> {
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    file.add_dimension("patch", 1)?;
    for (name, value) in variables {
        let mut variable = file.add_variable::<f64>(name, &["patch"])?;
        variable
            .put_values(&[*value], netcdf::Extents::All)
            .with_context(|| format!("cannot write {name} to {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "baseflow_optimizer_tests.rs"]
mod baseflow_optimizer_tests;
