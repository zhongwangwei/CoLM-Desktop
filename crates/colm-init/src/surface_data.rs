//! Single-point `srfdata.nc` adapter used by the Rust `mkinidata` path.
//!
//! This is intentionally a strict reader: a missing static field is a scientific input
//! error, not a cue to substitute a plausible value.  The rawdata fallback remains the
//! responsibility of the Rust `mksrfdata` stage that creates `srfdata.nc`.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::{HydraulicModel, LandCoverScheme, SoilLayerInput, SoilReflectance};

const SOURCE_SOIL_LAYERS: usize = 8;

/// All single-point landdata needed by the static Rust initialization kernels.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointSurfaceData {
    pub latitude_degrees: f64,
    pub longitude_degrees: f64,
    pub land_class: i32,
    pub canopy_height_m: f64,
    pub lake_depth_m: f64,
    pub albedo: SoilReflectance,
    pub soil_texture: i32,
    pub elevation_m: f64,
    pub elevation_std_m: f64,
    pub slope_ratio: f64,
    /// Exactly eight source layers, from top to bottom.
    pub soil_layers: Vec<SoilLayerInput>,
}

/// Site monthly vegetation records stored alongside single-point surface data.
///
/// Values retain the NetCDF `(LAI_year, month)` order.  This is the source order
/// exposed by `MOD_SingleSrfdata.F90`, not a Rust-specific transpose.
#[derive(Debug, Clone, PartialEq)]
pub struct SinglePointMonthlyVegetation {
    pub years: Vec<i32>,
    pub lai: Vec<f64>,
    pub sai: Vec<f64>,
}

impl SinglePointMonthlyVegetation {
    /// Resolves the selected one-based calendar month exactly as `LAI_readin` does.
    pub fn for_year(
        &self,
        target_year: i32,
        month: u8,
        use_site_lai: bool,
        configured_start_year: i32,
        configured_end_year: i32,
    ) -> Result<(f64, f64)> {
        ensure!((1..=12).contains(&month), "month must be in 1..=12");
        ensure!(
            configured_start_year <= configured_end_year,
            "LAI configured start year exceeds end year"
        );
        let selected_year = if use_site_lai {
            *self
                .years
                .iter()
                .min_by_key(|&&year| (i64::from(year) - i64::from(target_year)).abs())
                .context("single-point surface has no LAI years")?
        } else {
            target_year.clamp(configured_start_year, configured_end_year)
        };
        let year_index = self
            .years
            .iter()
            .position(|&year| year == selected_year)
            .with_context(|| {
                format!("single-point surface has no LAI record for {selected_year}")
            })?;
        let index = year_index * 12 + usize::from(month - 1);
        Ok((self.lai[index], self.sai[index]))
    }
}

/// Reads the `LAI_year`, `LAI_monthly`, and `SAI_monthly` single-point contract.
pub fn read_single_point_monthly_vegetation(
    path: impl AsRef<Path>,
) -> Result<SinglePointMonthlyVegetation> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let years = vector_i32(&file, "LAI_year")?;
    ensure!(!years.is_empty(), "LAI_year must not be empty");
    for window in years.windows(2) {
        ensure!(
            window[0] < window[1],
            "LAI_year must be strictly increasing"
        );
    }
    let lai = monthly_vector(&file, "LAI_monthly", years.len())?;
    let sai = monthly_vector(&file, "SAI_monthly", years.len())?;
    Ok(SinglePointMonthlyVegetation { years, lai, sai })
}

/// Reads the static single-point contract produced by `MOD_SingleSrfdata.F90`.
pub fn read_single_point_surface(
    path: impl AsRef<Path>,
    land_cover: LandCoverScheme,
    hydraulic_model: HydraulicModel,
) -> Result<SinglePointSurfaceData> {
    let path = path.as_ref();
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open single-point surface data {}", path.display()))?;
    let land_name = match land_cover {
        LandCoverScheme::Igbp => "IGBP_classification",
        LandCoverScheme::Usgs => "USGS_classification",
    };
    let source = SoilSourceFields {
        vf_quartz: vector(&file, "soil_vf_quartz_mineral")?,
        vf_gravels: vector(&file, "soil_vf_gravels")?,
        vf_om: vector(&file, "soil_vf_om")?,
        vf_sand: vector(&file, "soil_vf_sand")?,
        vf_clay: vector(&file, "soil_vf_clay")?,
        wf_gravels: vector(&file, "soil_wf_gravels")?,
        wf_sand: vector(&file, "soil_wf_sand")?,
        wf_clay: vector(&file, "soil_wf_clay")?,
        wf_om: vector(&file, "soil_wf_om")?,
        om_density: vector(&file, "soil_OM_density")?,
        bulk_density: vector(&file, "soil_BD_all")?,
        theta_s: vector(&file, "soil_theta_s")?,
        psi_s_cm: vector(&file, "soil_psi_s")?,
        lambda: vector(&file, "soil_lambda")?,
        theta_r: optional_vgm(&file, "soil_theta_r", hydraulic_model)?,
        alpha_vgm: optional_vgm(&file, "soil_alpha_vgm", hydraulic_model)?,
        l_vgm: optional_vgm(&file, "soil_L_vgm", hydraulic_model)?,
        n_vgm: optional_vgm(&file, "soil_n_vgm", hydraulic_model)?,
        k_s_cm_day: vector(&file, "soil_k_s")?,
        csol: vector(&file, "soil_csol")?,
        k_solids: vector(&file, "soil_k_solids")?,
        tksatu: vector(&file, "soil_tksatu")?,
        tksatf: vector(&file, "soil_tksatf")?,
        tkdry: vector(&file, "soil_tkdry")?,
        ba_alpha: vector(&file, "soil_BA_alpha")?,
        ba_beta: vector(&file, "soil_BA_beta")?,
    };
    source.validate()?;

    Ok(SinglePointSurfaceData {
        latitude_degrees: scalar(&file, "latitude")?,
        longitude_degrees: scalar(&file, "longitude")?,
        land_class: scalar_i32(&file, land_name)?,
        canopy_height_m: scalar(&file, "canopy_height")?,
        lake_depth_m: scalar(&file, "lakedepth")?,
        albedo: SoilReflectance {
            saturated_visible: scalar(&file, "soil_s_v_alb")?,
            dry_visible: scalar(&file, "soil_d_v_alb")?,
            saturated_near_infrared: scalar(&file, "soil_s_n_alb")?,
            dry_near_infrared: scalar(&file, "soil_d_n_alb")?,
        },
        soil_texture: scalar_i32(&file, "soil_texture")?,
        elevation_m: scalar(&file, "elevation")?,
        elevation_std_m: scalar(&file, "elvstd")?,
        slope_ratio: scalar(&file, "sloperatio")?,
        soil_layers: source.into_layers(),
    })
}

struct SoilSourceFields {
    vf_quartz: Vec<f64>,
    vf_gravels: Vec<f64>,
    vf_om: Vec<f64>,
    vf_sand: Vec<f64>,
    vf_clay: Vec<f64>,
    wf_gravels: Vec<f64>,
    wf_sand: Vec<f64>,
    wf_clay: Vec<f64>,
    wf_om: Vec<f64>,
    om_density: Vec<f64>,
    bulk_density: Vec<f64>,
    theta_s: Vec<f64>,
    psi_s_cm: Vec<f64>,
    lambda: Vec<f64>,
    theta_r: Vec<f64>,
    alpha_vgm: Vec<f64>,
    l_vgm: Vec<f64>,
    n_vgm: Vec<f64>,
    k_s_cm_day: Vec<f64>,
    csol: Vec<f64>,
    k_solids: Vec<f64>,
    tksatu: Vec<f64>,
    tksatf: Vec<f64>,
    tkdry: Vec<f64>,
    ba_alpha: Vec<f64>,
    ba_beta: Vec<f64>,
}

impl SoilSourceFields {
    fn validate(&self) -> Result<()> {
        for (name, values) in [
            ("soil_vf_quartz_mineral", &self.vf_quartz),
            ("soil_vf_gravels", &self.vf_gravels),
            ("soil_vf_om", &self.vf_om),
            ("soil_vf_sand", &self.vf_sand),
            ("soil_vf_clay", &self.vf_clay),
            ("soil_wf_gravels", &self.wf_gravels),
            ("soil_wf_sand", &self.wf_sand),
            ("soil_wf_clay", &self.wf_clay),
            ("soil_wf_om", &self.wf_om),
            ("soil_OM_density", &self.om_density),
            ("soil_BD_all", &self.bulk_density),
            ("soil_theta_s", &self.theta_s),
            ("soil_psi_s", &self.psi_s_cm),
            ("soil_lambda", &self.lambda),
            ("soil_theta_r", &self.theta_r),
            ("soil_alpha_vgm", &self.alpha_vgm),
            ("soil_L_vgm", &self.l_vgm),
            ("soil_n_vgm", &self.n_vgm),
            ("soil_k_s", &self.k_s_cm_day),
            ("soil_csol", &self.csol),
            ("soil_k_solids", &self.k_solids),
            ("soil_tksatu", &self.tksatu),
            ("soil_tksatf", &self.tksatf),
            ("soil_tkdry", &self.tkdry),
            ("soil_BA_alpha", &self.ba_alpha),
            ("soil_BA_beta", &self.ba_beta),
        ] {
            ensure!(
                values.len() == SOURCE_SOIL_LAYERS,
                "{name} has {} values; expected {SOURCE_SOIL_LAYERS} source soil layers",
                values.len()
            );
        }
        Ok(())
    }

    fn into_layers(self) -> Vec<SoilLayerInput> {
        (0..SOURCE_SOIL_LAYERS)
            .map(|layer| SoilLayerInput {
                vf_quartz: self.vf_quartz[layer],
                vf_gravels: self.vf_gravels[layer],
                vf_om: self.vf_om[layer],
                vf_sand: self.vf_sand[layer],
                vf_clay: self.vf_clay[layer],
                wf_gravels: self.wf_gravels[layer],
                wf_sand: self.wf_sand[layer],
                wf_clay: self.wf_clay[layer],
                wf_om: self.wf_om[layer],
                om_density: self.om_density[layer],
                bulk_density: self.bulk_density[layer],
                theta_s: self.theta_s[layer],
                psi_s_cm: self.psi_s_cm[layer],
                lambda: self.lambda[layer],
                theta_r: self.theta_r[layer],
                alpha_vgm: self.alpha_vgm[layer],
                l_vgm: self.l_vgm[layer],
                n_vgm: self.n_vgm[layer],
                k_s_cm_day: self.k_s_cm_day[layer],
                csol: self.csol[layer],
                k_solids: self.k_solids[layer],
                tksatu: self.tksatu[layer],
                tksatf: self.tksatf[layer],
                tkdry: self.tkdry[layer],
                ba_alpha: self.ba_alpha[layer],
                ba_beta: self.ba_beta[layer],
            })
            .collect()
    }
}

fn optional_vgm(
    file: &netcdf::File,
    name: &str,
    hydraulic_model: HydraulicModel,
) -> Result<Vec<f64>> {
    match hydraulic_model {
        HydraulicModel::VanGenuchten => vector(file, name),
        HydraulicModel::Campbell => Ok(vec![0.0; SOURCE_SOIL_LAYERS]),
    }
}

fn scalar(file: &netcdf::File, name: &str) -> Result<f64> {
    let values = vector(file, name)?;
    ensure!(
        values.len() == 1,
        "{name} must be scalar, got {} values",
        values.len()
    );
    Ok(values[0])
}

fn scalar_i32(file: &netcdf::File, name: &str) -> Result<i32> {
    let values = vector_i32(file, name)?;
    ensure!(
        values.len() == 1,
        "{name} must be scalar, got {} values",
        values.len()
    );
    Ok(values[0])
}

fn vector_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    variable
        .get_values::<i32, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

fn vector(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))
}

fn monthly_vector(file: &netcdf::File, name: &str, years: usize) -> Result<Vec<f64>> {
    let variable = file
        .variable(name)
        .with_context(|| format!("single-point surface data is missing {name}"))?;
    let dimensions = variable.dimensions();
    ensure!(
        dimensions.len() == 2
            && dimensions[0].name() == "LAI_year"
            && dimensions[0].len() == years
            && dimensions[1].name() == "month"
            && dimensions[1].len() == 12,
        "{name} must have dimensions (LAI_year, month=12)"
    );
    let values = variable
        .get_values::<f64, _>(..)
        .with_context(|| format!("cannot read {name}"))?;
    ensure!(
        values.len() == years * 12 && values.iter().all(|value| value.is_finite()),
        "{name} must contain finite monthly values for every LAI year"
    );
    Ok(values)
}

#[cfg(test)]
#[path = "surface_data_tests.rs"]
mod surface_data_tests;
