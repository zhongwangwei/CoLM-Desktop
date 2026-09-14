//! Spatial soil-parameter aggregation from `Aggregation_SoilParameters.F90`.
//!
//! The raw files hold eight independent two-dimensional layers.  This module
//! owns only the per-layer numerical contract; raster streaming and NetCDF
//! serialization remain in the spatial command layer.

use anyhow::{ensure, Result};
use rayon::prelude::*;

use crate::{
    minpack::{lmder, LeastSquaresProblem},
    surface::{FlatPatches, SURFACE_MISSING},
};

/// CoLM surface-data soil layers (the model has further derived layers).
pub const SOIL_LAYERS: usize = 8;

/// The statistic selected by `Aggregation_SoilParameters` for one raw field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SoilStatistic {
    AreaMean,
    GeometricMean,
    Median,
}

/// IGBP or USGS class codes that receive CoLM's water/ice soil fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoilPatchClasses {
    pub water: i32,
    pub glacier: i32,
}

/// One scalar soil field's water/ice fallback and aggregation rule.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SoilField {
    pub statistic: SoilStatistic,
    pub fill: f64,
}

/// Van Genuchten-Mualem fields from one source soil layer.
#[derive(Debug, Clone, PartialEq)]
pub struct VgmSoil {
    pub theta_r: Vec<f64>,
    pub alpha: Vec<f64>,
    pub n: Vec<f64>,
    pub theta_s: Vec<f64>,
    pub k_s: Vec<f64>,
    pub l: Vec<f64>,
}

/// VGM source fields for one soil layer, in raw mesh-pixel order.
#[derive(Debug, Clone, Copy)]
pub struct VgmInputs<'a> {
    pub theta_r: &'a [f64],
    pub alpha: &'a [f64],
    pub n: &'a [f64],
    pub theta_s: &'a [f64],
    pub k_s: &'a [f64],
    pub l: &'a [f64],
}

/// Campbell fields from one source soil layer.
#[derive(Debug, Clone, PartialEq)]
pub struct CampbellSoil {
    pub theta_s: Vec<f64>,
    pub k_s: Vec<f64>,
    pub psi_s: Vec<f64>,
    pub lambda: Vec<f64>,
}

/// Campbell source fields for one soil layer, in raw mesh-pixel order.
#[derive(Debug, Clone, Copy)]
pub struct CampbellInputs<'a> {
    pub theta_s: &'a [f64],
    pub k_s: &'a [f64],
    pub psi_s: &'a [f64],
    pub lambda: &'a [f64],
}

/// Aggregate one direct soil property, including CoLM's WMO copy and NaN fill.
pub fn aggregate_soil_field(
    patches: &FlatPatches,
    raw: &[f64],
    area: &[f64],
    classes: SoilPatchClasses,
    field: SoilField,
) -> Result<Vec<f64>> {
    validate(patches, raw, area)?;
    let mut output = vec![SURFACE_MISSING; patches.len()];
    for patch in 0..patches.len() {
        if let Some(source) = patches.wmo_source_for(patch) {
            output[patch] = output[source];
            continue;
        }
        if patches.patch_type_for(patch) == 0 {
            continue;
        }
        let values = filled_values(patches, patch, raw, classes, field.fill);
        output[patch] = statistic(&values, patches.raw_cells(patch), area, field.statistic);
    }
    Ok(output)
}

/// Derive Balland-Arp alpha and beta from raw gravel and sand fractions.
pub fn aggregate_balland_arp(
    patches: &FlatPatches,
    gravel: &[f64],
    sand: &[f64],
    area: &[f64],
    classes: SoilPatchClasses,
) -> Result<(Vec<f64>, Vec<f64>)> {
    validate(patches, gravel, area)?;
    validate(patches, sand, area)?;
    let mut alpha = vec![SURFACE_MISSING; patches.len()];
    let mut beta = vec![SURFACE_MISSING; patches.len()];
    for patch in 0..patches.len() {
        if let Some(source) = patches.wmo_source_for(patch) {
            alpha[patch] = alpha[source];
            beta[patch] = beta[source];
            continue;
        }
        if patches.patch_type_for(patch) == 0 {
            continue;
        }
        let gravel = filled_values(patches, patch, gravel, classes, 0.0);
        let sand = filled_values(patches, patch, sand, classes, 0.09);
        let mut alpha_cells = Vec::with_capacity(gravel.len());
        let mut beta_cells = Vec::with_capacity(gravel.len());
        for (gravel, sand) in gravel.iter().zip(sand) {
            if gravel + sand > 0.4 {
                alpha_cells.push(0.38);
                beta_cells.push(35.0);
            } else if gravel + sand > 0.25 {
                alpha_cells.push(0.24);
                beta_cells.push(26.0);
            } else {
                alpha_cells.push(0.2);
                beta_cells.push(10.0);
            }
        }
        alpha[patch] = statistic(
            &alpha_cells,
            patches.raw_cells(patch),
            area,
            SoilStatistic::Median,
        );
        beta[patch] = statistic(
            &beta_cells,
            patches.raw_cells(patch),
            area,
            SoilStatistic::Median,
        );
    }
    Ok((alpha, beta))
}

/// Aggregate VGM parameters and reproduce CoLM's optional 24-point LM fit.
pub fn aggregate_vgm(
    patches: &FlatPatches,
    input: VgmInputs<'_>,
    area: &[f64],
    classes: SoilPatchClasses,
    fills: VgmFills,
    fit: bool,
) -> Result<VgmSoil> {
    for raw in [
        input.theta_r,
        input.alpha,
        input.n,
        input.theta_s,
        input.k_s,
        input.l,
    ] {
        validate(patches, raw, area)?;
    }
    let values = aggregate_vgm_values(patches, input, area, classes, fills, fit);
    let mut output = VgmSoil {
        theta_r: vec![SURFACE_MISSING; patches.len()],
        alpha: vec![SURFACE_MISSING; patches.len()],
        n: vec![SURFACE_MISSING; patches.len()],
        theta_s: vec![SURFACE_MISSING; patches.len()],
        k_s: vec![SURFACE_MISSING; patches.len()],
        l: vec![SURFACE_MISSING; patches.len()],
    };
    for patch in 0..patches.len() {
        if let Some(source) = patches.wmo_source_for(patch) {
            output.theta_r[patch] = output.theta_r[source];
            output.alpha[patch] = output.alpha[source];
            output.n[patch] = output.n[source];
            output.theta_s[patch] = output.theta_s[source];
            output.k_s[patch] = output.k_s[source];
            output.l[patch] = output.l[source];
        } else if let Some(values) = values[patch] {
            output.theta_r[patch] = values[0];
            output.alpha[patch] = values[1];
            output.n[patch] = values[2];
            output.theta_s[patch] = values[3];
            output.k_s[patch] = values[4];
            output.l[patch] = values[5];
        }
    }
    Ok(output)
}

type VgmValues = [f64; 6];

fn aggregate_vgm_values(
    patches: &FlatPatches,
    input: VgmInputs<'_>,
    area: &[f64],
    classes: SoilPatchClasses,
    fills: VgmFills,
    fit: bool,
) -> Vec<Option<VgmValues>> {
    (0..patches.len())
        .into_par_iter()
        .map(|patch| aggregate_vgm_patch(patches, patch, input, area, classes, fills, fit))
        .collect()
}

fn aggregate_vgm_patch(
    patches: &FlatPatches,
    patch: usize,
    input: VgmInputs<'_>,
    area: &[f64],
    classes: SoilPatchClasses,
    fills: VgmFills,
    fit: bool,
) -> Option<VgmValues> {
    if patches.wmo_source_for(patch).is_some() || patches.patch_type_for(patch) == 0 {
        return None;
    }
    let theta_r = filled_values(patches, patch, input.theta_r, classes, fills.theta_r);
    let alpha = filled_values(patches, patch, input.alpha, classes, fills.alpha);
    let n = filled_values(patches, patch, input.n, classes, fills.n);
    let theta_s = filled_values(patches, patch, input.theta_s, classes, fills.theta_s);
    let k_s = filled_values(patches, patch, input.k_s, classes, fills.k_s);
    let l = filled_values(patches, patch, input.l, classes, fills.l);
    let cells = patches.raw_cells(patch);
    let mut values = [
        statistic(&theta_r, cells, area, SoilStatistic::AreaMean),
        statistic(&alpha, cells, area, SoilStatistic::Median),
        statistic(&n, cells, area, SoilStatistic::Median),
        statistic(&theta_s, cells, area, SoilStatistic::AreaMean),
        statistic(&k_s, cells, area, SoilStatistic::GeometricMean),
        statistic(&l, cells, area, SoilStatistic::Median),
    ];
    if fit && cells.len() > 1 {
        let problem = VgmProblem {
            theta_r: &theta_r,
            alpha: &alpha,
            n: &n,
            theta_s: &theta_s,
            k_s: &k_s,
            l: &l,
            phi: values[3],
            conductivity: values[4],
            l_patch: values[5],
        };
        let mut x = [values[0], values[1], values[2], values[4]];
        if lmder(&problem, &mut x, VGM_PRESSURES.len())
            && x[0] >= 0.0
            && x[0] <= values[3]
            && (1.0e-5..=1.0).contains(&x[1])
            && (1.1..=10.0).contains(&x[2])
            && x[3] > 0.0
            && x[3] <= 1.0e7
        {
            values[0] = x[0];
            values[1] = x[1];
            values[2] = x[2];
            values[4] = x[3];
        }
    }
    Some(values)
}

/// CoLM's water/glacier constants for the VGM source fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VgmFills {
    pub theta_r: f64,
    pub alpha: f64,
    pub n: f64,
    pub theta_s: f64,
    pub k_s: f64,
    pub l: f64,
}

impl Default for VgmFills {
    fn default() -> Self {
        Self {
            theta_r: 0.116,
            alpha: 0.01,
            n: 1.352,
            theta_s: 0.532,
            k_s: 11.616,
            l: 0.5,
        }
    }
}

/// Aggregate Campbell fields and reproduce CoLM's 17-point LM fit.
pub fn aggregate_campbell(
    patches: &FlatPatches,
    input: CampbellInputs<'_>,
    area: &[f64],
    classes: SoilPatchClasses,
    fills: CampbellFills,
    fit: bool,
) -> Result<CampbellSoil> {
    for raw in [input.theta_s, input.k_s, input.psi_s, input.lambda] {
        validate(patches, raw, area)?;
    }
    let mut output = CampbellSoil {
        theta_s: vec![SURFACE_MISSING; patches.len()],
        k_s: vec![SURFACE_MISSING; patches.len()],
        psi_s: vec![SURFACE_MISSING; patches.len()],
        lambda: vec![SURFACE_MISSING; patches.len()],
    };
    for patch in 0..patches.len() {
        if let Some(source) = patches.wmo_source_for(patch) {
            output.theta_s[patch] = output.theta_s[source];
            output.k_s[patch] = output.k_s[source];
            output.psi_s[patch] = output.psi_s[source];
            output.lambda[patch] = output.lambda[source];
            continue;
        }
        if patches.patch_type_for(patch) == 0 {
            continue;
        }
        let theta_s = filled_values(patches, patch, input.theta_s, classes, fills.theta_s);
        let k_s = filled_values(patches, patch, input.k_s, classes, fills.k_s);
        let psi_s = filled_values(patches, patch, input.psi_s, classes, fills.psi_s);
        let lambda = filled_values(patches, patch, input.lambda, classes, fills.lambda);
        let cells = patches.raw_cells(patch);
        output.theta_s[patch] = statistic(&theta_s, cells, area, SoilStatistic::AreaMean);
        output.k_s[patch] = statistic(&k_s, cells, area, SoilStatistic::GeometricMean);
        output.psi_s[patch] = statistic(&psi_s, cells, area, SoilStatistic::Median);
        output.lambda[patch] = statistic(&lambda, cells, area, SoilStatistic::Median);
        if fit && cells.len() > 1 {
            let problem = CampbellProblem {
                theta_s: &theta_s,
                k_s: &k_s,
                psi_s: &psi_s,
                lambda: &lambda,
                phi: output.theta_s[patch],
                conductivity: output.k_s[patch],
            };
            let mut x = [output.psi_s[patch], output.lambda[patch], output.k_s[patch]];
            if lmder(&problem, &mut x, CAMPBELL_PRESSURES.len())
                && (-300.0..0.0).contains(&x[0])
                && x[1] > 0.0
                && x[1] <= 1.0
                && x[2] > 0.0
                && x[2] <= 1.0e7
            {
                output.psi_s[patch] = x[0];
                output.lambda[patch] = x[1];
                output.k_s[patch] = x[2];
            }
        }
    }
    Ok(output)
}

/// CoLM's water/glacier constants for Campbell source fields.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CampbellFills {
    pub theta_s: f64,
    pub k_s: f64,
    pub psi_s: f64,
    pub lambda: f64,
}

impl Default for CampbellFills {
    fn default() -> Self {
        Self {
            theta_s: 0.532,
            k_s: 11.616,
            psi_s: -35.446,
            lambda: 0.108,
        }
    }
}

const VGM_PRESSURES: [f64; 24] = [
    1.0, 5.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0, 210.0,
    300.0, 345.0, 690.0, 1020.0, 5100.0, 15300.0, 20000.0, 100000.0, 1000000.0,
];
const CAMPBELL_PRESSURES: [f64; 17] = [
    60.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0, 210.0, 300.0, 345.0, 690.0, 1020.0, 5100.0,
    15300.0, 20000.0, 100000.0, 1000000.0,
];

struct VgmProblem<'a> {
    theta_r: &'a [f64],
    alpha: &'a [f64],
    n: &'a [f64],
    theta_s: &'a [f64],
    k_s: &'a [f64],
    l: &'a [f64],
    phi: f64,
    conductivity: f64,
    l_patch: f64,
}

impl LeastSquaresProblem for VgmProblem<'_> {
    fn residual(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[1] <= 0.0 || x[2] <= 0.1 || x[2] >= 100.0 || x[3] <= 0.0 {
            return false;
        }
        for (index, pressure) in VGM_PRESSURES.iter().copied().enumerate() {
            let mut value = 0.0;
            for cell in 0..self.theta_r.len() {
                let theta = (1.0 + (self.alpha[cell] * pressure).powf(self.n[cell]))
                    .powf(1.0 / self.n[cell] - 1.0);
                let observed =
                    self.theta_r[cell] + (self.theta_s[cell] - self.theta_r[cell]) * theta;
                let fitted = x[0]
                    + (self.phi - x[0])
                        * (1.0 + (x[1] * pressure).powf(x[2])).powf(1.0 / x[2] - 1.0);
                value += ((fitted - observed) / self.phi).powi(2);
                let observed_k = self.k_s[cell]
                    * theta.powf(self.l[cell])
                    * (1.0
                        - (1.0 - theta.powf(self.n[cell] / (self.n[cell] - 1.0)))
                            .powf(1.0 - 1.0 / self.n[cell]))
                    .powi(2);
                let base = 1.0 + (x[1] * pressure).powf(x[2]);
                let term = 1.0 - (1.0 - 1.0 / base).powf(1.0 - 1.0 / x[2]);
                let fitted_log = x[3].log10()
                    + (1.0 / x[2] - 1.0) * self.l_patch * base.log10()
                    + term.powi(2).log10();
                value += ((fitted_log - observed_k.log10()) / self.conductivity.log10()).powi(2);
            }
            output[index] = value;
        }
        output.iter().all(|value| value.is_finite())
    }

    fn jacobian(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[1] <= 0.0 || x[2] <= 0.1 || x[2] >= 100.0 || x[3] <= 0.0 {
            return false;
        }
        let log_conductivity = self.conductivity.log10();
        for (row, pressure) in VGM_PRESSURES.iter().copied().enumerate() {
            let z = x[1] * pressure;
            let z_to_n = z.powf(x[2]);
            let base = 1.0 + z_to_n;
            let q = base.powf(1.0 / x[2] - 1.0);
            let q_alpha = q * (1.0 - x[2]) * z.powf(x[2] - 1.0) * pressure / base;
            let base_n = z_to_n * z.ln();
            let q_n = q * (-base.ln() / x[2].powi(2) + (1.0 / x[2] - 1.0) * base_n / base);
            let fitted_theta = x[0] + (self.phi - x[0]) * q;
            let u = 1.0 - 1.0 / base;
            let power = 1.0 - 1.0 / x[2];
            let term = 1.0 - u.powf(power);
            let log_residual = |observed_k: f64| {
                let fitted = x[3].log10()
                    + (1.0 / x[2] - 1.0) * self.l_patch * base.log10()
                    + term.powi(2).log10();
                (fitted - observed_k.log10()) / log_conductivity
            };
            let log_alpha =
                self.l_patch * (1.0 / x[2] - 1.0) * x[2] * z.powf(x[2] - 1.0) * pressure
                    / (base * std::f64::consts::LN_10)
                    + 2.0
                        * (-(power) * u.powf(-1.0 / x[2]) * x[2] * z.powf(x[2] - 1.0) * pressure
                            / base.powi(2))
                        / (term * std::f64::consts::LN_10);
            let term_n =
                -u.powf(power) * (u.ln() / x[2].powi(2) + power * base_n / (u * base.powi(2)));
            let log_n = self.l_patch
                * (-base.log10() / x[2].powi(2)
                    + (1.0 / x[2] - 1.0) * base_n / (base * std::f64::consts::LN_10))
                + 2.0 * term_n / (term * std::f64::consts::LN_10);
            let log_k = 1.0 / (x[3] * std::f64::consts::LN_10);
            let mut derivatives = [0.0; 4];
            for cell in 0..self.theta_r.len() {
                let theta = (1.0 + (self.alpha[cell] * pressure).powf(self.n[cell]))
                    .powf(1.0 / self.n[cell] - 1.0);
                let observed_theta =
                    self.theta_r[cell] + (self.theta_s[cell] - self.theta_r[cell]) * theta;
                let theta_residual = (fitted_theta - observed_theta) / self.phi;
                let observed_k = self.k_s[cell]
                    * theta.powf(self.l[cell])
                    * (1.0
                        - (1.0 - theta.powf(self.n[cell] / (self.n[cell] - 1.0)))
                            .powf(1.0 - 1.0 / self.n[cell]))
                    .powi(2);
                let conductivity_residual = log_residual(observed_k);
                derivatives[0] += 2.0 * theta_residual * (1.0 - q) / self.phi;
                derivatives[1] += 2.0
                    * (theta_residual * (self.phi - x[0]) * q_alpha / self.phi
                        + conductivity_residual * log_alpha / log_conductivity);
                derivatives[2] += 2.0
                    * (theta_residual * (self.phi - x[0]) * q_n / self.phi
                        + conductivity_residual * log_n / log_conductivity);
                derivatives[3] += 2.0 * conductivity_residual * log_k / log_conductivity;
            }
            for (column, value) in derivatives.into_iter().enumerate() {
                output[row * 4 + column] = value;
            }
        }
        output.iter().all(|value| value.is_finite())
    }
}

struct CampbellProblem<'a> {
    theta_s: &'a [f64],
    k_s: &'a [f64],
    psi_s: &'a [f64],
    lambda: &'a [f64],
    phi: f64,
    conductivity: f64,
}

impl LeastSquaresProblem for CampbellProblem<'_> {
    fn residual(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[0] >= 0.0 || x[1].abs() >= 100.0 || x[2] <= 0.0 {
            return false;
        }
        for (index, pressure) in CAMPBELL_PRESSURES.iter().copied().enumerate() {
            let mut value = 0.0;
            for cell in 0..self.theta_s.len() {
                let ratio = -pressure / self.psi_s[cell];
                let observed = ratio.powf(-self.lambda[cell]) * self.theta_s[cell];
                let fitted = (-pressure / x[0]).powf(-x[1]) * self.phi;
                value += ((fitted - observed) / self.phi).powi(2);
                let observed_k = ratio.powf(-3.0 * self.lambda[cell] - 2.0) * self.k_s[cell];
                let fitted_log = (-pressure / x[0]).log10() * (-3.0 * x[1] - 2.0) + x[2].log10();
                value += ((fitted_log - observed_k.log10()) / self.conductivity.log10()).powi(2);
            }
            output[index] = value;
        }
        output.iter().all(|value| value.is_finite())
    }

    fn jacobian(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[0] >= 0.0 || x[1].abs() >= 100.0 || x[2] <= 0.0 {
            return false;
        }
        let log_conductivity = self.conductivity.log10();
        for (row, pressure) in CAMPBELL_PRESSURES.iter().copied().enumerate() {
            let ratio = -pressure / x[0];
            let fitted_theta = ratio.powf(-x[1]) * self.phi;
            let fitted_log = ratio.log10() * (-3.0 * x[1] - 2.0) + x[2].log10();
            let theta_psi = fitted_theta * x[1] / x[0];
            let theta_lambda = -fitted_theta * ratio.ln();
            let log_psi = (3.0 * x[1] + 2.0) / (x[0] * std::f64::consts::LN_10);
            let log_lambda = -3.0 * ratio.log10();
            let log_k = 1.0 / (x[2] * std::f64::consts::LN_10);
            let mut derivatives = [0.0; 3];
            for cell in 0..self.theta_s.len() {
                let raw_ratio = -pressure / self.psi_s[cell];
                let observed_theta = raw_ratio.powf(-self.lambda[cell]) * self.theta_s[cell];
                let theta_residual = (fitted_theta - observed_theta) / self.phi;
                let observed_k = raw_ratio.powf(-3.0 * self.lambda[cell] - 2.0) * self.k_s[cell];
                let conductivity_residual = (fitted_log - observed_k.log10()) / log_conductivity;
                derivatives[0] += 2.0
                    * (theta_residual * theta_psi / self.phi
                        + conductivity_residual * log_psi / log_conductivity);
                derivatives[1] += 2.0
                    * (theta_residual * theta_lambda / self.phi
                        + conductivity_residual * log_lambda / log_conductivity);
                derivatives[2] += 2.0 * conductivity_residual * log_k / log_conductivity;
            }
            for (column, value) in derivatives.into_iter().enumerate() {
                output[row * 3 + column] = value;
            }
        }
        output.iter().all(|value| value.is_finite())
    }
}

fn validate(patches: &FlatPatches, raw: &[f64], area: &[f64]) -> Result<()> {
    ensure!(
        raw.len() == area.len(),
        "soil field has {} cells but land area has {}",
        raw.len(),
        area.len()
    );
    ensure!(
        area.iter().all(|value| value.is_finite() && *value > 0.0),
        "soil land areas must be finite and positive"
    );
    for patch in 0..patches.len() {
        ensure!(
            patches
                .raw_cells(patch)
                .iter()
                .all(|cell| *cell < raw.len()),
            "soil patch {patch} references a raw cell outside its source"
        );
    }
    Ok(())
}

fn filled_values(
    patches: &FlatPatches,
    patch: usize,
    raw: &[f64],
    classes: SoilPatchClasses,
    fill: f64,
) -> Vec<f64> {
    let mut values = patches
        .raw_cells(patch)
        .iter()
        .map(|cell| raw[*cell])
        .collect::<Vec<_>>();
    let finite = values
        .iter()
        .copied()
        .filter(|value| !value.is_nan())
        .collect::<Vec<_>>();
    if !finite.is_empty() && finite.len() < values.len() {
        let mean = finite.iter().sum::<f64>() / finite.len() as f64;
        for value in &mut values {
            if value.is_nan() {
                *value = mean;
            }
        }
    } else if finite.is_empty()
        && matches!(patches.patch_type_for(patch), kind if kind == classes.water || kind == classes.glacier)
    {
        values.fill(fill);
    }
    values
}

fn statistic(values: &[f64], cells: &[usize], area: &[f64], method: SoilStatistic) -> f64 {
    match method {
        SoilStatistic::AreaMean => {
            let total = cells.iter().map(|cell| area[*cell]).sum::<f64>();
            values
                .iter()
                .zip(cells)
                .map(|(value, cell)| value * (area[*cell] / total))
                .sum()
        }
        SoilStatistic::GeometricMean => {
            let total = cells.iter().map(|cell| area[*cell]).sum::<f64>();
            values
                .iter()
                .zip(cells)
                .map(|(value, cell)| value.powf(area[*cell] / total))
                .product()
        }
        SoilStatistic::Median => {
            let mut values = values
                .iter()
                .copied()
                .filter(|value| *value != SURFACE_MISSING)
                .collect::<Vec<_>>();
            if values.is_empty() {
                return SURFACE_MISSING;
            }
            values.sort_unstable_by(f64::total_cmp);
            let upper = values.len() / 2;
            if values.len() % 2 == 0 {
                (values[upper - 1] + values[upper]) * 0.5
            } else {
                values[upper]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patches() -> FlatPatches {
        FlatPatches::new(
            vec![1, 17, 0],
            vec![0, 2, 3, 4],
            vec![0, 1, 2, 3],
            vec![None; 3],
        )
        .unwrap()
    }

    #[test]
    fn soil_field_uses_area_geometric_median_and_water_fills() {
        let area = [1.0, 3.0, 1.0, 1.0];
        let classes = SoilPatchClasses {
            water: 17,
            glacier: 15,
        };
        assert_eq!(
            aggregate_soil_field(
                &patches(),
                &[2.0, 6.0, f64::NAN, 4.0],
                &area,
                classes,
                SoilField {
                    statistic: SoilStatistic::AreaMean,
                    fill: 9.0
                },
            )
            .unwrap(),
            [5.0, 9.0, SURFACE_MISSING]
        );
        assert_eq!(
            aggregate_soil_field(
                &patches(),
                &[2.0, 8.0, 4.0, 4.0],
                &area,
                classes,
                SoilField {
                    statistic: SoilStatistic::GeometricMean,
                    fill: 9.0
                },
            )
            .unwrap()[0],
            2.0_f64.powf(0.25) * 8.0_f64.powf(0.75)
        );
    }

    #[test]
    fn vgm_fit_retains_a_uniform_source_column() {
        let layout = FlatPatches::new(vec![1], vec![0, 2], vec![0, 1], vec![None]).unwrap();
        let result = aggregate_vgm(
            &layout,
            VgmInputs {
                theta_r: &[0.1, 0.1],
                alpha: &[0.01, 0.01],
                n: &[1.5, 1.5],
                theta_s: &[0.45, 0.45],
                k_s: &[10.0, 10.0],
                l: &[0.5, 0.5],
            },
            &[1.0, 1.0],
            SoilPatchClasses {
                water: 17,
                glacier: 15,
            },
            VgmFills::default(),
            true,
        )
        .unwrap();
        assert!((result.theta_r[0] - 0.1).abs() < 1.0e-8);
        assert!((result.alpha[0] - 0.01).abs() < 1.0e-8);
        assert!((result.n[0] - 1.5).abs() < 1.0e-8);
        assert!((result.theta_s[0] - 0.45).abs() < 1.0e-8);
        assert!((result.k_s[0] - 10.0).abs() < 1.0e-8);
    }

    #[test]
    fn parallel_vgm_keeps_sequential_values_and_wmo_copies() {
        let layout = FlatPatches::new(
            vec![1, 1, 1],
            vec![0, 2, 4, 6],
            vec![0, 1, 2, 3, 4, 5],
            vec![None, None, Some(0)],
        )
        .unwrap();
        let input = VgmInputs {
            theta_r: &[0.10, 0.12, 0.08, 0.09, 0.2, 0.2],
            alpha: &[0.01, 0.02, 0.03, 0.04, 0.5, 0.5],
            n: &[1.5, 1.6, 1.7, 1.8, 2.0, 2.0],
            theta_s: &[0.45, 0.46, 0.47, 0.48, 0.5, 0.5],
            k_s: &[10.0, 11.0, 12.0, 13.0, 14.0, 15.0],
            l: &[0.4, 0.5, 0.6, 0.7, 0.8, 0.9],
        };
        let area = [1.0; 6];
        let classes = SoilPatchClasses {
            water: 17,
            glacier: 15,
        };
        for fit in [false, true] {
            let sequential = rayon::ThreadPoolBuilder::new()
                .num_threads(1)
                .build()
                .unwrap()
                .install(|| {
                    aggregate_vgm(&layout, input, &area, classes, VgmFills::default(), fit).unwrap()
                });
            let parallel = rayon::ThreadPoolBuilder::new()
                .num_threads(2)
                .build()
                .unwrap()
                .install(|| {
                    aggregate_vgm(&layout, input, &area, classes, VgmFills::default(), fit).unwrap()
                });
            assert_eq!(parallel, sequential);
            for field in [
                parallel.theta_r,
                parallel.alpha,
                parallel.n,
                parallel.theta_s,
                parallel.k_s,
                parallel.l,
            ] {
                assert_eq!(field[2], field[0]);
            }
        }
    }

    #[test]
    fn campbell_fit_retains_a_uniform_source_column() {
        let layout = FlatPatches::new(vec![1], vec![0, 2], vec![0, 1], vec![None]).unwrap();
        let result = aggregate_campbell(
            &layout,
            CampbellInputs {
                theta_s: &[0.45, 0.45],
                k_s: &[10.0, 10.0],
                psi_s: &[-35.0, -35.0],
                lambda: &[0.12, 0.12],
            },
            &[1.0, 1.0],
            SoilPatchClasses {
                water: 17,
                glacier: 15,
            },
            CampbellFills::default(),
            true,
        )
        .unwrap();
        assert!((result.theta_s[0] - 0.45).abs() < 1.0e-8);
        assert!((result.k_s[0] - 10.0).abs() < 1.0e-8);
        assert!((result.psi_s[0] + 35.0).abs() < 1.0e-8);
        assert!((result.lambda[0] - 0.12).abs() < 1.0e-8);
    }

    #[test]
    fn curve_jacobians_match_the_residual_functions() {
        let vgm = VgmProblem {
            theta_r: &[0.08, 0.12],
            alpha: &[0.008, 0.013],
            n: &[1.4, 1.7],
            theta_s: &[0.42, 0.49],
            k_s: &[8.0, 17.0],
            l: &[0.4, 0.6],
            phi: 0.46,
            conductivity: 12.0,
            l_patch: 0.5,
        };
        assert_jacobian(&vgm, &[0.1, 0.01, 1.5, 10.0], VGM_PRESSURES.len());
        let campbell = CampbellProblem {
            theta_s: &[0.42, 0.49],
            k_s: &[8.0, 17.0],
            psi_s: &[-30.0, -45.0],
            lambda: &[0.1, 0.15],
            phi: 0.46,
            conductivity: 12.0,
        };
        assert_jacobian(&campbell, &[-35.0, 0.12, 10.0], CAMPBELL_PRESSURES.len());
    }

    fn assert_jacobian(problem: &impl LeastSquaresProblem, x: &[f64], rows: usize) {
        let mut analytic = vec![0.0; rows * x.len()];
        assert!(problem.jacobian(x, &mut analytic));
        let mut left = vec![0.0; rows];
        let mut right = vec![0.0; rows];
        let mut trial = x.to_vec();
        for column in 0..x.len() {
            let step = x[column].abs().max(1.0) * 1.0e-6;
            trial[column] = x[column] - step;
            assert!(problem.residual(&trial, &mut left));
            trial[column] = x[column] + step;
            assert!(problem.residual(&trial, &mut right));
            for row in 0..rows {
                let expected = (right[row] - left[row]) / (2.0 * step);
                let actual = analytic[row * x.len() + column];
                assert!(
                    (actual - expected).abs() <= 2.0e-5 * expected.abs().max(1.0),
                    "row {row}, column {column}: expected {expected}, got {actual}"
                );
            }
            trial[column] = x[column];
        }
    }
}
