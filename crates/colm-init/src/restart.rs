//! NetCDF constant-restart output for `mkinidata`.
//!
//! The Fortran vector writer stores one restart block per `(west,south)` label.  This
//! module writes that same block schema in native NetCDF order: CoLM's in-memory
//! `(layer, patch)` buffers become `(patch, layer)` variables on disk.

use std::path::{Path, PathBuf};

use anyhow::{ensure, Context, Result};

use crate::{BedrockState, CanopyState, LakeState, SoilField, SoilState};

/// Dimensions written by `WRITE_TimeInvariants` for one CoLM restart block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestartDimensions {
    pub soil_layers: usize,
    pub lake_layers: usize,
    pub bands: usize,
    pub runoff_types: usize,
    pub snow_layers: usize,
    pub slope_types: usize,
    pub azimuths: usize,
    pub zeniths: usize,
    pub zenith_parameters: usize,
    pub aspect_types: usize,
    pub wavelengths: usize,
}

impl Default for RestartDimensions {
    fn default() -> Self {
        Self {
            soil_layers: 10,
            lake_layers: 10,
            bands: 2,
            runoff_types: 2,
            snow_layers: 5,
            slope_types: 4,
            azimuths: 16,
            zeniths: 101,
            zenith_parameters: 3,
            aspect_types: 9,
            wavelengths: 211,
        }
    }
}

/// Four broadband soil albedo vectors in Fortran's `WRITE_TimeInvariants` order.
#[derive(Debug, Clone, Copy)]
pub struct SoilAlbedo<'a> {
    pub saturated_visible: &'a [f64],
    pub dry_visible: &'a [f64],
    pub saturated_near_infrared: &'a [f64],
    pub dry_near_infrared: &'a [f64],
}

/// Per-patch fields that are not held by the soil, lake, or canopy state objects.
#[derive(Debug, Clone, Copy)]
pub struct RestartPatchFields<'a> {
    pub class: &'a [i32],
    pub kind: &'a [i32],
    pub mask: &'a [bool],
    pub longitude_radians: &'a [f64],
    pub latitude_radians: &'a [f64],
    pub albedo: SoilAlbedo<'a>,
    pub bvic: &'a [f64],
    pub soil_texture: &'a [i32],
    pub vic_b_infilt: &'a [f64],
    pub vic_dsmax: &'a [f64],
    pub vic_ds: &'a [f64],
    pub vic_ws: &'a [f64],
    pub vic_c: &'a [f64],
    pub elevation_mean_m: &'a [f64],
    pub elevation_std_m: &'a [f64],
    pub slope_ratio: &'a [f64],
}

/// The scalar values stored in the unblocked constant restart file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RestartTuning {
    pub zlnd: f64,
    pub zsno: f64,
    pub csoilc: f64,
    pub dewmx: f64,
    pub capr: f64,
    pub cnfac: f64,
    pub ssi: f64,
    pub wimp: f64,
    pub pondmx: f64,
    pub smpmax: f64,
    pub smpmin: f64,
    pub smpmax_hr: f64,
    pub smpmin_hr: f64,
    pub trsmx0: f64,
    pub tcrit: f64,
    pub wetwatmax: f64,
}

impl Default for RestartTuning {
    fn default() -> Self {
        // `share/MOD_Namelist.F90`: the expert-mode defaults consumed by
        // `mkinidata/MOD_Initialize.F90` before it writes this file.
        Self {
            zlnd: 0.01,
            zsno: 0.0024,
            csoilc: 0.004,
            dewmx: 0.1,
            capr: 0.34,
            cnfac: 0.5,
            ssi: 0.033,
            wimp: 0.05,
            pondmx: 10.0,
            smpmax: -1.5e5,
            smpmin: -1.0e8,
            smpmax_hr: -2.0e2,
            smpmin_hr: -2.0e5,
            trsmx0: 2.0e-4,
            tcrit: 2.5,
            wetwatmax: 200.0,
        }
    }
}

/// Inputs emitted only for `DEF_Runoff_SCHEME == 0`.
#[derive(Debug, Clone, Copy)]
pub struct TopmodelFields<'a> {
    pub topographic_index: &'a [f64],
    pub saturated_fraction_max: &'a [f64],
    pub saturated_fraction_decay: &'a [f64],
    pub alpha_twi: &'a [f64],
    pub chi_twi: &'a [f64],
    pub mu_twi: &'a [f64],
}

/// Lookup-table or curve form of terrain radiation, stored axis-major with patch last.
#[derive(Debug, Clone, Copy)]
pub enum TerrainRadiation<'a> {
    LookupTable { values: &'a [f64] },
    Curve { values: &'a [f64] },
}

/// Inputs emitted by `DEF_USE_Forcing_Downscaling`.
#[derive(Debug, Clone, Copy)]
pub struct TerrainFields<'a> {
    pub sky_view_factor: &'a [f64],
    pub curvature: &'a [f64],
    /// `slope_type * patches + patch`.
    pub slope_type: &'a [f64],
    /// `slope_type * patches + patch`.
    pub aspect_type: &'a [f64],
    /// `slope_type * patches + patch`.
    pub area_type: &'a [f64],
    pub radiation: TerrainRadiation<'a>,
}

/// Inputs emitted by `DEF_USE_Forcing_Downscaling_Simple`.
#[derive(Debug, Clone, Copy)]
pub struct SimpleTerrainFields<'a> {
    pub curvature: &'a [f64],
    /// `aspect_type * patches + patch`.
    pub slope_type: &'a [f64],
    /// `aspect_type * patches + patch`.
    pub aspect_type: &'a [f64],
}

/// Everything needed to serialize one constant-restart vector block.
#[derive(Debug, Clone, Copy)]
pub struct ConstantRestartInput<'a> {
    pub dimensions: RestartDimensions,
    pub patch: RestartPatchFields<'a>,
    pub lake: &'a LakeState,
    pub soil: &'a SoilState,
    pub canopy: &'a CanopyState,
    pub tuning: RestartTuning,
    /// `false` is Campbell; `true` writes the five van Genuchten arrays.
    pub uses_van_genuchten: bool,
    pub bedrock: Option<&'a BedrockState>,
    pub topmodel: Option<TopmodelFields<'a>>,
    pub terrain: Option<TerrainFields<'a>>,
    pub simple_terrain: Option<SimpleTerrainFields<'a>>,
    /// `wavelength * patches + patch`, as produced by the Fortran array.
    pub hyperspectral_albedo: Option<&'a [f64]>,
}

/// The paired paths produced by [`write_constant_restart`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConstantRestartFiles {
    pub constants: PathBuf,
    pub block: PathBuf,
}

/// Writes CoLM's unblocked scalar file and one vector-block constant restart file.
///
/// `block_label` is the suffix without its leading underscore, e.g. `w180_s90`.
pub fn write_constant_restart(
    restart_dir: impl AsRef<Path>,
    case_name: &str,
    lc_year: i32,
    block_label: &str,
    input: ConstantRestartInput<'_>,
) -> Result<ConstantRestartFiles> {
    ensure!(
        (0..=9999).contains(&lc_year),
        "land-cover year {lc_year} is outside the four-digit restart filename range"
    );
    validate_filename_component(case_name, "case name")?;
    validate_filename_component(block_label, "block label")?;

    let constants_dir = restart_dir.as_ref().join("const");
    std::fs::create_dir_all(&constants_dir)
        .with_context(|| format!("cannot create {}", constants_dir.display()))?;
    let stem = format!("{case_name}_restart_const_lc{lc_year:04}");
    let constants = constants_dir.join(format!("{stem}.nc"));
    let block = constants_dir.join(format!("{stem}_{block_label}.nc"));

    write_constant_restart_block(&block, input)?;
    write_restart_tuning(&constants, input.tuning)?;
    Ok(ConstantRestartFiles { constants, block })
}

/// Writes one already-addressed CoLM vector restart block.
pub fn write_constant_restart_block(
    path: impl AsRef<Path>,
    input: ConstantRestartInput<'_>,
) -> Result<()> {
    let patches = validate_input(input)?;
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file = netcdf::create(path)
        .with_context(|| format!("cannot create constant restart block {}", path.display()))?;
    define_dimensions(&mut file, patches, input.dimensions)?;

    put_i32_1d(&mut file, "patchclass", input.patch.class)?;
    put_i32_1d(&mut file, "patchtype", input.patch.kind)?;
    let mask = input
        .patch
        .mask
        .iter()
        .map(|value| i8::from(*value))
        .collect::<Vec<_>>();
    put_i8_1d(&mut file, "patchmask", &mask)?;
    put_f64_1d(&mut file, "patchlonr", input.patch.longitude_radians)?;
    put_f64_1d(&mut file, "patchlatr", input.patch.latitude_radians)?;

    put_f64_1d(&mut file, "lakedepth", &input.lake.depth_m)?;
    put_layer_major(
        &mut file,
        "dz_lake",
        "lake",
        input.dimensions.lake_layers,
        patches,
        &input.lake.thickness_m,
    )?;

    put_f64_1d(
        &mut file,
        "soil_s_v_alb",
        input.patch.albedo.saturated_visible,
    )?;
    put_f64_1d(&mut file, "soil_d_v_alb", input.patch.albedo.dry_visible)?;
    put_f64_1d(
        &mut file,
        "soil_s_n_alb",
        input.patch.albedo.saturated_near_infrared,
    )?;
    put_f64_1d(
        &mut file,
        "soil_d_n_alb",
        input.patch.albedo.dry_near_infrared,
    )?;
    if let Some(values) = input.hyperspectral_albedo {
        put_layer_major(
            &mut file,
            "soil_alb",
            "wavelength",
            input.dimensions.wavelengths,
            patches,
            values,
        )?;
    }

    for &(field, name) in &SOIL_FIELDS_COMMON {
        put_layer_major(
            &mut file,
            name,
            "soil",
            input.dimensions.soil_layers,
            patches,
            input.soil.field(field),
        )?;
    }
    put_f64_1d(&mut file, "BVIC", input.patch.bvic)?;
    if input.uses_van_genuchten {
        for &(field, name) in &SOIL_FIELDS_VAN_GENUCHTEN {
            put_layer_major(
                &mut file,
                name,
                "soil",
                input.dimensions.soil_layers,
                patches,
                input.soil.field(field),
            )?;
        }
    }
    put_i32_1d(&mut file, "soiltext", input.patch.soil_texture)?;

    if let Some(topmodel) = input.topmodel {
        put_f64_1d(&mut file, "topoweti", topmodel.topographic_index)?;
        put_f64_1d(&mut file, "fsatmax", topmodel.saturated_fraction_max)?;
        put_f64_1d(&mut file, "fsatdcf", topmodel.saturated_fraction_decay)?;
        put_f64_1d(&mut file, "alp_twi", topmodel.alpha_twi)?;
        put_f64_1d(&mut file, "chi_twi", topmodel.chi_twi)?;
        put_f64_1d(&mut file, "mu_twi", topmodel.mu_twi)?;
    }
    put_f64_1d(&mut file, "vic_b_infilt", input.patch.vic_b_infilt)?;
    put_f64_1d(&mut file, "vic_Dsmax", input.patch.vic_dsmax)?;
    put_f64_1d(&mut file, "vic_Ds", input.patch.vic_ds)?;
    put_f64_1d(&mut file, "vic_Ws", input.patch.vic_ws)?;
    put_f64_1d(&mut file, "vic_c", input.patch.vic_c)?;

    for &(field, name) in &SOIL_FIELDS_THERMAL {
        put_layer_major(
            &mut file,
            name,
            "soil",
            input.dimensions.soil_layers,
            patches,
            input.soil.field(field),
        )?;
    }
    put_f64_1d(&mut file, "htop", &input.canopy.patch_top_m)?;
    put_f64_1d(&mut file, "hbot", &input.canopy.patch_bottom_m)?;

    if let Some(bedrock) = input.bedrock {
        put_f64_1d(&mut file, "debdrock", &bedrock.depth)?;
        let layer_index = bedrock
            .layer_index
            .iter()
            .copied()
            .map(i32::try_from)
            .collect::<std::result::Result<Vec<_>, _>>()
            .context("bedrock layer index does not fit NetCDF int32")?;
        put_i32_1d(&mut file, "ibedrock", &layer_index)?;
    }
    put_f64_1d(&mut file, "elvmean", input.patch.elevation_mean_m)?;
    put_f64_1d(&mut file, "elvstd", input.patch.elevation_std_m)?;
    put_f64_1d(&mut file, "slpratio", input.patch.slope_ratio)?;

    if let Some(terrain) = input.terrain {
        put_f64_1d(&mut file, "svf_patches", terrain.sky_view_factor)?;
        put_f64_1d(&mut file, "cur_patches", terrain.curvature)?;
        put_layer_major(
            &mut file,
            "slp_type_patches",
            "type",
            input.dimensions.slope_types,
            patches,
            terrain.slope_type,
        )?;
        put_layer_major(
            &mut file,
            "asp_type_patches",
            "type",
            input.dimensions.slope_types,
            patches,
            terrain.aspect_type,
        )?;
        put_layer_major(
            &mut file,
            "area_type_patches",
            "type",
            input.dimensions.slope_types,
            patches,
            terrain.area_type,
        )?;
        match terrain.radiation {
            TerrainRadiation::LookupTable { values } => put_patch_last_3d(
                &mut file,
                "sf_lut_patches",
                ("azi", input.dimensions.azimuths),
                ("zen", input.dimensions.zeniths),
                patches,
                values,
            )?,
            TerrainRadiation::Curve { values } => put_patch_last_3d(
                &mut file,
                "sf_curve_patches",
                ("azi", input.dimensions.azimuths),
                ("zen_p", input.dimensions.zenith_parameters),
                patches,
                values,
            )?,
        }
    }
    if let Some(terrain) = input.simple_terrain {
        put_f64_1d(&mut file, "cur_patches", terrain.curvature)?;
        put_layer_major(
            &mut file,
            "slp_type_patches",
            "type_a",
            input.dimensions.aspect_types,
            patches,
            terrain.slope_type,
        )?;
        put_layer_major(
            &mut file,
            "asp_type_patches",
            "type_a",
            input.dimensions.aspect_types,
            patches,
            terrain.aspect_type,
        )?;
    }

    file.close()
        .with_context(|| format!("cannot close constant restart block {}", path.display()))?;
    Ok(())
}

/// Writes CoLM's unblocked scalar constant file.
pub fn write_restart_tuning(path: impl AsRef<Path>, tuning: RestartTuning) -> Result<()> {
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let mut file =
        netcdf::create(path).with_context(|| format!("cannot create {}", path.display()))?;
    for &(name, value) in &[
        ("zlnd", tuning.zlnd),
        ("zsno", tuning.zsno),
        ("csoilc", tuning.csoilc),
        ("dewmx", tuning.dewmx),
        ("capr", tuning.capr),
        ("cnfac", tuning.cnfac),
        ("ssi", tuning.ssi),
        ("wimp", tuning.wimp),
        ("pondmx", tuning.pondmx),
        ("smpmax", tuning.smpmax),
        ("smpmin", tuning.smpmin),
        ("smpmax_hr", tuning.smpmax_hr),
        ("smpmin_hr", tuning.smpmin_hr),
        ("trsmx0", tuning.trsmx0),
        ("tcrit", tuning.tcrit),
        ("wetwatmax", tuning.wetwatmax),
    ] {
        file.add_variable::<f64>(name, &[])?
            .put_values(&[value], ..)?;
    }
    file.close()
        .with_context(|| format!("cannot close {}", path.display()))?;
    Ok(())
}

const SOIL_FIELDS_COMMON: [(SoilField, &str); 16] = [
    (SoilField::VfQuartz, "vf_quartz"),
    (SoilField::VfGravels, "vf_gravels"),
    (SoilField::VfOm, "vf_om"),
    (SoilField::VfSand, "vf_sand"),
    (SoilField::VfClay, "vf_clay"),
    (SoilField::WfGravels, "wf_gravels"),
    (SoilField::WfSand, "wf_sand"),
    (SoilField::WfClay, "wf_clay"),
    (SoilField::WfOm, "wf_om"),
    (SoilField::OmDensity, "OM_density"),
    (SoilField::BulkDensity, "BD_all"),
    (SoilField::FieldCapacity, "wfc"),
    (SoilField::Porosity, "porsl"),
    (SoilField::Psi0, "psi0"),
    (SoilField::Bsw, "bsw"),
    (SoilField::ThetaR, "theta_r"),
];

const SOIL_FIELDS_VAN_GENUCHTEN: [(SoilField, &str); 5] = [
    (SoilField::AlphaVgm, "alpha_vgm"),
    (SoilField::LVgm, "L_vgm"),
    (SoilField::NVgm, "n_vgm"),
    (SoilField::ScVgm, "sc_vgm"),
    (SoilField::FcVgm, "fc_vgm"),
];

const SOIL_FIELDS_THERMAL: [(SoilField, &str); 8] = [
    (SoilField::HydraulicConductivity, "hksati"),
    (SoilField::HeatCapacity, "csol"),
    (SoilField::SolidThermalConductivity, "k_solids"),
    (SoilField::SaturatedUnfrozenConductivity, "dksatu"),
    (SoilField::SaturatedFrozenConductivity, "dksatf"),
    (SoilField::DryConductivity, "dkdry"),
    (SoilField::BaAlpha, "BA_alpha"),
    (SoilField::BaBeta, "BA_beta"),
];

fn validate_input(input: ConstantRestartInput<'_>) -> Result<usize> {
    let dimensions = input.dimensions;
    for (name, value) in [
        ("soil", dimensions.soil_layers),
        ("lake", dimensions.lake_layers),
        ("band", dimensions.bands),
        ("rtyp", dimensions.runoff_types),
        ("snow", dimensions.snow_layers),
        ("type", dimensions.slope_types),
        ("azi", dimensions.azimuths),
        ("zen", dimensions.zeniths),
        ("zen_p", dimensions.zenith_parameters),
        ("type_a", dimensions.aspect_types),
        ("wavelength", dimensions.wavelengths),
    ] {
        ensure!(value > 0, "restart dimension {name} must be positive");
    }
    let patches = input.patch.class.len();
    ensure!(patches > 0, "a restart block needs at least one patch");
    for (name, values) in [
        ("patch type", input.patch.kind.len()),
        ("patch mask", input.patch.mask.len()),
        ("patch longitude", input.patch.longitude_radians.len()),
        ("patch latitude", input.patch.latitude_radians.len()),
        (
            "saturated visible albedo",
            input.patch.albedo.saturated_visible.len(),
        ),
        ("dry visible albedo", input.patch.albedo.dry_visible.len()),
        (
            "saturated near-infrared albedo",
            input.patch.albedo.saturated_near_infrared.len(),
        ),
        (
            "dry near-infrared albedo",
            input.patch.albedo.dry_near_infrared.len(),
        ),
        ("BVIC", input.patch.bvic.len()),
        ("soil texture", input.patch.soil_texture.len()),
        ("VIC b_infilt", input.patch.vic_b_infilt.len()),
        ("VIC Dsmax", input.patch.vic_dsmax.len()),
        ("VIC Ds", input.patch.vic_ds.len()),
        ("VIC Ws", input.patch.vic_ws.len()),
        ("VIC c", input.patch.vic_c.len()),
        ("elevation mean", input.patch.elevation_mean_m.len()),
        (
            "elevation standard deviation",
            input.patch.elevation_std_m.len(),
        ),
        ("slope ratio", input.patch.slope_ratio.len()),
        ("lake depth", input.lake.depth_m.len()),
        ("canopy top", input.canopy.patch_top_m.len()),
        ("canopy bottom", input.canopy.patch_bottom_m.len()),
    ] {
        ensure!(
            values == patches,
            "{name} has {values} entries; expected {patches}"
        );
    }
    ensure!(
        input.soil.patches == patches && input.soil.layers == dimensions.soil_layers,
        "soil state is {} layers x {} patches; restart is {} layers x {} patches",
        input.soil.layers,
        input.soil.patches,
        dimensions.soil_layers,
        patches
    );
    ensure!(
        input.lake.thickness_m.len() == dimensions.lake_layers * patches,
        "lake thickness has {} entries; expected {} layers x {} patches",
        input.lake.thickness_m.len(),
        dimensions.lake_layers,
        patches
    );
    if let Some(values) = input.hyperspectral_albedo {
        ensure!(
            values.len() == dimensions.wavelengths * patches,
            "hyperspectral albedo has {} entries; expected {} wavelengths x {} patches",
            values.len(),
            dimensions.wavelengths,
            patches
        );
    }
    if let Some(bedrock) = input.bedrock {
        ensure!(
            bedrock.depth.len() == patches && bedrock.layer_index.len() == patches,
            "bedrock state must have one depth and layer index per patch"
        );
    }
    if let Some(fields) = input.topmodel {
        validate_patch_fields(
            "TOPMODEL",
            patches,
            [
                fields.topographic_index,
                fields.saturated_fraction_max,
                fields.saturated_fraction_decay,
                fields.alpha_twi,
                fields.chi_twi,
                fields.mu_twi,
            ],
        )?;
    }
    if let Some(fields) = input.terrain {
        validate_patch_fields(
            "terrain",
            patches,
            [
                fields.sky_view_factor,
                fields.curvature,
                fields.sky_view_factor,
            ],
        )?;
        validate_axis_major(
            "terrain slope",
            fields.slope_type,
            dimensions.slope_types,
            patches,
        )?;
        validate_axis_major(
            "terrain aspect",
            fields.aspect_type,
            dimensions.slope_types,
            patches,
        )?;
        validate_axis_major(
            "terrain area",
            fields.area_type,
            dimensions.slope_types,
            patches,
        )?;
        match fields.radiation {
            TerrainRadiation::LookupTable { values } => validate_patch_last_3d(
                "terrain lookup table",
                values,
                dimensions.azimuths,
                dimensions.zeniths,
                patches,
            )?,
            TerrainRadiation::Curve { values } => validate_patch_last_3d(
                "terrain curve",
                values,
                dimensions.azimuths,
                dimensions.zenith_parameters,
                patches,
            )?,
        }
    }
    if let Some(fields) = input.simple_terrain {
        validate_patch_fields("simple terrain", patches, [fields.curvature])?;
        validate_axis_major(
            "simple terrain slope",
            fields.slope_type,
            dimensions.aspect_types,
            patches,
        )?;
        validate_axis_major(
            "simple terrain aspect",
            fields.aspect_type,
            dimensions.aspect_types,
            patches,
        )?;
    }
    ensure!(
        input.terrain.is_none() || input.simple_terrain.is_none(),
        "CoLM writes either full or simple terrain downscaling fields, not both"
    );
    Ok(patches)
}

fn validate_patch_fields<const N: usize>(
    name: &str,
    patches: usize,
    fields: [&[f64]; N],
) -> Result<()> {
    ensure!(
        fields.iter().all(|field| field.len() == patches),
        "{name} fields must contain one value per patch"
    );
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

fn define_dimensions(
    file: &mut netcdf::FileMut,
    patches: usize,
    dimensions: RestartDimensions,
) -> Result<()> {
    for (name, length) in [
        ("patch", patches),
        ("soil", dimensions.soil_layers),
        ("lake", dimensions.lake_layers),
        ("band", dimensions.bands),
        ("rtyp", dimensions.runoff_types),
        ("snow", dimensions.snow_layers),
        ("snowp1", dimensions.snow_layers + 1),
        ("soilsnow", dimensions.soil_layers + dimensions.snow_layers),
        ("type", dimensions.slope_types),
        ("azi", dimensions.azimuths),
        ("zen", dimensions.zeniths),
        ("zen_p", dimensions.zenith_parameters),
        ("type_a", dimensions.aspect_types),
        ("wavelength", dimensions.wavelengths),
    ] {
        file.add_dimension(name, length)?;
    }
    Ok(())
}

fn put_i8_1d(file: &mut netcdf::FileMut, name: &str, values: &[i8]) -> Result<()> {
    file.add_variable::<i8>(name, &["patch"])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_i32_1d(file: &mut netcdf::FileMut, name: &str, values: &[i32]) -> Result<()> {
    file.add_variable::<i32>(name, &["patch"])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_f64_1d(file: &mut netcdf::FileMut, name: &str, values: &[f64]) -> Result<()> {
    file.add_variable::<f64>(name, &["patch"])?
        .put_values(values, ..)?;
    Ok(())
}

fn put_layer_major(
    file: &mut netcdf::FileMut,
    name: &str,
    layer_name: &str,
    layers: usize,
    patches: usize,
    values: &[f64],
) -> Result<()> {
    validate_axis_major(name, values, layers, patches)?;
    let mut on_disk = Vec::with_capacity(values.len());
    for patch in 0..patches {
        for layer in 0..layers {
            on_disk.push(values[layer * patches + patch]);
        }
    }
    file.add_variable::<f64>(name, &["patch", layer_name])?
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

#[cfg(test)]
#[path = "restart_tests.rs"]
mod restart_tests;
