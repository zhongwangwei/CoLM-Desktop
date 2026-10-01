//! history 旁车里的示踪物部分：区间跨过重启时，上游在主机累加器之后再写
//! `tracer_history_write`（`tracer_land_history_restart.inc`：描述符身份 + 44 个
//! `a_trc_*`/`a_water_*`），再走生命周期钩子（CH4 的 `write_methane_accflux_restart`）。
//! 读回时（`read_history_acc_restart`）先核对描述符，再整组读回并要求全部有限。
//!
//! 盘上布局照 `ncio_write_vector`：逐示踪物量是 `(patch, d1 = ntracers)`，逐层的
//! `a_trc_soil_mass`/`a_trc_snow_mass` 是 `(patch, d2 = 层, d1 = ntracers)`，按 patch 的
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

/// `trc_hist_schema`。
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
    ("a_trc_precip", Slot::Tracer(|a| &mut a.precip)),
    (
        "a_trc_vapor_exchange",
        Slot::Tracer(|a| &mut a.vapor_exchange),
    ),
    ("a_water_precip", Slot::Tracer(|a| &mut a.water_precip)),
    ("a_trc_evap", Slot::Tracer(|a| &mut a.evap)),
    (
        "a_water_evap_gross",
        Slot::Tracer(|a| &mut a.water_evap_gross),
    ),
    ("a_trc_transp", Slot::Tracer(|a| &mut a.transp)),
    ("a_trc_transp_src", Slot::Tracer(|a| &mut a.transp_src)),
    ("a_water_transp", Slot::Tracer(|a| &mut a.water_transp)),
    ("a_trc_soilevap", Slot::Tracer(|a| &mut a.soilevap)),
    ("a_water_soilevap", Slot::Tracer(|a| &mut a.water_soilevap)),
    ("a_trc_canopyevap", Slot::Tracer(|a| &mut a.canopyevap)),
    (
        "a_water_canopyevap",
        Slot::Tracer(|a| &mut a.water_canopyevap),
    ),
    ("a_trc_subl", Slot::Tracer(|a| &mut a.subl)),
    ("a_water_subl", Slot::Tracer(|a| &mut a.water_subl)),
    ("a_trc_wetland_evap", Slot::Tracer(|a| &mut a.wetland_evap)),
    (
        "a_water_wetland_evap",
        Slot::Tracer(|a| &mut a.water_wetland_evap),
    ),
    ("a_trc_rsur", Slot::Tracer(|a| &mut a.rsur)),
    ("a_trc_rsub", Slot::Tracer(|a| &mut a.rsub)),
    ("a_trc_rnof", Slot::Tracer(|a| &mut a.rnof)),
    ("a_water_rnof", Slot::Tracer(|a| &mut a.water_rnof)),
    ("a_trc_qinfl", Slot::Tracer(|a| &mut a.qinfl)),
    ("a_trc_qcharge", Slot::Tracer(|a| &mut a.qcharge)),
    ("a_trc_ldew_mass", Slot::Tracer(|a| &mut a.ldew_mass)),
    ("a_water_ldew", Slot::Water(|w| &mut w.ldew)),
    ("a_trc_soil_mass", Slot::TracerSoil),
    ("a_water_soil", Slot::WaterSoil),
    ("a_trc_snow_mass", Slot::TracerSnow),
    ("a_water_snow", Slot::WaterSnow),
    ("a_trc_wa_mass", Slot::Tracer(|a| &mut a.wa_mass)),
    ("a_water_wa", Slot::Water(|w| &mut w.wa)),
    ("a_trc_wa_debt_mass", Slot::Tracer(|a| &mut a.wa_debt_mass)),
    ("a_water_wa_debt", Slot::Water(|w| &mut w.wa_debt)),
    (
        "a_water_aquifer_actual",
        Slot::Water(|w| &mut w.aquifer_actual),
    ),
    (
        "a_trc_aquifer_actual_mass",
        Slot::Tracer(|a| &mut a.aquifer_actual_mass),
    ),
    ("a_trc_wdsrf_mass", Slot::Tracer(|a| &mut a.wdsrf_mass)),
    ("a_water_wdsrf", Slot::Water(|w| &mut w.wdsrf)),
    ("a_trc_wetwat_mass", Slot::Tracer(|a| &mut a.wetwat_mass)),
    ("a_water_wetwat", Slot::Water(|w| &mut w.wetwat)),
    (
        "a_trc_surface_residue_mass",
        Slot::Tracer(|a| &mut a.surface_residue_mass),
    ),
    (
        "a_trc_subsurface_residue_mass",
        Slot::Tracer(|a| &mut a.subsurface_residue_mass),
    ),
    (
        "a_trc_layer_dry_mass",
        Slot::Tracer(|a| &mut a.layer_dry_mass),
    ),
    ("a_trc_solid_mass", Slot::Tracer(|a| &mut a.solid_mass)),
    ("a_trc_scv_mass", Slot::Tracer(|a| &mut a.scv_mass)),
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

/// 区间跨过重启时把示踪物部分追加进刚写好的旁车（主机累加器之后、`history_complete` 不变）。
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
        put_scalar_i32(&mut file, "trc_hist_schema", SCHEMA)?;
        put_scalar_i32(&mut file, "trc_hist_count", i32::try_from(ntracers)?)?;
        ensure_dimension(
            &mut file,
            "trc_hist_descriptor_field",
            colm_core::tracer::DESCRIPTOR_IDENTITY_WIDTH,
        )?;
        ensure_dimension(&mut file, "trc_hist_descriptor_row", ntracers)?;
        put_array_i32(
            &mut file,
            "trc_hist_descriptor",
            &["trc_hist_descriptor_row", "trc_hist_descriptor_field"],
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

/// 旁车区间开着（`history_nac > 0`）时读回示踪物部分；旁车不存在或区间已关返回空。
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
    let Some(nac) = file.variable("history_nac") else {
        return Ok(RestoredTracers::default());
    };
    let nac = nac.get_values::<f64, _>(..)?;
    if !nac.first().is_some_and(|&n| n > 0.0) {
        return Ok(RestoredTracers::default());
    }
    let mut restored = RestoredTracers::default();
    if let Some(set) = set.filter(|set| set.transport_indices().next().is_some()) {
        let ntracers = set.len();
        let missing: Vec<&str> = FIELDS
            .iter()
            .map(|(name, _)| *name)
            .filter(|name| file.variable(name).is_none())
            .collect();
        ensure!(
            missing.is_empty(),
            "generic land-tracer history missing from active restart window ({})",
            missing.join(", ")
        );
        check_descriptor(&file, set)?;
        let mut accumulators = vec![
            PatchAccumulators {
                tracers: vec![TracerAccumulators::default(); ntracers],
                water: WaterAccumulators::default(),
            };
            patches
        ];
        for (name, slot) in FIELDS {
            let (dims, width) = shape(name, slot, ntracers);
            let variable = file.variable(name).expect("presence checked above");
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
                "{name} in {} has shape {actual:?}, expected {expected:?}",
                sidecar.display()
            );
            let values = variable.get_values::<f64, _>(..)?;
            ensure!(
                values.iter().all(|x| x.is_finite()),
                "non-finite generic land-tracer history accumulator"
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
    let schema = scalar("trc_hist_schema")?;
    let count = scalar("trc_hist_count")?;
    if schema != SCHEMA || usize::try_from(count).ok() != Some(set.len()) {
        bail!("incompatible or incomplete generic land-tracer history sidecar");
    }
    let expected: Vec<i32> = set
        .descriptor_identity_all()
        .into_iter()
        .flatten()
        .collect();
    let variable = file
        .variable("trc_hist_descriptor")
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
