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

    /// Port of the IGBP LCT branch of `Aggregation_ForestHeight`.
    ///
    /// This branch uses raw-cell land-area weighting and intentionally does
    /// not inherit a WMO source; that is also how the Fortran LCT path works.
    pub fn aggregate_igbp_forest_height(
        &self,
        height_m: &[f64],
        landarea: &[f64],
    ) -> Result<Vec<f64>> {
        let mut result = vec![SURFACE_MISSING; self.len()];
        for (patch, output) in result.iter_mut().enumerate() {
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
                height_sum += height * area;
            }
            ensure!(
                area_sum > 0.0 && area_sum.is_finite(),
                "forest-height patch {patch} has zero or non-finite land area"
            );
            *output = height_sum / area_sum;
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
                elevation_sum += height * area;
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
                variance_sum += ((height - mean).mul_add(height - mean, std * std)) * area;
                slope_sum += slope * area;
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

fn most_frequent(values: &mut [i32]) -> Result<i32> {
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
