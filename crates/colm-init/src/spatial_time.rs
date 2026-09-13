//! Spatial LCT cold-time restart adapter for Rust `mksrfdata` block artifacts.
//!
//! Each call owns one landpatch block, so both cold and observed initial states
//! stay layer-major and bounded by that block rather than the complete domain.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};
use colm_namelist::Document;

use crate::crop::{map_field_time_3d, map_field_time_profile_4d, AreaMapping, MapGrid};
use crate::runtime::coordinate_values;
use crate::single_point::enabled_existing_path;
use crate::spatial_static::{
    patch_coordinates, read_canopy, read_f64, read_patches, read_soil, read_spatial_pixel_sets,
    spatial_patch_type, Patches,
};
use crate::spatial_urban::SpatialUrbanData;
use crate::urban_restart::write_cold_urban_time_restart;
use crate::{
    cold_start_broadband_radiation_with_snow, cold_start_urban_radiation, colm_soil_grid,
    derive_initial_soil_hydraulics, derive_lake_layers, derive_snow_cover,
    derive_spatial_soil_parameters, initialize_snow_layers, leaf_optics_from_land_cover,
    orbital_calendar_day, orbital_cosine_zenith, resolve_cold_start_soil, write_time_restart,
    CalendarTime, ColdStartSoilInput, HydraulicModel, InitialSoilProfile, LandCoverScheme,
    OzoneFields, PlantHydraulicFields, RestartDate, RestartTuning, SnowAerosolFields,
    SnowSoilRestartFields, SoilField, SoilHydraulicModel, SoilReflectance, TimeLakeFields,
    TimePatchFields, TimeRadiationFields, TimeRestartDimensions, TimeRestartFile, TimeRestartInput,
    UrbanRadiationInput, UrbanRadiationState, MISSING,
};

/// Arguments for the LCT cold start of one spatial block.
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
    /// Optional monthly sources enabled by CoLM's `DEF_USE_*Init` switches.
    pub observations: SpatialObservedInitialization<'a>,
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
            observations: SpatialObservedInitialization::default(),
        }
    }
}

/// Existing optional observation files resolved from a CoLM namelist.
///
/// Upstream treats a requested but absent file as disabled, so this container
/// intentionally retains only existing regular files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpatialObservedInitializationPaths {
    pub soil: Option<PathBuf>,
    pub snow: Option<PathBuf>,
    pub water_table: Option<PathBuf>,
}

impl SpatialObservedInitializationPaths {
    pub fn from_document(document: &Document) -> Result<Self> {
        Ok(Self {
            soil: enabled_existing_path(document, "DEF_USE_SoilInit", "DEF_file_SoilInit")?,
            snow: enabled_existing_path(document, "DEF_USE_SnowInit", "DEF_file_SnowInit")?,
            water_table: enabled_existing_path(
                document,
                "DEF_USE_WaterTableInit",
                "DEF_file_WaterTable",
            )?,
        })
    }

    pub fn borrow(&self) -> SpatialObservedInitialization<'_> {
        SpatialObservedInitialization {
            soil: self.soil.as_deref(),
            snow: self.snow.as_deref(),
            water_table: self.water_table.as_deref(),
        }
    }
}

/// Borrowed observation sources passed to one spatial restart block.
#[derive(Debug, Clone, Copy, Default)]
pub struct SpatialObservedInitialization<'a> {
    pub soil: Option<&'a Path>,
    pub snow: Option<&'a Path>,
    pub water_table: Option<&'a Path>,
}

/// Writes the common timestamped restart emitted by the spatial LCT cold start.
pub fn write_spatial_lct_cold_time_restart(
    config: SpatialLctTimeConfig<'_>,
) -> Result<TimeRestartFile> {
    Ok(write_spatial_lct_cold_time_restart_with_urban(config, None)?.0)
}

pub(crate) fn write_spatial_lct_cold_time_restart_with_urban(
    config: SpatialLctTimeConfig<'_>,
    urban_data: Option<&SpatialUrbanData>,
) -> Result<(TimeRestartFile, Option<PathBuf>)> {
    let dimensions = TimeRestartDimensions::default();
    let patches = read_patches(config.landdata, config.land_cover_year, config.block_label)?;
    let count = patches.class.len();
    let month = month(config.date)?;
    let observed = read_observed_initialization(config, &patches, month)?;
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
    let mut canopy = read_canopy(
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
    let urban_count = urban_data.map_or(0, |data| data.urban_to_patch.len());
    let mut urban_at_patch = vec![None; count];
    if let Some(data) = urban_data {
        ensure!(
            data.state.tree_fraction.len() == urban_count
                && data.state.tree_top_m.len() == urban_count
                && data.state.tree_bottom_m.len() == urban_count,
            "urban cold-start geometry does not match its topology"
        );
        for (urban, &patch) in data.urban_to_patch.iter().enumerate() {
            ensure!(
                patch < count,
                "urban patch index exceeds the landpatch block"
            );
            ensure!(
                urban_at_patch[patch].replace(urban).is_none(),
                "multiple urban vectors map to landpatch {patch}"
            );
            ensure!(
                patches.class[patch] == 13,
                "urban vector maps to non-urban IGBP landpatch {}",
                patches.class[patch]
            );
            canopy.patch_top_m[patch] = data.state.tree_top_m[urban];
            canopy.patch_bottom_m[patch] = data.state.tree_bottom_m[urban];
        }
    }
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
    let mut snow_depth = vec![0.0; count];
    let mut snow_water_equivalent = vec![0.0; count];
    let mut ground_snow_fraction = vec![0.0; count];
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
    let mut urban_radiation = vec![None; urban_count];
    let mut urban_soil_liquid = vec![0.0; dimensions.soil_layers * urban_count];

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
            fveg[patch] = urban_at_patch[patch]
                .map(|urban| urban_data.expect("urban map has data").state.tree_fraction[urban])
                .unwrap_or(1.0);
            green[patch] = 1.0;
        }
        roughness[patch] = canopy.patch_top_m[patch] * 0.1;
        let porosity = soil_column(&soil, SoilField::Porosity, patch);
        let residual = soil_column(&soil, SoilField::ThetaR, patch);
        let psi0 = soil_column(&soil, SoilField::Psi0, patch);
        let ks = soil_column(&soil, SoilField::HydraulicConductivity, patch);
        let model = hydraulic_models(&soil, patch, config.hydraulic_model);
        let profile = observed.soil.as_ref().map(|state| InitialSoilProfile {
            depth_m: &state.depth_m,
            temperature_k: &state.temperature_k[patch],
            wetness: &state.wetness[patch],
            water_table_m: state.water_table_m[patch],
            valid: state.valid[patch],
        });
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
            profile,
            water_table_m: profile
                .is_none()
                .then(|| observed.water_table_m[patch])
                .flatten(),
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
        snow_depth[patch] = observed.snow_depth_m[patch];
        snow_water_equivalent[patch] = snow_depth[patch] * 250.0;
        let snow = initialize_snow_layers(kind[patch], snow_depth[patch], dimensions.snow_layers)?;
        for layer in 0..dimensions.snow_layers {
            let at = layer * count + patch;
            snow_node[at] = snow.node_depth_m[layer];
            snow_thickness[at] = snow.thickness_m[layer];
            temperature[at] = if snow.thickness_m[layer] > 0.0 {
                cold.temperature_k[0].min(272.16)
            } else {
                -999.0
            };
            ice[at] = snow.thickness_m[layer] * 250.0;
        }
        for layer in 0..dimensions.soil_layers {
            let at = layer * count + patch;
            let snow_soil = (dimensions.snow_layers + layer) * count + patch;
            temperature[snow_soil] = cold.temperature_k[layer];
            liquid[snow_soil] = if let Some(urban) = urban_at_patch[patch] {
                let data = urban_data.expect("urban map has data");
                urban_soil_liquid[layer * urban_count + urban] = cold.liquid_water_kg_m2[layer];
                cold.liquid_water_kg_m2[layer]
                    * (1.0 - data.state.roof_fraction[urban])
                    * data.state.pervious_road_fraction[urban]
            } else {
                cold.liquid_water_kg_m2[layer]
            };
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
            snow_water_equivalent[patch],
            snow_depth[patch],
            config.snow_cover_exponent,
        )?;
        ground_snow_fraction[patch] = cover.ground_snow_fraction;
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
            snow_depth[patch],
            cover.ground_snow_fraction,
            cold.temperature_k[0],
        )?;
        radiation.set(patch, &state);
        if let Some(urban) = urban_at_patch[patch] {
            let data = urban_data.expect("urban map has data");
            let state = cold_start_urban_radiation(UrbanRadiationInput {
                roof_fraction: data.state.roof_fraction[urban],
                pervious_ground_fraction: data.state.pervious_road_fraction[urban],
                water_fraction: data.state.water_fraction[urban],
                building_height_to_length: data.state.building_height_to_width[urban],
                roof_height_m: data.state.roof_height_m[urban],
                roof_albedo: urban_albedo(&data.roof_albedo, urban_count, urban),
                wall_albedo: urban_albedo(&data.wall_albedo, urban_count, urban),
                impervious_albedo: urban_albedo(&data.impervious_albedo, urban_count, urban),
                pervious_albedo: urban_albedo(&data.pervious_albedo, urban_count, urban),
                leaf_optics: leaf_optics_from_land_cover(config.land_cover, class)?,
                vegetation_fraction: fveg[patch],
                vegetation_center_height_m: data.state.roof_height_m[urban]
                    .min((data.state.tree_top_m[urban] + data.state.tree_bottom_m[urban]) / 2.0),
                lai: lai_now[patch],
                sai: sai_now[patch],
                wet_snow_fraction: 0.0,
                vegetation_snow: config.vegetation_snow,
                cosine_zenith: cosine_zenith[patch].max(0.01),
                previous_sunlit_wall_fraction: 0.5,
                lake_temperature_k: 285.0,
                roof_snow_fraction: 0.0,
                impervious_snow_fraction: 0.0,
                pervious_snow_fraction: 0.0,
                lake_snow_fraction: 0.0,
                roof_snow_water_mm: 0.0,
                impervious_snow_water_mm: 0.0,
                pervious_snow_water_mm: 0.0,
                lake_snow_water_mm: 0.0,
                roof_snow_age: 0.0,
                impervious_snow_age: 0.0,
                pervious_snow_age: 0.0,
                lake_snow_age: 0.0,
            })?;
            radiation.set_urban(patch, &state);
            urban_radiation[urban] = Some(state);
        }
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

    let common = write_time_restart(
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
                snow_water_equivalent_mm: &snow_water_equivalent,
                snow_depth_m: &snow_depth,
                vegetation_fraction: &fveg,
                ground_snow_fraction: &ground_snow_fraction,
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
    )?;
    let urban = if urban_count == 0 {
        None
    } else {
        let radiation = urban_radiation
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .context("urban cold restart has unmapped urban radiation")?;
        let data = urban_data.expect("nonzero urban count has data");
        let total_lai = data
            .urban_to_patch
            .iter()
            .map(|&patch| total_lai[patch])
            .collect::<Vec<_>>();
        let total_sai = data
            .urban_to_patch
            .iter()
            .map(|&patch| total_sai[patch])
            .collect::<Vec<_>>();
        Some(write_cold_urban_time_restart(
            config.restart_dir,
            config.case_name,
            config.land_cover_year,
            config.date,
            config.block_label,
            crate::urban_restart::ColdUrbanTimeRestartInput {
                radiation: &radiation,
                total_lai: &total_lai,
                total_sai: &total_sai,
                soil_liquid: &urban_soil_liquid,
            },
        )?)
    };
    Ok((common, urban))
}

#[derive(Debug)]
struct SpatialObservedState {
    soil: Option<SpatialSoilProfile>,
    snow_depth_m: Vec<f64>,
    water_table_m: Vec<Option<f64>>,
}

#[derive(Debug)]
struct SpatialSoilProfile {
    depth_m: Vec<f64>,
    temperature_k: Vec<Vec<f64>>,
    wetness: Vec<Vec<f64>>,
    water_table_m: Vec<f64>,
    valid: Vec<bool>,
}

/// Port the observed-state branch of `MOD_Initialize` before it calls
/// `IniTimeVar`: map all source fields over each complete landpatch, then let
/// the shared cold-soil kernel interpolate and initialize the native column.
fn read_observed_initialization(
    config: SpatialLctTimeConfig<'_>,
    patches: &Patches,
    month: u8,
) -> Result<SpatialObservedState> {
    let count = patches.class.len();
    let sources = config.observations;
    if sources.soil.is_none() && sources.snow.is_none() && sources.water_table.is_none() {
        return Ok(SpatialObservedState {
            soil: None,
            snow_depth_m: vec![0.0; count],
            water_table_m: vec![None; count],
        });
    }
    let pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches.element,
        &patches.start,
        &patches.end,
        &patches.shared_fraction,
        "landpatch",
    )?;
    let index = usize::from(month - 1);
    let soil = sources
        .soil
        .filter(|path| path.is_file())
        .map(|path| read_observed_soil(path, &pixels, index))
        .transpose()?;
    let snow_depth_m = sources
        .snow
        .filter(|path| path.is_file())
        .map(|path| read_observed_snow(path, &pixels, index))
        .transpose()?
        .unwrap_or_else(|| vec![0.0; count]);
    let water_table_m = if soil.is_none() {
        sources
            .water_table
            .filter(|path| path.is_file())
            .map(|path| read_observed_water_table(path, &pixels, index))
            .transpose()?
            .unwrap_or_else(|| vec![None; count])
    } else {
        vec![None; count]
    };
    Ok(SpatialObservedState {
        soil,
        snow_depth_m,
        water_table_m,
    })
}

fn read_observed_soil(
    path: &Path,
    pixels: &crate::spatial_static::SpatialPixelSets,
    month: usize,
) -> Result<SpatialSoilProfile> {
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open spatial soil initial state {}", path.display()))?;
    let grid = MapGrid::from_file(&file)?;
    let depth_m = coordinate_values(&file, "soildepth")?;
    ensure!(
        !depth_m.is_empty() && depth_m.iter().all(|depth| depth.is_finite()),
        "spatial soilstate soildepth must be finite and nonempty"
    );
    let zwt = map_field_time_3d(&file, "zwt", month, &grid)?;
    let mut mapping = AreaMapping::new(&grid, pixels)?;
    // `msoil2p%set_missing_value(zwt_grid, ...)` masks all three mapped
    // fields from the same zwt validity map before the averages are taken.
    mapping.exclude_invalid(zwt.validity())?;
    let water_table_m = mapping.average(&zwt)?;
    let valid = water_table_m
        .iter()
        .map(Option::is_some)
        .collect::<Vec<_>>();
    let water_table_m = water_table_m
        .into_iter()
        .map(|value| value.unwrap_or(0.0))
        .collect::<Vec<_>>();
    let mut temperature_k = vec![vec![0.0; depth_m.len()]; mapping.len()];
    let mut wetness = temperature_k.clone();
    for layer in 0..depth_m.len() {
        let temperature = mapping.average(&map_field_time_profile_4d(
            &file, "soiltemp", month, layer, &grid,
        )?)?;
        let water = mapping.average(&map_field_time_profile_4d(
            &file, "soilwat", month, layer, &grid,
        )?)?;
        for patch in 0..mapping.len() {
            // Invalid zwt profiles are replaced by `resolve_cold_start_soil`.
            // These placeholders are intentionally never used for those patches.
            temperature_k[patch][layer] = temperature[patch].unwrap_or(0.0);
            wetness[patch][layer] = water[patch].unwrap_or(0.0);
        }
    }
    Ok(SpatialSoilProfile {
        depth_m,
        temperature_k,
        wetness,
        water_table_m,
        valid,
    })
}

fn read_observed_snow(
    path: &Path,
    pixels: &crate::spatial_static::SpatialPixelSets,
    month: usize,
) -> Result<Vec<f64>> {
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open spatial snow initial state {}", path.display()))?;
    let grid = MapGrid::from_file(&file)?;
    let field = map_field_time_3d(&file, "snowdepth", month, &grid)?;
    let mut mapping = AreaMapping::new(&grid, pixels)?;
    mapping.exclude_invalid(field.validity())?;
    mapping
        .average(&field)?
        .into_iter()
        .map(|value| value.unwrap_or(0.0))
        .enumerate()
        .map(|(patch, depth)| {
            ensure!(
                depth.is_finite() && depth >= 0.0,
                "spatial snowdepth for landpatch {patch} must be finite and nonnegative"
            );
            Ok(depth)
        })
        .collect()
}

fn read_observed_water_table(
    path: &Path,
    pixels: &crate::spatial_static::SpatialPixelSets,
    month: usize,
) -> Result<Vec<Option<f64>>> {
    let file = netcdf::open(path).with_context(|| {
        format!(
            "cannot open spatial water-table initial state {}",
            path.display()
        )
    })?;
    // `gwtd%define_from_file(fwtd)` reads the explicit edge coordinates rather
    // than deriving cells from lat/lon centers.
    let grid = MapGrid::from_explicit_edges(&file)?;
    let field = map_field_time_3d(&file, "wtd", month, &grid)?;
    AreaMapping::new(&grid, pixels)?.average(&field)
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

fn urban_albedo(values: &[f64], urban_count: usize, urban: usize) -> [[f64; 2]; 2] {
    [
        [values[urban], values[urban_count + urban]],
        [
            values[2 * urban_count + urban],
            values[3 * urban_count + urban],
        ],
    ]
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

    fn set_urban(&mut self, patch: usize, state: &UrbanRadiationState) {
        for (target, source) in [
            (&mut self.albedo, state.albedo),
            (&mut self.sunlit, state.sunlit_tree_absorption),
            (&mut self.shaded, state.shaded_tree_absorption),
        ] {
            for (index, value) in source.into_iter().flatten().enumerate() {
                target[index * self.count + patch] = value;
            }
        }
        self.diffuse_extinction[patch] = state.diffuse_extinction;
    }
}
