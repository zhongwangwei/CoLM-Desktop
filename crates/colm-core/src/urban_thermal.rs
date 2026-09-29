//! `MOD_Urban_Thermal:UrbanTHERMAL`：城市地表能量平衡的编排层。
//!
//! 次序照上游：墙面阳/阴比例重分配 → 地面湿度与 `rss` → 长波矩阵（有树 5 面、无树 4 面）
//! → 地面相似性（`UrbanGroundFlux`）→ 湍流通量（`UrbanVegFlux`/`UrbanOnlyFlux`）→ 屋顶、
//! 两面墙、不透水与透水地面的温度 → 水体（`laketem`）→ 通量按新表面温度修正并汇总
//! → 长波增量 → 能量闭合 → 建筑能量模型（`SimpleBEM`）与人为热（`LUCY`）。
//!
//! 收缩形状对照 `MOD_Urban_Thermal.F90` 的 `-fdump-tree-optimized-lineno`。

// 循环照 Fortran 的下标逐句对照 GIMPLE，改成迭代器会让行号对照失去意义；
// `min(1).max(0.001)` 与 `clamp` 在 NaN 上语义不同（后者边界反转时还会 panic）。
#![allow(clippy::needless_range_loop, clippy::manual_clamp)]
use anyhow::{Context, Result};

use crate::{
    lake_temperature, root_uptake, saturation_specific_humidity, soil_psi_from_vliq,
    urban_bare_flux, urban_bem, urban_ground_flux, urban_impervious_temperature,
    urban_longwave_transfer, urban_lucy_flux, urban_pervious_temperature, urban_roof_temperature,
    urban_vegetated_flux, urban_wall_temperature, CalendarTime, GlacierColumn, LakeColumn,
    LakeTemperatureInput, LakeTemperatureState, LakeThermalFluxes, LeafBiochemistry, LibmPow,
    RootUptakeInput, ShortwaveForcing, SoilHydraulicModel, SoilThermalInput, StomataOptions,
    SurfaceLayerScheme, ThermalConductivityScheme, UrbanBemInput, UrbanFluxInput,
    UrbanGroundFluxInput, UrbanImperviousTemperatureInput, UrbanLongwaveInput,
    UrbanLongwaveVegetation, UrbanLucyFluxInput, UrbanPerviousTemperatureInput,
    UrbanRoofTemperatureInput, UrbanTreeInput, UrbanTreeState, UrbanWallTemperatureInput,
};

const HVAP: f64 = 2.5104e6;
const HSUB: f64 = 2.8440e6;
const STEFNC: f64 = 5.67e-8;
const TFRZ: f64 = 273.16;
const CPAIR: f64 = 1004.64;
const RGAS: f64 = 287.04;
const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;
/// `roverg = rwat/grav*1000`（`MOD_Const_Physical`）。
const ROVERG: f64 = 4.71047e4;
const FSH: f64 = 0.92;
const FLH: f64 = 0.08;

/// 本步不变的输入：强迫、几何、材料与参数（上游同名变量）。
#[derive(Debug, Clone, Copy)]
pub struct UrbanThermalContext<'a> {
    pub time_step_seconds: f64,
    pub latitude_radians: f64,
    pub longitude_radians: f64,
    /// LUCY 用的步末时刻与 `greenwich`。
    pub time: CalendarTime,
    pub greenwich: bool,
    pub wind_height_m: f64,
    pub temperature_height_m: f64,
    pub humidity_height_m: f64,
    pub absolute_heights: bool,
    pub eastward_wind_m_s: f64,
    pub northward_wind_m_s: f64,
    pub air_temperature_k: f64,
    pub specific_humidity: f64,
    pub surface_pressure_pa: f64,
    pub air_density_kg_m3: f64,
    pub downward_longwave_w_m2: f64,
    pub po2m: f64,
    pub pco2m: f64,
    pub shortwave: ShortwaveForcing,
    pub boundary_layer_height_m: Option<f64>,
    pub surface_layer_scheme: SurfaceLayerScheme,
    /// `theta = acos(max(coszen, 0.01))`
    pub zenith_angle_radians: f64,
    pub sabroof: f64,
    pub sabwsun: f64,
    pub sabwsha: f64,
    pub sabgimp: f64,
    pub sabgper: f64,
    pub sablake: f64,
    pub sabv: f64,
    pub par: f64,
    pub froof: f64,
    pub flake: f64,
    pub hroof: f64,
    pub hlr: f64,
    pub fgper: f64,
    pub eroof: f64,
    pub ewall: f64,
    pub egimp: f64,
    pub egper: f64,
    pub trsmx0: f64,
    pub zlnd: f64,
    pub zsno: f64,
    pub capr: f64,
    pub cnfac: f64,
    pub soil_thermal_inputs: &'a [SoilThermalInput],
    pub thermal_conductivity_scheme: ThermalConductivityScheme,
    pub soil_porosity: &'a [f64],
    pub soil_residual_water: &'a [f64],
    pub soil_suction_mm: &'a [f64],
    pub soil_hydraulic_model: &'a [SoilHydraulicModel],
    /// `DEF_USE_Campbell_SOIL_MODEL`
    pub campbell: bool,
    pub supercool_water: bool,
    pub cv_roof: &'a [f64],
    pub tk_roof: &'a [f64],
    pub cv_wall: &'a [f64],
    pub tk_wall: &'a [f64],
    pub cv_gimp: &'a [f64],
    pub tk_gimp: &'a [f64],
    pub dz_wall: &'a [f64],
    pub z_wall: &'a [f64],
    pub zi_wall: &'a [f64],
    pub lake_depth_m: f64,
    pub dewmx: f64,
    pub sqrtdi: f64,
    pub root_fraction: &'a [f64],
    pub root_stress_scheme: i32,
    pub biochemistry: LeafBiochemistry,
    pub stomata_options: StomataOptions,
    pub wue_lambda: f64,
    pub vegetation_snow: bool,
    pub lai: f64,
    pub sai: f64,
    pub htop: f64,
    pub hbot: f64,
    pub fveg: f64,
    pub sigf: f64,
    pub extkd: f64,
    pub fsno_roof: f64,
    pub fsno_gimp: f64,
    pub fsno_gper: f64,
    pub t_roommax: f64,
    pub t_roommin: f64,
    pub fix_holiday: &'a [f64],
    pub week_holiday: &'a [f64],
    pub hum_prof: &'a [f64],
    pub wdh_prof: &'a [f64],
    pub weh_prof: &'a [f64],
    pub pop_den: f64,
    pub vehicle: &'a [f64],
}

/// 一个带雪的城市面：雪层在前的打包列与它的雪标量。
#[derive(Debug)]
pub struct UrbanSurfaceRef<'a> {
    pub column: &'a mut GlacierColumn,
    pub snow_layers: usize,
    pub snow_water_equivalent_kg_m2: &'a mut f64,
    pub snow_depth_m: &'a mut f64,
}

/// 步内被改写的状态。
#[derive(Debug)]
pub struct UrbanThermalState<'a> {
    pub roof: UrbanSurfaceRef<'a>,
    pub impervious: UrbanSurfaceRef<'a>,
    pub pervious: UrbanSurfaceRef<'a>,
    /// 水体下的"土"列（雪层在前）；温度由 `laketem` 解。
    pub lake_bed: UrbanSurfaceRef<'a>,
    pub lake: &'a mut LakeColumn,
    pub saved_tke: &'a mut f64,
    pub t_wallsun: &'a mut [f64],
    pub t_wallsha: &'a mut [f64],
    pub fwsun: &'a mut f64,
    pub dfwsun: f64,
    pub lwsun: &'a mut f64,
    pub lwsha: &'a mut f64,
    pub lgimp: &'a mut f64,
    pub lgper: &'a mut f64,
    pub lveg: &'a mut f64,
    pub tree: &'a mut UrbanTreeState,
    pub t_room: &'a mut f64,
    pub troof_inner: &'a mut f64,
    pub twsun_inner: &'a mut f64,
    pub twsha_inner: &'a mut f64,
    pub tafu: &'a mut f64,
    pub fhac: &'a mut f64,
    pub fwst: &'a mut f64,
    pub fach: &'a mut f64,
    pub fahe: &'a mut f64,
    pub fhah: &'a mut f64,
    pub vehc: &'a mut f64,
    pub meta: &'a mut f64,
}

/// `UrbanTHERMAL` 的输出（上游同名变量）。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UrbanThermalOutput {
    pub taux: f64,
    pub tauy: f64,
    pub fsena: f64,
    pub fevpa: f64,
    pub lfevpa: f64,
    pub fsenl: f64,
    pub fevpl: f64,
    pub etr: f64,
    pub fseng: f64,
    pub fevpg: f64,
    pub olrg: f64,
    pub fgrnd: f64,
    pub fsen_roof: f64,
    pub fsen_wsun: f64,
    pub fsen_wsha: f64,
    pub fsen_gimp: f64,
    pub fsen_gper: f64,
    /// `fsen_urbl`：只在有树（`doveg`）时赋值，否则保持调用前的值（上游 `intent(out)` 不赋值，
    /// 实际是 `MOD_Urban_Vars_1DFluxes` 里初值 `spval` 的持久变量）。
    pub fsen_urbl: Option<f64>,
    pub troof: f64,
    pub twall: f64,
    pub lfevp_roof: f64,
    pub lfevp_gimp: f64,
    pub lfevp_gper: f64,
    pub lfevp_urbl: Option<f64>,
    pub qseva_roof: f64,
    pub qseva_gimp: f64,
    pub qseva_gper: f64,
    pub qseva_lake: f64,
    pub qsdew_roof: f64,
    pub qsdew_gimp: f64,
    pub qsdew_gper: f64,
    pub qsdew_lake: f64,
    pub qsubl_roof: f64,
    pub qsubl_gimp: f64,
    pub qsubl_gper: f64,
    pub qsubl_lake: f64,
    pub qfros_roof: f64,
    pub qfros_gimp: f64,
    pub qfros_gper: f64,
    pub qfros_lake: f64,
    pub imelt_roof: Vec<i32>,
    pub imelt_gimp: Vec<i32>,
    pub imelt_gper: Vec<i32>,
    pub imelt_lake: Vec<i32>,
    pub sm_roof: f64,
    pub sm_gimp: f64,
    pub sm_gper: f64,
    pub sm_lake: f64,
    pub sabg: f64,
    pub rss: f64,
    pub rstfac: f64,
    pub rootr: Vec<f64>,
    pub etr_deficit: f64,
    pub tref: f64,
    pub qref: f64,
    pub trad: f64,
    pub rst: f64,
    pub assim: f64,
    pub respc: f64,
    pub errore: f64,
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
    pub t_grnd: f64,
    /// `laketem` 解出的水面温度（`tlake`，`t_grnd` 实参）。
    pub tlake: f64,
    pub lake: LakeThermalFluxes,
}

/// Port of `MOD_Urban_Thermal:UrbanTHERMAL`（非 SNICAR）。
pub fn urban_thermal(
    ctx: UrbanThermalContext<'_>,
    st: UrbanThermalState<'_>,
) -> Result<UrbanThermalOutput> {
    let dt = ctx.time_step_seconds;
    let top = |surface: &UrbanSurfaceRef<'_>| {
        (
            surface.column.temperature_k[0],
            surface.column.liquid_water_kg_m2[0],
            surface.column.ice_water_kg_m2[0],
        )
    };
    let latent = |liquid: f64, ice: f64| {
        if liquid <= 0.0 && ice > 0.0 {
            HSUB
        } else {
            HVAP
        }
    };
    let (troof0, roof_liq, roof_ice) = top(&st.roof);
    let (tgimp0, gimp_liq, gimp_ice) = top(&st.impervious);
    let (tgper0, gper_liq, gper_ice) = top(&st.pervious);
    let htvp_roof = latent(roof_liq, roof_ice);
    let htvp_gimp = latent(gimp_liq, gimp_ice);
    let htvp_gper = latent(gper_liq, gper_ice);

    // `:597-601`
    let thm =
        crate::reference_height_temperature_k(ctx.air_temperature_k, ctx.temperature_height_m);
    let th = ctx.air_temperature_k * (100_000.0 / ctx.surface_pressure_pa).lpow(RGAS / CPAIR);
    let thv = th * ctx.specific_humidity.mul_add(0.61, 1.0);
    let ur = ctx
        .eastward_wind_m_s
        .mul_add(
            ctx.eastward_wind_m_s,
            ctx.northward_wind_m_s * ctx.northward_wind_m_s,
        )
        .sqrt()
        .max(0.1);

    // 墙面阳/阴比例变了，把温度与长波按面积重分配（`:605-620`）
    let fwsun = *st.fwsun;
    let fwsha = 1.0 - fwsun;
    let dfwsun = st.dfwsun;
    if dfwsun > 0.0 {
        let total = fwsun + dfwsun;
        for (sun, sha) in st.t_wallsun.iter_mut().zip(st.t_wallsha.iter()) {
            // `:608` `.FMA (t_wallsun, fwsun, t_wallsha*dfwsun)/(fwsun+dfwsun)`
            *sun = sun.mul_add(fwsun, *sha * dfwsun) / total;
        }
        *st.twsun_inner = fwsun.mul_add(*st.twsun_inner, dfwsun * *st.twsun_inner) / total;
        *st.lwsun = fwsun.mul_add(*st.lwsun, dfwsun * *st.lwsha) / total;
    }
    if dfwsun < 0.0 {
        let total = fwsha - dfwsun;
        for (sha, sun) in st.t_wallsha.iter_mut().zip(st.t_wallsun.iter()) {
            // `:614` `.FNMA (t_wallsun, dfwsun, fwsha*t_wallsha)/(fwsha-dfwsun)`
            *sha = (-*sun).mul_add(dfwsun, fwsha * *sha) / total;
        }
        // `:615` `.FMS (twsha_inner, fwsha, dfwsun*twsun_inner)/(…)`
        *st.twsha_inner = st.twsha_inner.mul_add(fwsha, -(dfwsun * *st.twsun_inner)) / total;
        *st.lwsha = st.lwsha.mul_add(fwsha, -(dfwsun * *st.lwsun)) / total;
    }
    let fwsun = fwsun + dfwsun;
    *st.fwsun = fwsun;
    // **不**重算 `fwsha`：上游 `:605` 只在更新 `fwsun` 之前算一次，之后全程
    // （长波、墙体导热、`:1002` 的 `twall`）用的都是旧值 `fwsha_1091`，
    // 所以更新后 `fwsun + fwsha ≠ 1`。

    let twsun0 = st.t_wallsun[0];
    let twsha0 = st.t_wallsha[0];
    let nl_roof = st.roof.column.temperature_k.len() - 1;
    let nl_wall = st.t_wallsun.len() - 1;
    let troof_nl_bef = st.roof.column.temperature_k[nl_roof];
    let twsun_nl_bef = st.t_wallsun[nl_wall];
    let twsha_nl_bef = st.t_wallsha[nl_wall];
    let tlake0 = if st.lake_bed.snow_layers > 0 {
        st.lake_bed.column.temperature_k[0]
    } else {
        st.lake.temperature_k[0]
    };
    let dlwsun = *st.lwsun;
    let dlwsha = *st.lwsha;
    let dlgimp = *st.lgimp;
    let dlgper = *st.lgper;
    let dlveg = *st.lveg;
    let fg = 1.0 - ctx.froof;
    let doveg = ctx.lai + ctx.sai > 1.0e-6 && ctx.fveg > 0.0;

    // 人为热换算到非水体面积（`:663-669`）
    let mut fhac = *st.fhac;
    let mut fwst = *st.fwst;
    let mut fach = *st.fach;
    let mut vehc = *st.vehc;
    let mut meta = *st.meta;
    if 1.0 - ctx.flake > 0.0 {
        let land = 1.0 - ctx.flake;
        fhac /= land;
        fwst /= land;
        fach /= land;
        vehc /= land;
        meta /= land;
    }

    // 透水地面湿度与 `rss`（`:676-723`）
    let qred;
    let saturation = saturation_specific_humidity(tgper0, ctx.surface_pressure_pa)?;
    let mut rss = 0.0;
    {
        let column = &st.pervious.column;
        let snow = st.pervious.snow_layers;
        let dz = &column.thickness_m;
        let liq = &column.liquid_water_kg_m2;
        let ice = &column.ice_water_kg_m2;
        let porsl = ctx.soil_porosity;
        let wx = (liq[snow] / DENH2O + ice[snow] / DENICE) / dz[snow];
        let fac = if porsl[0] < 1.0e-6 {
            0.001
        } else {
            (wx / porsl[0]).min(1.0).max(0.001)
        };
        let psit = if ctx.campbell {
            // `:692` `psi0*pow(fac, -bsw)`
            let bsw = match ctx.soil_hydraulic_model[0] {
                SoilHydraulicModel::Campbell { bsw } => bsw,
                _ => anyhow::bail!("DEF_USE_Campbell_SOIL_MODEL needs a Campbell soil"),
            };
            ctx.soil_suction_mm[0] * fac.lpow(-bsw)
        } else {
            // `:696` `vliq = .FMA (porsl-theta_r, fac, theta_r)`
            let residual = ctx.soil_residual_water[0];
            soil_psi_from_vliq(
                (porsl[0] - residual).mul_add(fac, residual),
                porsl[0],
                residual,
                ctx.soil_suction_mm[0],
                ctx.soil_hydraulic_model[0],
            )
        };
        let psit = psit.max(-1.0e8);
        let hr = ((psit / ROVERG) / tgper0).exp();
        // `:700` `.FMA (1-fsno, hr, fsno)`
        qred = (1.0 - ctx.fsno_gper).mul_add(hr, ctx.fsno_gper);
        if snow == 0 {
            let liquid = liq[0] + liq[1];
            let frozen = ice[0] + ice[1];
            let depth = dz[0] + dz[1];
            let wx = (liquid / DENH2O + frozen / DENICE) / depth;
            let fac = if porsl[0] + porsl[1] < 1.0e-6 {
                0.001
            } else {
                // `:709` `(wx*sumdz)/.FMA (dz1, porsl1, dz2*porsl2)`
                ((wx * depth) / dz[0].mul_add(porsl[0], dz[1] * porsl[1]))
                    .min(1.0)
                    .max(0.001)
            };
            // `:714` `(1-fsno)*exp(.FNMA (fac, 4.255, 8.206))`
            rss = (1.0 - ctx.fsno_gper) * (-fac).mul_add(4.255, 8.206).exp();
        }
    }
    let mut qgper = qred * saturation.specific_humidity;
    let mut dqgperdt = qred * saturation.specific_humidity_temperature_slope_k;
    if saturation.specific_humidity > ctx.specific_humidity
        && ctx.specific_humidity > qred * saturation.specific_humidity
    {
        qgper = ctx.specific_humidity;
        dqgperdt = 0.0;
    }
    let impervious = saturation_specific_humidity(tgimp0, ctx.surface_pressure_pa)?;
    let roof = saturation_specific_humidity(troof0, ctx.surface_pressure_pa)?;

    // 长波矩阵（`:738-807`）
    let transfer = urban_longwave_transfer(UrbanLongwaveInput {
        zenith_angle_radians: ctx.zenith_angle_radians,
        building_height_to_length: ctx.hlr,
        roof_fraction: ctx.froof,
        pervious_ground_fraction: ctx.fgper,
        roof_height_m: ctx.hroof,
        downward_longwave_w_m2: ctx.downward_longwave_w_m2,
        sunlit_wall_temperature_k: twsun0,
        shaded_wall_temperature_k: twsha0,
        impervious_temperature_k: tgimp0,
        pervious_temperature_k: tgper0,
        wall_emissivity: ctx.ewall,
        impervious_emissivity: ctx.egimp,
        pervious_emissivity: ctx.egper,
        vegetation: doveg.then_some(UrbanLongwaveVegetation {
            leaf_area_index: ctx.lai,
            stem_area_index: ctx.sai,
            cover_fraction: ctx.fveg,
            center_height_m: (ctx.htop + ctx.hbot) / 2.0,
        }),
    })?;
    let fcover = transfer.cover_fraction;
    let ainv = transfer.inverse;
    let n = transfer.surface_count;
    let emissivity = [ctx.ewall, ctx.ewall, ctx.egimp, ctx.egper];
    let mut lout = 0.0;
    if !doveg {
        // `:777` `X = matmul(Ainv, B)`（4×4）
        let mut x = [0.0; 4];
        for (i, value) in x.iter_mut().enumerate() {
            let mut acc = 0.0;
            for j in 0..4 {
                acc = ainv[i][j].mul_add(transfer.source[j], acc);
            }
            *value = acc;
        }
        let mut l = [0.0; 4];
        for i in 0..4 {
            // `:781` `.FMS (e, X, B1)/(1-e)`
            l[i] = emissivity[i].mul_add(x[i], -transfer.emitted[i]) / (1.0 - emissivity[i]);
        }
        for i in 0..4 {
            lout = x[i].mul_add(transfer.sky_view_factor[i], lout);
        }
        for i in 0..4 {
            if fcover[i + 1] > 0.0 {
                l[i] = fg * (l[i] / fcover[i + 1]);
            }
        }
        *st.lwsun = l[0] + dlwsun;
        *st.lwsha = l[1] + dlwsha;
        *st.lgimp = l[2] + dlgimp;
        *st.lgper = l[3] + dlgper;
    }
    // `:809` `.FMA (fc4, dlgper, .FMA (fc3, dlgimp, .FMA (fc1, dlwsun, fc2*dlwsha)))`
    let mut dlwbef = fcover[4].mul_add(
        dlgper,
        fcover[3].mul_add(dlgimp, fcover[1].mul_add(dlwsun, fcover[2] * dlwsha)),
    );
    if doveg {
        dlwbef = fcover[5].mul_add(dlveg, dlwbef);
    }
    dlwbef *= 1.0 - ctx.flake;
    // `:814` `.FMS (eroof, frl, (eroof*stefnc)*troof^4)`
    let roof_emission = ctx.eroof * STEFNC;
    let troof2 = troof0 * troof0;
    let lroof = ctx.eroof.mul_add(
        ctx.downward_longwave_w_m2,
        -(roof_emission * (troof2 * troof2)),
    );

    // 地面相似性（`:823-831`）
    let ground = urban_ground_flux(UrbanGroundFluxInput {
        wind_height_m: ctx.wind_height_m,
        temperature_height_m: ctx.temperature_height_m,
        humidity_height_m: ctx.humidity_height_m,
        reference_specific_humidity: ctx.specific_humidity,
        reference_wind_m_s: ur,
        reference_temperature_k: thm,
        potential_temperature_k: th,
        virtual_potential_temperature_k: thv,
        land_roughness_m: ctx.zlnd,
        snow_roughness_m: ctx.zsno,
        impervious_snow_fraction: ctx.fsno_gimp,
        impervious_has_snow_layers: st.impervious.snow_layers > 0,
        impervious_surface_liquid_water_kg_m2: gimp_liq_layer1(&st.impervious),
        impervious_surface_ice_kg_m2: gimp_ice_layer1(&st.impervious),
        cover_fraction: fcover,
        impervious_temperature_k: tgimp0,
        pervious_temperature_k: tgper0,
        impervious_specific_humidity: impervious.specific_humidity,
        pervious_specific_humidity: qgper,
    })?;
    let obu_g = ctx.wind_height_m / ground.dimensionless_height;

    let flux_input = UrbanFluxInput {
        time_step_seconds: dt,
        roof_has_snow_layers: st.roof.snow_layers > 0,
        impervious_has_snow_layers: st.impervious.snow_layers > 0,
        wind_height_m: ctx.wind_height_m,
        temperature_height_m: ctx.temperature_height_m,
        humidity_height_m: ctx.humidity_height_m,
        absolute_heights: ctx.absolute_heights,
        eastward_wind_m_s: ctx.eastward_wind_m_s,
        northward_wind_m_s: ctx.northward_wind_m_s,
        thm,
        th,
        thv,
        qm: ctx.specific_humidity,
        psrf: ctx.surface_pressure_pa,
        rhoair: ctx.air_density_kg_m3,
        fhac,
        fwst,
        fach,
        vehc,
        meta,
        hroof: ctx.hroof,
        hlr: ctx.hlr,
        fcover,
        z0h_g: ground.heat_roughness_m,
        obug: obu_g,
        ustarg: ground.friction_velocity_m_s,
        zlnd: ctx.zlnd,
        zsno: ctx.zsno,
        fsno_roof: ctx.fsno_roof,
        fsno_gimp: ctx.fsno_gimp,
        fsno_gper: ctx.fsno_gper,
        wliq_roof: layer1(&st.roof).0,
        wliq_gimp: gimp_liq_layer1(&st.impervious),
        wice_roof: layer1(&st.roof).1,
        wice_gimp: gimp_ice_layer1(&st.impervious),
        htvp_roof,
        htvp_gimp,
        htvp_gper,
        troof: troof0,
        twsun: twsun0,
        twsha: twsha0,
        tgimp: tgimp0,
        tgper: tgper0,
        qroof: roof.specific_humidity,
        qgimp: impervious.specific_humidity,
        qgper,
        dqroofdt: roof.specific_humidity_temperature_slope_k,
        dqgimpdt: impervious.specific_humidity_temperature_slope_k,
        dqgperdt,
        rss,
        surface_layer_scheme: SurfaceLayerScheme::Standard,
    };
    let mut out = UrbanThermalOutput {
        rst: 2.0e4,
        rootr: vec![0.0; ctx.root_fraction.len()],
        ..Default::default()
    };
    let flux;
    let mut dheatl = 0.0;
    if doveg {
        let snow = st.pervious.snow_layers;
        let roots = root_uptake(RootUptakeInput {
            maximum_transpiration_mm_s: ctx.trsmx0,
            porosity: ctx.soil_porosity,
            residual_water: ctx.soil_residual_water,
            saturated_soil_suction_mm: ctx.soil_suction_mm,
            hydraulic_model: ctx.soil_hydraulic_model,
            root_fraction: ctx.root_fraction,
            layer_thickness_m: &st.pervious.column.thickness_m[snow..],
            temperature_k: &st.pervious.column.temperature_k[snow..],
            liquid_water_kg_m2: &st.pervious.column.liquid_water_kg_m2[snow..],
            stress_scheme: ctx.root_stress_scheme,
        })?;
        out.rootr = roots.layer_fraction.clone();
        out.rstfac = roots.soil_water_stress;
        let (common, leaf) = urban_vegetated_flux(
            flux_input,
            UrbanTreeInput {
                frl: ctx.downward_longwave_w_m2,
                po2m: ctx.po2m,
                pco2m: ctx.pco2m,
                par: ctx.par,
                sabv: ctx.sabv,
                rstfac: roots.soil_water_stress,
                ewall: ctx.ewall,
                egimp: ctx.egimp,
                egper: ctx.egper,
                ev: transfer.vegetation_emissivity,
                htop: ctx.htop,
                hbot: ctx.hbot,
                lai: ctx.lai,
                sai: ctx.sai,
                sqrtdi: ctx.sqrtdi,
                extkd: ctx.extkd,
                dewmx: ctx.dewmx,
                etrc: roots.maximum_transpiration_mm_s,
                trsmx0: ctx.trsmx0,
                biochemistry: ctx.biochemistry,
                stomata_options: ctx.stomata_options,
                wue_lambda: ctx.wue_lambda,
                vegetation_snow: ctx.vegetation_snow,
                ainv,
                b: transfer.source,
                b1: transfer.emitted,
                dbdt: transfer.temperature_derivative,
                sky_vf: transfer.sky_view_factor,
                veg_vf: transfer.vegetation_view_factor,
                lwsun: dlwsun,
                lwsha: dlwsha,
                lgimp: dlgimp,
                lgper: dlgper,
                lveg: dlveg,
            },
            st.tree,
        )?;
        flux = common;
        out.fsenl = leaf.fsenl;
        out.fevpl = leaf.fevpl;
        out.etr = leaf.etr;
        out.rst = leaf.rst;
        out.assim = leaf.assim;
        out.respc = leaf.respc;
        out.etr_deficit = leaf.etr_deficit;
        dheatl = leaf.dheatl;
        *st.lwsun = leaf.lwsun;
        *st.lwsha = leaf.lwsha;
        *st.lgimp = leaf.lgimp;
        *st.lgper = leaf.lgper;
        *st.lveg = leaf.lveg;
        lout = leaf.lout;
    } else {
        flux = urban_bare_flux(flux_input)?;
        st.tree.tl = ctx.air_temperature_k;
        st.tree.ldew = 0.0;
        st.tree.ldew_rain = 0.0;
        st.tree.ldew_snow = 0.0;
        st.tree.fwet_snow = 0.0;
        out.rstfac = 0.0;
    }
    out.rss = rss;
    *st.tafu = flux.tafu;

    // 长波对表面温度的导数（`:949-958`）
    let troof3 = troof0 * troof2;
    let four_roof_emission = (ctx.eroof * 4.0) * STEFNC;
    let clroof = -(four_roof_emission * troof3);
    let mut cl = [0.0; 4];
    for i in 0..4 {
        // `:950` `(.FMA (e, Ainv(i,i), -1)/(1-e))*dBdT(i)`
        cl[i] = (emissivity[i].mul_add(ainv[i][i], -1.0) / (1.0 - emissivity[i]))
            * transfer.temperature_derivative[i];
        if fcover[i + 1] > 0.0 {
            cl[i] = fg * (cl[i] / fcover[i + 1]);
        }
    }

    // 各面温度（`:961-994`）
    let roof_state = urban_roof_temperature(UrbanRoofTemperatureInput {
        time_step_seconds: dt,
        surface_temperature_factor: ctx.capr,
        crank_nicolson_factor: ctx.cnfac,
        roof_heat_capacity_j_m3_k: ctx.cv_roof,
        roof_conductivity_w_m_k: ctx.tk_roof,
        snow_layers: st.roof.snow_layers,
        layer_thickness_m: &st.roof.column.thickness_m,
        node_depth_m: &st.roof.column.node_depth_m,
        interface_depth_m: &st.roof.column.interface_depth_m,
        temperature_k: &st.roof.column.temperature_k,
        liquid_water_kg_m2: &st.roof.column.liquid_water_kg_m2,
        ice_water_kg_m2: &st.roof.column.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: *st.roof.snow_water_equivalent_kg_m2,
        snow_depth_m: *st.roof.snow_depth_m,
        inner_surface_temperature_k: *st.troof_inner,
        absorbed_longwave_w_m2: lroof,
        longwave_temperature_slope_w_m2_k: clroof,
        absorbed_shortwave_w_m2: ctx.sabroof,
        sensible_heat_w_m2: flux.fsenroof,
        evaporation_kg_m2_s: flux.fevproof,
        surface_energy_temperature_slope_w_m2_k: flux.croof,
        vaporization_heat_j_kg: htvp_roof,
    })?;
    st.roof.column.temperature_k = roof_state.temperature_k;
    st.roof.column.liquid_water_kg_m2 = roof_state.liquid_water_kg_m2;
    st.roof.column.ice_water_kg_m2 = roof_state.ice_water_kg_m2;
    *st.roof.snow_water_equivalent_kg_m2 = roof_state.snow_water_equivalent_kg_m2;
    *st.roof.snow_depth_m = roof_state.snow_depth_m;
    out.sm_roof = roof_state.snow_melt_rate_kg_m2_s;
    out.imelt_roof = roof_state.phase_flag;
    let tkdz_roof = roof_state.inner_conductance_w_m2_k;

    let wall = |temperature: &[f64],
                inner: f64,
                longwave: f64,
                slope: f64,
                shortwave: f64,
                sensible: f64,
                sensible_slope: f64| {
        urban_wall_temperature(UrbanWallTemperatureInput {
            time_step_seconds: dt,
            crank_nicolson_factor: ctx.cnfac,
            heat_capacity_j_m3_k: ctx.cv_wall,
            conductivity_w_m_k: ctx.tk_wall,
            layer_thickness_m: ctx.dz_wall,
            node_depth_m: ctx.z_wall,
            interface_depth_m: ctx.zi_wall,
            inner_surface_temperature_k: inner,
            absorbed_longwave_w_m2: longwave,
            longwave_temperature_slope_w_m2_k: slope,
            absorbed_shortwave_w_m2: shortwave,
            sensible_heat_w_m2: sensible,
            sensible_temperature_slope_w_m2_k: sensible_slope,
            temperature_k: temperature,
        })
    };
    let sun = wall(
        st.t_wallsun,
        *st.twsun_inner,
        *st.lwsun,
        cl[0],
        ctx.sabwsun,
        flux.fsenwsun,
        flux.cwsuns,
    )?;
    st.t_wallsun.copy_from_slice(&sun.temperature_k);
    let sha = wall(
        st.t_wallsha,
        *st.twsha_inner,
        *st.lwsha,
        cl[1],
        ctx.sabwsha,
        flux.fsenwsha,
        flux.cwshas,
    )?;
    st.t_wallsha.copy_from_slice(&sha.temperature_k);

    let gimp_state = urban_impervious_temperature(UrbanImperviousTemperatureInput {
        time_step_seconds: dt,
        surface_temperature_factor: ctx.capr,
        crank_nicolson_factor: ctx.cnfac,
        thermal_conductivity_scheme: ctx.thermal_conductivity_scheme,
        soil_thermal_inputs: ctx.soil_thermal_inputs,
        impervious_heat_capacity_j_m3_k: ctx.cv_gimp,
        impervious_interface_conductivity_w_m_k: ctx.tk_gimp,
        snow_layers: st.impervious.snow_layers,
        layer_thickness_m: &st.impervious.column.thickness_m,
        node_depth_m: &st.impervious.column.node_depth_m,
        interface_depth_m: &st.impervious.column.interface_depth_m,
        temperature_k: &st.impervious.column.temperature_k,
        liquid_water_kg_m2: &st.impervious.column.liquid_water_kg_m2,
        ice_water_kg_m2: &st.impervious.column.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: *st.impervious.snow_water_equivalent_kg_m2,
        snow_depth_m: *st.impervious.snow_depth_m,
        absorbed_longwave_w_m2: *st.lgimp,
        longwave_temperature_slope_w_m2_k: cl[2],
        absorbed_shortwave_w_m2: ctx.sabgimp,
        sensible_heat_w_m2: flux.fsengimp,
        evaporation_kg_m2_s: flux.fevpgimp,
        surface_energy_temperature_slope_w_m2_k: flux.cgimp,
        vaporization_heat_j_kg: htvp_gimp,
    })?;
    st.impervious.column.temperature_k = gimp_state.temperature_k;
    st.impervious.column.liquid_water_kg_m2 = gimp_state.liquid_water_kg_m2;
    st.impervious.column.ice_water_kg_m2 = gimp_state.ice_water_kg_m2;
    *st.impervious.snow_water_equivalent_kg_m2 = gimp_state.snow_water_equivalent_kg_m2;
    *st.impervious.snow_depth_m = gimp_state.snow_depth_m;
    out.sm_gimp = gimp_state.snow_melt_rate_kg_m2_s;
    out.imelt_gimp = gimp_state.phase_flag;

    let gper_state = urban_pervious_temperature(UrbanPerviousTemperatureInput {
        patch_type: 1,
        time_step_seconds: dt,
        surface_temperature_factor: ctx.capr,
        crank_nicolson_factor: ctx.cnfac,
        thermal_conductivity_scheme: ctx.thermal_conductivity_scheme,
        soil_thermal_inputs: ctx.soil_thermal_inputs,
        soil_porosity: ctx.soil_porosity,
        soil_residual_water: ctx.soil_residual_water,
        soil_suction_mm: ctx.soil_suction_mm,
        soil_hydraulic_model: ctx.soil_hydraulic_model,
        snow_layers: st.pervious.snow_layers,
        layer_thickness_m: &st.pervious.column.thickness_m,
        node_depth_m: &st.pervious.column.node_depth_m,
        interface_depth_m: &st.pervious.column.interface_depth_m,
        temperature_k: &st.pervious.column.temperature_k,
        liquid_water_kg_m2: &st.pervious.column.liquid_water_kg_m2,
        ice_water_kg_m2: &st.pervious.column.ice_water_kg_m2,
        snow_water_equivalent_kg_m2: *st.pervious.snow_water_equivalent_kg_m2,
        snow_depth_m: *st.pervious.snow_depth_m,
        absorbed_longwave_w_m2: *st.lgper,
        longwave_temperature_slope_w_m2_k: cl[3],
        absorbed_shortwave_w_m2: ctx.sabgper,
        sensible_heat_w_m2: flux.fsengper,
        evaporation_kg_m2_s: flux.fevpgper,
        surface_energy_temperature_slope_w_m2_k: flux.cgper,
        vaporization_heat_j_kg: htvp_gper,
        supercool_water: ctx.supercool_water,
    })?;
    st.pervious.column.temperature_k = gper_state.temperature_k;
    st.pervious.column.liquid_water_kg_m2 = gper_state.liquid_water_kg_m2;
    st.pervious.column.ice_water_kg_m2 = gper_state.ice_water_kg_m2;
    *st.pervious.snow_water_equivalent_kg_m2 = gper_state.snow_water_equivalent_kg_m2;
    *st.pervious.snow_depth_m = gper_state.snow_depth_m;
    out.sm_gper = gper_state.snow_melt_rate_kg_m2_s;
    out.imelt_gper = gper_state.phase_flag;

    let twsun = st.t_wallsun[0];
    let twsha = st.t_wallsha[0];
    let troof = st.roof.column.temperature_k[0];
    let tgimp = st.impervious.column.temperature_k[0];
    let tgper = st.pervious.column.temperature_k[0];
    // `:1002` `.FMA (twsun, fwsun, twsha*fwsha)/(fwsun+fwsha)`
    out.twall = twsun.mul_add(fwsun, twsha * fwsha) / (fwsun + fwsha);
    out.troof = troof;

    // 水体（`:1005-1041`）
    let mut tlake = tlake0;
    let lake_state = lake_temperature(
        LakeTemperatureInput {
            time_step_seconds: dt,
            latitude_radians: ctx.latitude_radians,
            wind_height_m: ctx.wind_height_m,
            temperature_height_m: ctx.temperature_height_m,
            humidity_height_m: ctx.humidity_height_m,
            eastward_wind_m_s: ctx.eastward_wind_m_s,
            northward_wind_m_s: ctx.northward_wind_m_s,
            air_temperature_k: ctx.air_temperature_k,
            specific_humidity: ctx.specific_humidity,
            air_density_kg_m3: ctx.air_density_kg_m3,
            surface_pressure_pa: ctx.surface_pressure_pa,
            shortwave: ctx.shortwave,
            absorbed_shortwave_w_m2: ctx.sablake,
            downward_longwave_w_m2: ctx.downward_longwave_w_m2,
            boundary_layer_height_m: ctx.boundary_layer_height_m,
            surface_layer_scheme: ctx.surface_layer_scheme,
            lake_depth_m: ctx.lake_depth_m,
            thermal_conductivity_scheme: ctx.thermal_conductivity_scheme,
            soil_thermal_inputs: ctx.soil_thermal_inputs,
            snow_layers: st.lake_bed.snow_layers,
            // 城市水体带 `urban_call`，SNICAR 分支全部不走。
            snow_layer_absorption_w_m2: None,
        },
        LakeTemperatureState {
            lake: st.lake,
            saved_tke: st.saved_tke,
            ground_temperature_k: &mut tlake,
            snow_water_equivalent_kg_m2: st.lake_bed.snow_water_equivalent_kg_m2,
            snow_depth_m: st.lake_bed.snow_depth_m,
            column: st.lake_bed.column,
        },
    )
    .context("urban water body")?;
    let lake = lake_state.fluxes;
    out.imelt_lake = lake_state.phase_flag;
    out.sm_lake = lake.sm;
    out.qseva_lake = lake.qseva;
    out.qsubl_lake = lake.qsubl;
    out.qsdew_lake = lake.qsdew;
    out.qfros_lake = lake.qfros;
    out.tlake = tlake;
    let lnet_lake = ctx.downward_longwave_w_m2 - lake.olrg;

    // 通量按新表面温度修正（`:1048-1086`）
    let dt0 = troof - troof0;
    let dt1 = twsun - twsun0;
    let dt2 = twsha - twsha0;
    let dt3 = tgimp - tgimp0;
    let dt4 = tgper - tgper0;
    let mut fsenroof = dt0.mul_add(flux.croofs, flux.fsenroof);
    let fsenwsun = dt1.mul_add(flux.cwsuns, flux.fsenwsun);
    let fsenwsha = dt2.mul_add(flux.cwshas, flux.fsenwsha);
    let mut fsengimp = dt3.mul_add(flux.cgrnds, flux.fsengimp);
    let mut fsengper = dt4.mul_add(flux.cgrnds, flux.fsengper);
    let mut fevproof = dt0.mul_add(flux.croofl, flux.fevproof);
    let mut fevpgimp = dt3.mul_add(flux.cgimpl, flux.fevpgimp);
    let mut fevpgper = dt4.mul_add(flux.cgperl, flux.fevpgper);
    let cap = |fevp: &mut f64, fsen: &mut f64, surface: &UrbanSurfaceRef<'_>, htvp: f64| {
        let egsmax =
            (surface.column.ice_water_kg_m2[0] + surface.column.liquid_water_kg_m2[0]) / dt;
        let egidif = (*fevp - egsmax).max(0.0);
        *fevp = fevp.min(egsmax);
        *fsen = htvp.mul_add(egidif, *fsen);
    };
    cap(&mut fevpgper, &mut fsengper, &st.pervious, htvp_gper);
    cap(&mut fevpgimp, &mut fsengimp, &st.impervious, htvp_gimp);
    cap(&mut fevproof, &mut fsenroof, &st.roof, htvp_roof);

    // 汇总（`:1092-1138`）
    let lw = [*st.lwsun, *st.lwsha, *st.lgimp, *st.lgper];
    let mut lnet = lroof.mul_add(fcover[0], fcover[1] * lw[0]);
    lnet = fcover[2].mul_add(lw[1], lnet);
    lnet = fcover[3].mul_add(lw[2], lnet);
    lnet = fcover[4].mul_add(lw[3], lnet);
    let mut sabg = fcover[0].mul_add(ctx.sabroof, fcover[1] * ctx.sabwsun);
    sabg = fcover[2].mul_add(ctx.sabwsha, sabg);
    sabg = fcover[3].mul_add(ctx.sabgimp, sabg);
    sabg = fcover[4].mul_add(ctx.sabgper, sabg);
    // `:1100` 五个乘积与 `fsen_*` 共用，**不**融合
    out.fsen_roof = fsenroof * fcover[0];
    out.fsen_wsun = fcover[1] * fsenwsun;
    out.fsen_wsha = fcover[2] * fsenwsha;
    out.fsen_gimp = fcover[3] * fsengimp;
    out.fsen_gper = fcover[4] * fsengper;
    let mut fseng =
        (((out.fsen_roof + out.fsen_wsun) + out.fsen_wsha) + out.fsen_gimp) + out.fsen_gper;
    let mut fevpg = fcover[4].mul_add(fevpgper, fcover[0].mul_add(fevproof, fcover[3] * fevpgimp));
    out.lfevp_roof = fcover[0] * (htvp_roof * fevproof);
    out.lfevp_gimp = fcover[3] * (htvp_gimp * fevpgimp);
    out.lfevp_gper = fcover[4] * (htvp_gper * fevpgper);
    let mut lfevpa = (out.lfevp_roof + out.lfevp_gimp) + out.lfevp_gper;
    let mut fsena;
    let fevpa;
    if doveg {
        out.assim *= ctx.fveg;
        out.respc *= ctx.fveg;
        out.fsenl *= ctx.fveg;
        out.fevpl *= ctx.fveg;
        out.etr *= ctx.fveg;
        fsena = fseng + out.fsenl;
        fevpa = fevpg + out.fevpl;
        let lfevp_urbl = out.fevpl * HVAP;
        lfevpa += lfevp_urbl;
        out.lfevp_urbl = Some(lfevp_urbl);
        out.fsen_urbl = Some(out.fsenl);
        out.etr_deficit *= ctx.fveg;
    } else {
        fsena = fseng;
        fevpa = fevpg;
    }
    let anthropogenic = (fhac + fwst) + vehc;
    // `:1137` `.FMA (Fhac+Fwst+vehc, fsh, fsena) + Fach + meta`
    fsena = (anthropogenic.mul_add(FSH, fsena) + fach) + meta;
    lfevpa = anthropogenic.mul_add(FLH, lfevpa);

    // 与水体按面积混合（`:1141-1159`）：`.FMA (1-flake, x, flake*x_lake)`
    let land = 1.0 - ctx.flake;
    let blend = |x: f64, lake_x: f64| land.mul_add(x, ctx.flake * lake_x);
    out.taux = blend(flux.taux, lake.taux);
    out.tauy = blend(flux.tauy, lake.tauy);
    sabg = blend(sabg, ctx.sablake);
    lnet = blend(lnet, lnet_lake);
    fseng = blend(fseng, lake.fseng);
    fsena = blend(fsena, lake.fsena);
    fevpg = blend(fevpg, lake.fevpg);
    let lfevpa_lake_part = ctx.flake * lake.lfevpa;
    lfevpa = land.mul_add(lfevpa, lfevpa_lake_part);
    out.tref = blend(flux.tref, lake.tref);
    out.qref = blend(flux.qref, lake.qref);
    out.z0m = blend(flux.z0m, lake.z0m);
    out.zol = blend(flux.zol, lake.zol);
    out.rib = blend(flux.rib, lake.rib);
    out.ustar = blend(flux.ustar, lake.ustar);
    out.qstar = blend(flux.qstar, lake.qstar);
    out.tstar = blend(flux.tstar, lake.tstar);
    out.fm = blend(flux.fm, lake.fm);
    out.fh = blend(flux.fh, lake.fh);
    out.fq = blend(flux.fq, lake.fq);
    out.sabg = sabg;
    // `:1175` `.FMA (tgper, fgper, tgimp*(1-fgper))`
    out.t_grnd = tgper.mul_add(ctx.fgper, tgimp * (1.0 - ctx.fgper));

    // 各面的蒸发/凝结拆分（`:1178-1229`）
    let split = |fevp: f64, liquid: f64, t: f64| {
        if fevp >= 0.0 {
            let qseva = (liquid / dt).min(fevp);
            (qseva, fevp - qseva, 0.0, 0.0)
        } else if t < TFRZ {
            (0.0, 0.0, fevp.abs(), 0.0)
        } else {
            (0.0, 0.0, 0.0, fevp.abs())
        }
    };
    (
        out.qseva_roof,
        out.qsubl_roof,
        out.qfros_roof,
        out.qsdew_roof,
    ) = split(fevproof, st.roof.column.liquid_water_kg_m2[0], troof);
    (
        out.qseva_gimp,
        out.qsubl_gimp,
        out.qfros_gimp,
        out.qsdew_gimp,
    ) = split(fevpgimp, st.impervious.column.liquid_water_kg_m2[0], tgimp);
    (
        out.qseva_gper,
        out.qsubl_gper,
        out.qfros_gper,
        out.qsdew_gper,
    ) = split(fevpgper, st.pervious.column.liquid_water_kg_m2[0], tgper);

    // 长波增量（`:1235-1269`）
    let dtv = [dt1, dt2, dt3, dt4, 0.0];
    let mut d = [0.0; 5];
    for (i, value) in d.iter_mut().enumerate().take(n) {
        *value = transfer.temperature_derivative[i] * dtv[i];
    }
    let mut dx = [0.0; 5];
    for i in 0..n {
        let mut acc = 0.0;
        for j in 0..n {
            acc = ainv[i][j].mul_add(d[j], acc);
        }
        dx[i] = acc;
    }
    let mut dlw = [0.0; 4];
    for i in 0..4 {
        // `:1236` `.FMS (e, dX, dT*dBdT)/(1-e)`
        dlw[i] = emissivity[i].mul_add(dx[i], -(dtv[i] * transfer.temperature_derivative[i]))
            / (1.0 - emissivity[i]);
    }
    let mut dlveg_new = 0.0;
    if doveg {
        let mut acc = 0.0;
        for i in 0..5 {
            acc = dx[i].mul_add(transfer.vegetation_view_factor[i], acc);
        }
        dlveg_new = transfer.vegetation_emissivity * acc;
    }
    let mut dlout = 0.0;
    for i in 0..n {
        dlout = dx[i].mul_add(transfer.sky_view_factor[i], dlout);
    }
    for i in 0..4 {
        if fcover[i + 1] > 0.0 {
            dlw[i] = fg * (dlw[i] / fcover[i + 1]);
        }
    }
    if doveg {
        dlveg_new = fg * (dlveg_new / fcover[5]);
    }
    lout += dlout;
    // `:1271` `rout = (FMA (1-eroof, frl, eroof*stefnc*tb^4)) + dT0*(4*eroof*stefnc*tb^3)`
    let rout = (1.0 - ctx.eroof).mul_add(
        ctx.downward_longwave_w_m2,
        roof_emission * (troof2 * troof2),
    ) + dt0 * (four_roof_emission * troof3);
    let roof_linear = dt0 * (four_roof_emission * troof3);
    // `:1273` `.FMA (fg, lout, froof*rout)`，再与水体混合
    out.olrg = land.mul_add(fg.mul_add(lout, ctx.froof * rout), ctx.flake * lake.olrg);
    out.trad = (out.olrg / STEFNC).lpow(0.25);

    // 地面热通量与能量闭合（`:1300-1312`）
    let lfevp_ground = (out.lfevp_roof + out.lfevp_gimp) + out.lfevp_gper;
    let mut fgrnd = (-land).mul_add(
        ctx.froof * roof_linear,
        (-land).mul_add(fg * dlout, (sabg + lnet) - dlwbef),
    ) - fseng;
    fgrnd = (-land).mul_add(lfevp_ground, fgrnd) - lfevpa_lake_part;
    let anthro_total = (((fhac + fwst) + fach) + vehc) + meta;
    // `:1310` `.FNMA (1-flake, fveg*dheatl, (((.FMA (1-flake, anthro, (.FMA (1-flake, sabv*fveg,
    // sabg) + frl) - olrg) - fsena) - lfevpa) - fgrnd))`
    out.errore = (-land).mul_add(
        ctx.fveg * dheatl,
        ((land.mul_add(
            anthro_total,
            (land.mul_add(ctx.sabv * ctx.fveg, sabg) + ctx.downward_longwave_w_m2) - out.olrg,
        ) - fsena)
            - lfevpa)
            - fgrnd,
    );
    fgrnd = (-land).mul_add(anthro_total, fgrnd);
    out.fsena = fsena;
    out.fevpa = fevpa;
    out.lfevpa = lfevpa;
    out.fseng = fseng;
    out.fevpg = fevpg;

    // 下一步用的长波修正（`:1331-1335`）
    *st.lwsun = dlw[0];
    *st.lwsha = dlw[1];
    *st.lgimp = dlw[2];
    *st.lgper = dlw[3];
    *st.lveg = dlveg_new;

    // 建筑能量模型与人为热（`:1357-1378`）
    let bem = urban_bem(UrbanBemInput {
        time_step_seconds: dt,
        air_density_kg_m3: ctx.air_density_kg_m3,
        cover_fraction: [fcover[0], fcover[1], fcover[2]],
        building_height_m: ctx.hroof,
        room_max_temperature_k: ctx.t_roommax,
        room_min_temperature_k: ctx.t_roommin,
        roof_outer_temperature_previous_k: troof_nl_bef,
        sunlit_wall_outer_temperature_previous_k: twsun_nl_bef,
        shaded_wall_outer_temperature_previous_k: twsha_nl_bef,
        roof_outer_temperature_k: st.roof.column.temperature_k[nl_roof],
        sunlit_wall_outer_temperature_k: st.t_wallsun[nl_wall],
        shaded_wall_outer_temperature_k: st.t_wallsha[nl_wall],
        roof_inner_conductance_w_m2_k: tkdz_roof,
        sunlit_wall_inner_conductance_w_m2_k: sun.inner_conductance_w_m2_k,
        shaded_wall_inner_conductance_w_m2_k: sha.inner_conductance_w_m2_k,
        urban_air_temperature_k: flux.tafu,
        room_temperature_k: *st.t_room,
        roof_inner_temperature_k: *st.troof_inner,
        sunlit_wall_inner_temperature_k: *st.twsun_inner,
        shaded_wall_inner_temperature_k: *st.twsha_inner,
    })?;
    *st.t_room = bem.room_temperature_k;
    *st.troof_inner = bem.roof_inner_temperature_k;
    *st.twsun_inner = bem.sunlit_wall_inner_temperature_k;
    *st.twsha_inner = bem.shaded_wall_inner_temperature_k;
    let lucy = urban_lucy_flux(UrbanLucyFluxInput {
        time: ctx.time,
        greenwich: ctx.greenwich,
        longitude_radians: ctx.longitude_radians,
        fixed_holiday: ctx.fix_holiday,
        week_holiday: ctx.week_holiday,
        human_metabolic_profile: ctx.hum_prof,
        weekday_traffic_profile: ctx.wdh_prof,
        weekend_traffic_profile: ctx.weh_prof,
        population_density_per_km2: ctx.pop_den,
        vehicles_per_thousand: ctx.vehicle,
    })?;
    // `:1369` `((.FMA (1-flake, Fhac+Fwst+Fach, fgrnd)) + vehc) + meta`
    let bem_heat = (bem.cooling_energy_w_m2 + bem.waste_heat_w_m2) + bem.air_exchange_w_m2;
    fgrnd = (land.mul_add(bem_heat, fgrnd) + lucy.vehicle_heat) + lucy.metabolic_heat;
    out.fgrnd = fgrnd;
    *st.fhac = land * bem.cooling_energy_w_m2;
    *st.fwst = land * bem.waste_heat_w_m2;
    *st.fach = land * bem.air_exchange_w_m2;
    *st.fhah = land * bem.heating_energy_w_m2;
    *st.fahe = lucy.anthropogenic_heat;
    *st.vehc = lucy.vehicle_heat;
    *st.meta = lucy.metabolic_heat;
    out.lake = lake;
    Ok(out)
}

/// `wliq_roofsno(1)`、`wice_roofsno(1)`：第 1 个材料层（雪层之后那一层）。
fn layer1(surface: &UrbanSurfaceRef<'_>) -> (f64, f64) {
    (
        surface.column.liquid_water_kg_m2[surface.snow_layers],
        surface.column.ice_water_kg_m2[surface.snow_layers],
    )
}

fn gimp_liq_layer1(surface: &UrbanSurfaceRef<'_>) -> f64 {
    layer1(surface).0
}

fn gimp_ice_layer1(surface: &UrbanSurfaceRef<'_>) -> f64 {
    layer1(surface).1
}
