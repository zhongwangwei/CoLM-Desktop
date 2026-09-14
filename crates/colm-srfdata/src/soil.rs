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
            let fitted = (self.phi - x[0]).mul_add(
                (1.0 + (x[1] * pressure).powf(x[2])).powf(1.0 / x[2] - 1.0),
                x[0],
            );
            let base = 1.0 + (x[1] * pressure).powf(x[2]);
            let term = 1.0 - (1.0 - 1.0 / base).powf(1.0 - 1.0 / x[2]);
            let fitted_log = ((1.0 / x[2] - 1.0) * self.l_patch)
                .mul_add(base.log10(), x[3].log10())
                + term.powi(2).log10();
            for &[observed, observed_log_k] in
                &self.samples[index * self.cells..(index + 1) * self.cells]
            {
                let theta_residual = (fitted - observed) / self.phi;
                let conductivity_residual =
                    (fitted_log - observed_log_k) / self.conductivity.log10();
                retention = theta_residual.mul_add(theta_residual, retention);
                conductivity = conductivity_residual.mul_add(conductivity_residual, conductivity);
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
            let fitted_theta = (self.phi - x[0]).mul_add(q, x[0]);
            let u = 1.0 - 1.0 / base;
            let power = 1.0 - 1.0 / x[2];
            let u_power = u.powf(power);
            let term = 1.0 - u_power;
            let fitted_log = ((1.0 / x[2] - 1.0) * self.l_patch)
                .mul_add(base.log10(), x[3].log10())
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
            let log_n_leading = -self.l_patch * base.log10() / x[2].powi(2)
                + (1.0 / x[2] - 1.0) * self.l_patch * z_to_n * z.log10() / base;
            let log_n_product = 2.0 * u_power / term;
            let log_n_inner = u.log10() / x[2].powi(2) + power * z.log10() / base;
            let log_n = log_n_product.mul_add(-log_n_inner, log_n_leading);
            let mut retention = [0.0; 3];
            let mut conductivity = [0.0; 3];
            for &[observed_theta, observed_log_k] in
                &self.samples[row * self.cells..(row + 1) * self.cells]
            {
                let theta_residual = (fitted_theta - observed_theta) / self.phi;
                let conductivity_residual = (fitted_log - observed_log_k) / log_conductivity;
                retention[0] += 2.0 * theta_residual * (1.0 - q) / self.phi;
                let alpha_derivative_prefix = 2.0 * theta_residual / self.phi
                    * (self.phi - x[0])
                    * (1.0 - x[2])
                    * q_alpha_power
                    * alpha_power;
                retention[1] = alpha_derivative_prefix.mul_add(pressure_power, retention[1]);
                let n_derivative_prefix = 2.0 * theta_residual / self.phi * (self.phi - x[0]) * q;
                retention[2] = n_derivative_prefix.mul_add(q_n_factor, retention[2]);
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
                let theta_residual = (fitted - observed) / self.phi;
                let conductivity_residual =
                    (fitted_log - observed_log_k) / self.conductivity.log10();
                retention = theta_residual.mul_add(theta_residual, retention);
                conductivity = conductivity_residual.mul_add(conductivity_residual, conductivity);
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
    fn curve_residual_sums_match_original_fused_square_accumulation() {
        // /tmp/colm-soil-callback-golden/original_callback_golden.f90
        // compiled with: gfortran -O2 -fdefault-real-8 -ffree-line-length-none.
        // Both original SW_VG_dist and SW_CB_dist return 3F96E05AEA7035C1;
        // separate square-then-add reductions return 3F96E05AEA7035C2.
        let first_delta = f64::from_bits(0x3fbd_9541_8fc4_c914);
        let second_delta = f64::from_bits(0x3fb8_4497_ddfc_4485);
        let observed = [[1.0 - first_delta, 1.0], [1.0 - second_delta, 1.0]];

        let mut vgm_samples = Vec::with_capacity(VGM_PRESSURES.len() * observed.len());
        for pressure in VGM_PRESSURES {
            let x = [1.0, 1.0, 2.0, 1.0];
            let base = 1.0 + (x[1] * pressure).powf(x[2]);
            let term = 1.0 - (1.0 - 1.0 / base).powf(1.0 - 1.0 / x[2]);
            let fitted_log =
                x[3].log10() + (1.0 / x[2] - 1.0) * 0.5 * base.log10() + term.powi(2).log10();
            vgm_samples.extend(observed.iter().map(|sample| [sample[0], fitted_log]));
        }
        let vgm = VgmProblem {
            samples: vgm_samples,
            cells: observed.len(),
            phi: 1.0,
            conductivity: 10.0,
            l_patch: 0.5,
        };
        let mut residual = [0.0; 24];
        assert!(vgm.residual(&[1.0, 1.0, 2.0, 1.0], &mut residual));
        assert_eq!(residual[7].to_bits(), 0x3f96_e05a_ea70_35c1);

        let campbell = CampbellProblem {
            samples: observed.repeat(CAMPBELL_PRESSURES.len()),
            cells: observed.len(),
            phi: 1.0,
            conductivity: 10.0,
        };
        let mut residual = [0.0; 17];
        assert!(campbell.residual(&[-60.0, 1.0, 10.0], &mut residual));
        assert_eq!(residual[0].to_bits(), 0x3f96_e05a_ea70_35c1);
    }

    #[test]
    fn vgm_callback_matches_original_fma_operand_contract() {
        // Golden generated by the unchanged SW_VG_dist callback using the first
        // two cells from /tmp/colm-original-fit-input-probe/vgm_l5_e207390_c14_inputs.txt:
        // /tmp/colm-lm-qr-audit/subset_callback_original.f90
        // compiled in /tmp with: gfortran -J/tmp/colm-lm-qr-audit -O2 -fdefault-real-8 -ffree-line-length-none.
        fn bits(hex: &str) -> Vec<u64> {
            hex.split_whitespace()
                .map(|value| u64::from_str_radix(value, 16).expect("valid hex golden"))
                .collect()
        }

        let theta_bits = bits("3FDC52E742187508 3FDC894426602D16 3FDC33E883C0E8B2 3FDC6945D3AC9A60 3FDC02CCCC4F99E9 3FDC36AF4B0F16F5 3FDB9469923A1B10 3FDBC540DF9C6B17 3FDB2287B4600C20 3FDB508DA35F09A9 3FDAB35C5017DF89 3FDADEE609BBAE8E 3FDA4958B26FED68 3FDA72BA9572AE62 3FD9E55CBAEECBD8 3FDA0CE20237FA9F 3FD9878D84B78FCC 3FD9AD769340095A 3FD8DD80AFC9A0BF 3FD900CAFC170006 3FD8484495FD1048 3FD869890EA01823 3FD7C49298F52174 3FD7E43D482A0D6E 3FD74F85545B97A1 3FD76DE417DDB329 3FD6E6B774360CA1 3FD70403B7861FCE 3FD632761520A2C0 3FD64E154F7DCB68 3FD4FFA786BF3E87 3FD518CF4B3043C1 3FD488A7457BFB61 3FD4A0F3887EE48A 3FD258E38E7CBECD 3FD26D96239D4CCF 3FD13BF102912BA3 3FD14EED72C6257F 3FCB4BE70CFD0BFD 3FCB6612929BF1D2 3FC7F30CFF72C828 3FC806974A1DC6BB 3FC747EFE62B3D33 3FC75A047A12459D 3FC430D502C1F7C4 3FC43B5CF4D6A927 3FC1A7A7BEC340B8 3FC1AA9039F2DC64");
        let conductivity_bits = bits("4022B1CA4110DAD3 4022C857805EF7E6 4015E1C5A7298982 4015DF23A2B15361 400DAF2DC421B7E5 400D8F0FC312AFF3 40010869175BBDB4 4000DF364145977F 3FF61B1272970D3A 3FF5D03EDDCE761A 3FEEAA3886BF0DB6 3FEE2B968B98263F 3FE63FE1EC4E732A 3FE5D6DE05D23FBE 3FE0B29C33B16993 3FE05BD80347959E 3FD9BDB80BD17751 3FD92DED9DF321AA 3FD0466FACE91F39 3FCFC3C9EA7910EB 3FC5ED95E0521D1F 3FC55CEF0170CA13 3FBEFEE54011DE19 3FBE28F75A7875DD 3FB6BF26D12B0D46 3FB61CFBE0C68B56 3FB13522AD62B06E 3FB0B77DB7709425 3FA524B27BA3F055 3FA485557BCC68F8 3F91E2986C4C1A85 3F91574E34BAAADD 3F8943263978DC1F 3F887D0B87597075 3F60F4CF1BA4CD83 3F606FEE763CEAB7 3F4817F6EBD0F9C5 3F475E7CE9C9C78D 3EE40DE5236B614A 3EE386E0C50B7F7A 3EA02EE9A926879B 3E9F9CE44D617FE3 3E8F39EFCBC2C065 3E8E85C7CDD715A6 3E28FE2F7F7F1CE9 3E288B00C6BDA308 3D98386E02209226 3D97F15E720C2662");
        let samples = theta_bits
            .into_iter()
            .zip(conductivity_bits)
            .map(|(theta, conductivity)| {
                [f64::from_bits(theta), f64::from_bits(conductivity).log10()]
            })
            .collect::<Vec<_>>();
        let problem = VgmProblem {
            samples,
            cells: 2,
            phi: f64::from_bits(0x3FDE0D4832268437),
            conductivity: f64::from_bits(0x402CF82E61C674FD),
            l_patch: f64::from_bits(0x3FE0000000000000),
        };
        let x = [
            f64::from_bits(0x3FC92287134C083B),
            f64::from_bits(0x3F87A58C255A465A),
            f64::from_bits(0x3FF4ACF39CBD580B),
            f64::from_bits(0x402CF82E61C674FD),
        ];
        let mut residual = [0.0; 24];
        let mut jacobian = [0.0; 96];
        assert!(problem.residual(&x, &mut residual));
        assert!(problem.jacobian(&x, &mut jacobian));

        assert_eq!(
            residual.map(f64::to_bits).as_slice(),
            bits("3F911BFE24EBF3F7 3F8B2310F7DAE3D8 3F872BBFCB341A0E 3F83D1D520EB2F08 3F833E276431B3F2 3F8408F813A8F3D5 3F858BDB9536EBA6 3F876BC89A2A3A92 3F897468F6BF253F 3F8D92D964C7E823 3F90B8B6D5B9C99A 3F9277B1ADE7C564 3F94056680870A19 3F95661B219DC93B 3F97B6D868D0907C 3F9B700EA7655B2C 3F9CCCA85A82EEC4 3FA15009300D369A 3FA2A63D33118393 3FA6A9DD5D5C61CB 3FA886D9A735BADC 3FA8E73C5C878260 3FAAA816FE083A99 3FAC0FA77D8533DC").as_slice(),
            "VGM residual FMA operands must match original callback bits"
        );
        assert_eq!(
            jacobian.map(f64::to_bits).as_slice(),
            bits("3F35281DE4F5F1F2 40009870DF6A4789 BFD786A9EA6AA1F2 BF7F4DA50F3F065A 3F655F7AF8BFFDA5 400A9E2845B7CF28 BFD8F5906D899478 BF796B9F22CB0C6C 3F7A856B73237D9A 400DF95F6FF6CEEF BFD6A04A079BBC69 BF74D5E200C05AC4 3F9095BF5870275A 400B19AF67F32079 BFD0C474E7F58759 BF6C3A45E70E9DA2 3F9C50E25ACF062E 40026ADB0F877A83 BFC6F24D31C0C742 BF62053AB2891ADE 3FA4902A1E6F7619 3FED00565C022EC8 BFBCA23BE798120E BF53B087AC87F62E 3FAB441368925A79 BFE32167F0DF790E BFAE4442EAB4D392 BF39272BD61913B7 3FB10C18FB38DCD8 C000D0F649156E9B BF93679501DA339D 3F336D72D5DB6787 3FB476EE79711C61 C00C6471E8E5392C 3F8657D208708094 3F4C7628F477BA9C 3FBB28C1FA4BD93E C018B30529DD4DD0 3FA8503B14A03348 3F5D01ADD2978A89 3FC0C45DEB88ECF1 C020D0C68BCF8DD3 3FB01D3D58B26688 3F63FF9F41939B6D 3FC3C30713B897E8 C0248E33620F6C3E 3FB0B4745D95CC8D 3F68322FD5070F71 3FC68F2B9C4D7496 C027AD90A067789B 3FAE6B833C38AE90 3F6B798006B6A514 3FC92AE39E35CB9C C02A4A9841C4D378 3FA8DE4C2588F220 3F6E1596D42032E1 3FCDDF33B003EF8F C02E59557442DF55 3F9394599552F528 3F70F6B820834819 3FD33D0F7812F5DB C031F608BB016120 BFACC9E82BAC8D10 3F736D9830977032 3FD500F380E13D83 C032D0FF886E17D9 BFB79390F5942FFC 3F7426419FA0FCB9 3FDDFE06C7429FCC C0354B6FF677F0D7 BFD2AEE79B3C1D94 3F76292810C4E368 3FE17F083E088B96 C035B6C1DBF20B64 BFD9E505AD019731 3F767E37B1BAE38D 3FEA8A6C85651AC1 C0345740B72603E2 BFE8AD7D54CF5A94 3F756B232D8932B0 3FEF4EB9E89F5F60 C0329AF4559DA9F9 BFEDC386AB10494A 3F73FA9F940A6D0C 3FF026E409666CAA C0322C8F5CB3EFD4 BFEEB7AB7D1BA546 3F739B935A12F85C 3FF28E93C19A6406 C02F529E7603C744 BFF14F039A6EEE4F 3F715425BDB6D4FF 3FF4AAAE1BA85DBF C028B742A3EA7559 BFF2090904ECD08A 3F6C11BF28E9394F").as_slice(),
            "VGM Jacobian FMA operands must match original callback bits"
        );
        let mut fit = x;
        assert!(lmder(&problem, &mut fit, 24));
        assert_eq!(
            fit.map(f64::to_bits),
            [
                0x3FB8D9AC55556D3F,
                0x3F8C3353C983A9BF,
                0x3FF4964CCB0C2C2E,
                0x40335F93306E0B1A,
            ],
            "VGM lmder final fit must match original callback plus MOD_Utils bits"
        );
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
