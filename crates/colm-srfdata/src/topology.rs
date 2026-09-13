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

    /// Port the core of `landpatch_build` for a raw land-type vector already
    /// gathered in this mesh's flat pixel order.
    ///
    /// CoLM sorts every element by land type and reorders its pixel coordinates
    /// in the same permutation, so subsequent aggregations can address one
    /// contiguous range per patch.  This consumes the mesh to make that
    /// mutation explicit and returns it with the resulting patch pixelset.
    /// `dominant_type` matches `DEF_USE_DOMINANT_PATCHTYPE`.
    pub fn into_land_patches(
        mut self,
        land_types: &[i32],
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

        let mut element_ids = Vec::new();
        let mut pixel_start = Vec::new();
        let mut pixel_end = Vec::new();
        let mut set_type = Vec::new();
        let mut element_index = Vec::new();
        let mut types = Vec::new();
        let mut order = Vec::new();
        let mut sorted_lon = Vec::new();
        let mut sorted_lat = Vec::new();

        for element in 0..self.len() {
            let range = self.pixel_offsets[element]..self.pixel_offsets[element + 1];
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

            let mut start = 0_usize;
            while start < types.len() {
                let kind = types[start];
                let mut end = start + 1;
                while end < types.len() && types[end] == kind {
                    end += 1;
                }
                element_ids.push(self.element_ids[element]);
                pixel_start.push(start + 1);
                pixel_end.push(end);
                set_type.push(kind);
                element_index.push(element + 1);
                start = end;
            }
        }
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
