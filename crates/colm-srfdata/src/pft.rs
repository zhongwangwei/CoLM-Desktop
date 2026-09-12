//! PFT-fraction aggregation from `Aggregation_PercentagesPFT.F90`.

use anyhow::{ensure, Context, Result};

use crate::surface::FlatPatches;

/// The branch chosen by `Aggregation_PercentagesPFT` for one land patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PftPatchKind {
    Natural,
    Crop,
    Other,
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

#[cfg(test)]
#[path = "pft_tests.rs"]
mod pft_tests;
