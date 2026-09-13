//! NetCDF restart output for CoLM's PFT and PC state.
//!
//! `MOD_Vars_PFTimeInvariants` and `MOD_Vars_PFTimeVariables` use a separate
//! vector file family.  The arrays below retain the Fortran layout with PFT as
//! the final axis and serialize to NetCDF with PFT first, matching the vector
//! NetCDF writer's dimension reversal.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::RestartDate;
use colm_core::PFT_BGC_F64_VARIABLES;

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;
const WAVELENGTHS: usize = 211;
const BGC_ACTIVE_CROP_YEARS_AFTER: usize = 70;

/// Values stored by `WRITE_PFTimeInvariants`.
#[derive(Debug, Clone, Copy)]
pub struct PftConstantRestartInput<'a> {
    pub class: &'a [i32],
    pub fraction: &'a [f64],
    pub canopy_top_m: &'a [f64],
    pub canopy_bottom_m: &'a [f64],
    /// `CROP` only; one value per land patch.
    pub crop_fraction: Option<&'a [f64]>,
}

/// Required PFT/PC fields stored by `WRITE_PFTimeVariables`.
#[derive(Debug, Clone, Copy)]
pub struct PftTimeFields<'a> {
    pub leaf_temperature_k: &'a [f64],
    pub canopy_water_mm: &'a [f64],
    pub canopy_rain_mm: &'a [f64],
    pub canopy_snow_mm: &'a [f64],
    pub wet_snow_fraction: &'a [f64],
    pub vegetation_fraction: &'a [f64],
    pub total_lai: &'a [f64],
    pub lai: &'a [f64],
    pub total_sai: &'a [f64],
    pub sai: &'a [f64],
    /// `band * rtyp * pft` in the Fortran array order.
    pub sunlit_absorption: &'a [f64],
    /// `band * rtyp * pft` in the Fortran array order.
    pub shaded_absorption: &'a [f64],
    pub thermal_gap_fraction: &'a [f64],
    pub shade_fraction: &'a [f64],
    pub direct_extinction: &'a [f64],
    pub diffuse_extinction: &'a [f64],
    pub reference_temperature_k: &'a [f64],
    pub reference_humidity: &'a [f64],
    pub stomatal_resistance_s_m: &'a [f64],
    pub roughness_length_m: &'a [f64],
}

/// `HYPERSPECTRAL` PFT canopy absorption fields.
#[derive(Debug, Clone, Copy)]
pub struct PftHyperspectralFields<'a> {
    /// `wavelength * rtyp * pft` in the Fortran array order.
    pub sunlit_absorption: &'a [f64],
    /// `wavelength * rtyp * pft` in the Fortran array order.
    pub shaded_absorption: &'a [f64],
}

/// `DEF_USE_PLANTHYDRAULICS` PFT restart fields.
#[derive(Debug, Clone, Copy)]
pub struct PftPlantHydraulicFields<'a> {
    /// `vegnodes * pft` in the Fortran array order.
    pub water_potential_mm: &'a [f64],
    pub sunlit_stomatal_conductance: &'a [f64],
    pub shaded_stomatal_conductance: &'a [f64],
    pub vegetation_nodes: usize,
}

/// `DEF_USE_OZONESTRESS` fields that the upstream PFT writer persists.
#[derive(Debug, Clone, Copy)]
pub struct PftOzoneFields<'a> {
    pub lai_old: &'a [f64],
    pub sunlit_uptake: &'a [f64],
    pub shaded_uptake: &'a [f64],
    pub sunlit_vegetation_coefficient: &'a [f64],
    pub shaded_vegetation_coefficient: &'a [f64],
    pub sunlit_stomatal_coefficient: &'a [f64],
    pub shaded_stomatal_coefficient: &'a [f64],
}

/// Carbon, nitrogen, and phenology state emitted when `DEF_USE_BGC` is enabled.
///
/// `values` follows [`PFT_BGC_F64_VARIABLES`]. Every slice contains one value
/// per PFT; `nyrs_crop_active_p` is kept separate because it is integer data.
#[derive(Debug, Clone, Copy)]
pub struct PftBgcFields<'a> {
    pub values: &'a [&'a [f64]],
    pub active_crop_years: &'a [i32],
}

/// All PFT/PC fields for one time restart vector block.
#[derive(Debug, Clone, Copy)]
pub struct PftTimeRestartInput<'a> {
    pub fields: PftTimeFields<'a>,
    pub hyperspectral: Option<PftHyperspectralFields<'a>>,
    pub plant_hydraulics: Option<PftPlantHydraulicFields<'a>>,
    pub bgc: Option<PftBgcFields<'a>>,
    pub ozone: Option<PftOzoneFields<'a>>,
    /// `DEF_USE_IRRIGATION` only.
    pub irrigation_method: Option<&'a [i32]>,
}

/// Writes the `const/<case>_restart_pft_const_lcYYYY_<block>.nc` file.
pub fn write_pft_constant_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    block_label: &str,
    input: PftConstantRestartInput<'_>,
) -> Result<PathBuf> {
    validate_restart_name(case_name, "case name")?;
    validate_restart_name(block_label, "block label")?;
    validate_year(land_cover_year)?;
    let path = restart_dir.as_ref().join("const").join(format!(
        "{case_name}_restart_pft_const_lc{land_cover_year:04}_{block_label}.nc"
    ));
    write_pft_constant_restart_block(&path, input)?;
    Ok(path)
}

/// Writes one already-addressed PFT/PC constant restart vector block.
pub fn write_pft_constant_restart_block(
    path: impl AsRef<Path>,
    input: PftConstantRestartInput<'_>,
) -> Result<()> {
    let pfts = validate_constant_input(input)?;
    let path = path.as_ref();
    create_parent(path)?;
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create PFT constant restart {}", path.display()))?;
    file.add_dimension("pft", pfts)?;
    if let Some(crop_fraction) = input.crop_fraction {
        file.add_dimension("patch", crop_fraction.len())?;
    }
    file.add_variable::<i32>("pftclass", &["pft"])?
        .put_values(input.class, ..)?;
    for (name, values) in [
        ("pftfrac", input.fraction),
        ("htop_p", input.canopy_top_m),
        ("hbot_p", input.canopy_bottom_m),
    ] {
        put_f64_1d(&mut file, name, "pft", values)?;
    }
    if let Some(crop_fraction) = input.crop_fraction {
        put_f64_1d(&mut file, "cropfrac", "patch", crop_fraction)?;
    }
    Ok(())
}

/// Writes the timestamped PFT/PC vector block in CoLM's restart tree.
pub fn write_pft_time_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    date: RestartDate,
    block_label: &str,
    input: PftTimeRestartInput<'_>,
) -> Result<PathBuf> {
    validate_restart_name(case_name, "case name")?;
    validate_restart_name(block_label, "block label")?;
    validate_year(land_cover_year)?;
    validate_date(date)?;
    let date = date_label(date);
    let path = restart_dir.as_ref().join(&date).join(format!(
        "{case_name}_restart_pft_{date}_lc{land_cover_year:04}_{block_label}.nc"
    ));
    write_pft_time_restart_block(&path, input)?;
    Ok(path)
}

/// Writes one already-addressed PFT/PC time restart vector block.
pub fn write_pft_time_restart_block(
    path: impl AsRef<Path>,
    input: PftTimeRestartInput<'_>,
) -> Result<()> {
    let pfts = validate_time_input(input)?;
    let path = path.as_ref();
    create_parent(path)?;
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create PFT time restart {}", path.display()))?;
    file.add_dimension("pft", pfts)?;
    file.add_dimension("band", BANDS)?;
    file.add_dimension("rtyp", RADIATION_TYPES)?;
    // Upstream defines this dimension even when HYPERSPECTRAL is disabled.
    file.add_dimension("wavelength", WAVELENGTHS)?;
    if let Some(plant) = input.plant_hydraulics {
        file.add_dimension("vegnodes", plant.vegetation_nodes)?;
    }

    let entries = pft_entries(input.fields);
    for &(name, values) in &entries[..10] {
        put_f64_1d(&mut file, name, "pft", values)?;
    }
    for (name, values) in [
        ("ssun_p", input.fields.sunlit_absorption),
        ("ssha_p", input.fields.shaded_absorption),
    ] {
        put_pft_last_3d(
            &mut file,
            name,
            [("band", BANDS), ("rtyp", RADIATION_TYPES)],
            pfts,
            values,
        )?;
    }
    if let Some(hyperspectral) = input.hyperspectral {
        for (name, values) in [
            ("ssun_hires_p", hyperspectral.sunlit_absorption),
            ("ssha_hires_p", hyperspectral.shaded_absorption),
        ] {
            put_pft_last_3d(
                &mut file,
                name,
                [("wavelength", WAVELENGTHS), ("rtyp", RADIATION_TYPES)],
                pfts,
                values,
            )?;
        }
    }
    for &(name, values) in &entries[10..] {
        put_f64_1d(&mut file, name, "pft", values)?;
    }
    if let Some(plant) = input.plant_hydraulics {
        put_axis_major(
            &mut file,
            "vegwp_p",
            "vegnodes",
            plant.vegetation_nodes,
            pfts,
            plant.water_potential_mm,
        )?;
        put_f64_1d(
            &mut file,
            "gs0sun_p",
            "pft",
            plant.sunlit_stomatal_conductance,
        )?;
        put_f64_1d(
            &mut file,
            "gs0sha_p",
            "pft",
            plant.shaded_stomatal_conductance,
        )?;
    }
    if let Some(ozone) = input.ozone {
        for (name, values) in [
            ("lai_old_p", ozone.lai_old),
            ("o3uptakesun_p", ozone.sunlit_uptake),
            ("o3uptakesha_p", ozone.shaded_uptake),
            ("o3coefv_sun_p", ozone.sunlit_vegetation_coefficient),
            ("o3coefv_sha_p", ozone.shaded_vegetation_coefficient),
            ("o3coefg_sun_p", ozone.sunlit_stomatal_coefficient),
            ("o3coefg_sha_p", ozone.shaded_stomatal_coefficient),
        ] {
            put_f64_1d(&mut file, name, "pft", values)?;
        }
    }
    if let Some(irrigation_method) = input.irrigation_method {
        file.add_variable::<i32>("irrig_method_p", &["pft"])?
            .put_values(irrigation_method, ..)?;
    }
    if let Some(bgc) = input.bgc {
        for (index, (&name, values)) in PFT_BGC_F64_VARIABLES.iter().zip(bgc.values).enumerate() {
            if index == BGC_ACTIVE_CROP_YEARS_AFTER {
                file.add_variable::<i32>("nyrs_crop_active_p", &["pft"])?
                    .put_values(bgc.active_crop_years, ..)?;
            }
            put_f64_1d(&mut file, name, "pft", values)?;
        }
    }
    Ok(())
}

fn pft_entries(fields: PftTimeFields<'_>) -> [(&'static str, &[f64]); 18] {
    [
        ("tleaf_p", fields.leaf_temperature_k),
        ("ldew_p", fields.canopy_water_mm),
        ("ldew_rain_p", fields.canopy_rain_mm),
        ("ldew_snow_p", fields.canopy_snow_mm),
        ("fwet_snow_p", fields.wet_snow_fraction),
        ("sigf_p", fields.vegetation_fraction),
        ("tlai_p", fields.total_lai),
        ("lai_p", fields.lai),
        ("tsai_p", fields.total_sai),
        ("sai_p", fields.sai),
        ("thermk_p", fields.thermal_gap_fraction),
        ("fshade_p", fields.shade_fraction),
        ("extkb_p", fields.direct_extinction),
        ("extkd_p", fields.diffuse_extinction),
        ("tref_p", fields.reference_temperature_k),
        ("qref_p", fields.reference_humidity),
        ("rst_p", fields.stomatal_resistance_s_m),
        ("z0m_p", fields.roughness_length_m),
    ]
}

fn validate_constant_input(input: PftConstantRestartInput<'_>) -> Result<usize> {
    let pfts = input.class.len();
    ensure!(pfts > 0, "a PFT restart block needs at least one PFT");
    validate_pft_values(
        "PFT constants",
        pfts,
        &[
            ("pftfrac", input.fraction),
            ("htop_p", input.canopy_top_m),
            ("hbot_p", input.canopy_bottom_m),
        ],
    )?;
    if let Some(crop_fraction) = input.crop_fraction {
        ensure!(
            !crop_fraction.is_empty(),
            "cropfrac needs at least one patch when CROP is enabled"
        );
    }
    Ok(pfts)
}

fn validate_time_input(input: PftTimeRestartInput<'_>) -> Result<usize> {
    let pfts = input.fields.leaf_temperature_k.len();
    ensure!(pfts > 0, "a PFT restart block needs at least one PFT");
    validate_pft_values("PFT time state", pfts, &pft_entries(input.fields))?;
    for (name, values) in [
        ("ssun_p", input.fields.sunlit_absorption),
        ("ssha_p", input.fields.shaded_absorption),
    ] {
        validate_pft_last_3d(name, values, BANDS, RADIATION_TYPES, pfts)?;
    }
    if let Some(hyperspectral) = input.hyperspectral {
        for (name, values) in [
            ("ssun_hires_p", hyperspectral.sunlit_absorption),
            ("ssha_hires_p", hyperspectral.shaded_absorption),
        ] {
            validate_pft_last_3d(name, values, WAVELENGTHS, RADIATION_TYPES, pfts)?;
        }
    }
    if let Some(plant) = input.plant_hydraulics {
        ensure!(plant.vegetation_nodes > 0, "vegnodes must be positive");
        validate_axis_major(
            "vegwp_p",
            plant.water_potential_mm,
            plant.vegetation_nodes,
            pfts,
        )?;
        validate_pft_values(
            "PFT plant hydraulics",
            pfts,
            &[
                ("gs0sun_p", plant.sunlit_stomatal_conductance),
                ("gs0sha_p", plant.shaded_stomatal_conductance),
            ],
        )?;
    }
    if let Some(bgc) = input.bgc {
        ensure!(
            bgc.values.len() == PFT_BGC_F64_VARIABLES.len(),
            "BGC PFT restart has {} fields; expected {}",
            bgc.values.len(),
            PFT_BGC_F64_VARIABLES.len()
        );
        for (&name, values) in PFT_BGC_F64_VARIABLES.iter().zip(bgc.values) {
            ensure!(
                values.len() == pfts,
                "BGC PFT field {name} has {} entries; expected {pfts}",
                values.len()
            );
        }
        ensure!(
            bgc.active_crop_years.len() == pfts,
            "nyrs_crop_active_p has {} entries; expected {pfts}",
            bgc.active_crop_years.len()
        );
    }
    if let Some(ozone) = input.ozone {
        validate_pft_values(
            "PFT ozone",
            pfts,
            &[
                ("lai_old_p", ozone.lai_old),
                ("o3uptakesun_p", ozone.sunlit_uptake),
                ("o3uptakesha_p", ozone.shaded_uptake),
                ("o3coefv_sun_p", ozone.sunlit_vegetation_coefficient),
                ("o3coefv_sha_p", ozone.shaded_vegetation_coefficient),
                ("o3coefg_sun_p", ozone.sunlit_stomatal_coefficient),
                ("o3coefg_sha_p", ozone.shaded_stomatal_coefficient),
            ],
        )?;
    }
    if let Some(irrigation_method) = input.irrigation_method {
        ensure!(
            irrigation_method.len() == pfts,
            "irrig_method_p has {} entries; expected {pfts}",
            irrigation_method.len()
        );
    }
    Ok(pfts)
}

fn create_parent(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    Ok(())
}

fn validate_restart_name(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{name} must be a nonempty filename component"
    );
    Ok(())
}

fn validate_year(year: i32) -> Result<()> {
    ensure!(
        (0..=9999).contains(&year),
        "land-cover year {year} is outside the four-digit restart filename range"
    );
    Ok(())
}

fn validate_date(date: RestartDate) -> Result<()> {
    ensure!(
        (0..=9999).contains(&date.year)
            && (1..=366).contains(&date.julian_day)
            && date.seconds < 86_400,
        "restart date is not a valid CoLM year, Julian day, and seconds-of-day"
    );
    Ok(())
}

fn date_label(date: RestartDate) -> String {
    format!(
        "{:04}-{:03}-{:05}",
        date.year, date.julian_day, date.seconds
    )
}

fn validate_pft_values(name: &str, pfts: usize, fields: &[(&str, &[f64])]) -> Result<()> {
    if let Some((field, values)) = fields.iter().find(|(_, values)| values.len() != pfts) {
        anyhow::bail!(
            "{name} field {field} has {} entries; expected {pfts}",
            values.len()
        );
    }
    Ok(())
}

fn validate_axis_major(name: &str, values: &[f64], axis: usize, pfts: usize) -> Result<()> {
    ensure!(
        values.len() == axis * pfts,
        "{name} has {} entries; expected {axis} x {pfts}",
        values.len()
    );
    Ok(())
}

fn validate_pft_last_3d(
    name: &str,
    values: &[f64],
    first: usize,
    second: usize,
    pfts: usize,
) -> Result<()> {
    ensure!(
        values.len() == first * second * pfts,
        "{name} has {} entries; expected {first} x {second} x {pfts}",
        values.len()
    );
    Ok(())
}

fn put_f64_1d(
    file: &mut netcdf::FileMut,
    name: &str,
    dimension: &str,
    values: &[f64],
) -> Result<()> {
    file.add_variable::<f64>(name, &[dimension])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_axis_major(
    file: &mut netcdf::FileMut,
    name: &str,
    axis_name: &str,
    axis: usize,
    pfts: usize,
    values: &[f64],
) -> Result<()> {
    validate_axis_major(name, values, axis, pfts)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for pft in 0..pfts {
        for axis_index in 0..axis {
            on_disk.push(values[axis_index * pfts + pft]);
        }
    }
    file.add_variable::<f64>(name, &["pft", axis_name])?
        .put_values(&on_disk, (.., ..))?;
    Ok(())
}

fn put_pft_last_3d(
    file: &mut netcdf::FileMut,
    name: &str,
    [(first_name, first), (second_name, second)]: [(&str, usize); 2],
    pfts: usize,
    values: &[f64],
) -> Result<()> {
    validate_pft_last_3d(name, values, first, second, pfts)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for pft in 0..pfts {
        for second_index in 0..second {
            for first_index in 0..first {
                on_disk.push(values[(first_index * second + second_index) * pfts + pft]);
            }
        }
    }
    file.add_variable::<f64>(name, &["pft", second_name, first_name])?
        .put_values(&on_disk, (.., .., ..))?;
    Ok(())
}

#[cfg(test)]
#[path = "pft_restart_tests.rs"]
mod pft_restart_tests;
