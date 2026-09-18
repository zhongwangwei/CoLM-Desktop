//! Flat kernels for CoLM spatial `mksrfdata` aggregation.
//!
//! The Fortran implementation stores every patch through a graph of derived
//! types and allocates a temporary vector for each aggregation request.  The
//! Rust port uses one CSR-style layout instead.  A patch occupies
//! `cell_offsets[p]..cell_offsets[p + 1]` in the raw-cell index vector.  It is
//! compact, cache-friendly, and reusable for every surface field.
//!
//! These kernels intentionally contain no NetCDF or MPI calls.  File ownership
//! and rank scheduling stay at the outer layer; the numerical part can then be
//! tested against the corresponding Fortran routines without a rawdata mount.

use std::collections::BTreeMap;

use anyhow::{bail, ensure, Context, Result};

use crate::albedo::albedo;

/// CoLM's missing output marker for surface fields.
pub const SURFACE_MISSING: f64 = -1.0e36;

/// Patch-to-raw-cell topology in compressed sparse row form.
///
/// This is the flattened equivalent of the per-patch pixel lists requested by
/// `aggregation_request_data` in `mksrfdata/MOD_AggregationRequestData.F90`.
/// `wmo_source[p]` means that patch `p` copies the already-computed result of
/// the referenced WMO patch, matching `Aggregation_Topography.F90` and
/// `Aggregation_SoilTexture.F90`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatPatches {
    patch_types: Vec<i32>,
    cell_offsets: Vec<usize>,
    cells: Vec<usize>,
    wmo_source: Vec<Option<usize>>,
}

/// The three vectors written by `Aggregation_Topography.F90`.
#[derive(Debug, Clone, PartialEq)]
pub struct Topography {
    pub elevation: Vec<f64>,
    pub elevation_std: Vec<f64>,
    pub slope_ratio: Vec<f64>,
}

/// The four patch vectors written by `Aggregation_SoilBrightness.F90`.
#[derive(Debug, Clone, PartialEq)]
pub struct SoilBrightness {
    pub saturated_visible: Vec<f64>,
    pub dry_visible: Vec<f64>,
    pub saturated_near_infrared: Vec<f64>,
    pub dry_near_infrared: Vec<f64>,
}

/// Fitted TOPMODEL parameters from one gathered topographic-wetness sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopographicWetness {
    pub mean_twi: f64,
    pub fsatmax: f64,
    pub fsatdcf: f64,
    pub alp_twi: f64,
    pub chi_twi: f64,
    pub mu_twi: f64,
}

/// Patch outputs from `Aggregation_TopographyFactors_Simple`.
///
/// Type fields use `type * patches + patch` layout, matching Fortran's
/// `(slope_type, patch)` arrays.
#[derive(Debug, Clone, PartialEq)]
pub struct SimpleTopographyFactors {
    pub curvature: Vec<f64>,
    pub slope_by_aspect: Vec<f64>,
    pub aspect_by_aspect: Vec<f64>,
    pub aspect_types: usize,
}

/// Patch outputs from `Aggregation_TopographyFactors.F90`.
///
/// Type and shadow-curve fields are axis-major with patch last, which is the
/// layout consumed by the restart writer.
#[derive(Debug, Clone, PartialEq)]
pub struct RegularTopographyFactors {
    pub sky_view_factor: Vec<f64>,
    pub curvature: Vec<f64>,
    pub slope_type: Vec<f64>,
    pub aspect_type: Vec<f64>,
    pub area_type: Vec<f64>,
    pub shadow_curve: Vec<f64>,
}

const REGULAR_SLOPE_TYPES: usize = 4;
const REGULAR_AZIMUTHS: usize = 16;
const REGULAR_ZENITHS: usize = 101;
const REGULAR_CURVE_PARAMETERS: usize = 3;

/// Calculates the valid-sample branch of `Aggregation_TopoWetness`.
///
/// Callers gather all 25 raw TWI depths for one patch or element first.  A
/// return value of `None` is the Fortran routine's `npxl < 25` condition and
/// must be filled from the owning element's result by the topology adapter.
pub fn derive_topographic_wetness(raw_twi: &[f64]) -> Result<Option<TopographicWetness>> {
    let mut values = raw_twi
        .iter()
        .copied()
        .filter(|value| *value > -1.0e3)
        .collect::<Vec<_>>();
    ensure!(
        values.iter().all(|value| value.is_finite()),
        "valid topographic-wetness values must be finite"
    );
    if values.len() < 25 {
        return Ok(None);
    }
    values.sort_unstable_by(f64::total_cmp);
    let count = values.len();
    let count_f = count as f64;
    let mean_twi = values.iter().sum::<f64>() / count_f;
    let mut mean_index = 0;
    while values[mean_index] < mean_twi && mean_index < count - 2 {
        mean_index += 1;
    }
    let fsatmax = 1.0 - mean_index as f64 / count_f;
    let mut xx_yy_sum = 0.0;
    let mut xx_squared_sum = 0.0;
    for (index, &value) in values[mean_index..count - 1].iter().enumerate() {
        let rank = mean_index + index + 1;
        let xx = -(value - mean_twi);
        let yy = ((1.0 - rank as f64 / count_f) / fsatmax).ln();
        xx_yy_sum += xx * yy;
        xx_squared_sum += xx * xx;
    }
    ensure!(
        xx_squared_sum > 0.0 && xx_squared_sum.is_finite(),
        "topographic-wetness samples cannot fit fsatdcf"
    );
    let fsatdcf = xx_yy_sum / xx_squared_sum;
    let variance = values
        .iter()
        .map(|value| (value - mean_twi).powi(2))
        .sum::<f64>()
        / (count_f - 1.0);
    let sigma_twi = variance.sqrt();
    let (alp_twi, chi_twi, mu_twi) = if sigma_twi > 0.0 {
        let skew_twi = count_f / ((count_f - 1.0) * (count_f - 2.0))
            * values
                .iter()
                .map(|value| (value - mean_twi).powi(3))
                .sum::<f64>()
            / sigma_twi.powi(3);
        if skew_twi > 0.0 {
            (
                (2.0 / skew_twi).powi(2),
                sigma_twi * skew_twi / 2.0,
                mean_twi - 2.0 * sigma_twi / skew_twi,
            )
        } else {
            (0.1, 0.01, 0.0)
        }
    } else {
        (0.1, 0.01, 0.0)
    };
    Ok(Some(TopographicWetness {
        mean_twi,
        fsatmax: fsatmax.clamp(0.1, 1.0),
        fsatdcf: fsatdcf.clamp(0.2, 2.0),
        alp_twi: alp_twi.clamp(0.1, 50.0),
        chi_twi: chi_twi.clamp(0.01, 4.0),
        mu_twi: mu_twi.clamp(0.0, 12.0),
    }))
}

impl FlatPatches {
    /// Build a validated flattened patch layout.
    pub fn new(
        patch_types: Vec<i32>,
        cell_offsets: Vec<usize>,
        cells: Vec<usize>,
        wmo_source: Vec<Option<usize>>,
    ) -> Result<Self> {
        ensure!(
            cell_offsets.len() == patch_types.len() + 1,
            "cell_offsets must have one entry more than patch_types"
        );
        ensure!(
            cell_offsets.first() == Some(&0),
            "cell_offsets must begin at zero"
        );
        ensure!(
            cell_offsets.last() == Some(&cells.len()),
            "last cell offset must equal the number of cells"
        );
        ensure!(
            cell_offsets.windows(2).all(|pair| pair[0] <= pair[1]),
            "cell_offsets must be nondecreasing"
        );
        ensure!(
            wmo_source.len() == patch_types.len(),
            "wmo_source must have one entry per patch"
        );
        for (patch, source) in wmo_source.iter().enumerate() {
            if let Some(source) = source {
                ensure!(
                    *source < patch,
                    "WMO source patch {source} for patch {patch} must precede its consumer"
                );
            }
        }
        Ok(Self {
            patch_types,
            cell_offsets,
            cells,
            wmo_source,
        })
    }

    pub fn len(&self) -> usize {
        self.patch_types.len()
    }

    pub fn is_empty(&self) -> bool {
        self.patch_types.is_empty()
    }

    /// Port of `Aggregation_SoilTexture`: select the most frequent class in
    /// every patch.  Ties resolve to the smaller class, as the Fortran routine
    /// sorts values first and `maxloc` returns its first maximum.
    pub fn aggregate_soil_texture(&self, texture: &[i32]) -> Result<Vec<i32>> {
        let mut result = vec![0; self.len()];
        let mut scratch = Vec::new();
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            scratch.clear();
            self.gather(texture, patch, &mut scratch)?;
            result[patch] = most_frequent(&mut scratch)
                .with_context(|| format!("soil texture patch {patch} has no raw cells"))?;
        }
        Ok(result)
    }

    /// Port of `Aggregation_LakeDepth`.
    ///
    /// `lake_depth_decimeters` is the integer-like raw field before CoLM's
    /// `block_data_linear_transform(..., scl = 0.1)`; only patches whose type
    /// equals `waterbody_type` receive a median.  All other patches receive
    /// CoLM's documented missing marker.
    pub fn aggregate_lake_depth(
        &self,
        lake_depth_decimeters: &[f64],
        waterbody_type: i32,
    ) -> Result<Vec<f64>> {
        let mut result = vec![SURFACE_MISSING; self.len()];
        let mut scratch = Vec::new();
        for (patch, output) in result.iter_mut().enumerate() {
            if self.patch_types[patch] != waterbody_type {
                continue;
            }
            scratch.clear();
            self.gather(lake_depth_decimeters, patch, &mut scratch)?;
            for value in &mut scratch {
                ensure!(
                    value.is_finite(),
                    "lake depth patch {patch} contains a non-finite value"
                );
                *value *= 0.1;
            }
            *output = median(&mut scratch)
                .with_context(|| format!("lake patch {patch} has no raw cells"))?;
        }
        Ok(result)
    }

    /// Aggregate PHH2O's depth-weighted hydrogen activity over each relevant
    /// soil or wetland patch.
    ///
    /// The logarithm is intentionally applied after the area-depth mean, not
    /// before it: pH is logarithmic and averaging pH values directly changes
    /// methane production.  Patches without valid PHH2O coverage retain the
    /// upstream neutral fallback of 6.2.
    pub fn aggregate_methane_ph(
        &self,
        ph: &[f64],
        depth_weight: &[f64],
        area: &[f64],
        relevant: impl Fn(i32) -> bool,
    ) -> Result<Vec<f64>> {
        ensure!(
            ph.len() == depth_weight.len() && ph.len() == area.len(),
            "methane pH values, depth weights, and intersection areas must have the same length"
        );
        let mut result = vec![6.2; self.len()];
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            if !relevant(self.patch_types[patch]) {
                continue;
            }
            let mut activity_area = 0.0;
            let mut valid_area_depth = 0.0;
            for &cell in self.raw_cells(patch) {
                let ph_value = value(ph, cell, "methane pH", patch)?;
                let weight = value(depth_weight, cell, "methane pH depth weight", patch)?;
                let intersection = value(area, cell, "methane pH intersection area", patch)?;
                ensure!(
                    weight.is_finite()
                        && weight >= 0.0
                        && intersection.is_finite()
                        && intersection > 0.0,
                    "methane pH patch {patch} has an invalid depth weight or intersection area"
                );
                if !(2.0..=10.0).contains(&ph_value) || weight == 0.0 {
                    continue;
                }
                let area_depth = intersection * weight;
                activity_area += 10_f64.powf(-ph_value) * area_depth;
                valid_area_depth += area_depth;
            }
            if valid_area_depth > 0.0 {
                let value = -(activity_area / valid_area_depth).log10();
                ensure!(
                    value.is_finite() && (2.0..=10.0).contains(&value),
                    "methane pH patch {patch} has an invalid aggregate"
                );
                result[patch] = value;
            }
        }
        Ok(result)
    }

    /// Port of `Aggregation_LakeSoilC`.
    ///
    /// `raw_soil_carbon_g_m3` and the returned vector are layer-major.  Only
    /// waterbody patches are aggregated; invalid/missing raw values are
    /// excluded with their area, as in the reference implementation.
    pub fn aggregate_lake_soil_carbon(
        &self,
        raw_soil_carbon_g_m3: &[f64],
        layers: usize,
        landarea: &[f64],
        waterbody_type: i32,
    ) -> Result<Vec<f64>> {
        ensure!(
            layers > 0 && raw_soil_carbon_g_m3.len() == layers * landarea.len(),
            "lake soil carbon must be layers x raw cell count"
        );
        let mut result = vec![0.0; layers * self.len()];
        for patch in 0..self.len() {
            if self.patch_types[patch] != waterbody_type {
                continue;
            }
            for layer in 0..layers {
                let mut valid_area = 0.0;
                let mut carbon_sum = 0.0;
                for &cell in &self.cells[self.cells_for(patch)] {
                    let area = value(landarea, cell, "landarea", patch)?;
                    let carbon = raw_soil_carbon_g_m3
                        .get(layer * landarea.len() + cell)
                        .copied()
                        .with_context(|| {
                            format!(
                                "lake soil-carbon patch {patch} references layer {layer}, raw cell {cell}"
                            )
                        })?;
                    if area > 0.0
                        && area.is_finite()
                        && carbon.is_finite()
                        && carbon >= 0.0
                        && carbon.abs() < 0.5 * SURFACE_MISSING.abs()
                    {
                        valid_area += area;
                        carbon_sum += carbon * area;
                    }
                }
                if valid_area > f64::MIN_POSITIVE {
                    result[layer * self.len() + patch] = carbon_sum / valid_area;
                }
            }
        }
        Ok(result)
    }

    /// Aggregate the 25-layer `TWI.nc` source for each patch.
    ///
    /// `Aggregation_TopoWetness.F90` first tries the patch's own samples and
    /// then falls back to its owning element when fewer than 25 valid values
    /// exist.  This method is the first step: it preserves the `None` result
    /// so the topology adapter can apply that element-level fallback.
    pub fn aggregate_topographic_wetness(
        &self,
        raw_twi: &[f64],
        layers: usize,
    ) -> Result<Vec<Option<TopographicWetness>>> {
        ensure!(
            layers > 0,
            "topographic-wetness source needs at least one layer"
        );
        ensure!(
            raw_twi.len() % layers == 0,
            "topographic-wetness source must be layer-major"
        );
        let cells = raw_twi.len() / layers;
        let mut result = vec![None; self.len()];
        let mut gathered = Vec::new();
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            gathered.clear();
            for layer in 0..layers {
                for &cell in &self.cells[self.cells_for(patch)] {
                    gathered.push(*raw_twi.get(layer * cells + cell).with_context(|| {
                        format!(
                            "topographic-wetness patch {patch} references layer {layer}, raw cell {cell}, but the source has {cells} cells"
                        )
                    })?);
                }
            }
            result[patch] = derive_topographic_wetness(&gathered)?;
        }
        Ok(result)
    }

    /// Aggregate previous land-cover classes for CoLM LULCC transfer traces.
    ///
    /// Output is class-major: class * patches + patch. WMO consumer patches
    /// remain zero, matching the upstream routine's explicit skip.
    pub fn aggregate_lulcc_source_fractions(
        &self,
        previous_class: &[i32],
        landarea: &[f64],
        max_class: usize,
    ) -> Result<Vec<f64>> {
        let (mut result, patch_area, classes) =
            self.lulcc_source_area_by_patch(previous_class, landarea, max_class)?;
        for patch in 0..self.len() {
            if self.wmo_source[patch].is_some() {
                continue;
            }
            let total = patch_area[patch];
            ensure!(
                total > 0.0 && total.is_finite(),
                "LULCC patch {patch} has zero or non-finite land area"
            );
            for class in 0..classes {
                result[class * self.len() + patch] /= total;
            }
        }
        Ok(result)
    }

    /// Aggregate previous land-cover classes for `lccpct_matrix` diagnostics.
    ///
    /// Unlike the per-patch `lccpct_patches_lcXX` vectors, upstream normalizes
    /// the diagnostic source-class areas by the total non-WMO patch area in the
    /// owning element.  This preserves partial current-patch contributions in
    /// the diagnostic matrix instead of making every patch sum to one.
    pub fn aggregate_lulcc_element_source_fractions(
        &self,
        patch_elements: &[usize],
        previous_class: &[i32],
        landarea: &[f64],
        max_class: usize,
    ) -> Result<Vec<f64>> {
        ensure!(
            patch_elements.len() == self.len(),
            "LULCC diagnostic patch elements must match patch count"
        );
        let (mut result, patch_area, classes) =
            self.lulcc_source_area_by_patch(previous_class, landarea, max_class)?;
        let mut element_area = BTreeMap::<usize, f64>::new();
        for patch in 0..self.len() {
            let element = patch_elements[patch];
            ensure!(element > 0, "LULCC patch {patch} has zero element index");
            if self.wmo_source[patch].is_some() {
                continue;
            }
            *element_area.entry(element).or_insert(0.0) += patch_area[patch];
        }
        for patch in 0..self.len() {
            if self.wmo_source[patch].is_some() {
                continue;
            }
            let total = *element_area
                .get(&patch_elements[patch])
                .with_context(|| format!("LULCC patch {patch} has no element area"))?;
            ensure!(
                total > 0.0 && total.is_finite(),
                "LULCC patch {patch} has zero or non-finite element land area"
            );
            for class in 0..classes {
                result[class * self.len() + patch] /= total;
            }
        }
        Ok(result)
    }

    fn lulcc_source_area_by_patch(
        &self,
        previous_class: &[i32],
        landarea: &[f64],
        max_class: usize,
    ) -> Result<(Vec<f64>, Vec<f64>, usize)> {
        ensure!(
            previous_class.len() == landarea.len(),
            "LULCC previous class and landarea must have the same raw cell count"
        );
        let classes = max_class
            .checked_add(1)
            .context("LULCC class count overflow")?;
        let mut source_area = vec![0.0; classes * self.len()];
        let mut patch_area = vec![0.0; self.len()];
        for patch in 0..self.len() {
            if self.wmo_source[patch].is_some() {
                continue;
            }
            for &cell in &self.cells[self.cells_for(patch)] {
                let class = previous_class.get(cell).copied().with_context(|| {
                    format!(
                        "LULCC patch {patch} references raw cell {cell}, but previous classes have {} cells",
                        previous_class.len()
                    )
                })?;
                let class = usize::try_from(class)
                    .with_context(|| format!("LULCC patch {patch} has a negative source class"))?;
                ensure!(
                    class <= max_class,
                    "LULCC patch {patch} has source class {class} outside 0..={max_class}"
                );
                let area = value(landarea, cell, "landarea", patch)?;
                ensure!(
                    area.is_finite() && area >= 0.0,
                    "LULCC patch {patch} has a non-finite or negative land area"
                );
                patch_area[patch] += area;
                source_area[class * self.len() + patch] += area;
            }
        }
        Ok((source_area, patch_area, classes))
    }

    /// Area-weighted patch LAI or SAI from `Aggregation_LAI`'s LCT path.
    ///
    /// This deliberately does not share WMO values: the corresponding
    /// Fortran branch requests each patch's cells independently.
    pub fn aggregate_patch_vegetation_index(
        &self,
        raw_index: &[f64],
        landarea: &[f64],
    ) -> Result<Vec<f64>> {
        ensure!(
            raw_index.len() == landarea.len(),
            "vegetation index and landarea must have the same raw cell count"
        );
        let mut result = vec![0.0; self.len()];
        for (patch, output) in result.iter_mut().enumerate() {
            let mut area_sum = 0.0;
            let mut index_sum = 0.0;
            for &cell in &self.cells[self.cells_for(patch)] {
                let index = value(raw_index, cell, "vegetation index", patch)?;
                let area = value(landarea, cell, "landarea", patch)?;
                ensure!(
                    index.is_finite() && area.is_finite() && area >= 0.0,
                    "vegetation-index patch {patch} has a non-finite value or invalid land area"
                );
                area_sum += area;
                index_sum = index.mul_add(area, index_sum);
            }
            ensure!(
                area_sum > 0.0 && area_sum.is_finite(),
                "vegetation-index patch {patch} has zero or non-finite land area"
            );
            *output = index_sum / area_sum;
        }
        Ok(result)
    }

    /// Port of one wavelength in `Aggregation_SoilHyperAlbedo`.
    ///
    /// Raw values use CoLM's integer-like scale of 10,000.  The source
    /// routine transforms that scale before taking the unweighted median and
    /// leaves water and ice at the standard surface missing marker.
    pub fn aggregate_soil_hyper_albedo(
        &self,
        raw_albedo_x10k: &[f64],
        waterbody_type: i32,
        ice_type: i32,
    ) -> Result<Vec<f64>> {
        let mut result = vec![SURFACE_MISSING; self.len()];
        let mut scratch = Vec::new();
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            if self.patch_types[patch] == waterbody_type || self.patch_types[patch] == ice_type {
                continue;
            }
            scratch.clear();
            self.gather(raw_albedo_x10k, patch, &mut scratch)?;
            for value in &mut scratch {
                ensure!(
                    value.is_finite(),
                    "hyper-albedo patch {patch} contains a non-finite raw value"
                );
                *value /= 10_000.0;
            }
            result[patch] = median(&mut scratch)
                .with_context(|| format!("hyper-albedo patch {patch} has no raw cells"))?;
        }
        Ok(result)
    }

    /// Port of `Aggregation_TopographyFactors_Simple`.
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_simple_topography_factors(
        &self,
        curvature: &[f64],
        slope_by_aspect: &[f64],
        aspect_by_aspect: &[f64],
        aspect_types: usize,
        landarea: &[f64],
    ) -> Result<SimpleTopographyFactors> {
        ensure!(
            aspect_types > 0
                && curvature.len() == landarea.len()
                && slope_by_aspect.len() == aspect_types * landarea.len()
                && aspect_by_aspect.len() == aspect_types * landarea.len(),
            "simple topography fields must be type-major and match raw cell count"
        );
        ensure!(
            landarea.iter().all(|area| area.is_finite() && *area >= 0.0),
            "simple topography land area must be finite and non-negative"
        );
        let mut output = SimpleTopographyFactors {
            curvature: vec![SURFACE_MISSING; self.len()],
            slope_by_aspect: vec![SURFACE_MISSING; aspect_types * self.len()],
            aspect_by_aspect: vec![SURFACE_MISSING; aspect_types * self.len()],
            aspect_types,
        };
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                output.curvature[patch] = output.curvature[source];
                for aspect in 0..aspect_types {
                    output.slope_by_aspect[aspect * self.len() + patch] =
                        output.slope_by_aspect[aspect * self.len() + source];
                    output.aspect_by_aspect[aspect * self.len() + patch] =
                        output.aspect_by_aspect[aspect * self.len() + source];
                }
                continue;
            }
            output.curvature[patch] = weighted_not_missing(
                self.raw_cells(patch),
                curvature,
                landarea,
                0,
                patch,
                "curvature",
            )?;
            for aspect in 0..aspect_types {
                output.slope_by_aspect[aspect * self.len() + patch] = weighted_not_missing(
                    self.raw_cells(patch),
                    slope_by_aspect,
                    landarea,
                    aspect,
                    patch,
                    "slope",
                )?;
                output.aspect_by_aspect[aspect * self.len() + patch] = weighted_not_missing(
                    self.raw_cells(patch),
                    aspect_by_aspect,
                    landarea,
                    aspect,
                    patch,
                    "aspect",
                )?;
            }
        }
        Ok(output)
    }

    /// Port of `Aggregation_TopographyFactors` for regular forcing downscaling.
    #[allow(clippy::too_many_arguments)]
    pub fn aggregate_regular_topography_factors(
        &self,
        slope: &[f64],
        aspect: &[f64],
        sky_view_factor: &[f64],
        curvature: &[f64],
        terrain_angle_front: &[f64],
        terrain_angle_back: &[f64],
        landarea: &[f64],
    ) -> Result<RegularTopographyFactors> {
        let cells = slope.len();
        ensure!(
            cells > 0
                && aspect.len() == cells
                && sky_view_factor.len() == cells
                && curvature.len() == cells
                && landarea.len() == cells
                && terrain_angle_front.len() == REGULAR_AZIMUTHS * cells
                && terrain_angle_back.len() == REGULAR_AZIMUTHS * cells,
            "regular topography fields must match the raw cell count and 16 azimuths"
        );
        ensure!(
            landarea.iter().all(|area| area.is_finite() && *area >= 0.0),
            "regular topography land area must be finite and non-negative"
        );
        let patches = self.len();
        let mut output = RegularTopographyFactors {
            sky_view_factor: vec![SURFACE_MISSING; patches],
            curvature: vec![SURFACE_MISSING; patches],
            slope_type: vec![0.0; REGULAR_SLOPE_TYPES * patches],
            aspect_type: vec![0.0; REGULAR_SLOPE_TYPES * patches],
            area_type: vec![0.0; REGULAR_SLOPE_TYPES * patches],
            shadow_curve: vec![0.0; REGULAR_AZIMUTHS * REGULAR_CURVE_PARAMETERS * patches],
        };
        for patch in 0..patches {
            if let Some(source) = self.wmo_source[patch] {
                output.sky_view_factor[patch] = output.sky_view_factor[source];
                output.curvature[patch] = output.curvature[source];
                for kind in 0..REGULAR_SLOPE_TYPES {
                    output.slope_type[kind * patches + patch] =
                        output.slope_type[kind * patches + source];
                    output.aspect_type[kind * patches + patch] =
                        output.aspect_type[kind * patches + source];
                    output.area_type[kind * patches + patch] =
                        output.area_type[kind * patches + source];
                }
                for azimuth in 0..REGULAR_AZIMUTHS {
                    for parameter in 0..REGULAR_CURVE_PARAMETERS {
                        let offset = (azimuth * REGULAR_CURVE_PARAMETERS + parameter) * patches;
                        output.shadow_curve[offset + patch] = output.shadow_curve[offset + source];
                    }
                }
                continue;
            }
            output.sky_view_factor[patch] = weighted_not_missing(
                self.raw_cells(patch),
                sky_view_factor,
                landarea,
                0,
                patch,
                "sky-view factor",
            )?;
            output.curvature[patch] = weighted_not_missing(
                self.raw_cells(patch),
                curvature,
                landarea,
                0,
                patch,
                "curvature",
            )?;
            let shadow_lut = regular_shadow_lut(
                self.raw_cells(patch),
                terrain_angle_front,
                terrain_angle_back,
                cells,
                patch,
            )?;
            for azimuth in 0..REGULAR_AZIMUTHS {
                let curve = regular_shadow_curve(
                    &shadow_lut[azimuth * REGULAR_ZENITHS..(azimuth + 1) * REGULAR_ZENITHS],
                );
                for (parameter, value) in curve.into_iter().enumerate() {
                    output.shadow_curve
                        [(azimuth * REGULAR_CURVE_PARAMETERS + parameter) * patches + patch] =
                        value;
                }
            }
            let total_area = self
                .raw_cells(patch)
                .iter()
                .map(|&cell| landarea[cell])
                .filter(|area| *area > 0.0)
                .sum::<f64>();
            if total_area == 0.0 {
                continue;
            }
            for &cell in self.raw_cells(patch) {
                let slope = value(slope, cell, "slope", patch)?;
                let aspect = value(aspect, cell, "aspect", patch)?;
                ensure!(
                    (slope.is_finite() && aspect.is_finite())
                        || (slope == -9999.0 && aspect == -9999.0),
                    "regular topography patch {patch} contains a non-finite slope or aspect"
                );
                let kind = regular_slope_type(slope, aspect);
                let Some(kind) = kind else {
                    continue;
                };
                let area = landarea[cell];
                if area == 0.0 || area > total_area {
                    continue;
                }
                output.area_type[kind * patches + patch] += area / total_area;
                output.aspect_type[kind * patches + patch] += aspect * area / total_area;
                output.slope_type[kind * patches + patch] += slope * area / total_area;
            }
        }
        Ok(output)
    }

    /// Port of `Aggregation_DBedrock`.
    ///
    /// The raw field has no fill-value masking in the Fortran routine, so this
    /// deliberately uses every cell in a patch and applies area weighting.
    pub fn aggregate_bedrock(&self, depth: &[f64], landarea: &[f64]) -> Result<Vec<f64>> {
        self.aggregate_area_weighted(depth, landarea, "bedrock")
    }

    /// Port of the USGS branch of `Aggregation_ForestHeight`.
    ///
    /// The reference does not use WMO sharing for this path: every eligible
    /// patch takes the raw-cell median independently.  Ocean, urban, water,
    /// and ice receive CoLM's missing marker.
    pub fn aggregate_usgs_forest_height(
        &self,
        height_m: &[f64],
        urban_type: i32,
        waterbody_type: i32,
        ice_type: i32,
    ) -> Result<Vec<f64>> {
        let mut result = vec![SURFACE_MISSING; self.len()];
        let mut scratch = Vec::new();
        for (patch, output) in result.iter_mut().enumerate() {
            let patch_type = self.patch_types[patch];
            if patch_type == 0
                || patch_type == urban_type
                || patch_type == waterbody_type
                || patch_type == ice_type
            {
                continue;
            }
            scratch.clear();
            self.gather(height_m, patch, &mut scratch)?;
            *output = median(&mut scratch)
                .with_context(|| format!("forest-height patch {patch} has no raw cells"))?;
        }
        Ok(result)
    }

    /// Patch height shared by IGBP LCT and PFT/PC: area-weighted physical
    /// patches, with virtual PFT/PC WMO patches copying their source height.
    /// LCT disables WMO in the namelist before building the topology.
    pub fn aggregate_igbp_forest_height(
        &self,
        height_m: &[f64],
        landarea: &[f64],
    ) -> Result<Vec<f64>> {
        let mut result = vec![SURFACE_MISSING; self.len()];
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            if self.patch_types[patch] == 0 {
                continue;
            }
            let mut area_sum = 0.0;
            let mut height_sum = 0.0;
            for &cell in &self.cells[self.cells_for(patch)] {
                let height = value(height_m, cell, "forest height", patch)?;
                let area = value(landarea, cell, "landarea", patch)?;
                ensure!(
                    height.is_finite() && area.is_finite() && area >= 0.0,
                    "forest-height patch {patch} has a non-finite value or invalid land area"
                );
                area_sum += area;
                height_sum = height.mul_add(area, height_sum);
            }
            ensure!(
                area_sum > 0.0 && area_sum.is_finite(),
                "forest-height patch {patch} has zero or non-finite land area"
            );
            result[patch] = height_sum / area_sum;
        }
        Ok(result)
    }

    /// Port of the four median operations in `Aggregation_SoilBrightness`.
    ///
    /// The already-portioned CoLM colour lookup tables live in `albedo.rs`.
    /// Invalid raw colour classes remain missing; `median(..., spval)` excludes
    /// them rather than treating them as a numeric albedo.  Water and ice
    /// patch identifiers are parameters because the USGS and IGBP kernels use
    /// different values.
    pub fn aggregate_soil_brightness(
        &self,
        colour: &[i32],
        waterbody_type: i32,
        ice_type: i32,
    ) -> Result<SoilBrightness> {
        let mut result = SoilBrightness {
            saturated_visible: vec![SURFACE_MISSING; self.len()],
            dry_visible: vec![SURFACE_MISSING; self.len()],
            saturated_near_infrared: vec![SURFACE_MISSING; self.len()],
            dry_near_infrared: vec![SURFACE_MISSING; self.len()],
        };
        let mut saturated_visible = Vec::new();
        let mut dry_visible = Vec::new();
        let mut saturated_near_infrared = Vec::new();
        let mut dry_near_infrared = Vec::new();

        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result.saturated_visible[patch] = result.saturated_visible[source];
                result.dry_visible[patch] = result.dry_visible[source];
                result.saturated_near_infrared[patch] = result.saturated_near_infrared[source];
                result.dry_near_infrared[patch] = result.dry_near_infrared[source];
                continue;
            }
            let patch_type = self.patch_types[patch];
            if patch_type == waterbody_type || patch_type == ice_type {
                continue;
            }

            saturated_visible.clear();
            dry_visible.clear();
            saturated_near_infrared.clear();
            dry_near_infrared.clear();
            let range = self.cells_for(patch);
            saturated_visible.reserve(range.len());
            dry_visible.reserve(range.len());
            saturated_near_infrared.reserve(range.len());
            dry_near_infrared.reserve(range.len());
            for &cell in &self.cells[range] {
                let raw_colour = colour.get(cell).copied().with_context(|| {
                    format!(
                        "soil brightness patch {patch} references raw cell {cell}, but input has {} cells",
                        colour.len()
                    )
                })?;
                let reflectance = albedo(raw_colour, 0);
                saturated_visible.push(reflectance.map_or(SURFACE_MISSING, |v| v.s_v));
                dry_visible.push(reflectance.map_or(SURFACE_MISSING, |v| v.d_v));
                saturated_near_infrared.push(reflectance.map_or(SURFACE_MISSING, |v| v.s_n));
                dry_near_infrared.push(reflectance.map_or(SURFACE_MISSING, |v| v.d_n));
            }
            result.saturated_visible[patch] =
                median_excluding(&mut saturated_visible, SURFACE_MISSING)?;
            result.dry_visible[patch] = median_excluding(&mut dry_visible, SURFACE_MISSING)?;
            result.saturated_near_infrared[patch] =
                median_excluding(&mut saturated_near_infrared, SURFACE_MISSING)?;
            result.dry_near_infrared[patch] =
                median_excluding(&mut dry_near_infrared, SURFACE_MISSING)?;
        }
        Ok(result)
    }

    /// Port of `Aggregation_Topography`.
    ///
    /// The elevation mean and slope ratio are weighted by raw `landarea`.
    /// Elevation standard deviation combines between-cell elevation variance
    /// with each raw cell's supplied standard deviation exactly as CoLM does.
    /// An all-`-9999` elevation patch maps to zero for all three outputs.
    pub fn aggregate_topography(
        &self,
        landarea: &[f64],
        elevation: &[f64],
        elevation_std: &[f64],
        slope_ratio: &[f64],
    ) -> Result<Topography> {
        let mut result = Topography {
            elevation: vec![0.0; self.len()],
            elevation_std: vec![0.0; self.len()],
            slope_ratio: vec![0.0; self.len()],
        };

        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result.elevation[patch] = result.elevation[source];
                result.elevation_std[patch] = result.elevation_std[source];
                result.slope_ratio[patch] = result.slope_ratio[source];
                continue;
            }

            let range = self.cells_for(patch);
            let mut area_sum = 0.0;
            let mut elevation_sum = 0.0;
            let mut any_valid = false;
            for &cell in &self.cells[range.clone()] {
                let height = value(elevation, cell, "elevation", patch)?;
                if height == -9999.0 {
                    continue;
                }
                let area = value(landarea, cell, "landarea", patch)?;
                ensure!(
                    area.is_finite() && area >= 0.0,
                    "topography patch {patch} has invalid land area {area}"
                );
                ensure!(
                    height.is_finite(),
                    "topography patch {patch} has a non-finite elevation"
                );
                any_valid = true;
                area_sum += area;
                elevation_sum = height.mul_add(area, elevation_sum);
            }
            if !any_valid {
                continue;
            }
            ensure!(
                area_sum > 0.0 && area_sum.is_finite(),
                "topography patch {patch} has zero or non-finite valid land area"
            );
            let mean = elevation_sum / area_sum;
            let mut variance_sum = 0.0;
            let mut slope_sum = 0.0;
            for &cell in &self.cells[range] {
                let height = value(elevation, cell, "elevation", patch)?;
                if height == -9999.0 {
                    continue;
                }
                let area = value(landarea, cell, "landarea", patch)?;
                let std = value(elevation_std, cell, "elevation standard deviation", patch)?;
                let slope = value(slope_ratio, cell, "slope ratio", patch)?;
                ensure!(
                    std.is_finite() && slope.is_finite(),
                    "topography patch {patch} contains a non-finite standard deviation or slope"
                );
                let variance = (height - mean).mul_add(height - mean, std * std);
                variance_sum = variance.mul_add(area, variance_sum);
                slope_sum = slope.mul_add(area, slope_sum);
            }
            result.elevation[patch] = mean;
            result.elevation_std[patch] = (variance_sum / area_sum).sqrt();
            result.slope_ratio[patch] = slope_sum / area_sum;
        }
        Ok(result)
    }

    fn aggregate_area_weighted(
        &self,
        values: &[f64],
        landarea: &[f64],
        field: &str,
    ) -> Result<Vec<f64>> {
        let mut result = vec![0.0; self.len()];
        for patch in 0..self.len() {
            if let Some(source) = self.wmo_source[patch] {
                result[patch] = result[source];
                continue;
            }
            let mut area_sum = 0.0;
            let mut value_sum = 0.0;
            for &cell in &self.cells[self.cells_for(patch)] {
                let field_value = value(values, cell, field, patch)?;
                let area = value(landarea, cell, "landarea", patch)?;
                ensure!(
                    field_value.is_finite() && area.is_finite() && area >= 0.0,
                    "{field} patch {patch} has a non-finite value or invalid land area"
                );
                area_sum += area;
                value_sum += field_value * area;
            }
            ensure!(
                area_sum > 0.0 && area_sum.is_finite(),
                "{field} patch {patch} has zero or non-finite land area"
            );
            result[patch] = value_sum / area_sum;
        }
        Ok(result)
    }

    fn cells_for(&self, patch: usize) -> std::ops::Range<usize> {
        self.cell_offsets[patch]..self.cell_offsets[patch + 1]
    }

    pub(crate) fn raw_cells(&self, patch: usize) -> &[usize] {
        &self.cells[self.cells_for(patch)]
    }

    pub(crate) fn wmo_source_for(&self, patch: usize) -> Option<usize> {
        self.wmo_source[patch]
    }

    pub(crate) fn patch_type_for(&self, patch: usize) -> i32 {
        self.patch_types[patch]
    }

    fn gather<T: Copy>(&self, source: &[T], patch: usize, out: &mut Vec<T>) -> Result<()> {
        let range = self.cells_for(patch);
        out.reserve(range.len());
        for &cell in &self.cells[range] {
            out.push(source.get(cell).copied().with_context(|| {
                format!(
                    "patch {patch} references raw cell {cell}, but input has {} cells",
                    source.len()
                )
            })?);
        }
        Ok(())
    }
}

fn value(source: &[f64], cell: usize, name: &str, patch: usize) -> Result<f64> {
    source.get(cell).copied().with_context(|| {
        format!(
            "{name} patch {patch} references raw cell {cell}, but input has {} cells",
            source.len()
        )
    })
}

fn regular_shadow_lut(
    cells: &[usize],
    front: &[f64],
    back: &[f64],
    raw_cells: usize,
    patch: usize,
) -> Result<Vec<f64>> {
    let mut output = vec![0.0; REGULAR_AZIMUTHS * REGULAR_ZENITHS];
    for azimuth in 0..REGULAR_AZIMUTHS {
        for zenith in 0..REGULAR_ZENITHS {
            let zenith_angle =
                std::f64::consts::PI * zenith as f64 / (2.0 * REGULAR_ZENITHS as f64);
            let sun_altitude = std::f64::consts::FRAC_PI_2 - zenith_angle;
            let mut sum = 0.0;
            let mut valid = 0_usize;
            for &cell in cells {
                let front = front[azimuth * raw_cells + cell];
                let back = back[azimuth * raw_cells + cell];
                if front.is_nan() || back.is_nan() {
                    sum += 1.0;
                    continue;
                }
                ensure!(
                    front.is_finite() && back.is_finite(),
                    "regular topography patch {patch} has an infinite terrain angle"
                );
                let mut front = front.clamp(-1.0, 1.0).asin();
                let back = back.clamp(-1.0, 1.0).asin();
                valid += 1;
                let shadow = if sun_altitude < back {
                    0.0
                } else if sun_altitude > front {
                    1.0
                } else {
                    if front == back {
                        front += 0.001;
                    }
                    (sun_altitude - back) / (front - back)
                };
                sum += shadow;
            }
            ensure!(
                valid > 0,
                "regular topography patch {patch} has no finite terrain-angle samples"
            );
            output[azimuth * REGULAR_ZENITHS + zenith] = sum / valid as f64;
        }
    }
    Ok(output)
}

fn regular_shadow_curve(lut: &[f64]) -> [f64; REGULAR_CURVE_PARAMETERS] {
    debug_assert_eq!(lut.len(), REGULAR_ZENITHS);
    let mut index = 1_usize;
    for zenith in 0..REGULAR_ZENITHS - 1 {
        if lut[zenith] == 1.0 && lut[zenith + 1] < 1.0 {
            index = zenith + 1;
        }
    }
    let x = (index..REGULAR_ZENITHS)
        .map(|zenith| std::f64::consts::PI * zenith as f64 / (2.0 * REGULAR_ZENITHS as f64))
        .collect::<Vec<_>>();
    let y = lut[index..]
        .iter()
        .map(|value| (-value.clamp(0.001, 0.999).ln()).ln())
        .collect::<Vec<_>>();
    let count = x.len() as f64;
    let x_sum = x.iter().sum::<f64>();
    let y_sum = y.iter().sum::<f64>();
    let x2_sum = x.iter().map(|value| value * value).sum::<f64>();
    let xy_sum = x.iter().zip(&y).map(|(x, y)| x * y).sum::<f64>();
    let denominator = count * x2_sum - x_sum * x_sum;
    let (a1, a2) = if denominator == 0.0 {
        (0.0, 0.0)
    } else {
        let a1 = (count * xy_sum - x_sum * y_sum) / denominator;
        (a1, (y_sum - a1 * x_sum) / count)
    };
    [
        std::f64::consts::PI * (index - 1) as f64 / (2.0 * REGULAR_ZENITHS as f64),
        a1,
        a2,
    ]
}

fn regular_slope_type(slope: f64, aspect: f64) -> Option<usize> {
    let north = (0.0..=std::f64::consts::FRAC_PI_2).contains(&aspect)
        || (3.0 * std::f64::consts::FRAC_PI_2..=std::f64::consts::TAU).contains(&aspect);
    if north {
        Some(if slope >= std::f64::consts::PI / 12.0 {
            0
        } else {
            1
        })
    } else if (std::f64::consts::FRAC_PI_2..3.0 * std::f64::consts::FRAC_PI_2).contains(&aspect) {
        Some(if slope >= std::f64::consts::PI / 12.0 {
            2
        } else {
            3
        })
    } else {
        None
    }
}

fn weighted_not_missing(
    cells: &[usize],
    source: &[f64],
    landarea: &[f64],
    component: usize,
    patch: usize,
    name: &str,
) -> Result<f64> {
    let raw_cells = landarea.len();
    let mut area_sum = 0.0;
    let mut value_sum = 0.0;
    for &cell in cells {
        let value = source
            .get(component * raw_cells + cell)
            .copied()
            .with_context(|| {
                format!("{name} patch {patch} references raw component {component}, cell {cell}")
            })?;
        if value == -9999.0 {
            continue;
        }
        ensure!(
            value.is_finite(),
            "{name} patch {patch} contains a non-finite value"
        );
        let area = landarea[cell];
        area_sum += area;
        value_sum += value * area;
    }
    Ok(if area_sum > 0.0 {
        value_sum / area_sum
    } else {
        SURFACE_MISSING
    })
}

fn median(values: &mut [f64]) -> Result<f64> {
    if values.is_empty() {
        bail!("cannot calculate a median of zero values");
    }
    if values.iter().any(|value| !value.is_finite()) {
        bail!("cannot calculate a median containing non-finite values");
    }
    let upper = values.len() / 2;
    if values.len() % 2 == 1 {
        return Ok(*values.select_nth_unstable_by(upper, f64::total_cmp).1);
    }
    let left = *values.select_nth_unstable_by(upper - 1, f64::total_cmp).1;
    let right = *values.select_nth_unstable_by(upper, f64::total_cmp).1;
    Ok((left + right) * 0.5)
}

/// CoLM's `median(x, n, spval)`: an all-missing patch returns the marker.
fn median_excluding(values: &mut Vec<f64>, marker: f64) -> Result<f64> {
    values.retain(|value| *value != marker);
    if values.is_empty() {
        Ok(marker)
    } else {
        median(values)
    }
}

pub(crate) fn most_frequent(values: &mut [i32]) -> Result<i32> {
    if values.is_empty() {
        bail!("cannot select a most frequent value from zero values");
    }
    values.sort_unstable();
    let (mut best_value, mut best_count) = (values[0], 1_usize);
    let (mut run_value, mut run_count) = (values[0], 1_usize);
    for &value in &values[1..] {
        if value == run_value {
            run_count += 1;
        } else {
            if run_count > best_count {
                (best_value, best_count) = (run_value, run_count);
            }
            (run_value, run_count) = (value, 1);
        }
    }
    if run_count > best_count {
        best_value = run_value;
    }
    Ok(best_value)
}

#[cfg(test)]
#[path = "surface_tests.rs"]
mod tests;
