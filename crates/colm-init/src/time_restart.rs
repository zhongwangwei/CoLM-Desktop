//! NetCDF initial-time restart output for `mkinidata`.
//!
//! Buffers keep CoLM's native axis-major layout with patch as the last axis.  The
//! NetCDF writer reverses that layout to `(patch, ...)`, exactly as the Fortran
//! NetCDF interface does for its `( ..., patch )` arrays.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

/// Dimensions used by `WRITE_TimeVariables` for one restart block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRestartDimensions {
    pub soil_layers: usize,
    pub lake_layers: usize,
    pub snow_layers: usize,
    pub bands: usize,
    pub radiation_types: usize,
}

impl Default for TimeRestartDimensions {
    fn default() -> Self {
        Self {
            soil_layers: 10,
            lake_layers: 10,
            snow_layers: 5,
            bands: 2,
            radiation_types: 2,
        }
    }
}

/// Snow and soil profile fields, each axis-major with patch as its final axis.
#[derive(Debug, Clone, Copy)]
pub struct SnowSoilRestartFields<'a> {
    /// `snow * patches`.
    pub snow_node_depth_m: &'a [f64],
    /// `snow * patches`.
    pub snow_layer_thickness_m: &'a [f64],
    /// `soilsnow * patches`.
    pub temperature_k: &'a [f64],
    /// `soilsnow * patches`.
    pub liquid_water_kg_m2: &'a [f64],
    /// `soilsnow * patches`.
    pub ice_water_kg_m2: &'a [f64],
    /// `soil * patches`.
    pub matric_potential_mm: &'a [f64],
    /// `soil * patches`.
    pub hydraulic_conductivity_mm_s: &'a [f64],
}

/// One scalar time-varying value per patch in the standard restart file.
#[derive(Debug, Clone, Copy)]
pub struct TimePatchFields<'a> {
    pub ground_temperature_k: &'a [f64],
    pub leaf_temperature_k: &'a [f64],
    pub canopy_water_mm: &'a [f64],
    pub canopy_rain_mm: &'a [f64],
    pub canopy_snow_mm: &'a [f64],
    pub wet_snow_fraction: &'a [f64],
    pub snow_age: &'a [f64],
    pub snow_water_equivalent_mm: &'a [f64],
    pub snow_depth_m: &'a [f64],
    pub vegetation_fraction: &'a [f64],
    pub ground_snow_fraction: &'a [f64],
    pub snow_free_vegetation_fraction: &'a [f64],
    pub greenness: &'a [f64],
    pub lai: &'a [f64],
    pub total_lai: &'a [f64],
    pub sai: &'a [f64],
    pub total_sai: &'a [f64],
    pub cosine_zenith: &'a [f64],
    pub thermal_gap_fraction: &'a [f64],
    pub direct_extinction: &'a [f64],
    pub diffuse_extinction: &'a [f64],
    pub water_table_depth_m: &'a [f64],
    pub aquifer_water_mm: &'a [f64],
    pub wetland_water_mm: &'a [f64],
    pub surface_water_mm: &'a [f64],
    pub soil_surface_resistance_s_m: &'a [f64],
    pub saved_tke: &'a [f64],
    pub radiative_temperature_k: &'a [f64],
    pub reference_temperature_k: &'a [f64],
    pub reference_humidity: &'a [f64],
    pub stomatal_resistance_s_m: &'a [f64],
    pub emissivity: &'a [f64],
    pub roughness_length_m: &'a [f64],
    pub monin_obukhov_height: &'a [f64],
    pub bulk_richardson: &'a [f64],
    pub friction_velocity: &'a [f64],
    pub humidity_scale: &'a [f64],
    pub temperature_scale_k: &'a [f64],
    pub momentum_integral: &'a [f64],
    pub heat_integral: &'a [f64],
    pub moisture_integral: &'a [f64],
}

/// Radiative restart arrays, axis-major with patch as the final axis.
#[derive(Debug, Clone, Copy)]
pub struct TimeRadiationFields<'a> {
    /// Each is `band * radiation_type * patches`.
    pub albedo: &'a [f64],
    pub sunlit_absorption: &'a [f64],
    pub shaded_absorption: &'a [f64],
    pub soil_absorption: &'a [f64],
    pub snow_absorption: &'a [f64],
    /// `band * radiation_type * snowp1 * patches`.
    pub snow_layer_absorption: &'a [f64],
}

/// Lake fields, axis-major as `lake * patches`.
#[derive(Debug, Clone, Copy)]
pub struct TimeLakeFields<'a> {
    pub temperature_k: &'a [f64],
    pub ice_fraction: &'a [f64],
    /// Present only with `DEF_USE_Dynamic_Lake`.
    pub layer_thickness_m: Option<&'a [f64]>,
}

/// Snow optical and aerosol fields, each `snow * patches`.
#[derive(Debug, Clone, Copy)]
pub struct SnowAerosolFields<'a> {
    pub grain_radius: &'a [f64],
    pub black_carbon_hydrophobic: &'a [f64],
    pub black_carbon_hydrophilic: &'a [f64],
    pub organic_carbon_hydrophobic: &'a [f64],
    pub organic_carbon_hydrophilic: &'a [f64],
    pub dust_1: &'a [f64],
    pub dust_2: &'a [f64],
    pub dust_3: &'a [f64],
    pub dust_4: &'a [f64],
}

/// `DEF_USE_PLANTHYDRAULICS` restart state.
#[derive(Debug, Clone, Copy)]
pub struct PlantHydraulicFields<'a> {
    /// `vegetation_nodes * patches`.
    pub water_potential_mm: &'a [f64],
    pub sunlit_stomatal_conductance: &'a [f64],
    pub shaded_stomatal_conductance: &'a [f64],
    pub vegetation_nodes: usize,
}

/// `DEF_USE_OZONESTRESS` restart state.
#[derive(Debug, Clone, Copy)]
pub struct OzoneFields<'a> {
    pub lai_old: &'a [f64],
    pub sunlit_uptake: &'a [f64],
    pub shaded_uptake: &'a [f64],
    pub sunlit_vegetation_coefficient: &'a [f64],
    pub shaded_vegetation_coefficient: &'a [f64],
    pub sunlit_ground_coefficient: &'a [f64],
    pub shaded_ground_coefficient: &'a [f64],
}

/// `DEF_USE_IRRIGATION` restart state.
#[derive(Debug, Clone, Copy)]
pub struct IrrigationFields<'a> {
    pub rate: &'a [f64],
    pub cumulative: &'a [f64],
    pub cumulative_deficit: &'a [f64],
    pub event_count: &'a [f64],
    pub steps_left: &'a [i32],
    pub water_storage: &'a [f64],
    pub corn_method: &'a [i32],
    pub spring_wheat_method: &'a [i32],
    pub winter_wheat_method: &'a [i32],
    pub soybean_method: &'a [i32],
    pub cotton_method: &'a [i32],
    pub rice_1_method: &'a [i32],
    pub rice_2_method: &'a [i32],
    pub sugarcane_method: &'a [i32],
    pub groundwater_allocation: &'a [f64],
    pub surface_water_allocation: &'a [f64],
    pub standard_water_table_depth: &'a [f64],
}

/// All fields in one common (non-PFT/BGC/urban/tracer) time restart block.
#[derive(Debug, Clone, Copy)]
pub struct TimeRestartInput<'a> {
    pub dimensions: TimeRestartDimensions,
    pub snow_soil: SnowSoilRestartFields<'a>,
    pub patch: TimePatchFields<'a>,
    pub radiation: TimeRadiationFields<'a>,
    pub lake: TimeLakeFields<'a>,
    pub snow_aerosol: SnowAerosolFields<'a>,
    pub plant_hydraulics: Option<PlantHydraulicFields<'a>>,
    pub ozone: Option<OzoneFields<'a>>,
    pub irrigation: Option<IrrigationFields<'a>>,
}

/// CoLM's year, Julian day, and seconds-of-day restart timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartDate {
    pub year: i32,
    pub julian_day: u16,
    pub seconds: u32,
}

/// The single vector-block time restart produced by [`write_time_restart`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRestartFile {
    pub directory: PathBuf,
    pub block: PathBuf,
}

/// Writes a standard CoLM time restart under `<restart>/<yyyy-jjj-sssss>/`.
pub fn write_time_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    land_cover_year: i32,
    date: RestartDate,
    block_label: &str,
    input: TimeRestartInput<'_>,
) -> Result<TimeRestartFile> {
    validate_component(case_name, "case name")?;
    validate_component(block_label, "block label")?;
    ensure!(
        (0..=9999).contains(&land_cover_year) && (0..=9999).contains(&date.year),
        "restart years must fit CoLM's four-digit filename convention"
    );
    ensure!(
        (1..=366).contains(&date.julian_day) && date.seconds < 86_400,
        "restart date is not a valid Julian day and seconds-of-day"
    );
    let date_label = format!(
        "{:04}-{:03}-{:05}",
        date.year, date.julian_day, date.seconds
    );
    let directory = restart_dir.as_ref().join(&date_label);
    let block = directory.join(format!(
        "{case_name}_restart_{date_label}_lc{land_cover_year:04}_{block_label}.nc"
    ));
    write_time_restart_block(&block, input)?;
    Ok(TimeRestartFile { directory, block })
}

/// Writes one already-addressed common CoLM time restart block.
pub fn write_time_restart_block(path: impl AsRef<Path>, input: TimeRestartInput<'_>) -> Result<()> {
    let patches = validate_input(input)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create time restart block {}", path.display()))?;
    define_dimensions(&mut file, patches, input.dimensions, input.plant_hydraulics)?;
    let dimensions = input.dimensions;
    let soilsnow = dimensions.soil_layers + dimensions.snow_layers;
    let snowp1 = dimensions.snow_layers + 1;

    put_axis_major(
        &mut file,
        "z_sno",
        "snow",
        dimensions.snow_layers,
        patches,
        input.snow_soil.snow_node_depth_m,
    )?;
    put_axis_major(
        &mut file,
        "dz_sno",
        "snow",
        dimensions.snow_layers,
        patches,
        input.snow_soil.snow_layer_thickness_m,
    )?;
    for (name, values) in [
        ("t_soisno", input.snow_soil.temperature_k),
        ("wliq_soisno", input.snow_soil.liquid_water_kg_m2),
        ("wice_soisno", input.snow_soil.ice_water_kg_m2),
    ] {
        put_axis_major(&mut file, name, "soilsnow", soilsnow, patches, values)?;
    }
    put_axis_major(
        &mut file,
        "smp",
        "soil",
        dimensions.soil_layers,
        patches,
        input.snow_soil.matric_potential_mm,
    )?;
    put_axis_major(
        &mut file,
        "hk",
        "soil",
        dimensions.soil_layers,
        patches,
        input.snow_soil.hydraulic_conductivity_mm_s,
    )?;

    if let Some(plant) = input.plant_hydraulics {
        put_axis_major(
            &mut file,
            "vegwp",
            "vegnodes",
            plant.vegetation_nodes,
            patches,
            plant.water_potential_mm,
        )?;
        put_patch_values(
            &mut file,
            [
                ("gs0sun", plant.sunlit_stomatal_conductance),
                ("gs0sha", plant.shaded_stomatal_conductance),
            ],
        )?;
    }
    if let Some(ozone) = input.ozone {
        put_patch_values(
            &mut file,
            [
                ("lai_old", ozone.lai_old),
                ("o3uptakesun", ozone.sunlit_uptake),
                ("o3uptakesha", ozone.shaded_uptake),
                ("o3coefv_sun", ozone.sunlit_vegetation_coefficient),
                ("o3coefv_sha", ozone.shaded_vegetation_coefficient),
                ("o3coefg_sun", ozone.sunlit_ground_coefficient),
                ("o3coefg_sha", ozone.shaded_ground_coefficient),
            ],
        )?;
    }

    put_patch_values(&mut file, pre_radiation_patch_entries(input.patch))?;
    for (name, values) in [
        ("alb", input.radiation.albedo),
        ("ssun", input.radiation.sunlit_absorption),
        ("ssha", input.radiation.shaded_absorption),
        ("ssoi", input.radiation.soil_absorption),
        ("ssno", input.radiation.snow_absorption),
    ] {
        put_patch_last_3d(
            &mut file,
            name,
            ("band", dimensions.bands),
            ("rtyp", dimensions.radiation_types),
            patches,
            values,
        )?;
    }
    put_patch_values(&mut file, post_radiation_patch_entries(input.patch))?;
    put_axis_major(
        &mut file,
        "t_lake",
        "lake",
        dimensions.lake_layers,
        patches,
        input.lake.temperature_k,
    )?;
    put_axis_major(
        &mut file,
        "lake_icefrc",
        "lake",
        dimensions.lake_layers,
        patches,
        input.lake.ice_fraction,
    )?;
    if let Some(values) = input.lake.layer_thickness_m {
        put_axis_major(
            &mut file,
            "dz_lake",
            "lake",
            dimensions.lake_layers,
            patches,
            values,
        )?;
    }
    put_patch_values(&mut file, [("savedtke1", input.patch.saved_tke)])?;
    for (name, values) in snow_aerosol_entries(input.snow_aerosol) {
        put_axis_major(
            &mut file,
            name,
            "snow",
            dimensions.snow_layers,
            patches,
            values,
        )?;
    }
    put_patch_last_4d(
        &mut file,
        "ssno_lyr",
        ("band", dimensions.bands),
        ("rtyp", dimensions.radiation_types),
        ("snowp1", snowp1),
        patches,
        input.radiation.snow_layer_absorption,
    )?;
    put_patch_values(&mut file, regional_patch_entries(input.patch))?;
    if let Some(irrigation) = input.irrigation {
        put_patch_values(&mut file, irrigation_f64_entries(irrigation))?;
        put_patch_i32_values(&mut file, irrigation_i32_entries(irrigation))?;
    }
    file.close()
        .with_context(|| format!("cannot close time restart block {}", path.display()))?;
    Ok(())
}

fn patch_entries(fields: TimePatchFields<'_>) -> [(&str, &[f64]); 41] {
    [
        ("t_grnd", fields.ground_temperature_k),
        ("tleaf", fields.leaf_temperature_k),
        ("ldew", fields.canopy_water_mm),
        ("ldew_rain", fields.canopy_rain_mm),
        ("ldew_snow", fields.canopy_snow_mm),
        ("fwet_snow", fields.wet_snow_fraction),
        ("sag", fields.snow_age),
        ("scv", fields.snow_water_equivalent_mm),
        ("snowdp", fields.snow_depth_m),
        ("fveg", fields.vegetation_fraction),
        ("fsno", fields.ground_snow_fraction),
        ("sigf", fields.snow_free_vegetation_fraction),
        ("green", fields.greenness),
        ("lai", fields.lai),
        ("tlai", fields.total_lai),
        ("sai", fields.sai),
        ("tsai", fields.total_sai),
        ("coszen", fields.cosine_zenith),
        ("thermk", fields.thermal_gap_fraction),
        ("extkb", fields.direct_extinction),
        ("extkd", fields.diffuse_extinction),
        ("zwt", fields.water_table_depth_m),
        ("wa", fields.aquifer_water_mm),
        ("wetwat", fields.wetland_water_mm),
        ("wdsrf", fields.surface_water_mm),
        ("rss", fields.soil_surface_resistance_s_m),
        ("savedtke1", fields.saved_tke),
        ("trad", fields.radiative_temperature_k),
        ("tref", fields.reference_temperature_k),
        ("qref", fields.reference_humidity),
        ("rst", fields.stomatal_resistance_s_m),
        ("emis", fields.emissivity),
        ("z0m", fields.roughness_length_m),
        ("zol", fields.monin_obukhov_height),
        ("rib", fields.bulk_richardson),
        ("ustar", fields.friction_velocity),
        ("qstar", fields.humidity_scale),
        ("tstar", fields.temperature_scale_k),
        ("fm", fields.momentum_integral),
        ("fh", fields.heat_integral),
        ("fq", fields.moisture_integral),
    ]
}

fn pre_radiation_patch_entries(fields: TimePatchFields<'_>) -> [(&str, &[f64]); 18] {
    [
        ("t_grnd", fields.ground_temperature_k),
        ("tleaf", fields.leaf_temperature_k),
        ("ldew", fields.canopy_water_mm),
        ("ldew_rain", fields.canopy_rain_mm),
        ("ldew_snow", fields.canopy_snow_mm),
        ("fwet_snow", fields.wet_snow_fraction),
        ("sag", fields.snow_age),
        ("scv", fields.snow_water_equivalent_mm),
        ("snowdp", fields.snow_depth_m),
        ("fveg", fields.vegetation_fraction),
        ("fsno", fields.ground_snow_fraction),
        ("sigf", fields.snow_free_vegetation_fraction),
        ("green", fields.greenness),
        ("lai", fields.lai),
        ("tlai", fields.total_lai),
        ("sai", fields.sai),
        ("tsai", fields.total_sai),
        ("coszen", fields.cosine_zenith),
    ]
}

fn post_radiation_patch_entries(fields: TimePatchFields<'_>) -> [(&str, &[f64]); 8] {
    [
        ("thermk", fields.thermal_gap_fraction),
        ("extkb", fields.direct_extinction),
        ("extkd", fields.diffuse_extinction),
        ("zwt", fields.water_table_depth_m),
        ("wa", fields.aquifer_water_mm),
        ("wetwat", fields.wetland_water_mm),
        ("wdsrf", fields.surface_water_mm),
        ("rss", fields.soil_surface_resistance_s_m),
    ]
}

fn regional_patch_entries(fields: TimePatchFields<'_>) -> [(&str, &[f64]); 14] {
    [
        ("trad", fields.radiative_temperature_k),
        ("tref", fields.reference_temperature_k),
        ("qref", fields.reference_humidity),
        ("rst", fields.stomatal_resistance_s_m),
        ("emis", fields.emissivity),
        ("z0m", fields.roughness_length_m),
        ("zol", fields.monin_obukhov_height),
        ("rib", fields.bulk_richardson),
        ("ustar", fields.friction_velocity),
        ("qstar", fields.humidity_scale),
        ("tstar", fields.temperature_scale_k),
        ("fm", fields.momentum_integral),
        ("fh", fields.heat_integral),
        ("fq", fields.moisture_integral),
    ]
}

fn snow_aerosol_entries(fields: SnowAerosolFields<'_>) -> [(&str, &[f64]); 9] {
    [
        ("snw_rds", fields.grain_radius),
        ("mss_bcpho", fields.black_carbon_hydrophobic),
        ("mss_bcphi", fields.black_carbon_hydrophilic),
        ("mss_ocpho", fields.organic_carbon_hydrophobic),
        ("mss_ocphi", fields.organic_carbon_hydrophilic),
        ("mss_dst1", fields.dust_1),
        ("mss_dst2", fields.dust_2),
        ("mss_dst3", fields.dust_3),
        ("mss_dst4", fields.dust_4),
    ]
}

fn irrigation_f64_entries(fields: IrrigationFields<'_>) -> [(&str, &[f64]); 8] {
    [
        ("irrig_rate", fields.rate),
        ("sum_irrig", fields.cumulative),
        ("sum_deficit_irrig", fields.cumulative_deficit),
        ("sum_irrig_count", fields.event_count),
        ("waterstorage", fields.water_storage),
        ("irrig_gw_alloc", fields.groundwater_allocation),
        ("irrig_sw_alloc", fields.surface_water_allocation),
        ("zwt_stand", fields.standard_water_table_depth),
    ]
}

fn irrigation_i32_entries(fields: IrrigationFields<'_>) -> [(&str, &[i32]); 9] {
    [
        ("n_irrig_steps_left", fields.steps_left),
        ("irrig_method_corn", fields.corn_method),
        ("irrig_method_swheat", fields.spring_wheat_method),
        ("irrig_method_wwheat", fields.winter_wheat_method),
        ("irrig_method_soybean", fields.soybean_method),
        ("irrig_method_cotton", fields.cotton_method),
        ("irrig_method_rice1", fields.rice_1_method),
        ("irrig_method_rice2", fields.rice_2_method),
        ("irrig_method_sugarcane", fields.sugarcane_method),
    ]
}

fn validate_input(input: TimeRestartInput<'_>) -> Result<usize> {
    let dimensions = input.dimensions;
    for (name, size) in [
        ("soil", dimensions.soil_layers),
        ("lake", dimensions.lake_layers),
        ("snow", dimensions.snow_layers),
        ("band", dimensions.bands),
        ("rtyp", dimensions.radiation_types),
    ] {
        ensure!(size > 0, "time restart dimension {name} must be positive");
    }
    let patches = input.patch.ground_temperature_k.len();
    ensure!(patches > 0, "a time restart block needs at least one patch");
    validate_patch_values("time restart", patches, &patch_entries(input.patch))?;
    let snow = dimensions.snow_layers;
    let soilsnow = dimensions.soil_layers + snow;
    for (name, values, layers) in [
        ("z_sno", input.snow_soil.snow_node_depth_m, snow),
        ("dz_sno", input.snow_soil.snow_layer_thickness_m, snow),
        ("t_soisno", input.snow_soil.temperature_k, soilsnow),
        ("wliq_soisno", input.snow_soil.liquid_water_kg_m2, soilsnow),
        ("wice_soisno", input.snow_soil.ice_water_kg_m2, soilsnow),
        (
            "smp",
            input.snow_soil.matric_potential_mm,
            dimensions.soil_layers,
        ),
        (
            "hk",
            input.snow_soil.hydraulic_conductivity_mm_s,
            dimensions.soil_layers,
        ),
        ("t_lake", input.lake.temperature_k, dimensions.lake_layers),
        (
            "lake_icefrc",
            input.lake.ice_fraction,
            dimensions.lake_layers,
        ),
    ] {
        validate_axis_major(name, values, layers, patches)?;
    }
    if let Some(values) = input.lake.layer_thickness_m {
        validate_axis_major("dz_lake", values, dimensions.lake_layers, patches)?;
    }
    for (name, values) in snow_aerosol_entries(input.snow_aerosol) {
        validate_axis_major(name, values, snow, patches)?;
    }
    for (name, values) in [
        ("alb", input.radiation.albedo),
        ("ssun", input.radiation.sunlit_absorption),
        ("ssha", input.radiation.shaded_absorption),
        ("ssoi", input.radiation.soil_absorption),
        ("ssno", input.radiation.snow_absorption),
    ] {
        validate_patch_last_3d(
            name,
            values,
            dimensions.bands,
            dimensions.radiation_types,
            patches,
        )?;
    }
    validate_patch_last_4d(
        "ssno_lyr",
        input.radiation.snow_layer_absorption,
        dimensions.bands,
        dimensions.radiation_types,
        snow + 1,
        patches,
    )?;
    if let Some(plant) = input.plant_hydraulics {
        ensure!(plant.vegetation_nodes > 0, "vegnodes must be positive");
        validate_axis_major(
            "vegwp",
            plant.water_potential_mm,
            plant.vegetation_nodes,
            patches,
        )?;
        validate_patch_values(
            "plant hydraulics",
            patches,
            &[
                ("gs0sun", plant.sunlit_stomatal_conductance),
                ("gs0sha", plant.shaded_stomatal_conductance),
            ],
        )?;
    }
    if let Some(ozone) = input.ozone {
        validate_patch_values(
            "ozone",
            patches,
            &[
                ("lai_old", ozone.lai_old),
                ("o3uptakesun", ozone.sunlit_uptake),
                ("o3uptakesha", ozone.shaded_uptake),
                ("o3coefv_sun", ozone.sunlit_vegetation_coefficient),
                ("o3coefv_sha", ozone.shaded_vegetation_coefficient),
                ("o3coefg_sun", ozone.sunlit_ground_coefficient),
                ("o3coefg_sha", ozone.shaded_ground_coefficient),
            ],
        )?;
    }
    if let Some(irrigation) = input.irrigation {
        validate_patch_values("irrigation", patches, &irrigation_f64_entries(irrigation))?;
        validate_i32_patch_values("irrigation", patches, &irrigation_i32_entries(irrigation))?;
    }
    Ok(patches)
}

fn validate_component(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')),
        "{name} must be a nonempty filename component"
    );
    Ok(())
}

fn validate_patch_values(name: &str, patches: usize, fields: &[(&str, &[f64])]) -> Result<()> {
    if let Some((field, values)) = fields.iter().find(|(_, values)| values.len() != patches) {
        anyhow::bail!(
            "{name} field {field} has {} entries; expected {patches}",
            values.len()
        );
    }
    Ok(())
}

fn validate_i32_patch_values(name: &str, patches: usize, fields: &[(&str, &[i32])]) -> Result<()> {
    if let Some((field, values)) = fields.iter().find(|(_, values)| values.len() != patches) {
        anyhow::bail!(
            "{name} field {field} has {} entries; expected {patches}",
            values.len()
        );
    }
    Ok(())
}

fn validate_axis_major(name: &str, values: &[f64], axis: usize, patches: usize) -> Result<()> {
    ensure!(
        values.len() == axis * patches,
        "{name} has {} entries; expected {axis} x {patches}",
        values.len()
    );
    Ok(())
}

fn validate_patch_last_3d(
    name: &str,
    values: &[f64],
    first: usize,
    second: usize,
    patches: usize,
) -> Result<()> {
    ensure!(
        values.len() == first * second * patches,
        "{name} has {} entries; expected {first} x {second} x {patches}",
        values.len()
    );
    Ok(())
}

fn validate_patch_last_4d(
    name: &str,
    values: &[f64],
    first: usize,
    second: usize,
    third: usize,
    patches: usize,
) -> Result<()> {
    ensure!(
        values.len() == first * second * third * patches,
        "{name} has {} entries; expected {first} x {second} x {third} x {patches}",
        values.len()
    );
    Ok(())
}

fn define_dimensions(
    file: &mut netcdf::FileMut,
    patches: usize,
    dimensions: TimeRestartDimensions,
    plant: Option<PlantHydraulicFields<'_>>,
) -> Result<()> {
    for (name, length) in [
        ("patch", patches),
        ("snow", dimensions.snow_layers),
        ("snowp1", dimensions.snow_layers + 1),
        ("soilsnow", dimensions.soil_layers + dimensions.snow_layers),
        ("soil", dimensions.soil_layers),
        ("lake", dimensions.lake_layers),
    ] {
        file.add_dimension(name, length)?;
    }
    if let Some(plant) = plant {
        file.add_dimension("vegnodes", plant.vegetation_nodes)?;
    }
    for (name, length) in [
        ("band", dimensions.bands),
        ("rtyp", dimensions.radiation_types),
    ] {
        file.add_dimension(name, length)?;
    }
    Ok(())
}

fn put_patch_values<const N: usize>(
    file: &mut netcdf::FileMut,
    entries: [(&str, &[f64]); N],
) -> Result<()> {
    for (name, values) in entries {
        file.add_variable::<f64>(name, &["patch"])?
            .put_values(values, ..)?;
    }
    Ok(())
}

fn put_patch_i32_values<const N: usize>(
    file: &mut netcdf::FileMut,
    entries: [(&str, &[i32]); N],
) -> Result<()> {
    for (name, values) in entries {
        file.add_variable::<i32>(name, &["patch"])?
            .put_values(values, ..)?;
    }
    Ok(())
}

fn put_axis_major(
    file: &mut netcdf::FileMut,
    name: &str,
    axis_name: &str,
    axis: usize,
    patches: usize,
    values: &[f64],
) -> Result<()> {
    validate_axis_major(name, values, axis, patches)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for axis_index in 0..axis {
            on_disk.push(values[axis_index * patches + patch]);
        }
    }
    file.add_variable::<f64>(name, &["patch", axis_name])?
        .put_values(&on_disk, (.., ..))?;
    Ok(())
}

fn put_patch_last_3d(
    file: &mut netcdf::FileMut,
    name: &str,
    (first_name, first): (&str, usize),
    (second_name, second): (&str, usize),
    patches: usize,
    values: &[f64],
) -> Result<()> {
    validate_patch_last_3d(name, values, first, second, patches)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for second_index in 0..second {
            for first_index in 0..first {
                on_disk.push(values[(first_index * second + second_index) * patches + patch]);
            }
        }
    }
    file.add_variable::<f64>(name, &["patch", second_name, first_name])?
        .put_values(&on_disk, (.., .., ..))?;
    Ok(())
}

fn put_patch_last_4d(
    file: &mut netcdf::FileMut,
    name: &str,
    (first_name, first): (&str, usize),
    (second_name, second): (&str, usize),
    (third_name, third): (&str, usize),
    patches: usize,
    values: &[f64],
) -> Result<()> {
    validate_patch_last_4d(name, values, first, second, third, patches)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for third_index in 0..third {
            for second_index in 0..second {
                for first_index in 0..first {
                    on_disk.push(
                        values[((first_index * second + second_index) * third + third_index)
                            * patches
                            + patch],
                    );
                }
            }
        }
    }
    file.add_variable::<f64>(name, &["patch", third_name, second_name, first_name])?
        .put_values(&on_disk, (.., .., .., ..))?;
    Ok(())
}

#[cfg(test)]
#[path = "time_restart_tests.rs"]
mod time_restart_tests;
