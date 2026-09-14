//! Shared CROP cold-start state.
//!
//! The state is owned here and borrowed by both restart writers, so a future
//! Rust runtime uses the same initialization contract rather than duplicating
//! CROP setup from `mkinidata`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, ensure, Context, Result};
use colm_core::CropPhenologyState;

use crate::{
    runtime::{coordinate_values, nearest_cell_indices},
    spatial_static::SpatialPixelSets,
    BgcCropFields, IrrigationFields, PftCropFields, MISSING,
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
const IRRIGATION_ALLOCATION_FILE: &str = "surfdata_irrigation_allocation.nc";

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
    /// Read `surfdata_irrigation_allocation.nc` for allocation mode three.
    pub use_irrigation_allocation: bool,
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
    patch_phase: Vec<f64>,
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
    state.patch_phase.fill(4.0);
    Ok(state)
}

/// Builds CoLM's no-map CROP branch for a complete spatial `landpft` block.
///
/// Unlike the single-point helper, the block can contain natural and crop PFTs
/// and multiple landpatches. `pft_to_patch` owns the axis conversion used by
/// BGC's patch-level crop summary.
pub(crate) fn spatial_crop_cold_start_from_tuning(
    classes: &[i32],
    pft_to_patch: &[usize],
    pft_fraction: &[f64],
    patches: usize,
    planting_day: f64,
) -> Result<CropColdStartState> {
    ensure!(
        classes.len() == pft_to_patch.len() && classes.len() == pft_fraction.len() && patches > 0,
        "spatial CROP PFT classes, ownership, and fractions must be aligned"
    );
    ensure!(
        planting_day.is_finite() && planting_day > 0.0,
        "DEF_TUNING_CROP_PLANTING_DAY must be finite and positive"
    );
    ensure!(
        pft_to_patch.iter().all(|patch| *patch < patches),
        "spatial CROP PFT references a landpatch outside the block"
    );
    ensure!(
        pft_fraction
            .iter()
            .all(|fraction| fraction.is_finite() && *fraction >= 0.0),
        "spatial CROP PFT fractions must be finite and nonnegative"
    );

    let mut state = empty_crop_state(classes.len(), patches);
    let mut phase_weight = vec![0.0; patches];
    let mut phase_sum = vec![0.0; patches];
    for (pft, &class) in classes.iter().enumerate() {
        if class >= CFT_FIRST {
            ensure!(
                class <= CFT_LAST && pft_fraction[pft] > 0.0,
                "spatial CROP PFT class {class} must be a positive CFT fraction"
            );
            state.planting_date[pft] = planting_day;
        }
        let patch = pft_to_patch[pft];
        phase_weight[patch] += pft_fraction[pft];
        phase_sum[patch] += state.crop_phase[pft] * pft_fraction[pft];
    }
    for patch in 0..patches {
        if phase_weight[patch] > 0.0 {
            state.patch_phase[patch] = phase_sum[patch];
        }
    }
    state.planting_day_rice2.fill(0.0);
    Ok(state)
}

/// Mirrors the spatial, raster-reading branch of upstream `CROP_readin`.
///
/// Continuous fields use CoLM's spherical areal weighting.  Irrigation uses
/// its separate `grid2pset_dominant` rule: the source cell with the largest
/// overlapping area, rather than a numerical average of category codes.
pub(crate) fn spatial_crop_cold_start_from_management(
    classes: &[i32],
    pft_to_patch: &[usize],
    pft_fraction: &[f64],
    patches: usize,
    pft_pixels: &SpatialPixelSets,
    patch_pixels: &SpatialPixelSets,
    config: CropManagementConfig<'_>,
) -> Result<CropColdStartState> {
    ensure!(
        classes.len() == pft_to_patch.len()
            && classes.len() == pft_fraction.len()
            && classes.len() == pft_pixels.cells.len()
            && patches > 0
            && patches == patch_pixels.cells.len()
            && pft_to_patch.iter().all(|patch| *patch < patches)
            && pft_fraction
                .iter()
                .all(|fraction| fraction.is_finite() && *fraction >= 0.0),
        "spatial CROP topology, fractions, and pixel memberships must be aligned"
    );
    ensure!(
        classes
            .iter()
            .filter(|&&class| class >= CFT_FIRST)
            .all(|&class| class <= CFT_LAST),
        "spatial CROP has a PFT class outside {CFT_FIRST}..={CFT_LAST}"
    );
    if let Some(day) = config.planting_day_override {
        ensure!(
            day.is_finite() && day > 0.0,
            "DEF_TUNING_CROP_PLANTING_DAY must be finite and positive when set"
        );
    }

    let crop_dir = config.runtime_dir.join(CROP_DIR);
    let planting = open_map(crop_dir.join(PLANTING_FILE), "CROP planting-date")?;
    let crop_grid = MapGrid::from_file(&planting)?;
    let mut crop_pft = AreaMapping::new(&crop_grid, pft_pixels)?;
    let mut crop_patch = AreaMapping::new(&crop_grid, patch_pixels)?;
    let rice2 = map_field_2d(&planting, "pdrice2", &crop_grid)?;
    crop_pft.exclude_invalid(&rice2.valid)?;
    crop_patch.exclude_invalid(&rice2.valid)?;

    let mut state = empty_crop_state(classes.len(), patches);
    for (patch, value) in crop_patch.average(&rice2)?.into_iter().enumerate() {
        state.planting_day_rice2[patch] = truncated_or(value, 0, "pdrice2")? as f64;
    }
    let mut planting_dates = BTreeMap::new();
    for &class in classes {
        if class >= CFT_FIRST && !planting_dates.contains_key(&class) {
            let field = map_field_2d(&planting, &format!("PLANTDATE_CFT_{class:02}"), &crop_grid)?;
            planting_dates.insert(class, crop_pft.average(&field)?);
        }
    }
    for (pft, &class) in classes.iter().enumerate() {
        if let Some(values) = planting_dates.get(&class) {
            state.planting_date[pft] = values[pft]
                .filter(|value| *value > 0.0)
                .unwrap_or(CROP_MANAGEMENT_MISSING);
        }
    }
    if let Some(day) = config.planting_day_override {
        for (pft, &class) in classes.iter().enumerate() {
            if class >= CFT_FIRST {
                state.planting_date[pft] = day;
            }
        }
    }

    if config.use_fertilizer {
        match config.fertilizer_source {
            1 => read_spatial_fertilizer_source_one(
                &mut state,
                classes,
                &crop_pft,
                &crop_grid,
                crop_dir.join(FERTILIZER_SOURCE_ONE_FILE),
            )?,
            2 => read_spatial_fertilizer_source_two(
                &mut state,
                classes,
                pft_pixels,
                crop_dir.join(FERTILIZER_SOURCE_TWO_FILE),
            )?,
            source => bail!("DEF_FERT_SOURCE must be 1 or 2, got {source}"),
        }
    }
    if config.use_irrigation {
        state.irrigation_method = Some(read_spatial_irrigation_methods(
            classes,
            pft_pixels,
            crop_dir.join(IRRIGATION_FILE),
        )?);
    }
    if config.use_irrigation_allocation {
        ensure!(
            config.use_irrigation,
            "irrigation allocation requires DEF_USE_IRRIGATION = .true."
        );
        let (groundwater, surface_water) = read_spatial_irrigation_allocations(
            patch_pixels,
            crop_dir.join(IRRIGATION_ALLOCATION_FILE),
        )?;
        state.irrigation_groundwater_allocation = groundwater;
        state.irrigation_surface_water_allocation = surface_water;
    }
    state.set_patch_fertilizer(classes, pft_to_patch)?;
    state.set_patch_irrigation(classes, pft_to_patch)?;
    set_spatial_patch_phase(&mut state, pft_to_patch, pft_fraction)?;
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
    if config.use_irrigation_allocation {
        ensure!(
            config.use_irrigation,
            "irrigation allocation requires DEF_USE_IRRIGATION = .true."
        );
        let (groundwater, surface_water) = read_irrigation_allocations(
            crop_dir.join(IRRIGATION_ALLOCATION_FILE),
            latitude_degrees,
            longitude_degrees,
        )?;
        state.irrigation_groundwater_allocation.fill(groundwater);
        state
            .irrigation_surface_water_allocation
            .fill(surface_water);
    }
    let pft_to_patch = (0..classes.len()).collect::<Vec<_>>();
    state.set_patch_fertilizer(classes, &pft_to_patch)?;
    state.set_patch_irrigation(classes, &pft_to_patch)?;
    state.patch_phase.fill(4.0);
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
            crop_phase: &self.patch_phase,
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

    fn set_patch_fertilizer(&mut self, classes: &[i32], pft_to_patch: &[usize]) -> Result<()> {
        ensure!(
            classes.len() == pft_to_patch.len()
                && pft_to_patch
                    .iter()
                    .all(|&patch| patch < self.patch_phase.len()),
            "CROP PFT-to-patch ownership is inconsistent"
        );
        for (index, &class) in classes.iter().enumerate() {
            let fertilizer = self.fertilizer_nitrogen[index];
            let patch = pft_to_patch[index];
            match class {
                17 | 18 | 63 | 64 => self.fertilizer_nitrogen_corn[patch] = fertilizer,
                19 | 20 => self.fertilizer_nitrogen_spring_wheat[patch] = fertilizer,
                21 | 22 => self.fertilizer_nitrogen_winter_wheat[patch] = fertilizer,
                23 | 24 | 77 | 78 => self.fertilizer_nitrogen_soybean[patch] = fertilizer,
                41 | 42 => self.fertilizer_nitrogen_cotton[patch] = fertilizer,
                61 | 62 => {
                    self.fertilizer_nitrogen_rice1[patch] = fertilizer;
                    self.fertilizer_nitrogen_rice2[patch] = fertilizer;
                }
                67 | 68 => self.fertilizer_nitrogen_sugarcane[patch] = fertilizer,
                _ => {}
            }
        }
        Ok(())
    }

    fn set_patch_irrigation(&mut self, classes: &[i32], pft_to_patch: &[usize]) -> Result<()> {
        let Some(methods) = self.irrigation_method.as_deref() else {
            return Ok(());
        };
        ensure!(
            classes.len() == methods.len()
                && classes.len() == pft_to_patch.len()
                && pft_to_patch
                    .iter()
                    .all(|&patch| patch < self.patch_phase.len()),
            "CROP PFT-to-patch irrigation ownership is inconsistent"
        );
        for (index, &class) in classes.iter().enumerate() {
            let method = methods[index];
            let patch = pft_to_patch[index];
            match class {
                17 | 18 | 63 | 64 => self.irrigation_method_corn[patch] = method,
                19 | 20 => self.irrigation_method_spring_wheat[patch] = method,
                21 | 22 => self.irrigation_method_winter_wheat[patch] = method,
                23 | 24 | 77 | 78 => self.irrigation_method_soybean[patch] = method,
                41 | 42 => self.irrigation_method_cotton[patch] = method,
                61 | 62 => {
                    self.irrigation_method_rice1[patch] = method;
                    self.irrigation_method_rice2[patch] = method;
                }
                67 | 68 => self.irrigation_method_sugarcane[patch] = method,
                _ => {}
            }
        }
        Ok(())
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
        patch_phase: vec![MISSING; patches],
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

fn set_spatial_patch_phase(
    state: &mut CropColdStartState,
    pft_to_patch: &[usize],
    pft_fraction: &[f64],
) -> Result<()> {
    ensure!(
        pft_to_patch.len() == state.crop_phase.len()
            && pft_fraction.len() == state.crop_phase.len()
            && pft_to_patch
                .iter()
                .all(|&patch| patch < state.patch_phase.len()),
        "CROP PFT phase ownership is inconsistent"
    );
    let mut weight = vec![0.0; state.patch_phase.len()];
    let mut phase = vec![0.0; state.patch_phase.len()];
    for ((&patch, &fraction), &crop_phase) in
        pft_to_patch.iter().zip(pft_fraction).zip(&state.crop_phase)
    {
        weight[patch] += fraction;
        phase[patch] += crop_phase * fraction;
    }
    for patch in 0..state.patch_phase.len() {
        state.patch_phase[patch] = if weight[patch] > 0.0 {
            phase[patch] / weight[patch]
        } else {
            MISSING
        };
    }
    Ok(())
}

pub(crate) struct MapGrid {
    lat_s: Vec<f64>,
    lat_n: Vec<f64>,
    lon_w: Vec<f64>,
    lon_span: Vec<f64>,
    /// Cyclic longitudes unwrapped into one increasing 360° sweep.
    lon_start: Vec<f64>,
}

impl MapGrid {
    pub(crate) fn from_file(file: &netcdf::File) -> Result<Self> {
        let latitude = coordinate_values(file, "lat")?;
        let longitude = coordinate_values(file, "lon")?;
        ensure!(
            !latitude.is_empty()
                && !longitude.is_empty()
                && latitude.iter().all(|value| value.is_finite())
                && longitude.iter().all(|value| value.is_finite()),
            "CROP management coordinates must be finite and nonempty"
        );
        let increasing = latitude.len() < 2 || latitude[1] > latitude[0];
        ensure!(
            latitude.windows(2).all(|pair| if increasing {
                pair[1] > pair[0]
            } else {
                pair[1] < pair[0]
            }),
            "CROP management latitude coordinates must be strictly monotonic"
        );
        let (lat_s, lat_n) = (0..latitude.len())
            .map(|index| {
                if increasing {
                    (
                        if index == 0 {
                            -90.0
                        } else {
                            (latitude[index - 1] + latitude[index]) * 0.5
                        },
                        if index + 1 == latitude.len() {
                            90.0
                        } else {
                            (latitude[index] + latitude[index + 1]) * 0.5
                        },
                    )
                } else {
                    (
                        if index + 1 == latitude.len() {
                            -90.0
                        } else {
                            (latitude[index] + latitude[index + 1]) * 0.5
                        },
                        if index == 0 {
                            90.0
                        } else {
                            (latitude[index - 1] + latitude[index]) * 0.5
                        },
                    )
                }
            })
            .unzip();
        let longitude = longitude
            .into_iter()
            .map(normalize_longitude)
            .collect::<Vec<_>>();
        ensure!(
            longitude.len() == 1
                || longitude.windows(2).all(|pair| {
                    let distance = (pair[1] - pair[0]).rem_euclid(360.0);
                    distance > 0.0 && distance < 360.0
                }),
            "CROP management longitude coordinates must be unique in cyclic order"
        );
        let (lon_w, lon_span) = if longitude.len() == 1 {
            (vec![-180.0], vec![360.0])
        } else {
            (0..longitude.len())
                .map(|index| {
                    let previous = longitude[(index + longitude.len() - 1) % longitude.len()];
                    let current = longitude[index];
                    let next = longitude[(index + 1) % longitude.len()];
                    let west = midpoint_longitude(previous, current);
                    let east = midpoint_longitude(current, next);
                    (west, longitude_span(west, east))
                })
                .unzip()
        };
        let lon_start = unwrap_longitude_starts(&lon_w)?;
        Ok(Self {
            lat_s,
            lat_n,
            lon_w,
            lon_span,
            lon_start,
        })
    }

    /// Read CoLM's explicit cell edges (`grid_define_from_file` without
    /// center-coordinate arguments).  Groundwater-depth maps use this form.
    pub(crate) fn from_explicit_edges(file: &netcdf::File) -> Result<Self> {
        let lat_s = coordinate_values(file, "lat_s")?;
        let lat_n = coordinate_values(file, "lat_n")?;
        let lon_w = coordinate_values(file, "lon_w")?;
        let lon_e = coordinate_values(file, "lon_e")?;
        ensure!(
            !lat_s.is_empty()
                && !lon_w.is_empty()
                && lat_s.len() == lat_n.len()
                && lon_w.len() == lon_e.len()
                && lat_s
                    .iter()
                    .zip(&lat_n)
                    .all(|(&south, &north)| south.is_finite()
                        && north.is_finite()
                        && north > south)
                && lon_w
                    .iter()
                    .zip(&lon_e)
                    .all(|(&west, &east)| west.is_finite()
                        && east.is_finite()
                        && longitude_span(west, east) > 0.0),
            "CoLM map cell edges must be finite, nonempty, and have positive area"
        );
        let increasing = lat_s.len() < 2 || lat_s[1] > lat_s[0];
        ensure!(
            lat_s.windows(2).all(|pair| if increasing {
                pair[1] > pair[0]
            } else {
                pair[1] < pair[0]
            }) && lat_n.windows(2).all(|pair| if increasing {
                pair[1] > pair[0]
            } else {
                pair[1] < pair[0]
            }),
            "CoLM map latitude edges must be strictly monotonic"
        );
        let lon_span = lon_w
            .iter()
            .zip(&lon_e)
            .map(|(&west, &east)| longitude_span(west, east))
            .collect();
        let lon_start = unwrap_longitude_starts(&lon_w)?;
        Ok(Self {
            lat_s,
            lat_n,
            lon_w,
            lon_span,
            lon_start,
        })
    }

    fn values(&self) -> usize {
        self.lat_s.len() * self.lon_w.len()
    }

    fn matches(&self, other: &Self) -> bool {
        self.lat_s == other.lat_s
            && self.lat_n == other.lat_n
            && self.lon_w == other.lon_w
            && self.lon_span == other.lon_span
    }

    fn overlapping_latitudes(&self, south: f64, north: f64) -> std::ops::Range<usize> {
        let increasing = self.lat_s.len() < 2 || self.lat_s[1] > self.lat_s[0];
        let (start, end) = if increasing {
            (
                self.lat_n.partition_point(|edge| *edge <= south),
                self.lat_s.partition_point(|edge| *edge < north),
            )
        } else {
            (
                self.lat_s.partition_point(|edge| *edge >= north),
                self.lat_n.partition_point(|edge| *edge > south),
            )
        };
        start.min(end)..end
    }

    fn overlapping_longitudes(&self, west: f64, span: f64) -> Vec<(usize, f64)> {
        let first = self.lon_start[0];
        let mut start = west.rem_euclid(360.0);
        while start < first {
            start += 360.0;
        }
        while start >= first + 360.0 {
            start -= 360.0;
        }
        let end = start + span;
        let source_count = self.lon_start.len();
        let first_index = self
            .lon_start
            .partition_point(|candidate| *candidate <= start)
            .saturating_sub(1);
        let mut overlaps = Vec::new();
        for position in first_index..first_index + source_count + 1 {
            let index = position % source_count;
            let source_start =
                self.lon_start[index] + if position >= source_count { 360.0 } else { 0.0 };
            if source_start >= end {
                break;
            }
            if source_start + self.lon_span[index] <= start {
                continue;
            }
            let width = longitude_overlap(west, span, self.lon_w[index], self.lon_span[index]);
            if width >= 1.0e-6 {
                overlaps.push((index, width));
            }
        }
        overlaps
    }
}

pub(crate) struct MapField {
    values: Vec<f64>,
    valid: Vec<bool>,
}

impl MapField {
    pub(crate) fn validity(&self) -> &[bool] {
        &self.valid
    }
}

pub(crate) struct AreaMapping {
    parts: Vec<Vec<(usize, f64)>>,
}

impl AreaMapping {
    pub(crate) fn new(grid: &MapGrid, pixel_sets: &SpatialPixelSets) -> Result<Self> {
        ensure!(
            pixel_sets.cells.len() == pixel_sets.shared_fraction.len()
                && !pixel_sets.lon_w.is_empty()
                && pixel_sets.lon_w.len() == pixel_sets.lon_e.len()
                && !pixel_sets.lat_s.is_empty()
                && pixel_sets.lat_s.len() == pixel_sets.lat_n.len(),
            "spatial CROP pixel geometry is inconsistent"
        );
        let mut parts = Vec::with_capacity(pixel_sets.cells.len());
        for (set, cells) in pixel_sets.cells.iter().enumerate() {
            let share = pixel_sets.shared_fraction[set];
            let mut overlap = BTreeMap::<(usize, usize), f64>::new();
            for &(x, y) in cells {
                let x = usize::try_from(x).context("CROP mesh longitude is negative")?;
                let y = usize::try_from(y).context("CROP mesh latitude is negative")?;
                ensure!(
                    x > 0 && x <= pixel_sets.lon_w.len() && y > 0 && y <= pixel_sets.lat_s.len(),
                    "CROP mesh pixel is outside pixel axes"
                );
                let west = pixel_sets.lon_w[x - 1];
                let east = pixel_sets.lon_e[x - 1];
                let south = pixel_sets.lat_s[y - 1];
                let north = pixel_sets.lat_n[y - 1];
                ensure!(
                    south.is_finite()
                        && north.is_finite()
                        && west.is_finite()
                        && east.is_finite()
                        && north > south,
                    "CROP pixel has invalid geographic edges"
                );
                let target_span = longitude_span(west, east);
                ensure!(target_span > 0.0, "CROP pixel has zero longitude width");
                let longitudes = grid.overlapping_longitudes(west, target_span);
                for latitude in grid.overlapping_latitudes(south, north) {
                    let overlap_south = south.max(grid.lat_s[latitude]);
                    let overlap_north = north.min(grid.lat_n[latitude]);
                    for &(longitude, width) in &longitudes {
                        let area = width.to_radians()
                            * (overlap_north.to_radians().sin() - overlap_south.to_radians().sin())
                            * share;
                        ensure!(
                            area.is_finite() && area >= 0.0,
                            "CROP map overlap has invalid spherical area"
                        );
                        *overlap.entry((longitude, latitude)).or_default() += area;
                    }
                }
            }
            parts.push(
                overlap
                    .into_iter()
                    .filter_map(|((longitude, latitude), area)| {
                        (area > 0.0).then_some((latitude * grid.lon_w.len() + longitude, area))
                    })
                    .collect(),
            );
        }
        Ok(Self { parts })
    }

    pub(crate) fn len(&self) -> usize {
        self.parts.len()
    }

    /// Apply `spatial_mapping_set_missing_value` semantics before averaging.
    /// The remaining weights are renormalized by [`Self::average`].
    pub(crate) fn exclude_invalid(&mut self, valid: &[bool]) -> Result<()> {
        for parts in &mut self.parts {
            ensure!(
                parts.iter().all(|(index, _)| *index < valid.len()),
                "CROP map masking has inconsistent source dimensions"
            );
            parts.retain(|(index, _)| valid[*index]);
        }
        Ok(())
    }

    pub(crate) fn average(&self, field: &MapField) -> Result<Vec<Option<f64>>> {
        ensure!(
            field.values.len() == field.valid.len()
                && self
                    .parts
                    .iter()
                    .flatten()
                    .all(|(index, _)| *index < field.values.len()),
            "CROP map values have inconsistent dimensions"
        );
        Ok(self
            .parts
            .iter()
            .map(|parts| {
                let (sum, area) = parts
                    .iter()
                    .fold((0.0, 0.0), |(sum, area), &(index, weight)| {
                        (sum + field.values[index] * weight, area + weight)
                    });
                (area > 0.0).then_some(sum / area)
            })
            .collect())
    }

    /// CN equilibrium fields cannot safely substitute a missing value, unlike
    /// CROP's explicit `pdrice2` fallback.  Require complete mapped coverage.
    pub(crate) fn average_required(&self, field: &MapField, name: &str) -> Result<Vec<f64>> {
        ensure!(
            self.parts.iter().all(|parts| {
                !parts.is_empty() && parts.iter().all(|(index, _)| field.valid[*index])
            }),
            "spatial BGC equilibrium field {name} has missing mapped values"
        );
        self.average(field)?
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .with_context(|| format!("spatial BGC equilibrium field {name} has no mapped area"))
    }

    fn dominant(&self, field: &MapField) -> Result<Vec<Option<f64>>> {
        ensure!(
            field.values.len() == field.valid.len()
                && self
                    .parts
                    .iter()
                    .flatten()
                    .all(|(index, _)| *index < field.values.len()),
            "CROP map values have inconsistent dimensions"
        );
        Ok(self
            .parts
            .iter()
            .map(|parts| {
                let mut largest = None;
                for &(index, area) in parts {
                    if largest.is_none_or(|(_, largest_area)| area > largest_area) {
                        largest = Some((field.values[index], area));
                    }
                }
                largest.map(|(value, _)| value)
            })
            .collect())
    }
}

fn midpoint_longitude(west: f64, east: f64) -> f64 {
    normalize_longitude(if west > east {
        (west + east + 360.0) * 0.5
    } else {
        (west + east) * 0.5
    })
}

fn normalize_longitude(value: f64) -> f64 {
    (value + 180.0).rem_euclid(360.0) - 180.0
}

fn longitude_span(west: f64, east: f64) -> f64 {
    if (east - west).abs() >= 360.0 - 1.0e-10 {
        360.0
    } else {
        (normalize_longitude(east) - normalize_longitude(west)).rem_euclid(360.0)
    }
}

fn unwrap_longitude_starts(values: &[f64]) -> Result<Vec<f64>> {
    ensure!(
        !values.is_empty() && values.iter().all(|value| value.is_finite()),
        "CoLM map longitude edges must be finite and nonempty"
    );
    let mut starts = Vec::with_capacity(values.len());
    for &value in values {
        let mut value = value.rem_euclid(360.0);
        if let Some(&previous) = starts.last() {
            while value <= previous {
                value += 360.0;
            }
            ensure!(
                value - previous < 360.0,
                "CoLM map longitude edges must be in cyclic order"
            );
        }
        starts.push(value);
    }
    Ok(starts)
}

fn longitude_overlap(west_a: f64, span_a: f64, west_b: f64, span_b: f64) -> f64 {
    let segments = |west: f64, span: f64| {
        let west = west.rem_euclid(360.0);
        if span >= 360.0 - 1.0e-10 {
            vec![(0.0, 360.0)]
        } else if west + span <= 360.0 {
            vec![(west, west + span)]
        } else {
            vec![(west, 360.0), (0.0, west + span - 360.0)]
        }
    };
    segments(west_a, span_a)
        .into_iter()
        .flat_map(|(west_a, east_a)| {
            segments(west_b, span_b)
                .into_iter()
                .map(move |(west_b, east_b)| (east_a.min(east_b) - west_a.max(west_b)).max(0.0))
        })
        .sum()
}

pub(crate) fn map_field_2d(file: &netcdf::File, name: &str, grid: &MapGrid) -> Result<MapField> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["lat", "lon"])?;
    let values = variable.get_values::<f64, _>(..).or_else(|_| {
        variable
            .get_values::<f32, _>(..)
            .map(|values| values.into_iter().map(f64::from).collect())
    })?;
    map_field(variable, values, grid, name)
}

pub(crate) fn map_field_soil_3d(
    file: &netcdf::File,
    name: &str,
    soil: usize,
    grid: &MapGrid,
) -> Result<MapField> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["lat", "lon", "soil"])?;
    ensure!(
        soil < variable.dimensions()[2].len(),
        "{name} has no soil index {soil}"
    );
    let latitude = grid.lat_s.len();
    let longitude = grid.lon_w.len();
    let values = variable
        .get_values::<f64, _>((0..latitude, 0..longitude, soil..soil + 1))
        .or_else(|_| {
            variable
                .get_values::<f32, _>((0..latitude, 0..longitude, soil..soil + 1))
                .map(|values| values.into_iter().map(f64::from).collect())
        })?;
    map_field(variable, values, grid, name)
}

/// Read one monthly/time slice with CoLM's `(time, lat, lon)` layout.
///
/// `mkinidata` selects the calendar month before applying its areal mapper;
/// keeping that selection here makes soil, snow, and water-table readers share
/// the established spherical overlap implementation.
pub(crate) fn map_field_time_3d(
    file: &netcdf::File,
    name: &str,
    time: usize,
    grid: &MapGrid,
) -> Result<MapField> {
    let variable = required_variable(file, name)?;
    require_time_lat_lon(&variable, name)?;
    ensure!(
        time < variable.dimensions()[0].len(),
        "{name} has no time index {time}"
    );
    let latitude = grid.lat_s.len();
    let longitude = grid.lon_w.len();
    let values = variable
        .get_values::<f64, _>((time..time + 1, 0..latitude, 0..longitude))
        .or_else(|_| {
            variable
                .get_values::<f32, _>((time..time + 1, 0..latitude, 0..longitude))
                .map(|values| values.into_iter().map(f64::from).collect())
        })?;
    map_field(variable, values, grid, name)
}

/// Read one vertical layer of a CoLM `(time, lat, lon, layer)` map.
pub(crate) fn map_field_time_profile_4d(
    file: &netcdf::File,
    name: &str,
    time: usize,
    layer: usize,
    grid: &MapGrid,
) -> Result<MapField> {
    let variable = required_variable(file, name)?;
    require_time_lat_lon_layer(&variable, name)?;
    ensure!(
        time < variable.dimensions()[0].len(),
        "{name} has no time index {time}"
    );
    ensure!(
        layer < variable.dimensions()[3].len(),
        "{name} has no layer index {layer}"
    );
    let latitude = grid.lat_s.len();
    let longitude = grid.lon_w.len();
    let values = variable
        .get_values::<f64, _>((time..time + 1, 0..latitude, 0..longitude, layer..layer + 1))
        .or_else(|_| {
            variable
                .get_values::<f32, _>((time..time + 1, 0..latitude, 0..longitude, layer..layer + 1))
                .map(|values| values.into_iter().map(f64::from).collect())
        })?;
    map_field(variable, values, grid, name)
}

fn map_field_3d(file: &netcdf::File, name: &str, cft: usize, grid: &MapGrid) -> Result<MapField> {
    let variable = required_variable(file, name)?;
    require_dimensions(&variable, name, &["cft", "lat", "lon"])?;
    ensure!(
        cft < variable.dimensions()[0].len(),
        "{name} has no CFT index {cft}"
    );
    let latitude = grid.lat_s.len();
    let longitude = grid.lon_w.len();
    let values = variable
        .get_values::<f64, _>((cft..cft + 1, 0..latitude, 0..longitude))
        .or_else(|_| {
            variable
                .get_values::<f32, _>((cft..cft + 1, 0..latitude, 0..longitude))
                .map(|values| values.into_iter().map(f64::from).collect())
        })?;
    map_field(variable, values, grid, name)
}

fn map_field(
    variable: netcdf::Variable<'_>,
    values: Vec<f64>,
    grid: &MapGrid,
    name: &str,
) -> Result<MapField> {
    ensure!(
        values.len() == grid.values(),
        "CROP map {name} has {} values; expected {} from its coordinate axes",
        values.len(),
        grid.values()
    );
    let missing = ["missing_value", "_FillValue"]
        .into_iter()
        .filter_map(|attribute| {
            variable
                .attribute_value(attribute)
                .and_then(Result::ok)
                .and_then(numeric_attribute)
        })
        .collect::<Vec<_>>();
    let valid = values
        .iter()
        .map(|value| value.is_finite() && !missing.iter().any(|missing| value == missing))
        .collect();
    Ok(MapField { values, valid })
}

fn require_time_lat_lon(variable: &netcdf::Variable<'_>, name: &str) -> Result<()> {
    let dimensions = variable.dimensions();
    let actual = dimensions
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    ensure!(
        dimensions.len() == 3
            && matches!(actual[0].as_str(), "month" | "time")
            && actual[1..] == ["lat", "lon"],
        "CoLM map {name} dimensions are {actual:?}, expected (month|time, lat, lon)"
    );
    Ok(())
}

fn require_time_lat_lon_layer(variable: &netcdf::Variable<'_>, name: &str) -> Result<()> {
    let dimensions = variable.dimensions();
    let actual = dimensions
        .iter()
        .map(|dimension| dimension.name())
        .collect::<Vec<_>>();
    ensure!(
        dimensions.len() == 4
            && matches!(actual[0].as_str(), "month" | "time")
            && actual[1..] == ["lat", "lon", "layer"],
        "CoLM map {name} dimensions are {actual:?}, expected (month|time, lat, lon, layer)"
    );
    Ok(())
}

fn read_spatial_fertilizer_source_one(
    state: &mut CropColdStartState,
    classes: &[i32],
    mapping: &AreaMapping,
    crop_grid: &MapGrid,
    path: impl AsRef<Path>,
) -> Result<()> {
    state.fertilizer_nitrogen.fill(CROP_MANAGEMENT_MISSING);
    let file = open_map(path, "CROP fertilizer source 1")?;
    let grid = MapGrid::from_file(&file)?;
    ensure!(
        crop_grid.matches(&grid),
        "CROP fertilizer source 1 grid differs from the planting-date grid used by CoLM's shared mapping"
    );
    let mut fertilizer = BTreeMap::new();
    for &class in classes {
        if class >= CFT_FIRST && !fertilizer.contains_key(&class) {
            let field = map_field_2d(&file, &format!("CONST_FERTNITRO_CFT_{class:02}"), &grid)?;
            fertilizer.insert(class, mapping.average(&field)?);
        }
    }
    for (pft, &class) in classes.iter().enumerate() {
        if let Some(values) = fertilizer.get(&class) {
            state.fertilizer_nitrogen[pft] =
                values[pft].unwrap_or(CROP_MANAGEMENT_MISSING).max(0.0);
        }
    }
    Ok(())
}

fn read_spatial_fertilizer_source_two(
    state: &mut CropColdStartState,
    classes: &[i32],
    pft_pixels: &SpatialPixelSets,
    path: impl AsRef<Path>,
) -> Result<()> {
    state.fertilizer_nitrogen.fill(CROP_MANAGEMENT_MISSING);
    state.manure_nitrogen.fill(CROP_MANAGEMENT_MISSING);
    let file = open_map(path, "CROP fertilizer source 2")?;
    let grid = MapGrid::from_file(&file)?;
    let mapping = AreaMapping::new(&grid, pft_pixels)?;
    let manure = mapping.average(&map_field_2d(&file, "manure", &grid)?)?;
    let mut fertilizer = BTreeMap::new();
    for &class in classes {
        if class >= CFT_FIRST && !fertilizer.contains_key(&class) {
            let cft = usize::try_from(class - CFT_FIRST).expect("validated CFT class");
            fertilizer.insert(
                class,
                mapping.average(&map_field_3d(&file, "fertilizer", cft, &grid)?)?,
            );
        }
    }
    for (pft, &class) in classes.iter().enumerate() {
        if let Some(values) = fertilizer.get(&class) {
            state.manure_nitrogen[pft] = manure[pft].unwrap_or(CROP_MANAGEMENT_MISSING).max(0.0);
            state.fertilizer_nitrogen[pft] =
                values[pft].unwrap_or(CROP_MANAGEMENT_MISSING).max(0.0);
        }
    }
    Ok(())
}

fn read_spatial_irrigation_methods(
    classes: &[i32],
    pft_pixels: &SpatialPixelSets,
    path: impl AsRef<Path>,
) -> Result<Vec<i32>> {
    let file = open_map(path, "CROP irrigation")?;
    let grid = MapGrid::from_file(&file)?;
    let mapping = AreaMapping::new(&grid, pft_pixels)?;
    let mut methods = vec![CROP_MANAGEMENT_MISSING_I32; classes.len()];
    let mut fields = BTreeMap::new();
    for &class in classes {
        if class >= CFT_FIRST && !fields.contains_key(&class) {
            let cft = usize::try_from(class - CFT_FIRST).expect("validated CFT class");
            fields.insert(
                class,
                mapping.dominant(&map_field_3d(&file, "irrigation_method", cft, &grid)?)?,
            );
        }
    }
    for (pft, &class) in classes.iter().enumerate() {
        if let Some(values) = fields.get(&class) {
            methods[pft] = truncated_or(
                values[pft].filter(|value| *value >= 0.0),
                CROP_MANAGEMENT_MISSING_I32,
                "irrigation_method",
            )?;
        }
    }
    Ok(methods)
}

fn read_spatial_irrigation_allocations(
    patch_pixels: &SpatialPixelSets,
    path: impl AsRef<Path>,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let file = open_map(path, "CROP irrigation-allocation")?;
    let grid = MapGrid::from_file(&file)?;
    let mapping = AreaMapping::new(&grid, patch_pixels)?;
    let groundwater = mapping
        .average(&map_field_2d(&file, "irrig_gw_alloc", &grid)?)?
        .into_iter()
        .map(|value| value.unwrap_or(MISSING))
        .collect();
    let surface_water = mapping
        .average(&map_field_2d(&file, "irrig_sw_alloc", &grid)?)?
        .into_iter()
        .map(|value| value.unwrap_or(MISSING))
        .collect();
    Ok((groundwater, surface_water))
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

fn read_irrigation_allocations(
    path: impl AsRef<Path>,
    latitude_degrees: f64,
    longitude_degrees: f64,
) -> Result<(f64, f64)> {
    let file = open_map(path, "CROP irrigation-allocation")?;
    let cell = nearest_cell_indices(&file, latitude_degrees, longitude_degrees)?;
    let groundwater = sample_f64_2d(&file, "irrig_gw_alloc", cell)?
        .context("CROP irrigation groundwater allocation is missing")?;
    let surface_water = sample_f64_2d(&file, "irrig_sw_alloc", cell)?
        .context("CROP irrigation surface-water allocation is missing")?;
    Ok((groundwater, surface_water))
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
