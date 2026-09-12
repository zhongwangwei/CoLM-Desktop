//! PFT-fraction aggregation from `Aggregation_PercentagesPFT.F90`.

use anyhow::{ensure, Context, Result};

use crate::{surface::FlatPatches, topology::FlatLandPatches};

/// The branch chosen by `Aggregation_PercentagesPFT` for one land patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PftPatchKind {
    Natural,
    Crop,
    Other,
}

/// The `landpft` pixelset and its mapping back to `landpatch`.
///
/// `patch_offsets` partitions `pft_classes` once for every land patch.  The
/// structural vectors use the same one-based pixel ranges as CoLM's saved
/// `landpft` pixelset.
#[derive(Debug, Clone, PartialEq)]
pub struct PftTopology {
    pub land_pfts: FlatLandPatches,
    pub patch_offsets: Vec<usize>,
    pub pft_classes: Vec<usize>,
    pub patch_kind: Vec<PftPatchKind>,
}

/// Build CoLM's non-CROP `landpft` partition from class-major PFT fractions.
///
/// PFT land patches are created only for the merged natural land-cover type
/// (`settyp == 1`).  The weighted positive-class test and bare-soil fallback
/// are the `MOD_LandPFT::landpft_build` rules. `pft_class_count` permits
/// CoLM's 16-class MODIS source to retain only its 15 natural PFT types; the
/// actual percentages remain the responsibility of [`aggregate_pft_fractions`].
pub fn build_pft_topology(
    land_patches: &FlatLandPatches,
    patches: &FlatPatches,
    raw_class_count: usize,
    pft_class_count: usize,
    raw_percent: &[f64],
    land_area: &[f64],
) -> Result<PftTopology> {
    ensure!(
        raw_class_count > 0
            && pft_class_count > 0
            && pft_class_count <= raw_class_count
            && raw_percent.len() == raw_class_count * land_area.len(),
        "raw PFT percentages must be raw_class_count x raw cell count"
    );
    ensure!(
        land_patches.len() == patches.len(),
        "landpft needs matching structural and aggregation patch layouts"
    );
    ensure!(
        land_area
            .iter()
            .all(|area| area.is_finite() && *area >= 0.0)
            && raw_percent.iter().all(|value| value.is_finite()),
        "landpft inputs must be finite and land areas non-negative"
    );

    let mut element_ids = Vec::new();
    let mut pixel_start = Vec::new();
    let mut pixel_end = Vec::new();
    let mut set_type = Vec::new();
    let mut element_index = Vec::new();
    let mut patch_offsets = Vec::with_capacity(land_patches.len() + 1);
    let mut pft_classes = Vec::new();
    let mut patch_kind = Vec::with_capacity(land_patches.len());
    patch_offsets.push(0);

    for patch in 0..land_patches.len() {
        ensure!(
            patches.wmo_source_for(patch).is_none(),
            "landpft WMO sharing needs the upstream land2mWMO topology"
        );
        let kind = if land_patches.set_type[patch] == 1 {
            PftPatchKind::Natural
        } else {
            PftPatchKind::Other
        };
        patch_kind.push(kind);
        if kind == PftPatchKind::Natural {
            let mut weighted = vec![0.0; raw_class_count];
            let mut total = 0.0;
            for &cell in patches.raw_cells(patch) {
                let area = land_area[cell];
                let mut sum = 0.0;
                for class in 0..raw_class_count {
                    let value = raw_percent[class * land_area.len() + cell];
                    sum += value;
                    weighted[class] += value * area;
                }
                total += area * sum;
            }
            let classes = if total > 0.0 {
                weighted
                    .iter()
                    .take(pft_class_count)
                    .enumerate()
                    .filter_map(|(class, value)| (value / total > 0.0).then_some(class))
                    .collect::<Vec<_>>()
            } else {
                vec![0]
            };
            for class in classes {
                element_ids.push(land_patches.element_ids[patch]);
                pixel_start.push(land_patches.pixel_start[patch]);
                pixel_end.push(land_patches.pixel_end[patch]);
                set_type.push(i32::try_from(class)?);
                element_index.push(land_patches.element_index[patch]);
                pft_classes.push(class);
            }
        }
        patch_offsets.push(pft_classes.len());
    }
    Ok(PftTopology {
        land_pfts: FlatLandPatches {
            element_ids,
            pixel_start,
            pixel_end,
            set_type,
            element_index,
        },
        patch_offsets,
        pft_classes,
        patch_kind,
    })
}

/// PFT topology and raw PFT-percentage fields.
///
/// `raw_percent` is `raw_class * raw_cells + raw_cell`; output is in the CSR
/// order defined by `pft_offsets` and `pft_classes`.
#[derive(Debug, Clone, Copy)]
pub struct PftFractionInput<'a> {
    pub pft_offsets: &'a [usize],
    pub pft_classes: &'a [usize],
    pub patch_kind: &'a [PftPatchKind],
    pub raw_class_count: usize,
    pub raw_percent: &'a [f64],
    pub land_area: &'a [f64],
    /// CROP builds clear this raw PFT class before aggregation.
    pub crop_excluded_class: Option<usize>,
}

/// Raw PFT abundance and a monthly PFT LAI or SAI field.
///
/// Both raw fields are class-major: `raw_class * raw_cells + raw_cell`.
/// This is the computational portion shared by the PFT/PC LAI and SAI loops
/// in `Aggregation_LAI.F90`.
#[derive(Debug, Clone, Copy)]
pub struct PftIndexInput<'a> {
    pub pft_offsets: &'a [usize],
    pub pft_classes: &'a [usize],
    pub patch_kind: &'a [PftPatchKind],
    pub raw_class_count: usize,
    pub raw_percent: &'a [f64],
    pub raw_index: &'a [f64],
    pub land_area: &'a [f64],
}

/// Patch and PFT vectors written by one monthly PFT/PC LAI or SAI step.
#[derive(Debug, Clone, PartialEq)]
pub struct PftIndexState {
    pub patch_index: Vec<f64>,
    pub pft_index: Vec<f64>,
}

/// Applies the computational part of `Aggregation_PercentagesPFT`.
pub fn aggregate_pft_fractions(
    patches: &FlatPatches,
    input: PftFractionInput<'_>,
) -> Result<Vec<f64>> {
    validate_input(patches, input)?;
    let mut output = vec![0.0; input.pft_classes.len()];
    for patch in 0..patches.len() {
        let range = input.pft_offsets[patch]..input.pft_offsets[patch + 1];
        if patches.wmo_source_for(patch).is_some() {
            let first = range
                .clone()
                .next()
                .with_context(|| format!("WMO patch {patch} has no PFT"))?;
            output[first] = 1.0;
            continue;
        }
        match input.patch_kind[patch] {
            PftPatchKind::Natural => {
                aggregate_natural_patch(patches.raw_cells(patch), range, input, &mut output, patch)?
            }
            PftPatchKind::Crop => output[range].fill(1.0),
            PftPatchKind::Other => {}
        }
    }
    Ok(output)
}

/// Applies the PFT/PC branch of `Aggregation_LAI` to one LAI or SAI field.
pub fn aggregate_pft_index(
    patches: &FlatPatches,
    input: PftIndexInput<'_>,
) -> Result<PftIndexState> {
    validate_index_input(patches, input)?;
    let mut output = PftIndexState {
        patch_index: vec![0.0; patches.len()],
        pft_index: vec![0.0; input.pft_classes.len()],
    };
    for patch in 0..patches.len() {
        let range = input.pft_offsets[patch]..input.pft_offsets[patch + 1];
        let first = range
            .clone()
            .next()
            .with_context(|| format!("PFT/PC patch {patch} has no PFT"))?;
        if let Some(source) = patches.wmo_source_for(patch) {
            let class = input.pft_classes[first];
            if (12..=14).contains(&class) {
                let source_range = input.pft_offsets[source]..input.pft_offsets[source + 1];
                if let Some(source_pft) = source_range
                    .clone()
                    .find(|&pft| input.pft_classes[pft] == class)
                {
                    output.pft_index[first] = output.pft_index[source_pft];
                }
            }
            output.patch_index[patch] = output.pft_index[first];
            continue;
        }

        let (patch_index, area_sum) =
            aggregate_patch_index(patches.raw_cells(patch), input, patch)?;
        output.patch_index[patch] = patch_index / area_sum;
        match input.patch_kind[patch] {
            PftPatchKind::Natural => {
                for pft in range {
                    let class = input.pft_classes[pft];
                    let mut weighted_area = 0.0;
                    let mut weighted_index = 0.0;
                    for &cell in patches.raw_cells(patch) {
                        let percent = percentage_index(input, class, cell, patch)?.max(0.0);
                        let area = area(input.land_area, cell, patch)?;
                        weighted_area += percent * area;
                        weighted_index += index(input, class, cell, patch)? * percent * area;
                    }
                    if weighted_area > 0.0 {
                        output.pft_index[pft] = weighted_index / weighted_area;
                    }
                }
            }
            PftPatchKind::Crop => output.pft_index[first] = output.patch_index[patch],
            PftPatchKind::Other => {}
        }
    }
    Ok(output)
}

fn validate_input(patches: &FlatPatches, input: PftFractionInput<'_>) -> Result<()> {
    ensure!(
        input.pft_offsets.len() == patches.len() + 1
            && input.pft_offsets.first() == Some(&0)
            && input.pft_offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && input.pft_offsets.last() == Some(&input.pft_classes.len()),
        "PFT offsets must partition the PFT vector once per patch"
    );
    ensure!(
        input.patch_kind.len() == patches.len(),
        "PFT patch kinds must have one entry per patch"
    );
    ensure!(
        input.raw_class_count > 0
            && input.raw_percent.len() == input.raw_class_count * input.land_area.len(),
        "raw PFT percentages must be raw_class_count x raw cell count"
    );
    ensure!(
        input
            .pft_classes
            .iter()
            .all(|&class| class < input.raw_class_count),
        "a PFT class is outside the raw PFT class range"
    );
    ensure!(
        input
            .crop_excluded_class
            .is_none_or(|class| class < input.raw_class_count),
        "excluded crop PFT class is outside the raw PFT class range"
    );
    Ok(())
}

fn validate_index_input(patches: &FlatPatches, input: PftIndexInput<'_>) -> Result<()> {
    ensure!(
        input.pft_offsets.len() == patches.len() + 1
            && input.pft_offsets.first() == Some(&0)
            && input.pft_offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && input.pft_offsets.last() == Some(&input.pft_classes.len()),
        "PFT offsets must partition the PFT vector once per patch"
    );
    ensure!(
        input.patch_kind.len() == patches.len(),
        "PFT patch kinds must have one entry per patch"
    );
    ensure!(
        input.raw_class_count > 0
            && input.raw_percent.len() == input.raw_class_count * input.land_area.len()
            && input.raw_index.len() == input.raw_class_count * input.land_area.len(),
        "raw PFT percentage and index fields must be raw_class_count x raw cell count"
    );
    ensure!(
        input
            .pft_classes
            .iter()
            .all(|&class| class < input.raw_class_count),
        "a PFT class is outside the raw PFT class range"
    );
    ensure!(
        input
            .land_area
            .iter()
            .all(|area| area.is_finite() && *area >= 0.0),
        "PFT land area must be finite and non-negative"
    );
    ensure!(
        input
            .raw_percent
            .iter()
            .chain(input.raw_index)
            .all(|value| value.is_finite()),
        "PFT percentage and LAI/SAI inputs must be finite"
    );
    Ok(())
}

fn aggregate_patch_index(
    cells: &[usize],
    input: PftIndexInput<'_>,
    patch: usize,
) -> Result<(f64, f64)> {
    let mut area_sum = 0.0;
    let mut index_sum = 0.0;
    for &cell in cells {
        let area = area(input.land_area, cell, patch)?;
        let mut percent_sum = 0.0;
        let mut value_sum = 0.0;
        for class in 0..input.raw_class_count {
            let percent = percentage_index(input, class, cell, patch)?.max(0.0);
            percent_sum += percent;
            value_sum += index(input, class, cell, patch)? * percent;
        }
        index_sum += value_sum / percent_sum.max(1.0e-6) * area;
        area_sum += area;
    }
    ensure!(
        area_sum > 0.0 && area_sum.is_finite(),
        "PFT/PC patch {patch} has zero or non-finite land area"
    );
    Ok((index_sum, area_sum))
}

fn aggregate_natural_patch(
    cells: &[usize],
    range: std::ops::Range<usize>,
    input: PftFractionInput<'_>,
    output: &mut [f64],
    patch: usize,
) -> Result<()> {
    let first = range
        .clone()
        .next()
        .with_context(|| format!("natural patch {patch} has no PFT"))?;
    let mut total_area = 0.0;
    for &cell in cells {
        let area = area(input.land_area, cell, patch)?;
        total_area += area;
        let mut total = 0.0;
        for class in 0..input.raw_class_count {
            if Some(class) != input.crop_excluded_class {
                total += percentage(input, class, cell, patch)?.max(0.0);
            }
        }
        total = total.max(1.0e-6);
        for pft in range.clone() {
            let class = input.pft_classes[pft];
            let value = if Some(class) == input.crop_excluded_class {
                0.0
            } else {
                percentage(input, class, cell, patch)?.max(0.0)
            };
            output[pft] += value / total * area;
        }
    }
    ensure!(
        total_area > 0.0 && total_area.is_finite(),
        "natural PFT patch {patch} has zero or non-finite land area"
    );
    for value in &mut output[range.clone()] {
        *value /= total_area;
    }
    let sum = output[range.clone()].iter().sum::<f64>();
    if sum > 0.0 {
        for value in &mut output[range] {
            *value /= sum;
        }
    } else {
        output[first] = 1.0;
    }
    Ok(())
}

fn area(values: &[f64], cell: usize, patch: usize) -> Result<f64> {
    let value = values
        .get(cell)
        .copied()
        .with_context(|| format!("PFT patch {patch} references raw area cell {cell}"))?;
    ensure!(
        value.is_finite() && value >= 0.0,
        "PFT patch {patch} has invalid raw cell area {value}"
    );
    Ok(value)
}

fn percentage(input: PftFractionInput<'_>, class: usize, cell: usize, patch: usize) -> Result<f64> {
    let index = class * input.land_area.len() + cell;
    input.raw_percent.get(index).copied().with_context(|| {
        format!(
            "PFT patch {patch} references raw class {class}, cell {cell}, outside the raw percentage field"
        )
    })
}

fn percentage_index(
    input: PftIndexInput<'_>,
    class: usize,
    cell: usize,
    patch: usize,
) -> Result<f64> {
    let index = class * input.land_area.len() + cell;
    input.raw_percent.get(index).copied().with_context(|| {
        format!(
            "PFT/PC patch {patch} references raw class {class}, cell {cell}, outside the raw percentage field"
        )
    })
}

fn index(input: PftIndexInput<'_>, class: usize, cell: usize, patch: usize) -> Result<f64> {
    let offset = class * input.land_area.len() + cell;
    input.raw_index.get(offset).copied().with_context(|| {
        format!(
            "PFT/PC patch {patch} references raw class {class}, cell {cell}, outside the raw LAI/SAI field"
        )
    })
}

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;
