//! Canopy-height initialization from `MOD_HtopReadin.F90`.

use anyhow::{ensure, Result};

/// Flat PFT input. `offsets` is a CSR range for each patch; all PFT fields share it.
#[derive(Debug, Clone, Copy)]
pub struct PftCanopyInput<'a> {
    pub offsets: &'a [usize],
    pub class: &'a [i32],
    pub fraction: &'a [f64],
    pub observed_top_m: &'a [f64],
    /// Class-indexed arrays with class zero included.
    pub default_top_m: &'a [f64],
    pub default_bottom_m: &'a [f64],
}

/// Patch and PFT canopy state, all in metres except `pft_fraction` in the input.
#[derive(Debug, Clone, PartialEq)]
pub struct CanopyState {
    pub patch_top_m: Vec<f64>,
    pub patch_bottom_m: Vec<f64>,
    pub pft_top_m: Vec<f64>,
    pub pft_bottom_m: Vec<f64>,
}

/// Compile-time land-cover tables supported by `MOD_LAIEmpirical.F90`.
///
/// CoLM's current default desktop kernel is [`Self::Igbp`].  The other three
/// tables are retained because they are complete upstream branches, not a
/// second implementation of the calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmpiricalLandCover {
    Igbp,
    Usgs,
    Sib2,
    Bats,
}

/// Vegetation state derived by `MOD_LAIEmpirical:LAI_empirical`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EmpiricalVegetation {
    pub leaf_area_index: f64,
    pub stem_area_index: f64,
    pub vegetation_fraction: f64,
    pub greenness: f64,
}

/// Port of `MOD_LAIEmpirical:LAI_empirical`.
///
/// `land_class` uses CoLM's one-based class numbering.  The routine finds the
/// first soil layer containing more than 90% of roots, then derives seasonal
/// LAI from that layer's temperature.  It is a pure shared kernel: the native
/// runtime owns the evolving soil state while initialization only supplies it.
pub fn empirical_lai(
    scheme: EmpiricalLandCover,
    land_class: i32,
    root_fraction: &[f64],
    soil_temperature_k: &[f64],
) -> Result<EmpiricalVegetation> {
    ensure!(
        !root_fraction.is_empty()
            && root_fraction.len() == soil_temperature_k.len()
            && root_fraction
                .iter()
                .chain(soil_temperature_k)
                .all(|value| value.is_finite()),
        "root fractions and soil temperatures must be finite, nonempty, and equally sized"
    );
    let (vegetation_cover, maximum_lai, minimum_lai, stem_area) = empirical_table(scheme);
    let class = usize::try_from(land_class)
        .map_err(|_| anyhow::anyhow!("land class {land_class} is negative"))?;
    ensure!(
        (1..=vegetation_cover.len()).contains(&class),
        "land class {land_class} is outside the {:?} empirical-LAI table",
        scheme
    );
    let root_layer = root_fraction
        .iter()
        .scan(0.0, |sum, fraction| {
            *sum += fraction;
            Some(*sum)
        })
        .position(|sum| sum > 0.9)
        .unwrap_or(0);
    let seasonal =
        (1.0 - 0.0016 * (298.0 - soil_temperature_k[root_layer]).max(0.0).powi(2)).max(0.0);
    let index = class - 1;
    let vegetation_fraction = vegetation_cover[index];
    Ok(EmpiricalVegetation {
        leaf_area_index: maximum_lai[index]
            + (minimum_lai[index] - maximum_lai[index]) * (1.0 - seasonal),
        stem_area_index: stem_area[index],
        vegetation_fraction,
        greenness: f64::from(vegetation_fraction > 0.0),
    })
}

fn empirical_table(
    scheme: EmpiricalLandCover,
) -> (
    &'static [f64],
    &'static [f64],
    &'static [f64],
    &'static [f64],
) {
    match scheme {
        EmpiricalLandCover::Igbp => (
            &[
                1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0,
            ],
            &[
                4.8, 5.4, 4.8, 4.8, 4.7, 4.7, 1.6, 4.7, 4.8, 1.7, 4.6, 4.9, 3.8, 4.8, 0.0, 0.06,
                0.0,
            ],
            &[
                4.0, 4.5, 0.8, 0.8, 2.2, 1.6, 0.15, 1.8, 0.9, 0.4, 0.4, 0.4, 0.9, 2.0, 0.0, 0.006,
                0.0,
            ],
            &[
                1.6, 1.8, 1.6, 1.6, 1.5, 1.5, 0.45, 1.4, 1.6, 3.1, 1.6, 0.4, 1.1, 1.3, 0.0, 0.14,
                0.0,
            ],
        ),
        EmpiricalLandCover::Usgs => (
            &[
                1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0,
                1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 0.0,
            ],
            &[
                1.5, 3.29, 4.18, 3.5, 2.5, 3.6, 2.02, 1.53, 2.0, 0.85, 4.43, 4.42, 4.56, 3.95, 4.5,
                0.0, 4.0, 3.63, 0.0, 0.64, 1.6, 1.0, 0.0, 0.0,
            ],
            &[
                1.0, 0.5, 0.5, 0.5, 1.0, 0.5, 0.5, 0.5, 0.5, 0.3, 0.5, 0.5, 4.0, 4.0, 4.0, 0.0,
                3.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
            ],
            &[
                0.2, 0.2, 0.3, 0.3, 0.5, 0.5, 1.0, 0.5, 1.0, 0.5, 2.0, 2.0, 2.0, 2.0, 2.0, 0.0,
                2.0, 2.0, 0.0, 0.1, 0.1, 0.1, 0.0, 0.0,
            ],
        ),
        EmpiricalLandCover::Sib2 => (
            &[1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 0.0],
            &[4.8, 3.9, 5.6, 5.5, 4.6, 1.7, 1.3, 2.1, 3.6, 0.0, 0.0],
            &[4.0, 0.6, 0.5, 5.0, 0.5, 0.3, 0.6, 0.4, 0.2, 0.0, 0.0],
            &[1.6, 1.8, 1.6, 0.5, 0.5, 0.5, 0.5, 0.5, 0.5, 0.0, 0.0],
        ),
        EmpiricalLandCover::Bats => (
            &[
                1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0,
                1.0, 1.0, 1.0,
            ],
            &[
                5.1, 1.6, 4.8, 4.8, 4.8, 5.4, 4.8, 0.0, 3.6, 4.8, 0.6, 0.0, 4.8, 0.0, 0.0, 4.8,
                4.8, 4.8, 4.8,
            ],
            &[
                0.425, 0.4, 4.0, 0.8, 0.8, 4.5, 0.4, 0.0, 0.3, 0.4, 0.05, 0.0, 0.4, 0.0, 0.0, 4.0,
                0.8, 2.4, 2.4,
            ],
            &[
                0.425, 3.2, 1.6, 1.6, 1.6, 1.8, 1.6, 0.0, 0.3, 0.4, 0.2, 0.0, 1.6, 0.0, 0.0, 1.6,
                1.6, 1.6, 1.6,
            ],
        ),
    }
}

/// Applies the USGS portion of `HTOP_readin`.
///
/// Unlike IGBP, this branch uses only the class defaults and has no PFT override.
pub fn derive_usgs_canopy(
    land_class: &[i32],
    default_top_m: &[f64],
    default_bottom_m: &[f64],
) -> Result<CanopyState> {
    ensure!(
        default_top_m.len() == default_bottom_m.len() && !default_top_m.is_empty(),
        "canopy default tables must be nonempty and have equal length"
    );
    let mut patch_top_m = Vec::with_capacity(land_class.len());
    let mut patch_bottom_m = Vec::with_capacity(land_class.len());
    for &class in land_class {
        let class = checked_class(class, default_top_m.len(), "land")?;
        patch_top_m.push(default_top_m[class]);
        patch_bottom_m.push(default_bottom_m[class]);
    }
    Ok(CanopyState {
        patch_top_m,
        patch_bottom_m,
        pft_top_m: Vec::new(),
        pft_bottom_m: Vec::new(),
    })
}

/// Applies the IGBP portion of `HTOP_readin`.
///
/// Land classes are CoLM's one-based IGBP IDs. `default_*` are therefore class-indexed
/// with entry zero unused. When PFT is present, natural-soil patches (`patch_type == 0`)
/// are replaced by the PFT-fraction weighted canopy values exactly as the Fortran routine.
pub fn derive_igbp_canopy(
    land_class: &[i32],
    patch_type: &[i32],
    observed_top_m: &[f64],
    default_top_m: &[f64],
    default_bottom_m: &[f64],
    pft: Option<PftCanopyInput<'_>>,
) -> Result<CanopyState> {
    ensure!(
        land_class.len() == patch_type.len() && land_class.len() == observed_top_m.len(),
        "land class, patch type, and observed canopy height lengths must match"
    );
    ensure!(
        default_top_m.len() == default_bottom_m.len() && !default_top_m.is_empty(),
        "canopy default tables must be nonempty and have equal length"
    );

    let mut patch_top_m = Vec::with_capacity(land_class.len());
    let mut patch_bottom_m = Vec::with_capacity(land_class.len());
    for patch in 0..land_class.len() {
        let class = checked_class(land_class[patch], default_top_m.len(), "land")?;
        let mut top = default_top_m[class];
        let mut bottom = default_bottom_m[class];
        if class < 6 || class == 8 {
            top = observed_top_m[patch].max(2.0);
            bottom =
                (observed_top_m[patch] * default_bottom_m[class] / default_top_m[class]).max(1.0);
        }
        patch_top_m.push(top);
        patch_bottom_m.push(bottom);
    }

    let Some(pft) = pft else {
        return Ok(CanopyState {
            patch_top_m,
            patch_bottom_m,
            pft_top_m: Vec::new(),
            pft_bottom_m: Vec::new(),
        });
    };
    validate_pft_input(land_class.len(), pft)?;
    let mut pft_top_m = Vec::with_capacity(pft.class.len());
    let mut pft_bottom_m = Vec::with_capacity(pft.class.len());
    for pft_index in 0..pft.class.len() {
        let class = checked_class(pft.class[pft_index], pft.default_top_m.len(), "PFT")?;
        let mut top = pft.default_top_m[class];
        let mut bottom = pft.default_bottom_m[class];
        if class > 0 && class < 9 {
            top = pft.observed_top_m[pft_index].max(2.0);
            bottom = (pft.observed_top_m[pft_index] * pft.default_bottom_m[class]
                / pft.default_top_m[class])
                .max(1.0);
        }
        pft_top_m.push(top);
        pft_bottom_m.push(bottom);
    }
    for patch in 0..land_class.len() {
        if patch_type[patch] != 0 {
            continue;
        }
        let range = pft.offsets[patch]..pft.offsets[patch + 1];
        patch_top_m[patch] = range
            .clone()
            .map(|index| pft_top_m[index] * pft.fraction[index])
            .sum();
        patch_bottom_m[patch] = range
            .map(|index| pft_bottom_m[index] * pft.fraction[index])
            .sum();
    }
    Ok(CanopyState {
        patch_top_m,
        patch_bottom_m,
        pft_top_m,
        pft_bottom_m,
    })
}

fn checked_class(class: i32, table_len: usize, kind: &str) -> Result<usize> {
    let class =
        usize::try_from(class).map_err(|_| anyhow::anyhow!("{kind} class {class} is negative"))?;
    ensure!(
        class < table_len,
        "{kind} class {class} exceeds table size {table_len}"
    );
    Ok(class)
}

fn validate_pft_input(patches: usize, pft: PftCanopyInput<'_>) -> Result<()> {
    ensure!(
        pft.offsets.len() == patches + 1 && pft.offsets.first() == Some(&0),
        "PFT offsets must start at zero and contain one entry per patch plus one"
    );
    ensure!(
        pft.class.len() == pft.fraction.len() && pft.class.len() == pft.observed_top_m.len(),
        "PFT classes, fractions, and observed heights must have equal length"
    );
    ensure!(
        pft.default_top_m.len() == pft.default_bottom_m.len() && !pft.default_top_m.is_empty(),
        "PFT default tables must be nonempty and have equal length"
    );
    ensure!(
        pft.offsets.windows(2).all(|pair| pair[0] <= pair[1])
            && pft.offsets.last() == Some(&pft.class.len()),
        "PFT offsets must be monotonic and end at the PFT count"
    );
    Ok(())
}

#[cfg(test)]
#[path = "vegetation_tests.rs"]
mod vegetation_tests;
