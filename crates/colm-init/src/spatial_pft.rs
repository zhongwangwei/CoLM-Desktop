//! Spatial PFT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! The common land-patch restart is written by [`crate::spatial_static`].
//! This module writes its separate `landpft` companion, exactly as CoLM's
//! `WRITE_PFTimeInvariants` does after `pct_readin` and `HTOP_readin`.

use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    bsm_soil_moisture, cold_start_ground_albedo, cold_start_pc_broadband_radiation_from_ground,
    expand_broadband_ground_albedo, expand_broadband_leaf_optics,
    high_resolution_pft_cold_start_state, pft_high_resolution_radiation, prospect_leaf_optics,
    select_high_resolution_radiation, CalendarTime, HighResolutionLeafOptics, SoilReflectance,
    HIGH_RES_WAVELENGTHS,
};
use colm_forcing::{
    read_high_resolution_leaf_optics, read_high_resolution_radiation_table,
    read_high_resolution_urban_albedo, read_high_resolution_water_optics,
    HighResolutionLeafOpticsTable, HighResolutionRadiationTable, HighResolutionWaterOptics,
};
use colm_namelist::{parse, Value};

use crate::crop::{map_field_2d, map_field_soil_3d, AreaMapping, MapGrid};
use crate::single_point::{
    aggregate_pft_radiation, optional_i32, pc_canopy_layer, pc_uses_three_dimensional_canopy,
    pft_canopy, pft_leaf_optics, pft_parameters, required_string, IGBP_BOTTOM, IGBP_TOP,
};
use crate::spatial_static::{
    block_path, patch_coordinates, read_f64 as read_lct_f64, read_hyperspectral_albedo,
    read_patches, read_soil, read_spatial_pixel_sets, resolve_vic_parameter_file,
    spatial_patch_type, values_f64, values_i32, write_spatial_lct_constant_restart_with_canopy,
    SpatialLctStaticConfig, VicParameterSource,
};
use crate::{
    append_time_hyperspectral_fields, bgc_time_restart_input,
    cold_start_pc_broadband_radiation_with_snow, cold_start_pft_broadband_radiation_with_snow,
    derive_cold_start_bgc_state, merge_bgc_cold_start_states, write_bgc_time_restart,
    write_cold_start_bgc_constant_restart, write_pft_constant_restart, write_pft_time_restart,
    BgcColdStartInput, BgcConstantRestartFiles, BgcPftColdStartInput, BgcTimeRestartFile,
    ColdStartRadiation, ConstantRestartFiles, HydraulicModel, LandCoverScheme, PcPftInput,
    PftBgcFields, PftConstantRestartInput, PftHyperspectralFields, PftOzoneFields,
    PftPlantHydraulicFields, PftTimeFields, PftTimeRestartInput, RestartDate, RestartTuning,
    RuntimeCnState, RuntimeCnVegetationCarbon, SpatialLctTimeConfig,
    SpatialObservedInitializationPaths, TimeHyperspectralFields, TimeRestartFile, MISSING,
};

/// Arguments for one already-addressed spatial `landpft` block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialPftStaticConfig<'a> {
    pub namelist: &'a Path,
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// CoLM suffix without the leading underscore, e.g. `w180_s90`.
    pub block_label: &'a str,
}

/// The common and PFT-specific constant restart blocks for one spatial PFT case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialPftConstantRestartFiles {
    pub common: ConstantRestartFiles,
    pub pft: PathBuf,
    pub bgc: Option<BgcConstantRestartFiles>,
}

/// Arguments for the PFT cold start of one spatial block.
///
/// The PFT and BGC state derives through `colm-core`; CROP remains separate
/// because its crop-management state has not yet been materialized spatially.
#[derive(Debug, Clone, Copy)]
pub struct SpatialPftTimeConfig<'a> {
    pub static_config: SpatialPftStaticConfig<'a>,
    pub date: RestartDate,
    pub lai_year: i32,
    pub greenwich: bool,
    pub dynamic_lake: bool,
    pub plant_hydraulics: bool,
    pub ozone_stress: bool,
    pub variably_saturated_flow: bool,
    pub vegetation_snow: bool,
    pub snow_cover_exponent: f64,
    pub tuning: RestartTuning,
    /// Emit and derive CoLM's 211-band PFT cold-start state.
    pub use_hyperspectral: bool,
    /// `colm_PFT_params.nc`; required when `DEF_HighResVeg` or `DEF_PROSPECT` is enabled.
    pub high_resolution_leaf_optics: Option<&'a Path>,
    /// `water_params.txt`; required when `DEF_HighResSoil = .true.`.
    pub high_resolution_water_optics: Option<&'a Path>,
    /// `swnb_480bnd_fsds.nc`; required for every hyperspectral cold start.
    pub high_resolution_radiation: Option<&'a Path>,
    /// `DEF_HighResUrban_albedo`; read by upstream for every hyperspectral cold start.
    pub high_resolution_urban_albedo: Option<&'a Path>,
}

/// Timestamped restart blocks written by a spatial PFT cold start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialPftTimeRestartFiles {
    pub common: TimeRestartFile,
    pub pft: PathBuf,
    pub bgc: Option<BgcTimeRestartFile>,
}

impl<'a> SpatialPftStaticConfig<'a> {
    pub fn new(
        namelist: &'a Path,
        landdata: &'a Path,
        restart_dir: &'a Path,
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
    ) -> Self {
        Self {
            namelist,
            landdata,
            restart_dir,
            case_name,
            land_cover_year,
            block_label,
        }
    }
}

impl<'a> SpatialPftTimeConfig<'a> {
    pub fn new(static_config: SpatialPftStaticConfig<'a>, date: RestartDate) -> Self {
        Self {
            lai_year: static_config.land_cover_year,
            static_config,
            date,
            greenwich: false,
            dynamic_lake: false,
            plant_hydraulics: true,
            ozone_stress: false,
            variably_saturated_flow: false,
            vegetation_snow: true,
            snow_cover_exponent: 0.5,
            tuning: RestartTuning::default(),
            use_hyperspectral: false,
            high_resolution_leaf_optics: None,
            high_resolution_water_optics: None,
            high_resolution_radiation: None,
            high_resolution_urban_albedo: None,
        }
    }
}

/// Writes the PFT/PC constant restart for one Rust `mksrfdata-rs spatial-pft` block.
pub fn write_spatial_pft_constant_restart(config: SpatialPftStaticConfig<'_>) -> Result<PathBuf> {
    let document = read_pft_document(config.namelist)?;
    let compression_level = crate::restart::restart_compression_level(&document)?;
    let class = read_i32(
        config.landdata,
        "landpft",
        "landpft",
        "settyp",
        config.land_cover_year,
        config.block_label,
    )?;
    let fraction = read_f64(
        config.landdata,
        "pctpft",
        "pct_pfts",
        "pct_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    let observed_height_m = read_f64(
        config.landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    ensure!(
        !class.is_empty()
            && class.len() == fraction.len()
            && class.len() == observed_height_m.len(),
        "spatial landpft, pct_pfts, and htop_pfts vectors must be nonempty and have equal lengths"
    );
    let crop_fraction = optional_bool_or(&document, "DEF_USE_CROP", false)?.then(|| {
        read_f64(
            config.landdata,
            "pctpft",
            "pct_crops",
            "pct_crops",
            config.land_cover_year,
            config.block_label,
        )
    });
    let crop_fraction = crop_fraction.transpose()?;
    if let Some(crop_fraction) = &crop_fraction {
        let patches = read_i32(
            config.landdata,
            "landpatch",
            "landpatch",
            "settyp",
            config.land_cover_year,
            config.block_label,
        )?;
        ensure!(
            crop_fraction.len() == patches.len()
                && crop_fraction
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value)),
            "spatial pct_crops must be a finite [0, 1] value for every land patch"
        );
    }
    let canopy = pft_canopy(&document, &class, &observed_height_m)?;
    write_pft_constant_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        PftConstantRestartInput {
            compression_level,
            class: &class,
            fraction: &fraction,
            canopy_top_m: &canopy.top_m,
            canopy_bottom_m: &canopy.bottom_m,
            crop_fraction: crop_fraction.as_deref(),
        },
    )
}

/// Writes both restart families required by a spatial PFT cold start.
///
/// PFT landpatches retain IGBP's water and glacier classes, so the common
/// restart uses the IGBP static adapter rather than duplicating its soil,
/// lake, and terrain mapping.  The soil model is read from the same case
/// namelist that supplies PFT canopy overrides.
pub fn write_spatial_pft_constant_restarts(
    config: SpatialPftStaticConfig<'_>,
    use_bedrock: bool,
    use_hyperspectral: bool,
) -> Result<SpatialPftConstantRestartFiles> {
    let document = read_pft_document(config.namelist)?;
    let compression_level = crate::restart::restart_compression_level(&document)?;
    let hydraulic_model = pft_hydraulic_model(config.namelist)?;
    let mut common = SpatialLctStaticConfig::new(
        config.landdata,
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        LandCoverScheme::Igbp,
        hydraulic_model,
    );
    common.use_bedrock = use_bedrock;
    common.compression_level = compression_level;
    common.tuning = RestartTuning::from_document(&document)?;
    common.use_hyperspectral = use_hyperspectral;
    let runoff_scheme = optional_i32(&document, "DEF_Runoff_SCHEME")?.unwrap_or(3);
    common.use_topmodel = runoff_scheme == 0;
    common.topmodel_method = optional_i32(&document, "DEF_TOPMOD_method")?.unwrap_or(0);
    let vic_file = if runoff_scheme == 1 {
        let use_grid = optional_bool_or(&document, "DEF_VIC_OPT", false)?;
        Some((resolve_vic_parameter_file(&document, use_grid)?, use_grid))
    } else {
        None
    };
    common.vic_parameters = match &vic_file {
        Some((path, true)) => VicParameterSource::GridFile(path),
        Some((path, false)) => VicParameterSource::ScalarFile(path),
        None => VicParameterSource::None,
    };
    common.use_simple_terrain =
        optional_bool_or(&document, "DEF_USE_Forcing_Downscaling_Simple", false)?;
    common.use_regular_terrain = optional_bool_or(&document, "DEF_USE_Forcing_Downscaling", false)?;
    let patches = read_patches(config.landdata, config.land_cover_year, config.block_label)?;
    let patch_kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(LandCoverScheme::Igbp, class))
        .collect::<Result<Vec<_>>>()?;
    let pfts = read_pft_vectors(config)?;
    let use_crop = optional_bool_or(&document, "DEF_USE_CROP", false)?;
    let pft_indices = match_pfts_to_patches(&patches, &patch_kind, &pfts, use_crop)?;
    let pft_heights = pft_canopy(&document, &pfts.class, &pfts.observed_height_m)?;
    let mut canopy = colm_core::CanopyState {
        patch_top_m: patches
            .class
            .iter()
            .map(|&class| IGBP_TOP[class as usize])
            .collect(),
        patch_bottom_m: patches
            .class
            .iter()
            .map(|&class| IGBP_BOTTOM[class as usize])
            .collect(),
        pft_top_m: Vec::new(),
        pft_bottom_m: Vec::new(),
    };
    // MOD_HtopReadin weights the class-adjusted PFT heights, including WMO
    // grass/bare defaults. Non-soil patches retain the IGBP table heights.
    for (patch, indices) in pft_indices.iter().enumerate() {
        if patch_kind[patch] == 0 {
            ensure!(
                !indices.is_empty(),
                "natural patch {patch} has no PFT canopy"
            );
            canopy.patch_top_m[patch] = indices.iter().fold(0.0, |sum, &pft| {
                pft_heights.top_m[pft].mul_add(pfts.fraction[pft], sum)
            });
            canopy.patch_bottom_m[patch] = indices.iter().fold(0.0, |sum, &pft| {
                pft_heights.bottom_m[pft].mul_add(pfts.fraction[pft], sum)
            });
        }
    }
    let common = write_spatial_lct_constant_restart_with_canopy(common, Some(canopy))?;
    let pft = write_spatial_pft_constant_restart(config)?;
    let bgc = optional_bool_or(&document, "DEF_USE_BGC", false)?
        .then(|| {
            let patches =
                read_patches(config.landdata, config.land_cover_year, config.block_label)?;
            write_cold_start_bgc_constant_restart(
                config.restart_dir,
                config.case_name,
                config.land_cover_year,
                config.block_label,
                patches.class.len(),
                optional_bool_or(&document, "DEF_USE_NITRIF", true)?,
                compression_level,
            )
        })
        .transpose()?;
    Ok(SpatialPftConstantRestartFiles { common, pft, bgc })
}

/// Writes both common and PFT timestamped restart blocks for one spatial PFT
/// cold start.
///
/// The common block is first built by the LCT initializer, which owns the
/// shared observed/cold soil, lake, snow, and clock state.  Natural
/// patches are then replaced with their exact PFT-weighted optical state and
/// roughness, while water, ice, wetland, and urban patches retain that shared
/// common state.  This prevents a second copy of the cold-soil path here.
pub fn write_spatial_pft_cold_time_restarts(
    config: SpatialPftTimeConfig<'_>,
) -> Result<SpatialPftTimeRestartFiles> {
    let document = read_pft_document(config.static_config.namelist)?;
    let compression_level = crate::restart::restart_compression_level(&document)?;
    ensure!(
        !optional_bool_or(&document, "DEF_USE_SNICAR", false)?,
        "DEF_USE_SNICAR: SNICAR snow-optics cold-start initialization is not yet implemented in Rust"
    );
    let observations = SpatialObservedInitializationPaths::from_document(&document)?;
    let use_bgc = optional_bool_or(&document, "DEF_USE_BGC", false)?;
    let use_crop = optional_bool_or(&document, "DEF_USE_CROP", false)?;
    ensure!(
        !use_crop || use_bgc,
        "spatial CROP cold starts require DEF_USE_BGC = .true."
    );
    let cn_initial_state = optional_bool_or(&document, "DEF_USE_CN_INIT", false)?
        .then(|| required_string(&document, "DEF_file_cn_init").map(std::path::PathBuf::from))
        .transpose()?;
    if let Some(path) = &cn_initial_state {
        ensure!(
            path.is_file(),
            "spatial BGC equilibrium source does not exist: {}",
            path.display()
        );
    }
    let use_nitrification = optional_bool_or(&document, "DEF_USE_NITRIF", true)?;
    let subgrid = spatial_pft_subgrid(&document)?;
    if use_crop {
        ensure!(
            subgrid == SpatialPftSubgrid::Pft
                || optional_bool_or(&document, "DEF_PC_CROP_SPLIT", true)?,
            "CROP with DEF_USE_PC requires DEF_PC_CROP_SPLIT = .true."
        );
    }
    // `MOD_Albedo_HiRes` only runs its spectral canopy solver for PFT.
    // PC keeps spectral ground state and its spectral PFT arrays at their initialized
    // zero/missing values, then replaces the broadband state with ThreeDCanopy.
    let high_resolution_canopy = config.use_hyperspectral && subgrid == SpatialPftSubgrid::Pft;
    let use_prospect =
        high_resolution_canopy && optional_bool_or(&document, "DEF_PROSPECT", false)?;
    // Upstream leaf_property_init reads colm_PFT_params.nc for either branch:
    // PROSPECT replaces green tissue but retains the file's dead-stem spectrum.
    let high_resolution_vegetation = high_resolution_canopy
        && (optional_bool_or(&document, "DEF_HighResVeg", true)? || use_prospect);
    let high_resolution_soil =
        config.use_hyperspectral && optional_bool_or(&document, "DEF_HighResSoil", true)?;
    let high_resolution_urban_albedo = config
        .use_hyperspectral
        .then(|| {
            read_high_resolution_urban_albedo(config.high_resolution_urban_albedo.context(
                "HYPERSPECTRAL cold start needs --highres-urban-albedo because upstream mkinidata always reads DEF_HighResUrban_albedo",
            )?)
        })
        .transpose()?;
    let high_resolution_sources: Option<(
        Option<HighResolutionLeafOpticsTable>,
        Option<HighResolutionWaterOptics>,
        Option<HighResolutionRadiationTable>,
    )> = config
        .use_hyperspectral
        .then(|| {
            let radiation = Some(read_high_resolution_radiation_table(
                config
                    .high_resolution_radiation
                    .context("HYPERSPECTRAL cold start needs --highres-radiation")?,
            )?);
            let leaf = high_resolution_vegetation
                .then(|| {
                    read_high_resolution_leaf_optics(
                        config.high_resolution_leaf_optics.context(
                            "DEF_HighResVeg or DEF_PROSPECT requires --highres-leaf-optics",
                        )?,
                    )
                })
                .transpose()?;
            let water = high_resolution_soil
                .then(|| {
                    read_high_resolution_water_optics(
                        config
                            .high_resolution_water_optics
                            .context("DEF_HighResSoil requires --highres-water-optics")?,
                    )
                })
                .transpose()?;
            Ok::<_, anyhow::Error>((leaf, water, radiation))
        })
        .transpose()?;
    let hydraulic_model = pft_hydraulic_model_from_document(&document)?;
    let mut common_config = SpatialLctTimeConfig::new(
        config.static_config.landdata,
        config.static_config.restart_dir,
        config.static_config.case_name,
        config.static_config.land_cover_year,
        config.static_config.block_label,
        LandCoverScheme::Igbp,
        hydraulic_model,
        config.date,
    );
    common_config.lai_year = config.lai_year;
    common_config.greenwich = config.greenwich;
    common_config.dynamic_lake = config.dynamic_lake;
    common_config.plant_hydraulics = config.plant_hydraulics;
    common_config.ozone_stress = config.ozone_stress;
    common_config.variably_saturated_flow = config.variably_saturated_flow;
    common_config.vegetation_snow = config.vegetation_snow;
    common_config.snow_cover_exponent = config.snow_cover_exponent;
    common_config.compression_level = compression_level;
    common_config.tuning = config.tuning;
    common_config.observations = observations.borrow();
    let common = crate::write_spatial_lct_cold_time_restart(common_config)?;

    let patches = read_patches(
        config.static_config.landdata,
        config.static_config.land_cover_year,
        config.static_config.block_label,
    )?;
    let patch_kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(LandCoverScheme::Igbp, class))
        .collect::<Result<Vec<_>>>()?;
    let pfts = read_pft_vectors(config.static_config)?;
    let pft_to_patch = match_pfts_to_patches(&patches, &patch_kind, &pfts, use_crop)?;
    let pft_owner = pft_owners(&pft_to_patch, pfts.class.len())?;
    let crop = use_crop
        .then(|| spatial_crop_state(&document, config.static_config, &patches, &pfts, &pft_owner))
        .transpose()?;
    let bgc_state = use_bgc
        .then(|| {
            derive_spatial_bgc_state(
                &document,
                config.static_config,
                hydraulic_model,
                &patches,
                &patch_kind,
                &pfts,
                &pft_to_patch,
                cn_initial_state.as_deref(),
                use_nitrification,
            )
        })
        .transpose()?;
    let bgc_pft_values = bgc_state.as_ref().map(|state| {
        state
            .pft_values
            .iter()
            .map(Vec::as_slice)
            .collect::<Vec<_>>()
    });
    let pft_count = pfts.class.len();
    let month = crate::spatial_time::month(config.date)?;
    let mut total_lai = read_pft_monthly(config.static_config, config.lai_year, "LAI_pfts", month)?;
    let mut total_sai = read_pft_monthly(config.static_config, config.lai_year, "SAI_pfts", month)?;
    ensure!(
        total_lai.len() == pft_count && total_sai.len() == pft_count,
        "spatial PFT monthly vegetation has inconsistent vector lengths"
    );
    if crop.is_some() {
        for pft in 0..pft_count {
            if pfts.class[pft] >= 15 {
                total_lai[pft] = 0.0;
                total_sai[pft] = 0.0;
            }
        }
    }
    let canopy = pft_canopy(&document, &pfts.class, &pfts.observed_height_m)?;
    let common_state = read_common_state(&common.block, patches.class.len())?;
    let top_soil_thickness_m = crate::colm_soil_grid(10)?.thickness_m[0];
    let albedo = [
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_s_v_alb_patches",
            "soil_s_v_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_d_v_alb_patches",
            "soil_d_v_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_s_n_alb_patches",
            "soil_s_n_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_d_n_alb_patches",
            "soil_d_n_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
    ];
    let (
        high_resolution_ground,
        high_resolution_fractions,
        mut high_resolution_albedo,
        mut high_resolution_reflectance,
        mut high_resolution_transmittance,
    ) = if let Some((_, water, radiation)) = high_resolution_sources.as_ref() {
        let urban = high_resolution_urban_albedo
            .as_ref()
            .expect("hyperspectral sources include urban albedo");
        let (longitude, latitude) = patch_coordinates(
            config.static_config.landdata,
            config.static_config.land_cover_year,
            config.static_config.block_label,
            &patches,
        )?;
        let dry_soil_albedo = high_resolution_soil
            .then(|| {
                read_hyperspectral_albedo(
                    config.static_config.landdata,
                    config.static_config.land_cover_year,
                    config.static_config.block_label,
                    patches.class.len(),
                    HIGH_RES_WAVELENGTHS,
                )
            })
            .transpose()?;
        let mut ground = Vec::with_capacity(patches.class.len());
        let mut fractions = Vec::with_capacity(patches.class.len());
        let mut hires_albedo = vec![0.0; HIGH_RES_WAVELENGTHS * 2 * patches.class.len()];
        for patch in 0..patches.class.len() {
            let broadband_ground = cold_start_ground_albedo(
                patch_kind[patch],
                SoilReflectance {
                    saturated_visible: albedo[0][patch],
                    dry_visible: albedo[1][patch],
                    saturated_near_infrared: albedo[2][patch],
                    dry_near_infrared: albedo[3][patch],
                },
                common_state.top_liquid_kg_m2[patch],
                top_soil_thickness_m,
                common_state.cosine_zenith[patch].max(0.001),
                0.0,
                0.0,
                common_state.ground_temperature_k[patch],
            )?;
            let mut spectral_ground = if patch_kind[patch] == 1 {
                urban
                    .spectrum(
                        1,
                        latitude[patch].to_degrees(),
                        longitude[patch].to_degrees(),
                    )
                    .iter()
                    .flat_map(|&albedo| [albedo, albedo])
                    .collect()
            } else {
                expand_broadband_ground_albedo(broadband_ground.ground)
            };
            if high_resolution_soil && patch_kind[patch] == 0 {
                let dry = (0..HIGH_RES_WAVELENGTHS)
                    .map(|wavelength| {
                        dry_soil_albedo
                            .as_ref()
                            .expect("high-resolution soil source is loaded")
                            [wavelength * patches.class.len() + patch]
                    })
                    .collect::<Vec<_>>();
                if dry[0] >= 0.01 {
                    let water = water
                        .as_ref()
                        .expect("high-resolution water optics are loaded");
                    spectral_ground = bsm_soil_moisture(
                        (1.0e-3 * common_state.top_liquid_kg_m2[patch] / top_soil_thickness_m)
                            .min(1.0)
                            * 100.0,
                        // Upstream mkinidata passes a fixed `porsl = 0.8` to BSM here.
                        80.0,
                        &dry,
                        &water.absorption,
                        &water.refractive_index,
                    )?;
                }
            }
            for wavelength in 0..HIGH_RES_WAVELENGTHS {
                for radiation_type in 0..2 {
                    hires_albedo[(wavelength * 2 + radiation_type) * patches.class.len() + patch] =
                        spectral_ground[wavelength * 2 + radiation_type];
                }
            }
            ground.push(spectral_ground);
            fractions.push(
                radiation
                    .as_ref()
                    .map(|radiation| {
                        select_high_resolution_radiation(
                            CalendarTime {
                                year: config.date.year,
                                julian_day: config.date.julian_day,
                                seconds: config.date.seconds,
                            },
                            config.greenwich,
                            longitude[patch].to_degrees(),
                            common_state.cosine_zenith[patch],
                            latitude[patch],
                            radiation.tables(),
                        )
                    })
                    .transpose()?,
            );
        }
        (
            ground,
            fractions,
            hires_albedo,
            vec![-999.0; HIGH_RES_WAVELENGTHS * 16 * patches.class.len()],
            vec![-999.0; HIGH_RES_WAVELENGTHS * 16 * patches.class.len()],
        )
    } else {
        (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new())
    };

    let mut leaf_temperature = vec![0.0; pft_count];
    let zero = vec![0.0; pft_count];
    let one = vec![1.0; pft_count];
    let mut roughness = vec![0.0; pft_count];
    let mut sunlit = vec![0.0; 4 * pft_count];
    let mut shaded = sunlit.clone();
    let mut thermal_gap = vec![MISSING; pft_count];
    let mut shade = vec![MISSING; pft_count];
    let mut direct_extinction = vec![1.0; pft_count];
    let mut diffuse_extinction = vec![0.718; pft_count];
    let mut state_by_pft = vec![None; pft_count];
    let mut high_resolution_by_pft = vec![None; pft_count];
    let mut high_resolution_sunlit = vec![0.0; HIGH_RES_WAVELENGTHS * 2 * pft_count];
    let mut high_resolution_shaded = high_resolution_sunlit.clone();
    for patch in 0..patches.class.len() {
        let indices = &pft_to_patch[patch];
        if patch_kind[patch] != 0 {
            ensure!(
                indices.is_empty(),
                "non-natural spatial patch {patch} unexpectedly owns PFT entries"
            );
            continue;
        }
        ensure!(
            !indices.is_empty(),
            "natural spatial patch {patch} has no PFT entries"
        );
        let fraction_sum = indices
            .iter()
            .map(|&index| pfts.fraction[index])
            .sum::<f64>();
        ensure!(
            fraction_sum.is_finite() && (fraction_sum - 1.0).abs() <= 1.0e-8,
            "PFT fractions for natural spatial patch {patch} must sum to one, got {fraction_sum}"
        );
        for &pft in indices {
            let broadband_optics = pft_leaf_optics(
                &document,
                pfts.class[pft],
                hydraulic_model,
                subgrid == SpatialPftSubgrid::Pc,
            )?;
            let mut state = cold_start_pft_broadband_radiation_with_snow(
                patch_kind[patch],
                SoilReflectance {
                    saturated_visible: albedo[0][patch],
                    dry_visible: albedo[1][patch],
                    saturated_near_infrared: albedo[2][patch],
                    dry_near_infrared: albedo[3][patch],
                },
                common_state.top_liquid_kg_m2[patch],
                top_soil_thickness_m,
                broadband_optics,
                total_lai[pft],
                total_sai[pft],
                0.0,
                common_state.cosine_zenith[patch].max(0.001),
                config.vegetation_snow,
                0.0,
                0.0,
                common_state.ground_temperature_k[patch],
            )?;
            if high_resolution_canopy {
                let fallback = expand_broadband_leaf_optics(broadband_optics);
                let class = usize::try_from(pfts.class[pft])
                    .context("high-resolution PFT class must be nonnegative")?;
                let source_optics = if high_resolution_vegetation {
                    high_resolution_sources
                        .as_ref()
                        .expect("high-resolution sources are loaded")
                        .0
                        .as_ref()
                        .expect("high-resolution leaf optics are loaded")
                        .optics(class)?
                } else {
                    HighResolutionLeafOptics {
                        reflectance: &fallback.0,
                        transmittance: &fallback.1,
                    }
                };
                // CoLMMAIN sets ssw from the upper liquid layer, capped at saturation,
                // before MOD_Albedo_HiRes calls update_params_PROSPECT.
                let prospect = use_prospect
                    .then(|| {
                        prospect_leaf_optics(
                            class,
                            (1.0e-3 * common_state.top_liquid_kg_m2[patch] / top_soil_thickness_m)
                                .min(1.0),
                            source_optics,
                        )
                    })
                    .transpose()?;
                let optics =
                    prospect
                        .as_ref()
                        .map_or(source_optics, |optics| HighResolutionLeafOptics {
                            reflectance: &optics.reflectance,
                            transmittance: &optics.transmittance,
                        });
                let radiation = (total_lai[pft] + total_sai[pft] > 1.0e-6)
                    .then(|| {
                        pft_high_resolution_radiation(
                            broadband_optics.chil,
                            optics,
                            total_lai[pft],
                            total_sai[pft],
                            0.0,
                            common_state.cosine_zenith[patch].max(0.001),
                            &high_resolution_ground[patch],
                            config.vegetation_snow,
                        )
                    })
                    .transpose()?;
                state = high_resolution_pft_cold_start_state(
                    radiation.as_ref(),
                    &high_resolution_ground[patch],
                    high_resolution_fractions[patch]
                        .as_ref()
                        .expect("PFT high-resolution radiation fractions are loaded"),
                )?;
                if let Some(radiation) = radiation.as_ref() {
                    copy_high_resolution_pft_radiation(
                        &mut high_resolution_sunlit,
                        pft_count,
                        pft,
                        &radiation.sunlit_absorption,
                    )?;
                    copy_high_resolution_pft_radiation(
                        &mut high_resolution_shaded,
                        pft_count,
                        pft,
                        &radiation.shaded_absorption,
                    )?;
                    for wavelength in 0..HIGH_RES_WAVELENGTHS {
                        let index = (wavelength * 16 + class) * patches.class.len() + patch;
                        high_resolution_reflectance[index] = optics.reflectance[wavelength * 2];
                        high_resolution_transmittance[index] = optics.transmittance[wavelength * 2];
                    }
                }
                high_resolution_by_pft[pft] = radiation;
            }
            leaf_temperature[pft] = common_state.ground_temperature_k[patch];
            roughness[pft] = canopy.top_m[pft] * 0.1;
            thermal_gap[pft] = state.thermal_gap_fraction;
            direct_extinction[pft] = state.direct_extinction;
            diffuse_extinction[pft] = state.diffuse_extinction;
            copy_pft_radiation(&mut sunlit, pft_count, pft, state.sunlit_absorption);
            copy_pft_radiation(&mut shaded, pft_count, pft, state.shaded_absorption);
            state_by_pft[pft] = Some(state);
        }
    }
    if subgrid == SpatialPftSubgrid::Pc {
        let pc_crop_split = optional_bool_or(&document, "DEF_PC_CROP_SPLIT", true)?;
        for patch in 0..patches.class.len() {
            let indices = pft_to_patch[patch]
                .iter()
                .copied()
                .take_while(|&pft| pc_uses_three_dimensional_canopy(pfts.class[pft], pc_crop_split))
                .collect::<Vec<_>>();
            if indices.is_empty() {
                continue;
            }
            let inputs = indices
                .iter()
                .map(|&pft| {
                    Ok(PcPftInput {
                        canopy_layer: pc_canopy_layer(pfts.class[pft])?,
                        fraction: pfts.fraction[pft],
                        canopy_top_m: canopy.top_m[pft],
                        canopy_bottom_m: canopy.bottom_m[pft],
                        optics: pft_leaf_optics(
                            &document,
                            pfts.class[pft],
                            hydraulic_model,
                            subgrid == SpatialPftSubgrid::Pc,
                        )?,
                        lai: total_lai[pft],
                        sai: total_sai[pft],
                        wet_snow_fraction: 0.0,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let pc = if config.use_hyperspectral {
                let high_resolution = high_resolution_pft_cold_start_state(
                    None,
                    &high_resolution_ground[patch],
                    high_resolution_fractions[patch]
                        .as_ref()
                        .expect("hyperspectral radiation fractions are loaded"),
                )?;
                let mut pc = cold_start_pc_broadband_radiation_from_ground(
                    &inputs,
                    common_state.cosine_zenith[patch].max(0.001),
                    high_resolution.albedo,
                    [[1.0; 2]; 2],
                    high_resolution.albedo,
                    high_resolution.snow_age,
                )?;
                // MOD_Albedo_HiRes retains its default high-resolution
                // transmission when PC's broadband ThreeDCanopy solver runs.
                pc.common.soil_absorption = high_resolution.soil_absorption;
                pc.common.snow_absorption = high_resolution.snow_absorption;
                pc
            } else {
                cold_start_pc_broadband_radiation_with_snow(
                    patch_kind[patch],
                    crate::SoilReflectance {
                        saturated_visible: albedo[0][patch],
                        dry_visible: albedo[1][patch],
                        saturated_near_infrared: albedo[2][patch],
                        dry_near_infrared: albedo[3][patch],
                    },
                    common_state.top_liquid_kg_m2[patch],
                    top_soil_thickness_m,
                    &inputs,
                    common_state.cosine_zenith[patch].max(0.001),
                    0.0,
                    0.0,
                    common_state.ground_temperature_k[patch],
                )?
            };
            for (pc_index, &pft) in indices.iter().enumerate() {
                let state = &pc.pft[pc_index];
                copy_pft_radiation(&mut sunlit, pft_count, pft, state.sunlit_absorption);
                copy_pft_radiation(&mut shaded, pft_count, pft, state.shaded_absorption);
                thermal_gap[pft] = state.thermal_gap_fraction;
                shade[pft] = state.shade_fraction;
                direct_extinction[pft] = state.direct_extinction;
                diffuse_extinction[pft] = state.diffuse_extinction;
                // twostream_wrap retains per-PFT absorption when sharing PC optics.
                state_by_pft[pft] = Some(ColdStartRadiation {
                    sunlit_absorption: state.sunlit_absorption,
                    shaded_absorption: state.shaded_absorption,
                    ..pc.common.clone()
                });
            }
        }
    }
    let mut common_radiation = vec![None; patches.class.len()];
    let mut common_roughness = vec![None; patches.class.len()];
    let mut common_sai = vec![None; patches.class.len()];
    for (patch, indices) in pft_to_patch.iter().enumerate() {
        if patch_kind[patch] != 0 {
            continue;
        }
        let fractions = indices
            .iter()
            .map(|&index| pfts.fraction[index])
            .collect::<Vec<_>>();
        let states = indices
            .iter()
            .map(|&pft| {
                state_by_pft[pft]
                    .clone()
                    .with_context(|| format!("spatial PFT {pft} has no cold-start radiation"))
            })
            .collect::<Result<Vec<_>>>()?;
        let leaf_stem_area = indices
            .iter()
            .map(|&index| (total_lai[index] + total_sai[index]) * pfts.fraction[index])
            .sum();
        let ground = (!config.use_hyperspectral)
            .then(|| {
                cold_start_ground_albedo(
                    patch_kind[patch],
                    SoilReflectance {
                        saturated_visible: albedo[0][patch],
                        dry_visible: albedo[1][patch],
                        saturated_near_infrared: albedo[2][patch],
                        dry_near_infrared: albedo[3][patch],
                    },
                    common_state.top_liquid_kg_m2[patch],
                    top_soil_thickness_m,
                    common_state.cosine_zenith[patch].max(0.001),
                    0.0,
                    0.0,
                    common_state.ground_temperature_k[patch],
                )
            })
            .transpose()?;
        let mut radiation =
            aggregate_pft_radiation(&states, &fractions, leaf_stem_area, ground.as_ref())?;
        if high_resolution_canopy {
            // `albland_HiRes` stores PFT absorption in landpft, not landpatch.
            radiation.sunlit_absorption = [[0.0; 2]; 2];
            radiation.shaded_absorption = [[0.0; 2]; 2];
        }
        common_radiation[patch] = Some(radiation);
        // MOD_IniTimeVariable scales the aggregated HTOP, not each PFT's z0m.
        common_roughness[patch] = Some(
            indices.iter().fold(0.0, |sum, &pft| {
                canopy.top_m[pft].mul_add(pfts.fraction[pft], sum)
            }) * 0.1,
        );
        common_sai[patch] = Some(indices.iter().fold(0.0, |sum, &pft| {
            total_sai[pft].mul_add(pfts.fraction[pft], sum)
        }));
        if high_resolution_canopy {
            for wavelength in 0..HIGH_RES_WAVELENGTHS {
                for radiation_type in 0..2 {
                    high_resolution_albedo
                        [(wavelength * 2 + radiation_type) * patches.class.len() + patch] = indices
                        .iter()
                        .map(|&pft| {
                            high_resolution_by_pft[pft].as_ref().map_or(
                                high_resolution_ground[patch][wavelength * 2 + radiation_type],
                                |radiation| radiation.albedo[wavelength * 2 + radiation_type],
                            ) * pfts.fraction[pft]
                        })
                        .sum();
                }
            }
        }
    }

    update_common_pft_optics(
        &common.block,
        &common_radiation,
        &common_roughness,
        &common_sai,
    )?;
    if crop.is_some() {
        let crop_patch = pft_to_patch
            .iter()
            .map(|indices| indices.iter().any(|&pft| pfts.class[pft] >= 15))
            .collect::<Vec<_>>();
        zero_common_crop_vegetation(&common.block, &crop_patch)?;
    }
    if config.use_hyperspectral {
        append_time_hyperspectral_fields(
            &common.block,
            2,
            TimeHyperspectralFields {
                albedo: &high_resolution_albedo,
                reflectance: &high_resolution_reflectance,
                transmittance: &high_resolution_transmittance,
            },
            compression_level,
        )?;
    }
    let reference_humidity = vec![0.3; pft_count];
    let missing = vec![MISSING; pft_count];
    let plant_water = vec![-25_000.0; 4 * pft_count];
    let conductance = vec![10_000.0; pft_count];
    let ozone_zero = vec![0.0; pft_count];
    let pft = write_pft_time_restart(
        config.static_config.restart_dir,
        config.static_config.case_name,
        config.static_config.land_cover_year,
        config.date,
        config.static_config.block_label,
        PftTimeRestartInput {
            compression_level,
            fields: PftTimeFields {
                leaf_temperature_k: &leaf_temperature,
                canopy_water_mm: &zero,
                canopy_rain_mm: &zero,
                canopy_snow_mm: &zero,
                wet_snow_fraction: &zero,
                vegetation_fraction: &one,
                total_lai: &total_lai,
                lai: &total_lai,
                total_sai: &total_sai,
                sai: &total_sai,
                sunlit_absorption: &sunlit,
                shaded_absorption: &shaded,
                thermal_gap_fraction: &thermal_gap,
                shade_fraction: &shade,
                direct_extinction: &direct_extinction,
                diffuse_extinction: &diffuse_extinction,
                reference_temperature_k: &leaf_temperature,
                reference_humidity: &reference_humidity,
                stomatal_resistance_s_m: &missing,
                roughness_length_m: &roughness,
            },
            hyperspectral: config.use_hyperspectral.then_some(PftHyperspectralFields {
                sunlit_absorption: &high_resolution_sunlit,
                shaded_absorption: &high_resolution_shaded,
            }),
            plant_hydraulics: config.plant_hydraulics.then_some(PftPlantHydraulicFields {
                water_potential_mm: &plant_water,
                sunlit_stomatal_conductance: &conductance,
                shaded_stomatal_conductance: &conductance,
                vegetation_nodes: 4,
            }),
            bgc: bgc_state
                .as_ref()
                .zip(bgc_pft_values.as_deref())
                .map(|(state, values)| PftBgcFields {
                    values,
                    active_crop_years: &state.active_crop_years,
                }),
            crop: crop.as_ref().map(crate::CropColdStartState::pft_fields),
            ozone: config.ozone_stress.then_some(PftOzoneFields {
                lai_old: &total_lai,
                sunlit_uptake: &ozone_zero,
                shaded_uptake: &ozone_zero,
                sunlit_vegetation_coefficient: &one,
                shaded_vegetation_coefficient: &one,
                sunlit_stomatal_coefficient: &one,
                shaded_stomatal_coefficient: &one,
            }),
            irrigation_method: crop
                .as_ref()
                .and_then(crate::CropColdStartState::irrigation_method),
        },
    )?;
    let bgc = bgc_state
        .as_ref()
        .map(|state| {
            let mut input = bgc_time_restart_input(state, compression_level);
            input.crop = crop.as_ref().map(crate::CropColdStartState::bgc_fields);
            write_bgc_time_restart(
                config.static_config.restart_dir,
                config.static_config.case_name,
                config.static_config.land_cover_year,
                config.date,
                config.static_config.block_label,
                input,
            )
        })
        .transpose()?;
    Ok(SpatialPftTimeRestartFiles { common, pft, bgc })
}

#[derive(Debug)]
struct SpatialPftVectors {
    class: Vec<i32>,
    fraction: Vec<f64>,
    observed_height_m: Vec<f64>,
    element: Vec<i64>,
    start: Vec<i32>,
    end: Vec<i32>,
    shared_fraction: Vec<f64>,
}

#[derive(Debug)]
struct CommonColdState {
    ground_temperature_k: Vec<f64>,
    top_liquid_kg_m2: Vec<f64>,
    cosine_zenith: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpatialPftSubgrid {
    Pft,
    Pc,
}

fn spatial_crop_state(
    document: &colm_namelist::Document,
    config: SpatialPftStaticConfig<'_>,
    patches: &crate::spatial_static::Patches,
    pfts: &SpatialPftVectors,
    pft_owner: &[usize],
) -> Result<crate::CropColdStartState> {
    let planting_day = document
        .get("DEF_TUNING_CROP_PLANTING_DAY")
        .map(|value| {
            value
                .as_f64()
                .context("DEF_TUNING_CROP_PLANTING_DAY must be a real value")
        })
        .transpose()?;
    ensure!(
        planting_day.is_none_or(f64::is_finite),
        "DEF_TUNING_CROP_PLANTING_DAY must be finite"
    );
    let planting_day = planting_day.filter(|value| *value > 0.0);
    let use_fertilizer = optional_bool_or(document, "DEF_USE_FERT", true)?;
    let use_irrigation = optional_bool_or(document, "DEF_USE_IRRIGATION", false)?;
    if !use_fertilizer && !use_irrigation {
        if let Some(planting_day) = planting_day {
            return crate::crop::spatial_crop_cold_start_from_tuning(
                &pfts.class,
                pft_owner,
                &pfts.fraction,
                patches.class.len(),
                planting_day,
            );
        }
    }
    let pft_pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &pfts.element,
        &pfts.start,
        &pfts.end,
        &pfts.shared_fraction,
        "landpft",
    )?;
    let patch_pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches.element,
        &patches.start,
        &patches.end,
        &patches.shared_fraction,
        "landpatch",
    )?;
    let runtime_dir = std::path::PathBuf::from(required_string(document, "DEF_dir_runtime")?);
    crate::crop::spatial_crop_cold_start_from_management(
        &pfts.class,
        pft_owner,
        &pfts.fraction,
        patches.class.len(),
        &pft_pixels,
        &patch_pixels,
        crate::CropManagementConfig {
            runtime_dir: &runtime_dir,
            planting_day_override: planting_day,
            use_fertilizer,
            fertilizer_source: optional_i32(document, "DEF_FERT_SOURCE")?.unwrap_or(1),
            use_irrigation,
            use_irrigation_allocation: use_irrigation
                && optional_i32(document, "DEF_IRRIGATION_ALLOCATION")? == Some(3),
        },
    )
}

fn pft_owners(pft_to_patch: &[Vec<usize>], pfts: usize) -> Result<Vec<usize>> {
    let mut owner = vec![None; pfts];
    for (patch, indices) in pft_to_patch.iter().enumerate() {
        for &pft in indices {
            ensure!(
                pft < pfts && owner[pft].replace(patch).is_none(),
                "spatial PFT topology assigns a PFT to multiple landpatches"
            );
        }
    }
    owner
        .into_iter()
        .enumerate()
        .map(|(pft, patch)| patch.with_context(|| format!("spatial PFT {pft} has no landpatch")))
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn derive_spatial_bgc_state(
    document: &colm_namelist::Document,
    config: SpatialPftStaticConfig<'_>,
    hydraulic_model: HydraulicModel,
    patches: &crate::spatial_static::Patches,
    patch_kind: &[i32],
    pfts: &SpatialPftVectors,
    pft_to_patch: &[Vec<usize>],
    cn_initial_state: Option<&Path>,
    use_nitrification: bool,
) -> Result<crate::BgcColdStartState> {
    ensure!(
        patches.class.len() == patch_kind.len() && patch_kind.len() == pft_to_patch.len(),
        "spatial BGC patch topology is inconsistent"
    );
    let pft_order = pft_to_patch.iter().flatten().copied().collect::<Vec<_>>();
    ensure!(
        pft_order.iter().copied().eq(0..pfts.class.len()),
        "spatial landpft entries must be ordered by their owning landpatch"
    );

    let campbell = hydraulic_model == HydraulicModel::Campbell;
    let leaf_carbon_to_nitrogen =
        pft_parameters(document, "DEF_PFT_LEAFCN", &pfts.class, campbell)?;
    let fine_root_carbon_to_nitrogen =
        pft_parameters(document, "DEF_PFT_FROOTCN", &pfts.class, campbell)?;
    let live_wood_carbon_to_nitrogen =
        pft_parameters(document, "DEF_PFT_LIVEWDCN", &pfts.class, campbell)?;
    let dead_wood_carbon_to_nitrogen =
        pft_parameters(document, "DEF_PFT_DEADWDCN", &pfts.class, campbell)?;
    let soil = crate::derive_spatial_soil_parameters(
        &read_soil(
            config.landdata,
            config.land_cover_year,
            config.block_label,
            patches.class.len(),
        )?,
        &patches.class,
        patch_kind,
        10,
        hydraulic_model,
    )?;
    let soil_thickness_m = crate::colm_soil_grid(10)?.thickness_m;
    let equilibrium = cn_initial_state
        .map(|path| read_spatial_cn_equilibrium(path, config, patches, pfts))
        .transpose()?;
    let states = (0..patches.class.len())
        .map(|patch| {
            let indices = &pft_to_patch[patch];
            ensure!(
                patch_kind[patch] == 0 || indices.is_empty(),
                "non-natural spatial patch {patch} unexpectedly owns PFT entries"
            );
            let class = indices
                .iter()
                .map(|&index| pfts.class[index])
                .collect::<Vec<_>>();
            let fraction = indices
                .iter()
                .map(|&index| pfts.fraction[index])
                .collect::<Vec<_>>();
            let leaf_cn = indices
                .iter()
                .map(|&index| leaf_carbon_to_nitrogen[index])
                .collect::<Vec<_>>();
            let root_cn = indices
                .iter()
                .map(|&index| fine_root_carbon_to_nitrogen[index])
                .collect::<Vec<_>>();
            let live_wood_cn = indices
                .iter()
                .map(|&index| live_wood_carbon_to_nitrogen[index])
                .collect::<Vec<_>>();
            let dead_wood_cn = indices
                .iter()
                .map(|&index| dead_wood_carbon_to_nitrogen[index])
                .collect::<Vec<_>>();
            let vegetation = equilibrium.as_ref().map(|state| {
                indices
                    .iter()
                    .map(|&index| state.vegetation[index].clone())
                    .collect::<Vec<_>>()
            });
            let soil_bulk_density_kg_m3 = (0..soil.layers)
                .map(|layer| soil.get(crate::SoilField::BulkDensity, layer, patch))
                .collect::<Vec<_>>();
            derive_cold_start_bgc_state(BgcColdStartInput {
                soil_thickness_m: &soil_thickness_m,
                soil_bulk_density_kg_m3: &soil_bulk_density_kg_m3,
                pft: BgcPftColdStartInput {
                    class: &class,
                    fraction: &fraction,
                    leaf_carbon_to_nitrogen: &leaf_cn,
                    fine_root_carbon_to_nitrogen: &root_cn,
                    live_wood_carbon_to_nitrogen: &live_wood_cn,
                    dead_wood_carbon_to_nitrogen: &dead_wood_cn,
                },
                runtime_cn_state: equilibrium.as_ref().map(|state| &state.patch[patch]),
                runtime_vegetation_carbon: vegetation.as_deref(),
                use_nitrification,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    merge_bgc_cold_start_states(&states)
}

struct SpatialCnEquilibrium {
    patch: Vec<RuntimeCnState>,
    vegetation: Vec<RuntimeCnVegetationCarbon>,
}

fn read_spatial_cn_equilibrium(
    path: &Path,
    config: SpatialPftStaticConfig<'_>,
    patches: &crate::spatial_static::Patches,
    pfts: &SpatialPftVectors,
) -> Result<SpatialCnEquilibrium> {
    const CARBON: [&str; 7] = [
        "litr1c_vr",
        "litr2c_vr",
        "litr3c_vr",
        "cwdc_vr",
        "soil1c_vr",
        "soil2c_vr",
        "soil3c_vr",
    ];
    const NITROGEN: [&str; 7] = [
        "litr1n_vr",
        "litr2n_vr",
        "litr3n_vr",
        "cwdn_vr",
        "soil1n_vr",
        "soil2n_vr",
        "soil3n_vr",
    ];
    let file = netcdf::open(path).with_context(|| {
        format!(
            "cannot open spatial BGC equilibrium state {}",
            path.display()
        )
    })?;
    let grid = MapGrid::from_file(&file)?;
    let patch_pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches.element,
        &patches.start,
        &patches.end,
        &patches.shared_fraction,
        "landpatch",
    )?;
    let pft_pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &pfts.element,
        &pfts.start,
        &pfts.end,
        &pfts.shared_fraction,
        "landpft",
    )?;
    let patch_mapping = AreaMapping::new(&grid, &patch_pixels)?;
    let pft_mapping = AreaMapping::new(&grid, &pft_pixels)?;
    let carbon = map_cn_profiles(&file, &grid, &patch_mapping, &CARBON)?;
    let nitrogen = map_cn_profiles(&file, &grid, &patch_mapping, &NITROGEN)?;
    let ammonium = map_cn_profile(&file, &grid, &patch_mapping, "smin_nh4_vr")?;
    let nitrate = map_cn_profile(&file, &grid, &patch_mapping, "smin_no3_vr")?;
    let vegetation = [
        map_cn_scalar(&file, &grid, &pft_mapping, "leafc")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "leafc_storage")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "frootc")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "frootc_storage")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "livestemc")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "deadstemc")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "livecrootc")?,
        map_cn_scalar(&file, &grid, &pft_mapping, "deadcrootc")?,
    ];
    let zero_vegetation = || RuntimeCnVegetationCarbon {
        leaf_g_m2: 0.0,
        leaf_storage_g_m2: 0.0,
        fine_root_g_m2: 0.0,
        fine_root_storage_g_m2: 0.0,
        live_stem_g_m2: 0.0,
        dead_stem_g_m2: 0.0,
        live_coarse_root_g_m2: 0.0,
        dead_coarse_root_g_m2: 0.0,
    };
    Ok(SpatialCnEquilibrium {
        patch: (0..patches.class.len())
            .map(|patch| RuntimeCnState {
                decomposition_carbon_g_m3: carbon[patch].clone(),
                decomposition_nitrogen_g_m3: nitrogen[patch].clone(),
                ammonium_g_m3: ammonium[patch].clone(),
                nitrate_g_m3: nitrate[patch].clone(),
                vegetation_carbon: zero_vegetation(),
            })
            .collect(),
        vegetation: (0..pfts.class.len())
            .map(|pft| RuntimeCnVegetationCarbon {
                leaf_g_m2: vegetation[0][pft],
                leaf_storage_g_m2: vegetation[1][pft],
                fine_root_g_m2: vegetation[2][pft],
                fine_root_storage_g_m2: vegetation[3][pft],
                live_stem_g_m2: vegetation[4][pft],
                dead_stem_g_m2: vegetation[5][pft],
                live_coarse_root_g_m2: vegetation[6][pft],
                dead_coarse_root_g_m2: vegetation[7][pft],
            })
            .collect(),
    })
}

fn map_cn_profiles(
    file: &netcdf::File,
    grid: &MapGrid,
    mapping: &AreaMapping,
    names: &[&str],
) -> Result<Vec<Vec<f64>>> {
    let mut values = vec![vec![0.0; names.len() * 10]; mapping.len()];
    for (pool, &name) in names.iter().enumerate() {
        for soil in 0..10 {
            let mapped =
                mapping.average_required(&map_field_soil_3d(file, name, soil, grid)?, name)?;
            for (patch, value) in mapped.into_iter().enumerate() {
                values[patch][pool * 10 + soil] = value;
            }
        }
    }
    Ok(values)
}

fn map_cn_profile(
    file: &netcdf::File,
    grid: &MapGrid,
    mapping: &AreaMapping,
    name: &str,
) -> Result<Vec<Vec<f64>>> {
    (0..10)
        .map(|soil| mapping.average_required(&map_field_soil_3d(file, name, soil, grid)?, name))
        .collect::<Result<Vec<_>>>()
        .map(|layers| {
            (0..layers[0].len())
                .map(|patch| layers.iter().map(|layer| layer[patch]).collect())
                .collect()
        })
}

fn map_cn_scalar(
    file: &netcdf::File,
    grid: &MapGrid,
    mapping: &AreaMapping,
    name: &str,
) -> Result<Vec<f64>> {
    mapping.average_required(&map_field_2d(file, name, grid)?, name)
}

fn read_pft_vectors(config: SpatialPftStaticConfig<'_>) -> Result<SpatialPftVectors> {
    let path = block_path(
        config.landdata,
        "landpft",
        "landpft",
        config.land_cover_year,
        config.block_label,
    );
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let class = values_i32(&file, "settyp")?;
    let element = file
        .variable("eindex")
        .context("spatial landpft has no eindex")?
        .get_values::<i64, _>(..)?;
    let start = values_i32(&file, "ipxstt")?;
    let end = values_i32(&file, "ipxend")?;
    let shared_fraction = match file.variable("pctshared") {
        Some(variable) => variable
            .get_values::<f64, _>(..)
            .context("cannot read spatial landpft sharing fractions")?,
        None => vec![1.0; class.len()],
    };
    let fraction = read_f64(
        config.landdata,
        "pctpft",
        "pct_pfts",
        "pct_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    let observed_height_m = read_f64(
        config.landdata,
        "htop",
        "htop_pfts",
        "htop_pfts",
        config.land_cover_year,
        config.block_label,
    )?;
    let count = class.len();
    ensure!(
        count > 0
            && fraction.len() == count
            && observed_height_m.len() == count
            && element.len() == count
            && start.len() == count
            && end.len() == count
            && shared_fraction.len() == count
            && shared_fraction
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0)
            && fraction.iter().all(|value| value.is_finite() && *value >= 0.0),
        "spatial PFT topology, fraction, and height vectors must be finite and have equal nonzero lengths"
    );
    Ok(SpatialPftVectors {
        class,
        fraction,
        observed_height_m,
        element,
        start,
        end,
        shared_fraction,
    })
}

fn match_pfts_to_patches(
    patches: &crate::spatial_static::Patches,
    patch_kind: &[i32],
    pfts: &SpatialPftVectors,
    use_crop: bool,
) -> Result<Vec<Vec<usize>>> {
    ensure!(
        patches.class.len() == patch_kind.len(),
        "spatial patch kind vector does not match topology"
    );
    // MOD_LandPFT emits PFTs in patch order. Shared natural/CFT patches may
    // have identical ranges, so a unique (element,start,end) map loses owners.
    let mut out = vec![Vec::new(); patches.class.len()];
    let mut next = 0;
    for (patch, indices) in out.iter_mut().enumerate() {
        if patch_kind[patch] != 0 {
            continue;
        }
        let crop = use_crop && patches.class[patch] == 12;
        while next < pfts.class.len()
            && pfts.element[next] == patches.element[patch]
            && pfts.start[next] == patches.start[patch]
            && pfts.end[next] == patches.end[patch]
            && (!use_crop || (pfts.class[next] >= 15) == crop)
        {
            indices.push(next);
            next += 1;
            if crop {
                break;
            } // one PFT for each CFT child, even with shared pixels
        }
        ensure!(
            !indices.is_empty(),
            "natural/CFT patch {patch} has no matching ordered PFT"
        );
    }
    ensure!(
        next == pfts.class.len(),
        "PFT {next} does not match ordered landpatch topology"
    );
    Ok(out)
}

fn read_pft_monthly(
    config: SpatialPftStaticConfig<'_>,
    year: i32,
    variable: &str,
    month: u8,
) -> Result<Vec<f64>> {
    let stem = format!("{variable}{month:02}");
    let values = read_f64(
        config.landdata,
        "LAI",
        &stem,
        variable,
        year,
        config.block_label,
    )?;
    ensure!(
        values
            .iter()
            .all(|value| value.is_finite() && *value >= 0.0),
        "{variable} must contain finite nonnegative PFT values"
    );
    Ok(values)
}

fn read_common_state(path: &Path, patches: usize) -> Result<CommonColdState> {
    let file = netcdf::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let ground_temperature_k = values_f64(&file, "t_grnd")?;
    let cosine_zenith = values_f64(&file, "coszen")?;
    let liquid = values_f64(&file, "wliq_soisno")?;
    const SNOW_LAYERS: usize = 5;
    const SOIL_LAYERS: usize = 10;
    ensure!(
        ground_temperature_k.len() == patches
            && cosine_zenith.len() == patches
            && liquid.len() == patches * (SNOW_LAYERS + SOIL_LAYERS),
        "common spatial time restart has an unexpected soil/snow layout"
    );
    Ok(CommonColdState {
        ground_temperature_k,
        top_liquid_kg_m2: (0..patches)
            .map(|patch| liquid[patch * (SNOW_LAYERS + SOIL_LAYERS) + SNOW_LAYERS])
            .collect(),
        cosine_zenith,
    })
}

fn copy_pft_radiation(target: &mut [f64], pfts: usize, pft: usize, values: [[f64; 2]; 2]) {
    for band in 0..2 {
        for radiation_type in 0..2 {
            target[(band * 2 + radiation_type) * pfts + pft] = values[band][radiation_type];
        }
    }
}

fn copy_high_resolution_pft_radiation(
    target: &mut [f64],
    pfts: usize,
    pft: usize,
    values: &[f64],
) -> Result<()> {
    ensure!(
        values.len() == HIGH_RES_WAVELENGTHS * 2,
        "high-resolution PFT radiation must contain 211 wavelengths and two radiation types"
    );
    for wavelength in 0..HIGH_RES_WAVELENGTHS {
        for radiation_type in 0..2 {
            target[(wavelength * 2 + radiation_type) * pfts + pft] =
                values[wavelength * 2 + radiation_type];
        }
    }
    Ok(())
}

fn update_common_pft_optics(
    path: &Path,
    states: &[Option<ColdStartRadiation>],
    roughness: &[Option<f64>],
    stem_area: &[Option<f64>],
) -> Result<()> {
    ensure!(
        states.len() == roughness.len() && states.len() == stem_area.len(),
        "common PFT radiation, roughness and stem-area vectors must align"
    );
    let patches = states.len();
    let mut file = netcdf::append(path)
        .with_context(|| format!("cannot update common PFT restart {}", path.display()))?;
    for name in ["alb", "ssun", "ssha", "ssoi", "ssno"] {
        let mut values = file
            .variable(name)
            .with_context(|| format!("common restart has no {name}"))?
            .get_values::<f64, _>(..)?;
        ensure!(
            values.len() == 4 * patches,
            "common restart {name} has an unexpected radiation layout"
        );
        for (patch, state) in states.iter().enumerate() {
            let Some(state) = state else {
                continue;
            };
            let component = match name {
                "alb" => state.albedo,
                "ssun" => state.sunlit_absorption,
                "ssha" => state.shaded_absorption,
                "ssoi" => state.soil_absorption,
                "ssno" => state.snow_absorption,
                _ => unreachable!(),
            };
            for band in 0..2 {
                for radiation_type in 0..2 {
                    values[(patch * 2 + radiation_type) * 2 + band] =
                        component[band][radiation_type];
                }
            }
        }
        file.variable_mut(name)
            .expect("checked common restart variable exists")
            .put_values(&values, (.., .., ..))?;
    }
    for (name, select) in [
        (
            "thermk",
            (|state: &ColdStartRadiation| state.thermal_gap_fraction)
                as fn(&ColdStartRadiation) -> f64,
        ),
        (
            "extkb",
            (|state: &ColdStartRadiation| state.direct_extinction)
                as fn(&ColdStartRadiation) -> f64,
        ),
        (
            "extkd",
            (|state: &ColdStartRadiation| state.diffuse_extinction)
                as fn(&ColdStartRadiation) -> f64,
        ),
    ] {
        let mut values = file
            .variable(name)
            .with_context(|| format!("common restart has no {name}"))?
            .get_values::<f64, _>(..)?;
        ensure!(
            values.len() == patches,
            "common restart {name} has an unexpected patch layout"
        );
        for (patch, state) in states.iter().enumerate() {
            if let Some(state) = state {
                values[patch] = select(state);
            }
        }
        file.variable_mut(name)
            .expect("checked common restart variable exists")
            .put_values(&values, ..)?;
    }
    // IniTimeVariable uses PFT-weighted exposed SAI, not the independently
    // aggregated patch tsai. Keep total LAI/SAI and non-soil patches unchanged.
    for (name, replacements) in [("z0m", roughness), ("sai", stem_area)] {
        let mut values = file
            .variable(name)
            .with_context(|| format!("common restart has no {name}"))?
            .get_values::<f64, _>(..)?;
        ensure!(
            values.len() == patches,
            "common restart {name} has an unexpected patch layout"
        );
        for (value, replacement) in values.iter_mut().zip(replacements) {
            if let Some(replacement) = replacement {
                *value = *replacement;
            }
        }
        file.variable_mut(name)
            .expect("checked common restart variable exists")
            .put_values(&values, ..)?;
    }
    file.close()?;
    Ok(())
}

fn zero_common_crop_vegetation(path: &Path, crop_patch: &[bool]) -> Result<()> {
    let mut file = netcdf::append(path)
        .with_context(|| format!("cannot update CROP vegetation in {}", path.display()))?;
    for name in ["lai", "tlai", "sai", "tsai"] {
        let mut values = file
            .variable(name)
            .with_context(|| format!("common restart has no {name}"))?
            .get_values::<f64, _>(..)?;
        ensure!(
            values.len() == crop_patch.len(),
            "common restart {name} has an unexpected patch layout"
        );
        for (patch, crop) in crop_patch.iter().enumerate() {
            if *crop {
                values[patch] = 0.0;
            }
        }
        file.variable_mut(name)
            .expect("checked common restart variable exists")
            .put_values(&values, ..)?;
    }
    file.close()?;
    Ok(())
}

fn pft_hydraulic_model(namelist: &Path) -> Result<HydraulicModel> {
    pft_hydraulic_model_from_document(&read_pft_document(namelist)?)
}

fn pft_hydraulic_model_from_document(document: &colm_namelist::Document) -> Result<HydraulicModel> {
    match document.get("DEF_USE_Campbell_SOIL_MODEL") {
        Some(Value::Bool(true)) => Ok(HydraulicModel::Campbell),
        Some(Value::Bool(false)) | None => Ok(HydraulicModel::VanGenuchten),
        Some(_) => bail!("DEF_USE_Campbell_SOIL_MODEL must be a logical value"),
    }
}

fn read_pft_document(namelist: &Path) -> Result<colm_namelist::Document> {
    let text = std::fs::read_to_string(namelist)
        .with_context(|| format!("cannot read case namelist {}", namelist.display()))?;
    parse(&text).with_context(|| format!("cannot parse case namelist {}", namelist.display()))
}

fn spatial_pft_subgrid(document: &colm_namelist::Document) -> Result<SpatialPftSubgrid> {
    let pc = optional_bool_or(document, "DEF_USE_PC", false)?;
    let pft = optional_bool_or(document, "DEF_USE_PFT", !pc)?;
    ensure!(
        pft != pc,
        "exactly one of DEF_USE_PFT and DEF_USE_PC must be true for a spatial PFT restart"
    );
    Ok(if pc {
        SpatialPftSubgrid::Pc
    } else {
        SpatialPftSubgrid::Pft
    })
}

fn optional_bool_or(
    document: &colm_namelist::Document,
    field: &str,
    default: bool,
) -> Result<bool> {
    match document.get(field) {
        Some(Value::Bool(value)) => Ok(*value),
        Some(_) => bail!("{field} must be a logical value"),
        None => Ok(default),
    }
}

fn read_i32(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
) -> Result<Vec<i32>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    values_i32(&file, variable)
}

fn read_f64(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
) -> Result<Vec<f64>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    values_f64(&file, variable)
}

#[cfg(test)]
#[path = "spatial_pft_tests.rs"]
mod spatial_pft_tests;
