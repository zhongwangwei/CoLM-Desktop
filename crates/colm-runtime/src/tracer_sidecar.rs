//! history 旁车里的示踪物部分：区间跨过重启时，上游在主机累加器之后再写
//! `tracer_history_write`（描述符身份 + 44 个 `a_mt_*`/`a_water_*`），再走生命周期钩子
//! （CH4 的 `write_methane_accflux_restart`）。读回时（`read_history_acc_restart`）先核对
//! 描述符，再整组读回并要求全部有限。
//!
//! 上游把示踪物（TRACER）整体改名成材料（MATERIAL）时把盘上字段一并改成 `a_mt_*`，
//! 但给读回留了逐字段的旧名回退（`MOD_HistAccRestart.F90` 的 `hacc_read_name`：
//! 新名不存在时读 `a_trc_*`）。回退是**逐字段**的，不是整组：旧旁车里可以混着新旧名。
//! 判据只看变量在不在（上游 `ncio_vector_exist` 同样只看存在性），形状校验仍按读到的名字做。
//!
//! 盘上布局照 `ncio_write_vector`：逐示踪物量是 `(patch, d1 = ntracers)`，逐层的
//! `a_mt_soil_mass`/`a_mt_snow_mass` 是 `(patch, d2 = 层, d1 = ntracers)`，按 patch 的
//! 水量是 `(patch)` 或 `(patch, d1 = 层)`。雪层下标 `jsnow - 1 = soisno_slot(j)`。

use anyhow::{bail, ensure, Context, Result};
use colm_core::methane::driver::CoreAccumulator;
use colm_core::tracer::{
    PatchTracerState, TracerAccumulators, TracerSet, WaterAccumulators, MAX_SNOW_LAYERS,
    SOIL_LAYERS,
};
use std::path::Path;

use crate::methane::MethaneSetup;
use crate::tracer::{ensure_dimension, put_array_f64, put_array_i32, put_scalar_i32};

/// `mt_hist_schema`。
const SCHEMA: i32 = 1;

/// 一个累加器在状态里的位置。
#[derive(Clone, Copy)]
enum Slot {
    Tracer(fn(&mut TracerAccumulators) -> &mut f64),
    TracerSoil,
    TracerSnow,
    Water(fn(&mut WaterAccumulators) -> &mut f64),
    WaterSoil,
    WaterSnow,
}

/// `tracer_history_manifest` 的 44 项，顺序即上游写出顺序。
const FIELDS: [(&str, Slot); 44] = [
    ("a_mt_precip", Slot::Tracer(|a| &mut a.precip)),
    (
        "a_mt_vapor_exchange",
        Slot::Tracer(|a| &mut a.vapor_exchange),
    ),
    ("a_water_precip", Slot::Tracer(|a| &mut a.water_precip)),
    ("a_mt_evap", Slot::Tracer(|a| &mut a.evap)),
    (
        "a_water_evap_gross",
        Slot::Tracer(|a| &mut a.water_evap_gross),
    ),
    ("a_mt_transp", Slot::Tracer(|a| &mut a.transp)),
    ("a_mt_transp_src", Slot::Tracer(|a| &mut a.transp_src)),
    ("a_water_transp", Slot::Tracer(|a| &mut a.water_transp)),
    ("a_mt_soilevap", Slot::Tracer(|a| &mut a.soilevap)),
    ("a_water_soilevap", Slot::Tracer(|a| &mut a.water_soilevap)),
    ("a_mt_canopyevap", Slot::Tracer(|a| &mut a.canopyevap)),
    (
        "a_water_canopyevap",
        Slot::Tracer(|a| &mut a.water_canopyevap),
    ),
    ("a_mt_subl", Slot::Tracer(|a| &mut a.subl)),
    ("a_water_subl", Slot::Tracer(|a| &mut a.water_subl)),
    ("a_mt_wetland_evap", Slot::Tracer(|a| &mut a.wetland_evap)),
    (
        "a_water_wetland_evap",
        Slot::Tracer(|a| &mut a.water_wetland_evap),
    ),
    ("a_mt_rsur", Slot::Tracer(|a| &mut a.rsur)),
    ("a_mt_rsub", Slot::Tracer(|a| &mut a.rsub)),
    ("a_mt_rnof", Slot::Tracer(|a| &mut a.rnof)),
    ("a_water_rnof", Slot::Tracer(|a| &mut a.water_rnof)),
    ("a_mt_qinfl", Slot::Tracer(|a| &mut a.qinfl)),
    ("a_mt_qcharge", Slot::Tracer(|a| &mut a.qcharge)),
    ("a_mt_ldew_mass", Slot::Tracer(|a| &mut a.ldew_mass)),
    ("a_water_ldew", Slot::Water(|w| &mut w.ldew)),
    ("a_mt_soil_mass", Slot::TracerSoil),
    ("a_water_soil", Slot::WaterSoil),
    ("a_mt_snow_mass", Slot::TracerSnow),
    ("a_water_snow", Slot::WaterSnow),
    ("a_mt_wa_mass", Slot::Tracer(|a| &mut a.wa_mass)),
    ("a_water_wa", Slot::Water(|w| &mut w.wa)),
    ("a_mt_wa_debt_mass", Slot::Tracer(|a| &mut a.wa_debt_mass)),
    ("a_water_wa_debt", Slot::Water(|w| &mut w.wa_debt)),
    (
        "a_water_aquifer_actual",
        Slot::Water(|w| &mut w.aquifer_actual),
    ),
    (
        "a_mt_aquifer_actual_mass",
        Slot::Tracer(|a| &mut a.aquifer_actual_mass),
    ),
    ("a_mt_wdsrf_mass", Slot::Tracer(|a| &mut a.wdsrf_mass)),
    ("a_water_wdsrf", Slot::Water(|w| &mut w.wdsrf)),
    ("a_mt_wetwat_mass", Slot::Tracer(|a| &mut a.wetwat_mass)),
    ("a_water_wetwat", Slot::Water(|w| &mut w.wetwat)),
    (
        "a_mt_surface_residue_mass",
        Slot::Tracer(|a| &mut a.surface_residue_mass),
    ),
    (
        "a_mt_subsurface_residue_mass",
        Slot::Tracer(|a| &mut a.subsurface_residue_mass),
    ),
    (
        "a_mt_layer_dry_mass",
        Slot::Tracer(|a| &mut a.layer_dry_mass),
    ),
    ("a_mt_solid_mass", Slot::Tracer(|a| &mut a.solid_mass)),
    ("a_mt_scv_mass", Slot::Tracer(|a| &mut a.scv_mass)),
    ("a_water_scv", Slot::Water(|w| &mut w.scv)),
];

/// 一个 patch 的示踪物累加器（逐示踪物 + 按 patch 的水量）。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchAccumulators {
    pub tracers: Vec<TracerAccumulators>,
    pub water: WaterAccumulators,
}

impl PatchAccumulators {
    pub fn of(state: &PatchTracerState) -> Self {
        Self {
            tracers: state.acc.clone(),
            water: state.water_acc.clone(),
        }
    }

    /// 本 patch 一个清单项的值，按盘上顺序（层在外、示踪物在内）。
    fn values(&mut self, slot: Slot) -> Vec<f64> {
        match slot {
            Slot::Tracer(field) => self.tracers.iter_mut().map(|a| *field(a)).collect(),
            Slot::TracerSoil => (0..SOIL_LAYERS)
                .flat_map(|j| self.tracers.iter().map(move |a| a.soil_mass[j]))
                .collect(),
            Slot::TracerSnow => (0..MAX_SNOW_LAYERS)
                .flat_map(|j| self.tracers.iter().map(move |a| a.snow_mass[j]))
                .collect(),
            Slot::Water(field) => vec![*field(&mut self.water)],
            Slot::WaterSoil => self.water.soil.to_vec(),
            Slot::WaterSnow => self.water.snow.to_vec(),
        }
    }

    fn set(&mut self, slot: Slot, values: &[f64]) {
        let n = self.tracers.len();
        match slot {
            Slot::Tracer(field) => {
                for (a, &v) in self.tracers.iter_mut().zip(values) {
                    *field(a) = v;
                }
            }
            Slot::TracerSoil => {
                for j in 0..SOIL_LAYERS {
                    for (i, a) in self.tracers.iter_mut().enumerate() {
                        a.soil_mass[j] = values[j * n + i];
                    }
                }
            }
            Slot::TracerSnow => {
                for j in 0..MAX_SNOW_LAYERS {
                    for (i, a) in self.tracers.iter_mut().enumerate() {
                        a.snow_mass[j] = values[j * n + i];
                    }
                }
            }
            Slot::Water(field) => *field(&mut self.water) = values[0],
            Slot::WaterSoil => self.water.soil.copy_from_slice(values),
            Slot::WaterSnow => self.water.snow.copy_from_slice(values),
        }
    }
}

/// 清单项的盘上维度（不含 `patch`）与每 patch 的宽度。
fn shape(name: &str, slot: Slot, ntracers: usize) -> (Vec<(String, usize)>, usize) {
    let d1 = format!("d1_{name}");
    let d2 = format!("d2_{name}");
    let dims = match slot {
        Slot::Tracer(_) => vec![(d1, ntracers)],
        Slot::TracerSoil => vec![(d2, SOIL_LAYERS), (d1, ntracers)],
        Slot::TracerSnow => vec![(d2, MAX_SNOW_LAYERS), (d1, ntracers)],
        Slot::Water(_) => Vec::new(),
        Slot::WaterSoil => vec![(d1, SOIL_LAYERS)],
        Slot::WaterSnow => vec![(d1, MAX_SNOW_LAYERS)],
    };
    let width = dims.iter().map(|(_, len)| len).product();
    (dims, width)
}

/// 旁车里示踪物部分要写的东西。`set` 有输运示踪物时写 44 个累加器（`tracer_history_required`，
/// `patches` 逐 patch），`methane` 是 CH4 provider 的累加量。
pub struct SidecarTracers<'a> {
    pub set: Option<&'a TracerSet>,
    pub patches: Vec<PatchAccumulators>,
    pub methane: Option<(&'a MethaneSetup, Vec<CoreAccumulator>)>,
}

/// 运行终点不在 history 自然边界上时，上游先把原始窗口存进旁车、再写最后一条 history 并清零
/// （`MOD_Hist.F90:270`）。示踪物与 CH4 的累加器在清零前留一份，供终点那份旁车使用。
#[derive(Debug, Clone, PartialEq)]
pub struct RawTracers {
    pub tracer: Option<PatchAccumulators>,
    pub methane: Option<CoreAccumulator>,
}

/// history 会话与续跑写出共享的终点快照。
pub type RawTracersHandle = std::sync::Arc<std::sync::Mutex<Option<Vec<RawTracers>>>>;

/// 区间跨过重启时把示踪物部分追加进刚写好的旁车（主机累加器之后、完成标记 `hacc_complete`
/// 置 1 之前 —— 上游 `write_sidecar` 里的 `tracer_history_write` 也是这个位置）。
pub fn write(path: &Path, tracers: &SidecarTracers<'_>) -> Result<()> {
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    if let Some(set) = tracers.set {
        let identity: Vec<i32> = set
            .descriptor_identity_all()
            .into_iter()
            .flatten()
            .collect();
        let ntracers = set.len();
        put_scalar_i32(&mut file, "mt_hist_schema", SCHEMA)?;
        put_scalar_i32(&mut file, "mt_hist_count", i32::try_from(ntracers)?)?;
        ensure_dimension(
            &mut file,
            "mt_hist_descriptor_field",
            colm_core::tracer::DESCRIPTOR_IDENTITY_WIDTH,
        )?;
        ensure_dimension(&mut file, "mt_hist_descriptor_row", ntracers)?;
        put_array_i32(
            &mut file,
            "mt_hist_descriptor",
            &["mt_hist_descriptor_row", "mt_hist_descriptor_field"],
            &identity,
        )?;
        let mut patches = tracers.patches.clone();
        for (name, slot) in FIELDS {
            let (dims, _) = shape(name, slot, ntracers);
            for (dim, len) in &dims {
                ensure_dimension(&mut file, dim, *len)?;
            }
            let mut names = vec!["patch"];
            names.extend(dims.iter().map(|(dim, _)| dim.as_str()));
            let values: Vec<f64> = patches.iter_mut().flat_map(|p| p.values(slot)).collect();
            put_array_f64(&mut file, name, &names, &values)?;
        }
    }
    if let Some((setup, accumulators)) = &tracers.methane {
        let accumulators: Vec<&CoreAccumulator> = accumulators.iter().collect();
        crate::methane::write_accflux(&mut file, setup, &accumulators)?;
    }
    Ok(())
}

/// 读回的示踪物部分：没有对应内容时为 `None`。
#[derive(Debug, Default)]
pub struct RestoredTracers {
    pub tracers: Option<Vec<PatchAccumulators>>,
    pub methane: Option<Vec<CoreAccumulator>>,
}

/// 旁车区间开着（`hacc_nac > 0`，旧 Rust 旁车是 `history_nac`）时读回示踪物部分；
/// 旁车不存在或区间已关返回空。
/// 主机部分的标记校验由 [`crate::history_sidecar::read_sidecar`] 负责。
pub fn read(
    sidecar: &Path,
    set: Option<&TracerSet>,
    methane: Option<&MethaneSetup>,
    patches: usize,
) -> Result<RestoredTracers> {
    if !sidecar.is_file() {
        return Ok(RestoredTracers::default());
    }
    let file =
        netcdf::open(sidecar).with_context(|| format!("cannot open {}", sidecar.display()))?;
    // 主机样本数标记：新格式 `hacc_nac`，旧 Rust 旁车 `history_nac`（本轮起只写新名）。
    let Some(nac) = file
        .variable("hacc_nac")
        .or_else(|| file.variable("history_nac"))
    else {
        return Ok(RestoredTracers::default());
    };
    let nac = nac.get_values::<f64, _>(..)?;
    if !nac.first().is_some_and(|&n| n > 0.0) {
        return Ok(RestoredTracers::default());
    }
    let mut restored = RestoredTracers::default();
    if let Some(set) = set.filter(|set| set.transport_indices().next().is_some()) {
        let ntracers = set.len();
        // 逐字段解析读回名（`hacc_read_name`），缺任何一个都整组拒绝：这些累加和必须成套，
        // 半套读回来等于把一部分示踪物的区间悄悄截断。
        let read_names: Vec<(&str, std::borrow::Cow<'_, str>, Slot)> = FIELDS
            .iter()
            .map(|&(name, slot)| Ok((name, read_name(&file, name)?, slot)))
            .collect::<Result<_>>()?;
        check_descriptor(&file, set)?;
        let mut accumulators = vec![
            PatchAccumulators {
                tracers: vec![TracerAccumulators::default(); ntracers],
                water: WaterAccumulators::default(),
            };
            patches
        ];
        for (name, stored, slot) in read_names {
            let (dims, width) = shape(name, slot, ntracers);
            let variable = file
                .variable(stored.as_ref())
                .expect("presence checked by read_name");
            let actual: Vec<(String, usize)> = variable
                .dimensions()
                .iter()
                .map(|dim| (dim.name(), dim.len()))
                .collect();
            let mut expected = vec![("patch".to_owned(), patches)];
            expected.extend(dims);
            ensure!(
                actual
                    .iter()
                    .map(|(_, len)| len)
                    .eq(expected.iter().map(|(_, len)| len)),
                "{stored} in {} has shape {actual:?}, expected {expected:?}",
                sidecar.display()
            );
            let values = variable.get_values::<f64, _>(..)?;
            ensure!(
                values.iter().all(|x| x.is_finite()),
                "non-finite generic land-tracer history accumulator ({stored})"
            );
            for (patch, target) in accumulators.iter_mut().enumerate() {
                target.set(slot, &values[patch * width..(patch + 1) * width]);
            }
        }
        restored.tracers = Some(accumulators);
    }
    if let Some(setup) = methane {
        restored.methane = crate::methane::read_accflux_sidecar(&file, setup, patches)?;
    }
    Ok(restored)
}

/// `hacc_read_name`：`a_mt_*` 优先，变量不在时回退到旧名 `a_trc_*`。
///
/// 上游的判据是 `ncio_vector_exist`，只看变量存在、不看形状 —— 新名存在但形状坏了要
/// 让后面的形状校验报错，不能回退去读旧名里可能过期的数据，所以这里同样只判存在。
/// 回退只对 `a_mt_` 前缀生效；`a_water_*` 没有改过名。
fn read_name<'a>(file: &netcdf::File, name: &'a str) -> Result<std::borrow::Cow<'a, str>> {
    if file.variable(name).is_some() {
        return Ok(std::borrow::Cow::Borrowed(name));
    }
    match name.strip_prefix("a_mt_") {
        Some(rest) => {
            let legacy = format!("a_trc_{rest}");
            if file.variable(&legacy).is_some() {
                Ok(std::borrow::Cow::Owned(legacy))
            } else {
                bail!(
                    "generic land-tracer history missing from active restart window ({name}, \
                     nor legacy {legacy})"
                )
            }
        }
        None => bail!("generic land-tracer history missing from active restart window ({name})"),
    }
}

/// `tracer_history_descriptor(file, .false.)`：schema、示踪物数与逐行身份都要对上。
fn check_descriptor(file: &netcdf::File, set: &TracerSet) -> Result<()> {
    let scalar = |name: &str| -> Result<i32> {
        let variable = file.variable(name).with_context(|| {
            format!("incompatible or incomplete generic land-tracer history sidecar: no {name}")
        })?;
        ensure!(
            variable.dimensions().is_empty(),
            "incompatible or incomplete generic land-tracer history sidecar: {name} is not a scalar"
        );
        Ok(variable.get_value::<i32, _>(())?)
    };
    let schema = scalar("mt_hist_schema")?;
    let count = scalar("mt_hist_count")?;
    if schema != SCHEMA || usize::try_from(count).ok() != Some(set.len()) {
        bail!("incompatible or incomplete generic land-tracer history sidecar");
    }
    let expected: Vec<i32> = set
        .descriptor_identity_all()
        .into_iter()
        .flatten()
        .collect();
    let variable = file
        .variable("mt_hist_descriptor")
        .context("incompatible or incomplete generic land-tracer history sidecar")?;
    let lens: Vec<usize> = variable.dimensions().iter().map(|dim| dim.len()).collect();
    ensure!(
        lens == [set.len(), colm_core::tracer::DESCRIPTOR_IDENTITY_WIDTH]
            && variable.get_values::<i32, _>(..)? == expected,
        "incompatible or incomplete generic land-tracer history sidecar"
    );
    Ok(())
}

#[cfg(test)]
#[path = "tracer_sidecar_tests.rs"]
mod tests;
