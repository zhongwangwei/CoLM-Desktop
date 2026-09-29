//! `MOD_Lake:laketem`（非 SNICAR）：湖面湍流通量迭代 → 雪 + 湖 + 湖底土壤的
//! 联合热传导 → 相变 → 对流混合 → 能量闭合修正。
//!
//! 收缩形状逐句对照 `MOD_Lake.F90` 的 `-fdump-tree-optimized-lineno`
//! （`-O2 -ffp-contract=fast`）；`hConductivity_lake` 被内联进 `laketem`，
//! 它的形状在 [`crate::lake_thermal_conductivity`] 里单独对过。

use crate::LibmPow;
use anyhow::{ensure, Result};

use crate::lake::lake_water_density;
use crate::{
    initialize_monin_obukhov, lake_roughness, lake_thermal_conductivity, monin_obukhov_with_scheme,
    saturation_specific_humidity, soil_thermal_properties, GlacierColumn, LakeColumn,
    LakeConductivityInput, LakeRoughness, LakeRoughnessInput, MoninObukhovInitialInput,
    MoninObukhovInput, ShortwaveForcing, SoilThermalInput, SurfaceLayerScheme,
    ThermalConductivityScheme,
};

const HVAP: f64 = 2.5104e6;
const HSUB: f64 = 2.8440e6;
const HFUS: f64 = 0.3336e6;
const STEFNC: f64 = 5.67e-8;
const TFRZ: f64 = 273.16;
const CPLIQ: f64 = 4188.0;
const CPICE: f64 = 2117.27;
const CPAIR: f64 = 1004.64;
const RGAS: f64 = 287.04;
const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;
const TKICE: f64 = 2.290;
const TKAIR: f64 = 0.023;
const VONKAR: f64 = 0.4;
const GRAV: f64 = 9.80616;
/// `laketem` 里写死的 `emg = 0.97`。
pub const LAKE_EMISSIVITY: f64 = 0.97;
const CWAT: f64 = CPLIQ * DENH2O;
const CICE_EFF: f64 = CPICE * DENH2O;
const CFUS: f64 = HFUS * DENH2O;
/// `depthcrit`：深湖把涡扩散乘 5 的门槛。
const DEEP_LAKE_DEPTH_M: f64 = 25.0;
/// Crank-Nicolson 的 `cnfac`（`laketem` 自带，不走 namelist）。
const CNFAC: f64 = 0.5;

/// `laketem` 的标量输入。
#[derive(Debug, Clone, Copy)]
pub struct LakeTemperatureInput<'a> {
    pub time_step_seconds: f64,
    /// `dlat`（`patchlatr`，弧度）。
    pub latitude_radians: f64,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub air_density_kg_m3: f64,
    pub surface_pressure_pa: f64,
    /// `forc_sols/soll/solsd/solld`：只决定 `betaprime`。
    pub shortwave: ShortwaveForcing,
    /// `sabg`
    pub absorbed_shortwave_w_m2: f64,
    pub downward_longwave_w_m2: f64,
    pub boundary_layer_height_m: Option<f64>,
    pub surface_layer_scheme: SurfaceLayerScheme,
    /// `lakedepth`：与会随动态湖改变的 `dz_lake` 分开保存。
    pub lake_depth_m: f64,
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    /// 湖底土层的 `soil_hcap_cond` 静态参数；温度与体积含水率由本函数按当前列填。
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    pub snow_layers: usize,
    /// `DEF_USE_SNICAR`：`netsolar` 算出的 `sabg_snow_lyr(-4:1)`（五个雪槽加湖顶那一格）。
    /// 打开时 `betaprime` 不再因积雪取 1，湖层吸收用 `sabg_snow_lyr(1)`，雪层逐层吸收。
    pub snow_layer_absorption_w_m2: Option<[f64; 6]>,
}

/// `laketem` 在步内改写的湖量。
#[derive(Debug)]
pub struct LakeTemperatureState<'a> {
    pub lake: &'a mut LakeColumn,
    /// `savedtke1`
    pub saved_tke: &'a mut f64,
    /// `t_grnd`：迭代的初值，也是输出。
    pub ground_temperature_k: &'a mut f64,
    /// `scv`/`snowdp`：无雪层时湖面薄雪的融化会改它们。
    pub snow_water_equivalent_kg_m2: &'a mut f64,
    pub snow_depth_m: &'a mut f64,
    /// 雪层在前、湖底土层在后的打包列（上游下标 `lb..nl_soil`）。
    pub column: &'a mut GlacierColumn,
}

/// `laketem` 的通量输出（上游的同名变量）。
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LakeThermalFluxes {
    pub taux: f64,
    pub tauy: f64,
    pub fsena: f64,
    pub fevpa: f64,
    pub lfevpa: f64,
    pub fseng: f64,
    pub fevpg: f64,
    pub qseva: f64,
    pub qsubl: f64,
    pub qsdew: f64,
    pub qfros: f64,
    pub olrg: f64,
    pub fgrnd: f64,
    pub tref: f64,
    pub qref: f64,
    pub trad: f64,
    pub emis: f64,
    pub z0m: f64,
    pub zol: f64,
    pub rib: f64,
    pub ustar: f64,
    pub qstar: f64,
    pub tstar: f64,
    pub fm: f64,
    pub fh: f64,
    pub fq: f64,
    /// `sm`：雪的融化速率（湖面薄雪 + 雪层）。
    pub sm: f64,
}

/// `laketem` 的结果：通量与雪/土层的 `imelt`（打包列顺序）。
#[derive(Debug, Clone, PartialEq)]
pub struct LakeTemperatureOutput {
    pub fluxes: LakeThermalFluxes,
    pub phase_flag: Vec<i32>,
}

/// Port of `MOD_Lake:laketem`（含 `DEF_USE_SNICAR` 的分层吸收分支）。
pub fn lake_temperature(
    input: LakeTemperatureInput<'_>,
    state: LakeTemperatureState<'_>,
) -> Result<LakeTemperatureOutput> {
    let LakeTemperatureState {
        lake,
        saved_tke,
        ground_temperature_k,
        snow_water_equivalent_kg_m2: scv,
        snow_depth_m: snowdp,
        column,
    } = state;
    let n_lake = lake.temperature_k.len();
    let n = column.temperature_k.len();
    let snow = input.snow_layers;
    ensure!(
        n_lake >= 2
            && lake.thickness_m.len() == n_lake
            && lake.ice_fraction.len() == n_lake
            && snow < n
            && column.thickness_m.len() == n
            && column.node_depth_m.len() == n
            && column.interface_depth_m.len() == n + 1
            && column.liquid_water_kg_m2.len() == n
            && column.ice_water_kg_m2.len() == n
            && input.soil_thermal_inputs.len() == n - snow,
        "the lake column is inconsistent"
    );
    let dt = input.time_step_seconds;
    let dz = lake.thickness_m.clone();
    let forc_t = input.air_temperature_k;
    let forc_q = input.specific_humidity;
    let rho = input.air_density_kg_m3;
    let frl = input.downward_longwave_w_m2;
    let sabg = input.absorbed_shortwave_w_m2;

    // ---- 湖层节点深度与风区（`:684-699`）----
    let mut z_lake = vec![0.0; n_lake];
    z_lake[0] = dz[0] * 0.5;
    for j in 1..n_lake {
        // `:688` `.FMA (dz(j-1)+dz(j), 0.5, z(j-1))`
        z_lake[j] = (dz[j - 1] + dz[j]).mul_add(0.5, z_lake[j - 1]);
    }
    let bottom_node = z_lake[n_lake - 1];
    let (idlak, fetch) = if bottom_node < 4.0 {
        (0, 100.0)
    } else {
        (1, bottom_node * 25.0)
    };
    let za = [0.5, 0.6];

    // `betaprime`（`:706-715`）：近红外占比；`+ (1-b)*betavis` 被收成 `.FMA (1-b, 0, b)`，恒等于 `b`。
    // SNICAR（`:724-732`）：不论有没有雪都按短波算。
    let snicar = input.snow_layer_absorption_w_m2;
    let betaprime = if snow == 0 || snicar.is_some() {
        let sw = input.shortwave;
        (sw.direct_near_infrared_w_m2 + sw.diffuse_near_infrared_w_m2)
            / (((sw.direct_visible_w_m2 + sw.direct_near_infrared_w_m2) + sw.diffuse_visible_w_m2)
                + sw.diffuse_near_infrared_w_m2)
                .max(1.0e-5)
    } else {
        1.0
    };

    let mut t_grnd = *ground_temperature_k;
    let saturation = saturation_specific_humidity(t_grnd, input.surface_pressure_pa)?;
    let mut qsatg = saturation.specific_humidity;
    let mut qsatg_dt = saturation.specific_humidity_temperature_slope_k;
    let mut zii = 1000.0;
    let thm = crate::reference_height_temperature_k(forc_t, input.temperature_height_m);
    let th = forc_t * (100_000.0 / input.surface_pressure_pa).lpow(RGAS / CPAIR);
    let one_plus_061q = forc_q.mul_add(0.61, 1.0);
    let thv = th * one_plus_061q;
    let ur = input
        .eastward_wind_m_s
        .mul_add(
            input.eastward_wind_m_s,
            input.northward_wind_m_s * input.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);
    let th061 = th * 0.61;
    let mut dth = thm - t_grnd;
    let mut dqh = forc_q - qsatg;
    // `:749` `.FMA (1+0.61q, dth, dqh*(th*0.61))`
    let dthv = one_plus_061q.mul_add(dth, dqh * th061);
    let zldis = input.wind_height_m;
    // `:758` 空气运动黏度：`.FNMA (dT^3, 4.84e-9, .FMA (dT^2, 8.301e-6, .FMA (dT, 6.542e-3, 1)))`
    let dtc = forc_t - TFRZ;
    let dtc2 = dtc * dtc;
    let visa = (-(dtc * dtc2)).mul_add(4.84e-9, dtc2.mul_add(8.301e-6, dtc.mul_add(6.542e-3, 1.0)))
        * 1.326e-5;
    // `:761` Charnock 参数
    let fetch_term = -(((fetch * GRAV) / ur / ur).lpow(1.0 / 3.0) / 22.0);
    let depth_term = -((bottom_node * GRAV).lpow(0.5) / ur);
    let cur = fetch_term.max(depth_term).exp().mul_add(0.1, 0.01);
    let mut um = if dthv >= 0.0 {
        ur.max(0.1)
    } else {
        // `wc = 0.5`：`sqrt(.FMA (ur, ur, 0.25))`
        ur.mul_add(ur, 0.25).sqrt()
    };
    let mut ustar = 0.06;
    // 这里的 `z0mg` 只用来收敛 `ustar`：紧接着的 `roughness_lake` 会整个覆盖它。
    for _ in 0..5 {
        let z0mg = ((ustar * 0.013) * ustar) / GRAV + (visa * 0.11) / ustar;
        ustar = (um * VONKAR) / (zldis / z0mg).ln();
    }
    let mut roughness = roughness_at(snow, t_grnd, lake.temperature_k[0], input, cur, ustar)?;
    let initial = initialize_monin_obukhov(MoninObukhovInitialInput {
        reference_wind_m_s: ur,
        potential_temperature_k: th,
        reference_temperature_k: thm,
        virtual_potential_temperature_k: thv,
        temperature_difference_k: dth,
        humidity_difference_kg_kg: dqh,
        virtual_temperature_difference_k: dthv,
        reference_height_m: zldis,
        momentum_roughness_m: roughness.momentum_m,
    })?;
    um = initial.stability_adjusted_wind_m_s;
    let mut obu = initial.obukhov_length_m;
    let dzsur = if snow == 0 {
        dz[0] * 0.5
    } else {
        column.node_depth_m[0] - column.interface_depth_m[0]
    };

    // ---- 表面温度迭代（`:797-893`）----
    let emissive = LAKE_EMISSIVITY * STEFNC;
    let rho_cp = rho * CPAIR;
    let mut iteration = 1;
    let mut converged_steps = 0;
    let mut t_grnd_bef;
    let mut htvp;
    let mut profile;
    let mut ram;
    let mut rah;
    let mut raw;
    let mut fseng;
    let mut fevpg;
    let mut tstar;
    let mut qstar;
    let mut zeta;
    loop {
        t_grnd_bef = t_grnd;
        let tksur;
        let tsur;
        if t_grnd_bef > TFRZ && lake.temperature_k[0] > TFRZ && snow == 0 {
            tksur = *saved_tke;
            tsur = lake.temperature_k[0];
            htvp = HVAP;
        } else if snow == 0 {
            tksur = TKICE;
            tsur = lake.temperature_k[0];
            htvp = HSUB;
        } else {
            let rhosnow =
                (column.ice_water_kg_m2[0] + column.liquid_water_kg_m2[0]) / column.thickness_m[0];
            // `:814` `.FMA (.FMA (rho, 7.75e-5, (rho*1.105e-6)*rho), tkice-tkair, tkair)`
            tksur = rhosnow
                .mul_add(7.75e-5, (rhosnow * 1.105e-6) * rhosnow)
                .mul_add(TKICE - TKAIR, TKAIR);
            tsur = column.temperature_k[0];
            htvp = HSUB;
        }
        profile = monin_obukhov_with_scheme(
            MoninObukhovInput {
                wind_height_m: input.wind_height_m,
                temperature_height_m: input.temperature_height_m,
                humidity_height_m: input.humidity_height_m,
                displacement_height_m: 0.0,
                momentum_roughness_m: roughness.momentum_m,
                heat_roughness_m: roughness.sensible_heat_m,
                moisture_roughness_m: roughness.latent_heat_m,
                obukhov_length_m: obu,
                stability_adjusted_wind_m_s: um,
                boundary_layer_height_m: input.boundary_layer_height_m,
            },
            input.surface_layer_scheme,
        )?;
        ustar = profile.friction_velocity_m_s;
        ram = 1.0 / ((ustar * ustar) / um);
        rah = 1.0 / ((VONKAR / profile.heat) * ustar);
        raw = 1.0 / ((VONKAR / profile.moisture) * ustar);
        let stftg3 = ((t_grnd_bef * emissive) * t_grnd_bef) * t_grnd_bef;
        let sensible_conductance = rho_cp / rah;
        let latent_conductance = (rho * htvp) / raw;
        // `:838` `ax`：`sabg*b` 起头，`emg*frl`、`3*stftg3*t`、`rho*cp/rah*thm` 依次收进 FMA，
        // 潜热项 `.FNMA`，最后加 `tksur*tsur/dzsur`。
        let ax = (-latent_conductance).mul_add(
            (-qsatg_dt).mul_add(t_grnd_bef, qsatg) - forc_q,
            thm.mul_add(
                sensible_conductance,
                (stftg3 * 3.0).mul_add(t_grnd_bef, frl.mul_add(LAKE_EMISSIVITY, sabg * betaprime)),
            ),
        ) + (tksur * tsur) / dzsur;
        // `:841` `.FMA (rho*htvp/raw, qsatgdT, rho*cp/rah + 4*stftg3) + tksur/dzsur`
        let bx = latent_conductance.mul_add(qsatg_dt, sensible_conductance + stftg3 * 4.0)
            + tksur / dzsur;
        t_grnd = ax / bx;
        fseng = (rho_cp * (t_grnd - thm)) / rah;
        // `:858` `(rho*(.FMA (qsatgdT, t-tbef, qsatg) - q))/raw`
        fevpg = (rho * (qsatg_dt.mul_add(t_grnd - t_grnd_bef, qsatg) - forc_q)) / raw;
        let saturation = saturation_specific_humidity(t_grnd, input.surface_pressure_pa)?;
        qsatg = saturation.specific_humidity;
        qsatg_dt = saturation.specific_humidity_temperature_slope_k;
        dth = thm - t_grnd;
        dqh = forc_q - qsatg;
        tstar = dth * (VONKAR / profile.heat);
        qstar = dqh * (VONKAR / profile.moisture);
        // `:865` `.FMA (1+0.61q, tstar, (th*0.61)*qstar)`
        let thvstar = one_plus_061q.mul_add(tstar, th061 * qstar);
        zeta = (((zldis * VONKAR) * GRAV) * thvstar) / (thv * (ustar * ustar));
        zeta = if zeta >= 0.0 {
            zeta.clamp(1.0e-6, 2.0)
        } else {
            zeta.clamp(-100.0, -1.0e-6)
        };
        obu = zldis / zeta;
        if zeta >= 0.0 {
            um = ur.max(0.1);
        } else {
            if input.surface_layer_scheme == SurfaceLayerScheme::LargeEddy {
                let hpbl = input.boundary_layer_height_m.ok_or_else(|| {
                    anyhow::anyhow!(
                        "the large-eddy surface-layer scheme needs the forcing's boundary-layer height"
                    )
                })?;
                zii = (5.0 * input.wind_height_m).max(hpbl);
            }
            let wc = (-((((ustar * GRAV) * thvstar) * zii) / thv)).lpow(1.0 / 3.0);
            // `:881` `sqrt(.FMA (ur, ur, wc*wc))`（`beta1 = 1`）
            um = ur.mul_add(ur, wc * wc).sqrt();
        }
        roughness = roughness_at(snow, t_grnd, lake.temperature_k[0], input, cur, ustar)?;
        iteration += 1;
        let change = (t_grnd - t_grnd_bef).abs();
        if iteration > 6 {
            if change <= 0.01 {
                converged_steps += 1;
            }
            if converged_steps >= 4 {
                break;
            }
        }
        if iteration > 40 {
            break;
        }
    }

    // ---- 迭代后的表面温度钳制（`:905-925`）----
    let tdmax = TFRZ + 4.0;
    let lake_top = lake.temperature_k[0];
    let clamp_to = if (snow > 0 || lake_top <= TFRZ) && t_grnd > TFRZ {
        Some(TFRZ)
    } else if (lake_top > t_grnd && t_grnd > tdmax)
        || (lake_top < t_grnd && lake_top > TFRZ && t_grnd < tdmax)
    {
        Some(lake_top)
    } else {
        None
    };
    if let Some(value) = clamp_to {
        t_grnd_bef = t_grnd;
        t_grnd = value;
        fseng = (rho_cp * (t_grnd - thm)) / rah;
        fevpg = (rho * (qsatg_dt.mul_add(t_grnd - t_grnd_bef, qsatg) - forc_q)) / raw;
    }
    let stftg3 = ((t_grnd_bef * emissive) * t_grnd_bef) * t_grnd_bef;
    let tb2 = t_grnd_bef * t_grnd_bef;
    // `:925` `.FMA (4*stftg3, t-tbef, .FMA (frl, 1-emg, (tb^2)^2*emg*stefnc))`
    let olrg = (stftg3 * 4.0).mul_add(
        t_grnd - t_grnd_bef,
        frl.mul_add(1.0 - LAKE_EMISSIVITY, (tb2 * tb2) * emissive),
    );
    htvp = if t_grnd > TFRZ { HVAP } else { HSUB };
    let fgrnd1 = (((sabg * betaprime + frl) - olrg) - fseng) - htvp * fevpg;
    // SNICAR（`:936-939`）：雪顶的净热通量只收最上层雪的吸收，`dhsdT = 0`。
    // GIMPLE：`(((lyr(lb) + frl) - olrg) - fseng) - htvp*fevpg`，与 `fgrnd1` 共用 `htvp*fevpg`。
    let hs = snicar.map(|lyr| (((lyr[5 - snow] + frl) - olrg) - fseng) - htvp * fevpg);

    // ---- 热容与导热率（`:949-1015`）----
    let mut cv_lake = (0..n_lake)
        .map(|j| lake_heat_capacity(dz[j], lake.ice_fraction[j]))
        .collect::<Vec<_>>();
    let conductivity = lake_thermal_conductivity(
        LakeConductivityInput {
            snow_layer_count: -(snow as i32),
            ground_temperature_k: t_grnd,
            node_depth_m: &z_lake,
            latitude_radians: input.latitude_radians,
            friction_velocity_m_s: ustar,
            momentum_roughness_m: roughness.momentum_m,
            lake_depth_m: input.lake_depth_m,
            deep_lake_threshold_m: DEEP_LAKE_DEPTH_M,
        },
        lake,
    )?;
    let tk_lake = conductivity.thermal_conductivity_w_m_k;
    *saved_tke = conductivity.top_eddy_conductivity_w_m_k;

    let mut cv = vec![0.0; n];
    let mut thk = vec![0.0; n];
    for (layer, soil) in (snow..n).zip(input.soil_thermal_inputs) {
        let thickness = column.thickness_m[layer];
        let properties = soil_thermal_properties(
            SoilThermalInput {
                temperature_k: column.temperature_k[layer],
                liquid_volume_fraction: column.liquid_water_kg_m2[layer] / (thickness * DENH2O),
                ice_volume_fraction: column.ice_water_kg_m2[layer] / (thickness * DENICE),
                ..*soil
            },
            input.thermal_conductivity_scheme,
        )?;
        cv[layer] = properties.heat_capacity_j_m3_k * thickness;
        thk[layer] = properties.conductivity_w_m_k;
    }
    for layer in 0..snow {
        let liquid = column.liquid_water_kg_m2[layer];
        let ice = column.ice_water_kg_m2[layer];
        // `:993` `.FMA (wliq, cpliq, wice*cpice)`
        cv[layer] = liquid.mul_add(CPLIQ, ice * CPICE);
        let rhosnow = (ice + liquid) / column.thickness_m[layer];
        thk[layer] = rhosnow
            .mul_add(7.75e-5, (rhosnow * 1.105e-6) * rhosnow)
            .mul_add(TKICE - TKAIR, TKAIR);
    }
    let z = &column.node_depth_m;
    let zi = &column.interface_depth_m;
    let mut tk = vec![0.0; n];
    for layer in 0..n - 1 {
        tk[layer] = if snow > 0 && layer == snow - 1 {
            // 上游 `i == 0`：最底一层雪直接取自己的导热率。
            thk[layer]
        } else {
            let below = zi[layer + 1];
            // `:1012` 分母 `.FMA (thk(i), z(i+1)-zi(i), thk(i+1)*(zi(i)-z(i)))`
            ((thk[layer] * thk[layer + 1]) * (z[layer + 1] - z[layer]))
                / thk[layer].mul_add(z[layer + 1] - below, thk[layer + 1] * (below - z[layer]))
        };
    }
    let tk_top_soil = thk[snow];

    // ---- 步首能量（`:1024-1036`）----
    let ocvts = column_energy(lake, &dz, &cv_lake, column, &cv, snow, *scv);

    // ---- 湖层吸收的短波（`:1040-1066`）----
    let mut phi = vec![0.0; n_lake];
    let mut phi_soil = 0.0;
    if let Some(lyr) = snicar {
        // SNICAR（`:1073-1091`）：不看冰雪，一律按消光分到各湖层，入射取 `sabg_snow_lyr(1)`。
        let eta = input.lake_depth_m.max(1.0).lpow(-0.424) * 1.1925;
        let top = lyr[5];
        for j in 0..n_lake {
            let zin = (-dz[j]).mul_add(0.5, z_lake[j]);
            let zout = dz[j].mul_add(0.5, z_lake[j]);
            let rsfin = (-(eta * (zin - za[idlak]).max(0.0))).exp();
            let rsfout = (-(eta * (zout - za[idlak]).max(0.0))).exp();
            phi[j] = ((rsfin - rsfout) * top) * (1.0 - betaprime);
            if j == n_lake - 1 {
                phi_soil = (1.0 - betaprime) * (top * rsfout);
            }
        }
    } else if t_grnd > TFRZ && lake.temperature_k[0] > TFRZ && snow == 0 {
        let eta = input.lake_depth_m.max(1.0).lpow(-0.424) * 1.1925;
        for j in 0..n_lake {
            let zin = (-dz[j]).mul_add(0.5, z_lake[j]);
            let zout = dz[j].mul_add(0.5, z_lake[j]);
            let rsfin = (-(eta * (zin - za[idlak]).max(0.0))).exp();
            let rsfout = (-(eta * (zout - za[idlak]).max(0.0))).exp();
            phi[j] = (sabg * (rsfin - rsfout)) * (1.0 - betaprime);
            if j == n_lake - 1 {
                phi_soil = (1.0 - betaprime) * (sabg * rsfout);
            }
        }
    } else if snow == 0 {
        phi[0] = sabg * (1.0 - betaprime);
    }

    // ---- 雪 + 湖 + 土的联合列（`:1092-1165`）----
    let total = n + n_lake;
    let soil_start = snow + n_lake;
    let mut zx = vec![0.0; total];
    let mut tx = vec![0.0; total];
    let mut cvx = vec![0.0; total];
    let mut phix = vec![0.0; total];
    let lake_bottom = dz[n_lake - 1].mul_add(0.5, z_lake[n_lake - 1]);
    for k in 0..total {
        if k < snow {
            zx[k] = z[k];
            tx[k] = column.temperature_k[k];
            cvx[k] = cv[k];
        } else if k < soil_start {
            let j = k - snow;
            zx[k] = z_lake[j];
            tx[k] = lake.temperature_k[j];
            cvx[k] = cv_lake[j];
            phix[k] = phi[j];
        } else {
            let layer = k - n_lake;
            // `:1109` `.FMA (dz_nl, 0.5, z_nl) + z_soisno(j')`
            zx[k] = lake_bottom + z[layer];
            tx[k] = column.temperature_k[layer];
            cvx[k] = cv[layer];
        }
    }
    phix[soil_start] = phi_soil;
    let mut tkix = vec![0.0; total];
    for k in 0..total {
        tkix[k] = if k + 1 < snow {
            tk[k]
        } else if k + 1 == snow {
            let dzp = zx[k + 1] - zx[k];
            // `:1139` `((tkl1*tk0)*dzp) / .FMA (z_lake1, tk0, tkl1*(-zx0))`
            ((tk_lake[0] * tk[k]) * dzp) / z_lake[0].mul_add(tk[k], tk_lake[0] * (-zx[k]))
        } else if k + 1 < soil_start {
            let j = k - snow;
            // `:1143` `((tk_j*tk_j1)*(dz_j1+dz_j)) / .FMA (tk_j, dz_j1, tk_j1*dz_j)`
            ((tk_lake[j] * tk_lake[j + 1]) * (dz[j + 1] + dz[j]))
                / tk_lake[j].mul_add(dz[j + 1], tk_lake[j + 1] * dz[j])
        } else if k + 1 == soil_start {
            let j = n_lake - 1;
            let dzp = zx[k + 1] - zx[k];
            // `:1147` `((tktop*tk_n)*dzp) / .FMA (tktop*dz_n, 0.5, z_soisno(1)*tk_n)`
            ((tk_top_soil * tk_lake[j]) * dzp)
                / (tk_top_soil * dz[j]).mul_add(0.5, z[snow] * tk_lake[j])
        } else {
            tk[k - n_lake]
        };
    }
    let factx = cvx.iter().map(|value| dt / value).collect::<Vec<_>>();
    let mut fnx = vec![0.0; total];
    for k in 0..total - 1 {
        fnx[k] = (tkix[k] * (tx[k + 1] - tx[k])) / (zx[k + 1] - zx[k]);
    }

    // 三对角（`:1220-1240`）：`cnfac = 0.5`，`(1-cnfac)*factx` 与 `cnfac*factx` 是同一个
    // `factx*0.5`，编译器只算一次。
    let mut lower = vec![0.0; total];
    let mut diagonal = vec![0.0; total];
    let mut upper = vec![0.0; total];
    let mut rhs = vec![0.0; total];
    for k in 0..total {
        let half = factx[k] * (1.0 - CNFAC);
        if k == 0 {
            let dzp = zx[1] - zx[0];
            let coupling = (half * tkix[0]) / dzp;
            diagonal[0] = coupling + 1.0;
            upper[0] = -coupling;
            rhs[0] = match hs {
                // SNICAR 雪顶（`:1170-1176`）：`.FMA (factx, .FMA (fnx, 0.5, .FNMA (tx, 0, hs)), tx)`
                Some(hs) if snow > 0 => {
                    factx[0].mul_add(fnx[0].mul_add(CNFAC, (-tx[0]).mul_add(0.0, hs)), tx[0])
                }
                // `.FMA (factx, .FMA (fnx, cnfac, phix) + fgrnd1, tx)`
                _ => factx[0].mul_add(fnx[0].mul_add(CNFAC, phix[0]) + fgrnd1, tx[0]),
            };
        } else if k < total - 1 {
            let dzm = zx[k] - zx[k - 1];
            let dzp = zx[k + 1] - zx[k];
            lower[k] = -((half * tkix[k - 1]) / dzm);
            diagonal[k] = (tkix[k] / dzp + tkix[k - 1] / dzm).mul_add(half, 1.0);
            upper[k] = -((tkix[k] * half) / dzp);
            // SNICAR（`:1177-1190`）：非顶雪层的源项是本层吸收；雪下的湖顶层是
            // `.FMA (lyr(1), betaprime, phix)`。
            let source = match snicar {
                Some(lyr) if k < snow => lyr[5 - snow + k],
                Some(lyr) if k == snow => lyr[5].mul_add(betaprime, phix[k]),
                _ => phix[k],
            };
            rhs[k] = source.mul_add(factx[k], (fnx[k] - fnx[k - 1]).mul_add(half, tx[k]));
        } else {
            let dzm = zx[k] - zx[k - 1];
            let coupling = (half * tkix[k - 1]) / dzm;
            lower[k] = -coupling;
            diagonal[k] = coupling + 1.0;
            rhs[k] = (-half).mul_add(fnx[k - 1], tx[k]);
        }
    }
    let solved = crate::linear::solve_tridiagonal(&lower, &diagonal, &upper, &rhs)
        .map_err(|message| anyhow::anyhow!(message))?;
    for (k, value) in solved.into_iter().enumerate() {
        if k < snow {
            column.temperature_k[k] = value;
        } else if k < soil_start {
            lake.temperature_k[k - snow] = value;
        } else {
            column.temperature_k[k - n_lake] = value;
        }
    }

    // ---- 输出（`:1263-1301`）----
    let rib =
        ((zeta * (ustar * ustar)) / (((VONKAR * VONKAR) / profile.heat) * (um * um))).min(5.0);
    let trad = (olrg / STEFNC).lpow(0.25);
    let mut fgrnd = (((sabg + frl) - olrg) - fseng) - fevpg * htvp;
    let taux = -((input.eastward_wind_m_s * rho) / ram);
    let tauy = -((input.northward_wind_m_s * rho) / ram);
    let tref = (dth * (VONKAR / profile.heat))
        .mul_add(profile.heat_at_2m / VONKAR - profile.heat / VONKAR, thm);
    let qref = (dqh * (VONKAR / profile.moisture)).mul_add(
        profile.moisture_at_2m / VONKAR - profile.moisture / VONKAR,
        forc_q,
    );
    let (mut qseva, mut qsubl, mut qsdew, mut qfros) = (0.0, 0.0, 0.0, 0.0);
    if fevpg >= 0.0 {
        qseva = if snow > 0 {
            fevpg.min(column.liquid_water_kg_m2[0] / dt)
        } else {
            fevpg.min((dz[0] * ((1.0 - lake.ice_fraction[0]) * 1000.0)) / dt)
        };
        qsubl = fevpg - qseva;
    } else if t_grnd < TFRZ {
        qfros = fevpg.abs();
    } else {
        qsdew = fevpg.abs();
    }

    // ---- 相变（`:1330-1410`）----
    let mut sm = 0.0;
    let mut phase_flag = vec![0; n];
    if snow == 0 && *scv > 0.0 && lake.temperature_k[0] > TFRZ {
        let heat_available = (lake.temperature_k[0] - TFRZ) * cv_lake[0];
        let melt = (heat_available / HFUS).min(*scv);
        let heat_left = (heat_available - melt * HFUS).max(0.0);
        lake.temperature_k[0] = heat_left / cv_lake[0] + TFRZ;
        *snowdp = (*snowdp * (1.0 - melt / *scv)).max(0.0);
        *scv -= melt;
        if *scv < 1.0e-12 {
            *scv = 0.0;
        }
        if *snowdp < 1.0e-12 {
            *snowdp = 0.0;
        }
        sm += melt / dt;
    }
    for j in 0..n_lake {
        let t = lake.temperature_k[j];
        let fi = lake.ice_fraction[j];
        let heat_available = (t - TFRZ) * cv_lake[j];
        let (melt, heat_left) = if t > TFRZ && fi > 0.0 {
            let melt = ((fi * 1000.0) * dz[j]).min(heat_available / HFUS);
            (melt, (heat_available - melt * HFUS).max(0.0))
        } else if t < TFRZ && fi < 1.0 {
            let melt = ((-(1.0 - fi) * 1000.0) * dz[j]).max(heat_available / HFUS);
            (melt, (heat_available - melt * HFUS).min(0.0))
        } else {
            continue;
        };
        let mut fraction = fi - melt / (dz[j] * 1000.0);
        if fraction > 1.0 - 1.0e-12 {
            fraction = 1.0;
        }
        if fraction < 1.0e-12 {
            fraction = 0.0;
        }
        lake.ice_fraction[j] = fraction;
        // `:1376` `.FMA (melt, cpliq-cpice, cv)`
        cv_lake[j] = melt.mul_add(CPLIQ - CPICE, cv_lake[j]);
        lake.temperature_k[j] = heat_left / cv_lake[j] + TFRZ;
    }
    for layer in 0..n {
        let t = column.temperature_k[layer];
        let ice = column.ice_water_kg_m2[layer];
        let liquid = column.liquid_water_kg_m2[layer];
        let heat_available = (t - TFRZ) * cv[layer];
        let melt;
        let heat_left;
        if t > TFRZ && ice > 0.0 {
            phase_flag[layer] = 1;
            melt = ice.min(heat_available / HFUS);
            heat_left = (heat_available - melt * HFUS).max(0.0);
            if layer < snow {
                sm += melt / dt;
            }
        } else if t < TFRZ && liquid > 0.0 {
            phase_flag[layer] = 2;
            melt = (-liquid).max(heat_available / HFUS);
            heat_left = (heat_available - melt * HFUS).min(0.0);
        } else {
            continue;
        }
        let mut ice = ice - melt;
        let mut liquid = melt + liquid;
        if ice < 1.0e-12 {
            ice = 0.0;
        }
        if liquid < 1.0e-12 {
            liquid = 0.0;
        }
        column.ice_water_kg_m2[layer] = ice;
        column.liquid_water_kg_m2[layer] = liquid;
        cv[layer] = melt.mul_add(CPLIQ - CPICE, cv[layer]);
        column.temperature_k[layer] = heat_left / cv[layer] + TFRZ;
    }

    // ---- 对流混合（`:1448-1515`）----
    let mut density = (0..n_lake)
        .map(|j| lake_water_density(lake.temperature_k[j], lake.ice_fraction[j]))
        .collect::<Vec<_>>();
    let mut frozen_mean = 0.0;
    let mut unfrozen_mean = 0.0;
    for j in 0..n_lake - 1 {
        let mut heat = 0.0;
        let mut depth = 0.0;
        let mut ice = 0.0;
        if density[j] > density[j + 1]
            || (lake.ice_fraction[j] < 1.0 && lake.ice_fraction[j + 1] > 0.0)
        {
            #[allow(clippy::needless_range_loop)] // `i` 同时索引 dz、t_lake、lake_icefrac
            for i in 0..=j + 1 {
                let fi = lake.ice_fraction[i];
                // `:1462` `.FMA (dz*(t-tfrz), .FMA (1-f, cwat, f*cice_eff), qav)`
                heat = (dz[i] * (lake.temperature_k[i] - TFRZ))
                    .mul_add((1.0 - fi).mul_add(CWAT, fi * CICE_EFF), heat);
                ice = dz[i].mul_add(fi, ice);
                depth += dz[i];
            }
            heat /= depth;
            ice /= depth;
            if heat > 0.0 {
                frozen_mean = 0.0;
                unfrozen_mean = heat / ((1.0 - ice) * CWAT);
            } else if heat < 0.0 {
                frozen_mean = heat / (ice * CICE_EFF);
                unfrozen_mean = 0.0;
            } else {
                frozen_mean = 0.0;
                unfrozen_mean = 0.0;
            }
        }
        if depth > 0.0 {
            let mut zsum = 0.0;
            for i in 0..=j + 1 {
                if (zsum + dz[i]) / depth <= ice {
                    lake.ice_fraction[i] = 1.0;
                    lake.temperature_k[i] = frozen_mean + TFRZ;
                } else if zsum / depth < ice {
                    // `:1498` `.FMS (nav, iceav, zsum)/dz`
                    let fi = depth.mul_add(ice, -zsum) / dz[i];
                    lake.ice_fraction[i] = fi;
                    // `:1502` 分子 `.FMA (f*tfroz, cice_eff, ((1-f)*tunfr)*cwat)`，
                    // 分母 `.FMA (f, cice_eff, (1-f)*cwat)`
                    lake.temperature_k[i] = (fi * frozen_mean)
                        .mul_add(CICE_EFF, ((1.0 - fi) * unfrozen_mean) * CWAT)
                        / fi.mul_add(CICE_EFF, (1.0 - fi) * CWAT)
                        + TFRZ;
                } else {
                    lake.ice_fraction[i] = 0.0;
                    lake.temperature_k[i] = unfrozen_mean + TFRZ;
                }
                zsum += dz[i];
                density[i] = lake_water_density(lake.temperature_k[i], lake.ice_fraction[i]);
            }
        }
    }
    for j in 0..n_lake {
        cv_lake[j] = lake_heat_capacity(dz[j], lake.ice_fraction[j]);
    }

    // ---- 能量闭合（`:1522-1549`）：残差小于 0.1 W/m² 时并进感热与地表热通量 ----
    let ncvts = column_energy(lake, &dz, &cv_lake, column, &cv, snow, *scv);
    let error = (ncvts - ocvts) / dt - fgrnd;
    if error.abs() < 0.10 {
        fseng -= error;
        fgrnd += error;
    }

    *ground_temperature_k = t_grnd;
    Ok(LakeTemperatureOutput {
        fluxes: LakeThermalFluxes {
            taux,
            tauy,
            fsena: fseng,
            fevpa: fevpg,
            lfevpa: htvp * fevpg,
            fseng,
            fevpg,
            qseva,
            qsubl,
            qsdew,
            qfros,
            olrg,
            fgrnd,
            tref,
            qref,
            trad,
            emis: LAKE_EMISSIVITY,
            z0m: roughness.momentum_m,
            zol: zeta,
            rib,
            ustar,
            qstar,
            tstar,
            fm: profile.momentum,
            fh: profile.heat,
            fq: profile.moisture,
            sm,
        },
        phase_flag,
    })
}

fn roughness_at(
    snow: usize,
    ground_temperature_k: f64,
    lake_surface_temperature_k: f64,
    input: LakeTemperatureInput<'_>,
    charnock_parameter: f64,
    friction_velocity_m_s: f64,
) -> Result<LakeRoughness> {
    lake_roughness(LakeRoughnessInput {
        snow_layer_count: -(snow as i32),
        ground_temperature_k,
        lake_surface_temperature_k,
        surface_pressure_pa: input.surface_pressure_pa,
        charnock_parameter,
        friction_velocity_m_s,
    })
}

/// `cv_lake = dz*(cwat*(1-f) + cice_eff*f)`：`dz * .FMA (1-f, cwat, f*cice_eff)`（`:966`）。
fn lake_heat_capacity(thickness_m: f64, ice_fraction: f64) -> f64 {
    thickness_m * (1.0 - ice_fraction).mul_add(CWAT, ice_fraction * CICE_EFF)
}

/// `ocvts`/`ncvts`（`:1024-1036`、`:1522-1534`）：湖层 `.FMA (cv, t-tfrz, ·)` 再
/// `.FMA (dz*cfus, 1-f, ·)`；雪/土层 `.FMA (cv, t-tfrz, ·)` 再 `.FMA (wliq, hfus, ·)`，
/// 无雪层且 `scv > 0` 时在第一层土上 `.FNMA (scv, hfus, ·)`。
fn column_energy(
    lake: &LakeColumn,
    dz: &[f64],
    cv_lake: &[f64],
    column: &GlacierColumn,
    cv: &[f64],
    snow: usize,
    scv: f64,
) -> f64 {
    let mut energy = 0.0;
    for j in 0..dz.len() {
        energy = cv_lake[j].mul_add(lake.temperature_k[j] - TFRZ, energy);
        energy = (dz[j] * CFUS).mul_add(1.0 - lake.ice_fraction[j], energy);
    }
    #[allow(clippy::needless_range_loop)] // `layer` 同时索引 cv、t、wliq，且要判第一层
    for layer in 0..column.temperature_k.len() {
        energy = cv[layer].mul_add(column.temperature_k[layer] - TFRZ, energy);
        energy = column.liquid_water_kg_m2[layer].mul_add(HFUS, energy);
        if snow == 0 && layer == 0 && scv > 0.0 {
            energy = (-scv).mul_add(HFUS, energy);
        }
    }
    energy
}

#[cfg(test)]
#[path = "lake_temperature_tests.rs"]
mod lake_temperature_tests;
