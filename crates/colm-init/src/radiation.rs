//! Broadband cold-start radiation from `MOD_Albedo.F90`.
//!
//! The arrays use CoLM's native `(band, radiation_type)` layout.  NetCDF reversal
//! remains the responsibility of the restart writer.

use anyhow::{ensure, Result};

use crate::{LandCoverScheme, SoilReflectance};

const BANDS: usize = 2;
const RADIATION_TYPES: usize = 2;

/// Per-land-class optical constants passed by `MOD_Const_LC` to `twostream`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeafOptics {
    pub chil: f64,
    /// `[band][green leaf, dead stem]`.
    pub reflectance: [[f64; BANDS]; BANDS],
    /// `[band][green leaf, dead stem]`.
    pub transmittance: [[f64; BANDS]; BANDS],
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

/// Broadband arrays produced by the snow-free cold-start `albland` path.
#[derive(Debug, Clone, PartialEq)]
pub struct ColdStartRadiation {
    /// `[band][direct, diffuse]`.
    pub albedo: [[f64; RADIATION_TYPES]; BANDS],
    pub sunlit_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub shaded_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub soil_absorption: [[f64; RADIATION_TYPES]; BANDS],
    pub snow_absorption: [[f64; RADIATION_TYPES]; BANDS],
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

    let ground;
    let snow = [[1.0; RADIATION_TYPES]; BANDS];
    let mut sunlit_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut shaded_absorption = [[0.0; RADIATION_TYPES]; BANDS];
    let mut transmission = [[0.0, 1.0, 1.0]; BANDS];
    let mut thermal_gap_fraction = if lai + sai <= 1.0e-6 { 1.0 } else { 0.0 };
    let mut direct_extinction = 1.0;
    let mut diffuse_extinction = 0.718;

    if patch_type <= 2 {
        let wetness = (1.0e-3 * soil_liquid_water_kg_m2 / soil_thickness_m).min(1.0);
        let increase = (0.11 - 0.40 * wetness).max(0.0);
        let visible = (soil.saturated_visible + increase).min(soil.dry_visible);
        let near_infrared = (soil.saturated_near_infrared + increase).min(soil.dry_near_infrared);
        ground = [[visible; RADIATION_TYPES], [near_infrared; RADIATION_TYPES]];
    } else if patch_type == 3 {
        ground = [[0.8; RADIATION_TYPES], [0.55; RADIATION_TYPES]];
    } else {
        let albedo_water = 0.05 / (cosine_zenith + 0.15);
        ground = [[albedo_water, 0.1], [albedo_water, 0.1]];
    }
    let soil_ground = ground;
    let mut albedo = ground;

    if lai + sai > 1.0e-6 && patch_type < 3 && (patch_type != 0 || use_lct) {
        let two_stream = two_stream(
            optics,
            lai,
            sai,
            wet_snow_fraction,
            cosine_zenith,
            ground,
            usgs_land_cover,
            vegetation_snow,
        )?;
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
        thermal_gap_fraction,
        direct_extinction,
        diffuse_extinction,
    })
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

#[cfg(test)]
#[path = "radiation_tests.rs"]
mod radiation_tests;
