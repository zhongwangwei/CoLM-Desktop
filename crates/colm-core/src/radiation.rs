//! Broadband cold-start radiation from `MOD_Albedo.F90`.
//!
//! The arrays use CoLM's native `(band, radiation_type)` layout.  NetCDF reversal
//! remains the responsibility of the restart writer.

use anyhow::{ensure, Result};

use crate::{update_snow_age, LandCoverScheme, SoilReflectance};

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;
// `twostream_mod` uses this literal rather than the platform PI constant.
const FORTRAN_PI: f64 = 314_159.0 / 100_000.0;

/// Per-land-class optical constants passed by `MOD_Const_LC` to `twostream`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafOptics {
    pub chil: f64,
    /// `[band][green leaf, dead stem]`.
    pub reflectance: [[f64; BANDS]; BANDS],
    /// `[band][green leaf, dead stem]`.
    pub transmittance: [[f64; BANDS]; BANDS],
}

/// Ground optical state before a canopy two-stream calculation.
///
/// All matrices use CoLM's `[visible-or-near-infrared][direct-or-diffuse]`
/// layout.  Keeping this boundary separate lets broadband and hyperspectral
/// canopy drivers use exactly the same soil, water, and snow treatment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColdStartGroundAlbedo {
    pub soil: [[f64; RADIATION_TYPES]; BANDS],
    pub snow: [[f64; RADIATION_TYPES]; BANDS],
    pub ground: [[f64; RADIATION_TYPES]; BANDS],
    pub snow_age: f64,
}

/// Looks up CoLM's native broadband leaf optical constants for one land class.
///
/// This is the `rho`/`tau` assignment in `MOD_Const_LC.F90`; it deliberately
/// keeps classes one-based, as does the Fortran land-cover contract.
pub fn leaf_optics_from_land_cover(scheme: LandCoverScheme, land_class: i32) -> Result<LeafOptics> {
    let index = usize::try_from(land_class)
        .map_err(|_| anyhow::anyhow!("land class {land_class} is negative"))?
        .checked_sub(1)
        .ok_or_else(|| anyhow::anyhow!("land class must start at one"))?;
    let table = match scheme {
        LandCoverScheme::Igbp => &IGBP_LEAF_OPTICS[..],
        LandCoverScheme::Usgs => &USGS_LEAF_OPTICS[..],
    };
    table.get(index).copied().ok_or_else(|| {
        anyhow::anyhow!("land class {land_class} is outside the selected CoLM optical table")
    })
}

#[allow(clippy::too_many_arguments)]
const fn optics(
    chil: f64,
    rhol_vis: f64,
    rhos_vis: f64,
    rhol_nir: f64,
    rhos_nir: f64,
    taul_vis: f64,
    taus_vis: f64,
    taul_nir: f64,
    taus_nir: f64,
) -> LeafOptics {
    LeafOptics {
        chil,
        reflectance: [[rhol_vis, rhos_vis], [rhol_nir, rhos_nir]],
        transmittance: [[taul_vis, taus_vis], [taul_nir, taus_nir]],
    }
}

// `main/MOD_Const_LC.F90`, listed one class per row to avoid transposition bugs.
const IGBP_LEAF_OPTICS: [LeafOptics; 17] = [
    optics(0.01, 0.07, 0.16, 0.35, 0.39, 0.05, 0.001, 0.10, 0.001),
    optics(0.10, 0.10, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.07, 0.16, 0.35, 0.39, 0.05, 0.001, 0.10, 0.001),
    optics(0.25, 0.10, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.125, 0.07, 0.16, 0.40, 0.39, 0.05, 0.001, 0.15, 0.001),
    optics(0.01, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.105, 0.16, 0.58, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.105, 0.16, 0.58, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.10, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.01, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.01, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.105, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.105, 0.16, 0.58, 0.39, 0.05, 0.001, 0.25, 0.001),
];

const USGS_LEAF_OPTICS: [LeafOptics; 24] = [
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.01, 0.10, 0.16, 0.45, 0.39, 0.07, 0.001, 0.25, 0.001),
    optics(0.01, 0.10, 0.16, 0.45, 0.39, 0.07, 0.001, 0.25, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.25, 0.10, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.07, 0.16, 0.35, 0.39, 0.05, 0.001, 0.10, 0.001),
    optics(0.10, 0.10, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.07, 0.16, 0.35, 0.39, 0.05, 0.001, 0.10, 0.001),
    optics(0.125, 0.07, 0.16, 0.40, 0.39, 0.05, 0.001, 0.15, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(0.10, 0.10, 0.16, 0.45, 0.39, 0.05, 0.001, 0.25, 0.001),
    optics(0.01, 0.10, 0.16, 0.45, 0.39, 0.07, 0.001, 0.25, 0.001),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
    optics(-0.3, 0.105, 0.36, 0.58, 0.58, 0.07, 0.22, 0.25, 0.38),
];

/// Broadband arrays produced by CoLM's cold-start `albland` path.
#[derive(Debug, Clone, PartialEq)]
pub struct ColdStartRadiation {
    /// `[band][direct, diffuse]`.
    pub albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub sunlit_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub shaded_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub soil_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub snow_absorption: [[f64; RADIATION_TYPES]; BANDS],
    /// CoLM's dimensionless snow age after its first 1800-second update.
    pub snow_age: f64,
    pub thermal_gap_fraction: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
}

/// Applies the no-snow cold-start branch of `albland` and `twostream`.
///
/// `cosine_zenith` must already be clamped like the `IniTimeVar` caller does
/// (`max(0.001, coszen)`).  A non-LCT natural patch keeps the soil albedo, exactly
/// as the source's `DEF_USE_LCT` guard does.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_broadband_radiation(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    usgs_land_cover: bool,
    vegetation_snow: bool,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        usgs_land_cover,
        vegetation_snow,
        0.0,
        0.0,
        273.16,
    )
}

/// Applies `albland` with the non-SNICAR snow initialization used by `mkinidata`.
///
/// The supplied snow depth is converted with the source's fixed 250 kg m-3
/// initialization density.  SNICAR remains a distinct feature because its optical
/// lookup tables and layer absorption model are not part of this broadband kernel.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_broadband_radiation_with_snow(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    usgs_land_cover: bool,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow_using(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        use_lct,
        vegetation_snow,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
        TwoStreamKind::LandCover { usgs_land_cover },
    )
}

/// Applies the PFT-specific `twostream_mod` path from `MOD_Albedo.F90`.
///
/// CoLM uses a different two-stream implementation for PFT vectors than for
/// land-cover tiles; using the land-cover routine here changes initialized
/// canopy absorption even when the optical constants are correct.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_pft_broadband_radiation_with_snow(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartRadiation> {
    cold_start_broadband_radiation_with_snow_using(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        optics,
        lai,
        sai,
        wet_snow_fraction,
        cosine_zenith,
        true,
        vegetation_snow,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
        TwoStreamKind::Pft,
    )
}

enum TwoStreamKind {
    LandCover { usgs_land_cover: bool },
    Pft,
}

#[allow(clippy::too_many_arguments)]
fn cold_start_broadband_radiation_with_snow_using(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    use_lct: bool,
    vegetation_snow: bool,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
    two_stream_kind: TwoStreamKind,
) -> Result<ColdStartRadiation> {
    ensure!(
        soil_liquid_water_kg_m2.is_finite()
            && soil_thickness_m.is_finite()
            && soil_thickness_m > 0.0
            && lai.is_finite()
            && lai >= 0.0
            && sai.is_finite()
            && sai >= 0.0
            && wet_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&wet_snow_fraction)
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&ground_snow_fraction)
            && ground_temperature_k.is_finite()
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0
            && optics.chil.is_finite(),
        "cold-start radiation inputs are invalid"
    );
    for value in [
        soil.saturated_visible,
        soil.dry_visible,
        soil.saturated_near_infrared,
        soil.dry_near_infrared,
    ] {
        ensure!(value.is_finite(), "soil reflectance must be finite");
    }
    for values in optics.reflectance.into_iter().chain(optics.transmittance) {
        for value in values {
            ensure!(value.is_finite(), "leaf optical constants must be finite");
        }
    }

    let ground_state = cold_start_ground_albedo(
        patch_type,
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        cosine_zenith,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
    )?;
    let soil_ground = ground_state.soil;
    let snow = ground_state.snow;
    let snow_age = ground_state.snow_age;
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut transmission = [[0.0, 1.0, 1.0]; BANDS];
    let mut thermal_gap_fraction = if lai + sai <= 1.0e-6 { 1.0 } else { 0.0 };
    let mut direct_extinction = 1.0;
    let mut diffuse_extinction = 0.718;

    let mut albedo = ground_state.ground;

    if lai + sai > 1.0e-6 && patch_type < 3 && (patch_type != 0 || use_lct) {
        let two_stream = match two_stream_kind {
            TwoStreamKind::LandCover { usgs_land_cover } => two_stream(
                optics,
                lai,
                sai,
                wet_snow_fraction,
                cosine_zenith,
                ground_state.ground,
                usgs_land_cover,
                vegetation_snow,
            )?,
            TwoStreamKind::Pft => two_stream_mod(
                optics,
                lai,
                sai,
                wet_snow_fraction,
                cosine_zenith,
                ground_state.ground,
                vegetation_snow,
            )?,
        };
        albedo = two_stream.albedo;
        transmission = two_stream.transmission;
        sunlit_absorption = two_stream.sunlit_absorption;
        shaded_absorption = two_stream.shaded_absorption;
        thermal_gap_fraction = two_stream.thermal_gap_fraction;
        direct_extinction = two_stream.direct_extinction;
        diffuse_extinction = two_stream.diffuse_extinction;
    }

    let mut soil_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut snow_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    for band in 0..BANDS {
        soil_absorption[band][0] = transmission[band][0] * (1.0 - soil_ground[band][1])
            + transmission[band][2] * (1.0 - soil_ground[band][0]);
        soil_absorption[band][1] = transmission[band][1] * (1.0 - soil_ground[band][1]);
        snow_absorption[band][0] = transmission[band][0] * (1.0 - snow[band][1])
            + transmission[band][2] * (1.0 - snow[band][0]);
        snow_absorption[band][1] = transmission[band][1] * (1.0 - snow[band][1]);
    }

    Ok(ColdStartRadiation {
        albedo,
        sunlit_absorption,
        shaded_absorption,
        soil_absorption,
        snow_absorption,
        snow_age,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
    })
}

/// Applies CoLM's cold-start soil/water and non-SNICAR snow albedo branches.
#[allow(clippy::too_many_arguments)]
pub fn cold_start_ground_albedo(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    cosine_zenith: f64,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<ColdStartGroundAlbedo> {
    ensure!(
        soil_liquid_water_kg_m2.is_finite()
            && soil_thickness_m.is_finite()
            && soil_thickness_m > 0.0
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&ground_snow_fraction)
            && ground_temperature_k.is_finite()
            && cosine_zenith.is_finite()
            && cosine_zenith > 0.0,
        "cold-start ground-albedo inputs are invalid"
    );
    for value in [
        soil.saturated_visible,
        soil.dry_visible,
        soil.saturated_near_infrared,
        soil.dry_near_infrared,
    ] {
        ensure!(value.is_finite(), "soil reflectance must be finite");
    }
    let soil = if patch_type <= 2 {
        let wetness = (1.0e-3 * soil_liquid_water_kg_m2 / soil_thickness_m).min(1.0);
        let increase = (0.11 - 0.40 * wetness).max(0.0);
        let visible = (soil.saturated_visible + increase).min(soil.dry_visible);
        let near_infrared = (soil.saturated_near_infrared + increase).min(soil.dry_near_infrared);
        [[visible; RADIATION_TYPES], [near_infrared; RADIATION_TYPES]]
    } else if patch_type == 3 {
        [[0.8; RADIATION_TYPES], [0.55; RADIATION_TYPES]]
    } else {
        let albedo_water = 0.05 / (cosine_zenith + 0.15);
        [[albedo_water, 0.1], [albedo_water, 0.1]]
    };
    let (snow, snow_age) =
        generic_snow_albedo(snow_depth_m * 250.0, ground_temperature_k, cosine_zenith)?;
    Ok(ColdStartGroundAlbedo {
        ground: mix_ground_albedo(soil, snow, ground_snow_fraction),
        soil,
        snow,
        snow_age,
    })
}

pub(crate) fn mix_ground_albedo(
    soil: [[f64; RADIATION_TYPES]; BANDS],
    snow: [[f64; RADIATION_TYPES]; BANDS],
    snow_fraction: f64,
) -> [[f64; RADIATION_TYPES]; BANDS] {
    std::array::from_fn(|band| {
        std::array::from_fn(|radiation_type| {
            (1.0 - snow_fraction) * soil[band][radiation_type]
                + snow_fraction * snow[band][radiation_type]
        })
    })
}

/// `albland`'s non-SNICAR snow-age/albedo branch for a freshly initialized column.
pub(crate) fn generic_snow_albedo(
    snow_water_equivalent_mm: f64,
    ground_temperature_k: f64,
    cosine_zenith: f64,
) -> Result<([[f64; RADIATION_TYPES]; BANDS], f64)> {
    if snow_water_equivalent_mm <= 0.0 {
        return Ok(([[1.0; RADIATION_TYPES]; BANDS], 0.0));
    }
    let snow_age = update_snow_age(
        1800.0,
        ground_temperature_k,
        snow_water_equivalent_mm,
        snow_water_equivalent_mm,
        0.0,
    )?;
    let age = 1.0 - 1.0 / (1.0 + snow_age);
    let direct_correction = ((1.5 / (1.0 + 4.0 * cosine_zenith)) - 0.5).max(0.0);
    let snow_band = |new_snow_albedo: f64, age_factor: f64| {
        let diffuse = new_snow_albedo * (1.0 - age_factor * age);
        let direct = diffuse + 0.4 * direct_correction * (1.0 - diffuse);
        [direct, diffuse]
    };
    Ok(([snow_band(0.85, 0.2), snow_band(0.65, 0.5)], snow_age))
}

struct TwoStreamRadiation {
    albedo: [[f64; RADIATION_TYPES]; BANDS],
    transmission: [[f64; 3]; BANDS],
    thermal_gap_fraction: f64,
    direct_extinction: f64,
    diffuse_extinction: f64,
    sunlit_absorption: [[f64; RADIATION_TYPES]; BANDS],
    shaded_absorption: [[f64; RADIATION_TYPES]; BANDS],
}

#[allow(clippy::too_many_arguments)]
fn two_stream(
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    ground: [[f64; RADIATION_TYPES]; BANDS],
    usgs_land_cover: bool,
    vegetation_snow: bool,
) -> Result<TwoStreamRadiation> {
    let phi1 = 0.5 - 0.633 * optics.chil - 0.33 * optics.chil * optics.chil;
    let phi2 = 0.877 * (1.0 - 2.0 * phi1);
    let projection = phi1 + phi2 * cosine_zenith;
    let direct_extinction = projection / cosine_zenith;
    let diffuse_extinction = 0.719;
    let zmu = if phi1.abs() > 1.0e-6 && phi2.abs() > 1.0e-6 {
        1.0 / phi2 * (1.0 - phi1 / phi2 * ((phi1 + phi2) / phi1).ln())
    } else if phi1.abs() <= 1.0e-6 {
        1.0 / 0.877
    } else {
        1.0 / (2.0 * phi1)
    };
    ensure!(
        zmu.is_finite() && zmu > 0.0,
        "invalid leaf angle distribution"
    );
    let stem_area = if usgs_land_cover { 0.0 } else { sai };
    let leaf_stem_area = lai + stem_area;
    let thermal_gap_fraction = (-(lai + sai).clamp(1.0e-5 * zmu, 50.0 * zmu) / zmu).exp();
    ensure!(
        leaf_stem_area > 1.0e-6,
        "two-stream canopy needs positive leaf or stem area"
    );

    let mut albedo = ground;
    let mut transmission = [[0.0; 3]; BANDS];
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    for band in 0..BANDS {
        let mut scattering = lai / leaf_stem_area
            * (optics.transmittance[band][0] + optics.reflectance[band][0])
            + stem_area / leaf_stem_area
                * (optics.transmittance[band][1] + optics.reflectance[band][1]);
        let directional_scattering = scattering / 2.0 * projection
            / (projection + cosine_zenith * phi2)
            * (1.0
                - cosine_zenith * phi1 / (projection + cosine_zenith * phi2)
                    * ((projection + cosine_zenith * phi2 + cosine_zenith * phi1)
                        / (cosine_zenith * phi1))
                        .ln());
        let mut upward_scattering = lai / leaf_stem_area * optics.transmittance[band][0]
            + stem_area / leaf_stem_area * optics.transmittance[band][1];
        upward_scattering = 0.5
            * (scattering
                + (scattering - 2.0 * upward_scattering) * ((1.0 + optics.chil) / 2.0).powi(2));
        let mut beta0 = (1.0 + zmu * direct_extinction) / (scattering * zmu * direct_extinction)
            * directional_scattering;
        if vegetation_snow {
            let snow_scattering = if band == 0 { 0.8 } else { 0.4 };
            scattering =
                (1.0 - wet_snow_fraction) * scattering + wet_snow_fraction * snow_scattering;
            upward_scattering = ((1.0 - wet_snow_fraction) * scattering * upward_scattering
                + wet_snow_fraction * snow_scattering * 0.5)
                / scattering;
            beta0 = ((1.0 - wet_snow_fraction) * scattering * beta0
                + wet_snow_fraction * snow_scattering * 0.5)
                / scattering;
        }

        let be = 1.0 - scattering + upward_scattering;
        let ce = upward_scattering;
        let de = scattering * zmu * direct_extinction * beta0;
        let fe = scattering * zmu * direct_extinction * (1.0 - beta0);
        let psi = (be.powi(2) - ce.powi(2)).sqrt() / zmu;
        let power1 = (psi * leaf_stem_area).min(50.0);
        let power2 = (direct_extinction * leaf_stem_area).min(50.0);
        let s1 = (-power1).exp();
        let s2 = (-power2).exp();
        let p1 = be + zmu * psi;
        let p2 = be - zmu * psi;
        let p3 = be + zmu * direct_extinction;
        let p4 = be - zmu * direct_extinction;
        let f1 = 1.0 - ground[band][1] * p1 / ce;
        let f2 = 1.0 - ground[band][1] * p2 / ce;
        let h1 = -(de * p4 + ce * fe);
        let h4 = -(fe * p3 + ce * de);
        let sigma = (zmu * direct_extinction).powi(2) + (ce.powi(2) - be.powi(2));
        let (albedo_direct, transmission_direct, eup_direct, edown_direct) = if sigma.abs()
            > 1.0e-10
        {
            let hh1 = h1 / sigma;
            let hh4 = h4 / sigma;
            let m1 = f1 * s1;
            let m2 = f2 / s1;
            let m3 = (ground[band][0] - (hh1 - ground[band][1] * hh4)) * s2;
            let n1 = p1 / ce;
            let n2 = p2 / ce;
            let n3 = -hh4;
            let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
            let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
            let hh5 = hh2 * p1 / ce;
            let hh6 = hh3 * p2 / ce;
            (
                hh1 + hh2 + hh3,
                hh4 * s2 + hh5 * s1 + hh6 / s1,
                hh1 * (1.0 - s2 * s2) / (2.0 * direct_extinction)
                    + hh2 * (1.0 - s1 * s2) / (direct_extinction + psi)
                    + hh3 * (1.0 - s2 / s1) / (direct_extinction - psi),
                hh4 * (1.0 - s2 * s2) / (2.0 * direct_extinction)
                    + hh5 * (1.0 - s1 * s2) / (direct_extinction + psi)
                    + hh6 * (1.0 - s2 / s1) / (direct_extinction - psi),
            )
        } else {
            let zmu2 = zmu * zmu;
            let m1 = f1 * s1;
            let m2 = f2 / s1;
            let m3 = h1 / zmu2 * (leaf_stem_area + 1.0 / (2.0 * direct_extinction)) * s2
                + ground[band][1] / ce
                    * (-h1 / (2.0 * direct_extinction) / zmu2
                        * (p3 * leaf_stem_area + p4 / (2.0 * direct_extinction))
                        - de)
                    * s2
                + ground[band][0] * s2;
            let n1 = p1 / ce;
            let n2 = p2 / ce;
            let n3 =
                1.0 / ce * (h1 * p4 / (4.0 * direct_extinction * direct_extinction) / zmu2 + de);
            let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
            let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
            let hh5 = hh2 * p1 / ce;
            let hh6 = hh3 * p2 / ce;
            (
                -h1 / (2.0 * direct_extinction * zmu2) + hh2 + hh3,
                1.0 / ce
                    * (-h1 / (2.0 * direct_extinction * zmu2)
                        * (p3 * leaf_stem_area + p4 / (2.0 * direct_extinction))
                        - de)
                    * s2
                    + hh5 * s1
                    + hh6 / s1,
                (hh2 - h1 / (2.0 * direct_extinction * zmu2)) * (1.0 - s2 * s2)
                    / (2.0 * direct_extinction)
                    + hh3 * leaf_stem_area
                    + h1 / (2.0 * direct_extinction * zmu2)
                        * (leaf_stem_area * s2 * s2 - (1.0 - s2 * s2) / (2.0 * direct_extinction)),
                (hh5 - (h1 * p4 / (4.0 * direct_extinction * direct_extinction * zmu) + de) / ce)
                    * (1.0 - s2 * s2)
                    / (2.0 * direct_extinction)
                    + hh6 * leaf_stem_area
                    + h1 * p3 / (ce * 4.0 * direct_extinction * direct_extinction * zmu2)
                        * (leaf_stem_area * s2 * s2 - (1.0 - s2 * s2) / (2.0 * direct_extinction)),
            )
        };
        sunlit_absorption[band][0] =
            (1.0 - scattering) * (1.0 - s2 + (eup_direct + edown_direct) / zmu);
        shaded_absorption[band][0] = scattering * (1.0 - s2)
            + (ground[band][1] * transmission_direct + ground[band][0] * s2 - transmission_direct)
            - albedo_direct
            - (1.0 - scattering) * (eup_direct + edown_direct) / zmu;
        albedo[band][0] = albedo_direct;
        transmission[band][0] = transmission_direct;

        let m1 = f1 * s1;
        let m2 = f2 / s1;
        let n1 = p1 / ce;
        let n2 = p2 / ce;
        let hh7 = -m2 / (m1 * n2 - m2 * n1);
        let hh8 = -m1 / (m2 * n1 - m1 * n2);
        let hh9 = hh7 * p1 / ce;
        let hh10 = hh8 * p2 / ce;
        let transmission_diffuse = hh9 * s1 + hh10 / s1;
        let (eup_diffuse, edown_diffuse) = if sigma.abs() > 1.0e-10 {
            (
                hh7 * (1.0 - s1 * s2) / (direct_extinction + psi)
                    + hh8 * (1.0 - s2 / s1) / (direct_extinction - psi),
                hh9 * (1.0 - s1 * s2) / (direct_extinction + psi)
                    + hh10 * (1.0 - s2 / s1) / (direct_extinction - psi),
            )
        } else {
            (
                hh7 * (1.0 - s1 * s2) / (direct_extinction + psi) + hh8 * leaf_stem_area,
                hh9 * (1.0 - s1 * s2) / (direct_extinction + psi) + hh10 * leaf_stem_area,
            )
        };
        let albedo_diffuse = hh7 + hh8;
        sunlit_absorption[band][1] = (1.0 - scattering) * (eup_diffuse + edown_diffuse) / zmu;
        shaded_absorption[band][1] = transmission_diffuse * (ground[band][1] - 1.0)
            - (albedo_diffuse - 1.0)
            - (1.0 - scattering) * (eup_diffuse + edown_diffuse) / zmu;
        albedo[band][1] = albedo_diffuse;
        transmission[band][1] = transmission_diffuse;
        transmission[band][2] = s2;
    }

    Ok(TwoStreamRadiation {
        albedo,
        transmission,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
        sunlit_absorption,
        shaded_absorption,
    })
}

/// PFT-vector `twostream_mod`, distinct from the land-cover `twostream` above.
fn two_stream_mod(
    optics: LeafOptics,
    lai: f64,
    sai: f64,
    wet_snow_fraction: f64,
    cosine_zenith: f64,
    ground: [[f64; RADIATION_TYPES]; BANDS],
    vegetation_snow: bool,
) -> Result<TwoStreamRadiation> {
    let phi1 = 0.5 - 0.633 * optics.chil - 0.33 * optics.chil * optics.chil;
    let phi2 = 0.877 * (1.0 - 2.0 * phi1);
    let zmu = if phi1.abs() > 1.0e-6 && phi2.abs() > 1.0e-6 {
        1.0 / phi2 * (1.0 - phi1 / phi2 * ((phi1 + phi2) / phi1).ln())
    } else if phi1.abs() <= 1.0e-6 {
        1.0 / 0.877
    } else {
        1.0 / (2.0 * phi1)
    };
    ensure!(
        zmu.is_finite() && zmu > 0.0 && lai + sai > 1.0e-6,
        "invalid PFT two-stream canopy"
    );
    let leaf_stem_area = lai + sai;
    let thermal_gap_fraction = (-(leaf_stem_area / zmu).clamp(1.0e-5, 50.0)).exp();
    let cosine_diffuse = {
        let half_area = 0.5 * leaf_stem_area;
        -half_area / ((-0.87 * half_area).exp() / (1.0 + 0.92 * half_area)).ln()
    };
    let mut albedo = [[0.0; RADIATION_TYPES]; BANDS];
    let mut transmission = [[0.0; 3]; BANDS];
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut direct_extinction = 0.0;

    for band in 0..BANDS {
        let mut absorption = [0.0; RADIATION_TYPES];
        let mut direct_transmission = 0.0;
        let mut diffuse_transmission = 0.0;
        let mut direct_extinction_band = 0.0;
        let mut reverse_sunlit = 0.0;
        for incidence in 0..RADIATION_TYPES {
            let cosine = if incidence == 0 {
                cosine_zenith
            } else {
                let theta =
                    cosine_diffuse.max(0.001).acos() / FORTRAN_PI * 180.0 + optics.chil * 5.0;
                (theta / 180.0 * FORTRAN_PI).cos()
            };
            let projection = phi1 + phi2 * cosine;
            let extinction = projection / cosine;
            let leaf_transmittance = lai / leaf_stem_area * optics.transmittance[band][0]
                + sai / leaf_stem_area * optics.transmittance[band][1];
            let leaf_reflectance = lai / leaf_stem_area * optics.reflectance[band][0]
                + sai / leaf_stem_area * optics.reflectance[band][1];
            let mut scattering = leaf_transmittance + leaf_reflectance;
            let mut upward = 0.5
                * (scattering
                    + (scattering - 2.0 * leaf_transmittance)
                        * ((1.0 + optics.chil) / 2.0).powi(2));
            let mut beta0 = 0.5
                * (scattering
                    + (1.0 + optics.chil).powi(2) / (4.0 * extinction)
                        * (leaf_reflectance - leaf_transmittance))
                / scattering;
            if vegetation_snow {
                let snow_scattering = if band == 0 { 0.8 } else { 0.4 };
                scattering =
                    (1.0 - wet_snow_fraction) * scattering + wet_snow_fraction * snow_scattering;
                upward = ((1.0 - wet_snow_fraction) * scattering * upward
                    + wet_snow_fraction * snow_scattering * 0.5)
                    / scattering;
                beta0 = ((1.0 - wet_snow_fraction) * scattering * beta0
                    + wet_snow_fraction * snow_scattering * 0.5)
                    / scattering;
            }
            let be = 1.0 - scattering + upward;
            let ce = upward;
            let de = scattering * zmu * extinction * beta0;
            let fe = scattering * zmu * extinction * (1.0 - beta0);
            let psi = (be.powi(2) - ce.powi(2)).sqrt() / zmu;
            let s1 = (-(psi * leaf_stem_area).min(50.0)).exp();
            let s2 = (-(extinction * leaf_stem_area).min(50.0)).exp();
            let p1 = be + zmu * psi;
            let p2 = be - zmu * psi;
            let p3 = be + zmu * extinction;
            let p4 = be - zmu * extinction;
            let black = 1.0e-6;
            let f1 = 1.0 - black * p1 / ce;
            let f2 = 1.0 - black * p2 / ce;
            let h1 = -(de * p4 + ce * fe);
            let h4 = -(fe * p3 + ce * de);
            let sigma = (zmu * extinction).powi(2) + ce.powi(2) - be.powi(2);
            if incidence == 0 {
                direct_transmission = s2;
                direct_extinction_band = extinction;
            }
            let (hh1, hh2, hh3, hh4, hh5, hh6, up, down) = if sigma.abs() > 1.0e-10 {
                let hh1 = h1 / sigma;
                let hh4 = h4 / sigma;
                let m1 = f1 * s1;
                let m2 = f2 / s1;
                let m3 = (black - (hh1 - black * hh4)) * s2;
                let n1 = p1 / ce;
                let n2 = p2 / ce;
                let n3 = -hh4;
                let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
                let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
                let hh5 = hh2 * p1 / ce;
                let hh6 = hh3 * p2 / ce;
                let up = hh1 * (1.0 - s2 * direct_transmission)
                    / (direct_extinction_band + extinction)
                    + hh2 * (1.0 - direct_transmission * s1) / (direct_extinction_band + psi)
                    + hh3 * (1.0 - direct_transmission / s1) / (direct_extinction_band - psi);
                let down = hh4 * (1.0 - s2 * direct_transmission)
                    / (direct_extinction_band + extinction)
                    + hh5 * (1.0 - direct_transmission * s1) / (direct_extinction_band + psi)
                    + hh6 * (1.0 - direct_transmission / s1) / (direct_extinction_band - psi);
                (hh1, hh2, hh3, hh4, hh5, hh6, up, down)
            } else {
                let zmu2 = zmu * zmu;
                let m1 = f1 * s1;
                let m2 = f2 / s1;
                let sum_extinction = extinction + direct_extinction_band;
                let m3 = h1 / zmu2 * (leaf_stem_area + 1.0 / sum_extinction) * s2
                    + black / ce
                        * (-h1 / sum_extinction / zmu2
                            * (p3 * leaf_stem_area + p4 / sum_extinction)
                            - de)
                        * s2
                    + black * s2;
                let n1 = p1 / ce;
                let n2 = p2 / ce;
                let n3 = (h1 * p4 / (sum_extinction * sum_extinction) / zmu2 + de) / ce;
                let hh2 = (m3 * n2 - m2 * n3) / (m1 * n2 - m2 * n1);
                let hh3 = (m3 * n1 - m1 * n3) / (m2 * n1 - m1 * n2);
                let hh5 = hh2 * p1 / ce;
                let hh6 = hh3 * p2 / ce;
                let hh1 = -h1 / (sum_extinction * zmu2);
                let hh4 = (-h1 / (sum_extinction * zmu2)
                    * (p3 * leaf_stem_area + p4 / sum_extinction)
                    - de)
                    / ce;
                let up = (hh2 - h1 / (sum_extinction * zmu2)) * (1.0 - s2 * direct_transmission)
                    / sum_extinction
                    + hh3 * leaf_stem_area
                    + h1 / (sum_extinction * zmu2)
                        * (leaf_stem_area * s2 * direct_transmission
                            - (1.0 - s2 * direct_transmission) / sum_extinction);
                let down = (hh5 - (h1 * p4 / (sum_extinction * sum_extinction * zmu) + de) / ce)
                    * (1.0 - s2 * direct_transmission)
                    / sum_extinction
                    + hh6 * leaf_stem_area
                    + h1 * p3 / (ce * sum_extinction * sum_extinction * zmu2)
                        * (leaf_stem_area * s2 * direct_transmission
                            - (1.0 - s2 * direct_transmission) / sum_extinction);
                (hh1, hh2, hh3, hh4, hh5, hh6, up, down)
            };
            albedo[band][incidence] = hh1 + hh2 + hh3;
            transmission[band][incidence] = hh4 * s2 + hh5 * s1 + hh6 / s1;
            absorption[incidence] = 1.0
                - albedo[band][incidence]
                - (1.0 - black) * (transmission[band][incidence] + s2);
            sunlit_absorption[band][incidence] = if incidence == 0 {
                (1.0 - scattering) * (1.0 - s2 + (up + down) / zmu)
            } else {
                (1.0 - scattering)
                    * (extinction * (1.0 - s2 * direct_transmission)
                        / (extinction + direct_extinction_band)
                        + (up + down) / zmu)
            };
            shaded_absorption[band][incidence] =
                absorption[incidence] - sunlit_absorption[band][incidence];
            if incidence == 1 {
                let up = hh1 * (1.0 - s2 / direct_transmission)
                    / (extinction - direct_extinction_band)
                    + hh2 * (1.0 - s1 / direct_transmission) / (psi - direct_extinction_band)
                    + hh3 * (1.0 / s1 / direct_transmission - 1.0) / (psi + direct_extinction_band);
                let down = hh4 * (1.0 - s2 / direct_transmission)
                    / (extinction - direct_extinction_band)
                    + hh5 * (1.0 - s1 / direct_transmission) / (psi - direct_extinction_band)
                    + hh6 * (1.0 / s1 / direct_transmission - 1.0) / (psi + direct_extinction_band);
                reverse_sunlit = direct_transmission
                    * (1.0 - scattering)
                    * (extinction * (1.0 - s2 / direct_transmission)
                        / (extinction - direct_extinction_band)
                        + (up + down) / zmu);
                diffuse_transmission = s2;
            }
        }
        let diffuse_albedo = albedo[band][1];
        let diffuse_absorption = absorption[1];
        let q = ground[band][1] * diffuse_albedo;
        transmission[band][0] = (direct_transmission * ground[band][0] * diffuse_albedo
            + transmission[band][0])
            / (1.0 - q);
        let correction =
            transmission[band][0] * ground[band][1] + direct_transmission * ground[band][0];
        absorption[0] += correction * diffuse_absorption;
        albedo[band][0] = 1.0
            - absorption[0]
            - (1.0 - ground[band][1]) * transmission[band][0]
            - (1.0 - ground[band][0]) * direct_transmission;
        sunlit_absorption[band][0] += correction * reverse_sunlit;
        shaded_absorption[band][0] = absorption[0] - sunlit_absorption[band][0];
        transmission[band][1] = (transmission[band][1] + diffuse_transmission) / (1.0 - q);
        absorption[1] += transmission[band][1] * ground[band][1] * diffuse_absorption;
        albedo[band][1] = 1.0 - absorption[1] - (1.0 - ground[band][1]) * transmission[band][1];
        sunlit_absorption[band][1] += transmission[band][1] * ground[band][1] * reverse_sunlit;
        shaded_absorption[band][1] = absorption[1] - sunlit_absorption[band][1];
        transmission[band][2] = direct_transmission;
        direct_extinction = direct_extinction_band;
    }
    Ok(TwoStreamRadiation {
        albedo,
        transmission,
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction: 0.719,
        sunlit_absorption,
        shaded_absorption,
    })
}

#[cfg(test)]
#[path = "radiation_tests.rs"]
mod radiation_tests;
