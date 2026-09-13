//! Spatial PFT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! The common land-patch restart is written by [`crate::spatial_static`].
//! This module writes its separate `landpft` companion, exactly as CoLM's
//! `WRITE_PFTimeInvariants` does after `pct_readin` and `HTOP_readin`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};
use colm_namelist::{parse, Value};

use crate::single_point::{
    aggregate_pft_radiation, pc_canopy_layer, pc_uses_three_dimensional_canopy, pft_canopy,
    pft_leaf_optics,
};
use crate::spatial_static::{
    block_path, read_f64 as read_lct_f64, read_patches, spatial_patch_type, values_f64, values_i32,
    write_spatial_lct_constant_restart, SpatialLctStaticConfig,
};
use crate::{
    cold_start_pc_broadband_radiation_with_snow, cold_start_pft_broadband_radiation_with_snow,
    write_pft_constant_restart, write_pft_time_restart, ColdStartRadiation, ConstantRestartFiles,
    HydraulicModel, LandCoverScheme, PcPftInput, PftConstantRestartInput, PftOzoneFields,
    PftPlantHydraulicFields, PftTimeFields, PftTimeRestartInput, RestartDate, RestartTuning,
    SpatialLctTimeConfig, TimeRestartFile, MISSING,
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
}

/// Arguments for the no-observation PFT cold start of one spatial block.
///
/// PFT and PC share this adapter.  BGC and CROP need separate runtime state
/// families and are rejected instead of being initialized as PFT.
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
}

/// Timestamped restart blocks written by a spatial PFT cold start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpatialPftTimeRestartFiles {
    pub common: TimeRestartFile,
    pub pft: PathBuf,
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
        }
    }
}

/// Writes the PFT/PC constant restart for one Rust `mksrfdata-rs spatial-pft` block.
pub fn write_spatial_pft_constant_restart(config: SpatialPftStaticConfig<'_>) -> Result<PathBuf> {
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
    let text = std::fs::read_to_string(config.namelist)
        .with_context(|| format!("cannot read case namelist {}", config.namelist.display()))?;
    let document = parse(&text)
        .with_context(|| format!("cannot parse case namelist {}", config.namelist.display()))?;
    let canopy = pft_canopy(&document, &class, &observed_height_m)?;
    write_pft_constant_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        PftConstantRestartInput {
            class: &class,
            fraction: &fraction,
            canopy_top_m: &canopy.top_m,
            canopy_bottom_m: &canopy.bottom_m,
            crop_fraction: None,
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
    common.use_hyperspectral = use_hyperspectral;
    let common = write_spatial_lct_constant_restart(common)?;
    let pft = write_spatial_pft_constant_restart(config)?;
    Ok(SpatialPftConstantRestartFiles { common, pft })
}

/// Writes both common and PFT timestamped restart blocks for one spatial PFT
/// cold start.
///
/// The common block is first built by the LCT no-observation initializer, which
/// is the owner of the shared cold soil, lake, snow, and clock state.  Natural
/// patches are then replaced with their exact PFT-weighted optical state and
/// roughness, while water, ice, wetland, and urban patches retain that shared
/// common state.  This prevents a second copy of the cold-soil path here.
pub fn write_spatial_pft_cold_time_restarts(
    config: SpatialPftTimeConfig<'_>,
) -> Result<SpatialPftTimeRestartFiles> {
    let document = read_pft_document(config.static_config.namelist)?;
    reject_unsupported_time_features(&document)?;
    let subgrid = spatial_pft_subgrid(&document)?;
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
    common_config.tuning = config.tuning;
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
    let pft_to_patch = match_pfts_to_patches(&patches, &patch_kind, &pfts)?;
    let pft_count = pfts.class.len();
    let month = crate::spatial_time::month(config.date)?;
    let total_lai = read_pft_monthly(config.static_config, config.lai_year, "LAI_pfts", month)?;
    let total_sai = read_pft_monthly(config.static_config, config.lai_year, "SAI_pfts", month)?;
    ensure!(
        total_lai.len() == pft_count && total_sai.len() == pft_count,
        "spatial PFT monthly vegetation has inconsistent vector lengths"
    );
    let canopy = pft_canopy(&document, &pfts.class, &pfts.observed_height_m)?;
    let common_state = read_common_state(&common.block, patches.class.len())?;
    let top_soil_thickness_m = crate::colm_soil_grid(10)?.thickness_m[0];
    let albedo = [
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_s_v_alb",
            "soil_s_v_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_d_v_alb",
            "soil_d_v_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_s_n_alb",
            "soil_s_n_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
        read_lct_f64(
            config.static_config.landdata,
            "soil",
            "soil_d_n_alb",
            "soil_d_n_alb",
            config.static_config.land_cover_year,
            config.static_config.block_label,
            patches.class.len(),
        )?,
    ];

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
            let state = cold_start_pft_broadband_radiation_with_snow(
                patch_kind[patch],
                crate::SoilReflectance {
                    saturated_visible: albedo[0][patch],
                    dry_visible: albedo[1][patch],
                    saturated_near_infrared: albedo[2][patch],
                    dry_near_infrared: albedo[3][patch],
                },
                common_state.top_liquid_kg_m2[patch],
                top_soil_thickness_m,
                pft_leaf_optics(&document, pfts.class[pft], hydraulic_model)?,
                total_lai[pft],
                total_sai[pft],
                0.0,
                common_state.cosine_zenith[patch].max(0.001),
                config.vegetation_snow,
                0.0,
                0.0,
                common_state.ground_temperature_k[patch],
            )?;
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
                .filter(|&pft| pc_uses_three_dimensional_canopy(pfts.class[pft], pc_crop_split))
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
                        optics: pft_leaf_optics(&document, pfts.class[pft], hydraulic_model)?,
                        lai: total_lai[pft],
                        sai: total_sai[pft],
                        wet_snow_fraction: 0.0,
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let pc = cold_start_pc_broadband_radiation_with_snow(
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
            )?;
            for (pc_index, &pft) in indices.iter().enumerate() {
                let state = &pc.pft[pc_index];
                copy_pft_radiation(&mut sunlit, pft_count, pft, state.sunlit_absorption);
                copy_pft_radiation(&mut shaded, pft_count, pft, state.shaded_absorption);
                thermal_gap[pft] = state.thermal_gap_fraction;
                shade[pft] = state.shade_fraction;
                direct_extinction[pft] = state.direct_extinction;
                diffuse_extinction[pft] = state.diffuse_extinction;
                state_by_pft[pft] = Some(pc.common.clone());
            }
        }
    }
    let mut common_radiation = vec![None; patches.class.len()];
    let mut common_roughness = vec![None; patches.class.len()];
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
        common_radiation[patch] = Some(aggregate_pft_radiation(
            &states,
            &fractions,
            leaf_stem_area,
        )?);
        common_roughness[patch] = Some(
            indices
                .iter()
                .map(|&index| roughness[index] * pfts.fraction[index])
                .sum(),
        );
    }

    update_common_pft_optics(&common.block, &common_radiation, &common_roughness)?;
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
            hyperspectral: None,
            plant_hydraulics: config.plant_hydraulics.then_some(PftPlantHydraulicFields {
                water_potential_mm: &plant_water,
                sunlit_stomatal_conductance: &conductance,
                shaded_stomatal_conductance: &conductance,
                vegetation_nodes: 4,
            }),
            bgc: None,
            crop: None,
            ozone: config.ozone_stress.then_some(PftOzoneFields {
                lai_old: &total_lai,
                sunlit_uptake: &ozone_zero,
                shaded_uptake: &ozone_zero,
                sunlit_vegetation_coefficient: &one,
                shaded_vegetation_coefficient: &one,
                sunlit_stomatal_coefficient: &one,
                shaded_stomatal_coefficient: &one,
            }),
            irrigation_method: None,
        },
    )?;
    Ok(SpatialPftTimeRestartFiles { common, pft })
}

#[derive(Debug)]
struct SpatialPftVectors {
    class: Vec<i32>,
    fraction: Vec<f64>,
    observed_height_m: Vec<f64>,
    element: Vec<i64>,
    start: Vec<i32>,
    end: Vec<i32>,
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
    })
}

fn match_pfts_to_patches(
    patches: &crate::spatial_static::Patches,
    patch_kind: &[i32],
    pfts: &SpatialPftVectors,
) -> Result<Vec<Vec<usize>>> {
    ensure!(
        patches.class.len() == patch_kind.len(),
        "spatial patch kind vector does not match topology"
    );
    let mut indices = BTreeMap::new();
    for patch in 0..patches.class.len() {
        ensure!(
            indices
                .insert(
                    (
                        patches.element[patch],
                        patches.start[patch],
                        patches.end[patch]
                    ),
                    patch,
                )
                .is_none(),
            "spatial landpatch topology duplicates a pixel range"
        );
    }
    let mut out = vec![Vec::new(); patches.class.len()];
    for pft in 0..pfts.class.len() {
        let key = (pfts.element[pft], pfts.start[pft], pfts.end[pft]);
        let patch = *indices
            .get(&key)
            .with_context(|| format!("PFT {pft} does not match a landpatch pixel range"))?;
        ensure!(
            patch_kind[patch] == 0,
            "PFT {pft} belongs to non-natural landpatch {patch}"
        );
        out[patch].push(pft);
    }
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

fn update_common_pft_optics(
    path: &Path,
    states: &[Option<ColdStartRadiation>],
    roughness: &[Option<f64>],
) -> Result<()> {
    ensure!(
        states.len() == roughness.len(),
        "common PFT radiation and roughness vectors must align"
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
    let mut z0m = file
        .variable("z0m")
        .context("common restart has no z0m")?
        .get_values::<f64, _>(..)?;
    ensure!(
        z0m.len() == patches,
        "common restart z0m has an unexpected patch layout"
    );
    for (patch, value) in roughness.iter().enumerate() {
        if let Some(value) = value {
            z0m[patch] = *value;
        }
    }
    file.variable_mut("z0m")
        .expect("checked common restart z0m exists")
        .put_values(&z0m, ..)?;
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

fn reject_unsupported_time_features(document: &colm_namelist::Document) -> Result<()> {
    for (field, message) in [
        (
            "DEF_USE_BGC",
            "spatial BGC cold starts are not implemented by the Rust initializer",
        ),
        (
            "DEF_USE_CROP",
            "spatial CROP cold starts are not implemented by the Rust initializer",
        ),
    ] {
        ensure!(!optional_bool_or(document, field, false)?, "{message}");
    }
    Ok(())
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
