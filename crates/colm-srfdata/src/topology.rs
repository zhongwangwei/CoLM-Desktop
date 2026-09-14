//! Flattened spatial topology for the Rust `mksrfdata` port.
//!
//! `MOD_Mesh.F90` represents every element as a separate derived type with two
//! independently allocated coordinate arrays.  `MOD_LandElm.F90` then creates
//! a pixelset that refers back to those allocations.  This module preserves
//! that data contract in two contiguous arrays per coordinate plus CSR offsets.
//! It is the ownership boundary for the later patch/PFT/urban builders.

use std::collections::HashSet;

use anyhow::{ensure, Context, Result};

use crate::surface::FlatPatches;

/// Element-to-pixel topology equivalent to CoLM's `mesh(:)`.
///
/// The coordinates are CoLM's one-based raw-pixel indices.  Element `e` owns
/// the interval `pixel_offsets[e]..pixel_offsets[e + 1]`; `ilon` and `ilat`
/// use the same flat index.  IDs are signed 64-bit because unstructured mesh
/// indices are read as Fortran `integer*8`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatMesh {
    element_ids: Vec<i64>,
    pixel_offsets: Vec<usize>,
    ilon: Vec<i32>,
    ilat: Vec<i32>,
}

/// The `landelm` pixelset created by `MOD_LandElm::landelm_build`.
///
/// `pixel_start`, `pixel_end`, and `element_index` intentionally retain the
/// one-based values written by Fortran.  The actual coordinate storage remains
/// zero-based CSR in [`FlatMesh`], avoiding a second pixel list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatLandElements {
    pub element_ids: Vec<i64>,
    pub pixel_start: Vec<usize>,
    pub pixel_end: Vec<usize>,
    pub set_type: Vec<i32>,
    pub element_index: Vec<usize>,
}

/// The flattened result of `MOD_LandPatch::landpatch_build`.
///
/// Ranges are one-based and inclusive because they are part of CoLM's on-disk
/// pixelset contract.  The referenced [`FlatMesh`] owns the reordered pixel
/// coordinates; this structure stores no duplicate coordinate vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlatLandPatches {
    pub element_ids: Vec<i64>,
    pub pixel_start: Vec<usize>,
    pub pixel_end: Vec<usize>,
    pub set_type: Vec<i32>,
    pub element_index: Vec<usize>,
}

/// The `landhru` pixelset created by `MOD_LandHRU::landhru_build`.
///
/// Its flat field layout is identical to a land-patch pixelset, but its
/// `set_type` is the hydrologic response unit ID, negated for lake catchments.
pub type FlatLandHrus = FlatLandPatches;

impl FlatMesh {
    /// Construct and validate one immutable mesh topology.
    pub fn new(
        element_ids: Vec<i64>,
        pixel_offsets: Vec<usize>,
        ilon: Vec<i32>,
        ilat: Vec<i32>,
    ) -> Result<Self> {
        ensure!(
            pixel_offsets.len() == element_ids.len() + 1,
            "pixel_offsets must have one entry more than element_ids"
        );
        ensure!(
            pixel_offsets.first() == Some(&0),
            "pixel_offsets must begin at zero"
        );
        ensure!(
            pixel_offsets.last() == Some(&ilon.len()),
            "last pixel offset must equal the number of longitude indices"
        );
        ensure!(
            ilon.len() == ilat.len(),
            "longitude and latitude index arrays must have the same length"
        );
        ensure!(
            element_ids.iter().all(|id| *id > 0),
            "mesh element IDs must be positive"
        );
        ensure!(
            pixel_offsets.windows(2).all(|pair| pair[0] < pair[1]),
            "every mesh element must own at least one pixel"
        );
        ensure!(
            ilon.iter().all(|index| *index > 0) && ilat.iter().all(|index| *index > 0),
            "mesh pixel coordinates must be one-based positive indices"
        );
        let mut seen = HashSet::with_capacity(element_ids.len());
        for id in &element_ids {
            ensure!(seen.insert(*id), "mesh element ID {id} is duplicated");
        }
        Ok(Self {
            element_ids,
            pixel_offsets,
            ilon,
            ilat,
        })
    }

    pub fn len(&self) -> usize {
        self.element_ids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.element_ids.is_empty()
    }

    pub fn element_id(&self, element: usize) -> Result<i64> {
        self.element_ids
            .get(element)
            .copied()
            .with_context(|| format!("mesh element {element} is outside 0..{}", self.len()))
    }

    pub fn pixel_count(&self, element: usize) -> Result<usize> {
        let end = *self
            .pixel_offsets
            .get(element + 1)
            .with_context(|| format!("mesh element {element} is outside 0..{}", self.len()))?;
        Ok(end - self.pixel_offsets[element])
    }

    /// Return the one-based `(ilon, ilat)` cells owned by an element.
    pub fn pixels(&self, element: usize) -> Result<(&[i32], &[i32])> {
        let start = *self
            .pixel_offsets
            .get(element)
            .with_context(|| format!("mesh element {element} is outside 0..{}", self.len()))?;
        let end = self.pixel_offsets[element + 1];
        Ok((&self.ilon[start..end], &self.ilat[start..end]))
    }

    /// In-place MOD_MeshFilter compaction; drop ocean pixels and empty elements
    /// without allocating another mesh-sized coordinate or raster buffer.
    pub(crate) fn retain_land_pixels(&mut self, types: &mut Vec<i32>) -> Result<()> {
        ensure!(
            types.len() == self.ilon.len(),
            "landtype must match mesh pixel count"
        );
        let mut output = 0;
        let mut elements = 0;
        for element in 0..self.len() {
            let start = self.pixel_offsets[element];
            let end = self.pixel_offsets[element + 1];
            let kept_start = output;
            for input in start..end {
                if types[input] > 0 {
                    self.ilon[output] = self.ilon[input];
                    self.ilat[output] = self.ilat[input];
                    types[output] = types[input];
                    output += 1;
                }
            }
            if output > kept_start {
                self.element_ids[elements] = self.element_ids[element];
                self.pixel_offsets[elements] = kept_start;
                elements += 1;
            }
        }
        self.pixel_offsets[elements] = output;
        self.pixel_offsets.truncate(elements + 1);
        self.element_ids.truncate(elements);
        self.ilon.truncate(output);
        self.ilat.truncate(output);
        types.truncate(output);
        Ok(())
    }

    /// Build the exact structural fields written by `landelm_build`.
    pub fn land_elements(&self) -> FlatLandElements {
        let mut pixel_end = Vec::with_capacity(self.len());
        for element in 0..self.len() {
            // Construction guarantees this cannot fail.
            pixel_end.push(self.pixel_offsets[element + 1] - self.pixel_offsets[element]);
        }
        FlatLandElements {
            element_ids: self.element_ids.clone(),
            pixel_start: vec![1; self.len()],
            pixel_end,
            set_type: vec![0; self.len()],
            element_index: (1..=self.len()).collect(),
        }
    }

    /// Port `MOD_LandHRU::landhru_build` after catchment pixels reach a flat mesh.
    ///
    /// `hydrounit_types` is one positive `ihydrounit2d` value per mesh pixel in
    /// current mesh order. `lake_id_by_element` is one `lake_id` per catchment
    /// element. As upstream does, lake-catchment HRU IDs are written negative.
    pub fn into_land_hrus(
        self,
        hydrounit_types: &[i32],
        lake_id_by_element: &[i32],
    ) -> Result<(Self, FlatLandHrus)> {
        ensure!(
            hydrounit_types.len() == self.ilon.len()
                && hydrounit_types.iter().all(|&value| value > 0),
            "catchment hydrologic-unit types must be positive and match mesh pixels"
        );
        ensure!(
            lake_id_by_element.len() == self.len(),
            "catchment lake IDs must have one entry per mesh element"
        );
        let (mesh, mut hrus) = self.into_land_patches(hydrounit_types, false)?;
        for (hru, set_type) in hrus.set_type.iter_mut().enumerate() {
            let element = hrus.element_index[hru]
                .checked_sub(1)
                .context("catchment HRU has zero element index")?;
            if lake_id_by_element[element] > 0 {
                *set_type = -*set_type;
            }
        }
        Ok((mesh, hrus))
    }

    /// Port the core of landpatch_build for a raw land-type vector already
    /// gathered in this mesh's flat pixel order.
    ///
    /// CoLM sorts every element by land type and reorders its pixel coordinates
    /// in the same permutation, so subsequent aggregations can address one
    /// contiguous range per patch. This consumes the mesh to make that
    /// mutation explicit and returns it with the resulting patch pixelset.
    /// dominant_type matches DEF_USE_DOMINANT_PATCHTYPE.
    pub fn into_land_patches(
        self,
        land_types: &[i32],
        dominant_type: bool,
    ) -> Result<(Self, FlatLandPatches)> {
        let parent_sets = self.land_elements();
        let parent_sets = FlatLandPatches {
            element_ids: parent_sets.element_ids,
            pixel_start: parent_sets.pixel_start,
            pixel_end: parent_sets.pixel_end,
            set_type: parent_sets.set_type,
            element_index: parent_sets.element_index,
        };
        self.into_land_patches_by_sets(land_types, &parent_sets, dominant_type)
    }

    /// Port landpatch_build for a pre-existing partition of every mesh element.
    ///
    /// CATCHMENT first groups pixels into HRUs; land cover is then sorted only
    /// inside each HRU range, never across HRU boundaries.
    pub fn into_land_patches_by_sets(
        mut self,
        land_types: &[i32],
        parent_sets: &FlatLandPatches,
        dominant_type: bool,
    ) -> Result<(Self, FlatLandPatches)> {
        ensure!(
            land_types.len() == self.ilon.len(),
            "land-type vector has {} cells; mesh has {}",
            land_types.len(),
            self.ilon.len()
        );
        ensure!(
            land_types.iter().all(|kind| *kind >= 0),
            "land-type values must be non-negative"
        );
        ensure!(
            parent_sets.element_ids.len() == parent_sets.len()
                && parent_sets.pixel_start.len() == parent_sets.len()
                && parent_sets.pixel_end.len() == parent_sets.len()
                && parent_sets.element_index.len() == parent_sets.len(),
            "parent pixelset vectors must have equal lengths"
        );

        let mut element_ids = Vec::new();
        let mut pixel_start = Vec::new();
        let mut pixel_end = Vec::new();
        let mut set_type = Vec::new();
        let mut element_index = Vec::new();
        let mut types = Vec::new();
        let mut order = Vec::new();
        let mut sorted_lon = Vec::new();
        let mut sorted_lat = Vec::new();
        let mut covered = vec![false; self.ilon.len()];

        for set in 0..parent_sets.len() {
            let element = parent_sets.element_index[set]
                .checked_sub(1)
                .with_context(|| format!("parent pixelset {set} has zero element index"))?;
            ensure!(
                self.element_id(element)? == parent_sets.element_ids[set],
                "parent pixelset {set} element ID does not match the mesh"
            );
            let count = self.pixel_count(element)?;
            let start = parent_sets.pixel_start[set];
            let end = parent_sets.pixel_end[set];
            ensure!(
                start > 0 && start <= end && end <= count,
                "parent pixelset {set} has invalid one-based pixel range {start}..={end} for element size {count}"
            );
            let offset = self.pixel_offsets[element];
            let range = offset + start - 1..offset + end;
            ensure!(
                covered[range.clone()].iter().all(|seen| !*seen),
                "parent pixelsets overlap at set {set}"
            );
            covered[range.clone()].fill(true);

            types.clear();
            types.extend_from_slice(&land_types[range.clone()]);
            order.clear();
            order.extend(0..types.len());
            colm_quicksort(&mut types, &mut order);
            if dominant_type {
                collapse_to_dominant(&mut types);
            }

            sorted_lon.clear();
            sorted_lat.clear();
            sorted_lon.reserve(order.len());
            sorted_lat.reserve(order.len());
            for &source in &order {
                sorted_lon.push(self.ilon[range.start + source]);
                sorted_lat.push(self.ilat[range.start + source]);
            }
            self.ilon[range.clone()].copy_from_slice(&sorted_lon);
            self.ilat[range.clone()].copy_from_slice(&sorted_lat);

            let mut local_start = 0_usize;
            while local_start < types.len() {
                let kind = types[local_start];
                let mut local_end = local_start + 1;
                while local_end < types.len() && types[local_end] == kind {
                    local_end += 1;
                }
                element_ids.push(self.element_ids[element]);
                pixel_start.push(start + local_start);
                pixel_end.push(start + local_end - 1);
                set_type.push(kind);
                element_index.push(element + 1);
                local_start = local_end;
            }
        }
        ensure!(
            covered.iter().all(|seen| *seen),
            "parent pixelsets do not cover every mesh pixel"
        );
        Ok((
            self,
            FlatLandPatches {
                element_ids,
                pixel_start,
                pixel_end,
                set_type,
                element_index,
            },
        ))
    }

    /// Port `MOD_LandUrban::landurban_build` after the ordinary LCT patches
    /// exist.  Each urban patch is subdivided by the raw urban-density/LCZ
    /// class, while `landpatch` deliberately keeps the ordinary urban land
    /// type and `landurban` carries the density/LCZ type.
    ///
    /// Missing raw classes follow the upstream fill rule exactly, including
    /// its historic exclusion of the first and final class when proportions
    /// are calculated.  That quirk matters for reproducible existing cases.
    #[allow(clippy::too_many_arguments)]
    pub fn into_urban_land_patches(
        mut self,
        patches: &FlatLandPatches,
        urban_types: &[i32],
        cell_area: &[f64],
        urban_land_type: i32,
        urban_class_count: usize,
    ) -> Result<(Self, FlatLandPatches, FlatLandPatches)> {
        ensure!(
            urban_types.len() == self.ilon.len() && cell_area.len() == self.ilon.len(),
            "urban type and cell-area vectors must match the mesh pixel count"
        );
        ensure!(urban_class_count > 0, "urban class count must be positive");
        ensure!(
            cell_area
                .iter()
                .all(|area| area.is_finite() && *area >= 0.0),
            "urban cell areas must be finite and non-negative"
        );
        validate_flat_land_patches(&self, patches)?;

        let mut refined = FlatLandPatches {
            element_ids: Vec::with_capacity(patches.len()),
            pixel_start: Vec::with_capacity(patches.len()),
            pixel_end: Vec::with_capacity(patches.len()),
            set_type: Vec::with_capacity(patches.len()),
            element_index: Vec::with_capacity(patches.len()),
        };
        let mut urban = FlatLandPatches {
            element_ids: Vec::new(),
            pixel_start: Vec::new(),
            pixel_end: Vec::new(),
            set_type: Vec::new(),
            element_index: Vec::new(),
        };

        for patch in 0..patches.len() {
            let element = patches.element_index[patch]
                .checked_sub(1)
                .with_context(|| format!("urban parent patch {patch} has zero element index"))?;
            let mesh_offset = self.pixel_offsets[element];
            let start = patches.pixel_start[patch];
            let end = patches.pixel_end[patch];
            if patches.set_type[patch] != urban_land_type {
                push_patch(
                    &mut refined,
                    patches,
                    patch,
                    start,
                    end,
                    patches.set_type[patch],
                );
                continue;
            }

            let range = mesh_offset + start - 1..mesh_offset + end;
            let mut types = urban_types[range.clone()].to_vec();
            fill_missing_urban_types(&mut types, &cell_area[range.clone()], urban_class_count);
            let mut order: Vec<usize> = (0..types.len()).collect();
            colm_quicksort(&mut types, &mut order);
            let mut sorted_lon = Vec::with_capacity(order.len());
            let mut sorted_lat = Vec::with_capacity(order.len());
            for source in order {
                sorted_lon.push(self.ilon[range.start + source]);
                sorted_lat.push(self.ilat[range.start + source]);
            }
            self.ilon[range.clone()].copy_from_slice(&sorted_lon);
            self.ilat[range].copy_from_slice(&sorted_lat);

            let mut local_start = 0_usize;
            while local_start < types.len() {
                let class = types[local_start];
                let mut local_end = local_start + 1;
                while local_end < types.len() && types[local_end] == class {
                    local_end += 1;
                }
                let refined_start = start + local_start;
                let refined_end = start + local_end - 1;
                push_patch(
                    &mut refined,
                    patches,
                    patch,
                    refined_start,
                    refined_end,
                    urban_land_type,
                );
                push_patch(
                    &mut urban,
                    patches,
                    patch,
                    refined_start,
                    refined_end,
                    class,
                );
                local_start = local_end;
            }
        }
        Ok((self, refined, urban))
    }
}

fn push_patch(
    output: &mut FlatLandPatches,
    source: &FlatLandPatches,
    patch: usize,
    pixel_start: usize,
    pixel_end: usize,
    set_type: i32,
) {
    output.element_ids.push(source.element_ids[patch]);
    output.pixel_start.push(pixel_start);
    output.pixel_end.push(pixel_end);
    output.set_type.push(set_type);
    output.element_index.push(source.element_index[patch]);
}

fn validate_flat_land_patches(mesh: &FlatMesh, patches: &FlatLandPatches) -> Result<()> {
    ensure!(
        patches.element_ids.len() == patches.len()
            && patches.pixel_start.len() == patches.len()
            && patches.pixel_end.len() == patches.len()
            && patches.element_index.len() == patches.len(),
        "land-patch vectors must have equal lengths"
    );
    for patch in 0..patches.len() {
        let element = patches.element_index[patch]
            .checked_sub(1)
            .with_context(|| format!("land patch {patch} has zero element index"))?;
        ensure!(
            mesh.element_id(element)? == patches.element_ids[patch],
            "land patch {patch} element ID does not match the mesh"
        );
        let count = mesh.pixel_count(element)?;
        ensure!(
            patches.pixel_start[patch] > 0
                && patches.pixel_start[patch] <= patches.pixel_end[patch]
                && patches.pixel_end[patch] <= count,
            "land patch {patch} has invalid pixel range"
        );
    }
    Ok(())
}

fn fill_missing_urban_types(types: &mut [i32], cell_area: &[f64], class_count: usize) {
    debug_assert_eq!(types.len(), cell_area.len());
    let missing = types
        .iter()
        .filter(|&&class| class < 1 || class > class_count as i32)
        .count();
    if missing == 0 {
        return;
    }

    let mut proportions = vec![0.0; class_count];
    let mut valid_area = 0.0;
    for (&class, &area) in types.iter().zip(cell_area) {
        if (1..=class_count as i32).contains(&class) {
            valid_area += area;
            // This intentionally mirrors `ibuff > 1 .and. ibuff < N_URB`.
            if class > 1 && class < class_count as i32 {
                proportions[class as usize - 1] += area;
            }
        }
    }
    if valid_area > 0.0 {
        for proportion in &mut proportions {
            *proportion /= valid_area;
        }
    }

    let mut counts = vec![0_usize; class_count];
    for class in 0..class_count - 1 {
        counts[class] = (proportions[class] * missing as f64) as usize;
    }
    counts[class_count - 1] = missing - counts[..class_count - 1].iter().sum::<usize>();
    for class in types {
        if *class < 1 || *class > class_count as i32 {
            for (index, count) in counts.iter_mut().enumerate() {
                if *count > 0 {
                    *class = index as i32 + 1;
                    *count -= 1;
                    break;
                }
            }
        }
    }
}

impl FlatLandPatches {
    pub fn len(&self) -> usize {
        self.set_type.len()
    }

    pub fn is_empty(&self) -> bool {
        self.set_type.is_empty()
    }

    /// Convert the CoLM one-based patch ranges into the zero-based flat cell
    /// indices consumed by Rust aggregation kernels.
    pub fn aggregation_layout(
        &self,
        mesh: &FlatMesh,
        wmo_source: Vec<Option<usize>>,
    ) -> Result<FlatPatches> {
        ensure!(
            self.element_ids.len() == self.len()
                && self.pixel_start.len() == self.len()
                && self.pixel_end.len() == self.len()
                && self.element_index.len() == self.len(),
            "land-patch vectors must have equal lengths"
        );
        let mut offsets = Vec::with_capacity(self.len() + 1);
        let mut cells = Vec::new();
        offsets.push(0);
        for patch in 0..self.len() {
            let element = self.element_index[patch]
                .checked_sub(1)
                .with_context(|| format!("patch {patch} has zero element index"))?;
            ensure!(
                mesh.element_id(element)? == self.element_ids[patch],
                "patch {patch} element ID does not match the mesh"
            );
            let count = mesh.pixel_count(element)?;
            let start = self.pixel_start[patch];
            let end = self.pixel_end[patch];
            ensure!(
                start > 0 && start <= end && end <= count,
                "patch {patch} has invalid one-based pixel range {start}..={end} for element size {count}"
            );
            let base = mesh.pixel_offsets[element];
            cells.extend(base + start - 1..base + end);
            offsets.push(cells.len());
        }
        FlatPatches::new(self.set_type.clone(), offsets, cells, wmo_source)
    }
}

/// Exact port of `MOD_Utils::quicksort_int32` for the paired land-type/order
/// vectors used by `landpatch_build`.  Equal-type order is deliberately not
/// replaced with a stable sort: it influences floating-point aggregation order.
fn colm_quicksort(values: &mut [i32], order: &mut [usize]) {
    debug_assert_eq!(values.len(), order.len());
    let length = values.len();
    if length <= 1 {
        return;
    }
    // Fortran is one-based, so `A(nA / 2)` maps to `length / 2 - 1`.
    let pivot = values[length / 2 - 1];
    let (mut left, mut right) = (0_usize, length + 1);
    while left < right {
        right -= 1;
        while values[right - 1] > pivot {
            right -= 1;
        }
        left += 1;
        while values[left - 1] < pivot {
            left += 1;
        }
        if left < right {
            values.swap(left - 1, right - 1);
            order.swap(left - 1, right - 1);
        }
    }
    let marker = right;
    colm_quicksort(&mut values[..marker], &mut order[..marker]);
    colm_quicksort(&mut values[marker..], &mut order[marker..]);
}

fn collapse_to_dominant(types: &mut [i32]) {
    let Some(first_positive) = types.iter().position(|kind| *kind > 0) else {
        return;
    };
    let maximum = usize::try_from(*types.last().expect("non-empty patch type list"))
        .expect("validated non-negative patch type");
    let mut counts = vec![0_usize; maximum + 1];
    for &kind in &types[first_positive..] {
        counts[kind as usize] += 1;
    }
    // `maxloc` returns the first maximum, hence the strict comparison.
    let (mut dominant, mut dominant_count) = (1_i32, 0_usize);
    for (kind, &count) in counts.iter().enumerate().skip(1) {
        if count > dominant_count {
            (dominant, dominant_count) = (kind as i32, count);
        }
    }
    types[first_positive..].fill(dominant);
}

#[cfg(test)]
#[path = "topology_tests.rs"]
mod tests;
