//! Cold-start port of `MOD_3DCanopyRadiation.F90` for plant communities.

use anyhow::{ensure, Result};

use crate::{
    radiation::{generic_snow_albedo, mix_ground_albedo},
    ColdStartGroundAlbedo, ColdStartRadiation, LeafOptics, SoilReflectance, MISSING,
};

const BANDS: usize = 2;
const RTYPES: usize = 2;
const LAYERS: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PcPftInput {
    /// Original `canlay_p`: zero is an inactive bare-ground sentinel.
    pub canopy_layer: usize,
    pub fraction: f64,
    pub canopy_top_m: f64,
    pub canopy_bottom_m: f64,
    pub optics: LeafOptics,
    pub lai: f64,
    pub sai: f64,
    pub wet_snow_fraction: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PcPftRadiation {
    pub sunlit_absorption: [[f64; RTYPES]; BANDS],
    pub shaded_absorption: [[f64; RTYPES]; BANDS],
    pub thermal_gap_fraction: f64,
    pub shade_fraction: f64,
    pub direct_extinction: f64,
    pub diffuse_extinction: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PcCanopyRadiation {
    pub common: ColdStartRadiation,
    pub pft: Vec<PcPftRadiation>,
}

#[allow(clippy::too_many_arguments)]
pub fn cold_start_pc_broadband_radiation_with_snow(
    patch_type: i32,
    soil: SoilReflectance,
    soil_liquid_water_kg_m2: f64,
    soil_thickness_m: f64,
    pfts: &[PcPftInput],
    cosine_zenith: f64,
    snow_depth_m: f64,
    ground_snow_fraction: f64,
    ground_temperature_k: f64,
) -> Result<PcCanopyRadiation> {
    ensure!(
        patch_type == 0
            && soil_liquid_water_kg_m2.is_finite()
            && soil_thickness_m.is_finite()
            && soil_thickness_m > 0.0
            && snow_depth_m.is_finite()
            && snow_depth_m >= 0.0
            && ground_snow_fraction.is_finite()
            && (0.0..=1.0).contains(&ground_snow_fraction)
            && ground_temperature_k.is_finite(),
        "PC cold-start radiation inputs are invalid"
    );
    let (soil_ground, snow, ground, snow_age) = ground_albedos(
        soil,
        soil_liquid_water_kg_m2,
        soil_thickness_m,
        cosine_zenith,
        snow_depth_m,
        ground_snow_fraction,
        ground_temperature_k,
    )?;
    cold_start_pc_broadband_radiation_from_ground(
        pfts,
        cosine_zenith,
        ColdStartGroundAlbedo {
            soil: soil_ground,
            snow,
            ground,
            snow_age,
        },
    )
}

/// Runs PC cold-start canopy radiation with an already-resolved ground state.
///
/// HYPERSPECTRAL initialization obtains this state from CoLM's 211-band soil
/// spectrum before the PC three-dimensional canopy solver is invoked.
pub fn cold_start_pc_broadband_radiation_from_ground(
    pfts: &[PcPftInput],
    cosine_zenith: f64,
    ground_state: ColdStartGroundAlbedo,
) -> Result<PcCanopyRadiation> {
    let ColdStartGroundAlbedo {
        soil: soil_ground,
        snow,
        ground,
        snow_age,
    } = ground_state;
    ensure!(
        cosine_zenith.is_finite()
            && cosine_zenith > 0.0
            && snow_age.is_finite()
            && !pfts.is_empty()
            && soil_ground
                .iter()
                .chain(&snow)
                .chain(&ground)
                .flatten()
                .all(|value| value.is_finite()),
        "PC cold-start radiation inputs are invalid"
    );
    for pft in pfts {
        ensure!(
            ((1..=LAYERS).contains(&pft.canopy_layer)
                || (pft.canopy_layer == 0 && (pft.lai + pft.sai <= 1.0e-6 || pft.fraction == 0.0)))
                && pft.fraction.is_finite()
                && pft.fraction >= 0.0
                && pft.canopy_top_m.is_finite()
                && pft.canopy_bottom_m.is_finite()
                && pft.canopy_top_m >= pft.canopy_bottom_m
                && pft.lai.is_finite()
                && pft.lai >= 0.0
                && pft.sai.is_finite()
                && pft.sai >= 0.0
                && pft.wet_snow_fraction.is_finite()
                && (0.0..=1.0).contains(&pft.wet_snow_fraction),
            "PC PFT input is invalid"
        );
    }
    let fraction_sum: f64 = pfts.iter().map(|pft| pft.fraction).sum();
    ensure!(
        fraction_sum > 0.0,
        "PC PFT fractions must have a positive sum"
    );
    let fractions = pfts
        .iter()
        .map(|pft| pft.fraction / fraction_sum)
        .collect::<Vec<_>>();
    let core = three_d_canopy(pfts, &fractions, cosine_zenith, ground)?;
    let pft = (0..pfts.len())
        .map(|index| PcPftRadiation {
            sunlit_absorption: core.sunlit[index],
            shaded_absorption: core.shaded[index],
            thermal_gap_fraction: core.thermal_gap[index],
            shade_fraction: core.shade[index],
            direct_extinction: core.direct_extinction[index],
            diffuse_extinction: 0.719,
        })
        .collect::<Vec<_>>();
    let weighted = |select: fn(&PcPftRadiation) -> [[f64; RTYPES]; BANDS]| {
        std::array::from_fn(|band| {
            std::array::from_fn(|rtyp| {
                pft.iter()
                    .zip(&fractions)
                    .map(|(state, fraction)| select(state)[band][rtyp] * fraction)
                    .sum()
            })
        })
    };
    let mut soil_absorption = [[0.0; RTYPES]; BANDS];
    let mut snow_absorption = [[0.0; RTYPES]; BANDS];
    for band in 0..BANDS {
        soil_absorption[band][0] = core.transmission[band][0] * (1.0 - soil_ground[band][1])
            + core.transmission[band][2] * (1.0 - soil_ground[band][0]);
        soil_absorption[band][1] = core.transmission[band][1] * (1.0 - soil_ground[band][1]);
        snow_absorption[band][0] = core.transmission[band][0] * (1.0 - snow[band][1])
            + core.transmission[band][2] * (1.0 - snow[band][0]);
        snow_absorption[band][1] = core.transmission[band][1] * (1.0 - snow[band][1]);
    }
    let leaf_stem_area: f64 = pfts
        .iter()
        .zip(&fractions)
        .map(|(pft, fraction)| (pft.lai + pft.sai) * fraction)
        .sum();
    Ok(PcCanopyRadiation {
        common: ColdStartRadiation {
            albedo: core.albedo,
            sunlit_absorption: weighted(|state| state.sunlit_absorption),
            shaded_absorption: weighted(|state| state.shaded_absorption),
            soil_absorption,
            snow_absorption,
            transmission: Some(core.transmission),
            snow_age,
            thermal_gap_fraction: if leaf_stem_area <= 1.0e-6 {
                1.0
            } else {
                MISSING
            },
            direct_extinction: 1.0,
            diffuse_extinction: 0.718,
        },
        pft,
    })
}

struct CoreRadiation {
    albedo: [[f64; RTYPES]; BANDS],
    transmission: [[f64; 3]; BANDS],
    sunlit: Vec<[[f64; RTYPES]; BANDS]>,
    shaded: Vec<[[f64; RTYPES]; BANDS]>,
    thermal_gap: Vec<f64>,
    shade: Vec<f64>,
    direct_extinction: Vec<f64>,
}

fn three_d_canopy(
    pfts: &[PcPftInput],
    fractions: &[f64],
    cosine_zenith: f64,
    ground: [[f64; RTYPES]; BANDS],
) -> Result<CoreRadiation> {
    const GEE: f64 = 0.5;
    const COSINE_DIFFUSE: f64 = 0.5;
    const EPSILON: f64 = 1.0e-6;
    ensure!(
        pfts.len() == fractions.len(),
        "PC PFT fractions do not match inputs"
    );

    let count = pfts.len();
    let mut active = vec![false; count];
    let mut canopy = vec![0usize; count];
    let mut leaf_stem_area = vec![0.0; count];
    let mut rho = vec![[0.0; BANDS]; count];
    let mut tau = vec![[0.0; BANDS]; count];
    let cosz = vec![cosine_zenith; count];
    let cosd = vec![COSINE_DIFFUSE; count];
    let mut gdir = vec![0.0; count];
    let mut gdif = vec![0.0; count];
    let mut direct_extinction = vec![1.0; count];
    let mut cover = [[0.0; LAYERS]; 1];
    let mut crown_size = [0.0; LAYERS];
    let mut crown_height = [0.0; LAYERS];
    let mut layer_lsai = [0.0; LAYERS];
    let mut layer_cosz = [0.0; LAYERS];
    let mut layer_cosd = [0.0; LAYERS];
    let mut layer_gdir = [0.0; LAYERS];
    let mut layer_gdif = [0.0; LAYERS];
    let mut layer_rho = [[0.0; BANDS]; LAYERS];
    let mut layer_tau = [[0.0; BANDS]; LAYERS];
    let mut layer_omega = [[0.0; BANDS]; LAYERS];

    for (index, (pft, &fraction)) in pfts.iter().zip(fractions).enumerate() {
        let lsai = pft.lai + pft.sai;
        leaf_stem_area[index] = lsai;
        let phi1 = 0.5 - 0.633 * pft.optics.chil - 0.33 * pft.optics.chil * pft.optics.chil;
        let phi2 = 0.877 * (1.0 - 2.0 * phi1);
        gdir[index] = phi1 + phi2 * cosz[index];
        gdif[index] = phi1 + phi2 * cosd[index];
        direct_extinction[index] = gdir[index] / cosz[index];
        if lsai <= EPSILON || fraction <= 0.0 {
            continue;
        }
        active[index] = true;
        canopy[index] = pft.canopy_layer - 1;
        let layer = canopy[index];
        cover[0][layer] += fraction;
        let size = (pft.canopy_top_m - pft.canopy_bottom_m) * 0.5;
        let height = (pft.canopy_top_m + pft.canopy_bottom_m) * 0.5;
        crown_size[layer] += fraction * size;
        crown_height[layer] += fraction * height;
        layer_lsai[layer] += fraction * lsai;
        layer_cosz[layer] += fraction * cosz[index];
        layer_cosd[layer] += fraction * cosd[index];
        layer_gdir[layer] += fraction * gdir[index];
        layer_gdif[layer] += fraction * gdif[index];
        for band in 0..BANDS {
            let leaf_weight = pft.lai / lsai;
            let stem_weight = pft.sai / lsai;
            rho[index][band] = leaf_weight * pft.optics.reflectance[band][0]
                + stem_weight * pft.optics.reflectance[band][1];
            tau[index][band] = leaf_weight * pft.optics.transmittance[band][0]
                + stem_weight * pft.optics.transmittance[band][1];
            rho[index][band] = (1.0 - pft.wet_snow_fraction) * rho[index][band]
                + pft.wet_snow_fraction * if band == 0 { 0.5 } else { 0.2 };
            tau[index][band] = (1.0 - pft.wet_snow_fraction) * tau[index][band]
                + pft.wet_snow_fraction * if band == 0 { 0.3 } else { 0.2 };
            layer_rho[layer][band] += fraction * rho[index][band];
            layer_tau[layer][band] += fraction * tau[index][band];
            layer_omega[layer][band] += fraction * (rho[index][band] + tau[index][band]);
        }
    }
    if !active.iter().any(|&value| value) {
        return Ok(CoreRadiation {
            albedo: ground,
            transmission: [[0.0, 1.0, 1.0]; BANDS],
            sunlit: vec![[[0.0; RTYPES]; BANDS]; count],
            shaded: vec![[[0.0; RTYPES]; BANDS]; count],
            thermal_gap: vec![1.0; count],
            shade: vec![0.0; count],
            direct_extinction,
        });
    }
    let mut bottom = [0.0; LAYERS];
    for layer in 0..LAYERS {
        if cover[0][layer] > 0.0 {
            let inverse_cover = 1.0 / cover[0][layer];
            crown_size[layer] = (crown_size[layer] * inverse_cover).max(0.0);
            crown_height[layer] = (crown_height[layer] * inverse_cover).max(0.0);
            bottom[layer] = crown_height[layer] - crown_size[layer];
            layer_lsai[layer] = (layer_lsai[layer] * inverse_cover).max(0.0);
            layer_cosz[layer] = (layer_cosz[layer] * inverse_cover).max(0.0);
            layer_cosd[layer] = (layer_cosd[layer] * inverse_cover).max(0.0);
            layer_gdir[layer] = (layer_gdir[layer] * inverse_cover).max(0.0);
            layer_gdif[layer] = (layer_gdif[layer] * inverse_cover).max(0.0);
            for band in 0..BANDS {
                layer_rho[layer][band] = (layer_rho[layer][band] * inverse_cover).max(0.0);
                layer_tau[layer][band] = (layer_tau[layer][band] * inverse_cover).max(0.0);
                layer_omega[layer][band] = (layer_omega[layer][band] * inverse_cover).max(0.0);
            }
        }
    }

    let mut shadow_direct = [0.0; LAYERS];
    let mut shadow_diffuse = [0.0; LAYERS];
    let mut direct_depth = [0.0; LAYERS];
    let mut diffuse_depth = [0.0; LAYERS];
    let mut direct_unscattered = [0.0; LAYERS];
    let mut diffuse_unscattered = [0.0; LAYERS];
    let mut direct_unscattered_original = [0.0; LAYERS];
    let mut diffuse_unscattered_original = [0.0; LAYERS];
    let mut direct_calibration = [1.0; LAYERS];
    let mut diffuse_calibration = [1.0; LAYERS];
    let mut sunlit_direct_weight = [0.0; LAYERS];
    let mut sunlit_downward_weight = [0.0; LAYERS];
    let mut sunlit_upward_weight = [0.0; LAYERS];
    for layer in 0..LAYERS {
        if cover[0][layer] <= 0.0 || layer_cosz[layer] <= 0.0 {
            continue;
        }
        shadow_direct[layer] = (1.0 - (-cover[0][layer] / layer_cosz[layer]).exp())
            / (1.0 - cover[0][layer] * (-1.0 / layer_cosz[layer]).exp());
        shadow_direct[layer] = shadow_direct[layer].max(cover[0][layer]);
        shadow_diffuse[layer] = (1.0 - (-cover[0][layer] / layer_cosd[layer]).exp())
            / (1.0 - cover[0][layer] * (-1.0 / layer_cosd[layer]).exp());
        shadow_diffuse[layer] = shadow_diffuse[layer].max(cover[0][layer]);
        if layer_lsai[layer] <= 0.0 {
            continue;
        }
        direct_depth[layer] = 0.75 * GEE * cover[0][layer] * layer_lsai[layer]
            / (layer_cosz[layer] * shadow_direct[layer]);
        diffuse_depth[layer] = 0.75 * GEE * cover[0][layer] * layer_lsai[layer]
            / (layer_cosd[layer] * shadow_diffuse[layer]);
        direct_unscattered_original[layer] = canopy_transmittance(direct_depth[layer]);
        diffuse_unscattered_original[layer] = canopy_transmittance(diffuse_depth[layer]);
        direct_unscattered[layer] =
            canopy_transmittance(direct_depth[layer] / GEE * layer_gdir[layer]);
        diffuse_unscattered[layer] =
            canopy_transmittance(diffuse_depth[layer] / GEE * layer_gdif[layer]);
        direct_calibration[layer] =
            (1.0 - direct_unscattered[layer]) / (1.0 - direct_unscattered_original[layer]);
        diffuse_calibration[layer] =
            (1.0 - diffuse_unscattered[layer]) / (1.0 - diffuse_unscattered_original[layer]);
        let forward = 0.5 * (1.0 - canopy_transmittance(2.0 * direct_depth[layer]))
            / (1.0 - canopy_transmittance(direct_depth[layer]));
        let backward = 2.0
            * (canopy_transmittance(direct_depth[layer]) - (-2.0 * direct_depth[layer]).exp())
            / (1.0 - canopy_transmittance(direct_depth[layer]));
        let average = 0.5 * (forward + backward);
        let difference = 0.5 * (forward - backward);
        sunlit_direct_weight[layer] = forward;
        sunlit_downward_weight[layer] = average + 0.5 * layer_cosz[layer] * difference;
        sunlit_upward_weight[layer] = average - 0.5 * layer_cosz[layer] * difference;
    }

    let mut overlap = [[0.0; LAYERS]; LAYERS];
    overlap[2][1] = cover[0][2]
        * overlap_area(
            crown_size[2],
            crown_height[2] - bottom[1],
            layer_cosz[2].acos(),
        );
    overlap[2][0] = cover[0][2]
        * overlap_area(
            crown_size[2],
            crown_height[2] - bottom[0],
            layer_cosz[2].acos(),
        );
    overlap[1][0] = cover[0][1]
        * overlap_area(
            crown_size[1],
            crown_height[1] - bottom[0],
            layer_cosz[1].acos(),
        );
    let mut tt = [[0.0; 5]; 5];
    let clamp_between = |value: f64, low: f64, high: f64| value.max(low).min(high);
    tt[4][3] = clamp_between(shadow_direct[2], 0.0, 1.0);
    tt[4][2] = clamp_between(
        shadow_direct[1] * (1.0 - shadow_direct[2] + overlap[2][1]),
        0.0,
        1.0 - tt[4][3],
    );
    tt[4][1] = clamp_between(
        shadow_direct[0]
            * (1.0 - (shadow_direct[1] - overlap[1][0]) - (shadow_direct[2] - overlap[2][0])
                + (shadow_direct[1] - overlap[1][0]) * (shadow_direct[2] - overlap[2][1])),
        0.0,
        1.0 - tt[4][3] - tt[4][2],
    );
    tt[4][0] = clamp_between(
        1.0 - (shadow_direct[0] + shadow_direct[1] + shadow_direct[2]
            - (shadow_direct[1] - overlap[1][0]) * shadow_direct[0]
            - (shadow_direct[2] - overlap[2][1]) * shadow_direct[1]
            - (shadow_direct[2] - overlap[2][0]) * shadow_direct[0]
            + (shadow_direct[1] - overlap[1][0])
                * (shadow_direct[2] - overlap[2][1])
                * shadow_direct[0]),
        0.0,
        1.0 - tt[4][3] - tt[4][2] - tt[4][1],
    );
    if shadow_direct[2] > 0.0 {
        tt[3][2] = clamp_between(
            shadow_direct[1] * (shadow_direct[2] - overlap[2][1]),
            0.0,
            shadow_direct[2],
        );
        tt[3][1] = clamp_between(
            shadow_direct[0]
                * (shadow_direct[2]
                    - overlap[2][0]
                    - (shadow_direct[2] - overlap[2][1]) * (shadow_direct[1] - overlap[1][0])),
            0.0,
            shadow_direct[2] - tt[3][2],
        );
        tt[3][0] = shadow_direct[2] - tt[3][2] - tt[3][1];
        tt[3][2] *= direct_unscattered[2];
        tt[3][1] *= direct_unscattered[2];
        tt[3][0] *= direct_unscattered[2];
    }
    if shadow_direct[1] > 0.0 {
        tt[2][1] = clamp_between(
            shadow_direct[0] * (shadow_direct[1] - overlap[1][0]),
            0.0,
            shadow_direct[1],
        );
        tt[2][0] = shadow_direct[1] - tt[2][1];
        tt[2][1] *= direct_unscattered[1] * (tt[4][2] + tt[3][2]) / shadow_direct[1];
        tt[2][0] *= direct_unscattered[1] * (tt[4][2] + tt[3][2]) / shadow_direct[1];
    }
    if shadow_direct[0] > 0.0 {
        tt[1][0] = direct_unscattered[0] * (tt[4][1] + tt[3][1] + tt[2][1]);
    }
    tt[3][2] += tt[4][2];
    tt[2][1] += tt[4][1] + tt[3][1];
    tt[1][0] += tt[4][0] + tt[3][0] + tt[2][0];
    let direct_column_transmission = tt[1][0];
    let direct_path = [tt[4][3], tt[3][2], tt[2][1], tt[1][0]];
    tt = [[0.0; 5]; 5];
    tt[4][3] = direct_path[0];
    tt[3][2] = direct_path[1];
    tt[2][1] = direct_path[2];
    tt[1][0] = direct_path[3];

    let mut albedo = [[0.0; RTYPES]; BANDS];
    let mut transmission = [[0.0; 3]; BANDS];
    let mut sunlit = vec![[[0.0; RTYPES]; BANDS]; count];
    let mut shaded = vec![[[0.0; RTYPES]; BANDS]; count];
    let mut thermal_gap = vec![1.0; count];
    let mut shade = vec![0.0; count];
    let mut psun_layer = [0.0; LAYERS];
    let mut fsun_id_layer = [0.0; LAYERS];
    let mut fsun_ii_layer = [0.0; LAYERS];
    for band in 0..BANDS {
        let mut pft_direct_depth = vec![0.0; count];
        let mut pft_diffuse_depth = vec![0.0; count];
        let mut pft_shadow_direct = vec![0.0; count];
        let mut pft_shadow_diffuse = vec![0.0; count];
        let mut pft_direct_original = vec![0.0; count];
        let mut pft_diffuse_original = vec![0.0; count];
        let mut pft_direct = vec![0.0; count];
        let mut pft_diffuse = vec![1.0; count];
        let mut pft_direct_calibration = vec![1.0; count];
        let mut pft_diffuse_calibration = vec![1.0; count];
        for index in 0..count {
            if !active[index] {
                continue;
            }
            let layer = canopy[index];
            let portion = (fractions[index] / cover[0][layer]).min(1.0);
            pft_shadow_direct[index] = portion * shadow_direct[layer];
            pft_shadow_diffuse[index] = portion * shadow_diffuse[layer];
            pft_direct_depth[index] = 0.75 * GEE * fractions[index] * leaf_stem_area[index]
                / (cosz[index] * pft_shadow_direct[index]);
            pft_diffuse_depth[index] = 0.75 * GEE * fractions[index] * leaf_stem_area[index]
                / (cosd[index] * pft_shadow_diffuse[index]);
            pft_direct_original[index] = canopy_transmittance(pft_direct_depth[index]);
            pft_diffuse_original[index] = canopy_transmittance(pft_diffuse_depth[index]);
            pft_direct[index] = canopy_transmittance(pft_direct_depth[index] / GEE * gdir[index]);
            pft_diffuse[index] = canopy_transmittance(pft_diffuse_depth[index] / GEE * gdif[index]);
            pft_direct_calibration[index] =
                (1.0 - pft_direct[index]) / (1.0 - pft_direct_original[index]);
            pft_diffuse_calibration[index] =
                (1.0 - pft_diffuse[index]) / (1.0 - pft_diffuse_original[index]);
        }

        let mut layer_direct_scattered = [0.0; LAYERS];
        let mut layer_diffuse_transmission = [1.0; LAYERS];
        let mut layer_direct_reflection = [0.0; LAYERS];
        let mut layer_diffuse_reflection = [0.0; LAYERS];
        let mut layer_direct_absorption = [0.0; LAYERS];
        let mut layer_diffuse_absorption = [0.0; LAYERS];
        for layer in 0..LAYERS {
            if shadow_direct[layer] <= 0.0 {
                continue;
            }
            let mut canopy_radiation = canopy_radiation(
                direct_depth[layer],
                diffuse_depth[layer],
                direct_unscattered_original[layer],
                diffuse_unscattered_original[layer],
                layer_cosz[layer],
                layer_cosd[layer],
                shadow_direct[layer],
                shadow_diffuse[layer],
                cover[0][layer],
                layer_omega[layer][band],
                layer_lsai[layer],
                layer_tau[layer][band],
                layer_rho[layer][band],
            );
            canopy_radiation.direct_scattered *= direct_calibration[layer];
            canopy_radiation.diffuse_transmission = diffuse_calibration[layer]
                * (canopy_radiation.diffuse_transmission - diffuse_unscattered_original[layer])
                + diffuse_unscattered[layer];
            canopy_radiation.direct_reflection *= direct_calibration[layer];
            canopy_radiation.diffuse_reflection *= diffuse_calibration[layer];
            canopy_radiation.direct_absorption *= direct_calibration[layer];
            canopy_radiation.diffuse_absorption *= diffuse_calibration[layer];
            layer_direct_scattered[layer] = canopy_radiation.direct_scattered;
            layer_diffuse_transmission[layer] = canopy_radiation.diffuse_transmission;
            layer_direct_reflection[layer] = canopy_radiation.direct_reflection;
            layer_diffuse_reflection[layer] = canopy_radiation.diffuse_reflection;
            layer_direct_absorption[layer] = canopy_radiation.direct_absorption;
            layer_diffuse_absorption[layer] = canopy_radiation.diffuse_absorption;
        }
        let mut layer_direct_sunlit = [0.0; LAYERS];
        for layer in 0..LAYERS {
            if cover[0][layer] > 0.0 && layer_lsai[layer] > 0.0 {
                layer_direct_sunlit[layer] = tt[layer + 2][layer + 1]
                    * (1.0 - direct_unscattered[layer])
                    * (1.0 - layer_omega[layer][band]);
            }
        }
        let matrix = radiation_matrix(
            shadow_diffuse,
            layer_diffuse_transmission,
            layer_direct_reflection,
            layer_diffuse_reflection,
            ground[band][0],
            ground[band][1],
            tt,
            layer_direct_scattered,
        );
        let solution = solve_six(matrix.0, matrix.1)?;
        let mut layer_direct_total = [0.0; LAYERS];
        let mut layer_diffuse_total = [0.0; LAYERS];
        layer_direct_total[2] = tt[4][3] * layer_direct_absorption[2]
            + solution[2][0] * shadow_diffuse[2] * layer_diffuse_absorption[2];
        layer_direct_total[1] = tt[3][2] * layer_direct_absorption[1]
            + (solution[1][0] + solution[4][0]) * shadow_diffuse[1] * layer_diffuse_absorption[1];
        layer_direct_total[0] = tt[2][1] * layer_direct_absorption[0]
            + (solution[3][0] + solution[5][0] * ground[band][1] + tt[1][0] * ground[band][0])
                * shadow_diffuse[0]
                * layer_diffuse_absorption[0];
        layer_diffuse_total[2] =
            (1.0 + solution[2][1]) * shadow_diffuse[2] * layer_diffuse_absorption[2];
        layer_diffuse_total[1] =
            (solution[1][1] + solution[4][1]) * shadow_diffuse[1] * layer_diffuse_absorption[1];
        layer_diffuse_total[0] = (solution[3][1] + solution[5][1] * ground[band][1])
            * shadow_diffuse[0]
            * layer_diffuse_absorption[0];
        let direct_absorption: f64 = layer_direct_total.iter().sum();
        let diffuse_absorption: f64 = layer_diffuse_total.iter().sum();
        albedo[band] = [solution[0][0], solution[0][1]];
        transmission[band] = [
            (1.0 - albedo[band][0]
                - direct_absorption
                - direct_column_transmission * (1.0 - ground[band][0]))
                / (1.0 - ground[band][1]),
            (1.0 - albedo[band][1] - diffuse_absorption) / (1.0 - ground[band][1]),
            direct_column_transmission,
        ];
        if band == 0 {
            if cover[0][2] > 0.0 && layer_lsai[2] > 0.0 {
                psun_layer[2] = tt[4][3] / shadow_direct[2];
                fsun_id_layer[2] = (psun_layer[2] * sunlit_direct_weight[2]
                    + solution[2][0] * sunlit_upward_weight[2])
                    / (psun_layer[2] + solution[2][0]);
                fsun_ii_layer[2] = (sunlit_downward_weight[2]
                    + solution[2][1] * sunlit_upward_weight[2])
                    / (1.0 + solution[2][1]);
            }
            if cover[0][1] > 0.0 && layer_lsai[1] > 0.0 {
                psun_layer[1] = tt[3][2] / shadow_direct[1];
                fsun_id_layer[1] = (psun_layer[1] * sunlit_direct_weight[1]
                    + solution[1][0] * sunlit_downward_weight[1]
                    + solution[4][0] * sunlit_upward_weight[1])
                    / (psun_layer[1] + solution[1][0] + solution[4][0]);
                fsun_ii_layer[1] = (solution[1][1] * sunlit_downward_weight[1]
                    + solution[4][1] * sunlit_upward_weight[1])
                    / (solution[1][1] + solution[4][1]);
            }
            if cover[0][0] > 0.0 && layer_lsai[0] > 0.0 {
                psun_layer[0] = tt[2][1] / shadow_direct[0];
                fsun_id_layer[0] = (psun_layer[0] * sunlit_direct_weight[0]
                    + solution[3][0] * sunlit_downward_weight[0]
                    + (solution[5][0] * ground[band][1] + tt[1][0] * ground[band][0])
                        * sunlit_upward_weight[0])
                    / (psun_layer[0]
                        + solution[3][0]
                        + solution[5][0] * ground[band][1]
                        + tt[1][0] * ground[band][0]);
                fsun_ii_layer[0] = (solution[3][1] * sunlit_downward_weight[0]
                    + solution[5][1] * ground[band][1] * sunlit_upward_weight[0])
                    / (solution[3][1] + solution[5][1] * ground[band][1]);
            }
        }

        let mut sum_direct = [0.0; LAYERS];
        let mut sum_diffuse = [0.0; LAYERS];
        let mut sum_sunlit = [0.0; LAYERS];
        let mut pft_direct_absorption = vec![0.0; count];
        let mut pft_diffuse_absorption = vec![0.0; count];
        let mut pft_sunlit_absorption = vec![0.0; count];
        for index in 0..count {
            if !active[index] {
                continue;
            }
            let layer = canopy[index];
            let mut values = canopy_radiation(
                pft_direct_depth[index],
                pft_diffuse_depth[index],
                pft_direct_original[index],
                pft_diffuse_original[index],
                cosz[index],
                cosd[index],
                pft_shadow_direct[index],
                pft_shadow_diffuse[index],
                fractions[index],
                rho[index][band] + tau[index][band],
                leaf_stem_area[index],
                tau[index][band],
                rho[index][band],
            );
            values.direct_scattered *= pft_direct_calibration[index];
            values.diffuse_transmission = pft_diffuse_calibration[index]
                * (values.diffuse_transmission - pft_diffuse_original[index])
                + pft_diffuse[index];
            values.direct_reflection *= pft_direct_calibration[index];
            values.diffuse_reflection *= pft_diffuse_calibration[index];
            values.direct_absorption *= pft_direct_calibration[index];
            values.diffuse_absorption *= pft_diffuse_calibration[index];
            let sky = pft_shadow_diffuse[index];
            let probability = values.diffuse_reflection * sky * ground[band][1];
            let direct_ground = (1.0 - pft_shadow_direct[index]
                + pft_shadow_direct[index] * pft_direct[index])
                * ground[band][0]
                + pft_shadow_direct[index] * values.direct_scattered * ground[band][1];
            pft_direct_absorption[index] = pft_shadow_direct[index] * values.direct_absorption
                + direct_ground * values.diffuse_absorption * sky / (1.0 - probability);
            let diffuse_ground =
                1.0 - pft_shadow_diffuse[index] * (1.0 - values.diffuse_transmission);
            pft_diffuse_absorption[index] = pft_shadow_diffuse[index] * values.diffuse_absorption
                + diffuse_ground * ground[band][1] * values.diffuse_absorption * sky
                    / (1.0 - probability);
            pft_sunlit_absorption[index] = pft_shadow_direct[index]
                * (1.0 - pft_direct[index])
                * (1.0 - rho[index][band] - tau[index][band]);
            sum_direct[layer] += pft_direct_absorption[index];
            sum_diffuse[layer] += pft_diffuse_absorption[index];
            sum_sunlit[layer] += pft_sunlit_absorption[index];
        }
        for index in 0..count {
            if !active[index] {
                continue;
            }
            let layer = canopy[index];
            let direct = pft_direct_absorption[index] * layer_direct_total[layer]
                / sum_direct[layer]
                / fractions[index];
            let diffuse = pft_diffuse_absorption[index] * layer_diffuse_total[layer]
                / sum_diffuse[layer]
                / fractions[index];
            let direct_sunlit = (pft_sunlit_absorption[index] * layer_direct_sunlit[layer]
                / sum_sunlit[layer]
                / fractions[index])
                .min(direct);
            // `ThreeDCanopy_wrap` keeps the source's `fsun3D = .false.`
            // default: 3D geometry supplies `psun`, while leaf-level diffuse
            // partitioning uses its 1D expression below.
            let psun = psun_layer[layer];
            let extinction = direct_extinction[index];
            let lsai = leaf_stem_area[index];
            let fsun_id =
                (1.0 - (-2.0 * extinction * lsai).exp()) / (1.0 - (-extinction * lsai).exp()) / 2.0
                    * psun;
            let fsun_ii = (1.0 - (-extinction * lsai - lsai).exp())
                / (1.0 - (-lsai).exp())
                / (1.0 + extinction)
                * psun;
            sunlit[index][band][0] = (direct - direct_sunlit) * fsun_id + direct_sunlit;
            shaded[index][band][0] = (direct - direct_sunlit) * (1.0 - fsun_id);
            sunlit[index][band][1] = diffuse * fsun_ii;
            shaded[index][band][1] = diffuse * (1.0 - fsun_ii);
            thermal_gap[index] = pft_diffuse[index];
            shade[index] = pft_shadow_diffuse[index];
        }
    }
    Ok(CoreRadiation {
        albedo,
        transmission,
        sunlit,
        shaded,
        thermal_gap,
        shade,
        direct_extinction,
    })
}

struct CanopyRadiation {
    direct_scattered: f64,
    diffuse_transmission: f64,
    direct_reflection: f64,
    diffuse_reflection: f64,
    direct_absorption: f64,
    diffuse_absorption: f64,
}

#[allow(clippy::too_many_arguments)]
fn canopy_radiation(
    direct_depth: f64,
    diffuse_depth: f64,
    direct_unscattered: f64,
    diffuse_unscattered: f64,
    cosine_direct: f64,
    cosine_diffuse: f64,
    shadow_direct: f64,
    shadow_diffuse: f64,
    cover: f64,
    omega: f64,
    leaf_stem_area: f64,
    transmittance: f64,
    reflectance: f64,
) -> CanopyRadiation {
    let (total_direct, difference_direct, _) =
        canopy_scattering(direct_depth, omega, transmittance, reflectance);
    let (total_diffuse, difference_diffuse, _) =
        canopy_scattering(diffuse_depth, omega, transmittance, reflectance);
    let sphere_depth = 0.75 * 0.5 * leaf_stem_area;
    let (total_sphere, difference_sphere, absorption_probability) =
        canopy_scattering(sphere_depth, omega, transmittance, reflectance);
    let forward_sphere = (0.5 * (total_sphere - 0.5 * difference_sphere)).clamp(0.0, 1.0);
    let forward_shape = 3.0
        * (1.0 - (1.0 - 3.0_f64.sqrt() * cover / (2.0 * std::f64::consts::PI)).sqrt())
        + 3.0 * (1.0 - (1.0 - 3.0_f64.sqrt() * cover / (6.0 * std::f64::consts::PI)).sqrt());
    let wb = (2.0 * reflectance + transmittance) / 3.0;
    let alpha = (1.0 - omega).sqrt() * (1.0 - omega + 2.0 * wb).sqrt();
    let direct_factor = (1.0 + 2.0 * alpha) / (1.0 + 2.0 * alpha * cosine_direct);
    let diffuse_factor = (1.0 + 2.0 * alpha) / (1.0 + 2.0 * alpha * cosine_diffuse);
    let correction =
        total_sphere * forward_shape * (1.0 - canopy_transmittance(sphere_depth)) * (1.0 - omega)
            / (1.0 - omega * absorption_probability);
    let direct_lateral = (direct_factor - 1.0)
        * forward_sphere
        * cover
        * (1.0 / shadow_direct - cosine_direct / cover);
    let diffuse_lateral = (diffuse_factor - 1.0)
        * forward_sphere
        * cover
        * (1.0 / shadow_diffuse - cosine_diffuse / cover);
    let mut direct_reflection = 0.5 * (total_direct - 0.5 * cosine_direct * difference_direct)
        + direct_lateral
        - 0.5 * correction;
    let mut diffuse_reflection = 0.5 * (total_diffuse - 0.5 * cosine_diffuse * difference_diffuse)
        + diffuse_lateral
        - 0.5 * correction;
    let mut direct_scattered = 0.5 * (total_direct + 0.5 * cosine_direct * difference_direct)
        - 0.5 * direct_lateral
        - 0.5 * correction;
    let mut diffuse_transmission =
        0.5 * (total_diffuse + 0.5 * cosine_diffuse * difference_diffuse) + diffuse_unscattered
            - 0.5 * diffuse_lateral
            - 0.5 * correction;
    direct_reflection = direct_reflection.clamp(0.0, 1.0);
    diffuse_reflection = diffuse_reflection.clamp(0.0, 1.0);
    direct_scattered = direct_scattered.clamp(0.0, 1.0);
    diffuse_transmission = diffuse_transmission.clamp(0.0, 1.0);
    let mut direct_absorption =
        (1.0 - direct_unscattered - direct_reflection - direct_scattered).clamp(0.0, 1.0);
    let mut diffuse_absorption = (1.0 - diffuse_reflection - diffuse_transmission).clamp(0.0, 1.0);
    if shadow_direct == 0.0 {
        direct_scattered = 0.0;
        direct_reflection = 0.0;
        direct_absorption = 0.0;
    }
    if shadow_diffuse == 0.0 {
        diffuse_transmission = 1.0;
        diffuse_reflection = 0.0;
        diffuse_absorption = 0.0;
    }
    CanopyRadiation {
        direct_scattered,
        diffuse_transmission,
        direct_reflection,
        diffuse_reflection,
        direct_absorption,
        diffuse_absorption,
    }
}

/// Shared spherical-canopy multiple-scattering approximation from
/// `MOD_3DCanopyRadiation::phi` with its calibrated (`runmode=.true.`) branch.
pub(crate) fn canopy_scattering(
    depth: f64,
    omega: f64,
    transmittance: f64,
    reflectance: f64,
) -> (f64, f64, f64) {
    let forward_first =
        1.0 / depth.powi(2) - (1.0 / depth.powi(2) + 2.0 / depth + 2.0) * (-2.0 * depth).exp();
    let backward_first = 0.5 * (1.0 - canopy_transmittance(2.0 * depth));
    let aa = 0.70;
    let bb = 1.74;
    let backward_second = aa
        * (1.0 / (bb + 1.0) - canopy_transmittance(2.0 * depth) / (bb - 1.0)
            + 2.0 * canopy_transmittance((bb + 1.0) * depth) / ((bb + 1.0) * (bb - 1.0)));
    let forward_second = aa
        * (2.0 * bb * forward_first / (bb * bb - 1.0)
            - (1.0 / (bb + 1.0).powi(2) + 1.0 / (bb - 1.0).powi(2)) * canopy_transmittance(depth)
            + canopy_transmittance(depth * bb) / (bb - 1.0).powi(2)
            + canopy_transmittance((bb + 2.0) * depth) / (bb + 1.0).powi(2));
    let average_second = 0.5 * (backward_second + forward_second);
    let absorption_probability = (1.0
        - average_second
            / (1.0
                - canopy_transmittance(depth)
                - (reflectance * backward_first + transmittance * forward_first)
                    / (transmittance + reflectance)))
        .clamp(0.0, 1.0);
    let forward_multiple = forward_second
        + omega * absorption_probability * average_second / (1.0 - omega * absorption_probability);
    let backward_multiple = backward_second
        + omega * absorption_probability * average_second / (1.0 - omega * absorption_probability);
    let forward = transmittance * forward_first + 0.5 * omega * omega * forward_multiple;
    let backward = reflectance * backward_first + 0.5 * omega * omega * backward_multiple;
    (
        forward + backward,
        forward - backward,
        absorption_probability,
    )
}

/// Mean direct transmission through a spherical canopy (`tee` in CoLM).
pub(crate) fn canopy_transmittance(depth: f64) -> f64 {
    0.5 * (1.0 / depth.powi(2) - (1.0 / depth.powi(2) + 2.0 / depth) * (-2.0 * depth).exp())
}

fn overlap_area(radius: f64, height: f64, zenith: f64) -> f64 {
    if radius == 0.0 {
        return 0.0;
    }
    let cosine = height * zenith.tan() / radius / (1.0 + 1.0 / zenith.cos());
    if cosine >= 1.0 {
        return 0.0;
    }
    let angle = cosine.acos();
    (angle - cosine * angle.sin()) * (1.0 + 1.0 / zenith.cos()) / std::f64::consts::PI
}

#[allow(clippy::too_many_arguments)]
fn radiation_matrix(
    shadow: [f64; LAYERS],
    transmission: [f64; LAYERS],
    direct_reflection: [f64; LAYERS],
    diffuse_reflection: [f64; LAYERS],
    ground_direct: f64,
    ground_diffuse: f64,
    tt: [[f64; 5]; 5],
    direct_scattered: [f64; LAYERS],
) -> ([[f64; 6]; 6], [[f64; 2]; 6]) {
    let mut a = [[0.0; 6]; 6];
    let mut b = [[0.0; 2]; 6];
    a[0][0] = 1.0;
    a[0][2] = -shadow[2] * transmission[2] + shadow[2] - 1.0;
    a[1][1] = 1.0;
    a[1][2] = -shadow[2] * diffuse_reflection[2];
    a[2][2] = 1.0;
    a[2][1] = -shadow[1] * diffuse_reflection[1];
    a[2][4] = -shadow[1] * transmission[1] + shadow[1] - 1.0;
    a[3][3] = 1.0;
    a[3][4] = -shadow[1] * diffuse_reflection[1];
    a[3][1] = -shadow[1] * transmission[1] + shadow[1] - 1.0;
    a[4][4] = 1.0;
    a[4][3] = -shadow[0] * diffuse_reflection[0];
    a[4][5] = (-shadow[0] * transmission[0] + shadow[0] - 1.0) * ground_diffuse;
    a[5][5] = 1.0 - ground_diffuse * shadow[0] * diffuse_reflection[0];
    a[5][3] = -shadow[0] * transmission[0] + shadow[0] - 1.0;
    b[0] = [
        tt[4][3] * diffuse_reflection[2],
        shadow[2] * diffuse_reflection[2],
    ];
    b[1] = [
        tt[4][3] * direct_scattered[2],
        shadow[2] * transmission[2] - shadow[2] + 1.0,
    ];
    b[2] = [tt[3][2] * direct_reflection[1], 0.0];
    b[3] = [tt[3][2] * direct_scattered[1], 0.0];
    b[4] = [
        tt[2][1] * direct_reflection[0]
            + tt[1][0] * ground_direct * (shadow[0] * transmission[0] - shadow[0] + 1.0),
        0.0,
    ];
    b[5] = [
        tt[2][1] * direct_scattered[0]
            + tt[1][0] * ground_direct * shadow[0] * diffuse_reflection[0],
        0.0,
    ];
    (a, b)
}

fn solve_six(mut a: [[f64; 6]; 6], mut b: [[f64; 2]; 6]) -> Result<[[f64; 2]; 6]> {
    for (row, span) in [0usize, 2, 1, 2, 1].into_iter().enumerate() {
        for target in row + 1..=row + span {
            ensure!(
                a[row][row].abs() >= 1.0e-10,
                "singular PC canopy radiation matrix"
            );
            let factor = -a[target][row] / a[row][row];
            let pivot_a = a[row];
            for (target_value, pivot_value) in a[target].iter_mut().zip(pivot_a) {
                *target_value += factor * pivot_value;
            }
            let pivot_b = b[row];
            for (target_value, pivot_value) in b[target].iter_mut().zip(pivot_b) {
                *target_value += factor * pivot_value;
            }
        }
    }
    ensure!(
        a[5][5].abs() >= 1.0e-10,
        "singular PC canopy radiation matrix"
    );
    let mut x = [[0.0; 2]; 6];
    for beam in 0..2 {
        x[5][beam] = b[5][beam] / a[5][5];
    }
    for row in (0..5).rev() {
        for beam in 0..2 {
            let upper: f64 = (row + 1..6)
                .map(|column| a[row][column] * x[column][beam])
                .sum();
            x[row][beam] = (b[row][beam] - upper) / a[row][row];
        }
    }
    Ok(x)
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn ground_albedos(
    soil: SoilReflectance,
    liquid_water: f64,
    thickness: f64,
    cosine_zenith: f64,
    snow_depth: f64,
    snow_fraction: f64,
    temperature: f64,
) -> Result<(
    [[f64; RTYPES]; BANDS],
    [[f64; RTYPES]; BANDS],
    [[f64; RTYPES]; BANDS],
    f64,
)> {
    let wetness = (1.0e-3 * liquid_water / thickness).min(1.0);
    let increase = (0.11 - 0.40 * wetness).max(0.0);
    let soil_ground = [
        [(soil.saturated_visible + increase).min(soil.dry_visible); RTYPES],
        [(soil.saturated_near_infrared + increase).min(soil.dry_near_infrared); RTYPES],
    ];
    let (snow, snow_age) = generic_snow_albedo(snow_depth * 250.0, temperature, cosine_zenith)?;
    let ground = mix_ground_albedo(soil_ground, snow, snow_fraction);
    Ok((soil_ground, snow, ground, snow_age))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_pc_sentinel_keeps_scalar_outputs_without_an_active_layer() {
        let mut bare = PcPftInput {
            canopy_layer: 0,
            fraction: 1.0,
            canopy_top_m: 0.0,
            canopy_bottom_m: 0.0,
            optics: LeafOptics {
                chil: -0.3,
                reflectance: [[0.11, 0.31], [0.35, 0.53]],
                transmittance: [[0.05, 0.12], [0.34, 0.25]],
            },
            lai: 0.0,
            sai: 0.0,
            wet_snow_fraction: 0.0,
        };
        let ground = [[0.14; 2], [0.28; 2]];
        let evaluate = |pft| {
            cold_start_pc_broadband_radiation_from_ground(
                &[pft],
                0.5,
                ColdStartGroundAlbedo {
                    soil: ground,
                    snow: ground,
                    ground,
                    snow_age: 0.0,
                },
            )
        };
        let state = evaluate(bare).unwrap();
        assert_eq!(state.common.albedo, ground);
        assert_eq!(state.pft[0].sunlit_absorption, [[0.0; 2]; 2]);
        assert_eq!(state.pft[0].shaded_absorption, [[0.0; 2]; 2]);
        assert_eq!(state.pft[0].thermal_gap_fraction, 1.0);
        assert_eq!(state.pft[0].shade_fraction, 0.0);
        assert_eq!(state.pft[0].diffuse_extinction, 0.719);
        // MOD_3DCanopyRadiation computes gdir/czen even for class 0.
        let phi1 = 0.5 - 0.633 * bare.optics.chil - 0.33 * bare.optics.chil * bare.optics.chil;
        let phi2 = 0.877 * (1.0 - 2.0 * phi1);
        assert_eq!(state.pft[0].direct_extinction, (phi1 + phi2 * 0.5) / 0.5);
        bare.lai = 1.0;
        assert!(evaluate(bare).is_err());
    }

    #[test]
    fn pc_canopy_closes_shortwave_energy_for_each_band_and_beam() {
        let state = cold_start_pc_broadband_radiation_with_snow(
            0,
            SoilReflectance {
                saturated_visible: 0.12,
                dry_visible: 0.22,
                saturated_near_infrared: 0.26,
                dry_near_infrared: 0.36,
            },
            0.0,
            0.02,
            &[PcPftInput {
                canopy_layer: 1,
                fraction: 1.0,
                canopy_top_m: 0.5,
                canopy_bottom_m: 0.05,
                optics: LeafOptics {
                    chil: -0.3,
                    reflectance: [[0.105, 0.36], [0.58, 0.58]],
                    transmittance: [[0.07, 0.22], [0.25, 0.38]],
                },
                lai: 1.2,
                sai: 0.3,
                wet_snow_fraction: 0.0,
            }],
            0.5,
            0.0,
            0.0,
            273.16,
        )
        .expect("valid PC canopy");
        assert!(state.common.transmission.is_some());
        for band in 0..BANDS {
            for beam in 0..RTYPES {
                let total = state.common.albedo[band][beam]
                    + state.common.sunlit_absorption[band][beam]
                    + state.common.shaded_absorption[band][beam]
                    + state.common.soil_absorption[band][beam];
                assert!(
                    (total - 1.0).abs() < 1.0e-10,
                    "band={band}, beam={beam}, {total}"
                );
            }
        }
    }

    #[test]
    fn explicit_ground_entry_matches_the_standard_ground_resolution() {
        let pfts = [PcPftInput {
            canopy_layer: 1,
            fraction: 1.0,
            canopy_top_m: 0.5,
            canopy_bottom_m: 0.05,
            optics: LeafOptics {
                chil: -0.3,
                reflectance: [[0.105, 0.36], [0.58, 0.58]],
                transmittance: [[0.07, 0.22], [0.25, 0.38]],
            },
            lai: 1.2,
            sai: 0.3,
            wet_snow_fraction: 0.0,
        }];
        let soil = SoilReflectance {
            saturated_visible: 0.12,
            dry_visible: 0.22,
            saturated_near_infrared: 0.26,
            dry_near_infrared: 0.36,
        };
        let standard = cold_start_pc_broadband_radiation_with_snow(
            0, soil, 0.0, 0.02, &pfts, 0.5, 0.0, 0.0, 273.16,
        )
        .unwrap();
        let (soil_ground, snow, ground, snow_age) =
            ground_albedos(soil, 0.0, 0.02, 0.5, 0.0, 0.0, 273.16).unwrap();
        assert_eq!(
            cold_start_pc_broadband_radiation_from_ground(
                &pfts,
                0.5,
                ColdStartGroundAlbedo {
                    soil: soil_ground,
                    snow,
                    ground,
                    snow_age
                },
            )
            .unwrap(),
            standard
        );
    }

    #[test]
    fn pc_canopy_matches_the_upstream_cn_pc_cold_restart() {
        let optics = LeafOptics {
            chil: -0.3,
            reflectance: [[0.11, 0.31], [0.35, 0.53]],
            transmittance: [[0.05, 0.12], [0.34, 0.25]],
        };
        let state = cold_start_pc_broadband_radiation_with_snow(
            0,
            SoilReflectance {
                saturated_visible: 0.14,
                dry_visible: 0.25,
                saturated_near_infrared: 0.28,
                dry_near_infrared: 0.39,
            },
            0.0,
            0.017_512_817_916_255_2,
            &[
                PcPftInput {
                    canopy_layer: 1,
                    fraction: 0.540_000_005_364_418,
                    canopy_top_m: 0.5,
                    canopy_bottom_m: 0.0,
                    optics,
                    lai: 0.200_000_002_980_232,
                    sai: 0.449_999_988_079_071,
                    wet_snow_fraction: 0.0,
                },
                PcPftInput {
                    canopy_layer: 1,
                    fraction: 0.459_999_994_635_582,
                    canopy_top_m: 0.5,
                    canopy_bottom_m: 0.0,
                    optics,
                    lai: 0.200_000_002_980_232,
                    sai: 0.449_999_988_079_071,
                    wet_snow_fraction: 0.0,
                },
            ],
            0.001,
            0.0,
            0.0,
            257.804_183_316_718,
        )
        .expect("valid PC reference inputs");
        assert_matrix_close(
            state.common.albedo,
            [
                [0.157_156_279_854_289, 0.166_059_611_949_924],
                [0.364_303_491_021_769, 0.377_695_612_280_933],
            ],
        );
        assert_matrix_close(
            state.pft[0].sunlit_absorption,
            [
                [0.736_832_292_383_366, 0.001_245_853_045_327_45],
                [0.391_388_968_314_026, 0.000_579_908_579_960_18],
            ],
        );
        assert_matrix_close(
            state.pft[0].shaded_absorption,
            [
                [0.083_758_521_958_294, 0.392_305_455_198_16],
                [0.143_697_856_436_49, 0.182_606_849_409_597],
            ],
        );
        assert_close(state.pft[0].thermal_gap_fraction, 0.524_190_845_267_958);
        assert_close(state.pft[0].shade_fraction, 0.540_000_005_364_418);
        assert_close(state.pft[0].direct_extinction, 659.919_009_2);
    }

    fn assert_matrix_close(actual: [[f64; RTYPES]; BANDS], expected: [[f64; RTYPES]; BANDS]) {
        for band in 0..BANDS {
            for beam in 0..RTYPES {
                assert_close(actual[band][beam], expected[band][beam]);
            }
        }
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1.0e-12,
            "actual={actual:.16e}, expected={expected:.16e}"
        );
    }
}
