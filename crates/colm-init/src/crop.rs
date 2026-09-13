//! Shared CROP cold-start state.
//!
//! The state is owned here and borrowed by both restart writers, so a future
//! Rust runtime uses the same initialization contract rather than duplicating
//! CROP setup from `mkinidata`.

use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::CropPhenologyState;

use crate::{
    runtime::nearest_cell_indices, BgcCropFields, IrrigationFields, PftCropFields, MISSING,
};

const CFT_FIRST: i32 = 15;
const CFT_LAST: i32 = 78;
const CROP_IDOP_INITIAL: i32 = 99_999_999;
const CROP_MANAGEMENT_MISSING: f64 = -99_999_999.0;
const CROP_MANAGEMENT_MISSING_I32: i32 = -99_999_999;
const CROP_DIR: &str = "crop";
const PLANTING_FILE: &str = "plantdt-colm-64cfts-rice2_fillcoast.nc";
const FERTILIZER_SOURCE_ONE_FILE: &str = "fertnitro_fillcoast.nc";
const FERTILIZER_SOURCE_TWO_FILE: &str = "fertilizer_2015soc.nc";
const IRRIGATION_FILE: &str = "surfdata_irrigation_method_96x144.nc";

/// Runtime choices consumed by upstream `CROP_readin`.
#[derive(Debug, Clone, Copy)]
pub struct CropManagementConfig<'a> {
    pub runtime_dir: &'a Path,
    /// Positive values replace raster planting days, as in CoLM.
    pub planting_day_override: Option<f64>,
    pub use_fertilizer: bool,
    /// CoLM's `DEF_FERT_SOURCE`; consulted only when fertilizer is enabled.
    pub fertilizer_source: i32,
    pub use_irrigation: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CropColdStartState {
    crop_live: Vec<i8>,
    heat_unit_index: Vec<f64>,
    growing_degree_days_at_planting: Vec<f64>,
    peak_lai_day: Vec<i32>,
    root_allocation: Vec<f64>,
    stem_allocation: Vec<f64>,
    reproductive_allocation: Vec<f64>,
    leaf_allocation: Vec<f64>,
    stem_allocation_increment: Vec<f64>,
    leaf_allocation_increment: Vec<f64>,
    growing_degree_days_at_maturity: Vec<f64>,
    crop_planted: Vec<i8>,
    day_of_planting: Vec<i32>,
    five_day_minimum_temperature: Vec<f64>,
    ten_day_minimum_temperature: Vec<f64>,
    ten_day_temperature: Vec<f64>,
    cumulative_vernalization_days: Vec<f64>,
    vernalization_factor: Vec<f64>,
    crop_phase: Vec<f64>,
    fertilizer_counter: Vec<f64>,
    minimum_reference_temperature: Vec<f64>,
    maximum_reference_temperature: Vec<f64>,
    instantaneous_minimum_reference_temperature: Vec<f64>,
    instantaneous_maximum_reference_temperature: Vec<f64>,
    fertilizer_nitrogen: Vec<f64>,
    manure_nitrogen: Vec<f64>,
    fertilizer: Vec<f64>,
    latitude_base_temperature: Vec<f64>,
    planting_date: Vec<f64>,
    irrigation_method: Option<Vec<i32>>,
    irrigation_rate: Vec<f64>,
    irrigation_cumulative: Vec<f64>,
    irrigation_cumulative_deficit: Vec<f64>,
    irrigation_event_count: Vec<f64>,
    irrigation_steps_left: Vec<i32>,
    irrigation_water_storage: Vec<f64>,
    irrigation_method_corn: Vec<i32>,
    irrigation_method_spring_wheat: Vec<i32>,
    irrigation_method_winter_wheat: Vec<i32>,
    irrigation_method_soybean: Vec<i32>,
    irrigation_method_cotton: Vec<i32>,
    irrigation_method_rice1: Vec<i32>,
    irrigation_method_rice2: Vec<i32>,
    irrigation_method_sugarcane: Vec<i32>,
    irrigation_groundwater_allocation: Vec<f64>,
    irrigation_surface_water_allocation: Vec<f64>,
    planting_day_corn: Vec<f64>,
    planting_day_spring_wheat: Vec<f64>,
    planting_day_winter_wheat: Vec<f64>,
    planting_day_soybean: Vec<f64>,
    planting_day_cotton: Vec<f64>,
    planting_day_rice1: Vec<f64>,
    planting_day_rice2: Vec<f64>,
    planting_day_sugarcane: Vec<f64>,
    fertilizer_nitrogen_corn: Vec<f64>,
    fertilizer_nitrogen_spring_wheat: Vec<f64>,
    fertilizer_nitrogen_winter_wheat: Vec<f64>,
    fertilizer_nitrogen_soybean: Vec<f64>,
    fertilizer_nitrogen_cotton: Vec<f64>,
    fertilizer_nitrogen_rice1: Vec<f64>,
    fertilizer_nitrogen_rice2: Vec<f64>,
    fertilizer_nitrogen_sugarcane: Vec<f64>,
}

/// Mirrors `CROP_readin`'s explicit-planting-day branch, which intentionally
/// avoids global management rasters when fertilization and irrigation are off.
pub fn crop_cold_start_from_tuning(
    classes: &[i32],
    crop_fraction: &[f64],
    planting_day: f64,
) -> Result<CropColdStartState> {
    validate_crop_tiles(classes, crop_fraction)?;
    ensure!(
        planting_day.is_finite() && planting_day > 0.0,
        "DEF_TUNING_CROP_PLANTING_DAY must be finite and positive"
    );
    let mut state = empty_crop_state(classes.len(), crop_fraction.len());
    state.planting_date.fill(planting_day);
    state.planting_day_rice2.fill(0.0);
    Ok(state)
}

/// Mirrors the normal raster-reading path in upstream `CROP_readin` for one
/// point.  The spatial lookup is intentionally shared with other runtime maps.
pub fn crop_cold_start_from_management(
    classes: &[i32],
    crop_fraction: &[f64],
    latitude_degrees: f64,
    longitude_degrees: f64,
    config: CropManagementConfig<'_>,
) -> Result<CropColdStartState> {
    validate_crop_tiles(classes, crop_fraction)?;
    if let Some(day) = config.planting_day_override {
        ensure!(
            day.is_finite() && day > 0.0,
            "DEF_TUNING_CROP_PLANTING_DAY must be finite and positive when set"
        );
    }

    let mut state = empty_crop_state(classes.len(), crop_fraction.len());
    let crop_dir = config.runtime_dir.join(CROP_DIR);
    let planting = open_map(crop_dir.join(PLANTING_FILE), "CROP planting-date")?;
    let planting_cell = nearest_cell_indices(&planting, latitude_degrees, longitude_degrees)?;
    let rice2 = sample_f64_2d(&planting, "pdrice2", planting_cell)?;
    state
        .planting_day_rice2
        .fill(truncated_or(rice2, 0, "pdrice2")? as f64);
    for (index, &class) in classes.iter().enumerate() {
        let name = format!("PLANTDATE_CFT_{class:02}");
        state.planting_date[index] = sample_f64_2d(&planting, &name, planting_cell)?
            .filter(|value| *value > 0.0)
            .unwrap_or(CROP_MANAGEMENT_MISSING);
    }
    if let Some(day) = config.planting_day_override {
        state.planting_date.fill(day);
    }

    if config.use_fertilizer {
        match config.fertilizer_source {
            1 => read_fertilizer_source_one(
                &mut state,
                classes,
                crop_dir.join(FERTILIZER_SOURCE_ONE_FILE),
                latitude_degrees,
                longitude_degrees,
            )?,
            2 => read_fertilizer_source_two(
                &mut state,
                classes,
                crop_dir.join(FERTILIZER_SOURCE_TWO_FILE),
                latitude_degrees,
                longitude_degrees,
            )?,
            source => bail!("DEF_FERT_SOURCE must be 1 or 2, got {source}"),
        }
    }
    if config.use_irrigation {
        state.irrigation_method = Some(read_irrigation_methods(
            classes,
            crop_dir.join(IRRIGATION_FILE),
            latitude_degrees,
            longitude_degrees,
        )?);
    }
    state.set_patch_fertilizer(classes);
    state.set_patch_irrigation(classes);
    Ok(state)
}

impl CropColdStartState {
    /// Builds the shared Rust runtime state from this cold-start record.
    ///
    /// Restart serialization remains an adapter concern; the executable
    /// receives the same state object that owns CropPhenology transitions.
    pub fn cold_runtime_phenology_state(&self) -> CropPhenologyState {
        let mut state = CropPhenologyState::new(self.crop_live.len());
        state.crop_live = self.crop_live.iter().map(|value| *value != 0).collect();
        state.crop_planted = self.crop_planted.iter().map(|value| *value != 0).collect();
        state.heat_unit_index.clone_from(&self.heat_unit_index);
        state
            .growing_degree_days_at_maturity_c
            .clone_from(&self.growing_degree_days_at_maturity);
        state.planting_day.clone_from(&self.planting_date);
        state.day_of_planting.clone_from(&self.day_of_planting);
        state
            .cumulative_vernalization_days
            .clone_from(&self.cumulative_vernalization_days);
        state
            .vernalization_factor
            .clone_from(&self.vernalization_factor);
        state.crop_phase.clone_from(&self.crop_phase);
        state
            .fertilizer_counter_seconds
            .clone_from(&self.fertilizer_counter);
        state
            .fertilizer_nitrogen_g_m2
            .clone_from(&self.fertilizer_nitrogen);
        state.manure_nitrogen_g_m2.clone_from(&self.manure_nitrogen);
        state.fertilizer_rate_g_m2_s.clone_from(&self.fertilizer);
        state
    }

    pub fn pft_fields(&self) -> PftCropFields<'_> {
        PftCropFields {
            crop_live: &self.crop_live,
            heat_unit_index: &self.heat_unit_index,
            growing_degree_days_at_planting: &self.growing_degree_days_at_planting,
            peak_lai_day: &self.peak_lai_day,
            root_allocation: &self.root_allocation,
            stem_allocation: &self.stem_allocation,
            reproductive_allocation: &self.reproductive_allocation,
            leaf_allocation: &self.leaf_allocation,
            stem_allocation_increment: &self.stem_allocation_increment,
            leaf_allocation_increment: &self.leaf_allocation_increment,
            growing_degree_days_at_maturity: &self.growing_degree_days_at_maturity,
            crop_planted: &self.crop_planted,
            day_of_planting: &self.day_of_planting,
            five_day_minimum_temperature: &self.five_day_minimum_temperature,
            ten_day_minimum_temperature: &self.ten_day_minimum_temperature,
            ten_day_temperature: &self.ten_day_temperature,
            cumulative_vernalization_days: &self.cumulative_vernalization_days,
            vernalization_factor: &self.vernalization_factor,
            crop_phase: &self.crop_phase,
            fertilizer_counter: &self.fertilizer_counter,
            minimum_reference_temperature: &self.minimum_reference_temperature,
            maximum_reference_temperature: &self.maximum_reference_temperature,
            instantaneous_minimum_reference_temperature: &self
                .instantaneous_minimum_reference_temperature,
            instantaneous_maximum_reference_temperature: &self
                .instantaneous_maximum_reference_temperature,
            fertilizer_nitrogen: &self.fertilizer_nitrogen,
            manure_nitrogen: &self.manure_nitrogen,
            fertilizer: &self.fertilizer,
            latitude_base_temperature: &self.latitude_base_temperature,
            planting_date: &self.planting_date,
        }
    }

    pub fn bgc_fields(&self) -> BgcCropFields<'_> {
        BgcCropFields {
            crop_phase: &self.crop_phase,
            planting_day_corn: &self.planting_day_corn,
            planting_day_spring_wheat: &self.planting_day_spring_wheat,
            planting_day_winter_wheat: &self.planting_day_winter_wheat,
            planting_day_soybean: &self.planting_day_soybean,
            planting_day_cotton: &self.planting_day_cotton,
            planting_day_rice1: &self.planting_day_rice1,
            planting_day_rice2: &self.planting_day_rice2,
            planting_day_sugarcane: &self.planting_day_sugarcane,
            fertilizer_nitrogen_corn: &self.fertilizer_nitrogen_corn,
            fertilizer_nitrogen_spring_wheat: &self.fertilizer_nitrogen_spring_wheat,
            fertilizer_nitrogen_winter_wheat: &self.fertilizer_nitrogen_winter_wheat,
            fertilizer_nitrogen_soybean: &self.fertilizer_nitrogen_soybean,
            fertilizer_nitrogen_cotton: &self.fertilizer_nitrogen_cotton,
            fertilizer_nitrogen_rice1: &self.fertilizer_nitrogen_rice1,
            fertilizer_nitrogen_rice2: &self.fertilizer_nitrogen_rice2,
            fertilizer_nitrogen_sugarcane: &self.fertilizer_nitrogen_sugarcane,
        }
    }

    pub fn irrigation_method(&self) -> Option<&[i32]> {
        self.irrigation_method.as_deref()
    }

    pub fn irrigation_fields<'a>(
        &'a self,
        standard_water_table_depth: &'a [f64],
    ) -> Option<IrrigationFields<'a>> {
        self.irrigation_method.as_ref()?;
        Some(IrrigationFields {
            rate: &self.irrigation_rate,
            cumulative: &self.irrigation_cumulative,
            cumulative_deficit: &self.irrigation_cumulative_deficit,
            event_count: &self.irrigation_event_count,
            steps_left: &self.irrigation_steps_left,
            water_storage: &self.irrigation_water_storage,
            corn_method: &self.irrigation_method_corn,
            spring_wheat_method: &self.irrigation_method_spring_wheat,
            winter_wheat_method: &self.irrigation_method_winter_wheat,
            soybean_method: &self.irrigation_method_soybean,
            cotton_method: &self.irrigation_method_cotton,
            rice_1_method: &self.irrigation_method_rice1,
            rice_2_method: &self.irrigation_method_rice2,
            sugarcane_method: &self.irrigation_method_sugarcane,
            groundwater_allocation: &self.irrigation_groundwater_allocation,
            surface_water_allocation: &self.irrigation_surface_water_allocation,
            standard_water_table_depth,
        })
    }

    fn set_patch_fertilizer(&mut self, classes: &[i32]) {
        for (index, &class) in classes.iter().enumerate() {
            let fertilizer = self.fertilizer_nitrogen[index];
            match class {
                17 | 18 | 63 | 64 => self.fertilizer_nitrogen_corn[index] = fertilizer,
                19 | 20 => self.fertilizer_nitrogen_spring_wheat[index] = fertilizer,
                21 | 22 => self.fertilizer_nitrogen_winter_wheat[index] = fertilizer,
                23 | 24 | 77 | 78 => self.fertilizer_nitrogen_soybean[index] = fertilizer,
                41 | 42 => self.fertilizer_nitrogen_cotton[index] = fertilizer,
                61 | 62 => {
                    self.fertilizer_nitrogen_rice1[index] = fertilizer;
                    self.fertilizer_nitrogen_rice2[index] = fertilizer;
                }
                67 | 68 => self.fertilizer_nitrogen_sugarcane[index] = fertilizer,
                _ => {}
            }
        }
    }

    fn set_patch_irrigation(&mut self, classes: &[i32]) {
        let Some(methods) = self.irrigation_method.as_deref() else {
            return;
        };
        for (index, &class) in classes.iter().enumerate() {
            let method = methods[index];
            match class {
                17 | 18 | 63 | 64 => self.irrigation_method_corn[index] = method,
                19 | 20 => self.irrigation_method_spring_wheat[index] = method,
                21 | 22 => self.irrigation_method_winter_wheat[index] = method,
                23 | 24 | 77 | 78 => self.irrigation_method_soybean[index] = method,
                41 | 42 => self.irrigation_method_cotton[index] = method,
                61 | 62 => {
                    self.irrigation_method_rice1[index] = method;
                    self.irrigation_method_rice2[index] = method;
                }
                67 | 68 => self.irrigation_method_sugarcane[index] = method,
                _ => {}
            }
        }
    }
}

fn validate_crop_tiles(classes: &[i32], crop_fraction: &[f64]) -> Result<()> {
    ensure!(
        !classes.is_empty() && classes.len() == crop_fraction.len(),
        "CROP classes and crop fractions must be nonempty equal-length vectors"
    );
    ensure!(
        classes
            .iter()
            .all(|class| (CFT_FIRST..=CFT_LAST).contains(class)),
        "CROP classes must be in {CFT_FIRST}..={CFT_LAST}"
    );
    ensure!(
        crop_fraction
            .iter()
            .all(|fraction| fraction.is_finite() && *fraction > 0.0),
        "CROP fractions must be finite and positive"
    );
    Ok(())
}

fn empty_crop_state(pfts: usize, patches: usize) -> CropColdStartState {
    let missing = vec![MISSING; pfts];
    let patch_missing = vec![MISSING; patches];
    CropColdStartState {
        crop_live: vec![0; pfts],
        heat_unit_index: missing.clone(),
        growing_degree_days_at_planting: missing.clone(),
        peak_lai_day: vec![0; pfts],
        root_allocation: missing.clone(),
        stem_allocation: missing.clone(),
        reproductive_allocation: missing.clone(),
        leaf_allocation: missing.clone(),
        stem_allocation_increment: missing.clone(),
        leaf_allocation_increment: missing.clone(),
        growing_degree_days_at_maturity: missing.clone(),
        crop_planted: vec![0; pfts],
        day_of_planting: vec![CROP_IDOP_INITIAL; pfts],
        five_day_minimum_temperature: missing.clone(),
        ten_day_minimum_temperature: missing.clone(),
        ten_day_temperature: missing.clone(),
        cumulative_vernalization_days: missing.clone(),
        vernalization_factor: vec![0.0; pfts],
        crop_phase: vec![4.0; pfts],
        fertilizer_counter: vec![0.0; pfts],
        minimum_reference_temperature: vec![273.15; pfts],
        maximum_reference_temperature: vec![273.15; pfts],
        instantaneous_minimum_reference_temperature: missing.clone(),
        instantaneous_maximum_reference_temperature: missing.clone(),
        fertilizer_nitrogen: vec![0.0; pfts],
        manure_nitrogen: vec![0.0; pfts],
        fertilizer: vec![0.0; pfts],
        latitude_base_temperature: missing,
        planting_date: vec![CROP_MANAGEMENT_MISSING; pfts],
        irrigation_method: None,
        irrigation_rate: vec![MISSING; patches],
        irrigation_cumulative: vec![0.0; patches],
        irrigation_cumulative_deficit: vec![0.0; patches],
        irrigation_event_count: vec![0.0; patches],
        irrigation_steps_left: vec![-9_999; patches],
        irrigation_water_storage: vec![0.0; patches],
        irrigation_method_corn: vec![-9_999; patches],
        irrigation_method_spring_wheat: vec![-9_999; patches],
        irrigation_method_winter_wheat: vec![-9_999; patches],
        irrigation_method_soybean: vec![-9_999; patches],
        irrigation_method_cotton: vec![-9_999; patches],
        irrigation_method_rice1: vec![-9_999; patches],
        irrigation_method_rice2: vec![-9_999; patches],
        irrigation_method_sugarcane: vec![-9_999; patches],
        irrigation_groundwater_allocation: vec![MISSING; patches],
        irrigation_surface_water_allocation: vec![MISSING; patches],
        planting_day_corn: patch_missing.clone(),
        planting_day_spring_wheat: patch_missing.clone(),
        planting_day_winter_wheat: patch_missing.clone(),
        planting_day_soybean: patch_missing.clone(),
        planting_day_cotton: patch_missing.clone(),
        planting_day_rice1: patch_missing.clone(),
        planting_day_rice2: patch_missing.clone(),
        planting_day_sugarcane: patch_missing,
        fertilizer_nitrogen_corn: vec![0.0; patches],
        fertilizer_nitrogen_spring_wheat: vec![0.0; patches],
        fertilizer_nitrogen_winter_wheat: vec![0.0; patches],
        fertilizer_nitrogen_soybean: vec![0.0; patches],
        fertilizer_nitrogen_cotton: vec![0.0; patches],
        fertilizer_nitrogen_rice1: vec![0.0; patches],
        fertilizer_nitrogen_rice2: vec![0.0; patches],
        fertilizer_nitrogen_sugarcane: vec![0.0; patches],
    }
}

fn read_fertilizer_source_one(
    state: &mut CropColdStartState,
    classes: &[i32],
    path: impl AsRef<Path>,
    latitude: f64,
    longitude: f64,
) -> Result<()> {
    let file = open_map(path, "CROP fertilizer source 1")?;
    let cell = nearest_cell_indices(&file, latitude, longitude)?;
    for (index, &class) in classes.iter().enumerate() {
        let name = format!("CONST_FERTNITRO_CFT_{class:02}");
        state.fertilizer_nitrogen[index] = sample_f64_2d(&file, &name, cell)?
            .filter(|value| *value > 0.0)
            .unwrap_or(0.0);
    }
    Ok(())
}

fn read_fertilizer_source_two(
    state: &mut CropColdStartState,
    classes: &[i32],
    path: impl AsRef<Path>,
    latitude: f64,
    longitude: f64,
) -> Result<()> {
    let file = open_map(path, "CROP fertilizer source 2")?;
    let cell = nearest_cell_indices(&file, latitude, longitude)?;
    let manure = sample_f32_2d(&file, "manure", cell)?
        .unwrap_or(0.0)
        .max(0.0);
    for (index, &class) in classes.iter().enumerate() {
        state.manure_nitrogen[index] = manure;
        state.fertilizer_nitrogen[index] = sample_f32_3d(
            &file,
            "fertilizer",
            usize::try_from(class - CFT_FIRST).expect("validated CFT class"),
            cell,
        )?
        .unwrap_or(0.0)
        .max(0.0);
    }
    Ok(())
}

fn read_irrigation_methods(
    classes: &[i32],
    path: impl AsRef<Path>,
    latitude: f64,
    longitude: f64,
) -> Result<Vec<i32>> {
    let file = open_map(path, "CROP irrigation")?;
    let cell = nearest_cell_indices(&file, latitude, longitude)?;
    classes
        .iter()
        .map(|&class| {
            sample_f32_3d(
                &file,
                "irrigation_method",
                usize::try_from(class - CFT_FIRST).expect("validated CFT class"),
                cell,
            )
            .and_then(|value| {
                truncated_or(
                    value.filter(|value| *value >= 0.0),
                    CROP_MANAGEMENT_MISSING_I32,
                    "irrigation_method",
                )
            })
        })
        .collect()
}

fn open_map(path: impl AsRef<Path>, label: &str) -> Result<netcdf::File> {
    let path = path.as_ref();
    netcdf::open(path).with_context(|| format!("cannot open {label} map {}", path.display()))
}

fn sample_f64_2d(
    file: &netcdf::File,
    name: &str,
    (latitude, longitude): (usize, usize),
) -> Result<Option<f64>> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["lat", "lon"])?;
    let value = first_value(
        variable.get_values::<f64, _>((latitude..latitude + 1, longitude..longitude + 1))?,
        name,
    )?;
    Ok(valid_value(&variable, value).then_some(value))
}

fn sample_f32_2d(
    file: &netcdf::File,
    name: &str,
    (latitude, longitude): (usize, usize),
) -> Result<Option<f64>> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["lat", "lon"])?;
    let value = first_value(
        variable
            .get_values::<f32, _>((latitude..latitude + 1, longitude..longitude + 1))?
            .into_iter()
            .map(f64::from)
            .collect(),
        name,
    )?;
    Ok(valid_value(&variable, value).then_some(value))
}

fn sample_f32_3d(
    file: &netcdf::File,
    name: &str,
    cft: usize,
    (latitude, longitude): (usize, usize),
) -> Result<Option<f64>> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["cft", "lat", "lon"])?;
    ensure!(
        cft < variable.dimensions()[0].len(),
        "{name} has no CFT index {cft}"
    );
    let value = first_value(
        variable
            .get_values::<f32, _>((
                cft..cft + 1,
                latitude..latitude + 1,
                longitude..longitude + 1,
            ))?
            .into_iter()
            .map(f64::from)
            .collect(),
        name,
    )?;
    Ok(valid_value(&variable, value).then_some(value))
}

fn required_variable<'a>(file: &'a netcdf::File, name: &str) -> Result<netcdf::Variable<'a>> {
    file.variable(name)
        .with_context(|| format!("CROP management map has no {name}"))
}

fn require_dimensions(
    variable: &netcdf::Variable<'_>,
    name: &str,
    expected: &[&str],
) -> Result<()> {
    let actual = variable
        .dimensions()
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    ensure!(
        actual == expected,
        "CROP map {name} dimensions are {actual:?}, expected {expected:?}"
    );
    Ok(())
}

fn first_value(values: Vec<f64>, name: &str) -> Result<f64> {
    values
        .into_iter()
        .next()
        .with_context(|| format!("CROP map {name} selection returned no value"))
}

fn valid_value(variable: &netcdf::Variable<'_>, value: f64) -> bool {
    value.is_finite()
        && !["missing_value", "_FillValue"]
            .into_iter()
            .any(|attribute| {
                variable
                    .attribute_value(attribute)
                    .and_then(Result::ok)
                    .and_then(numeric_attribute)
                    .is_some_and(|missing| value == missing)
            })
}

fn numeric_attribute(value: netcdf::AttributeValue) -> Option<f64> {
    match value {
        netcdf::AttributeValue::Double(value) => Some(value),
        netcdf::AttributeValue::Float(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Int(value) => Some(f64::from(value)),
        netcdf::AttributeValue::Short(value) => Some(f64::from(value)),
        _ => None,
    }
}

fn truncated_or(value: Option<f64>, default: i32, name: &str) -> Result<i32> {
    match value {
        Some(value) => {
            ensure!(
                value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX),
                "CROP map {name} value {value} is outside CoLM's 32-bit integer range"
            );
            Ok(value.trunc() as i32)
        }
        None => Ok(default),
    }
}

#[cfg(test)]
#[path = "crop_tests.rs"]
mod tests;
