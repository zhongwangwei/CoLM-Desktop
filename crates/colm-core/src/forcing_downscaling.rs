//! Terrain-aware atmospheric forcing downscaling shared by all Rust drivers.
//!
//! This is a direct numerical port of `MOD_ForcingDownscaling.F90`.  It has no
//! namelist or NetCDF dependency: drivers map their configured values into
//! [`ForcingDownscalingConfig`] and use the same kernel for every column.

use anyhow::{ensure, Result};

use crate::{saturation_specific_humidity, RuntimeForcing};

/// Number of terrain classes used by CoLM's full shortwave/wind scheme.
pub const SLOPE_TYPES: usize = 4;
/// Number of terrain aspect classes used by CoLM's simple scheme.
pub const ASPECT_TYPES: usize = 9;
/// Number of azimuth bins in CoLM's full-mode shadow tables.
pub const AZIMUTH_BINS: usize = 16;
/// Number of zenith bins in CoLM's single-point shadow table.
pub const ZENITH_BINS: usize = 101;
/// Number of parameters per spatial-mode shadow curve.
pub const SHADOW_CURVE_PARAMETERS: usize = 3;

const SOLAR_CONSTANT_W_M2: f64 = 1370.0;
const GRAVITY_M_S2: f64 = 9.80616;
const DRY_AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;
const WATER_VAPOR_MOLECULAR_WEIGHT: f64 = 18.016;
const DRY_AIR_MOLECULAR_WEIGHT: f64 = 28.966;
const AVOGADRO_PER_KMOLE: f64 = 6.02214e26;
const BOLTZMANN_J_K: f64 = 1.38065e-23;
const STEFAN_BOLTZMANN_W_M2_K4: f64 = 5.67e-8;
const MISSING: f64 = -1.0e36;
// `MOD_Vars_Global:PI = 4 * atan(1.)`.
const F77_PI: f64 = std::f64::consts::PI;

/// CoLM's `DEF_DS_longwave_adjust_scheme` choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LongwaveDownscaling {
    /// Scheme I: Fiddes and Gruber clear-sky emissivity adjustment.
    ClearSky,
    /// Scheme II: elevation lapse-rate adjustment (the upstream default).
    LapseRate,
}

/// CoLM's `DEF_DS_precipitation_adjust_scheme` choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrecipitationDownscaling {
    /// Scheme I: elevation fraction relative to the grid maximum elevation.
    ElevationFraction,
    /// Scheme II: Liston and Elder's fixed 0.27/km adjustment.
    ListonElder,
}

/// Numerical runtime settings corresponding to CoLM's downscaling namelist.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcingDownscalingConfig {
    pub temperature_lapse_rate_k_m: f64,
    pub glacier_longwave_lapse_rate_w_m2_m: f64,
    pub longwave_limit: f64,
    pub full_shortwave_limit: f64,
    pub simple_shortwave_limit: f64,
    pub longwave: LongwaveDownscaling,
    pub precipitation: PrecipitationDownscaling,
}

impl Default for ForcingDownscalingConfig {
    fn default() -> Self {
        Self {
            temperature_lapse_rate_k_m: 0.006,
            glacier_longwave_lapse_rate_w_m2_m: 0.032,
            longwave_limit: 0.5,
            full_shortwave_limit: 0.5,
            simple_shortwave_limit: 0.2,
            longwave: LongwaveDownscaling::LapseRate,
            precipitation: PrecipitationDownscaling::ElevationFraction,
        }
    }
}

/// Grid-level atmospheric forcing before elevation-aware adjustment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridForcing {
    pub surface_elevation_m: f64,
    pub maximum_elevation_m: f64,
    pub air_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub specific_humidity: f64,
    pub bottom_pressure_pa: f64,
    pub density_kg_m3: f64,
    pub convective_precipitation_kg_m2_s: f64,
    pub large_scale_precipitation_kg_m2_s: f64,
    pub downward_longwave_w_m2: f64,
    pub reference_height_m: f64,
    pub downward_shortwave_w_m2: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
}

/// Solar quantities prepared once by `MOD_Forcing` for a column time step.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownscalingSolarGeometry {
    /// CoLM orbital calendar day (`julian_day` in the Fortran routine).
    pub calendar_day: f64,
    pub cosine_zenith: f64,
    pub cosine_azimuth: f64,
}

/// Single-point / grid-mode shadow data used by the full terrain scheme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ShadowMask<'a> {
    /// `sf_lut_c(1:num_azimuth, 1:num_zenith)`.
    Lookup(&'a [[f64; ZENITH_BINS]; AZIMUTH_BINS]),
    /// `sf_curve_c(1:num_azimuth, 1:num_zenith_parameter)`.
    Curve(&'a [[f64; SHADOW_CURVE_PARAMETERS]; AZIMUTH_BINS]),
}

/// Full (slope/aspect, sky-view and shadow-aware) terrain characterization.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FullTerrain<'a> {
    /// Slope angles in radians, one for each of CoLM's four slope classes.
    pub slope_radians: &'a [f64; SLOPE_TYPES],
    /// Aspect angles in radians, one for each of CoLM's four slope classes.
    pub aspect_radians: &'a [f64; SLOPE_TYPES],
    /// Fractional area of each slope class.
    pub area_fraction: &'a [f64; SLOPE_TYPES],
    pub sky_view_factor: f64,
    /// A NaN preserves CoLM's missing-albedo branch: reflected shortwave is omitted.
    pub blue_sky_albedo: f64,
    pub shadow: ShadowMask<'a>,
}

/// Simple nine-aspect terrain characterization used by coupled grid runs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SimpleTerrain<'a> {
    /// Tangent of slope for N, NE, E, SE, S, SW, W, NW, and flat terrain.
    pub slope_tangent: &'a [f64; ASPECT_TYPES],
    /// Fractional area of those same terrain classes.
    pub area_fraction: &'a [f64; ASPECT_TYPES],
}

/// The terrain representation selected by the caller's spatial configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DownscalingTerrain<'a> {
    Full(FullTerrain<'a>),
    Simple(SimpleTerrain<'a>),
}

/// Inputs to one column's exact CoLM forcing-downscaling calculation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ForcingDownscalingInput<'a> {
    pub glacier: bool,
    pub grid: GridForcing,
    pub column_surface_elevation_m: f64,
    pub solar: DownscalingSolarGeometry,
    pub terrain: DownscalingTerrain<'a>,
}

/// Column atmospheric forcing after CoLM's terrain-aware adjustment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DownscaledForcing {
    pub air_temperature_k: f64,
    pub potential_temperature_k: f64,
    pub specific_humidity: f64,
    pub bottom_pressure_pa: f64,
    pub density_kg_m3: f64,
    pub convective_precipitation_kg_m2_s: f64,
    pub large_scale_precipitation_kg_m2_s: f64,
    pub downward_longwave_w_m2: f64,
    pub downward_shortwave_w_m2: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
}

/// Builds the `MOD_Forcing` grid record consumed by [`downscale_forcings`].
///
/// `RuntimeForcing` is the shared output of every reader.  Keeping this small
/// adapter beside the numerical kernel prevents each driver from recalculating
/// CoLM's potential temperature and grid density before downscaling.
pub fn grid_forcing_from_runtime(
    forcing: RuntimeForcing,
    surface_elevation_m: f64,
    maximum_elevation_m: f64,
    reference_height_m: f64,
) -> Result<GridForcing> {
    ensure!(
        surface_elevation_m.is_finite()
            && maximum_elevation_m.is_finite()
            && reference_height_m.is_finite()
            && reference_height_m >= 0.0,
        "grid forcing geometry is physically invalid"
    );
    let density_kg_m3 = (forcing.bottom_pressure_pa
        - 0.378 * forcing.specific_humidity * forcing.bottom_pressure_pa
            / (0.622 + 0.378 * forcing.specific_humidity))
        / (287.04 * forcing.air_temperature_k);
    Ok(GridForcing {
        surface_elevation_m,
        maximum_elevation_m,
        air_temperature_k: forcing.air_temperature_k,
        potential_temperature_k: forcing.air_temperature_k
            * (100_000.0 / forcing.bottom_pressure_pa)
                .powf(dry_air_gas_constant() / DRY_AIR_HEAT_CAPACITY_J_KG_K),
        specific_humidity: forcing.specific_humidity,
        bottom_pressure_pa: forcing.bottom_pressure_pa,
        density_kg_m3,
        convective_precipitation_kg_m2_s: forcing.convective_precipitation_kg_m2_s,
        large_scale_precipitation_kg_m2_s: forcing.large_scale_precipitation_kg_m2_s,
        downward_longwave_w_m2: forcing.downward_longwave_w_m2,
        reference_height_m,
        downward_shortwave_w_m2: runtime_shortwave_total(forcing),
        eastward_wind_m_s: forcing.eastward_wind_m_s,
        northward_wind_m_s: forcing.northward_wind_m_s,
    })
}

/// Replaces a reader's grid-level fields with one downscaled column record.
///
/// This is the inverse hand-off of [`grid_forcing_from_runtime`]: the runtime
/// keeps its calendar and shortwave geometry while downstream physics sees the
/// adjusted pressure, moisture, radiation, precipitation, and wind values.
pub fn apply_downscaled_runtime_forcing(
    forcing: RuntimeForcing,
    downscaled: DownscaledForcing,
) -> RuntimeForcing {
    RuntimeForcing {
        air_temperature_k: downscaled.air_temperature_k,
        specific_humidity: downscaled.specific_humidity,
        surface_pressure_pa: downscaled.bottom_pressure_pa,
        bottom_pressure_pa: downscaled.bottom_pressure_pa,
        convective_precipitation_kg_m2_s: downscaled.convective_precipitation_kg_m2_s,
        large_scale_precipitation_kg_m2_s: downscaled.large_scale_precipitation_kg_m2_s,
        eastward_wind_m_s: downscaled.eastward_wind_m_s,
        northward_wind_m_s: downscaled.northward_wind_m_s,
        downward_longwave_w_m2: downscaled.downward_longwave_w_m2,
        shortwave: runtime_shortwave_from_total(
            downscaled.downward_shortwave_w_m2,
            forcing.cosine_zenith,
        ),
        cosine_zenith: forcing.cosine_zenith,
    }
}

fn runtime_shortwave_total(forcing: RuntimeForcing) -> f64 {
    forcing.shortwave.direct_visible_w_m2
        + forcing.shortwave.direct_near_infrared_w_m2
        + forcing.shortwave.diffuse_visible_w_m2
        + forcing.shortwave.diffuse_near_infrared_w_m2
}

fn runtime_shortwave_from_total(total_w_m2: f64, cosine_zenith: f64) -> crate::ShortwaveForcing {
    crate::runtime_forcing::split_broadband_shortwave(total_w_m2, cosine_zenith)
}

/// Port of `MOD_ForcingDownscaling:rhos`.
pub fn atmospheric_density(
    specific_humidity: f64,
    pressure_pa: f64,
    temperature_k: f64,
) -> Result<f64> {
    ensure!(
        specific_humidity.is_finite()
            && (0.0..1.0).contains(&specific_humidity)
            && pressure_pa.is_finite()
            && pressure_pa > 0.0
            && temperature_k.is_finite()
            && temperature_k > 0.0,
        "density inputs are physically invalid"
    );
    let water_to_dry_air = WATER_VAPOR_MOLECULAR_WEIGHT / DRY_AIR_MOLECULAR_WEIGHT;
    let vapor_pressure = specific_humidity * pressure_pa
        / (water_to_dry_air + (1.0 - water_to_dry_air) * specific_humidity);
    let dry_air_gas_constant = AVOGADRO_PER_KMOLE * BOLTZMANN_J_K / DRY_AIR_MOLECULAR_WEIGHT;
    Ok((pressure_pa - (1.0 - water_to_dry_air) * vapor_pressure)
        / (dry_air_gas_constant * temperature_k))
}

/// Port of `MOD_ForcingDownscaling:downscale_forcings`.
///
/// This is intentionally the only shared boundary that performs all state,
/// radiation, and precipitation adjustments.  `colm-init` and the runtime can
/// call it without reimplementing any part of the `colm.x` forcing path.
pub fn downscale_forcings(
    input: ForcingDownscalingInput<'_>,
    config: ForcingDownscalingConfig,
) -> Result<DownscaledForcing> {
    validate_input(input, config)?;
    let grid = input.grid;
    let elevation_difference = input.column_surface_elevation_m - grid.surface_elevation_m;
    let air_temperature_k =
        grid.air_temperature_k - config.temperature_lapse_rate_k_m * elevation_difference;
    let scale_height_m =
        dry_air_gas_constant() * 0.5 * (grid.air_temperature_k + air_temperature_k) / GRAVITY_M_S2;
    let bottom_pressure_pa =
        grid.bottom_pressure_pa * (-elevation_difference / scale_height_m).exp();
    let potential_temperature_k = grid.potential_temperature_k
        + (air_temperature_k - grid.air_temperature_k)
            * ((grid.reference_height_m / scale_height_m)
                * (dry_air_gas_constant() / DRY_AIR_HEAT_CAPACITY_J_KG_K))
                .exp();
    let specific_humidity = grid.specific_humidity
        * (saturation_specific_humidity(air_temperature_k, bottom_pressure_pa)?.specific_humidity
            / saturation_specific_humidity(grid.air_temperature_k, grid.bottom_pressure_pa)?
                .specific_humidity);
    let density_kg_m3 = grid.density_kg_m3
        * (atmospheric_density(specific_humidity, bottom_pressure_pa, air_temperature_k)?
            / atmospheric_density(
                grid.specific_humidity,
                grid.bottom_pressure_pa,
                grid.air_temperature_k,
            )?);
    let downward_longwave_w_m2 = downscale_longwave(
        input.glacier,
        grid,
        input.column_surface_elevation_m,
        air_temperature_k,
        specific_humidity,
        bottom_pressure_pa,
        config,
    )?;
    let downward_shortwave_w_m2 = match input.terrain {
        DownscalingTerrain::Full(terrain) => downscale_shortwave(
            grid,
            bottom_pressure_pa,
            input.solar,
            terrain,
            config.full_shortwave_limit,
        )?,
        DownscalingTerrain::Simple(terrain) => downscale_shortwave_simple(
            grid,
            bottom_pressure_pa,
            input.solar,
            terrain,
            config.simple_shortwave_limit,
        )?,
    };
    let (convective_precipitation_kg_m2_s, large_scale_precipitation_kg_m2_s) =
        downscale_precipitation(grid, input.column_surface_elevation_m, config.precipitation);
    Ok(DownscaledForcing {
        air_temperature_k,
        potential_temperature_k,
        specific_humidity,
        bottom_pressure_pa,
        density_kg_m3,
        convective_precipitation_kg_m2_s,
        large_scale_precipitation_kg_m2_s,
        downward_longwave_w_m2,
        downward_shortwave_w_m2,
        eastward_wind_m_s: grid.eastward_wind_m_s,
        northward_wind_m_s: grid.northward_wind_m_s,
    })
}

/// Port of `MOD_ForcingDownscaling:downscale_wind`.
pub fn downscale_wind(
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    slope_radians: &[f64; SLOPE_TYPES],
    aspect_radians: &[f64; SLOPE_TYPES],
    area_fraction: &[f64; SLOPE_TYPES],
    curvature: f64,
) -> Result<(f64, f64)> {
    validate_finite(
        slope_radians
            .iter()
            .chain(aspect_radians)
            .chain(area_fraction)
            .copied()
            .chain([eastward_wind_m_s, northward_wind_m_s, curvature]),
        "full wind-downscaling inputs",
    )?;
    let wind_direction = if eastward_wind_m_s == 0.0 {
        F77_PI / 2.0
    } else {
        (northward_wind_m_s / eastward_wind_m_s).atan()
    };
    let grid_speed = eastward_wind_m_s.hypot(northward_wind_m_s);
    let column_speed = slope_radians
        .iter()
        .zip(aspect_radians)
        .zip(area_fraction)
        .map(|((&slope, &aspect), &area)| {
            let factor = (1.0 + 0.58 * slope * (wind_direction - aspect).cos() + 0.42 * curvature)
                .clamp(-1.5, 1.5);
            grid_speed * factor * area
        })
        .sum::<f64>();
    Ok((
        column_speed * wind_direction.cos(),
        column_speed * wind_direction.sin(),
    ))
}

/// Port of `MOD_ForcingDownscaling:downscale_wind_simple`.
pub fn downscale_wind_simple(
    eastward_wind_m_s: f64,
    northward_wind_m_s: f64,
    slope_tangent: &[f64; ASPECT_TYPES],
    area_fraction: &[f64; ASPECT_TYPES],
    curvature: f64,
) -> Result<(f64, f64)> {
    validate_finite(
        slope_tangent.iter().chain(area_fraction).copied().chain([
            eastward_wind_m_s,
            northward_wind_m_s,
            curvature,
        ]),
        "simple wind-downscaling inputs",
    )?;
    let mut wind_direction = if eastward_wind_m_s == 0.0 {
        F77_PI / 2.0
    } else {
        northward_wind_m_s.atan2(eastward_wind_m_s)
    };
    wind_direction = compass_direction(wind_direction);
    let grid_speed = eastward_wind_m_s.hypot(northward_wind_m_s);
    let column_speed = slope_tangent
        .iter()
        .zip(area_fraction)
        .enumerate()
        .filter_map(|(index, (&slope, &area))| {
            let factor = if index == ASPECT_TYPES - 1 {
                1.0
            } else if slope == MISSING || curvature == MISSING {
                MISSING
            } else {
                1.0 + 0.58 * slope.atan() * (wind_direction - simple_aspect(index)).cos()
                    + 0.42 * curvature
            }
            .clamp(-1.5, 1.5);
            if factor == MISSING || area == MISSING {
                None
            } else {
                Some(grid_speed * factor * area)
            }
        })
        .sum::<f64>();
    wind_direction = compass_direction(wind_direction);
    let eastward_sign = if eastward_wind_m_s >= 0.0 { 1.0 } else { -1.0 };
    let northward_sign = if northward_wind_m_s >= 0.0 { 1.0 } else { -1.0 };
    Ok((
        eastward_sign * (column_speed * wind_direction.cos()).powi(2).sqrt(),
        northward_sign * (column_speed * wind_direction.sin()).powi(2).sqrt(),
    ))
}

fn downscale_longwave(
    glacier: bool,
    grid: GridForcing,
    column_surface_elevation_m: f64,
    column_temperature_k: f64,
    column_specific_humidity: f64,
    column_pressure_pa: f64,
    config: ForcingDownscalingConfig,
) -> Result<f64> {
    let elevation_difference = column_surface_elevation_m - grid.surface_elevation_m;
    let longwave = match config.longwave {
        LongwaveDownscaling::ClearSky => {
            let vapor_pressure_grid_hpa = grid.specific_humidity
                * saturation_specific_humidity(grid.air_temperature_k, grid.bottom_pressure_pa)?
                    .vapor_pressure_pa
                / 100.0;
            let vapor_pressure_column_hpa = column_specific_humidity
                * saturation_specific_humidity(column_temperature_k, column_pressure_pa)?
                    .vapor_pressure_pa
                / 100.0;
            let clear_sky_emissivity_grid =
                0.23 + 0.43 * (vapor_pressure_grid_hpa / grid.air_temperature_k).powf(1.0 / 5.7);
            let clear_sky_emissivity_column =
                0.23 + 0.43 * (vapor_pressure_column_hpa / column_temperature_k).powf(1.0 / 5.7);
            let all_sky_emissivity_grid = grid.downward_longwave_w_m2
                / (STEFAN_BOLTZMANN_W_M2_K4 * grid.air_temperature_k.powi(4));
            (clear_sky_emissivity_column + all_sky_emissivity_grid - clear_sky_emissivity_grid)
                * STEFAN_BOLTZMANN_W_M2_K4
                * column_temperature_k.powi(4)
        }
        LongwaveDownscaling::LapseRate if glacier => {
            grid.downward_longwave_w_m2
                - config.glacier_longwave_lapse_rate_w_m2_m * elevation_difference
        }
        LongwaveDownscaling::LapseRate => {
            grid.downward_longwave_w_m2
                - 4.0 * grid.downward_longwave_w_m2
                    / (0.5 * (column_temperature_k + grid.air_temperature_k))
                    * config.temperature_lapse_rate_k_m
                    * elevation_difference
        }
    };
    Ok(longwave.clamp(
        grid.downward_longwave_w_m2 * (1.0 - config.longwave_limit),
        grid.downward_longwave_w_m2 * (1.0 + config.longwave_limit),
    ))
}

fn downscale_shortwave(
    grid: GridForcing,
    column_pressure_pa: f64,
    solar: DownscalingSolarGeometry,
    terrain: FullTerrain<'_>,
    limit: f64,
) -> Result<f64> {
    let mut zenith_radians = solar.cosine_zenith.acos();
    let shadow = full_shadow_factor(zenith_radians, solar.cosine_azimuth.acos(), terrain.shadow);
    let (diffuse_grid, beam_grid, optical_factor) = shortwave_components(
        grid,
        column_pressure_pa,
        solar.calendar_day,
        solar.cosine_zenith,
        false,
    );
    let zenith_limit = 85.0 * F77_PI / 180.0;
    if zenith_radians > zenith_limit {
        zenith_radians = zenith_limit;
    }
    let beam_column = terrain
        .slope_radians
        .iter()
        .zip(terrain.aspect_radians)
        .zip(terrain.area_fraction)
        .map(|((&slope, &aspect), &area)| {
            let illumination =
                (slope.cos() + zenith_radians.tan() * slope.sin() * aspect.cos()).clamp(0.0, 1.0);
            shadow * illumination * optical_factor * area.clamp(0.0, 1.0) * beam_grid
        })
        .sum::<f64>();
    let sky_view_factor = terrain.sky_view_factor.clamp(0.0, 1.0);
    let diffuse_column = sky_view_factor * diffuse_grid;
    let reflected_column = if terrain.blue_sky_albedo.is_nan() {
        0.0
    } else {
        let albedo = if (0.0..=1.0).contains(&terrain.blue_sky_albedo) {
            terrain.blue_sky_albedo
        } else {
            0.0
        };
        terrain
            .slope_radians
            .iter()
            .map(|slope| {
                let terrain_configuration = ((1.0 + slope.cos()) / 2.0 - sky_view_factor).max(0.0);
                albedo
                    * terrain_configuration
                    * (beam_column * solar.cosine_zenith + (1.0 - sky_view_factor) * diffuse_column)
            })
            .sum::<f64>()
    };
    let shortwave = beam_column + diffuse_column + reflected_column;
    Ok(shortwave
        .clamp(
            grid.downward_shortwave_w_m2 * (1.0 - limit),
            grid.downward_shortwave_w_m2 * (1.0 + limit),
        )
        .max(0.0001))
}

fn downscale_shortwave_simple(
    grid: GridForcing,
    column_pressure_pa: f64,
    solar: DownscalingSolarGeometry,
    terrain: SimpleTerrain<'_>,
    limit: f64,
) -> Result<f64> {
    let mut zenith_radians = solar.cosine_zenith.acos();
    let (diffuse_grid, beam_grid, optical_factor) = shortwave_components(
        grid,
        column_pressure_pa,
        solar.calendar_day,
        solar.cosine_zenith,
        true,
    );
    let zenith_limit = 85.0 * F77_PI / 180.0;
    if zenith_radians > zenith_limit {
        zenith_radians = zenith_limit;
    }
    let beam_column = terrain
        .slope_tangent
        .iter()
        .zip(terrain.area_fraction)
        .enumerate()
        .map(|(index, (&slope, &area))| {
            let illumination = if index == ASPECT_TYPES - 1 {
                1.0
            } else {
                (slope.atan().cos()
                    + zenith_radians.tan() * slope.atan().sin() * simple_aspect(index).cos())
                .clamp(0.0, 1.0)
            };
            illumination * optical_factor * area.clamp(0.0, 1.0) * beam_grid
        })
        .sum::<f64>();
    let shortwave = beam_column + diffuse_grid;
    Ok(shortwave
        .clamp(
            grid.downward_shortwave_w_m2 * (1.0 - limit),
            grid.downward_shortwave_w_m2 * (1.0 + limit),
        )
        .max(0.0001))
}

fn shortwave_components(
    grid: GridForcing,
    column_pressure_pa: f64,
    calendar_day: f64,
    cosine_zenith: f64,
    simple: bool,
) -> (f64, f64, f64) {
    let earth_sun_distance_ratio = 1.0 - 0.01672 * (0.9856 * (calendar_day - 4.0)).cos();
    let top_of_atmosphere = SOLAR_CONSTANT_W_M2 * earth_sun_distance_ratio.powi(2) * cosine_zenith;
    let mut clearness_index =
        if (simple && top_of_atmosphere < 1.0e-7) || (!simple && top_of_atmosphere == 0.0) {
            0.0
        } else {
            grid.downward_shortwave_w_m2 / top_of_atmosphere
        };
    if clearness_index > 1.0 {
        clearness_index = 1.0;
    }
    let diffuse_weight =
        (0.952 - 1.041 * (-(2.3 - 4.702 * clearness_index).min(3.5).exp()).exp()).clamp(0.0, 1.0);
    let attenuation_coefficient = if clearness_index <= 0.0 {
        0.0
    } else {
        clearness_index.ln() / column_pressure_pa
    };
    let mut optical_factor =
        (attenuation_coefficient * (grid.bottom_pressure_pa - column_pressure_pa)).exp();
    if optical_factor.is_finite() && !(-10_000.0..=10_000.0).contains(&optical_factor) {
        optical_factor = 0.0;
    }
    (
        grid.downward_shortwave_w_m2 * diffuse_weight,
        grid.downward_shortwave_w_m2 * (1.0 - diffuse_weight),
        optical_factor,
    )
}

fn full_shadow_factor(zenith_radians: f64, azimuth_radians: f64, shadow: ShadowMask<'_>) -> f64 {
    let azimuth_degrees = azimuth_radians * 180.0 / F77_PI;
    let mut azimuth_index = (azimuth_degrees * AZIMUTH_BINS as f64 / 360.0) as usize;
    if azimuth_index == 0 {
        azimuth_index = 1;
    }
    let factor = match shadow {
        ShadowMask::Lookup(table) => {
            let zenith_degrees = zenith_radians * 180.0 / F77_PI;
            let mut zenith_index = (zenith_degrees * ZENITH_BINS as f64 / 90.0) as usize;
            if zenith_index == 0 {
                zenith_index = 1;
            }
            zenith_index = zenith_index.min(ZENITH_BINS);
            table[azimuth_index - 1][zenith_index - 1]
        }
        ShadowMask::Curve(curves) => {
            let [segment, a1, a2] = curves[azimuth_index - 1];
            if zenith_radians <= segment || a1 <= 1.0e-10 {
                1.0
            } else {
                (-(a1 * zenith_radians + a2).min(3.5).exp()).exp()
            }
        }
    };
    factor.clamp(0.0, 1.0)
}

fn downscale_precipitation(
    grid: GridForcing,
    column_surface_elevation_m: f64,
    scheme: PrecipitationDownscaling,
) -> (f64, f64) {
    let elevation_difference = column_surface_elevation_m - grid.surface_elevation_m;
    let adjust = |precipitation: f64| match scheme {
        PrecipitationDownscaling::ElevationFraction => {
            let delta = if grid.maximum_elevation_m != 0.0 {
                precipitation * elevation_difference / grid.maximum_elevation_m
            } else {
                0.0
            };
            let mut adjusted = precipitation + delta;
            if adjusted <= 0.0 {
                adjusted = precipitation;
            }
            if adjusted == 0.0 {
                1.0e-10
            } else {
                adjusted
            }
        }
        PrecipitationDownscaling::ListonElder => {
            precipitation
                + precipitation * 0.27 * elevation_difference / (1.0 - 0.27 * elevation_difference)
        }
    };
    (
        adjust(grid.convective_precipitation_kg_m2_s).max(0.0),
        adjust(grid.large_scale_precipitation_kg_m2_s).max(0.0),
    )
}

fn dry_air_gas_constant() -> f64 {
    AVOGADRO_PER_KMOLE * BOLTZMANN_J_K / DRY_AIR_MOLECULAR_WEIGHT
}

fn compass_direction(wind_direction: f64) -> f64 {
    if wind_direction > F77_PI / 2.0 {
        2.0 * F77_PI - wind_direction + F77_PI / 2.0
    } else {
        F77_PI / 2.0 - wind_direction
    }
}

fn simple_aspect(index: usize) -> f64 {
    debug_assert!(index < ASPECT_TYPES - 1);
    index as f64 * F77_PI / 4.0
}

fn validate_input(
    input: ForcingDownscalingInput<'_>,
    config: ForcingDownscalingConfig,
) -> Result<()> {
    validate_finite(
        [
            input.grid.surface_elevation_m,
            input.grid.maximum_elevation_m,
            input.grid.air_temperature_k,
            input.grid.potential_temperature_k,
            input.grid.specific_humidity,
            input.grid.bottom_pressure_pa,
            input.grid.density_kg_m3,
            input.grid.convective_precipitation_kg_m2_s,
            input.grid.large_scale_precipitation_kg_m2_s,
            input.grid.downward_longwave_w_m2,
            input.grid.reference_height_m,
            input.grid.downward_shortwave_w_m2,
            input.grid.eastward_wind_m_s,
            input.grid.northward_wind_m_s,
            input.column_surface_elevation_m,
            input.solar.calendar_day,
            input.solar.cosine_zenith,
            input.solar.cosine_azimuth,
        ],
        "forcing-downscaling input",
    )?;
    ensure!(
        input.grid.air_temperature_k > 0.0
            && input.grid.bottom_pressure_pa > 0.0
            && (0.0..1.0).contains(&input.grid.specific_humidity)
            && input.grid.density_kg_m3 > 0.0
            && input.grid.convective_precipitation_kg_m2_s >= 0.0
            && input.grid.large_scale_precipitation_kg_m2_s >= 0.0
            && input.grid.downward_longwave_w_m2 >= 0.0
            && input.grid.downward_shortwave_w_m2 >= 0.0
            && input.grid.reference_height_m >= 0.0
            && (-1.0..=1.0).contains(&input.solar.cosine_zenith)
            && (-1.0..=1.0).contains(&input.solar.cosine_azimuth),
        "forcing-downscaling input is physically invalid"
    );
    validate_finite(
        [
            config.temperature_lapse_rate_k_m,
            config.glacier_longwave_lapse_rate_w_m2_m,
            config.longwave_limit,
            config.full_shortwave_limit,
            config.simple_shortwave_limit,
        ],
        "forcing-downscaling configuration",
    )?;
    ensure!(
        config.temperature_lapse_rate_k_m >= 0.0
            && config.glacier_longwave_lapse_rate_w_m2_m >= 0.0
            && (0.0..=1.0).contains(&config.longwave_limit)
            && (0.0..=1.0).contains(&config.full_shortwave_limit)
            && (0.0..=1.0).contains(&config.simple_shortwave_limit),
        "forcing-downscaling configuration is physically invalid"
    );
    match input.terrain {
        DownscalingTerrain::Full(terrain) => validate_finite(
            terrain
                .slope_radians
                .iter()
                .chain(terrain.aspect_radians)
                .chain(terrain.area_fraction)
                .copied()
                .chain([terrain.sky_view_factor]),
            "full terrain characterization",
        ),
        DownscalingTerrain::Simple(terrain) => validate_finite(
            terrain
                .slope_tangent
                .iter()
                .chain(terrain.area_fraction)
                .copied(),
            "simple terrain characterization",
        ),
    }
}

fn validate_finite(values: impl IntoIterator<Item = f64>, description: &str) -> Result<()> {
    ensure!(
        values.into_iter().all(f64::is_finite),
        "{description} must be finite"
    );
    Ok(())
}

#[cfg(test)]
#[path = "forcing_downscaling_tests.rs"]
mod forcing_downscaling_tests;
