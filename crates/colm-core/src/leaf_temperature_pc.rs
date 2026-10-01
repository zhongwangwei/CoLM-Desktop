//! `MOD_LeafTemperaturePC:LeafTemperaturePC` —— PC（植物群落）三层冠层的叶温与通量。
//!
//! 与 LCT/PFT 的单冠层 `LeafTemperature` 的区别：PFT 按 `canlay_p` 分到至多三层，每层有
//! 自己的冠层空气温湿度（`taf`/`qaf`），层间阻力由廓线积分（`frd`）给出，长波在层间按
//! 遮蔽比例（`fshade`）逐层传递。所有 PFT 在同一个牛顿迭代里一起解叶温。
//!
//! 每条浮点语句的舍入形状取自 `main/MOD_LeafTemperaturePC.F90` 的 GIMPLE（行号见注释）：
//! gfortran 在 `a*b + c` 上几乎总是收缩成 FMA，所以平铺写法在这里几乎处处是错的。

use anyhow::{bail, ensure, Result};

use crate::{
    canopy_diffusivity, canopy_diffusivity_resistance_analytic, canopy_monin_obukhov_with_scheme,
    canopy_roughness, canopy_wind_speed, effective_canopy_wind,
    initialize_monin_obukhov, saturation_specific_humidity, stomata, update_photosynthesis,
    CanopyDiffusivityProfileInput, CanopyMoninObukhovInput, CanopyWater, CanopyWindProfileInput,
    LeafPhotosynthesisInput, LeafTemperatureInput, MoninObukhovInitialInput, MoninObukhovInput,
    PftColumn, PftParameters, PhotosynthesisUpdateInput, PlantHydraulicInput, StomataInput,
    SurfaceLayerScheme, FREEZING_K,
};

const VON_KARMAN: f64 = 0.4;
const GRAVITY_M_S2: f64 = 9.80616;
const AIR_HEAT_CAPACITY_J_KG_K: f64 = 1004.64;
const WATER_HEAT_CAPACITY_J_KG_K: f64 = 4188.0;
const ICE_HEAT_CAPACITY_J_KG_K: f64 = 2117.27;
const LATENT_HEAT_VAPORIZATION_J_KG: f64 = 2.5104e6;
const LATENT_HEAT_FUSION_J_KG: f64 = 3.336e5;
const STEFAN_BOLTZMANN: f64 = 5.67e-8;
const MAX_ITERATIONS: usize = 40;
const MIN_ITERATIONS: usize = 6;
const MAX_TEMPERATURE_STEP_K: f64 = 3.0;
const TEMPERATURE_TOLERANCE_K: f64 = 0.01;
const FLUX_TOLERANCE_W_M2: f64 = 0.1;
const LAYERS: usize = 3;

/// 一个 PFT 在本步的冠层驱动（`THERMAL` 在调用前备好的逐 PFT 数组）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct PcPftDrive {
    /// `canlay_p`：0 是裸地，1..=3 是层号。
    pub(crate) canopy_layer: usize,
    /// `fcover = pftfrac/sum(pftfrac)`
    pub(crate) fcover: f64,
    pub(crate) par_sunlit_w_m2: f64,
    pub(crate) par_shaded_w_m2: f64,
    /// `fsun_p`（`THERMAL` 在 PC 分支里按 `extkb*lai` 算好）。
    pub(crate) sunlit_fraction: f64,
    /// `sabv_p = sabvsun_p + sabvsha_p`
    pub(crate) absorbed_solar_w_m2: f64,
    pub(crate) retained_rain_kg_m2_s: f64,
    pub(crate) retained_snow_kg_m2_s: f64,
}

/// 逐 PFT 的输出（写回 `_p` 数组，随后按 `pftfrac` 聚合）。
#[derive(Debug, Clone, Default)]
pub(crate) struct PcPftFlux {
    pub(crate) rst: f64,
    pub(crate) assim: f64,
    pub(crate) respc: f64,
    pub(crate) fsenl: f64,
    pub(crate) fevpl: f64,
    pub(crate) etr: f64,
    pub(crate) gssun: f64,
    pub(crate) gssha: f64,
    pub(crate) assimsun: f64,
    pub(crate) etrsun: f64,
    pub(crate) assimsha: f64,
    pub(crate) etrsha: f64,
    pub(crate) hprl: f64,
    pub(crate) dheatl: f64,
    /// `canopy_smelt_mass_p_out`/`canopy_frzc_mass_p_out`：`qmelt*deltim`、`qfrz*deltim`。
    pub(crate) canopy_melt_mass_mm: f64,
    pub(crate) canopy_freeze_mass_mm: f64,
    pub(crate) rstfacsun: f64,
    pub(crate) rstfacsha: f64,
    pub(crate) rootflux: Vec<f64>,
    pub(crate) gs0sun: Option<f64>,
    pub(crate) gs0sha: Option<f64>,
}

/// patch 级输出（`THERMAL` 随后把它们抄进每个 PFT 的 `_p`）。
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct PcPatchFlux {
    pub(crate) taux: f64,
    pub(crate) tauy: f64,
    pub(crate) fseng: f64,
    pub(crate) fseng_soil: f64,
    pub(crate) fseng_snow: f64,
    pub(crate) fevpg: f64,
    pub(crate) fevpg_soil: f64,
    pub(crate) fevpg_snow: f64,
    pub(crate) cgrnd: f64,
    pub(crate) cgrndl: f64,
    pub(crate) cgrnds: f64,
    pub(crate) tref: f64,
    pub(crate) qref: f64,
    pub(crate) dlrad: f64,
    pub(crate) ulrad: f64,
    pub(crate) z0m: f64,
    pub(crate) zol: f64,
    pub(crate) rib: f64,
    pub(crate) ustar: f64,
    pub(crate) qstar: f64,
    pub(crate) tstar: f64,
    pub(crate) fm: f64,
    pub(crate) fh: f64,
    pub(crate) fq: f64,
}

/// `(tl*tl)*(tl*tl)`：gfortran 的 `powmult` 展开。
fn fourth(t: f64) -> f64 {
    let square = t * t;
    square * square
}

/// `(tl*tl)*tl`
fn cube(t: f64) -> f64 {
    (t * t) * t
}

fn max_value(values: &[f64]) -> f64 {
    // `maxval` 忽略 NaN（只要还有非 NaN 元素）——`f64::max` 同义。
    values.iter().copied().fold(f64::NAN, f64::max)
}

/// `LeafTemperaturePC`。
///
/// `leaf` 是 patch 级的叶温输入模板（`leaf_input` 已填好气象、地面边界与选项）；
/// 冠层几何与光学逐 PFT 取自 `parameters`/`columns`/`drive`。
#[allow(
    clippy::too_many_lines,
    clippy::needless_range_loop,
    clippy::too_many_arguments,
    unused_assignments
)]
pub(crate) fn leaf_temperature_pc(
    leaf: LeafTemperatureInput<'_>,
    forcing_air_temperature_k: f64,
    ground_heat_roughness_m: f64,
    ground_friction_velocity_m_s: f64,
    parameters: &[PftParameters],
    columns: &mut [PftColumn],
    drive: &[PcPftDrive],
) -> Result<(PcPatchFlux, Vec<PcPftFlux>)> {
    let n = parameters.len();
    ensure!(
        columns.len() == n && drive.len() == n,
        "PC leaf temperature needs one column and one drive per PFT"
    );
    let deltim = leaf.time_step_seconds;
    let thm = leaf.reference_air_temperature_k;
    let th = leaf.potential_temperature_k;
    let thv = leaf.virtual_potential_temperature_k;
    let qm = leaf.reference_specific_humidity;
    let psrf = leaf.surface_pressure_pa;
    let rhoair = leaf.air_density_kg_m3;
    let frl = leaf.atmospheric_longwave_w_m2;
    let tg = leaf.ground_temperature_k;
    let qg = leaf.ground_specific_humidity;
    let rss = leaf.soil_surface_resistance_s_m;
    let emg = leaf.ground_emissivity;
    let fsno = leaf.snow_cover_fraction;
    let t_precip = leaf.precipitation_temperature_k;
    let vegetation_snow = leaf.options.vegetation_snow;
    let plant_hydraulics = leaf.plant_hydraulics;
    let layers_soil = plant_hydraulics.map_or(0, |p| p.layer_thickness_m.len());

    let lai: Vec<f64> = columns.iter().map(|c| c.leaf_area_index).collect();
    let sai: Vec<f64> = columns.iter().map(|c| c.stem_area_index).collect();
    // `:535`
    let lsai: Vec<f64> = (0..n).map(|i| lai[i] + sai[i]).collect();
    let vegetated: Vec<bool> = (0..n)
        .map(|i| drive[i].fcover > 0.0 && lsai[i] > 1.0e-6)
        .collect();
    let mut tl: Vec<f64> = columns.iter().map(|c| c.leaf.leaf_temperature_k).collect();
    for i in 0..n {
        if !vegetated[i] {
            tl[i] = forcing_air_temperature_k;
        }
    }
    if !vegetated.iter().any(|&v| v) {
        bail!(
            "a PC patch without any vegetated PFT returns early from LeafTemperaturePC with \
             undefined outputs upstream; the Rust port does not reproduce that"
        );
    }

    // `:578`：`FMA(1-fsno, zlnd, fsno*zsno)`
    let z0mg = (1.0 - fsno).mul_add(leaf.soil_roughness_m, fsno * leaf.snow_roughness_m);
    let mut z0qg = z0mg;

    let canlay: Vec<usize> = drive.iter().map(|d| d.canopy_layer).collect();
    let fcover: Vec<f64> = drive.iter().map(|d| d.fcover).collect();
    let htop: Vec<f64> = parameters.iter().map(|p| p.canopy_top_m).collect();
    let hbot: Vec<f64> = parameters.iter().map(|p| p.canopy_bottom_m).collect();
    let extkb: Vec<f64> = columns.iter().map(|c| c.direct_extinction).collect();
    let extkd: Vec<f64> = columns.iter().map(|c| c.diffuse_extinction).collect();
    let thermk: Vec<f64> = columns.iter().map(|c| c.thermal_gap_fraction).collect();
    let fshade: Vec<f64> = columns.iter().map(|c| c.shade_fraction).collect();

    // `:628-638`
    let mut laisun = vec![0.0; n];
    let mut laisha = vec![0.0; n];
    let mut cintsun = vec![[0.0; 3]; n];
    let mut cintsha = vec![[0.0; 3]; n];
    for i in 0..n {
        let fsha = 1.0 - drive[i].sunlit_fraction;
        laisun[i] = lai[i] * drive[i].sunlit_fraction;
        laisha[i] = lai[i] * fsha;
        let a = 0.110 + extkb[i];
        cintsun[i][0] = (1.0 - (-(a * lai[i])).exp()) / a;
        let b = extkb[i] + extkd[i];
        cintsun[i][1] = (1.0 - (-(b * lai[i])).exp()) / b;
        cintsun[i][2] = (1.0 - (-(extkb[i] * lai[i])).exp()) / extkb[i];
        cintsha[i][0] = (1.0 - (-(lai[i] * 0.110)).exp()) / 0.110 - cintsun[i][0];
        cintsha[i][1] = (1.0 - (-(extkd[i] * lai[i])).exp()) / extkd[i] - cintsun[i][1];
        cintsha[i][2] = lai[i] - cintsun[i][2];
    }

    // `:645-667`
    let mut clai = vec![0.0; n];
    let mut fwet = vec![0.0; n];
    let mut sat: Vec<Option<crate::SaturationState>> = vec![None; n];
    for i in 0..n {
        let water = columns[i].leaf.canopy_water;
        if vegetation_snow {
            // `:651`：`FMA(ldew_snow, cpice, FMA(0.2*lsai, cpliq, ldew_rain*cpliq))`
            clai[i] = water.snow_mm.mul_add(
                ICE_HEAT_CAPACITY_J_KG_K,
                (lsai[i] * 0.2).mul_add(
                    WATER_HEAT_CAPACITY_J_KG_K,
                    water.rain_mm * WATER_HEAT_CAPACITY_J_KG_K,
                ),
            );
        }
        if vegetated[i] {
            // `dewfraction(..., colm2024_rain_capacity_for_fwet(...))`：PC 不走截留方案 8，
            // 雨水容量是 `dewmx·max(0, lai+sai)`。
            let capacity = leaf.maximum_dew_mm * (lai[i] + sai[i]).max(0.0);
            fwet[i] = crate::interception::canopy_wetness_with_capacity(
                lai[i],
                sai[i],
                leaf.maximum_dew_mm,
                water,
                vegetation_snow,
                capacity,
            )?
            .wet_fraction;
            sat[i] = Some(saturation_specific_humidity(tl[i], psrf)?);
        }
    }

    // `:682-712`：层几何（`fcover_lays` 算完就被清零 —— 上游如此）。
    let mut htop_lay = [0.0; LAYERS];
    let mut hbot_lay = [0.0; LAYERS];
    let mut lsai_lay = [0.0; LAYERS];
    let mut fcover_lay = [0.0; LAYERS];
    for i in 0..n {
        if vegetated[i] {
            let l = canlay[i] - 1;
            htop_lay[l] = fcover[i].mul_add(htop[i], htop_lay[l]);
            hbot_lay[l] = fcover[i].mul_add(hbot[i], hbot_lay[l]);
            lsai_lay[l] = fcover[i].mul_add(lsai[i], lsai_lay[l]);
            fcover_lay[l] += fcover[i];
        }
    }
    for l in 0..LAYERS {
        if fcover_lay[l] > 0.0 {
            htop_lay[l] /= fcover_lay[l];
            hbot_lay[l] /= fcover_lay[l];
            lsai_lay[l] /= fcover_lay[l];
        }
    }
    let fcover_lays = [0.0; LAYERS + 1];
    let bee = 1.0;
    let active = |l: usize| fcover_lay[l] > 0.0 && lsai_lay[l] > 0.0;

    // `:735-762`
    let mut displa_lay = [0.0; LAYERS];
    let mut displa_lays = [0.0; LAYERS + 1];
    let mut z0m_lay = [0.0; LAYERS];
    let mut z0m_lays = [0.0; LAYERS + 1];
    for l in 0..LAYERS {
        if active(l) {
            let whole = canopy_roughness(lsai_lay[l], htop_lay[l], 1.0)?;
            z0m_lay[l] = whole.momentum_roughness_m;
            displa_lay[l] = whole.displacement_height_m;
            let partial = canopy_roughness(lsai_lay[l], htop_lay[l], fcover_lay[l])?;
            z0m_lays[l + 1] = partial.momentum_roughness_m;
            displa_lays[l + 1] = partial.displacement_height_m;
        }
    }
    z0m_lays[0] = z0mg;
    displa_lays[0] = 0.0;
    for value in z0m_lays.iter_mut() {
        if *value < z0mg {
            *value = z0mg;
        }
    }
    for value in z0m_lay.iter_mut() {
        if *value < z0mg {
            *value = z0mg;
        }
    }
    for l in 1..=LAYERS {
        z0m_lays[l] = max_value(&z0m_lays[0..=l]);
    }
    for l in 1..=LAYERS {
        displa_lays[l] = max_value(&displa_lays[0..=l]);
    }
    let mut z0h_lays = z0m_lays;

    // `:779-808`：`a_lay = a_lay_k71`，另三种写法的 a 不被读。
    let mut a_lay = [0.0; LAYERS];
    for l in 0..LAYERS {
        if active(l) {
            let fai = 1.0 - (-(lsai_lay[l] * 0.5)).exp();
            let sqrtdragc = crate::LibmPow::lpow(fai.mul_add(0.3, 0.003), 0.5).min(0.3);
            let gap = htop_lay[l] - displa_lay[l];
            a_lay[l] = htop_lay[l] / gap / (VON_KARMAN / sqrtdragc);
            displa_lay[l] = displa_lay[l].max(htop_lay[l] * 0.5);
        }
    }
    // `:815-834`
    let mut toplay = 0usize;
    let mut botlay = 0usize;
    let mut numlay = 0usize;
    for l in (1..=LAYERS).rev() {
        if active(l - 1) {
            numlay += 1;
            if toplay == 0 {
                toplay = l;
            }
            botlay = l;
            displa_lay[l - 1] = displa_lay[l - 1].max(hbot_lay[l - 1]);
        }
    }
    let top = toplay - 1;
    let bot = botlay - 1;

    // `:841-906`：逐层长波透过率与传递矩阵。
    let mut thermk_lay = [0.0; LAYERS];
    let mut fshade_lay = [0.0; LAYERS];
    for i in 0..n {
        if fshade[i] > 0.0 && canlay[i] > 0 {
            let l = canlay[i] - 1;
            thermk_lay[l] = fshade[i].mul_add(thermk[i], thermk_lay[l]);
            fshade_lay[l] += fshade[i];
        }
    }
    for l in 0..LAYERS {
        thermk_lay[l] = if fshade_lay[l] > 0.0 {
            thermk_lay[l] / fshade_lay[l]
        } else {
            1.0
        };
    }
    let (f1, f2, f3) = (fshade_lay[0], fshade_lay[1], fshade_lay[2]);
    let mut tdn = [[0.0; 5]; 5]; // tdn[i][j] = tdn(i,j)
    let mut tup = [[0.0; 5]; 5];
    tdn[1][0] = 1.0;
    let one_f1 = 1.0 - f1;
    tdn[2][0] = one_f1;
    let one_f1_f2 = one_f1 - f2;
    let f1f2 = f1 * f2;
    tdn[3][0] = one_f1_f2 + f1f2;
    tdn[4][0] = (-f1f2).mul_add(f3, f2.mul_add(f3, f1.mul_add(f3, f1f2 + (one_f1_f2 - f3))));
    tdn[2][1] = f1;
    let one_f2 = 1.0 - f2;
    tdn[3][1] = f1 * one_f2;
    let open23 = f2.mul_add(f3, one_f2 - f3);
    tdn[4][1] = f1 * open23;
    tdn[3][2] = f2;
    let one_f3 = 1.0 - f3;
    tdn[4][2] = f2 * one_f3;
    tdn[4][3] = f3;
    tup[0][1] = f1;
    tup[0][2] = f2 * one_f1;
    tup[1][2] = f2;
    tup[0][3] = f3 * tdn[3][0];
    tup[1][3] = f3 * one_f2;
    tup[2][3] = f3;
    tup[0][4] = tdn[4][0];
    tup[1][4] = open23;
    tup[2][4] = one_f3;
    tup[3][4] = 1.0;
    let (tk1, tk2, tk3) = (thermk_lay[0], thermk_lay[1], thermk_lay[2]);
    let f1tk1 = f1 * tk1;
    let mut dlvpar = [0.0; LAYERS];
    dlvpar[0] = 1.0;
    let pass1 = one_f1 + f1tk1;
    dlvpar[1] = pass1 * pass1;
    let pass2 = f1.mul_add(one_f2 * tk1, (f2 * tk2).mul_add(pass1, tdn[3][0]));
    dlvpar[2] = pass2 * pass2;

    // `:912-935`
    let mut taf = [0.0; LAYERS];
    let mut qaf = [0.0; LAYERS];
    match numlay {
        1 => {
            taf[top] = (tg + thm) * 0.5;
            qaf[top] = (qm + qg) * 0.5;
        }
        2 => {
            taf[bot] = tg.mul_add(2.0, thm) / 3.0;
            qaf[bot] = qg.mul_add(2.0, qm) / 3.0;
            taf[top] = thm.mul_add(2.0, tg) / 3.0;
            qaf[top] = qm.mul_add(2.0, qg) / 3.0;
        }
        3 => {
            taf[0] = tg.mul_add(3.0, thm) * 0.25;
            qaf[0] = qg.mul_add(3.0, qm) * 0.25;
            taf[1] = (tg + thm) * 0.5;
            qaf[1] = (qg + qm) * 0.5;
            taf[2] = thm.mul_add(3.0, tg) * 0.25;
            qaf[2] = qm.mul_add(3.0, qg) * 0.25;
        }
        _ => bail!("PC canopy has {numlay} active layers"),
    }

    // `:941-992`
    let mut pco2a = leaf.atmospheric_co2_pa;
    let tprcor = psrf * 12182.936000000002 / 1.013e5;
    let rsoil = 0.22 * 1.0e-6;
    let z0mv = z0m_lays[3];
    let mut z0hv = z0m_lays[3];
    let mut z0qv = z0m_lays[3];
    let us = leaf.eastward_wind_m_s;
    let vs = leaf.northward_wind_m_s;
    let ur = us.mul_add(us, vs * vs).sqrt().max(0.1);
    let mut dth = thm - taf[top];
    let mut dqh = qm - qaf[top];
    let one_plus_061_qm = qm.mul_add(0.61, 1.0);
    let th061 = th * 0.61;
    let dthv = dth.mul_add(one_plus_061_qm, dqh * th061);
    let (hu, ht, hq) = match leaf.options.observation_height_mode {
        crate::ObservationHeightMode::Absolute => {
            let floor = htop_lay[top] + 1.0;
            (
                if leaf.wind_height_m <= floor {
                    floor
                } else {
                    leaf.wind_height_m
                },
                if leaf.temperature_height_m <= floor {
                    floor
                } else {
                    leaf.temperature_height_m
                },
                if leaf.humidity_height_m <= floor {
                    floor
                } else {
                    leaf.humidity_height_m
                },
            )
        }
        crate::ObservationHeightMode::RelativeToCanopy => (
            leaf.wind_height_m + htop_lay[top],
            leaf.temperature_height_m + htop_lay[top],
            leaf.humidity_height_m + htop_lay[top],
        ),
    };
    let zldis = hu - displa_lays[toplay];
    ensure!(
        zldis > 0.0,
        "the obs height of u is below the PC zero displacement height"
    );
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: ur,
        potential_temperature_k: th,
        reference_temperature_k: thm,
        virtual_potential_temperature_k: thv,
        temperature_difference_k: dth,
        humidity_difference_kg_kg: dqh,
        virtual_temperature_difference_k: dthv,
        reference_height_m: zldis,
        momentum_roughness_m: z0mv,
    })?;
    let mut um = initial.stability_adjusted_wind_m_s;
    let mut obu = initial.obukhov_length_m;

    let mut nmozsgn = 0;
    let mut obuold = 0.0;
    let mut del = vec![0.0; n];
    let mut dele = vec![0.0; n];
    let mut del2;
    let mut dele2;
    let mut dtl = vec![[0.0; MAX_ITERATIONS + 2]; n];
    let mut dtl_noadj = vec![0.0; n];
    let mut fevpl_bef = vec![0.0; n];
    let mut tlbef = tl.clone();
    let mut ueff_lay_norm = [0.0; LAYERS];
    let mut fluxes = vec![PcPftFlux::default(); n];
    for f in &mut fluxes {
        // `THERMAL` 的 PC 初始化（`MOD_Thermal.F90:1052-1079`）。
        f.rst = 2.0e4;
        f.rstfacsun = 1.0;
        f.rstfacsha = 1.0;
        f.rootflux = vec![0.0; layers_soil];
    }
    let mut rssun = vec![0.0; n];
    let mut rssha = vec![0.0; n];
    let mut respcsun = vec![0.0; n];
    let mut respcsha = vec![0.0; n];
    let mut rb = vec![0.0; n];
    let mut delta = vec![0.0; n];
    let mut cfh = vec![0.0; n];
    let mut cfw = vec![0.0; n];
    let mut wlh = vec![0.0; n];
    let mut wlq = vec![0.0; n];
    let mut fsenl_dtl = vec![0.0; n];
    let mut etr_dtl = vec![0.0; n];
    let mut evplwet = vec![0.0; n];
    let mut evplwet_dtl = vec![0.0; n];
    let mut fevpl_dtl = vec![0.0; n];
    let mut fevpl_noadj = vec![0.0; n];
    let mut erre = vec![0.0; n];
    let mut irab = vec![0.0; n];
    let mut dirab = vec![0.0; n];
    let mut wah = [0.0; LAYERS];
    let mut wgh = [0.0; LAYERS];
    let mut waq = [0.0; LAYERS];
    let mut wgq = [0.0; LAYERS];
    let mut wlhl = [0.0; LAYERS];
    let mut wlql = [0.0; LAYERS];
    let mut cgh = [0.0; LAYERS];
    let mut cgw = [0.0; LAYERS];
    let mut rd = [0.0; LAYERS];
    let mut fact = 1.0;
    let mut facq = 1.0;
    let mut lin = [0.0; 5];
    let mut ram = 0.0;
    let mut raw = 0.0;
    let mut fh = 0.0;
    let mut fq = 0.0;
    let mut fm = 0.0;
    let mut fht = 0.0;
    let mut fqt = 0.0;
    let mut fh2m = 0.0;
    let mut fq2m = 0.0;
    let mut ustar = 0.0;
    let mut zeta = 0.0;
    let mut tstar = 0.0;
    let mut qstar = 0.0;
    let mut it = 1;

    while it <= MAX_ITERATIONS {
        tlbef = tl.clone();
        del2 = del.clone();
        dele2 = dele.clone();

        // `:1018-1026`
        let profile = canopy_monin_obukhov_with_scheme(
            CanopyMoninObukhovInput {
                surface: MoninObukhovInput {
                    wind_height_m: hu,
                    temperature_height_m: ht,
                    humidity_height_m: hq,
                    displacement_height_m: displa_lays[toplay],
                    momentum_roughness_m: z0mv,
                    heat_roughness_m: z0hv,
                    moisture_roughness_m: z0qv,
                    obukhov_length_m: obu,
                    stability_adjusted_wind_m_s: um,
                    boundary_layer_height_m: leaf.boundary_layer_height_m,
                },
                top_layer_displacement_m: displa_lay[top],
                top_layer_roughness_m: z0m_lay[top],
                canopy_top_height_m: htop_lay[top],
            },
            leaf.options.surface_layer_scheme,
        )?;
        let surface = profile.surface;
        ustar = surface.friction_velocity_m_s;
        fm = surface.momentum;
        fh = surface.heat;
        fq = surface.moisture;
        fh2m = surface.heat_at_2m;
        fq2m = surface.moisture_at_2m;
        fht = profile.heat_at_top_layer;
        fqt = profile.moisture_at_top_layer;
        let fmtop = profile.momentum_at_canopy_top;
        let phih = profile.canopy_top_heat_similarity;
        // `:1034-1039`
        ram = 1.0 / (ustar * ustar / um);
        let rah = 1.0 / (ustar * (VON_KARMAN / (fh - fht)));
        raw = 1.0 / (ustar * (VON_KARMAN / (fq - fqt)));
        // `:1042`
        let z0hg = z0mg / (crate::LibmPow::lpow(ustar * z0mg / 1.5e-5, 0.45) * 0.13).exp();
        z0qg = z0hg;
        z0h_lays[0] = z0hg;
        for l in 1..=LAYERS {
            z0h_lays[l] = max_value(&z0h_lays[0..=l]);
        }
        z0hv = z0h_lays[3];
        z0qv = z0h_lays[3];

        // `:1062-1146`：层间阻力。
        rd = [0.0; LAYERS];
        let mut upplay = 0usize;
        let utop = ustar / VON_KARMAN * fmtop;
        let ktop = ustar * ((htop_lay[top] - displa_lays[toplay]) * VON_KARMAN) / phih;
        let displah = displa_lays[toplay] / htop_lay[top];
        let mut utop_lay = [0.0; LAYERS];
        let mut ubot_lay = [0.0; LAYERS];
        let mut ktop_lay = [0.0; LAYERS];
        let mut kbot_lay = [0.0; LAYERS];
        let mut ueff_lay = [0.0; LAYERS];
        let diffusivity =
            |k: f64, fc: f64, alpha: f64, h_top: f64, h_bot: f64| CanopyDiffusivityProfileInput {
                diffusivity_at_canopy_top_m2_s: k,
                canopy_cover_fraction: fc,
                canopy_blend_weight: bee,
                attenuation_coefficient: alpha,
                displacement_height_m: displah,
                canopy_top_height_m: h_top,
                canopy_bottom_height_m: h_bot,
                obukhov_length_m: leaf.ground_obukhov_length_m,
                friction_velocity_m_s: ground_friction_velocity_m_s,
            };
        for l in (1..=toplay).rev() {
            let li = l - 1;
            if !active(li) {
                continue;
            }
            if l == toplay {
                utop_lay[li] = utop;
                ktop_lay[li] = ktop;
            } else {
                let up = upplay - 1;
                utop_lay[li] = canopy_wind_speed(
                    CanopyWindProfileInput {
                        wind_at_canopy_top_m_s: ubot_lay[up],
                        canopy_cover_fraction: fcover_lays[upplay],
                        canopy_blend_weight: bee,
                        attenuation_coefficient: 0.0,
                        ground_momentum_roughness_m: z0mg,
                        canopy_top_height_m: hbot_lay[up],
                        canopy_bottom_height_m: htop_lay[li],
                    },
                    htop_lay[li],
                )?;
                ktop_lay[li] = canopy_diffusivity(
                    diffusivity(
                        kbot_lay[up],
                        fcover_lays[upplay],
                        0.0,
                        hbot_lay[up],
                        htop_lay[li],
                    ),
                    htop_lay[li],
                )?;
                rd[up] += canopy_diffusivity_resistance_analytic(
                    diffusivity(
                        kbot_lay[up],
                        fcover_lays[upplay],
                        0.0,
                        hbot_lay[up],
                        htop_lay[li],
                    ),
                    hbot_lay[up],
                    htop_lay[li],
                    ground_heat_roughness_m,
                )?;
            }
            // `:1099`
            hbot_lay[li] = hbot_lay[li].max(displa_lays[l - 1] + z0m_lays[l - 1]);
            let wind = CanopyWindProfileInput {
                wind_at_canopy_top_m_s: utop_lay[li],
                canopy_cover_fraction: fcover_lay[li],
                canopy_blend_weight: bee,
                attenuation_coefficient: a_lay[li],
                ground_momentum_roughness_m: z0mg,
                canopy_top_height_m: htop_lay[li],
                canopy_bottom_height_m: hbot_lay[li],
            };
            ubot_lay[li] = canopy_wind_speed(wind, hbot_lay[li])?;
            if it == 1 {
                ueff_lay_norm[li] = effective_canopy_wind(CanopyWindProfileInput {
                    wind_at_canopy_top_m_s: 1.0,
                    ..wind
                })?;
            }
            ueff_lay[li] = utop_lay[li] * ueff_lay_norm[li];
            let layer_diffusivity = diffusivity(
                ktop_lay[li],
                fcover_lay[li],
                a_lay[li],
                htop_lay[li],
                hbot_lay[li],
            );
            kbot_lay[li] = canopy_diffusivity(layer_diffusivity, hbot_lay[li])?;
            if upplay > 0 {
                rd[upplay - 1] += canopy_diffusivity_resistance_analytic(
                    layer_diffusivity,
                    htop_lay[li],
                    displa_lay[li] + z0m_lay[li],
                    ground_heat_roughness_m,
                )?;
            }
            rd[li] += canopy_diffusivity_resistance_analytic(
                layer_diffusivity,
                displa_lay[li] + z0m_lay[li],
                z0qg.max(hbot_lay[li]),
                ground_heat_roughness_m,
            )?;
            upplay = l;
        }
        // `:1144`
        rd[bot] += canopy_diffusivity_resistance_analytic(
            diffusivity(kbot_lay[bot], fcover_lays[botlay], 0.0, hbot_lay[bot], z0qg),
            hbot_lay[bot],
            z0qg,
            ground_heat_roughness_m,
        )?;

        // `:1151-1159`
        for i in 0..n {
            if vegetated[i] {
                let cf = parameters[i].inverse_sqrt_leaf_dimension_m_neg_half
                    * 0.01
                    * ueff_lay[canlay[i] - 1].sqrt();
                rb[i] = 1.0 / cf;
            }
        }

        // `:1186-1271`：气孔（PHS 时再解一遍植物水力）。
        for i in 0..n {
            if !(fcover[i] > 0.0 && lai[i] > 0.001) {
                rssun[i] = 2.0e4;
                fluxes[i].assimsun = 0.0;
                respcsun[i] = 0.0;
                rssha[i] = 2.0e4;
                fluxes[i].assimsha = 0.0;
                respcsha[i] = 0.0;
                if plant_hydraulics.is_some() {
                    fluxes[i].etr = 0.0;
                    fluxes[i].rootflux = vec![0.0; layers_soil];
                }
                continue;
            }
            let rbsun = rb[i] / laisun[i];
            let rbsha = rb[i] / laisha[i];
            let l = canlay[i] - 1;
            let eah = psrf * qaf[l] / qaf[l].mul_add(0.378, 0.622);
            let saturation = sat[i].expect("vegetated PFT has a saturation state");
            if plant_hydraulics.is_some() {
                fluxes[i].rstfacsun = 1.0;
                fluxes[i].rstfacsha = 1.0;
            }
            let photosynthesis =
                |par: f64, stress: f64, rb_leaf: f64, cint: [f64; 3]| LeafPhotosynthesisInput {
                    biochemistry: parameters[i].biochemistry,
                    canopy_integration: cint,
                    leaf_temperature_k: tl[i],
                    oxygen_partial_pressure_pa: leaf.oxygen_partial_pressure_pa,
                    absorbed_par_w_m2: par,
                    air_pressure_pa: psrf,
                    soil_water_stress: stress,
                    leaf_boundary_resistance_s_m: rb_leaf,
                };
            let sun = stomata(
                StomataInput {
                    photosynthesis: photosynthesis(
                        drive[i].par_sunlit_w_m2,
                        fluxes[i].rstfacsun,
                        rbsun,
                        cintsun[i],
                    ),
                    atmospheric_co2_pa: leaf.atmospheric_co2_pa,
                    canopy_air_co2_pa: pco2a,
                    canopy_air_vapor_pressure_pa: eah,
                    leaf_saturation_vapor_pressure_pa: saturation.vapor_pressure_pa,
                    wue_lambda: parameters[i].wue_lambda,
                },
                leaf.options.stomata,
            )?;
            let sha = stomata(
                StomataInput {
                    photosynthesis: photosynthesis(
                        drive[i].par_shaded_w_m2,
                        fluxes[i].rstfacsha,
                        rbsha,
                        cintsha[i],
                    ),
                    atmospheric_co2_pa: leaf.atmospheric_co2_pa,
                    canopy_air_co2_pa: pco2a,
                    canopy_air_vapor_pressure_pa: eah,
                    leaf_saturation_vapor_pressure_pa: saturation.vapor_pressure_pa,
                    wue_lambda: parameters[i].wue_lambda,
                },
                leaf.options.stomata,
            )?;
            fluxes[i].assimsun = sun.assimilation_mol_m2_s;
            respcsun[i] = sun.respiration_mol_m2_s;
            rssun[i] = sun.stomatal_resistance_s_m;
            fluxes[i].assimsha = sha.assimilation_mol_m2_s;
            respcsha[i] = sha.respiration_mol_m2_s;
            rssha[i] = sha.stomatal_resistance_s_m;
            if let Some(hydraulic) = plant_hydraulics {
                // `:1231-1232`
                let gs0sun =
                    (1.0 / (rssun[i] * tl[i] / tprcor)).min(1.0e6) / laisun[i] * 1.0e6 * 1.0;
                let gs0sha =
                    (1.0 / (rssha[i] * tl[i] / tprcor)).min(1.0e6) / laisha[i] * 1.0e6 * 1.0;
                fluxes[i].gs0sun = Some(gs0sun);
                fluxes[i].gs0sha = Some(gs0sha);
                // `sum(rd(1:clev))`：从 0 起的普通加法。
                let rd_sum = rd[..=l].iter().fold(0.0, |sum, value| value + sum);
                let traits = parameters[i].plant_hydraulic_traits;
                let state = columns[i].leaf.plant_hydraulics.as_mut().ok_or_else(|| {
                    anyhow::anyhow!("PC plant hydraulics needs a persistent vegwp state")
                })?;
                let output = crate::plant_hydraulic_stress(
                    PlantHydraulicInput {
                        node_depth_m: hydraulic.node_depth_m,
                        layer_thickness_m: hydraulic.layer_thickness_m,
                        root_fraction: &parameters[i].root_fraction,
                        soil_matric_potential_mm: hydraulic.soil_matric_potential_mm,
                        soil_hydraulic_conductivity_mm_s: hydraulic
                            .soil_hydraulic_conductivity_mm_s,
                        saturated_hydraulic_conductivity_mm_s: hydraulic
                            .saturated_hydraulic_conductivity_mm_s,
                        surface_pressure_pa: psrf,
                        leaf_saturation_specific_humidity: saturation.specific_humidity,
                        canopy_air_specific_humidity: qaf[l],
                        ground_specific_humidity: qg,
                        reference_specific_humidity: qm,
                        leaf_temperature_k: tl[i],
                        leaf_boundary_resistance_s_m: rbsun,
                        soil_surface_resistance_s_m: rss,
                        reference_to_canopy_moisture_resistance_s_m: raw,
                        ground_to_canopy_moisture_resistance_s_m: rd_sum,
                        air_density_kg_m3: rhoair,
                        wet_canopy_fraction: fwet[i],
                        sunlit_leaf_area_index: laisun[i],
                        shaded_leaf_area_index: laisha[i],
                        stem_area_index: sai[i],
                        canopy_top_height_m: htop[i],
                        maximum_sunlit_leaf_conductance_umol_m2_s: gs0sun,
                        maximum_shaded_leaf_conductance_umol_m2_s: gs0sha,
                        maximum_sunlit_leaf_hydraulic_conductance: traits
                            .maximum_sunlit_leaf_conductance,
                        maximum_shaded_leaf_hydraulic_conductance: traits
                            .maximum_shaded_leaf_conductance,
                        maximum_xylem_hydraulic_conductance: traits.maximum_xylem_conductance,
                        maximum_root_hydraulic_conductance: traits.maximum_root_conductance,
                        sunlit_leaf_psi50_mm: traits.sunlit_leaf_psi50_mm,
                        shaded_leaf_psi50_mm: traits.shaded_leaf_psi50_mm,
                        xylem_psi50_mm: traits.xylem_psi50_mm,
                        root_psi50_mm: traits.root_psi50_mm,
                        vulnerability_shape: traits.vulnerability_shape,
                        soil_surface_resistance_scheme: hydraulic.soil_surface_resistance_scheme,
                        parameters: hydraulic.parameters,
                    },
                    state,
                )?;
                fluxes[i].rstfacsun = output.sunlit_stress;
                fluxes[i].rstfacsha = output.shaded_stress;
                fluxes[i].etrsun = output.sunlit_transpiration_kg_m2_s;
                fluxes[i].etrsha = output.shaded_transpiration_kg_m2_s;
                fluxes[i].rootflux = output.root_flux_kg_m2_s;
                // `:1245-1259`：`gssun` 先按 `laisun*1e-6` 折成 mol，再交给 `update_photosyn`，
                // 其边界层阻力是 `rb`（不是 `rbsun`）。
                fluxes[i].etr = fluxes[i].etrsun + fluxes[i].etrsha;
                let gssun = output.sunlit_stomatal_conductance_umol_m2_s * laisun[i] * 1.0e-6;
                let gssha = output.shaded_stomatal_conductance_umol_m2_s * laisha[i] * 1.0e-6;
                let update = |par: f64, stress: f64, cint: [f64; 3], g: f64| {
                    update_photosynthesis(
                        PhotosynthesisUpdateInput {
                            photosynthesis: photosynthesis(par, stress, rb[i], cint),
                            atmospheric_co2_pa: leaf.atmospheric_co2_pa,
                            canopy_air_co2_pa: pco2a,
                            canopy_conductance_h2o_umol_m2_s: g,
                        },
                        leaf.options.stomata,
                    )
                };
                let sun = update(
                    drive[i].par_sunlit_w_m2,
                    fluxes[i].rstfacsun,
                    cintsun[i],
                    gssun,
                )?;
                let sha = update(
                    drive[i].par_shaded_w_m2,
                    fluxes[i].rstfacsha,
                    cintsha[i],
                    gssha,
                )?;
                fluxes[i].assimsun = sun.assimilation_mol_m2_s;
                respcsun[i] = sun.respiration_mol_m2_s;
                fluxes[i].assimsha = sha.assimilation_mol_m2_s;
                respcsha[i] = sha.respiration_mol_m2_s;
                let base = tprcor / tl[i];
                rssun[i] = base / gssun;
                rssha[i] = base / gssha;
            }
        }
        // `:1275-1276`
        for i in 0..n {
            rssun[i] *= laisun[i];
            rssha[i] *= laisha[i];
        }

        // `:1283-1301`
        for i in 0..n {
            if !vegetated[i] {
                cfh[i] = 0.0;
                cfw[i] = 0.0;
                continue;
            }
            let l = canlay[i] - 1;
            let saturation = sat[i].expect("vegetated");
            delta[i] = if saturation.specific_humidity - qaf[l] > 0.0 {
                1.0
            } else {
                0.0
            };
            cfh[i] = lsai[i] / rb[i];
            let dry_share = (1.0 - fwet[i]) * delta[i];
            let leaf_sum = laisun[i] / (rb[i] + rssun[i]) + laisha[i] / (rb[i] + rssha[i]);
            cfw[i] = dry_share.mul_add(leaf_sum, lsai[i] * (1.0 - dry_share) / rb[i]);
        }

        // `:1304-1359`
        let mut cah = [0.0; LAYERS];
        let mut caw = [0.0; LAYERS];
        cgh = [0.0; LAYERS];
        cgw = [0.0; LAYERS];
        for l in 0..LAYERS {
            if !active(l) {
                continue;
            }
            if l == top {
                cah[l] = 1.0 / rah;
                caw[l] = 1.0 / raw;
            } else {
                cah[l] = 1.0 / rd[l + 1];
                caw[l] = 1.0 / rd[l + 1];
            }
            cgh[l] = 1.0 / rd[l];
            cgw[l] = if l == bot {
                if qg < qaf[bot] {
                    1.0 / rd[l]
                } else if leaf.options.soil_resistance_is_conductance {
                    rss / rd[l]
                } else {
                    1.0 / (rd[l] + rss)
                }
            } else {
                1.0 / rd[l]
            };
        }
        let mut wtshi = [0.0; LAYERS];
        let mut wtsqi = [0.0; LAYERS];
        for l in 0..LAYERS {
            wtshi[l] = cah[l] + cgh[l];
            wtsqi[l] = caw[l] + cgw[l];
        }
        for i in 0..n {
            if vegetated[i] {
                let l = canlay[i] - 1;
                wtshi[l] = fcover[i].mul_add(cfh[i], wtshi[l]);
                wtsqi[l] = fcover[i].mul_add(cfw[i], wtsqi[l]);
            }
        }
        for l in 0..LAYERS {
            if active(l) {
                wtshi[l] = 1.0 / wtshi[l];
                wtsqi[l] = 1.0 / wtsqi[l];
            }
        }
        for l in 0..LAYERS {
            wah[l] = wtshi[l] * cah[l];
            wgh[l] = wtshi[l] * cgh[l];
            waq[l] = wtsqi[l] * caw[l];
            wgq[l] = wtsqi[l] * cgw[l];
        }
        wlhl = [0.0; LAYERS];
        wlql = [0.0; LAYERS];
        for i in 0..n {
            if vegetated[i] {
                let l = canlay[i] - 1;
                wlh[i] = fcover[i] * (cfh[i] * wtshi[l]);
                wlhl[l] = wlh[i].mul_add(tl[i], wlhl[l]);
                wlq[i] = fcover[i] * (cfw[i] * wtsqi[l]);
                let q = sat[i].expect("vegetated").specific_humidity;
                wlql[l] = wlq[i].mul_add(q, wlql[l]);
            }
        }
        solve_canopy_air(
            numlay, top, bot, thm, tg, qm, qg, &wah, &wgh, &waq, &wgq, &wlhl, &wlql, &mut taf,
            &mut qaf, &mut fact, &mut facq,
        );

        // `:1429-1495`：层间长波。
        let mut emitted = [0.0; LAYERS];
        for i in 0..n {
            if vegetated[i] {
                let l = canlay[i] - 1;
                emitted[l] = (fshade[i] * (1.0 - thermk[i]) * STEFAN_BOLTZMANN)
                    .mul_add(fourth(tl[i]), emitted[l]);
            }
        }
        let (l1, l2, l3) = (emitted[0], emitted[1], emitted[2]);
        let ltd3 = f3 * tk3 * frl;
        let ld3 = ltd3 + l3;
        let ltd2 = tk2 * tdn[4][2].mul_add(frl, f2 * ld3);
        let ld2 = ltd2 + l2;
        let ltd1 = tk1 * f1.mul_add(ld2, tdn[4][1].mul_add(frl, tdn[3][1] * ld3));
        let ld = [0.0, ltd1 + l1, ld2, ld3, frl];
        lin = [0.0; 5];
        for j in 0..5 {
            for k in 0..5 {
                lin[j] = ld[k].mul_add(tdn[k][j], lin[j]);
            }
        }
        let one_minus_emg = 1.0 - emg;
        let base = one_minus_emg * lin[0];
        let lg = if leaf.options.split_soil_snow {
            let soil = ((1.0 - fsno) * emg * STEFAN_BOLTZMANN)
                .mul_add(fourth(leaf.soil_surface_temperature_k), base);
            (fsno * emg * STEFAN_BOLTZMANN).mul_add(fourth(leaf.snow_surface_temperature_k), soil)
        } else {
            fourth(tg).mul_add(emg * STEFAN_BOLTZMANN, base)
        };
        let ltu1 = f1tk1 * lg;
        let lu1 = ltu1 + l1;
        let ltu2 = tk2 * tup[0][2].mul_add(lg, f2 * lu1);
        let lu2 = l2 + ltu2;
        let ltu3 = tk3 * f3.mul_add(lu2, tup[0][3].mul_add(lg, tup[1][3] * lu1));
        let lu = [lg, lu1, lu2, l3 + ltu3, 0.0];
        let mut upward = [0.0; 5];
        for j in 0..5 {
            for k in 0..5 {
                upward[j] = lu[k].mul_add(tup[k][j], upward[j]);
            }
        }
        for j in 0..5 {
            lin[j] += upward[j];
        }
        for i in 0..n {
            irab[i] = 0.0;
            dirab[i] = 0.0;
            if fshade[i] > 0.0 && canlay[i] > 0 {
                let l = canlay[i] - 1;
                let gap = 1.0 - thermk[i];
                let absorbed = fshade[i] / fshade_lay[l] * gap * lin[l + 1] / fcover[i];
                let emission =
                    fourth(tl[i]) * (gap * (fshade[i] * 2.0) * STEFAN_BOLTZMANN) / fcover[i];
                irab[i] = absorbed - emission;
                let factor = fshade[i] * (dlvpar[l] * 4.0 * one_minus_emg);
                dirab[i] = cube(tl[i])
                    * (gap * (fshade[i] * factor.mul_add(gap, -8.0)) * STEFAN_BOLTZMANN)
                    / fcover[i];
            }
        }

        // `:1502-1648`：逐 PFT 的能量平衡与叶温增量。
        for i in 0..n {
            if !vegetated[i] {
                continue;
            }
            let l = canlay[i] - 1;
            let saturation = sat[i].expect("vegetated");
            let heat = rhoair * AIR_HEAT_CAPACITY_J_KG_K * cfh[i];
            fluxes[i].fsenl = heat * (tl[i] - taf[l]);
            let coupled = |w: f64, a: f64, b: f64, factor: f64| (1.0 - a * b * w / factor) - w;
            fsenl_dtl[i] = if numlay < 3 || l == 1 {
                heat * (1.0 - wlh[i] / fact)
            } else if l == 0 {
                heat * coupled(wlh[i], wgh[1], wah[0], fact)
            } else {
                heat * coupled(wlh[i], wah[1], wgh[2], fact)
            };
            let leaf_sum = laisun[i] / (rb[i] + rssun[i]) + laisha[i] / (rb[i] + rssha[i]);
            let transpiration_weight = rhoair * (1.0 - fwet[i]) * delta[i] * leaf_sum;
            let gradient = saturation.specific_humidity - qaf[l];
            fluxes[i].etr = transpiration_weight * gradient;
            let slope = saturation.specific_humidity_temperature_slope_k;
            let moisture_factor = if numlay < 3 || l == 1 {
                1.0 - wlq[i] / facq
            } else if l == 0 {
                coupled(wlq[i], wgq[1], waq[0], facq)
            } else {
                coupled(wlq[i], waq[1], wgq[2], facq)
            };
            etr_dtl[i] = transpiration_weight * moisture_factor * slope;
            if plant_hydraulics.is_none() && fluxes[i].etr >= 0.0 {
                // `:1556`：`etrc_p` 在 PC 分支里被 `THERMAL` 的本地修补置 0（见 `pft::pc_records`）。
                fluxes[i].etr = 0.0;
                etr_dtl[i] = 0.0;
            }
            // `:1562-1580`：`FNMA(1-fwet, delta, 1)`
            let wet_weight = rhoair * (-(1.0 - fwet[i])).mul_add(delta[i], 1.0) * lsai[i] / rb[i];
            evplwet[i] = gradient * wet_weight;
            evplwet_dtl[i] = moisture_factor * wet_weight * slope;
            let ldew = columns[i].leaf.canopy_water.total_mm;
            if evplwet[i] >= ldew / deltim {
                evplwet[i] = ldew / deltim;
                evplwet_dtl[i] = 0.0;
            }
            let mut fevpl = fluxes[i].etr + evplwet[i];
            fevpl_dtl[i] = etr_dtl[i] + evplwet_dtl[i];
            erre[i] = 0.0;
            fevpl_noadj[i] = fevpl;
            if fevpl * fevpl_bef[i] < 0.0 {
                erre[i] = -(fevpl * 0.9);
                fevpl *= 0.1;
            }
            fluxes[i].fevpl = fevpl;
            // `:1605-1609`
            let rain_heat = drive[i].retained_rain_kg_m2_s * WATER_HEAT_CAPACITY_J_KG_K;
            let snow_heat = drive[i].retained_snow_kg_m2_s * ICE_HEAT_CAPACITY_J_KG_K;
            let dt_precip = t_precip - tl[i];
            let numerator = dt_precip.mul_add(
                snow_heat,
                rain_heat.mul_add(
                    dt_precip,
                    (-fevpl).mul_add(
                        LATENT_HEAT_VAPORIZATION_J_KG,
                        drive[i].absorbed_solar_w_m2 + irab[i] - fluxes[i].fsenl,
                    ),
                ),
            );
            let denominator = snow_heat
                + (rain_heat
                    + ((clai[i] / deltim - dirab[i] + fsenl_dtl[i])
                        + fevpl_dtl[i] * LATENT_HEAT_VAPORIZATION_J_KG));
            let mut step = numerator / denominator;
            dtl_noadj[i] = step;
            if step.abs() > MAX_TEMPERATURE_STEP_K {
                step = step * MAX_TEMPERATURE_STEP_K / step.abs();
            }
            if it >= 2 && dtl[i][it - 1] * step <= 0.0 {
                step = (dtl[i][it - 1] + step) * 0.5;
            }
            dtl[i][it] = step;
            tl[i] = tlbef[i] + step;
            del[i] = (step * step).sqrt();
            let latent = LATENT_HEAT_VAPORIZATION_J_KG * fevpl_dtl[i];
            dele[i] = ((step * step)
                * latent.mul_add(
                    latent,
                    dirab[i].mul_add(dirab[i], fsenl_dtl[i] * fsenl_dtl[i]),
                ))
            .sqrt();
            sat[i] = Some(saturation_specific_humidity(tl[i], psrf)?);
        }

        // `:1654-1707`
        wlhl = [0.0; LAYERS];
        wlql = [0.0; LAYERS];
        for i in 0..n {
            if vegetated[i] {
                let l = canlay[i] - 1;
                wlhl[l] = wlh[i].mul_add(tl[i], wlhl[l]);
                wlql[l] = wlq[i].mul_add(sat[i].expect("vegetated").specific_humidity, wlql[l]);
            }
        }
        solve_canopy_air(
            numlay, top, bot, thm, tg, qm, qg, &wah, &wgh, &waq, &wgq, &wlhl, &wlql, &mut taf,
            &mut qaf, &mut fact, &mut facq,
        );

        // `:1714-1722`
        let gah2o = 1.0 / raw * tprcor / thm;
        let net_uptake = (0..n).fold(0.0, |sum, i| {
            fcover[i].mul_add(
                fluxes[i].assimsun + fluxes[i].assimsha - respcsun[i] - respcsha[i] - rsoil,
                sum,
            )
        });
        pco2a = (-net_uptake).mul_add(psrf * 1.37 / gah2o.max(0.446), leaf.atmospheric_co2_pa);

        // `:1728-1756`
        dth = thm - taf[top];
        dqh = qm - qaf[top];
        tstar = dth * (VON_KARMAN / (fh - fht));
        qstar = dqh * (VON_KARMAN / (fq - fqt));
        let thvstar = one_plus_061_qm.mul_add(tstar, th061 * qstar);
        zeta = zldis * VON_KARMAN * GRAVITY_M_S2 * thvstar / (thv * (ustar * ustar));
        zeta = if zeta >= 0.0 {
            zeta.clamp(1.0e-6, 2.0)
        } else {
            zeta.clamp(-100.0, -1.0e-6)
        };
        obu = zldis / zeta;
        um = if zeta >= 0.0 {
            ur.max(0.1)
        } else {
            let zii = match leaf.options.surface_layer_scheme {
                SurfaceLayerScheme::Standard => 1000.0,
                SurfaceLayerScheme::LargeEddy => (hu * 5.0).max(
                    leaf.boundary_layer_height_m
                        .ok_or_else(|| anyhow::anyhow!("DEF_USE_CBL_HEIGHT needs forc_hpbl"))?,
                ),
            };
            let wc = crate::LibmPow::lpow(-(ustar * GRAVITY_M_S2 * thvstar * zii / thv), 1.0 / 3.0);
            ur.mul_add(ur, wc * wc).sqrt()
        };
        if obuold * obu < 0.0 {
            nmozsgn += 1;
        }
        if nmozsgn >= 4 {
            obu = zldis / -0.01;
        }
        obuold = obu;

        it += 1;
        if it > MIN_ITERATIONS {
            fevpl_bef = fluxes.iter().map(|f| f.fevpl).collect();
            let det = (0..n).map(|i| del[i].max(del2[i])).fold(f64::NAN, f64::max);
            let dee = (0..n)
                .map(|i| dele[i].max(dele2[i]))
                .fold(f64::NAN, f64::max);
            if det < TEMPERATURE_TOLERANCE_K && dee < FLUX_TOLERANCE_W_M2 {
                break;
            }
        }
    }
    let last = it - 1;

    // `:1782-1789`
    for i in 0..n {
        if fcover[i] > 0.0 && lai[i] > 0.001 {
            fluxes[i].gssun = laisun[i] / rssun[i] * (tprcor / tlbef[i]);
            fluxes[i].gssha = laisha[i] / rssha[i] * (tprcor / tlbef[i]);
        } else {
            fluxes[i].gssun = 0.0;
            fluxes[i].gssha = 0.0;
        }
    }
    let rib = (zeta * (ustar * ustar) / (0.160_000_000_000_000_03 / fh * (um * um))).min(5.0);

    // `:1819-2056`
    for i in 0..n {
        if !vegetated[i] {
            continue;
        }
        if lai[i] > 0.001 {
            fluxes[i].rst = 1.0 / (laisun[i] / rssun[i] + laisha[i] / rssha[i]);
        } else {
            fluxes[i].assimsun = 0.0;
            fluxes[i].assimsha = 0.0;
            respcsun[i] = 0.0;
            respcsha[i] = 0.0;
            fluxes[i].rst = 2.0e4;
        }
        fluxes[i].assim = fluxes[i].assimsun + fluxes[i].assimsha;
        fluxes[i].respc = respcsun[i] + respcsha[i] + rsoil;
        let d = dtl[i][last];
        let rain_heat = drive[i].retained_rain_kg_m2_s * WATER_HEAT_CAPACITY_J_KG_K;
        let snow_heat = drive[i].retained_snow_kg_m2_s * ICE_HEAT_CAPACITY_J_KG_K;
        let bracket = (fevpl_dtl[i].mul_add(
            LATENT_HEAT_VAPORIZATION_J_KG,
            fsenl_dtl[i] + (clai[i] / deltim - dirab[i]),
        ) + rain_heat)
            + snow_heat;
        let fsenl = (dtl_noadj[i] - d).mul_add(bracket, fsenl_dtl[i].mul_add(d, fluxes[i].fsenl));
        let mut fsenl = erre[i].mul_add(LATENT_HEAT_VAPORIZATION_J_KG, fsenl);
        let etr0 = fluxes[i].etr;
        fluxes[i].etr = d.mul_add(etr_dtl[i], fluxes[i].etr);
        if let Some(hydraulic) = plant_hydraulics {
            let etr = fluxes[i].etr;
            if etr0.abs() >= 1.0e-15 {
                for flux in &mut fluxes[i].rootflux {
                    *flux = *flux * etr / etr0;
                }
            } else {
                let total = hydraulic
                    .layer_thickness_m
                    .iter()
                    .fold(0.0, |sum, value| value + sum);
                for (flux, dz) in fluxes[i]
                    .rootflux
                    .iter_mut()
                    .zip(hydraulic.layer_thickness_m)
                {
                    *flux = (dz / total * etr_dtl[i]).mul_add(d, *flux);
                }
            }
            let positive = fluxes[i]
                .rootflux
                .iter()
                .filter(|value| **value > 0.0)
                .fold(0.0, |sum, value| value + sum);
            if positive.abs() > 0.0 {
                let scale = etr / positive;
                for flux in &mut fluxes[i].rootflux {
                    *flux = scale * flux.max(0.0);
                }
            } else {
                for (flux, fraction) in fluxes[i]
                    .rootflux
                    .iter_mut()
                    .zip(&parameters[i].root_fraction)
                {
                    *flux = etr * fraction;
                }
            }
        }
        evplwet[i] = d.mul_add(evplwet_dtl[i], evplwet[i]);
        let mut fevpl = d.mul_add(fevpl_dtl[i], fevpl_noadj[i]);
        // `:1864-1871`：负蒸腾记作湿叶结露，不是倒流的液流。
        if fluxes[i].etr < 0.0 {
            evplwet[i] += fluxes[i].etr;
            fluxes[i].etr = 0.0;
            fluxes[i].etrsun = 0.0;
            fluxes[i].etrsha = 0.0;
            if plant_hydraulics.is_some() {
                fluxes[i].rootflux.fill(0.0);
            }
        }
        let ldew = columns[i].leaf.canopy_water.total_mm;
        let elwmax = ldew / deltim;
        let elwdif = (evplwet[i] - elwmax).max(0.0);
        evplwet[i] = elwmax.min(evplwet[i]);
        fevpl -= elwdif;
        fsenl = elwdif.mul_add(LATENT_HEAT_VAPORIZATION_J_KG, fsenl);
        fluxes[i].fsenl = fsenl;
        fluxes[i].fevpl = fevpl;
        let dt_precip = t_precip - tl[i];
        fluxes[i].hprl = rain_heat.mul_add(dt_precip, snow_heat * dt_precip);
        fluxes[i].dheatl = d * (clai[i] / deltim);
        let (melt, freeze) = update_pc_canopy_water(
            &mut columns[i].leaf.canopy_water,
            &mut columns[i].wet_snow_fraction,
            &mut tl[i],
            evplwet[i],
            lsai[i],
            deltim,
            vegetation_snow,
        );
        fluxes[i].canopy_melt_mass_mm = melt;
        fluxes[i].canopy_freeze_mass_mm = freeze;
    }

    // `:2061-2065`
    let emission_change =
        |i: usize| cube(tlbef[i]) * (fshade[i] * 4.0 * (1.0 - thermk[i]) * STEFAN_BOLTZMANN);
    let canopy_down = (0..n).fold(0.0, |sum, i| emission_change(i).mul_add(dtl[i][last], sum));
    let dlrad = canopy_down + lin[0];
    let absorbed_change = (0..n).fold(0.0, |sum, i| {
        (fcover[i] * dirab[i]).mul_add(dtl[i][last], sum)
    });
    let ulrad = (-emg).mul_add(canopy_down, lin[4] - absorbed_change);

    // `:2071-2131`
    let taux = -(us * rhoair / ram);
    let tauy = -(vs * rhoair / ram);
    let (ttaf, tqaf) = match numlay {
        1 => (thm, qm),
        2 => (taf[top], qaf[top]),
        _ => (taf[1], qaf[1]),
    };
    let heat_ground = rhoair * AIR_HEAT_CAPACITY_J_KG_K * cgh[bot];
    let air_part = wah[bot] * ttaf;
    let ground_share = 1.0 - wgh[bot];
    let fseng = heat_ground * (tg - taf[bot]);
    let fseng_soil = heat_ground
        * (ground_share.mul_add(leaf.soil_surface_temperature_k, -air_part) - wlhl[bot]);
    let fseng_snow = heat_ground
        * (ground_share.mul_add(leaf.snow_surface_temperature_k, -air_part) - wlhl[bot]);
    let water_ground = rhoair * cgw[bot];
    let moisture_part = waq[bot] * tqaf;
    let ground_moisture_share = 1.0 - wgq[bot];
    let fevpg = water_ground * (qg - qaf[bot]);
    let fevpg_soil = water_ground
        * (ground_moisture_share.mul_add(leaf.soil_specific_humidity, -moisture_part) - wlql[bot]);
    let fevpg_snow = water_ground
        * (ground_moisture_share.mul_add(leaf.snow_specific_humidity, -moisture_part) - wlql[bot]);
    let dqgdt = leaf.ground_humidity_temperature_slope_k;
    let (cgrnds, cgrndl) = if numlay < 3 {
        (
            heat_ground * (1.0 - wgh[bot] / fact),
            water_ground * (1.0 - wgq[bot] / facq) * dqgdt,
        )
    } else {
        (
            heat_ground * ((1.0 - wah[0] * wgh[1] * wgh[0] / fact) - wgh[0]),
            water_ground * ((1.0 - waq[0] * wgq[1] * wgq[0] / facq) - wgq[0]) * dqgdt,
        )
    };
    let cgrnd = cgrndl.mul_add(leaf.ground_latent_heat_j_kg, cgrnds);
    let tref = (VON_KARMAN / (fh - fht) * dth).mul_add(fh2m / VON_KARMAN - fh / VON_KARMAN, thm);
    let qref = (VON_KARMAN / (fq - fqt) * dqh).mul_add(fq2m / VON_KARMAN - fq / VON_KARMAN, qm);

    for i in 0..n {
        columns[i].leaf.leaf_temperature_k = tl[i];
    }
    Ok((
        PcPatchFlux {
            taux,
            tauy,
            fseng,
            fseng_soil,
            fseng_snow,
            fevpg,
            fevpg_soil,
            fevpg_snow,
            cgrnd,
            cgrndl,
            cgrnds,
            tref,
            qref,
            dlrad,
            ulrad,
            z0m: z0mv,
            zol: zeta,
            rib,
            ustar,
            qstar,
            tstar,
            fm,
            fh,
            fq,
        },
        fluxes,
    ))
}

/// `:1378-1420` 与 `:1665-1707`：解各层冠层空气温湿度（两处同式）。
#[allow(clippy::too_many_arguments)]
fn solve_canopy_air(
    numlay: usize,
    top: usize,
    bot: usize,
    thm: f64,
    tg: f64,
    qm: f64,
    qg: f64,
    wah: &[f64; LAYERS],
    wgh: &[f64; LAYERS],
    waq: &[f64; LAYERS],
    wgq: &[f64; LAYERS],
    wlhl: &[f64; LAYERS],
    wlql: &[f64; LAYERS],
    taf: &mut [f64; LAYERS],
    qaf: &mut [f64; LAYERS],
    fact: &mut f64,
    facq: &mut f64,
) {
    match numlay {
        1 => {
            taf[top] = wah[top].mul_add(thm, wgh[top] * tg) + wlhl[top];
            qaf[top] = waq[top].mul_add(qm, wgq[top] * qg) + wlql[top];
            *fact = 1.0;
            *facq = 1.0;
        }
        2 => {
            let tmpw1 = wgh[bot].mul_add(tg, wlhl[bot]);
            *fact = (-wgh[top]).mul_add(wah[bot], 1.0);
            taf[top] = (wah[top].mul_add(thm, wgh[top] * tmpw1) + wlhl[top]) / *fact;
            let tmpw1 = wgq[bot].mul_add(qg, wlql[bot]);
            *facq = (-wgq[top]).mul_add(waq[bot], 1.0);
            qaf[top] = (waq[top].mul_add(qm, wgq[top] * tmpw1) + wlql[top]) / *facq;
            taf[bot] = wlhl[bot] + wgh[bot].mul_add(tg, wah[bot] * taf[top]);
            qaf[bot] = wlql[bot] + wgq[bot].mul_add(qg, waq[bot] * qaf[top]);
        }
        _ => {
            let tmpw1 = wah[2].mul_add(thm, wlhl[2]);
            let tmpw2 = wgh[0].mul_add(tg, wlhl[0]);
            *fact = (-wgh[1]).mul_add(wah[0], (-wah[1]).mul_add(wgh[2], 1.0));
            taf[1] = (tmpw1.mul_add(wah[1], tmpw2 * wgh[1]) + wlhl[1]) / *fact;
            let tmpw1 = waq[2].mul_add(qm, wlql[2]);
            let tmpw2 = wgq[0].mul_add(qg, wlql[0]);
            *facq = (-waq[0]).mul_add(wgq[1], (-waq[1]).mul_add(wgq[2], 1.0));
            qaf[1] = (waq[1].mul_add(tmpw1, tmpw2 * wgq[1]) + wlql[1]) / *facq;
            taf[0] = wlhl[0] + wgh[0].mul_add(tg, taf[1] * wah[0]);
            qaf[0] = wlql[0] + wgq[0].mul_add(qg, qaf[1] * waq[0]);
            taf[2] = wlhl[2] + wah[2].mul_add(thm, taf[1] * wgh[2]);
            qaf[2] = wlql[2] + waq[2].mul_add(qm, qaf[1] * wgq[2]);
        }
    }
}

/// `:1901-2038`（`DEF_Interception_scheme = 1` 与 `DEF_VEG_SNOW`）：叶面蒸发/凝结记账、
/// 湿雪比例与冠层融化/冻结（Niu 2004 的叶温拉回）。返回冠层融化、冻结的质量
/// （`qmelt*deltim`、`qfrz*deltim`，示踪物用；没有相变时为 0）。
fn update_pc_canopy_water(
    water: &mut CanopyWater,
    wet_snow_fraction: &mut f64,
    tl: &mut f64,
    evplwet: f64,
    lsai: f64,
    deltim: f64,
    vegetation_snow: bool,
) -> (f64, f64) {
    // `:1903`：`max(FNMA(deltim, evplwet, ldew), 0)`
    water.total_mm = (-deltim).mul_add(evplwet, water.total_mm).max(0.0);
    if !vegetation_snow {
        // `:1931-1941`：不分冠层雪时也让雨/雪两分量与总量一致。
        let parts = water.rain_mm + water.snow_mm;
        if parts > 1.0e-10 {
            water.rain_mm = water.total_mm * (water.rain_mm / parts);
            water.snow_mm = water.total_mm - water.rain_mm;
        } else if *tl > FREEZING_K {
            water.rain_mm = water.total_mm;
            water.snow_mm = 0.0;
        } else {
            water.rain_mm = 0.0;
            water.snow_mm = water.total_mm;
        }
        return (0.0, 0.0);
    }
    let (mut qevpl, qdewl, mut qsubl, qfrol);
    if *tl > FREEZING_K {
        qevpl = evplwet.max(0.0);
        qdewl = evplwet.min(0.0).abs();
        qsubl = 0.0;
        qfrol = 0.0;
        if qevpl > water.rain_mm / deltim {
            qsubl = qevpl - water.rain_mm / deltim;
            qevpl = water.rain_mm / deltim;
        }
    } else {
        qevpl = 0.0;
        qdewl = 0.0;
        qsubl = evplwet.max(0.0);
        qfrol = evplwet.min(0.0).abs();
        if qsubl > water.snow_mm / deltim {
            qevpl = qsubl - water.snow_mm / deltim;
            qsubl = water.snow_mm / deltim;
        }
    }
    water.rain_mm = deltim.mul_add(qdewl - qevpl, water.rain_mm);
    water.snow_mm = deltim.mul_add(qfrol - qsubl, water.snow_mm);
    water.total_mm = water.rain_mm + water.snow_mm;

    // `:2005-2038`
    *wet_snow_fraction = 0.0;
    if water.snow_mm > 0.0 {
        let fraction = crate::LibmPow::lpow(water.snow_mm * (10.0 / (lsai * 48.0)), 0.666666666666);
        *wet_snow_fraction = fraction.min(1.0);
    }
    let fwet_snow = *wet_snow_fraction;
    let (mut melt_mass, mut freeze_mass) = (0.0, 0.0);
    if water.snow_mm > 1.0e-6 && *tl > FREEZING_K {
        let qmelt = (water.snow_mm / deltim).min(
            water.snow_mm * ((*tl - FREEZING_K) * ICE_HEAT_CAPACITY_J_KG_K)
                / (deltim * LATENT_HEAT_FUSION_J_KG),
        );
        let melted = deltim * qmelt;
        melt_mass = melted;
        water.snow_mm = (water.snow_mm - melted).max(0.0);
        water.rain_mm = (melted + water.rain_mm).max(0.0);
        *tl = fwet_snow.mul_add(FREEZING_K, *tl * (1.0 - fwet_snow));
    }
    if water.rain_mm > 1.0e-6 && *tl < FREEZING_K {
        let qfrz = (water.rain_mm / deltim).min(
            (FREEZING_K - *tl) * WATER_HEAT_CAPACITY_J_KG_K * water.rain_mm
                / (deltim * LATENT_HEAT_FUSION_J_KG),
        );
        let frozen = deltim * qfrz;
        freeze_mass = frozen;
        water.rain_mm = (water.rain_mm - frozen).max(0.0);
        water.snow_mm = (frozen + water.snow_mm).max(0.0);
        *tl = fwet_snow.mul_add(FREEZING_K, *tl * (1.0 - fwet_snow));
    }
    (melt_mass, freeze_mass)
}

#[cfg(test)]
#[path = "leaf_temperature_pc_tests.rs"]
mod leaf_temperature_pc_tests;
