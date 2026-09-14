//! Spatial LCT constant-restart adapter for Rust `mksrfdata` block artifacts.
//!
//! This is the non-SinglePoint counterpart of the static portion of
//! `mkinidata/MOD_Initialize.F90`: it keeps NetCDF block I/O here and delegates
//! lake, soil, texture, and canopy physics to `colm-core`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::crop::{map_field_2d, AreaMapping, MapGrid};
use crate::single_point::{patch_type, BVIC_USDA, IGBP_BOTTOM, IGBP_TOP, USGS_BOTTOM, USGS_TOP};
use crate::{
    colm_soil_grid, derive_bedrock, derive_igbp_canopy, derive_lake_layers,
    derive_spatial_soil_parameters, derive_usgs_canopy, normalize_soil_texture,
    write_constant_restart, CanopyState, ConstantRestartFiles, ConstantRestartInput,
    HydraulicModel, LandCoverScheme, RestartDimensions, RestartPatchFields, RestartTuning,
    SimpleTerrainFields, SoilAlbedo, SoilLayerInput, TerrainFields, TerrainRadiation,
    TopmodelFields,
};

/// Arguments for one already-addressed LCT landpatch block.
#[derive(Debug, Clone, Copy)]
pub struct SpatialLctStaticConfig<'a> {
    /// DEF_REST_CompressLevel; validated before output creation.
    pub compression_level: u8,
    pub landdata: &'a Path,
    pub restart_dir: &'a Path,
    pub case_name: &'a str,
    pub land_cover_year: i32,
    /// CoLM suffix without the leading underscore, e.g. `w180_s90`.
    pub block_label: &'a str,
    pub land_cover: LandCoverScheme,
    pub hydraulic_model: HydraulicModel,
    pub tuning: RestartTuning,
    /// Write `dbedrock` and `ibedrock`, matching `DEF_USE_BEDROCK`.
    pub use_bedrock: bool,
    /// Write 211-band `soil_alb`, matching the `HYPERSPECTRAL` build.
    pub use_hyperspectral: bool,
    /// Write the TOPMODEL vectors required by `DEF_Runoff_SCHEME = 0`.
    pub use_topmodel: bool,
    pub topmodel_method: i32,
    pub vic_parameters: VicParameterSource<'a>,
    /// Write the nine-aspect vectors required by simple forcing downscaling.
    pub use_simple_terrain: bool,
    /// Write the four slope-type and shadow-curve vectors required by regular forcing downscaling.
    pub use_regular_terrain: bool,
}

impl<'a> SpatialLctStaticConfig<'a> {
    pub fn new(
        landdata: &'a Path,
        restart_dir: &'a Path,
        case_name: &'a str,
        land_cover_year: i32,
        block_label: &'a str,
        land_cover: LandCoverScheme,
        hydraulic_model: HydraulicModel,
    ) -> Self {
        Self {
            compression_level: 1,
            landdata,
            restart_dir,
            case_name,
            land_cover_year,
            block_label,
            land_cover,
            hydraulic_model,
            tuning: RestartTuning::default(),
            use_bedrock: false,
            use_hyperspectral: false,
            use_topmodel: false,
            topmodel_method: 0,
            vic_parameters: VicParameterSource::None,
            use_simple_terrain: false,
            use_regular_terrain: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VicParameters {
    pub b_infilt: f64,
    pub dsmax: f64,
    pub ds: f64,
    pub ws: f64,
    pub c: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VicParameterSource<'a> {
    None,
    ScalarFile(&'a Path),
    GridFile(&'a Path),
}

pub fn resolve_vic_parameter_file(
    document: &colm_namelist::Document,
    use_grid: bool,
) -> Result<PathBuf> {
    let (field, suffix) = if use_grid {
        ("DEF_file_VIC_OPT", "vic/vic_para.nc")
    } else {
        ("DEF_file_VIC_para", "vic/vic_para.txt")
    };
    if let Some(colm_namelist::Value::Str(value)) = document.get(field) {
        let value = value.trim();
        if !value.is_empty() && !value.eq_ignore_ascii_case("null") {
            return Ok(PathBuf::from(value));
        }
    } else if document.get(field).is_some() {
        anyhow::bail!("{field} must be a quoted string");
    }
    match document.get("DEF_dir_runtime") {
        Some(colm_namelist::Value::Str(value))
            if !value.trim().is_empty() && !value.trim().eq_ignore_ascii_case("null") =>
        {
            Ok(PathBuf::from(value.trim()).join(suffix))
        }
        Some(colm_namelist::Value::Str(_)) | None => {
            anyhow::bail!("{field} or DEF_dir_runtime must be set for DEF_Runoff_SCHEME=1")
        }
        Some(_) => anyhow::bail!("DEF_dir_runtime must be a quoted string"),
    }
}

pub(crate) struct TopmodelSurfaceFields {
    pub(crate) topographic_index: Vec<f64>,
    pub(crate) saturated_fraction_max: Vec<f64>,
    pub(crate) saturated_fraction_decay: Vec<f64>,
    pub(crate) alpha_twi: Vec<f64>,
    pub(crate) chi_twi: Vec<f64>,
    pub(crate) mu_twi: Vec<f64>,
}

pub(crate) struct VicSurfaceFields {
    pub(crate) b_infilt: Vec<f64>,
    pub(crate) dsmax: Vec<f64>,
    pub(crate) ds: Vec<f64>,
    pub(crate) ws: Vec<f64>,
    pub(crate) c: Vec<f64>,
}

struct SimpleTerrainSurfaceFields {
    curvature: Vec<f64>,
    slope_type: Vec<f64>,
    aspect_type: Vec<f64>,
}

struct RegularTerrainSurfaceFields {
    sky_view_factor: Vec<f64>,
    curvature: Vec<f64>,
    slope_type: Vec<f64>,
    aspect_type: Vec<f64>,
    area_type: Vec<f64>,
    shadow_curve: Vec<f64>,
}

/// Writes the common LCT constant restart for one spatial block written by
/// `mksrfdata-rs spatial-lct`.
pub fn write_spatial_lct_constant_restart(
    config: SpatialLctStaticConfig<'_>,
) -> Result<ConstantRestartFiles> {
    write_spatial_lct_constant_restart_with_canopy(config, None)
}

/// Common spatial writer with canopy supplied by PFT/PC `HTOP_readin` or
/// urban `Urban_readin`, without duplicating soil/lake/terrain initialization.
pub(crate) fn write_spatial_lct_constant_restart_with_canopy(
    config: SpatialLctStaticConfig<'_>,
    canopy_override: Option<CanopyState>,
) -> Result<ConstantRestartFiles> {
    let dimensions = RestartDimensions::default();
    ensure!(
        !config.use_regular_terrain || !config.use_simple_terrain,
        "regular and simple forcing downscaling cannot share one constant restart"
    );
    ensure!(
        !config.use_topmodel || matches!(config.vic_parameters, VicParameterSource::None),
        "DEF_Runoff_SCHEME cannot enable TOPMODEL and VIC parameters at the same time"
    );
    ensure!(
        !config.use_topmodel || (0..=2).contains(&config.topmodel_method),
        "DEF_TOPMOD_method must be 0, 1, or 2; got {}",
        config.topmodel_method
    );
    let patches = read_patches(config.landdata, config.land_cover_year, config.block_label)?;
    let patch_kind = patches
        .class
        .iter()
        .map(|&class| spatial_patch_type(config.land_cover, class))
        .collect::<Result<Vec<_>>>()?;
    let patch_count = patches.class.len();
    let (longitude_radians, latitude_radians) = patch_coordinates(
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
        patch_count,
    )?;
    let lake = derive_lake_layers(&lake_depth, dimensions.lake_layers)?;
    let source_soil = read_soil(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil = derive_spatial_soil_parameters(
        &source_soil,
        &patches.class,
        &patch_kind,
        dimensions.soil_layers,
        config.hydraulic_model,
    )?;
    let mut texture = read_i32(
        config.landdata,
        "soil",
        "soiltexture_patches",
        "soiltext_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    normalize_soil_texture(&mut texture);
    let bvic = texture
        .iter()
        .map(|&value| BVIC_USDA[value as usize])
        .collect::<Vec<_>>();
    let canopy = match canopy_override {
        Some(canopy) => canopy,
        None => read_canopy(config, &patches.class, &patch_kind, patch_count)?,
    };
    // Virtual WMO patches retain geometry but do not contribute to aggregation.
    let mask = patches
        .start
        .iter()
        .map(|&start| start != -1)
        .collect::<Vec<_>>();
    let soil_s_v_alb = read_f64(
        config.landdata,
        "soil",
        "soil_s_v_alb_patches",
        "soil_s_v_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_d_v_alb = read_f64(
        config.landdata,
        "soil",
        "soil_d_v_alb_patches",
        "soil_d_v_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_s_n_alb = read_f64(
        config.landdata,
        "soil",
        "soil_s_n_alb_patches",
        "soil_s_n_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let soil_d_n_alb = read_f64(
        config.landdata,
        "soil",
        "soil_d_n_alb_patches",
        "soil_d_n_alb",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let albedo = SoilAlbedo {
        saturated_visible: &soil_s_v_alb,
        dry_visible: &soil_d_v_alb,
        saturated_near_infrared: &soil_s_n_alb,
        dry_near_infrared: &soil_d_n_alb,
    };
    let elevation = read_f64(
        config.landdata,
        "topography",
        "elevation_patches",
        "elevation_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let elevation_std = read_f64(
        config.landdata,
        "topography",
        "elvstd_patches",
        "elvstd_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let slope = read_f64(
        config.landdata,
        "topography",
        "sloperatio_patches",
        "sloperatio_patches",
        config.land_cover_year,
        config.block_label,
        patch_count,
    )?;
    let bedrock = config
        .use_bedrock
        .then(|| {
            let grid = colm_soil_grid(dimensions.soil_layers)?;
            let depth_cm = read_f64(
                config.landdata,
                "dbedrock",
                "dbedrock_patches",
                "dbedrock_patches",
                config.land_cover_year,
                config.block_label,
                patch_count,
            )?;
            derive_bedrock(
                &depth_cm,
                &patches.class,
                &grid.thickness_m,
                &grid.interface_depth_m[1..],
            )
        })
        .transpose()?;
    let hyperspectral_albedo = config
        .use_hyperspectral
        .then(|| {
            read_hyperspectral_albedo(
                config.landdata,
                config.land_cover_year,
                config.block_label,
                patch_count,
                dimensions.wavelengths,
            )
        })
        .transpose()?;
    let topmodel = config
        .use_topmodel
        .then(|| read_topmodel(config, patch_count))
        .transpose()?;
    let vic = read_vic_parameters(config, &patches)?;
    let zeros = vec![0.0; patch_count];
    let simple_terrain = config
        .use_simple_terrain
        .then(|| read_simple_terrain(config, patch_count, dimensions.aspect_types))
        .transpose()?;
    let regular_terrain = config
        .use_regular_terrain
        .then(|| {
            read_regular_terrain(
                config,
                patch_count,
                dimensions.slope_types,
                dimensions.azimuths,
                dimensions.zenith_parameters,
            )
        })
        .transpose()?;

    write_constant_restart(
        config.restart_dir,
        config.case_name,
        config.land_cover_year,
        config.block_label,
        ConstantRestartInput {
            compression_level: config.compression_level,
            dimensions,
            patch: RestartPatchFields {
                class: &patches.class,
                kind: &patch_kind,
                mask: &mask,
                longitude_radians: &longitude_radians,
                latitude_radians: &latitude_radians,
                albedo,
                bvic: &bvic,
                soil_texture: &texture,
                vic_b_infilt: vic
                    .as_ref()
                    .map_or(zeros.as_slice(), |fields| &fields.b_infilt),
                vic_dsmax: vic
                    .as_ref()
                    .map_or(zeros.as_slice(), |fields| &fields.dsmax),
                vic_ds: vic.as_ref().map_or(zeros.as_slice(), |fields| &fields.ds),
                vic_ws: vic.as_ref().map_or(zeros.as_slice(), |fields| &fields.ws),
                vic_c: vic.as_ref().map_or(zeros.as_slice(), |fields| &fields.c),
                elevation_mean_m: &elevation,
                elevation_std_m: &elevation_std,
                slope_ratio: &slope,
            },
            lake: &lake,
            soil: &soil,
            canopy: &canopy,
            tuning: config.tuning,
            uses_van_genuchten: config.hydraulic_model == HydraulicModel::VanGenuchten,
            bedrock: bedrock.as_ref(),
            topmodel: topmodel.as_ref().map(|fields| TopmodelFields {
                topographic_index: &fields.topographic_index,
                saturated_fraction_max: &fields.saturated_fraction_max,
                saturated_fraction_decay: &fields.saturated_fraction_decay,
                alpha_twi: &fields.alpha_twi,
                chi_twi: &fields.chi_twi,
                mu_twi: &fields.mu_twi,
            }),
            terrain: regular_terrain.as_ref().map(|fields| TerrainFields {
                sky_view_factor: &fields.sky_view_factor,
                curvature: &fields.curvature,
                slope_type: &fields.slope_type,
                aspect_type: &fields.aspect_type,
                area_type: &fields.area_type,
                radiation: TerrainRadiation::Curve {
                    values: &fields.shadow_curve,
                },
            }),
            simple_terrain: simple_terrain.as_ref().map(|fields| SimpleTerrainFields {
                curvature: &fields.curvature,
                slope_type: &fields.slope_type,
                aspect_type: &fields.aspect_type,
            }),
            hyperspectral_albedo: hyperspectral_albedo.as_deref(),
        },
    )
}

fn read_topmodel(
    config: SpatialLctStaticConfig<'_>,
    patches: usize,
) -> Result<TopmodelSurfaceFields> {
    let defaults = topmodel_defaults(patches);
    let read = |stem| {
        read_f64(
            config.landdata,
            "topography",
            stem,
            stem,
            config.land_cover_year,
            config.block_label,
            patches,
        )
    };
    match config.topmodel_method {
        0 => Ok(defaults),
        1 => Ok(TopmodelSurfaceFields {
            topographic_index: read("mean_twi_patches")?,
            saturated_fraction_max: read("fsatmax_patches")?,
            saturated_fraction_decay: read("fsatdcf_patches")?,
            ..defaults
        }),
        2 => Ok(TopmodelSurfaceFields {
            topographic_index: read("mean_twi_patches")?,
            alpha_twi: read("alp_twi_patches")?,
            chi_twi: read("chi_twi_patches")?,
            mu_twi: read("mu_twi_patches")?,
            ..defaults
        }),
        value => anyhow::bail!("DEF_TOPMOD_method must be 0, 1, or 2; got {value}"),
    }
}

pub(crate) fn topmodel_defaults(patches: usize) -> TopmodelSurfaceFields {
    // ponytail: deterministic inactive TOPMODEL slots; upstream mkinidata leaves
    // some method-inactive arrays unassigned before writing them.
    TopmodelSurfaceFields {
        topographic_index: vec![9.27; patches],
        saturated_fraction_max: vec![0.38; patches],
        saturated_fraction_decay: vec![0.125; patches],
        alpha_twi: vec![1.34; patches],
        chi_twi: vec![1.61; patches],
        mu_twi: vec![6.95; patches],
    }
}

pub(crate) fn read_vic_scalar_file(path: &Path) -> Result<VicParameters> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read VIC parameter file {}", path.display()))?;
    let line = text.lines().nth(1).with_context(|| {
        format!(
            "{} must contain a header and one VIC parameter line",
            path.display()
        )
    })?;
    let data = line
        .split_whitespace()
        .map(|value| value.replace(['d', 'D'], "e").parse::<f64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .with_context(|| {
            format!(
                "{} second line must contain five VIC parameters",
                path.display()
            )
        })?;
    ensure!(
        data.len() == 5 && data.iter().all(|value| value.is_finite()),
        "{} second line must contain five finite VIC parameters",
        path.display()
    );
    Ok(VicParameters {
        b_infilt: data[0],
        dsmax: data[1],
        ds: data[2],
        ws: data[3],
        c: data[4],
    })
}

pub(crate) fn constant_vic_fields(parameters: VicParameters, patches: usize) -> VicSurfaceFields {
    VicSurfaceFields {
        b_infilt: vec![parameters.b_infilt; patches],
        dsmax: vec![parameters.dsmax; patches],
        ds: vec![parameters.ds; patches],
        ws: vec![parameters.ws; patches],
        c: vec![parameters.c; patches],
    }
}

fn read_vic_parameters(
    config: SpatialLctStaticConfig<'_>,
    patches: &Patches,
) -> Result<Option<VicSurfaceFields>> {
    match config.vic_parameters {
        VicParameterSource::None => Ok(None),
        VicParameterSource::ScalarFile(path) => Ok(Some(constant_vic_fields(
            read_vic_scalar_file(path)?,
            patches.class.len(),
        ))),
        VicParameterSource::GridFile(path) => read_vic_grid(path, config, patches).map(Some),
    }
}

fn read_vic_grid(
    path: &Path,
    config: SpatialLctStaticConfig<'_>,
    patches: &Patches,
) -> Result<VicSurfaceFields> {
    let file = netcdf::open(path)
        .with_context(|| format!("cannot open gridded VIC parameter file {}", path.display()))?;
    let grid = MapGrid::from_file(&file)?;
    let pixels = read_spatial_pixel_sets(
        config.landdata,
        config.land_cover_year,
        config.block_label,
        &patches.element,
        &patches.start,
        &patches.end,
        &patches.shared_fraction,
        "VIC landpatch",
    )?;
    let mapping = AreaMapping::new(&grid, &pixels)?;
    let required = |name| -> Result<Vec<f64>> {
        mapping
            .average(&map_field_2d(&file, name, &grid)?)?
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .with_context(|| format!("gridded VIC parameter {name} has no mapped area"))
    };
    let patches = mapping.len();
    Ok(VicSurfaceFields {
        b_infilt: required("b")?,
        ws: required("Ws")?,
        ds: required("Ds")?,
        dsmax: required("DsM")?,
        c: vec![2.0; patches],
    })
}

fn read_simple_terrain(
    config: SpatialLctStaticConfig<'_>,
    patches: usize,
    aspects: usize,
) -> Result<SimpleTerrainSurfaceFields> {
    Ok(SimpleTerrainSurfaceFields {
        curvature: read_f64(
            config.landdata,
            "topography",
            "cur_patches",
            "cur_patches",
            config.land_cover_year,
            config.block_label,
            patches,
        )?,
        slope_type: read_layered_f64(
            config,
            "slp_type_patches",
            "slp_type_patches",
            patches,
            aspects,
        )?,
        aspect_type: read_layered_f64(
            config,
            "asp_type_patches",
            "asp_type_patches",
            patches,
            aspects,
        )?,
    })
}

fn read_regular_terrain(
    config: SpatialLctStaticConfig<'_>,
    patches: usize,
    slope_types: usize,
    azimuths: usize,
    curve_parameters: usize,
) -> Result<RegularTerrainSurfaceFields> {
    let read = |stem| {
        read_f64(
            config.landdata,
            "topography",
            stem,
            stem,
            config.land_cover_year,
            config.block_label,
            patches,
        )
    };
    Ok(RegularTerrainSurfaceFields {
        sky_view_factor: read("svf_patches")?,
        curvature: read("cur_patches")?,
        slope_type: read_layered_f64(
            config,
            "slp_type_patches",
            "slp_type_patches",
            patches,
            slope_types,
        )?,
        aspect_type: read_layered_f64(
            config,
            "asp_type_patches",
            "asp_type_patches",
            patches,
            slope_types,
        )?,
        area_type: read_layered_f64(
            config,
            "area_type_patches",
            "area_type_patches",
            patches,
            slope_types,
        )?,
        shadow_curve: read_patch_last_3d_f64(
            config,
            "sf_curve_patches",
            "sf_curve_patches",
            patches,
            azimuths,
            curve_parameters,
        )?,
    })
}

pub(crate) fn read_hyperspectral_albedo(
    landdata: &Path,
    year: i32,
    block: &str,
    patches: usize,
    wavelengths: usize,
) -> Result<Vec<f64>> {
    ensure!(
        wavelengths == 211,
        "CoLM hyperspectral landdata has 211 wavelengths, got {wavelengths}"
    );
    let mut values = Vec::with_capacity(wavelengths * patches);
    for wavelength_nm in (400..=2500).step_by(10) {
        let stem = format!("soil_hyper_alb_{wavelength_nm}nm_patches");
        values.extend(read_f64(
            landdata,
            "HyperAlbedo",
            &stem,
            "soil_hyper_alb",
            year,
            block,
            patches,
        )?);
    }
    Ok(values)
}

pub(crate) struct Patches {
    pub(crate) class: Vec<i32>,
    pub(crate) element: Vec<i64>,
    pub(crate) start: Vec<i32>,
    pub(crate) end: Vec<i32>,
    pub(crate) shared_fraction: Vec<f64>,
}

pub(crate) fn read_patches(landdata: &Path, year: i32, block: &str) -> Result<Patches> {
    let path = block_path(landdata, "landpatch", "landpatch", year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let class = values_i32(&file, "settyp")?;
    let element = values_i64(&file, "eindex")?;
    let start = values_i32(&file, "ipxstt")?;
    let end = values_i32(&file, "ipxend")?;
    let shared_fraction = match file.variable("pctshared") {
        Some(variable) => variable.get_values::<f64, _>(..).with_context(|| {
            format!(
                "cannot read landpatch sharing fractions from {}",
                path.display()
            )
        })?,
        None => vec![1.0; class.len()],
    };
    ensure!(
        !class.is_empty()
            && class.len() == element.len()
            && class.len() == start.len()
            && class.len() == end.len()
            && class.len() == shared_fraction.len()
            && shared_fraction
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "landpatch block has inconsistent vectors"
    );
    Ok(Patches {
        class,
        element,
        start,
        end,
        shared_fraction,
    })
}

pub(crate) fn read_canopy(
    config: SpatialLctStaticConfig<'_>,
    class: &[i32],
    kind: &[i32],
    patches: usize,
) -> Result<CanopyState> {
    match config.land_cover {
        LandCoverScheme::Usgs => derive_usgs_canopy(class, &USGS_TOP, &USGS_BOTTOM),
        LandCoverScheme::Igbp => derive_igbp_canopy(
            class,
            kind,
            &read_f64(
                config.landdata,
                "htop",
                "htop_patches",
                "htop_patches",
                config.land_cover_year,
                config.block_label,
                patches,
            )?,
            &IGBP_TOP,
            &IGBP_BOTTOM,
            None,
        ),
    }
}

pub(crate) fn read_soil(
    landdata: &Path,
    year: i32,
    block: &str,
    patches: usize,
) -> Result<Vec<SoilLayerInput>> {
    const FIELDS: [&str; 26] = [
        "vf_quartz_mineral_s",
        "vf_gravels_s",
        "vf_om_s",
        "vf_sand_s",
        "vf_clay_s",
        "wf_gravels_s",
        "wf_sand_s",
        "wf_clay_s",
        "wf_om_s",
        "OM_density_s",
        "BD_all_s",
        "theta_s",
        "psi_s",
        "lambda",
        "theta_r",
        "alpha_vgm",
        "L_vgm",
        "n_vgm",
        "k_s",
        "csol",
        "k_solids",
        "tksatu",
        "tksatf",
        "tkdry",
        "BA_alpha",
        "BA_beta",
    ];
    let mut data = Vec::with_capacity(FIELDS.len());
    for field in FIELDS {
        let mut layers = Vec::with_capacity(8);
        for layer in 1..=8 {
            let name = format!("{field}_l{layer}_patches");
            layers.push(read_f64(
                landdata, "soil", &name, &name, year, block, patches,
            )?);
        }
        data.push(layers);
    }
    let value = |field: usize, layer: usize, patch: usize| data[field][layer][patch];
    Ok((0..8)
        .flat_map(|layer| {
            (0..patches).map(move |patch| SoilLayerInput {
                vf_quartz: value(0, layer, patch),
                vf_gravels: value(1, layer, patch),
                vf_om: value(2, layer, patch),
                vf_sand: value(3, layer, patch),
                vf_clay: value(4, layer, patch),
                wf_gravels: value(5, layer, patch),
                wf_sand: value(6, layer, patch),
                wf_clay: value(7, layer, patch),
                wf_om: value(8, layer, patch),
                om_density: value(9, layer, patch),
                bulk_density: value(10, layer, patch),
                theta_s: value(11, layer, patch),
                psi_s_cm: value(12, layer, patch),
                lambda: value(13, layer, patch),
                theta_r: value(14, layer, patch),
                alpha_vgm: value(15, layer, patch),
                l_vgm: value(16, layer, patch),
                n_vgm: value(17, layer, patch),
                k_s_cm_day: value(18, layer, patch),
                csol: value(19, layer, patch),
                k_solids: value(20, layer, patch),
                tksatu: value(21, layer, patch),
                tksatf: value(22, layer, patch),
                tkdry: value(23, layer, patch),
                ba_alpha: value(24, layer, patch),
                ba_beta: value(25, layer, patch),
            })
        })
        .collect())
}

pub(crate) fn spatial_patch_type(land_cover: LandCoverScheme, class: i32) -> Result<i32> {
    if class == 0 {
        Ok(0)
    } else {
        patch_type(land_cover, class)
    }
}

pub(crate) fn read_i32(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    expected: usize,
) -> Result<Vec<i32>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let values = values_i32(&file, variable)?;
    ensure!(
        values.len() == expected,
        "{} has {} values; expected {expected}",
        path.display(),
        values.len()
    );
    Ok(values)
}

pub(crate) fn read_f64(
    landdata: &Path,
    directory: &str,
    stem: &str,
    variable: &str,
    year: i32,
    block: &str,
    expected: usize,
) -> Result<Vec<f64>> {
    let path = block_path(landdata, directory, stem, year, block);
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let values = values_f64(&file, variable)?;
    ensure!(
        values.len() == expected,
        "{} has {} values; expected {expected}",
        path.display(),
        values.len()
    );
    Ok(values)
}

fn read_layered_f64(
    config: SpatialLctStaticConfig<'_>,
    stem: &str,
    variable: &str,
    patches: usize,
    layers: usize,
) -> Result<Vec<f64>> {
    let values = read_f64(
        config.landdata,
        "topography",
        stem,
        variable,
        config.land_cover_year,
        config.block_label,
        patches * layers,
    )?;
    let mut result = vec![0.0; values.len()];
    for patch in 0..patches {
        for layer in 0..layers {
            result[layer * patches + patch] = values[patch * layers + layer];
        }
    }
    Ok(result)
}

fn read_patch_last_3d_f64(
    config: SpatialLctStaticConfig<'_>,
    stem: &str,
    variable: &str,
    patches: usize,
    first: usize,
    second: usize,
) -> Result<Vec<f64>> {
    let path = block_path(
        config.landdata,
        "topography",
        stem,
        config.land_cover_year,
        config.block_label,
    );
    let file = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let source = file
        .variable(variable)
        .with_context(|| format!("{variable} is absent from {}", path.display()))?;
    let dimensions = source.dimensions();
    ensure!(
        dimensions.len() == 3
            && dimensions[0].name() == "patch"
            && dimensions[0].len() == patches
            && dimensions[1].len() == second
            && dimensions[2].len() == first,
        "{variable} in {} must have patch, second, first dimensions of {patches}x{second}x{first}",
        path.display()
    );
    let values = source.get_values::<f64, _>(..)?;
    ensure!(
        values.len() == patches * first * second,
        "{variable} in {} has {} values; expected {}",
        path.display(),
        values.len(),
        patches * first * second
    );
    let mut result = vec![0.0; values.len()];
    for patch in 0..patches {
        for first_index in 0..first {
            for second_index in 0..second {
                result[(first_index * second + second_index) * patches + patch] =
                    values[(patch * second + second_index) * first + first_index];
            }
        }
    }
    Ok(result)
}

pub(crate) fn block_path(
    landdata: &Path,
    directory: &str,
    stem: &str,
    year: i32,
    block: &str,
) -> PathBuf {
    landdata
        .join(directory)
        .join(format!("{year:04}"))
        .join(format!("{stem}_{block}.nc"))
}

pub(crate) fn patch_coordinates(
    landdata: &Path,
    year: i32,
    block: &str,
    patches: &Patches,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let pixel_sets = read_spatial_pixel_sets(
        landdata,
        year,
        block,
        &patches.element,
        &patches.start,
        &patches.end,
        &patches.shared_fraction,
        "patch",
    )?;
    let mut longitude = Vec::with_capacity(patches.class.len());
    let mut latitude = Vec::with_capacity(patches.class.len());
    for cells in &pixel_sets.cells {
        let (lon, lat) = pixel_sets.mean(cells)?;
        // MOD_Pixelset multiplies by pi before dividing by 180 (not a folded scale).
        longitude.push(lon * std::f64::consts::PI / 180.0);
        latitude.push(lat * std::f64::consts::PI / 180.0);
    }
    Ok((longitude, latitude))
}

/// Pixel memberships and geographic edges shared by spatial restart readers.
///
/// `pctshared` is retained because CoLM's areal mapper applies it before a
/// `landpatch` or `landpft` result is normalized.
#[derive(Debug, Clone)]
pub(crate) struct SpatialPixelSets {
    pub(crate) lon_w: Vec<f64>,
    pub(crate) lon_e: Vec<f64>,
    pub(crate) lat_s: Vec<f64>,
    pub(crate) lat_n: Vec<f64>,
    pub(crate) cells: Vec<Vec<(i32, i32)>>,
    pub(crate) shared_fraction: Vec<f64>,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn read_spatial_pixel_sets(
    landdata: &Path,
    year: i32,
    block: &str,
    element: &[i64],
    start: &[i32],
    end: &[i32],
    shared_fraction: &[f64],
    label: &str,
) -> Result<SpatialPixelSets> {
    ensure!(
        !element.is_empty()
            && element.len() == start.len()
            && element.len() == end.len()
            && element.len() == shared_fraction.len()
            && shared_fraction
                .iter()
                .all(|value| value.is_finite() && *value >= 0.0),
        "{label} pixel topology has inconsistent vectors"
    );
    let pixel_file = netcdf::open(landdata.join("pixel.nc"))?;
    let lon_w = values_f64(&pixel_file, "lon_w")?;
    let lon_e = values_f64(&pixel_file, "lon_e")?;
    let lat_s = values_f64(&pixel_file, "lat_s")?;
    let lat_n = values_f64(&pixel_file, "lat_n")?;
    ensure!(
        lon_w.len() == lon_e.len() && lat_s.len() == lat_n.len(),
        "pixel edge vectors have inconsistent lengths"
    );
    let path = block_path(landdata, "mesh", "mesh", year, block);
    let mesh = netcdf::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    let ids = values_i64(&mesh, "elmindex")?;
    let counts = values_i32(&mesh, "elmnpxl")?;
    let points = values_i32(&mesh, "elmpixels")?;
    ensure!(
        ids.len() == counts.len() && points.len() == 2 * counts.iter().sum::<i32>() as usize,
        "mesh block has inconsistent element vectors"
    );
    let mut elements = BTreeMap::new();
    let mut offset = 0;
    for (&id, &count) in ids.iter().zip(&counts) {
        let count = usize::try_from(count).context("mesh pixel count is negative")?;
        let cells = (0..count)
            .map(|index| {
                (
                    points[2 * (offset + index)],
                    points[2 * (offset + index) + 1],
                )
            })
            .collect::<Vec<_>>();
        elements.insert(id, cells);
        offset += count;
    }
    let cells = element
        .iter()
        .zip(start)
        .zip(end)
        .enumerate()
        .map(|(index, ((&element, &start), &end))| {
            let cells = elements
                .get(&element)
                .with_context(|| format!("{label} {index} references unknown mesh element"))?;
            Ok(cells[pixel_range(start, end, cells.len())?].to_vec())
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SpatialPixelSets {
        lon_w,
        lon_e,
        lat_s,
        lat_n,
        cells,
        shared_fraction: shared_fraction.to_vec(),
    })
}

impl SpatialPixelSets {
    fn mean(&self, cells: &[(i32, i32)]) -> Result<(f64, f64)> {
        ensure!(!cells.is_empty(), "patch has no pixels");
        let mut area_sum = 0.0;
        let mut latitude = 0.0;
        let mut longitude = 0.0;
        for &(x, y) in cells {
            let x = usize::try_from(x).context("mesh longitude is negative")?;
            let y = usize::try_from(y).context("mesh latitude is negative")?;
            ensure!(
                x > 0 && x <= self.lon_w.len() && y > 0 && y <= self.lat_s.len(),
                "mesh pixel is outside pixel axes"
            );
            let west = self.lon_w[x - 1];
            let east = self.lon_e[x - 1];
            let south = self.lat_s[y - 1];
            let north = self.lat_n[y - 1];
            let width = if east < west {
                east + 360.0 - west
            } else {
                east - west
            };
            // Preserve MOD_Utils::areaquad's rounded conversion and km² units.
            // Cancelling the common radius scale changes centroid rounding; near sunrise
            // that perturbation is amplified by the canopy extinction's 1/coszen factor.
            let deg2rad = 1.745_329_251_994_33e-2;
            let area = width
                * deg2rad
                * ((north * deg2rad).sin() - (south * deg2rad).sin())
                * 6.37122e3
                * 6.37122e3;
            ensure!(
                area.is_finite() && area > 0.0,
                "pixel has invalid spherical area"
            );
            let mut center = if east < west {
                (west + east + 360.0) * 0.5
            } else {
                (west + east) * 0.5
            };
            center = normalize_longitude(center);
            let adjusted_center = if longitude - center > 180.0 {
                center + 360.0
            } else if longitude - center < -180.0 {
                center - 360.0
            } else {
                center
            };
            longitude = adjusted_center.mul_add(area, longitude * area_sum);
            area_sum += area;
            longitude = normalize_longitude(longitude / area_sum);
            latitude = ((south + north) * 0.5).mul_add(area, latitude);
        }
        Ok((longitude, latitude / area_sum))
    }
}

fn pixel_range(start: i32, end: i32, count: usize) -> Result<std::ops::Range<usize>> {
    if start == -1 && end == -1 {
        return Ok(0..count);
    }
    let start = usize::try_from(start).context("patch pixel start is negative")?;
    let end = usize::try_from(end).context("patch pixel end is negative")?;
    ensure!(
        start > 0 && start <= end && end <= count,
        "patch pixel range is outside its mesh element"
    );
    Ok(start - 1..end)
}

fn normalize_longitude(value: f64) -> f64 {
    let mut value = value % 360.0;
    if value >= 180.0 {
        value -= 360.0;
    }
    if value < -180.0 {
        value += 360.0;
    }
    value
}

pub(crate) fn values_f64(file: &netcdf::File, name: &str) -> Result<Vec<f64>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

pub(crate) fn values_i32(file: &netcdf::File, name: &str) -> Result<Vec<i32>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

pub(crate) fn values_i64(file: &netcdf::File, name: &str) -> Result<Vec<i64>> {
    file.variable(name)
        .with_context(|| format!("NetCDF file is missing {name}"))?
        .get_values(..)
        .with_context(|| format!("cannot read {name}"))
}

#[cfg(test)]
#[path = "spatial_static_tests.rs"]
mod spatial_static_tests;
