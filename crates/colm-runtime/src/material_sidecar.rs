//! history 旁车里的材料部分：区间跨过重启时，上游在主机累加器之后再写
//! `material_history_write`（描述符身份 + 44 个 `a_mt_*`/`a_water_*`），再走生命周期钩子
//! （CH4 的 `write_methane_accflux_restart`）。读回时（`read_history_acc_restart`）先核对
//! 描述符，再整组读回并要求全部有限。
//!
//! 上游把材料（MATERIAL）整体改名成材料（MATERIAL）时把盘上字段一并改成 `a_mt_*`，
//! 但给读回留了逐字段的旧名回退（`MOD_HistAccRestart.F90` 的 `hacc_read_name`：
//! 新名不存在时读 `a_trc_*`）。回退是**逐字段**的，不是整组：旧旁车里可以混着新旧名。
//! 判据只看变量在不在（上游 `ncio_vector_exist` 同样只看存在性），形状校验仍按读到的名字做。
//!
//! 盘上布局照 `ncio_write_vector`：逐材料量是 `(patch, d1 = nmaterials)`，逐层的
//! `a_mt_soil_mass`/`a_mt_snow_mass` 是 `(patch, d2 = 层, d1 = nmaterials)`，按 patch 的
//! 水量是 `(patch)` 或 `(patch, d1 = 层)`。雪层下标 `jsnow - 1 = soisno_slot(j)`。

use anyhow::{bail, ensure, Context, Result};
use colm_core::material::{
    MaterialAccumulators, MaterialSet, PatchMaterialState, WaterAccumulators, MAX_SNOW_LAYERS,
    SOIL_LAYERS,
};
use colm_core::methane::driver::CoreAccumulator;
use std::path::Path;

use crate::material::{ensure_dimension, put_array_f64, put_array_i32, put_scalar_i32};
use crate::methane::MethaneSetup;

/// `mt_hist_schema`。
const SCHEMA: i32 = 1;

/// 一个累加器在状态里的位置。
#[derive(Clone, Copy)]
enum Slot {
    Material(fn(&mut MaterialAccumulators) -> &mut f64),
    MaterialSoil,
    MaterialSnow,
    Water(fn(&mut WaterAccumulators) -> &mut f64),
    WaterSoil,
    WaterSnow,
}

/// `material_history_manifest` 的 44 项，顺序即上游写出顺序。
const FIELDS: [(&str, Slot); 44] = [
    ("a_mt_precip", Slot::Material(|a| &mut a.precip)),
    (
        "a_mt_vapor_exchange",
        Slot::Material(|a| &mut a.vapor_exchange),
    ),
    ("a_water_precip", Slot::Material(|a| &mut a.water_precip)),
    ("a_mt_evap", Slot::Material(|a| &mut a.evap)),
    (
        "a_water_evap_gross",
        Slot::Material(|a| &mut a.water_evap_gross),
    ),
    ("a_mt_transp", Slot::Material(|a| &mut a.transp)),
    ("a_mt_transp_src", Slot::Material(|a| &mut a.transp_src)),
    ("a_water_transp", Slot::Material(|a| &mut a.water_transp)),
    ("a_mt_soilevap", Slot::Material(|a| &mut a.soilevap)),
    (
        "a_water_soilevap",
        Slot::Material(|a| &mut a.water_soilevap),
    ),
    ("a_mt_canopyevap", Slot::Material(|a| &mut a.canopyevap)),
    (
        "a_water_canopyevap",
        Slot::Material(|a| &mut a.water_canopyevap),
    ),
    ("a_mt_subl", Slot::Material(|a| &mut a.subl)),
    ("a_water_subl", Slot::Material(|a| &mut a.water_subl)),
    ("a_mt_wetland_evap", Slot::Material(|a| &mut a.wetland_evap)),
    (
        "a_water_wetland_evap",
        Slot::Material(|a| &mut a.water_wetland_evap),
    ),
    ("a_mt_rsur", Slot::Material(|a| &mut a.rsur)),
    ("a_mt_rsub", Slot::Material(|a| &mut a.rsub)),
    ("a_mt_rnof", Slot::Material(|a| &mut a.rnof)),
    ("a_water_rnof", Slot::Material(|a| &mut a.water_rnof)),
    ("a_mt_qinfl", Slot::Material(|a| &mut a.qinfl)),
    ("a_mt_qcharge", Slot::Material(|a| &mut a.qcharge)),
    ("a_mt_ldew_mass", Slot::Material(|a| &mut a.ldew_mass)),
    ("a_water_ldew", Slot::Water(|w| &mut w.ldew)),
    ("a_mt_soil_mass", Slot::MaterialSoil),
    ("a_water_soil", Slot::WaterSoil),
    ("a_mt_snow_mass", Slot::MaterialSnow),
    ("a_water_snow", Slot::WaterSnow),
    ("a_mt_wa_mass", Slot::Material(|a| &mut a.wa_mass)),
    ("a_water_wa", Slot::Water(|w| &mut w.wa)),
    ("a_mt_wa_debt_mass", Slot::Material(|a| &mut a.wa_debt_mass)),
    ("a_water_wa_debt", Slot::Water(|w| &mut w.wa_debt)),
    (
        "a_water_aquifer_actual",
        Slot::Water(|w| &mut w.aquifer_actual),
    ),
    (
        "a_mt_aquifer_actual_mass",
        Slot::Material(|a| &mut a.aquifer_actual_mass),
    ),
    ("a_mt_wdsrf_mass", Slot::Material(|a| &mut a.wdsrf_mass)),
    ("a_water_wdsrf", Slot::Water(|w| &mut w.wdsrf)),
    ("a_mt_wetwat_mass", Slot::Material(|a| &mut a.wetwat_mass)),
    ("a_water_wetwat", Slot::Water(|w| &mut w.wetwat)),
    (
        "a_mt_surface_residue_mass",
        Slot::Material(|a| &mut a.surface_residue_mass),
    ),
    (
        "a_mt_subsurface_residue_mass",
        Slot::Material(|a| &mut a.subsurface_residue_mass),
    ),
    (
        "a_mt_layer_dry_mass",
        Slot::Material(|a| &mut a.layer_dry_mass),
    ),
    ("a_mt_solid_mass", Slot::Material(|a| &mut a.solid_mass)),
    ("a_mt_scv_mass", Slot::Material(|a| &mut a.scv_mass)),
    ("a_water_scv", Slot::Water(|w| &mut w.scv)),
];

/// 一个 patch 的材料累加器（逐材料 + 按 patch 的水量）。
#[derive(Debug, Clone, PartialEq)]
pub struct PatchAccumulators {
    pub materials: Vec<MaterialAccumulators>,
    pub water: WaterAccumulators,
}

impl PatchAccumulators {
    pub fn of(state: &PatchMaterialState) -> Self {
        Self {
            materials: state.acc.clone(),
            water: state.water_acc.clone(),
        }
    }

    /// 本 patch 一个清单项的值，按盘上顺序（层在外、材料在内）。
    fn values(&mut self, slot: Slot) -> Vec<f64> {
        match slot {
            Slot::Material(field) => self.materials.iter_mut().map(|a| *field(a)).collect(),
            Slot::MaterialSoil => (0..SOIL_LAYERS)
                .flat_map(|j| self.materials.iter().map(move |a| a.soil_mass[j]))
                .collect(),
            Slot::MaterialSnow => (0..MAX_SNOW_LAYERS)
                .flat_map(|j| self.materials.iter().map(move |a| a.snow_mass[j]))
                .collect(),
            Slot::Water(field) => vec![*field(&mut self.water)],
            Slot::WaterSoil => self.water.soil.to_vec(),
            Slot::WaterSnow => self.water.snow.to_vec(),
        }
    }

    fn set(&mut self, slot: Slot, values: &[f64]) {
        let n = self.materials.len();
        match slot {
            Slot::Material(field) => {
                for (a, &v) in self.materials.iter_mut().zip(values) {
                    *field(a) = v;
                }
            }
            Slot::MaterialSoil => {
                for j in 0..SOIL_LAYERS {
                    for (i, a) in self.materials.iter_mut().enumerate() {
                        a.soil_mass[j] = values[j * n + i];
                    }
                }
            }
            Slot::MaterialSnow => {
                for j in 0..MAX_SNOW_LAYERS {
                    for (i, a) in self.materials.iter_mut().enumerate() {
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
fn shape(name: &str, slot: Slot, nmaterials: usize) -> (Vec<(String, usize)>, usize) {
    let d1 = format!("d1_{name}");
    let d2 = format!("d2_{name}");
    let dims = match slot {
        Slot::Material(_) => vec![(d1, nmaterials)],
        Slot::MaterialSoil => vec![(d2, SOIL_LAYERS), (d1, nmaterials)],
        Slot::MaterialSnow => vec![(d2, MAX_SNOW_LAYERS), (d1, nmaterials)],
        Slot::Water(_) => Vec::new(),
        Slot::WaterSoil => vec![(d1, SOIL_LAYERS)],
        Slot::WaterSnow => vec![(d1, MAX_SNOW_LAYERS)],
    };
    let width = dims.iter().map(|(_, len)| len).product();
    (dims, width)
}

/// 旁车里材料部分要写的东西。`set` 有输运材料时写 44 个累加器（`material_history_required`，
/// `patches` 逐 patch），`methane` 是 CH4 provider 的累加量。
pub struct SidecarMaterials<'a> {
    pub set: Option<&'a MaterialSet>,
    pub patches: Vec<PatchAccumulators>,
    pub methane: Option<(&'a MethaneSetup, Vec<CoreAccumulator>)>,
}

/// 运行终点不在 history 自然边界上时，上游先把原始窗口存进旁车、再写最后一条 history 并清零
/// （`MOD_Hist.F90:270`）。材料与 CH4 的累加器在清零前留一份，供终点那份旁车使用。
#[derive(Debug, Clone, PartialEq)]
pub struct RawMaterials {
    pub material: Option<PatchAccumulators>,
    pub methane: Option<CoreAccumulator>,
}

/// history 会话与续跑写出共享的终点快照。
pub type RawMaterialsHandle = std::sync::Arc<std::sync::Mutex<Option<Vec<RawMaterials>>>>;

/// 区间跨过重启时把材料部分追加进刚写好的旁车（主机累加器之后、完成标记 `hacc_complete`
/// 置 1 之前 —— 上游 `write_sidecar` 里的 `material_history_write` 也是这个位置）。
pub fn write(path: &Path, materials: &SidecarMaterials<'_>) -> Result<()> {
    let mut file =
        netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
    if let Some(set) = materials.set {
        let identity: Vec<i32> = set
            .descriptor_identity_all()
            .into_iter()
            .flatten()
            .collect();
        let nmaterials = set.len();
        put_scalar_i32(&mut file, "mt_hist_schema", SCHEMA)?;
        put_scalar_i32(&mut file, "mt_hist_count", i32::try_from(nmaterials)?)?;
        ensure_dimension(
            &mut file,
            "mt_hist_descriptor_field",
            colm_core::material::DESCRIPTOR_IDENTITY_WIDTH,
        )?;
        ensure_dimension(&mut file, "mt_hist_descriptor_row", nmaterials)?;
        put_array_i32(
            &mut file,
            "mt_hist_descriptor",
            &["mt_hist_descriptor_row", "mt_hist_descriptor_field"],
            &identity,
        )?;
        let mut patches = materials.patches.clone();
        for (name, slot) in FIELDS {
            let (dims, _) = shape(name, slot, nmaterials);
            for (dim, len) in &dims {
                ensure_dimension(&mut file, dim, *len)?;
            }
            let mut names = vec!["patch"];
            names.extend(dims.iter().map(|(dim, _)| dim.as_str()));
            let values: Vec<f64> = patches.iter_mut().flat_map(|p| p.values(slot)).collect();
            put_array_f64(&mut file, name, &names, &values)?;
        }
    }
    if let Some((setup, accumulators)) = &materials.methane {
        let accumulators: Vec<&CoreAccumulator> = accumulators.iter().collect();
        crate::methane::write_accflux(&mut file, setup, &accumulators)?;
    }
    Ok(())
}

/// 读回的材料部分：没有对应内容时为 `None`。
#[derive(Debug, Default)]
pub struct RestoredMaterials {
    pub materials: Option<Vec<PatchAccumulators>>,
    pub methane: Option<Vec<CoreAccumulator>>,
}

/// 旁车区间开着（`hacc_nac > 0`，旧 Rust 旁车是 `history_nac`）时读回材料部分；
/// 旁车不存在或区间已关返回空。
/// 主机部分的标记校验由 [`crate::history_sidecar::read_sidecar`] 负责。
pub fn read(
    sidecar: &Path,
    set: Option<&MaterialSet>,
    methane: Option<&MethaneSetup>,
    patches: usize,
) -> Result<RestoredMaterials> {
    if !sidecar.is_file() {
        return Ok(RestoredMaterials::default());
    }
    let file =
        netcdf::open(sidecar).with_context(|| format!("cannot open {}", sidecar.display()))?;
    // 主机样本数标记：新格式 `hacc_nac`，旧 Rust 旁车 `history_nac`（本轮起只写新名）。
    let Some(nac) = file
        .variable("hacc_nac")
        .or_else(|| file.variable("history_nac"))
    else {
        return Ok(RestoredMaterials::default());
    };
    let nac = nac.get_values::<f64, _>(..)?;
    if !nac.first().is_some_and(|&n| n > 0.0) {
        return Ok(RestoredMaterials::default());
    }
    let mut restored = RestoredMaterials::default();
    if let Some(set) = set.filter(|set| set.transport_indices().next().is_some()) {
        // 上游 Fortran 写的旁车只有 `a_mt_*`/`a_water_*`，没有 Rust 的 `mt_hist_*`
        // 私有描述符；按上游盘上布局整组读回，让两引擎的 restart 能互相接续。
        if file.variable("mt_hist_schema").is_none() {
            if methane.is_some() {
                bail!(
                    "cross-engine material history sidecar with methane accumulators is not supported"
                );
            }
            if let Some(accumulators) = read_upstream_accumulators(&file, set, patches)? {
                restored.materials = Some(accumulators);
            } else if let Some(name) = FIELDS
                .iter()
                .map(|&(name, _)| name)
                .find(|name| read_name_opt(&file, name).is_none())
            {
                eprintln!(
                    "material history sidecar {} is missing upstream field {name}; \
                     starting the material history window from zero",
                    sidecar.display()
                );
            }
            return Ok(restored);
        }
        let nmaterials = set.len();
        // 逐字段解析读回名（`hacc_read_name`），缺任何一个都整组拒绝：这些累加和必须成套，
        // 半套读回来等于把一部分材料的区间悄悄截断。
        let read_names: Vec<(&str, std::borrow::Cow<'_, str>, Slot)> = FIELDS
            .iter()
            .map(|&(name, slot)| Ok((name, read_name(&file, name)?, slot)))
            .collect::<Result<_>>()?;
        check_descriptor(&file, set)?;
        let mut accumulators = vec![
            PatchAccumulators {
                materials: vec![MaterialAccumulators::default(); nmaterials],
                water: WaterAccumulators::default(),
            };
            patches
        ];
        for (name, stored, slot) in read_names {
            let (dims, width) = shape(name, slot, nmaterials);
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
                "non-finite generic land-material history accumulator ({stored})"
            );
            for (patch, target) in accumulators.iter_mut().enumerate() {
                target.set(slot, &values[patch * width..(patch + 1) * width]);
            }
        }
        restored.materials = Some(accumulators);
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
    if let Some(stored) = read_name_opt(file, name) {
        return Ok(stored);
    }
    match name.strip_prefix("a_mt_") {
        Some(rest) => bail!(
            "generic land-material history missing from active restart window ({name}, nor legacy \
             a_trc_{rest})"
        ),
        None => bail!("generic land-material history missing from active restart window ({name})"),
    }
}

fn read_name_opt<'a>(file: &netcdf::File, name: &'a str) -> Option<std::borrow::Cow<'a, str>> {
    if file.variable(name).is_some() {
        return Some(std::borrow::Cow::Borrowed(name));
    }
    name.strip_prefix("a_mt_")
        .map(|rest| format!("a_trc_{rest}"))
        .filter(|legacy| file.variable(legacy).is_some())
        .map(std::borrow::Cow::Owned)
}

/// 读上游 Fortran 布局的 44 个累加器（没有 Rust 的 `mt_hist_*` 描述符）。
/// 字段不齐或形状与 `(patch, d1/d2)` 不符时返回 `None`，调用方把窗口当冷启动。
fn read_upstream_accumulators(
    file: &netcdf::File,
    set: &MaterialSet,
    patches: usize,
) -> Result<Option<Vec<PatchAccumulators>>> {
    let nmaterials = set.len();
    let mut accumulators = vec![
        PatchAccumulators {
            materials: vec![MaterialAccumulators::default(); nmaterials],
            water: WaterAccumulators::default(),
        };
        patches
    ];
    for &(name, slot) in &FIELDS {
        let Some(stored) = read_name_opt(file, name) else {
            return Ok(None);
        };
        let (dims, width) = shape(name, slot, nmaterials);
        let variable = file
            .variable(stored.as_ref())
            .expect("presence checked by read_name_opt");
        let actual: Vec<usize> = variable.dimensions().iter().map(|dim| dim.len()).collect();
        let mut expected = vec![patches];
        expected.extend(dims.iter().map(|(_, len)| *len));
        if actual != expected {
            return Ok(None);
        }
        let values = variable.get_values::<f64, _>(..)?;
        ensure!(
            values.iter().all(|x| x.is_finite()),
            "non-finite generic land-material history accumulator ({stored})"
        );
        for (patch, target) in accumulators.iter_mut().enumerate() {
            target.set(slot, &values[patch * width..(patch + 1) * width]);
        }
    }
    Ok(Some(accumulators))
}

/// `material_history_descriptor(file, .false.)`：schema、材料数与逐行身份都要对上。
fn check_descriptor(file: &netcdf::File, set: &MaterialSet) -> Result<()> {
    let scalar = |name: &str| -> Result<i32> {
        let variable = file.variable(name).with_context(|| {
            format!("incompatible or incomplete generic land-material history sidecar: no {name}")
        })?;
        ensure!(
            variable.dimensions().is_empty(),
            "incompatible or incomplete generic land-material history sidecar: {name} is not a scalar"
        );
        Ok(variable.get_value::<i32, _>(())?)
    };
    let schema = scalar("mt_hist_schema")?;
    let count = scalar("mt_hist_count")?;
    if schema != SCHEMA || usize::try_from(count).ok() != Some(set.len()) {
        bail!("incompatible or incomplete generic land-material history sidecar");
    }
    let expected: Vec<i32> = set
        .descriptor_identity_all()
        .into_iter()
        .flatten()
        .collect();
    let variable = file
        .variable("mt_hist_descriptor")
        .context("incompatible or incomplete generic land-material history sidecar")?;
    let lens: Vec<usize> = variable.dimensions().iter().map(|dim| dim.len()).collect();
    ensure!(
        lens == [set.len(), colm_core::material::DESCRIPTOR_IDENTITY_WIDTH]
            && variable.get_values::<i32, _>(..)? == expected,
        "incompatible or incomplete generic land-material history sidecar"
    );
    Ok(())
}

#[cfg(test)]
#[path = "material_sidecar_tests.rs"]
mod tests;
