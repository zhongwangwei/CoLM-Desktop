//! Spatial LCT cold-time restart adapter for Rust `mksrfdata` block artifacts.
//!
//! This is the no-observation branch of `mkinidata/MOD_Initialize.F90`: each
//! call owns exactly one landpatch block, so its buffers stay layer-major and
//! bounded by that block rather than by the complete domain.

use std::path::Path;

use anyhow::{ensure, Context, Result};

use crate::spatial_static::{
    patch_coordinates, read_canopy, read_f64, read_patches, read_soil, spatial_patch_type,
};
use crate::{
    cold_start_broadband_radiation_with_snow, colm_soil_grid, derive_initial_soil_hydraulics,
    derive_lake_layers, derive_snow_cover, derive_spatial_soil_parameters, initialize_snow_layers,
    leaf_optics_from_land_cover, orbital_calendar_day, orbital_cosine_zenith,
    resolve_cold_start_soil, write_time_restart, CalendarTime, ColdStartSoilInput, HydraulicModel,
    LandCoverScheme, OzoneFields, PlantHydraulicFields, RestartDate, RestartTuning,
    SnowAerosolFields, SnowSoilRestartFields, SoilField, SoilHydraulicModel, SoilReflectance,
    TimeLakeFields, TimePatchFields, TimeRadiationFields, TimeRestartDimensions, TimeRestartFile,
    TimeRestartInput, MISSING,
};

/// Arguments for the no-observation LCT cold start of one spatial block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialLctTimeConfig<'a> {
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// CoLM suffix without the leading underscore, e.g. `w180_s90`.
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub date: RestartDate,
    pub greenwich: bool,
    /// The year of the already materialized monthly LAI/SAI vector files.
    pub lai_year: i32,
    pub dynamic_lake: bool,
    pub plant_hydraulics: bool,
    pub ozone_stress: bool,
    pub variably_saturated_flow: bool,
    pub vegetation_snow: bool,
    pub snow_cover_exponent: f64,
    pub tuning: RestartTuning,
}

impl<'a> SpatialLctTimeConfig<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        landdata: &'a Path,
        restart_dir: &'a Path,
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
        land_cover: LandCoverScheme,
        hydraulic_model: HydraulicModel,
        date: RestartDate,
    ) -> Self {
        Self {
            landdata,
            restart_dir,
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            date,
            greenwich: false,
            lai_year: land_cover_year,
            dynamic_lake: false,
            plant_hydraulics: true,
            ozone_stress: false,
            variably_saturated_flow: false,
            vegetation_snow: true,
            snow_cover_exponent: 0.5,
            tuning: RestartTuning::default(),
        }
    }
}

/// Writes the common timestamped restart emitted by the spatial LCT cold start.
///
/// Optional observed soil, snow, and water-table maps deliberately remain outside
/// this entry point.  The caller must not silently use this no-observation branch
/// when any of those namelist switches are enabled.
pub fn write_spatial_lct_cold_time_restart(
    config: SpatialLctTimeConfig<'_>,
) -> Result<TimeRestartFile> {
    let dimensions = TimeRestartDimensions::default();
    let patches = read_patches(config.landdata, config.land_cover_year, config.block_label)?;
    let count = patches.class.len();
    let kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(config.land_cover, class))
        .collect::<Result<Vec<_>>>()?;
    ensure!(
        count > 0,
        "spatial cold start needs at least one land patch"
    );
    let (longitude, latitude) = patch_coordinates(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches,
    )?;
    let lake_depth = read_f64(
        config.landdata,
        "lakedepth",
        "lakedepth_patches",
        "lakedepth_patches",
        config.land_cover_year,
        config.block_label,
        count,
    )?;
    let lake = derive_lake_layers(&lake_depth, dimensions.lake_layers)?;
    let soil = derive_spatial_soil_parameters(
        &read_soil(
            config.landdata,
            config.land_cover_year,
            config.block_label,
            count,
        )?,
        &patches.class,
        &kind,
        dimensions.soil_layers,
        config.hydraulic_model,
    )?;
    let canopy = read_canopy(
        crate::SpatialLctStaticConfig::new(
            config.landdata,
            config.restart_dir,
            config.case_name,
            config.land_cover_year,
            config.block_label,
            config.land_cover,
            config.hydraulic_model,
        ),
        &patches.class,
        &kind,
        count,
    )?;
    let albedo = [
        read_f64(
            config.landdata,
            "soil",
            "soil_s_v_alb",
            "soil_s_v_alb",
            config.land_cover_year,
            config.block_label,
            count,
        )?,
        read_f64(
            config.landdata,
            "soil",
            "soil_d_v_alb",
            "soil_d_v_alb",
            config.land_cover_year,
            config.block_label,
            count,
        )?,
        read_f64(
            config.landdata,
            "soil",
            "soil_s_n_alb",
            "soil_s_n_alb",
            config.land_cover_year,
            config.block_label,
            count,
        )?,
        read_f64(
            config.landdata,
            "soil",
            "soil_d_n_alb",
            "soil_d_n_alb",
            config.land_cover_year,
            config.block_label,
            count,
        )?,
    ];
    let month = month(config.date)?;
    let lai = read_monthly(config, "LAI_patches", month)?;
    let sai = read_monthly(config, "SAI_patches", month)?;
    let grid = colm_soil_grid(dimensions.soil_layers)?;
    let interface_mm = grid
        .interface_depth_m
        .iter()
        .map(|depth| depth * 1000.0)
        .collect::<Vec<_>>();

    let mut snow_node = vec![0.0; dimensions.snow_layers * count];
    let mut snow_thickness = snow_node.clone();
    let mut temperature = vec![0.0; (dimensions.snow_layers + dimensions.soil_layers) * count];
    let mut liquid = temperature.clone();
    let mut ice = temperature.clone();
    let mut matric = vec![0.0; dimensions.soil_layers * count];
    let mut conductivity = matric.clone();
    let mut ground_temperature = vec![0.0; count];
    let mut water_table = vec![0.0; count];
    let mut aquifer = vec![0.0; count];
    let mut roughness = vec![0.0; count];
    let mut cosine_zenith = vec![0.0; count];
    let mut fveg = vec![0.0; count];
    let mut green = vec![0.0; count];
    let mut lai_now = lai.clone();
    let mut sai_now = sai.clone();
    let mut total_lai = lai;
    let mut total_sai = sai;
    let mut sigf = vec![1.0; count];
    let mut radiation = RadiationBuffers::new(count);

    for patch in 0..count {
        let class = patches.class[patch];
        ensure!(
            class > 0,
            "spatial cold start does not accept an ocean landpatch"
        );
        let water = is_water(config.land_cover, class);
        if water {
            total_lai[patch] = 0.0;
            total_sai[patch] = 0.0;
            lai_now[patch] = 0.0;
            sai_now[patch] = 0.0;
        } else {
            fveg[patch] = 1.0;
            green[patch] = 1.0;
        }
        roughness[patch] = canopy.patch_top_m[patch] * 0.1;
        let porosity = soil_column(&soil, SoilField::Porosity, patch);
        let residual = soil_column(&soil, SoilField::ThetaR, patch);
        let psi0 = soil_column(&soil, SoilField::Psi0, patch);
        let ks = soil_column(&soil, SoilField::HydraulicConductivity, patch);
        let model = hydraulic_models(&soil, patch, config.hydraulic_model);
        let cold = resolve_cold_start_soil(ColdStartSoilInput {
            patch_type: kind[patch],
            porosity: &porosity,
            residual_water: &residual,
            psi_s_mm: &psi0,
            saturated_conductivity_mm_s: &ks,
            hydraulic_model: &model,
            soil_node_depth_m: &grid.node_depth_m,
            soil_thickness_m: &grid.thickness_m,
            soil_interface_depth_m: &grid.interface_depth_m[1..],
            variably_saturated_flow: config.variably_saturated_flow,
            profile: None,
            water_table_m: None,
        })?;
        let hydraulics = derive_initial_soil_hydraulics(
            kind[patch],
            &cold.temperature_k,
            &cold.liquid_water_kg_m2,
            &interface_mm,
            &porosity,
            &residual,
            &psi0,
            &ks,
            &model,
        )?;
        let snow = initialize_snow_layers(kind[patch], 0.0, dimensions.snow_layers)?;
        for layer in 0..dimensions.snow_layers {
            let at = layer * count + patch;
            snow_node[at] = snow.node_depth_m[layer];
            snow_thickness[at] = snow.thickness_m[layer];
            temperature[at] = -999.0;
        }
        for layer in 0..dimensions.soil_layers {
            let at = layer * count + patch;
            let snow_soil = (dimensions.snow_layers + layer) * count + patch;
            temperature[snow_soil] = cold.temperature_k[layer];
            liquid[snow_soil] = cold.liquid_water_kg_m2[layer];
            ice[snow_soil] = cold.ice_water_kg_m2[layer];
            matric[at] = hydraulics.matric_potential_mm[layer];
            conductivity[at] = hydraulics.hydraulic_conductivity_mm_s[layer];
        }
        ground_temperature[patch] = cold.temperature_k[0];
        water_table[patch] = cold.water_table_depth_m;
        aquifer[patch] = cold.aquifer_water_mm;
        let cover = derive_snow_cover(
            total_lai[patch],
            total_sai[patch],
            roughness[patch],
            config.tuning.zlnd,
            0.0,
            0.0,
            config.snow_cover_exponent,
        )?;
        sigf[patch] = if water {
            0.0
        } else {
            cover.snow_free_vegetation_fraction
        };
        sai_now[patch] *= sigf[patch];
        let calendar_day = orbital_calendar_day(
            CalendarTime {
                year: config.date.year,
                julian_day: config.date.julian_day,
                seconds: config.date.seconds,
            },
            config.greenwich,
            longitude[patch].to_degrees(),
        )?;
        cosine_zenith[patch] =
            orbital_cosine_zenith(calendar_day, longitude[patch], latitude[patch]);
        let state = cold_start_broadband_radiation_with_snow(
            kind[patch],
            SoilReflectance {
                saturated_visible: albedo[0][patch],
                dry_visible: albedo[1][patch],
                saturated_near_infrared: albedo[2][patch],
                dry_near_infrared: albedo[3][patch],
            },
            cold.liquid_water_kg_m2[0],
            grid.thickness_m[0],
            leaf_optics_from_land_cover(config.land_cover, class)?,
            lai_now[patch],
            sai_now[patch],
            0.0,
            cosine_zenith[patch].max(0.001),
            true,
            config.land_cover == LandCoverScheme::Usgs,
            config.vegetation_snow,
            0.0,
            cover.ground_snow_fraction,
            cold.temperature_k[0],
        )?;
        radiation.set(patch, &state);
    }

    let zero = vec![0.0; count];
    let one = vec![1.0; count];
    let missing = vec![MISSING; count];
    let log30 = vec![30.0_f64.ln(); count];
    let lake_temperature = vec![285.0; dimensions.lake_layers * count];
    let lake_ice = vec![0.0; dimensions.lake_layers * count];
    let snow_zero = vec![0.0; dimensions.snow_layers * count];
    let grain = vec![54.526; dimensions.snow_layers * count];
    let snow_layer_absorption =
        vec![
            0.0;
            dimensions.bands * dimensions.radiation_types * (dimensions.snow_layers + 1) * count
        ];
    let plant_water = vec![-25_000.0; 4 * count];
    let conductance = vec![10_000.0; count];
    let wetland = kind
        .iter()
        .map(|&value| if value == 2 { 200.0 } else { 0.0 })
        .collect::<Vec<_>>();
    let surface_water = kind
        .iter()
        .zip(&lake.depth_m)
        .map(|(&value, &depth)| if value == 4 { depth * 1000.0 } else { 0.0 })
        .collect::<Vec<_>>();

    write_time_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.date,
        config.block_label,
        TimeRestartInput {
            dimensions,
            snow_soil: SnowSoilRestartFields {
                snow_node_depth_m: &snow_node,
                snow_layer_thickness_m: &snow_thickness,
                temperature_k: &temperature,
                liquid_water_kg_m2: &liquid,
                ice_water_kg_m2: &ice,
                matric_potential_mm: &matric,
                hydraulic_conductivity_mm_s: &conductivity,
            },
            patch: TimePatchFields {
                ground_temperature_k: &ground_temperature,
                leaf_temperature_k: &ground_temperature,
                canopy_water_mm: &zero,
                canopy_rain_mm: &zero,
                canopy_snow_mm: &zero,
                wet_snow_fraction: &zero,
                snow_age: &radiation.snow_age,
                snow_water_equivalent_mm: &zero,
                snow_depth_m: &zero,
                vegetation_fraction: &fveg,
                ground_snow_fraction: &zero,
                snow_free_vegetation_fraction: &sigf,
                greenness: &green,
                lai: &lai_now,
                total_lai: &total_lai,
                sai: &sai_now,
                total_sai: &total_sai,
                cosine_zenith: &cosine_zenith,
                thermal_gap_fraction: &radiation.thermal_gap,
                direct_extinction: &radiation.direct_extinction,
                diffuse_extinction: &radiation.diffuse_extinction,
                water_table_depth_m: &water_table,
                aquifer_water_mm: &aquifer,
                wetland_water_mm: &wetland,
                surface_water_mm: &surface_water,
                soil_surface_resistance_s_m: &missing,
                saved_tke: &vec![0.6; count],
                radiative_temperature_k: &ground_temperature,
                reference_temperature_k: &ground_temperature,
                reference_humidity: &vec![0.3; count],
                stomatal_resistance_s_m: &missing,
                emissivity: &one,
                roughness_length_m: &roughness,
                monin_obukhov_height: &vec![-1.0; count],
                bulk_richardson: &vec![-0.1; count],
                friction_velocity: &vec![0.25; count],
                humidity_scale: &vec![0.001; count],
                temperature_scale_k: &vec![-1.5; count],
                momentum_integral: &log30,
                heat_integral: &log30,
                moisture_integral: &log30,
            },
            radiation: TimeRadiationFields {
                albedo: &radiation.albedo,
                sunlit_absorption: &radiation.sunlit,
                shaded_absorption: &radiation.shaded,
                soil_absorption: &radiation.soil,
                snow_absorption: &radiation.snow,
                snow_layer_absorption: &snow_layer_absorption,
            },
            hyperspectral: None,
            lake: TimeLakeFields {
                temperature_k: &lake_temperature,
                ice_fraction: &lake_ice,
                layer_thickness_m: config.dynamic_lake.then_some(lake.thickness_m.as_slice()),
            },
            snow_aerosol: SnowAerosolFields {
                grain_radius: &grain,
                black_carbon_hydrophobic: &snow_zero,
                black_carbon_hydrophilic: &snow_zero,
                organic_carbon_hydrophobic: &snow_zero,
                organic_carbon_hydrophilic: &snow_zero,
                dust_1: &snow_zero,
                dust_2: &snow_zero,
                dust_3: &snow_zero,
                dust_4: &snow_zero,
            },
            plant_hydraulics: config.plant_hydraulics.then_some(PlantHydraulicFields {
                water_potential_mm: &plant_water,
                sunlit_stomatal_conductance: &conductance,
                shaded_stomatal_conductance: &conductance,
                vegetation_nodes: 4,
            }),
            ozone: config.ozone_stress.then_some(OzoneFields {
                lai_old: &lai_now,
                sunlit_uptake: &zero,
                shaded_uptake: &zero,
                sunlit_vegetation_coefficient: &one,
                shaded_vegetation_coefficient: &one,
                sunlit_ground_coefficient: &one,
                shaded_ground_coefficient: &one,
            }),
            irrigation: None,
        },
    )
}

fn read_monthly(config: SpatialLctTimeConfig<'_>, variable: &str, month: u8) -> Result<Vec<f64>> {
    let stem = format!("{variable}{month:02}");
    read_f64(
        config.landdata,
        "LAI",
        &stem,
        variable,
        config.lai_year,
        config.block_label,
        read_patches(config.landdata, config.land_cover_year, config.block_label)?
            .class
            .len(),
    )
}

pub(crate) fn month(date: RestartDate) -> Result<u8> {
    crate::month_lengths(date.year)
        .into_iter()
        .scan(i32::from(date.julian_day), |day, length| {
            let hit = *day <= length;
            *day -= length;
            Some(hit)
        })
        .position(|hit| hit)
        .map(|index| index as u8 + 1)
        .context("restart Julian day is outside its year")
}

fn soil_column(soil: &crate::SoilState, field: SoilField, patch: usize) -> Vec<f64> {
    (0..soil.layers)
        .map(|layer| soil.get(field, layer, patch))
        .collect()
}

fn hydraulic_models(
    soil: &crate::SoilState,
    patch: usize,
    model: HydraulicModel,
) -> Vec<SoilHydraulicModel> {
    (0..soil.layers)
        .map(|layer| match model {
            HydraulicModel::Campbell => SoilHydraulicModel::Campbell {
                bsw: soil.get(SoilField::Bsw, layer, patch),
            },
            HydraulicModel::VanGenuchten => SoilHydraulicModel::VanGenuchten {
                alpha_vgm: soil.get(SoilField::AlphaVgm, layer, patch),
                n_vgm: soil.get(SoilField::NVgm, layer, patch),
                l_vgm: soil.get(SoilField::LVgm, layer, patch),
                sc_vgm: soil.get(SoilField::ScVgm, layer, patch),
                fc_vgm: soil.get(SoilField::FcVgm, layer, patch),
            },
        })
        .collect()
}

fn is_water(scheme: LandCoverScheme, class: i32) -> bool {
    matches!(scheme, LandCoverScheme::Igbp) && class == 17
        || matches!(scheme, LandCoverScheme::Usgs) && class == 16
}

struct RadiationBuffers {
    albedo: Vec<f64>,
    sunlit: Vec<f64>,
    shaded: Vec<f64>,
    soil: Vec<f64>,
    snow: Vec<f64>,
    snow_age: Vec<f64>,
    thermal_gap: Vec<f64>,
    direct_extinction: Vec<f64>,
    diffuse_extinction: Vec<f64>,
    count: usize,
}

impl RadiationBuffers {
    fn new(count: usize) -> Self {
        Self {
            albedo: vec![0.0; 4 * count],
            sunlit: vec![0.0; 4 * count],
            shaded: vec![0.0; 4 * count],
            soil: vec![0.0; 4 * count],
            snow: vec![0.0; 4 * count],
            snow_age: vec![0.0; count],
            thermal_gap: vec![0.0; count],
            direct_extinction: vec![0.0; count],
            diffuse_extinction: vec![0.0; count],
            count,
        }
    }
    fn set(&mut self, patch: usize, state: &crate::ColdStartRadiation) {
        for (target, source) in [
            (&mut self.albedo, state.albedo),
            (&mut self.sunlit, state.sunlit_absorption),
            (&mut self.shaded, state.shaded_absorption),
            (&mut self.soil, state.soil_absorption),
            (&mut self.snow, state.snow_absorption),
        ] {
            for (index, value) in source.into_iter().flatten().enumerate() {
                target[index * self.count + patch] = value;
            }
        }
        self.snow_age[patch] = state.snow_age;
        self.thermal_gap[patch] = state.thermal_gap_fraction;
        self.direct_extinction[patch] = state.direct_extinction;
        self.diffuse_extinction[patch] = state.diffuse_extinction;
    }
}
