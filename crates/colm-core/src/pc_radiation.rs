//! Cold-start port of `MOD_3DCanopyRadiation.F90` for plant communities.

use anyhow::{ensure, Result};
use colm_numeric::Contract;

use crate::extended::DoubleDouble;
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
    let core = three_d_canopy_wrap(pfts, &fractions, cosine_zenith, ground, true);
    let pft = core.pft;
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

/// `ThreeDCanopy_wrap`（`MOD_3DCanopyRadiation.F90:42-283`）的结果。
///
/// 列量（`albv`/`tran`）取自第一个 PFT —— `ThreeDCanopy` 末尾给每个 PFT 写的是同一组列值。
pub struct ThreeDCanopyOutput {
    pub albedo: [[f64; RTYPES]; BANDS],
    pub transmission: [[f64; 3]; BANDS],
    pub pft: Vec<PcPftRadiation>,
}

/// `ThreeDCanopy_wrap`：由 PFT 参数拼出逐 PFT 的冠层尺寸与光学，调 `ThreeDCanopy`，
/// 再按 `fsun3D = .false.` 那一支把吸收拆成阳叶/阴叶。
///
/// `fractions` 是 `fcover = pftfrac/sum(pftfrac)`（只含走三维模型的自然 PFT）。
/// 每条语句的舍入形状取自 `main/` 的 GIMPLE（行号见注释）。
pub fn three_d_canopy_wrap(
    pfts: &[PcPftInput],
    fractions: &[f64],
    cosine_zenith: f64,
    ground: [[f64; RTYPES]; BANDS],
    vegetation_snow: bool,
) -> ThreeDCanopyOutput {
    const RHO_SNOW: [f64; BANDS] = [0.5, 0.2];
    const TAU_SNOW: [f64; BANDS] = [0.3, 0.2];
    let count = pfts.len();
    let mut canopy = vec![0usize; count];
    let mut size = vec![0.0; count];
    let mut height = vec![0.0; count];
    let mut chil = vec![0.0; count];
    let mut lsai = vec![0.0; count];
    let mut rho = vec![[0.0; BANDS]; count];
    let mut tau = vec![[0.0; BANDS]; count];
    for (index, pft) in pfts.iter().enumerate() {
        // `:156-157`：`(htop-hbot)/2` 与 `(htop+hbot)/2` 都是乘 0.5。
        size[index] = (pft.canopy_top_m - pft.canopy_bottom_m) * 0.5;
        height[index] = (pft.canopy_top_m + pft.canopy_bottom_m) * 0.5;
        lsai[index] = pft.lai + pft.sai;
        canopy[index] = pft.canopy_layer;
        chil[index] = pft.optics.chil;
        for band in 0..BANDS {
            if lsai[index] > 0.0 {
                // `:173/175`：两项各自 `x*lai/lsai`，再相加。
                rho[index][band] = pft.optics.reflectance[band][0] * pft.lai / lsai[index]
                    + pft.optics.reflectance[band][1] * pft.sai / lsai[index];
                tau[index][band] = pft.optics.transmittance[band][0] * pft.lai / lsai[index]
                    + pft.optics.transmittance[band][1] * pft.sai / lsai[index];
            }
            if vegetation_snow {
                // `:181-182`：`FMA(rho, 1-fwet, fwet*rho_sno)`。
                let dry = 1.0 - pft.wet_snow_fraction;
                rho[index][band] =
                    rho[index][band].contract(dry, pft.wet_snow_fraction * RHO_SNOW[band]);
                tau[index][band] =
                    tau[index][band].contract(dry, pft.wet_snow_fraction * TAU_SNOW[band]);
            }
        }
    }
    let core = three_d_canopy(
        &canopy,
        fractions,
        &size,
        &height,
        &chil,
        cosine_zenith,
        &lsai,
        &rho,
        &tau,
        ground,
    );
    let pft = (0..count)
        .map(|index| {
            // `:200-206`：wrap 自己的 `phi1/phi2`，`gdir = FMA(phi2, czen, phi1)`，未经 `cosz` 修正。
            let (phi1, phi2) = leaf_projection(chil[index]);
            let direct_extinction = phi2.contract(cosine_zenith, phi1) / cosine_zenith;
            let area = lsai[index];
            let psun = core.psun[index];
            let (fsun_id, fsun_ii) = if area > 0.0 {
                // `:217/221`
                let two = (-(area * (direct_extinction * 2.0))).exp();
                let one = -(area * direct_extinction);
                (
                    (1.0 - two) / (1.0 - one.exp()) * 0.5 * psun,
                    psun * ((1.0 - (one - area).exp())
                        / (1.0 - (-area).exp())
                        / (direct_extinction + 1.0)),
                )
            } else {
                (0.0, 0.0)
            };
            let fabd = core.fabd[index];
            let fabi = core.fabi[index];
            let fadd = core.fadd[index];
            // `:231-238`：`ssun(:,1) = FMA(fabd-fadd, fsun_id, fadd)`，其余三项是单次乘法。
            PcPftRadiation {
                sunlit_absorption: std::array::from_fn(|band| {
                    [
                        (fabd[band] - fadd[band]).contract(fsun_id, fadd[band]),
                        fabi[band] * fsun_ii,
                    ]
                }),
                shaded_absorption: std::array::from_fn(|band| {
                    [
                        (fabd[band] - fadd[band]) * (1.0 - fsun_id),
                        fabi[band] * (1.0 - fsun_ii),
                    ]
                }),
                thermal_gap_fraction: core.thermal_gap[index],
                shade_fraction: core.shade[index],
                direct_extinction,
                diffuse_extinction: 0.719,
            }
        })
        .collect();
    ThreeDCanopyOutput {
        albedo: std::array::from_fn(|band| [core.albd[band], core.albi[band]]),
        transmission: std::array::from_fn(|band| [core.ftid[band], core.ftii[band], core.ftdd]),
        pft,
    }
}

/// `phi1 = 0.5 - 0.633*chil - 0.33*chil*chil`、`phi2 = 0.877*(1-2*phi1)`：
/// GIMPLE（`:200-201`、`:481-482`）是 `FNMA(chil, chil*0.33, FNMA(chil, 0.633, 0.5))` 与
/// `FNMA(phi1, 2, 1)*0.877`。
fn leaf_projection(chil: f64) -> (f64, f64) {
    let phi1 = (-chil).contract(chil * 0.33, (-chil).contract(0.633, 0.5));
    (phi1, (-phi1).contract(2.0, 1.0) * 0.877)
}

struct ThreeDCore {
    albd: [f64; BANDS],
    albi: [f64; BANDS],
    ftdd: f64,
    ftid: [f64; BANDS],
    ftii: [f64; BANDS],
    fabd: Vec<[f64; BANDS]>,
    fabi: Vec<[f64; BANDS]>,
    fadd: Vec<[f64; BANDS]>,
    psun: Vec<f64>,
    thermal_gap: Vec<f64>,
    shade: Vec<f64>,
}

fn quad_tee(depth: f64) -> f64 {
    crate::extended::tee(DoubleDouble::new(depth))
}

/// `tee(DD1*depth/gee*g)`：`/gee` 折成 `*2`，整条在四精度里（`:605`、`:785` 的 GIMPLE）。
fn quad_tee_projected(depth: f64, projection: f64) -> f64 {
    crate::extended::tee(
        DoubleDouble::new(projection) * (DoubleDouble::new(depth) * DoubleDouble::new(2.0)),
    )
}

/// `ThreeDCanopy`（`MOD_3DCanopyRadiation.F90:284-1106`），逐句按 GIMPLE。
///
/// `max(·,0)` 后再 `min(·,上界)` 保留两步写法：`clamp` 在上界小于 0 时会 panic，上游不会。
#[allow(
    clippy::too_many_arguments,
    clippy::needless_range_loop,
    clippy::manual_clamp
)]
fn three_d_canopy(
    canopy_layer: &[usize],
    fcover: &[f64],
    size: &[f64],
    height: &[f64],
    chil: &[f64],
    coszen: f64,
    lsai: &[f64],
    rho: &[[f64; BANDS]],
    tau: &[[f64; BANDS]],
    ground: [[f64; RTYPES]; BANDS],
) -> ThreeDCore {
    let count = canopy_layer.len();
    // `:486-491`：`cosz = coszen*sqrt(1/(cdcw²·sin²(zenith) + cos²(zenith)))`，`cdcw = 1`；
    // `cosd` 的两个三角函数值在编译期折成常数，`FMA(1, 0.75-ε, 0.25+ε)` 恰为 1。
    let zenith = coszen.acos();
    // gfortran 把这对 `sin`/`cos` 合成 libm `sincos`，结果与分开调用逐位相同；Rust 的
    // `sin_cos()` 在 release 下被 LLVM 并成 `__sincos_stret`，**不**同 —— AT-Neu PC 全年
    // 1213 次日间调用里有 2 次 `sin` 差 1 ULP，`cosz` 随之差，`tt(1,0)` 与 history 的
    // `sabvsun`/`alb` 差一位。用不内联的分开调用挡住合并。
    let (sine, cosine) = (
        crate::atmosphere::fortran_sin(zenith),
        crate::atmosphere::fortran_cos(zenith),
    );
    let cosz = coszen * (1.0 / (sine * sine).contract(1.0, cosine * cosine)).sqrt();
    let cosd = f64::from_bits(0x3fe0_0000_0000_0001);
    let mut gdir = vec![0.0; count];
    let mut gdif = vec![0.0; count];
    for index in 0..count {
        let (phi1, phi2) = leaf_projection(chil[index]);
        gdir[index] = phi2.contract(cosz, phi1);
        gdif[index] = phi2.contract(cosd, phi1);
    }

    // `:506-535`：层聚合。`fc0` 是普通加法，其余都是 `FMA(fcover, x, acc)`。
    let mut active = vec![false; count];
    let mut fc0 = [0.0; LAYERS];
    let mut csiz_lay = [0.0; LAYERS];
    let mut chgt_lay = [0.0; LAYERS];
    let mut lsai_lay = [0.0; LAYERS];
    let mut cosz_lay = [0.0; LAYERS];
    let mut cosd_lay = [0.0; LAYERS];
    let mut gdir_lay = [0.0; LAYERS];
    let mut gdif_lay = [0.0; LAYERS];
    let mut rho_lay = [[0.0; BANDS]; LAYERS];
    let mut tau_lay = [[0.0; BANDS]; LAYERS];
    let mut omg_lay = [[0.0; BANDS]; LAYERS];
    let mut omega = vec![[0.0; BANDS]; count];
    for index in 0..count {
        if !(lsai[index] > 1.0e-6 && fcover[index] > 0.0) {
            continue;
        }
        active[index] = true;
        let lev = canopy_layer[index] - 1;
        let f = fcover[index];
        fc0[lev] += f;
        csiz_lay[lev] = f.contract(size[index], csiz_lay[lev]);
        chgt_lay[lev] = f.contract(height[index], chgt_lay[lev]);
        lsai_lay[lev] = lsai[index].contract(f, lsai_lay[lev]);
        cosz_lay[lev] = f.contract(cosz, cosz_lay[lev]);
        cosd_lay[lev] = f.contract(cosd, cosd_lay[lev]);
        gdir_lay[lev] = f.contract(gdir[index], gdir_lay[lev]);
        gdif_lay[lev] = f.contract(gdif[index], gdif_lay[lev]);
        for band in 0..BANDS {
            omega[index][band] = rho[index][band] + tau[index][band];
            tau_lay[lev][band] = f.contract(tau[index][band], tau_lay[lev][band]);
            rho_lay[lev][band] = f.contract(rho[index][band], rho_lay[lev][band]);
            omg_lay[lev][band] = f.contract(omega[index][band], omg_lay[lev][band]);
        }
    }
    // `:546-563`：除以 `fc0`（不是乘倒数），再 `max(·, 0)`。
    let mut hbot_lay = [0.0; LAYERS];
    for lev in 0..LAYERS {
        if fc0[lev] > 0.0 {
            let c = fc0[lev];
            csiz_lay[lev] = (csiz_lay[lev] / c).max(0.0);
            chgt_lay[lev] = (chgt_lay[lev] / c).max(0.0);
            hbot_lay[lev] = chgt_lay[lev] - csiz_lay[lev];
            lsai_lay[lev] = (lsai_lay[lev] / c).max(0.0);
            cosz_lay[lev] = (cosz_lay[lev] / c).max(0.0);
            cosd_lay[lev] = (cosd_lay[lev] / c).max(0.0);
            for band in 0..BANDS {
                tau_lay[lev][band] = (tau_lay[lev][band] / c).max(0.0);
                rho_lay[lev][band] = (rho_lay[lev][band] / c).max(0.0);
                omg_lay[lev][band] = (omg_lay[lev][band] / c).max(0.0);
            }
            gdir_lay[lev] = (gdir_lay[lev] / c).max(0.0);
            gdif_lay[lev] = (gdif_lay[lev] / c).max(0.0);
        }
    }

    // `:571-580`：分母是 `FNMA(fc0, exp(-1/cos), 1)`。
    let mut shadow_d = [0.0; LAYERS];
    let mut shadow_i = [0.0; LAYERS];
    let shadow = |fc: f64, cos: f64| {
        let value = (1.0 - (-(fc / cos)).exp()) / (-fc).contract((-(1.0 / cos)).exp(), 1.0);
        fc.max(value)
    };
    for lev in 0..LAYERS {
        if fc0[lev] > 0.0 && cosz_lay[lev] > 0.0 {
            shadow_d[lev] = shadow(fc0[lev], cosz_lay[lev]);
            shadow_i[lev] = shadow(fc0[lev], cosd_lay[lev]);
        }
    }

    // `:592-613`：`taud = (fc0*0.375)*lsai/(cosz*shadow)`。
    let mut taud_lay = [0.0; LAYERS];
    let mut taui_lay = [0.0; LAYERS];
    let mut ftdd_lay = [0.0; LAYERS];
    let mut ftdi_lay = [0.0; LAYERS];
    let mut fcad_lay = [1.0; LAYERS];
    let mut fcai_lay = [1.0; LAYERS];
    let mut ftdd_lay_orig = [0.0; LAYERS];
    let mut ftdi_lay_orig = [0.0; LAYERS];
    for lev in 0..LAYERS {
        if !(fc0[lev] > 0.0 && lsai_lay[lev] > 0.0) {
            continue;
        }
        let numerator = fc0[lev] * 0.375 * lsai_lay[lev];
        taud_lay[lev] = numerator / (cosz_lay[lev] * shadow_d[lev]);
        taui_lay[lev] = numerator / (cosd_lay[lev] * shadow_i[lev]);
        ftdd_lay_orig[lev] = quad_tee(taud_lay[lev]);
        ftdi_lay_orig[lev] = quad_tee(taui_lay[lev]);
        ftdd_lay[lev] = quad_tee_projected(taud_lay[lev], gdir_lay[lev]);
        ftdi_lay[lev] = quad_tee_projected(taui_lay[lev], gdif_lay[lev]);
        fcad_lay[lev] = (1.0 - ftdd_lay[lev]) / (1.0 - ftdd_lay_orig[lev]);
        fcai_lay[lev] = (1.0 - ftdi_lay[lev]) / (1.0 - ftdi_lay_orig[lev]);
        // `:628-639`
    }

    // `:655-661`：`shad_oa = fc0*OverlapArea(...)` 不单独舍入，下面直接熔进 FMA。
    let zenith3 = cosz_lay[2].acos();
    let oa32 = overlap_area(csiz_lay[2], chgt_lay[2] - hbot_lay[1], zenith3);
    let oa31 = overlap_area(csiz_lay[2], chgt_lay[2] - hbot_lay[0], zenith3);
    let zenith2 = cosz_lay[1].acos();
    let oa21 = overlap_area(csiz_lay[1], chgt_lay[1] - hbot_lay[0], zenith2);
    let (sd1, sd2, sd3) = (shadow_d[0], shadow_d[1], shadow_d[2]);
    let s21 = (-oa21).contract(fc0[1], sd2); // sd2 - shad_oa(2,1)
    let s31 = (-oa31).contract(fc0[2], sd3); // sd3 - shad_oa(3,1)
    let s32 = (-oa32).contract(fc0[2], sd3); // sd3 - shad_oa(3,2)
    let mut tt = [[0.0; 5]; 5];
    // `:670-686`
    tt[4][3] = sd3.max(0.0).min(1.0);
    tt[4][2] = (sd2 * oa32.contract(fc0[2], 1.0 - sd3))
        .max(0.0)
        .min(1.0 - tt[4][3]);
    let s21_s32 = s21 * s32;
    tt[4][1] = (sd1 * (((1.0 - s21) - s31) + s21_s32))
        .max(0.0)
        .min(1.0 - tt[4][3] - tt[4][2]);
    let sd1_s21 = sd1 * s21;
    let sd2_s32 = sd2 * s32;
    let open = (-sd1).contract(s31, (sd3 + (sd2 + sd1)) - sd1_s21 - sd2_s32);
    tt[4][0] = (1.0 - sd1.contract(s21_s32, open))
        .max(0.0)
        .min(1.0 - tt[4][3] - tt[4][2] - tt[4][1]);
    if sd3 > 0.0 {
        // `:693-702`
        tt[3][2] = sd3.min(sd2_s32.max(0.0));
        tt[3][1] = (sd1 * (s31 - s21_s32)).max(0.0).min(sd3 - tt[3][2]);
        tt[3][0] = sd3 - tt[3][2] - tt[3][1];
        tt[3][2] *= ftdd_lay[2];
        tt[3][1] *= ftdd_lay[2];
        tt[3][0] *= ftdd_lay[2];
    }
    let tt32 = tt[4][2] + tt[3][2];
    if sd2 > 0.0 {
        // `:707-712`
        tt[2][1] = sd2.min(sd1_s21.max(0.0));
        tt[2][0] = sd2 - tt[2][1];
        tt[2][1] = ftdd_lay[1] * tt[2][1] * tt32 / sd2;
        tt[2][0] = tt32 * (tt[2][0] * ftdd_lay[1]) / sd2;
    }
    let tt21 = tt[2][1] + (tt[3][1] + tt[4][1]);
    if sd1 > 0.0 {
        tt[1][0] = ftdd_lay[0] * tt21;
    }
    let tt43 = tt[4][3];
    let tt10 = tt[4][0] + tt[3][0] + tt[2][0] + tt[1][0];
    let ftdd_col = tt10;

    let mut out = ThreeDCore {
        albd: [0.0; BANDS],
        albi: [0.0; BANDS],
        ftdd: ftdd_col,
        ftid: [0.0; BANDS],
        ftii: [0.0; BANDS],
        fabd: vec![[0.0; BANDS]; count],
        fabi: vec![[0.0; BANDS]; count],
        fadd: vec![[0.0; BANDS]; count],
        psun: vec![0.0; count],
        thermal_gap: vec![1.0; count],
        shade: vec![0.0; count],
    };
    let mut psun_lay = [0.0; LAYERS];
    for band in 0..BANDS {
        let albgrd = ground[band][0];
        let albgri = ground[band][1];
        // `:752-796`：逐 PFT 的光学深度与未散射透过率。
        let mut ftdi = vec![1.0; count];
        let mut taud = vec![0.0; count];
        let mut taui = vec![0.0; count];
        let mut shadow_pd = vec![0.0; count];
        let mut shadow_pi = vec![0.0; count];
        let mut ftdd = vec![0.0; count];
        let mut ftdd_orig = vec![0.0; count];
        let mut ftdi_orig = vec![0.0; count];
        let mut fcad = vec![0.0; count];
        let mut fcai = vec![0.0; count];
        for index in 0..count {
            if !active[index] {
                continue;
            }
            let lev = canopy_layer[index] - 1;
            let pfc = (fcover[index] / fc0[lev]).min(1.0);
            shadow_pd[index] = shadow_d[lev] * pfc;
            shadow_pi[index] = shadow_i[lev] * pfc;
            let numerator = fcover[index] * 0.375 * lsai[index];
            taud[index] = numerator / (shadow_pd[index] * cosz);
            taui[index] = numerator / (shadow_pi[index] * cosd);
            ftdd_orig[index] = quad_tee(taud[index]);
            ftdi_orig[index] = quad_tee(taui[index]);
            ftdd[index] = quad_tee_projected(taud[index], gdir[index]);
            ftdi[index] = quad_tee_projected(taui[index], gdif[index]);
            fcad[index] = (1.0 - ftdd[index]) / (1.0 - ftdd_orig[index]);
            fcai[index] = (1.0 - ftdi[index]) / (1.0 - ftdi_orig[index]);
        }

        // `:801-823`：层的 `CanopyRad`，再按 `fcad`/`fcai` 校正。
        let mut layer = [CanopyRadOutput::default(); LAYERS];
        for lev in 0..LAYERS {
            layer[lev].ftii = 1.0;
            if shadow_d[lev] > 0.0 {
                layer[lev] = canopy_rad(
                    taud_lay[lev],
                    taui_lay[lev],
                    ftdd_lay_orig[lev],
                    ftdi_lay_orig[lev],
                    cosz_lay[lev],
                    cosd_lay[lev],
                    shadow_d[lev],
                    shadow_i[lev],
                    fc0[lev],
                    omg_lay[lev][band],
                    lsai_lay[lev],
                    tau_lay[lev][band],
                    rho_lay[lev][band],
                );
            }
            let l = &mut layer[lev];
            l.ftid *= fcad_lay[lev];
            l.ftii = fcai_lay[lev].contract(l.ftii - ftdi_lay_orig[lev], ftdi_lay[lev]);
            l.frid *= fcad_lay[lev];
            l.frii *= fcai_lay[lev];
            l.faid *= fcad_lay[lev];
            l.faii *= fcai_lay[lev];
        }
        // `:829-836`
        let tt_down = [tt21, tt32, tt43];
        let mut fadd_lay = [0.0; LAYERS];
        for lev in 0..LAYERS {
            if fc0[lev] > 0.0 && lsai_lay[lev] > 0.0 {
                fadd_lay[lev] = tt_down[lev] * (1.0 - ftdd_lay[lev]) * (1.0 - omg_lay[lev][band]);
            }
        }

        // `:838-866`：六元方程组。
        let (si1, si2, si3) = (shadow_i[0], shadow_i[1], shadow_i[2]);
        let (l1, l2, l3) = (layer[0], layer[1], layer[2]);
        let mut a = [[0.0; 6]; 6];
        let mut b = [[0.0; 2]; 6];
        a[0][0] = 1.0;
        a[0][2] = (-si3).contract(l3.ftii, si3) - 1.0;
        a[1][1] = 1.0;
        a[1][2] = -(si3 * l3.frii);
        a[2][2] = 1.0;
        a[2][1] = -(si2 * l2.frii);
        let open2 = (-si2).contract(l2.ftii, si2) - 1.0;
        a[2][4] = open2;
        a[3][3] = 1.0;
        a[3][4] = a[2][1];
        a[3][1] = open2;
        a[4][4] = 1.0;
        a[4][3] = -(si1 * l1.frii);
        let open1 = (-l1.ftii).contract(si1, si1) - 1.0;
        a[4][5] = open1 * albgri;
        a[5][5] = (-(albgri * si1)).contract(l1.frii, 1.0);
        a[5][3] = open1;
        b[0][0] = tt43 * l3.frid;
        b[0][1] = si3 * l3.frii;
        b[1][0] = tt43 * l3.ftid;
        b[1][1] = si3.contract(l3.ftii, -si3) + 1.0;
        b[2][0] = l2.frid * tt32;
        b[3][0] = l2.ftid * tt32;
        let direct_ground = tt10 * albgrd;
        b[4][0] = l1
            .frid
            .contract(tt21, direct_ground * (l1.ftii.contract(si1, -si1) + 1.0));
        b[5][0] = l1.ftid.contract(tt21, direct_ground * si1 * l1.frii);
        let x = gauss(a, b);

        // `:877-909`
        let f31 = tt43.contract(l3.faid, x[2][0] * si3 * l3.faii);
        let f21 = l2.faid.contract(tt32, si2 * (x[1][0] + x[4][0]) * l2.faii);
        let ground_bounce = direct_ground + (x[3][0] + albgri * x[5][0]);
        let f11 = l1.faid.contract(tt21, ground_bounce * si1 * l1.faii);
        let (one_minus_albgrd, one_minus_albgri) = (1.0 - albgrd, 1.0 - albgri);
        let f32 = (x[2][1] + 1.0) * si3 * l3.faii;
        let f22 = si2 * (x[1][1] + x[4][1]) * l2.faii;
        let f12 = (x[3][1] + albgri * x[5][1]) * si1 * l1.faii;
        let fabd_lay = [f11, f21, f31];
        let fabi_lay = [f12, f22, f32];
        let fabd_col = f31 + (f21 + f11);
        let fabi_col = f32 + (f22 + f12);
        let albd_col = x[0][0];
        let albi_col = x[0][1];

        if band == 0 {
            // `:916-956`：只保留 `psun`。同一段算的 `fsun_id_lay`/`fsun_ii_lay` 在
            // `fsun3D = .false.` 时被 wrap 整个丢弃（`:210-223` 重算），没有读者。
            psun_lay = [0.0; LAYERS];
            if fc0[2] > 0.0 && lsai_lay[2] > 0.0 {
                psun_lay[2] = tt43 / sd3;
            }
            if fc0[1] > 0.0 && lsai_lay[1] > 0.0 {
                psun_lay[1] = tt32 / sd2;
            }
            if fc0[0] > 0.0 && lsai_lay[0] > 0.0 {
                psun_lay[0] = tt21 / sd1;
            }
        }

        // `:973-1035`：逐 PFT 的 `CanopyRad`，按层归一化前的原始份额。
        let mut sum_fabd = [0.0; LAYERS];
        let mut sum_fabi = [0.0; LAYERS];
        let mut sum_fadd = [0.0; LAYERS];
        for index in 0..count {
            if canopy_layer[index] == 0 {
                continue;
            }
            let lev = canopy_layer[index] - 1;
            if !(shadow_d[lev] > 0.0 && active[index]) {
                continue;
            }
            let sky = shadow_pi[index];
            let pd = shadow_pd[index];
            let r = canopy_rad(
                taud[index],
                taui[index],
                ftdd_orig[index],
                ftdi_orig[index],
                cosz,
                cosd,
                pd,
                sky,
                fcover[index],
                omega[index][band],
                lsai[index],
                tau[index][band],
                rho[index][band],
            );
            let ftid = fcad[index] * r.ftid;
            let ftii = fcai[index].contract(r.ftii - ftdi_orig[index], ftdi[index]);
            let albi = fcai[index] * r.frii;
            let faid = fcad[index] * r.faid;
            let faii = fcai[index] * r.faii;
            let one_minus_probm = (-albgri).contract(sky * albi, 1.0);
            let ftran = albgrd.contract(pd.contract(ftdd[index], 1.0 - pd), albgri * (ftid * pd));
            let fabsm = sky * (faii * ftran) / one_minus_probm;
            let fabd = faid.contract(pd, fabsm);
            let ftran = (-sky).contract(1.0 - ftii, 1.0);
            let fabsm = sky * (faii * (albgri * ftran)) / one_minus_probm;
            let fabi = sky.contract(faii, fabsm);
            sum_fabd[lev] += fabd;
            sum_fabi[lev] += fabi;
            let fadd = pd * (1.0 - ftdd[index]) * (1.0 - omega[index][band]);
            sum_fadd[lev] += fadd;
            out.fabd[index][band] = fabd;
            out.fabi[index][band] = fabi;
            out.fadd[index][band] = fadd;
        }
        // `:1040-1086`
        for index in 0..count {
            if active[index] {
                let lev = canopy_layer[index] - 1;
                let f = fcover[index];
                let fabd = out.fabd[index][band] * fabd_lay[lev] / sum_fabd[lev] / f;
                let fabi = out.fabi[index][band] * fabi_lay[lev] / sum_fabi[lev] / f;
                let fadd = out.fadd[index][band] * fadd_lay[lev] / sum_fadd[lev] / f;
                out.fabd[index][band] = fabd;
                out.fabi[index][band] = fabi;
                out.fadd[index][band] = fabd.min(fadd);
                out.psun[index] = psun_lay[lev];
            } else {
                out.fabd[index][band] = 0.0;
                out.fabi[index][band] = 0.0;
                out.fadd[index][band] = 0.0;
                out.psun[index] = 0.0;
            }
        }
        out.albd[band] = albd_col;
        out.albi[band] = albi_col;
        out.ftid[band] =
            (((1.0 - albd_col) - fabd_col) - ftdd_col * one_minus_albgrd) / one_minus_albgri;
        out.ftii[band] = ((1.0 - albi_col) - fabi_col) / one_minus_albgri;
        if band == 0 {
            out.thermal_gap = ftdi.clone();
        }
        out.shade = shadow_pi;
    }
    out
}

#[derive(Debug, Clone, Copy, Default)]
struct CanopyRadOutput {
    ftid: f64,
    ftii: f64,
    frid: f64,
    frii: f64,
    faid: f64,
    faii: f64,
}

/// `CanopyRad`（`:1161-1302`，`runmode = .true.`），按 GIMPLE。
#[allow(clippy::too_many_arguments, clippy::manual_clamp)]
fn canopy_rad(
    tau_d: f64,
    tau_i: f64,
    ftdd: f64,
    ftdi: f64,
    cosz: f64,
    cosd: f64,
    shadow_d: f64,
    shadow_i: f64,
    fc: f64,
    omg: f64,
    lsai: f64,
    tau_p: f64,
    rho_p: f64,
) -> CanopyRadOutput {
    use crate::extended::canopy_scattering_runmode as phi;
    let tau = lsai * 0.375;
    let (tot_d, dif_d, _) = phi(tau_d, omg, tau_p, rho_p);
    let (tot_i, dif_i, _) = phi(tau_i, omg, tau_p, rho_p);
    let (tot_o, dif_o, pa2) = phi(tau, omg, tau_p, rho_p);
    let frio = ((-dif_o).contract(0.5, tot_o) * 0.5).min(1.0).max(0.0);
    let spread = fc * 1.732_050_807_568_877_2;
    let near = 1.0 - (1.0 - spread / std::f64::consts::TAU).sqrt();
    let far = 1.0 - (1.0 - spread / (3.0 * std::f64::consts::TAU)).sqrt();
    let muv = near.contract(3.0, far * 3.0);
    let wb = rho_p.contract(2.0 / 3.0, tau_p * (1.0 / 3.0));
    let dry = 1.0 - omg;
    let alpha = dry.sqrt() * wb.contract(2.0, dry).sqrt();
    let two_alpha = alpha * 2.0;
    let nd = (two_alpha + 1.0) / two_alpha.contract(cosz, 1.0);
    let ni = (two_alpha + 1.0) / two_alpha.contract(cosd, 1.0);
    let ac = tot_o * muv * (1.0 - quad_tee(tau)) * dry / (-omg).contract(pa2, 1.0);
    let ald = fc * ((nd - 1.0) * frio) * (1.0 / shadow_d - cosz / fc);
    let ali = fc * ((ni - 1.0) * frio) * (1.0 / shadow_i - cosd / fc);
    let spread_d = cosz * 0.5 * dif_d;
    let spread_i = cosd * 0.5 * dif_i;
    let mut frid = (-ac)
        .contract(0.5, (tot_d - spread_d).contract(0.5, ald))
        .min(1.0)
        .max(0.0);
    let mut frii = (-ac)
        .contract(0.5, (tot_i - spread_i).contract(0.5, ali))
        .min(1.0)
        .max(0.0);
    let mut ftid = (-ac)
        .contract(0.5, (-ald).contract(0.5, (spread_d + tot_d) * 0.5))
        .min(1.0)
        .max(0.0);
    let mut ftii = (-ac)
        .contract(
            0.5,
            (-ali).contract(0.5, (spread_i + tot_i).contract(0.5, ftdi)),
        )
        .min(1.0)
        .max(0.0);
    let mut faid = (((1.0 - ftdd) - frid) - ftid).min(1.0).max(0.0);
    let mut faii = ((1.0 - frii) - ftii).min(1.0).max(0.0);
    if shadow_d == 0.0 {
        ftid = 0.0;
        frid = 0.0;
        faid = 0.0;
    }
    if shadow_i == 0.0 {
        ftii = 1.0;
        frii = 0.0;
        faii = 0.0;
    }
    CanopyRadOutput {
        ftid,
        ftii,
        frid,
        frii,
        faid,
        faii,
    }
}

/// `OverlapArea`（`:1128-1155`）：`(1/cos+1)*FNMA(sin θ, cost, θ)/π`。
fn overlap_area(radius: f64, height: f64, zenith: f64) -> f64 {
    if radius == 0.0 {
        return 0.0;
    }
    let secant = 1.0 / zenith.cos() + 1.0;
    let cost = height * zenith.tan() / radius / secant;
    if cost >= 1.0 {
        return 0.0;
    }
    let theta = cost.acos();
    secant * (-theta.sin()).contract(cost, theta) / std::f64::consts::PI
}

/// `mGauss`（`:1416-1450`）：消元是 `FNMA(A(j,i)/A(i,i), A(i,k), A(j,k))`，
/// 回代的 `sum(A(i,i+1:6)*X(i+1:6))` 是从 0 起的 FMA 链。
#[allow(clippy::needless_range_loop)]
fn gauss(mut a: [[f64; 6]; 6], mut b: [[f64; 2]; 6]) -> [[f64; 2]; 6] {
    const STEPS: [usize; 5] = [0, 2, 1, 2, 1];
    for i in 0..5 {
        for j in i + 1..=i + STEPS[i] {
            let ratio = a[j][i] / a[i][i];
            for k in 0..6 {
                a[j][k] = (-ratio).contract(a[i][k], a[j][k]);
            }
            for k in 0..2 {
                b[j][k] = (-ratio).contract(b[i][k], b[j][k]);
            }
        }
    }
    let mut x = [[0.0; 2]; 6];
    for k in 0..2 {
        x[5][k] = b[5][k] / a[5][5];
    }
    for i in (0..5).rev() {
        for k in 0..2 {
            let sum = (i + 1..6).fold(0.0, |sum, m| a[i][m].contract(x[m][k], sum));
            x[i][k] = (b[i][k] - sum) / a[i][i];
        }
    }
    x
}

#[allow(clippy::type_complexity)]
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
    fn sparse_canopy_transmission_matches_the_upstream_quad_precision_tail() {
        // Actual Pearl River PFT optical depths; MOD_3DCanopyRadiation::tee
        // evaluates in real(r16) before returning f64.
        for (depth, expected) in [
            (5.535_032_188_353_261e-3, 0x3fef_c3ca_fd13_258e),
            (8.204_479_852_876_279e-3, 0x3fef_a6ef_299e_9485),
        ] {
            assert_eq!(quad_tee(depth).to_bits(), expected);
        }
    }

    #[test]
    fn diffuse_pc_state_keeps_the_original_quad_precision_rounding() {
        let values = [
            (0, 0.115_217_488_614_401_69, 0.0, 0.0, 0.0),
            (
                2,
                0.023_185_641_975_434_653,
                0.570_502_674_625_059_4,
                0.664_139_568_501_346_6,
                0.01,
            ),
            (
                2,
                0.001_655_228_518_701_910_8,
                1.078_252_744_197_161,
                0.729_361_705_903_759_7,
                0.1,
            ),
            (
                2,
                0.030_869_091_807_732_268,
                0.380_747_583_159_298_3,
                0.673_486_992_663_461_2,
                0.25,
            ),
            (
                1,
                0.000_410_000_956_726_737_43,
                0.846_162_133_557_223,
                0.797_991_901_655_422_3,
                0.01,
            ),
            (
                1,
                0.000_273_333_969_844_663_2,
                0.846_162_138_285_942,
                0.797_991_901_083_981_7,
                0.25,
            ),
            (
                1,
                0.003_917_038_798_867_017,
                0.860_805_152_754_536_3,
                0.816_439_591_056_742_9,
                -0.3,
            ),
        ];
        let pfts = values.map(|(canopy_layer, fraction, lai, sai, chil)| PcPftInput {
            canopy_layer,
            fraction,
            canopy_top_m: if canopy_layer == 0 { 0.5 } else { 4.0 },
            canopy_bottom_m: if canopy_layer == 0 { 0.0 } else { 1.0 },
            optics: LeafOptics {
                chil,
                reflectance: [[0.1, 0.2], [0.3, 0.4]],
                transmittance: [[0.05, 0.1], [0.2, 0.3]],
            },
            lai,
            sai,
            wet_snow_fraction: 0.0,
        });
        let state = cold_start_pc_broadband_radiation_from_ground(
            &pfts,
            0.5,
            ColdStartGroundAlbedo {
                soil: [[0.2; 2]; 2],
                snow: [[0.8; 2]; 2],
                ground: [[0.2; 2]; 2],
                snow_age: 0.0,
            },
        )
        .unwrap();
        assert_eq!(
            state
                .pft
                .iter()
                .map(|pft| pft.shade_fraction.to_bits())
                .collect::<Vec<_>>(),
            [
                0,
                0x3fca_2881_b9f7_5a6c,
                0x3f8d_e10b_1fe3_ab83,
                0x3fd1_69d3_44fe_09da,
                0x3f72_b561_7599_273d,
                0x3f68_f1d7_4576_c62d,
                0x3fa6_578e_8960_b3be,
            ]
        );
        assert_eq!(
            state
                .pft
                .iter()
                .map(|pft| pft.thermal_gap_fraction.to_bits())
                .collect::<Vec<_>>(),
            [
                0x3ff0_0000_0000_0000,
                0x3fde_1340_cc3e_76df,
                0x3fd6_2532_0e33_5c0e,
                0x3fe1_25f3_8f62_2270,
                0x3fdc_f3d2_9446_4f0f,
                0x3fdd_e2f8_ff1f_7dd8,
                0x3fdb_a6a7_ae7a_5983,
            ]
        );
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
