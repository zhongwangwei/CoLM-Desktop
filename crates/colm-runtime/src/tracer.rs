//! 示踪物的运行时部分：namelist 与参数文件 → [`TracerSet`]，陆面示踪物重启的读写
//! （`MOD_Tracer_Rest`）。

use std::path::Path;

use anyhow::{Context, Result};
use colm_core::tracer::{
    PatchTracerState, TracerNamelist, TracerParameterOverrides, TracerPools, TracerSet,
    DESCRIPTOR_IDENTITY_WIDTH, SOISNO_LAYERS,
};
use colm_namelist::{Document, Value};

use crate::physics::{integer, logical, real, text};

/// `LAND_TRACER_RESTART_SCHEMA_VERSION`。
const LAND_TRACER_RESTART_SCHEMA: i32 = 5;
/// `TRC_FORC_CACHE_SCHEMA`。
const FORCING_CACHE_SCHEMA: i32 = 1;

/// 算例开了 `DEF_USE_TRACER` 时注册示踪物（`tracer_defs_init`）；关着返回 `None`。
pub fn tracer_set_from_document(document: &Document) -> Result<Option<TracerSet>> {
    if !logical(document, "DEF_USE_TRACER")? {
        return Ok(None);
    }
    let namelist = TracerNamelist {
        num: integer(document, "DEF_TRACER_NUM")?,
        names: text(document, "DEF_TRACER_NAMES")?,
        types: text(document, "DEF_TRACER_TYPES")?,
        mrat: text(document, "DEF_TRACER_MRAT")?,
        ref_ratio: text(document, "DEF_TRACER_REF_RATIO")?,
        init_delta: text(document, "DEF_TRACER_INIT_DELTA")?,
        reactive_decay_rate: text(document, "DEF_TRACER_REACTIVE_DECAY_RATE")?,
        param_files: text(document, "DEF_TRACER_PARAM_FILES")?,
        use_bgc: logical(document, "DEF_USE_BGC")?,
        variably_saturated_flow: logical(document, "DEF_USE_VariablySaturatedFlow")?,
        aquifer_mixing_water_mm: real(document, "DEF_TRACER_AQUIFER_MIXING_WATER_MM")?,
    };
    TracerSet::build(&namelist, read_tracer_parameter_file).map(Some)
}

/// `read_tracer_parameter_file`：文件必须存在；没有 `&nl_colm_tracer_parameter` 组时
/// 什么都不改（`tracer_parameter_group_present`）。
fn read_tracer_parameter_file(path: &str) -> Result<Option<TracerParameterOverrides>> {
    let source = std::fs::read_to_string(path).with_context(|| {
        format!("read_tracer_parameter_file: missing tracer parameter file: {path}")
    })?;
    let present = source.lines().any(|line| {
        let line = line.trim_start().to_ascii_lowercase();
        !line.starts_with('!')
            && (line.starts_with("&nl_colm_tracer_parameter")
                || line.starts_with("$nl_colm_tracer_parameter"))
    });
    if !present {
        return Ok(None);
    }
    let document = colm_namelist::parse(&source)
        .with_context(|| format!("invalid &nl_colm_tracer_parameter in {path}"))?;
    let real_field = |name: &str| -> Result<Option<f64>> {
        match document.get(&format!("DEF_TRACER%{name}")) {
            None => Ok(None),
            Some(value) => value
                .as_f64()
                .map(Some)
                .with_context(|| format!("DEF_TRACER%{name} in {path} is not a real")),
        }
    };
    let charge = match document.get("DEF_TRACER%charge") {
        None => None,
        Some(Value::Int(value)) => Some(i32::try_from(*value)?),
        Some(other) => {
            anyhow::bail!("DEF_TRACER%charge in {path} must be an integer, got {other:?}")
        }
    };
    let unit_kind = match document.get("DEF_TRACER%unit_kind") {
        None => None,
        Some(Value::Str(value)) => Some(value.clone()),
        Some(other) => {
            anyhow::bail!("DEF_TRACER%unit_kind in {path} must be a string, got {other:?}")
        }
    };
    Ok(Some(TracerParameterOverrides {
        unit_kind,
        mol_weight: real_field("mol_weight")?,
        ref_ratio: real_field("ref_ratio")?,
        init_delta: real_field("init_delta")?,
        init_conc: real_field("init_conc")?,
        precip_default_conc: real_field("precip_default_conc")?,
        vapor_default_conc: real_field("vapor_default_conc")?,
        max_dissolved_conc: real_field("max_dissolved_conc")?,
        reactive_decay_rate: real_field("reactive_decay_rate")?,
        charge,
    }))
}

/// 每个输运示踪物逐 patch 的一个量（重启里的 `(patch, trc_land_transport)`）。
type PatchField = fn(&TracerPools) -> f64;
/// 逐层的量（`(patch, soilsnow, trc_land_transport)`）。
type LayerField = fn(&TracerPools) -> &[f64; SOISNO_LAYERS];

/// 上游 `write_land_tracer_restart` 的写出顺序（`trc_wa` 之后插 `trc_aquifer_ref_*`）。
const CANOPY_FIELDS: [(&str, PatchField); 2] = [
    ("trc_ldew_rain", |p| p.ldew_rain),
    ("trc_ldew_snow", |p| p.ldew_snow),
];
const LAYER_FIELDS: [(&str, LayerField); 3] = [
    ("trc_wliq_soisno", |p| &p.wliq_soisno),
    ("trc_wice_soisno", |p| &p.wice_soisno),
    ("trc_solid_soisno", |p| &p.solid_soisno),
];
const TAIL_FIELDS: [(&str, PatchField); 15] = [
    ("trc_wdsrf", |p| p.wdsrf),
    ("trc_wetwat", |p| p.wetwat),
    ("trc_surface_residue", |p| p.surface_residue),
    ("trc_subsurface_residue", |p| p.subsurface_residue),
    ("trc_canopy_solid", |p| p.canopy_solid),
    ("trc_surface_solid", |p| p.surface_solid),
    ("trc_subsurface_solid", |p| p.subsurface_solid),
    ("trc_waterstorage_solid", |p| p.waterstorage_solid),
    ("trc_scv", |p| p.scv),
    ("trc_waterstorage", |p| p.waterstorage),
    ("trc_leaf_delta_e", |p| p.leaf_delta_e),
    ("trc_leaf_delta_b", |p| p.leaf_delta_b),
    ("trc_leaf_peclet", |p| p.leaf_peclet),
    ("trc_leaf_water_moles", |p| p.leaf_water_moles),
    ("trc_leaf_iso_storage", |p| p.leaf_iso_storage),
];

/// `write_land_tracer_restart`（含 `tracer_forcing_write_restart` 的计数）：把示踪物
/// 预报量追加进一个已写好的陆面时间重启（`patch`/`soilsnow` 维已在）。没有输运示踪物时
/// 只写空事务（见 `colm_init::write_empty_land_tracer_transaction`）。
pub fn write_land_tracer_restart(
    path: &Path,
    set: &TracerSet,
    states: &[&PatchTracerState],
    aquifer_mixing_water_mm: f64,
) -> Result<()> {
    let transport: Vec<usize> = set.transport_indices().collect();
    if transport.is_empty() {
        colm_init::write_empty_land_tracer_transaction(path, aquifer_mixing_water_mm)?;
    } else {
        let mut file =
            netcdf::append(path).with_context(|| format!("cannot reopen {}", path.display()))?;
        let ntransport = transport.len();
        put_scalar_i32(&mut file, "trc_land_restart_complete", 0)?;
        put_scalar_i32(
            &mut file,
            "trc_land_restart_schema",
            LAND_TRACER_RESTART_SCHEMA,
        )?;
        put_scalar_i32(
            &mut file,
            "trc_land_transport_count",
            i32::try_from(ntransport)?,
        )?;
        put_scalar_f64(
            &mut file,
            "trc_aquifer_mixing_water_mm",
            aquifer_mixing_water_mm,
        )?;
        ensure_dimension(
            &mut file,
            "trc_land_descriptor_field",
            DESCRIPTOR_IDENTITY_WIDTH,
        )?;
        ensure_dimension(&mut file, "trc_land_transport", ntransport)?;
        let identity: Vec<i32> = set.descriptor_identity().into_iter().flatten().collect();
        put_array_i32(
            &mut file,
            "trc_land_descriptor_identity",
            &["trc_land_transport", "trc_land_descriptor_field"],
            &identity,
        )?;
        let patch_field = |field: PatchField| -> Vec<f64> {
            states
                .iter()
                .flat_map(|state| transport.iter().map(move |&itrc| field(&state.pools[itrc])))
                .collect()
        };
        for (name, field) in &CANOPY_FIELDS {
            put_array_f64(
                &mut file,
                name,
                &["patch", "trc_land_transport"],
                &patch_field(*field),
            )?;
        }
        for (name, field) in LAYER_FIELDS {
            let mut values = Vec::with_capacity(states.len() * SOISNO_LAYERS * transport.len());
            for state in states {
                for slot in 0..SOISNO_LAYERS {
                    for &itrc in &transport {
                        values.push(field(&state.pools[itrc])[slot]);
                    }
                }
            }
            put_array_f64(
                &mut file,
                name,
                &["patch", "soilsnow", "trc_land_transport"],
                &values,
            )?;
        }
        put_array_f64(
            &mut file,
            "trc_wa",
            &["patch", "trc_land_transport"],
            &patch_field(|p| p.wa),
        )?;
        let reference: Vec<f64> = states.iter().map(|state| state.aquifer_ref_water).collect();
        put_array_f64(&mut file, "trc_aquifer_ref_water", &["patch"], &reference)?;
        put_array_f64(
            &mut file,
            "trc_aquifer_ref_mass",
            &["patch", "trc_land_transport"],
            &patch_field(|p| p.aquifer_ref_mass),
        )?;
        for (name, field) in &TAIL_FIELDS {
            put_array_f64(
                &mut file,
                name,
                &["patch", "trc_land_transport"],
                &patch_field(*field),
            )?;
        }
        put_scalar_i32(&mut file, "trc_forcing_cache_schema", FORCING_CACHE_SCHEMA)?;
        put_scalar_i32(&mut file, "trc_forcing_cache_count", 0)?;
        put_scalar_i32(&mut file, "trc_land_restart_complete", 1)?;
    }
    Ok(())
}

/// `read_land_tracer_restart` 的判定与读入：已提交（`complete = 1`）、schema 5、
/// 输运示踪物个数与描述符指纹都一致才读；否则返回 `None`，由调用方从水量冷启动
/// （上游打印 "Generic land tracer restart is legacy/incompatible"）。
pub fn read_land_tracer_restart(
    restart: &colm_init::RestartFile,
    set: &TracerSet,
    patches: usize,
) -> Result<Option<Vec<PatchTracerState>>> {
    let transport: Vec<usize> = set.transport_indices().collect();
    let scalar = |name: &str| -> Option<i64> {
        restart
            .integers(name)
            .ok()
            .and_then(|values| values.first().copied())
    };
    let identity_matches = || -> bool {
        let Ok(stored) = restart.integers("trc_land_descriptor_identity") else {
            return false;
        };
        let expected: Vec<i64> = set
            .descriptor_identity()
            .into_iter()
            .flatten()
            .map(i64::from)
            .collect();
        stored == expected.as_slice()
    };
    let matches = scalar("trc_land_restart_complete") == Some(1)
        && scalar("trc_land_restart_schema") == Some(i64::from(LAND_TRACER_RESTART_SCHEMA))
        && scalar("trc_land_transport_count") == Some(transport.len() as i64)
        && (transport.is_empty() || identity_matches());
    if !matches {
        return Ok(None);
    }
    let mut states: Vec<PatchTracerState> = (0..patches)
        .map(|_| PatchTracerState::allocated(set))
        .collect();
    if transport.is_empty() {
        return Ok(Some(states));
    }
    let n = transport.len();
    let read_patch = |name: &str,
                      states: &mut [PatchTracerState],
                      set_value: fn(&mut TracerPools, f64)|
     -> Result<()> {
        let values = restart.floats(name)?;
        anyhow::ensure!(values.len() == patches * n, "{name} has an unexpected size");
        for (patch, state) in states.iter_mut().enumerate() {
            for (k, &itrc) in transport.iter().enumerate() {
                set_value(&mut state.pools[itrc], values[patch * n + k]);
            }
        }
        Ok(())
    };
    let setters: [(&str, fn(&mut TracerPools, f64)); 19] = [
        ("trc_ldew_rain", |p, v| p.ldew_rain = v),
        ("trc_ldew_snow", |p, v| p.ldew_snow = v),
        ("trc_wa", |p, v| p.wa = v),
        ("trc_aquifer_ref_mass", |p, v| p.aquifer_ref_mass = v),
        ("trc_wdsrf", |p, v| p.wdsrf = v),
        ("trc_wetwat", |p, v| p.wetwat = v),
        ("trc_surface_residue", |p, v| p.surface_residue = v),
        ("trc_subsurface_residue", |p, v| p.subsurface_residue = v),
        ("trc_canopy_solid", |p, v| p.canopy_solid = v),
        ("trc_surface_solid", |p, v| p.surface_solid = v),
        ("trc_subsurface_solid", |p, v| p.subsurface_solid = v),
        ("trc_waterstorage_solid", |p, v| p.waterstorage_solid = v),
        ("trc_scv", |p, v| p.scv = v),
        ("trc_waterstorage", |p, v| p.waterstorage = v),
        ("trc_leaf_delta_e", |p, v| p.leaf_delta_e = v),
        ("trc_leaf_delta_b", |p, v| p.leaf_delta_b = v),
        ("trc_leaf_peclet", |p, v| p.leaf_peclet = v),
        ("trc_leaf_water_moles", |p, v| p.leaf_water_moles = v),
        ("trc_leaf_iso_storage", |p, v| p.leaf_iso_storage = v),
    ];
    for (name, setter) in setters {
        read_patch(name, &mut states, setter)?;
    }
    let layer_setters: [(&str, fn(&mut TracerPools) -> &mut [f64; SOISNO_LAYERS]); 3] = [
        ("trc_wliq_soisno", |p| &mut p.wliq_soisno),
        ("trc_wice_soisno", |p| &mut p.wice_soisno),
        ("trc_solid_soisno", |p| &mut p.solid_soisno),
    ];
    for (name, layers) in layer_setters {
        let values = restart.floats(name)?;
        anyhow::ensure!(
            values.len() == patches * SOISNO_LAYERS * n,
            "{name} has an unexpected size"
        );
        for (patch, state) in states.iter_mut().enumerate() {
            for slot in 0..SOISNO_LAYERS {
                for (k, &itrc) in transport.iter().enumerate() {
                    layers(&mut state.pools[itrc])[slot] =
                        values[(patch * SOISNO_LAYERS + slot) * n + k];
                }
            }
        }
    }
    let reference = restart.floats("trc_aquifer_ref_water")?;
    anyhow::ensure!(
        reference.len() == patches,
        "trc_aquifer_ref_water has an unexpected size"
    );
    for (state, &value) in states.iter_mut().zip(reference) {
        state.aquifer_ref_water = value;
    }
    Ok(Some(states))
}

fn ensure_dimension(file: &mut netcdf::FileMut, name: &str, len: usize) -> Result<()> {
    match file.dimension(name) {
        Some(dimension) => {
            anyhow::ensure!(
                dimension.len() == len,
                "{name} already has {} entries, expected {len}",
                dimension.len()
            );
        }
        None => {
            file.add_dimension(name, len)?;
        }
    }
    Ok(())
}

fn put_scalar_i32(file: &mut netcdf::FileMut, name: &str, value: i32) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<i32>(name, &[])?,
    };
    variable.put_value(value, ())?;
    Ok(())
}

fn put_scalar_f64(file: &mut netcdf::FileMut, name: &str, value: f64) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<f64>(name, &[])?,
    };
    variable.put_value(value, ())?;
    Ok(())
}

fn put_array_f64(
    file: &mut netcdf::FileMut,
    name: &str,
    dims: &[&str],
    values: &[f64],
) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<f64>(name, dims)?,
    };
    variable.put_values(values, netcdf::Extents::All)?;
    Ok(())
}

fn put_array_i32(
    file: &mut netcdf::FileMut,
    name: &str,
    dims: &[&str],
    values: &[i32],
) -> Result<()> {
    let mut variable = match file.variable_mut(name) {
        Some(variable) => variable,
        None => file.add_variable::<i32>(name, dims)?,
    };
    variable.put_values(values, netcdf::Extents::All)?;
    Ok(())
}

#[cfg(test)]
#[path = "tracer_tests.rs"]
mod tests;
