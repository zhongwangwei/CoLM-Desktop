//! BGC time-varying restart output from `WRITE_BGCTimeVariables`.
//!
//! Buffers use the original Fortran axis order with patch last. NetCDF vector
//! files reverse that order, so this module owns the required transpose.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::RestartDate;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BgcTimeRestartDimensions {
    pub soil_layers: usize,
    pub full_soil_layers: usize,
    pub decomposition_pools: usize,
    pub days_per_year: usize,
}

impl Default for BgcTimeRestartDimensions {
    fn default() -> Self {
        Self {
            soil_layers: 10,
            full_soil_layers: 15,
            decomposition_pools: 7,
            days_per_year: 365,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BgcTotals<'a> {
    pub litter_carbon: &'a [f64],
    pub vegetation_carbon: &'a [f64],
    pub soil_carbon: &'a [f64],
    pub coarse_woody_carbon: &'a [f64],
    pub total_carbon: &'a [f64],
    pub litter_nitrogen: &'a [f64],
    pub vegetation_nitrogen: &'a [f64],
    pub soil_nitrogen: &'a [f64],
    pub coarse_woody_nitrogen: &'a [f64],
    pub total_nitrogen: &'a [f64],
    pub mineral_nitrogen: &'a [f64],
    pub deposition: &'a [f64],
}

/// Layer-major fields are `axis * patch`; pool fields are `soil * pool * patch`.
#[derive(Debug, Clone, Copy)]
pub struct BgcPoolFields<'a> {
    pub carbon: &'a [f64],
    pub nitrogen: &'a [f64],
    pub total_soil_nitrogen: &'a [f64],
    pub mineral_nitrogen: &'a [f64],
    pub nitrate: &'a [f64],
    pub ammonium: &'a [f64],
    pub lagged_npp: &'a [f64],
}

#[derive(Debug, Clone, Copy)]
pub struct BgcTruncationFields<'a> {
    pub carbon_profile: &'a [f64],
    pub carbon_vegetation: &'a [f64],
    pub carbon_soil: &'a [f64],
    pub nitrogen_profile: &'a [f64],
    pub nitrogen_vegetation: &'a [f64],
    pub nitrogen_soil: &'a [f64],
}

#[derive(Debug, Clone, Copy)]
pub struct BgcPermafrostFields<'a> {
    pub maximum_active_layer_depth: &'a [f64],
    pub previous_maximum_active_layer_depth: &'a [f64],
    pub previous_maximum_active_layer_index: &'a [i32],
}

#[derive(Debug, Clone, Copy)]
pub struct BgcClimateFields<'a> {
    pub precipitation_10_day: &'a [f64],
    pub precipitation_60_day: &'a [f64],
    pub precipitation_365_day: &'a [f64],
    pub precipitation_today: &'a [f64],
    /// `day * patch` in Fortran order.
    pub precipitation_daily: &'a [f64],
    pub soil_temperature_17: &'a [f64],
    pub relative_humidity_30_day: &'a [f64],
    pub accumulated_steps: &'a [f64],
    pub skip_balance_check: &'a [i8],
}

#[derive(Debug, Clone, Copy)]
pub struct BgcNitrificationFields<'a> {
    pub oxygen_concentration_unsaturated: &'a [f64],
    pub oxygen_decomposition_depth_unsaturated: &'a [f64],
}

/// `CROP` patch state written after the shared BGC restart fields.
#[derive(Debug, Clone, Copy)]
pub struct BgcCropFields<'a> {
    pub crop_phase: &'a [f64],
    pub planting_day_corn: &'a [f64],
    pub planting_day_spring_wheat: &'a [f64],
    pub planting_day_winter_wheat: &'a [f64],
    pub planting_day_soybean: &'a [f64],
    pub planting_day_cotton: &'a [f64],
    pub planting_day_rice1: &'a [f64],
    pub planting_day_rice2: &'a [f64],
    pub planting_day_sugarcane: &'a [f64],
    pub fertilizer_nitrogen_corn: &'a [f64],
    pub fertilizer_nitrogen_spring_wheat: &'a [f64],
    pub fertilizer_nitrogen_winter_wheat: &'a [f64],
    pub fertilizer_nitrogen_soybean: &'a [f64],
    pub fertilizer_nitrogen_cotton: &'a [f64],
    pub fertilizer_nitrogen_rice1: &'a [f64],
    pub fertilizer_nitrogen_rice2: &'a [f64],
    pub fertilizer_nitrogen_sugarcane: &'a [f64],
}

#[derive(Debug, Clone, Copy)]
pub struct BgcTimeRestartInput<'a> {
    pub dimensions: BgcTimeRestartDimensions,
    pub totals: BgcTotals<'a>,
    pub pools: BgcPoolFields<'a>,
    pub truncation: BgcTruncationFields<'a>,
    pub permafrost: BgcPermafrostFields<'a>,
    pub climate: BgcClimateFields<'a>,
    pub nitrification: Option<BgcNitrificationFields<'a>>,
    pub crop: Option<BgcCropFields<'a>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgcTimeRestartFile {
    pub block: PathBuf,
}

pub fn write_bgc_time_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    date: RestartDate,
    block_label: &str,
    input: BgcTimeRestartInput<'_>,
) -> Result<BgcTimeRestartFile> {
    validate_filename_component(case_name, "case name")?;
    validate_filename_component(block_label, "block label")?;
    validate_year(land_cover_year)?;
    validate_date(date)?;
    let date_label = date_label(date);
    let block = restart_dir.as_ref().join(&date_label).join(format!(
        "{case_name}_restart_bgc_{date_label}_lc{land_cover_year:04}_{block_label}.nc"
    ));
    write_bgc_time_restart_block(&block, input)?;
    Ok(BgcTimeRestartFile { block })
}

pub fn write_bgc_time_restart_block(
    path: impl AsRef<Path>,
    input: BgcTimeRestartInput<'_>,
) -> Result<()> {
    let patches = validate_input(input)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create BGC time restart {}", path.display()))?;
    let dimensions = input.dimensions;
    for (name, length) in [
        ("patch", patches),
        ("soil", dimensions.soil_layers),
        ("soil_full", dimensions.full_soil_layers),
        ("ndecomp_pools", dimensions.decomposition_pools),
        ("doy", dimensions.days_per_year),
    ] {
        file.add_dimension(name, length)?;
    }

    for (name, values) in total_entries(input.totals) {
        put_f64_1d(&mut file, name, "patch", values)?;
    }
    put_pool_major(
        &mut file,
        "decomp_cpools_vr",
        dimensions.full_soil_layers,
        dimensions.decomposition_pools,
        patches,
        input.pools.carbon,
    )?;
    put_f64_axis_major(
        &mut file,
        "ctrunc_vr",
        "soil",
        dimensions.soil_layers,
        patches,
        input.truncation.carbon_profile,
    )?;
    for (name, values) in [
        ("ctrunc_veg", input.truncation.carbon_vegetation),
        ("ctrunc_soil", input.truncation.carbon_soil),
        ("altmax", input.permafrost.maximum_active_layer_depth),
        (
            "altmax_lastyear",
            input.permafrost.previous_maximum_active_layer_depth,
        ),
    ] {
        put_f64_1d(&mut file, name, "patch", values)?;
    }
    put_i32_1d(
        &mut file,
        "altmax_lastyear_indx",
        "patch",
        input.permafrost.previous_maximum_active_layer_index,
    )?;
    put_pool_major(
        &mut file,
        "decomp_npools_vr",
        dimensions.full_soil_layers,
        dimensions.decomposition_pools,
        patches,
        input.pools.nitrogen,
    )?;
    for (name, values) in [
        ("totsoiln_vr", input.pools.total_soil_nitrogen),
        ("ntrunc_vr", input.truncation.nitrogen_profile),
    ] {
        put_f64_axis_major(
            &mut file,
            name,
            "soil",
            dimensions.soil_layers,
            patches,
            values,
        )?;
    }
    for (name, values) in [
        ("ntrunc_veg", input.truncation.nitrogen_vegetation),
        ("ntrunc_soil", input.truncation.nitrogen_soil),
    ] {
        put_f64_1d(&mut file, name, "patch", values)?;
    }
    for (name, values) in [
        ("sminn_vr", input.pools.mineral_nitrogen),
        ("smin_no3_vr", input.pools.nitrate),
        ("smin_nh4_vr", input.pools.ammonium),
    ] {
        put_f64_axis_major(
            &mut file,
            name,
            "soil",
            dimensions.soil_layers,
            patches,
            values,
        )?;
    }
    put_f64_1d(&mut file, "lag_npp", "patch", input.pools.lagged_npp)?;
    if let Some(nitrification) = input.nitrification {
        for (name, values) in [
            (
                "tCONC_O2_UNSAT",
                nitrification.oxygen_concentration_unsaturated,
            ),
            (
                "tO2_DECOMP_DEPTH_UNSAT",
                nitrification.oxygen_decomposition_depth_unsaturated,
            ),
        ] {
            put_f64_axis_major(
                &mut file,
                name,
                "soil",
                dimensions.soil_layers,
                patches,
                values,
            )?;
        }
    }
    for (name, values) in precipitation_entries(input.climate) {
        put_f64_1d(&mut file, name, "patch", values)?;
    }
    put_f64_axis_major(
        &mut file,
        "prec_daily",
        "doy",
        dimensions.days_per_year,
        patches,
        input.climate.precipitation_daily,
    )?;
    for (name, values) in climate_entries(input.climate) {
        put_f64_1d(&mut file, name, "patch", values)?;
    }
    file.add_variable::<i8>("skip_balance_check", &["patch"])?
        .put_values(input.climate.skip_balance_check, ..)?;
    if let Some(crop) = input.crop {
        for (name, values) in crop_entries(crop) {
            put_f64_1d(&mut file, name, "patch", values)?;
        }
    }
    file.close()
        .with_context(|| format!("cannot close BGC time restart {}", path.display()))?;
    Ok(())
}

fn crop_entries(crop: BgcCropFields<'_>) -> [(&'static str, &[f64]); 17] {
    [
        ("cphase", crop.crop_phase),
        ("pdcorn", crop.planting_day_corn),
        ("pdswheat", crop.planting_day_spring_wheat),
        ("pdwwheat", crop.planting_day_winter_wheat),
        ("pdsoybean", crop.planting_day_soybean),
        ("pdcotton", crop.planting_day_cotton),
        ("pdrice1", crop.planting_day_rice1),
        ("pdrice2", crop.planting_day_rice2),
        ("pdsugarcane", crop.planting_day_sugarcane),
        ("fertnitro_corn", crop.fertilizer_nitrogen_corn),
        ("fertnitro_swheat", crop.fertilizer_nitrogen_spring_wheat),
        ("fertnitro_wwheat", crop.fertilizer_nitrogen_winter_wheat),
        ("fertnitro_soybean", crop.fertilizer_nitrogen_soybean),
        ("fertnitro_cotton", crop.fertilizer_nitrogen_cotton),
        ("fertnitro_rice1", crop.fertilizer_nitrogen_rice1),
        ("fertnitro_rice2", crop.fertilizer_nitrogen_rice2),
        ("fertnitro_sugarcane", crop.fertilizer_nitrogen_sugarcane),
    ]
}

fn total_entries(fields: BgcTotals<'_>) -> [(&'static str, &[f64]); 12] {
    [
        ("totlitc", fields.litter_carbon),
        ("totvegc", fields.vegetation_carbon),
        ("totsomc", fields.soil_carbon),
        ("totcwdc", fields.coarse_woody_carbon),
        ("totcolc", fields.total_carbon),
        ("totlitn", fields.litter_nitrogen),
        ("totvegn", fields.vegetation_nitrogen),
        ("totsomn", fields.soil_nitrogen),
        ("totcwdn", fields.coarse_woody_nitrogen),
        ("totcoln", fields.total_nitrogen),
        ("sminn", fields.mineral_nitrogen),
        ("ndep", fields.deposition),
    ]
}

fn precipitation_entries(fields: BgcClimateFields<'_>) -> [(&'static str, &[f64]); 4] {
    [
        ("prec10", fields.precipitation_10_day),
        ("prec60", fields.precipitation_60_day),
        ("prec365", fields.precipitation_365_day),
        ("prec_today", fields.precipitation_today),
    ]
}

fn climate_entries(fields: BgcClimateFields<'_>) -> [(&'static str, &[f64]); 3] {
    [
        ("tsoi17", fields.soil_temperature_17),
        ("rh30", fields.relative_humidity_30_day),
        ("accumnstep", fields.accumulated_steps),
    ]
}

fn validate_input(input: BgcTimeRestartInput<'_>) -> Result<usize> {
    let dimensions = input.dimensions;
    for (name, value) in [
        ("soil", dimensions.soil_layers),
        ("soil_full", dimensions.full_soil_layers),
        ("ndecomp_pools", dimensions.decomposition_pools),
        ("doy", dimensions.days_per_year),
    ] {
        ensure!(value > 0, "BGC restart dimension {name} must be positive");
    }
    let patches = input.totals.litter_carbon.len();
    ensure!(patches > 0, "a BGC time restart needs at least one patch");
    validate_patch_fields("BGC totals", patches, &total_entries(input.totals))?;
    validate_patch_fields(
        "BGC scalar state",
        patches,
        &[
            ("ctrunc_veg", input.truncation.carbon_vegetation),
            ("ctrunc_soil", input.truncation.carbon_soil),
            ("ntrunc_veg", input.truncation.nitrogen_vegetation),
            ("ntrunc_soil", input.truncation.nitrogen_soil),
            ("altmax", input.permafrost.maximum_active_layer_depth),
            (
                "altmax_lastyear",
                input.permafrost.previous_maximum_active_layer_depth,
            ),
            ("lag_npp", input.pools.lagged_npp),
            ("prec10", input.climate.precipitation_10_day),
            ("prec60", input.climate.precipitation_60_day),
            ("prec365", input.climate.precipitation_365_day),
            ("prec_today", input.climate.precipitation_today),
            ("tsoi17", input.climate.soil_temperature_17),
            ("rh30", input.climate.relative_humidity_30_day),
            ("accumnstep", input.climate.accumulated_steps),
        ],
    )?;
    ensure!(
        input.permafrost.previous_maximum_active_layer_index.len() == patches,
        "altmax_lastyear_indx has {} entries; expected {patches}",
        input.permafrost.previous_maximum_active_layer_index.len()
    );
    ensure!(
        input.climate.skip_balance_check.len() == patches,
        "skip_balance_check has {} entries; expected {patches}",
        input.climate.skip_balance_check.len()
    );
    validate_axis(
        "decomp_cpools_vr",
        input.pools.carbon,
        dimensions.full_soil_layers * dimensions.decomposition_pools,
        patches,
    )?;
    validate_axis(
        "decomp_npools_vr",
        input.pools.nitrogen,
        dimensions.full_soil_layers * dimensions.decomposition_pools,
        patches,
    )?;
    for (name, values) in [
        ("totsoiln_vr", input.pools.total_soil_nitrogen),
        ("ctrunc_vr", input.truncation.carbon_profile),
        ("ntrunc_vr", input.truncation.nitrogen_profile),
        ("sminn_vr", input.pools.mineral_nitrogen),
        ("smin_no3_vr", input.pools.nitrate),
        ("smin_nh4_vr", input.pools.ammonium),
    ] {
        validate_axis(name, values, dimensions.soil_layers, patches)?;
    }
    validate_axis(
        "prec_daily",
        input.climate.precipitation_daily,
        dimensions.days_per_year,
        patches,
    )?;
    if let Some(crop) = input.crop {
        validate_patch_fields("CROP BGC state", patches, &crop_entries(crop))?;
    }
    if let Some(nitrification) = input.nitrification {
        validate_axis(
            "tCONC_O2_UNSAT",
            nitrification.oxygen_concentration_unsaturated,
            dimensions.soil_layers,
            patches,
        )?;
        validate_axis(
            "tO2_DECOMP_DEPTH_UNSAT",
            nitrification.oxygen_decomposition_depth_unsaturated,
            dimensions.soil_layers,
            patches,
        )?;
    }
    Ok(patches)
}

fn validate_patch_fields(name: &str, patches: usize, fields: &[(&str, &[f64])]) -> Result<()> {
    if let Some((field, values)) = fields.iter().find(|(_, values)| values.len() != patches) {
        anyhow::bail!(
            "{name} field {field} has {} entries; expected {patches}",
            values.len()
        );
    }
    Ok(())
}

fn validate_axis(name: &str, values: &[f64], axis: usize, patches: usize) -> Result<()> {
    ensure!(
        values.len() == axis * patches,
        "{name} has {} entries; expected {axis} x {patches}",
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

fn put_i32_1d(
    file: &mut netcdf::FileMut,
    name: &str,
    dimension: &str,
    values: &[i32],
) -> Result<()> {
    file.add_variable::<i32>(name, &[dimension])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_f64_axis_major(
    file: &mut netcdf::FileMut,
    name: &str,
    axis_name: &str,
    axis: usize,
    patches: usize,
    values: &[f64],
) -> Result<()> {
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for index in 0..axis {
            on_disk.push(values[index * patches + patch]);
        }
    }
    file.add_variable::<f64>(name, &["patch", axis_name])?
        .put_values(&on_disk, (.., ..))?;
    Ok(())
}

fn put_pool_major(
    file: &mut netcdf::FileMut,
    name: &str,
    soils: usize,
    pools: usize,
    patches: usize,
    values: &[f64],
) -> Result<()> {
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for pool in 0..pools {
            for soil in 0..soils {
                on_disk.push(values[(soil * pools + pool) * patches + patch]);
            }
        }
    }
    file.add_variable::<f64>(name, &["patch", "ndecomp_pools", "soil_full"])?
        .put_values(&on_disk, (.., .., ..))?;
    Ok(())
}

fn validate_filename_component(value: &str, name: &str) -> Result<()> {
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

#[cfg(test)]
#[path = "bgc_time_restart_tests.rs"]
mod tests;
