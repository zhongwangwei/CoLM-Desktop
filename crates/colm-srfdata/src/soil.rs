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
        let problem = VgmProblem::new(
            VgmInputs {
                theta_r: &theta_r,
                alpha: &alpha,
                n: &n,
                theta_s: &theta_s,
                k_s: &k_s,
                l: &l,
            },
            values[3],
            values[4],
            values[5],
        );
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
    let values: Vec<_> = (0..patches.len())
        .into_par_iter()
        .map(|patch| aggregate_campbell_patch(patches, patch, input, area, classes, fills, fit))
        .collect();
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
        if let Some(values) = values[patch] {
            output.theta_s[patch] = values[0];
            output.k_s[patch] = values[1];
            output.psi_s[patch] = values[2];
            output.lambda[patch] = values[3];
        }
    }
    Ok(output)
}

fn aggregate_campbell_patch(
    patches: &FlatPatches,
    patch: usize,
    input: CampbellInputs<'_>,
    area: &[f64],
    classes: SoilPatchClasses,
    fills: CampbellFills,
    fit: bool,
) -> Option<[f64; 4]> {
    if patches.wmo_source_for(patch).is_some() || patches.patch_type_for(patch) == 0 {
        return None;
    }
    let theta_s = filled_values(patches, patch, input.theta_s, classes, fills.theta_s);
    let k_s = filled_values(patches, patch, input.k_s, classes, fills.k_s);
    let psi_s = filled_values(patches, patch, input.psi_s, classes, fills.psi_s);
    let lambda = filled_values(patches, patch, input.lambda, classes, fills.lambda);
    let cells = patches.raw_cells(patch);
    let mut values = [
        statistic(&theta_s, cells, area, SoilStatistic::AreaMean),
        statistic(&k_s, cells, area, SoilStatistic::GeometricMean),
        statistic(&psi_s, cells, area, SoilStatistic::Median),
        statistic(&lambda, cells, area, SoilStatistic::Median),
    ];
    if fit && cells.len() > 1 {
        let problem = CampbellProblem::new(
            CampbellInputs {
                theta_s: &theta_s,
                k_s: &k_s,
                psi_s: &psi_s,
                lambda: &lambda,
            },
            values[0],
            values[1],
        );
        let mut x = [values[2], values[3], values[1]];
        if lmder(&problem, &mut x, CAMPBELL_PRESSURES.len())
            && (-300.0..0.0).contains(&x[0])
            && x[1] > 0.0
            && x[1] <= 1.0
            && x[2] > 0.0
            && x[2] <= 1.0e7
        {
            values[2] = x[0];
            values[3] = x[1];
            values[1] = x[2];
        }
    }
    Some(values)
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

struct VgmProblem {
    samples: Vec<[f64; 2]>,
    cells: usize,
    phi: f64,
    conductivity: f64,
    l_patch: f64,
}

impl VgmProblem {
    fn new(input: VgmInputs<'_>, phi: f64, conductivity: f64, l_patch: f64) -> Self {
        let cells = input.theta_r.len();
        let mut samples = Vec::with_capacity(cells * VGM_PRESSURES.len());
        // Upstream builds ydatv/ydatvks once, not on every LM iteration.
        for pressure in VGM_PRESSURES {
            for cell in 0..cells {
                let theta = (1.0 + (input.alpha[cell] * pressure).powf(input.n[cell]))
                    .powf(1.0 / input.n[cell] - 1.0);
                // The original array expression contracts this final product.
                let observed =
                    (input.theta_s[cell] - input.theta_r[cell]).mul_add(theta, input.theta_r[cell]);
                let observed_k = input.k_s[cell]
                    * theta.powf(input.l[cell])
                    * (1.0
                        - (1.0 - theta.powf(input.n[cell] / (input.n[cell] - 1.0)))
                            .powf(1.0 - 1.0 / input.n[cell]))
                    .powi(2);
                samples.push([observed, observed_k.log10()]);
            }
        }
        Self {
            samples,
            cells,
            phi,
            conductivity,
            l_patch,
        }
    }
}

impl LeastSquaresProblem for VgmProblem {
    fn residual(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[1] <= 0.0 || x[2] <= 0.1 || x[2] >= 100.0 || x[3] <= 0.0 {
            return false;
        }
        for (index, pressure) in VGM_PRESSURES.iter().copied().enumerate() {
            // Upstream adds two independently accumulated SUMs.
            let mut retention = 0.0;
            let mut conductivity = 0.0;
            let fitted = x[0]
                + (self.phi - x[0]) * (1.0 + (x[1] * pressure).powf(x[2])).powf(1.0 / x[2] - 1.0);
            let base = 1.0 + (x[1] * pressure).powf(x[2]);
            let term = 1.0 - (1.0 - 1.0 / base).powf(1.0 - 1.0 / x[2]);
            let fitted_log = x[3].log10()
                + (1.0 / x[2] - 1.0) * self.l_patch * base.log10()
                + term.powi(2).log10();
            for &[observed, observed_log_k] in
                &self.samples[index * self.cells..(index + 1) * self.cells]
            {
                retention += ((fitted - observed) / self.phi).powi(2);
                conductivity += ((fitted_log - observed_log_k) / self.conductivity.log10()).powi(2);
            }
            output[index] = retention + conductivity;
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
            // Retain SW_VG_dist's powers and multiplication order. Equivalent
            // chain-rule rearrangements perturb this poorly conditioned fit.
            let alpha_power = x[1].powf(x[2] - 1.0);
            let pressure_power = pressure.powf(x[2]);
            let q_alpha_power = base.powf(1.0 / x[2] - 2.0);
            let q_n_factor =
                (1.0 - x[2]) * z_to_n * z.ln() / (x[2] * base) - base.ln() / x[2].powi(2);
            let fitted_theta = x[0] + (self.phi - x[0]) * q;
            let u = 1.0 - 1.0 / base;
            let power = 1.0 - 1.0 / x[2];
            let u_power = u.powf(power);
            let term = 1.0 - u_power;
            let fitted_log = x[3].log10()
                + (1.0 / x[2] - 1.0) * self.l_patch * base.log10()
                + term.powi(2).log10();
            let log_alpha = self.l_patch * (1.0 - x[2]) * alpha_power * pressure_power
                / (base * std::f64::consts::LN_10)
                + 2.0
                    * (1.0 - x[2])
                    * u.powf(-1.0 / x[2])
                    * alpha_power
                    * pressure_power
                    * base.powi(-2)
                    / (term * std::f64::consts::LN_10);
            let log_n = -self.l_patch * base.log10() / x[2].powi(2)
                + (1.0 / x[2] - 1.0) * self.l_patch * z_to_n * z.log10() / base
                - 2.0 * u_power / term * (u.log10() / x[2].powi(2) + power * z.log10() / base);
            let mut retention = [0.0; 3];
            let mut conductivity = [0.0; 3];
            for &[observed_theta, observed_log_k] in
                &self.samples[row * self.cells..(row + 1) * self.cells]
            {
                let theta_residual = (fitted_theta - observed_theta) / self.phi;
                let conductivity_residual = (fitted_log - observed_log_k) / log_conductivity;
                retention[0] += 2.0 * theta_residual * (1.0 - q) / self.phi;
                retention[1] += 2.0 * theta_residual / self.phi
                    * (self.phi - x[0])
                    * (1.0 - x[2])
                    * q_alpha_power
                    * alpha_power
                    * pressure_power;
                retention[2] +=
                    2.0 * theta_residual / self.phi * (self.phi - x[0]) * q * q_n_factor;
                conductivity[0] += 2.0 * conductivity_residual * log_alpha / log_conductivity;
                conductivity[1] += 2.0 * conductivity_residual * log_n / log_conductivity;
                conductivity[2] += 2.0 * conductivity_residual
                    / (x[3] * std::f64::consts::LN_10)
                    / log_conductivity;
            }
            let derivatives = [
                retention[0],
                retention[1] + conductivity[0],
                retention[2] + conductivity[1],
                conductivity[2],
            ];
            for (column, value) in derivatives.into_iter().enumerate() {
                output[row * 4 + column] = value;
            }
        }
        output.iter().all(|value| value.is_finite())
    }
}

struct CampbellProblem {
    samples: Vec<[f64; 2]>,
    cells: usize,
    phi: f64,
    conductivity: f64,
}

impl CampbellProblem {
    fn new(input: CampbellInputs<'_>, phi: f64, conductivity: f64) -> Self {
        let cells = input.theta_s.len();
        let mut samples = Vec::with_capacity(cells * CAMPBELL_PRESSURES.len());
        for pressure in CAMPBELL_PRESSURES {
            for cell in 0..cells {
                let ratio = -pressure / input.psi_s[cell];
                let observed = ratio.powf(-input.lambda[cell]) * input.theta_s[cell];
                let observed_k = ratio.powf(-3.0 * input.lambda[cell] - 2.0) * input.k_s[cell];
                samples.push([observed, observed_k.log10()]);
            }
        }
        Self {
            samples,
            cells,
            phi,
            conductivity,
        }
    }
}

impl LeastSquaresProblem for CampbellProblem {
    fn residual(&self, x: &[f64], output: &mut [f64]) -> bool {
        if x[0] >= 0.0 || x[1].abs() >= 100.0 || x[2] <= 0.0 {
            return false;
        }
        for (index, pressure) in CAMPBELL_PRESSURES.iter().copied().enumerate() {
            // Upstream adds two independently accumulated SUMs.
            let mut retention = 0.0;
            let mut conductivity = 0.0;
            let fitted = (-pressure / x[0]).powf(-x[1]) * self.phi;
            let fitted_log = (-pressure / x[0]).log10() * (-3.0 * x[1] - 2.0) + x[2].log10();
            for &[observed, observed_log_k] in
                &self.samples[index * self.cells..(index + 1) * self.cells]
            {
                retention += ((fitted - observed) / self.phi).powi(2);
                conductivity += ((fitted_log - observed_log_k) / self.conductivity.log10()).powi(2);
            }
            output[index] = retention + conductivity;
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
            let theta = ratio.powf(-x[1]);
            let mut retention = [0.0; 2];
            let mut conductivity = [0.0; 3];
            for &[observed_theta, observed_log_k] in
                &self.samples[row * self.cells..(row + 1) * self.cells]
            {
                let theta_residual = (fitted_theta - observed_theta) / self.phi;
                let conductivity_residual = (fitted_log - observed_log_k) / log_conductivity;
                retention[0] += 2.0 * theta_residual * x[1] * theta / x[0];
                retention[1] += -2.0 * theta_residual * theta * ratio.ln();
                conductivity[0] += 2.0 * conductivity_residual * (3.0 * x[1] + 2.0)
                    / (x[0] * std::f64::consts::LN_10)
                    / log_conductivity;
                conductivity[1] += -6.0 * conductivity_residual * ratio.log10() / log_conductivity;
                conductivity[2] += 2.0 * conductivity_residual
                    / (x[2] * std::f64::consts::LN_10 * log_conductivity);
            }
            let derivatives = [
                retention[0] + conductivity[0],
                retention[1] + conductivity[1],
                conductivity[2],
            ];
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
                // The production Fortran SUM contracts this weighted product.
                // One ULP in porosity can change the subsequent LM trajectory.
                .fold(0.0, |sum, (value, cell)| {
                    value.mul_add(area[*cell] / total, sum)
                })
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
    fn parallel_soil_keeps_sequential_values_and_wmo_copies() {
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
            let campbell = CampbellInputs {
                theta_s: input.theta_s,
                k_s: input.k_s,
                psi_s: &[-30.0, -45.0, -35.0, -55.0, -40.0, -60.0],
                lambda: &[0.1, 0.15, 0.12, 0.18, 0.2, 0.3],
            };
            let run = |threads| {
                rayon::ThreadPoolBuilder::new()
                    .num_threads(threads)
                    .build()
                    .unwrap()
                    .install(|| {
                        aggregate_campbell(
                            &layout,
                            campbell,
                            &area,
                            classes,
                            CampbellFills::default(),
                            fit,
                        )
                        .unwrap()
                    })
            };
            let cb = run(2);
            assert_eq!(cb, run(1));
            for field in [&cb.theta_s, &cb.k_s, &cb.psi_s, &cb.lambda] {
                assert_eq!(field[0], field[2]);
            }
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
    fn soil_inputs_retain_production_fortran_single_rounding() {
        // Original -O2 contracts each final weighted product into its SUM.
        // The separate multiply/add yields 0.07999999999999999 instead.
        assert_eq!(
            statistic(&[0.08, 0.08], &[0, 1], &[1.0, 2.0], SoilStatistic::AreaMean),
            0.08
        );
        // Original production array expression, element 207867/class 2/layer 1,
        // source cell 1 at pressure 5. A scalar Fortran probe does not contract.
        let problem = VgmProblem::new(
            VgmInputs {
                theta_r: &[f64::from_bits(0x3fb329ce57ce1825)],
                theta_s: &[f64::from_bits(0x3fe0fab010392ec0)],
                alpha: &[f64::from_bits(0x3f95c098cea15564)],
                n: &[f64::from_bits(0x3ff4a2210f47ab99)],
                k_s: &[10.0],
                l: &[0.5],
            },
            0.5,
            10.0,
            0.5,
        );
        assert_eq!(problem.samples[1][0].to_bits(), 0x3fe0cdb06b4c933b);
    }

    #[test]
    fn curve_residuals_keep_the_two_upstream_sum_reductions_separate() {
        // SW_CB_dist/SW_VG_dist compute SUM(retention²) + SUM(conductivity²).
        // Interleaving both channels loses the small conductivity contribution.
        let cb = CampbellProblem {
            samples: [[-99_999_999.0, 0.0], [0.0, 0.0]].repeat(CAMPBELL_PRESSURES.len()),
            cells: 2,
            phi: 1.0,
            conductivity: 10.0,
        };
        let mut residual = [0.0; 17];
        assert!(cb.residual(&[-60.0, 1.0, 10.0], &mut residual));
        assert_eq!(residual[0], 10_000_000_000_000_002.0);
        let mut jacobian = [0.0; 17 * 3];
        assert!(cb.jacobian(&[-60.0, 1.0, 10.0], &mut jacobian));
        // Original SW_CB_dist, same synthetic inputs, production real-kind flags.
        assert_eq!(jacobian[0].to_bits(), 0xc1496e6ac1769652);
        assert_eq!(jacobian[1], 0.0);
        assert_eq!(jacobian[2].to_bits(), 0x3fc63c62775250d7);

        let log_k = -0.25 * 2.0_f64.log10() + (1.0 - 0.5_f64.sqrt()).powi(2).log10();
        let vg = VgmProblem {
            samples: [[-99_999_999.0, log_k - 1.0], [0.0, log_k - 1.0]].repeat(VGM_PRESSURES.len()),
            cells: 2,
            phi: 1.0,
            conductivity: 10.0,
            l_patch: 0.5,
        };
        let mut residual = [0.0; 24];
        assert!(vg.residual(&[1.0, 1.0, 2.0, 1.0], &mut residual));
        assert_eq!(residual[0], 10_000_000_000_000_002.0);
    }

    #[test]
    fn curve_jacobians_match_the_residual_functions() {
        let vgm = VgmProblem::new(
            VgmInputs {
                theta_r: &[0.08, 0.12],
                alpha: &[0.008, 0.013],
                n: &[1.4, 1.7],
                theta_s: &[0.42, 0.49],
                k_s: &[8.0, 17.0],
                l: &[0.4, 0.6],
            },
            0.46,
            12.0,
            0.5,
        );
        assert_jacobian(&vgm, &[0.1, 0.01, 1.5, 10.0], VGM_PRESSURES.len());
        let campbell = CampbellProblem::new(
            CampbellInputs {
                theta_s: &[0.42, 0.49],
                k_s: &[8.0, 17.0],
                psi_s: &[-30.0, -45.0],
                lambda: &[0.1, 0.15],
            },
            0.46,
            12.0,
        );
        assert_jacobian(&campbell, &[-35.0, 0.12, 10.0], CAMPBELL_PRESSURES.len());
    }

    fn assert_jacobian(problem: &impl LeastSquaresProblem, x: &[f64], rows: usize) {
        let mut residual = vec![0.0; rows];
        assert!(problem.residual(x, &mut residual));
        // Recorded before invariant source-curve precomputation; also check
        // the full analytic Jacobian below against finite differences.
        let expected: &[f64] = match rows {
            24 => &[
                0.09599902381414789,
                0.1159547520305157,
                0.1213355001809209,
                0.1140519363687003,
                0.09756423113189755,
                0.07868190486103455,
                0.06056346133327949,
                0.04468417676245743,
                0.031590960453860575,
                0.013694972901779588,
                0.00506761571966774,
                0.0031990170912509813,
                0.006002839421341494,
                0.011960351337433566,
                0.029449520200716464,
                0.07883548423658349,
                0.10432614990147343,
                0.27359430180917377,
                0.397002189470646,
                1.1074778055779406,
                1.7847149472069783,
                1.973952008783068,
                3.310727800466762,
                5.819281686934102,
            ],
            17 => &[
                0.257525410364438,
                0.2493316919350615,
                0.23650092077838236,
                0.22669662463617563,
                0.21881508417037904,
                0.21225616438481512,
                0.2066590247880124,
                0.19749211220688015,
                0.18277502752975155,
                0.17725147718254705,
                0.15169522312723793,
                0.13851531472941594,
                0.09218378023307755,
                0.06695019343127888,
                0.06151387614002263,
                0.03448555081173949,
                0.01217891333703709,
            ],
            _ => unreachable!(),
        };
        for (&actual, &expected) in residual.iter().zip(expected) {
            assert!((actual - expected).abs() <= 1e-12 * expected.abs().max(1.0));
        }
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
