//! `CoLMMAIN_Urban`（`main/URBAN/CoLMMAIN_Urban.F90`）：城市 patch 的一步，
//! 以及它调用的 `UrbanHydrology`（`MOD_Urban_Hydrology.F90`）。
//!
//! 城市 patch 有四个带雪的面：屋顶、不透水地面、透水地面、水体（水体下还有一段"土"），
//! 外加阳/阴两面墙、行道树与建筑室内。每个面各自一根雪列（[`UrbanSurface`]）；
//! [`StandardLctSnowSoilState`] 的雪列/土柱存的是上游写进主重启的**面积加权聚合量**
//! （`wliq_soisno` 等，`CoLMMAIN_Urban.F90:1242-1344`），`t_soisno` 是透水地面的温度。
//!
//! 收缩形状对照 `CoLMMAIN_Urban.F90` 与 `MOD_Urban_Hydrology.F90` 的 GIMPLE。

use anyhow::{ensure, Context, Result};
use colm_numeric::Contract;

use crate::snow::{snow_interface_slot, snow_layer_slot, MAX_SNOW_LAYERS};
use crate::{
    add_lake_new_snow, add_new_snow, cold_start_urban_radiation, combine_snow_layers,
    compact_snow_layers, divide_snow_layers, intercept_canopy, lake_snow_water,
    saturation_specific_humidity, snow_fraction, snow_water, update_snow_age, urban_net_solar,
    urban_thermal, CalendarTime, CanopyInterceptionInput, GlacierColumn, LakeNewSnowInput,
    LakeSnowWaterFluxes, LakeSnowWaterInput, LakeSnowWaterSoil, LeafOptics, NewSnowInput,
    PrecipitationState, RuntimeSnowColumn, SnowToSoilTransfer, SnowWaterInput,
    StandardLctSnowSoilInput, StandardLctSnowSoilState, UrbanNetSolarFluxes, UrbanNetSolarInput,
    UrbanRadiationInput, UrbanRadiationState, UrbanSurfaceRef, UrbanThermalContext,
    UrbanThermalOutput, UrbanThermalState, UrbanTreeState, Water2014SnowSoilInput,
    Water2014SoilFluxes, Water2014SoilInput, Water2014SoilState,
};

const DENH2O: f64 = 1000.0;
const DENICE: f64 = 917.0;

/// 一个城市面：雪列与它下面的材料层（屋顶结构层或土层），上游下标 `1..nl`。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanSurface {
    pub snow: RuntimeSnowColumn,
    pub temperature_k: Vec<f64>,
    pub liquid_water_kg_m2: Vec<f64>,
    pub ice_water_kg_m2: Vec<f64>,
}

/// 城市 patch 的运行态（`MOD_Urban_Vars_TimeVariables` 的重启量）。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanPatchState {
    pub roof: UrbanSurface,
    pub impervious: UrbanSurface,
    pub pervious: UrbanSurface,
    /// 水体上的雪与水体下的土（`*_lakesno`）。
    pub lake_bed: UrbanSurface,
    pub t_wallsun: Vec<f64>,
    pub t_wallsha: Vec<f64>,
    /// `alburban` 的全部结果（`fwsun`/`dfwsun`/`extkd`、`alb`/`ssun`/`ssha` 与各面吸收系数）。
    pub radiation: UrbanRadiationState,
    pub lwsun: f64,
    pub lwsha: f64,
    pub lgimp: f64,
    pub lgper: f64,
    pub lveg: f64,
    pub troof_inner: f64,
    pub twsun_inner: f64,
    pub twsha_inner: f64,
    pub t_room: f64,
    pub t_roof: f64,
    pub t_wall: f64,
    pub tafu: f64,
    pub fhac: f64,
    pub fwst: f64,
    pub fach: f64,
    pub fahe: f64,
    pub fhah: f64,
    pub vehc: f64,
    pub meta: f64,
    /// 树冠感热/潜热（`fsen_urbl`/`lfevp_urbl`）：上游是初值 `spval` 的持久通量，只在有树的步里
    /// 更新；不进重启，续跑后重新从 `spval` 开始。`None` 即 `spval`。
    pub fsen_urbl: Option<f64>,
    pub lfevp_urbl: Option<f64>,
}

/// 城市 patch 的时不变量（城市常数重启与主常数重启里的城市字段）。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanSite {
    pub froof: f64,
    pub flake: f64,
    pub hroof: f64,
    pub hlr: f64,
    pub fgper: f64,
    pub fveg: f64,
    pub htop: f64,
    pub hbot: f64,
    pub em_roof: f64,
    pub em_wall: f64,
    pub em_gimp: f64,
    pub em_gper: f64,
    pub cv_roof: Vec<f64>,
    pub tk_roof: Vec<f64>,
    pub cv_wall: Vec<f64>,
    pub tk_wall: Vec<f64>,
    pub cv_gimp: Vec<f64>,
    pub tk_gimp: Vec<f64>,
    pub z_roof: Vec<f64>,
    pub dz_roof: Vec<f64>,
    pub z_wall: Vec<f64>,
    pub dz_wall: Vec<f64>,
    pub alb_roof: [[f64; 2]; 2],
    pub alb_wall: [[f64; 2]; 2],
    pub alb_gimp: [[f64; 2]; 2],
    pub alb_gper: [[f64; 2]; 2],
    pub t_roommax: f64,
    pub t_roommin: f64,
    pub pop_den: f64,
    pub vehicle: Vec<f64>,
    pub week_holiday: Vec<f64>,
    pub weh_prof: Vec<f64>,
    pub wdh_prof: Vec<f64>,
    pub hum_prof: Vec<f64>,
    pub fix_holiday: Vec<f64>,
    pub lake_depth_m: f64,
    pub latitude_radians: f64,
    pub leaf_optics: LeafOptics,
    /// 土壤格点（透水地面、不透水地面与水体下的土共用）。
    pub soil_node_depth_m: Vec<f64>,
    pub soil_layer_thickness_m: Vec<f64>,
    pub soil_interface_depth_m: Vec<f64>,
    pub campbell: bool,
    pub absolute_heights: bool,
    /// `DEF_SNOWCOVER_*` 的指数（`snowfraction` 的 `fmelt` 幂）。
    pub snow_cover_exponent: f64,
}

/// 本步的时钟与几何量。
#[derive(Debug, Clone, Copy)]
pub struct UrbanClock {
    /// 步末时刻（`idate`），LUCY 用。
    pub time: CalendarTime,
    pub greenwich: bool,
    pub longitude_radians: f64,
    /// 强迫时刻的 `coszen`（`theta = acos(max(coszen, 0.01))`）。
    pub cosine_zenith: f64,
    /// 步末的 `coszen`（末尾 `alburban` 用）。
    pub surface_cosine_zenith: f64,
}

/// 一步城市分支的诊断。
#[derive(Debug, Clone, PartialEq)]
pub struct UrbanStepOutput {
    pub precipitation: PrecipitationState,
    pub shortwave: UrbanNetSolarFluxes,
    pub thermal: UrbanThermalOutput,
    pub sabvsun: f64,
    pub qintr: f64,
    pub qdrip: f64,
    pub rsur: f64,
    pub rnof: f64,
    pub qinfl: f64,
    pub qcharge: f64,
    pub xerr: f64,
    pub zerr: f64,
    pub urb_irrig: f64,
    /// `qref/qsatl(tref)`：上游把城市的 `qref` 换成相对湿度写出。
    pub qref: f64,
    pub h2osoi: Vec<f64>,
    pub wat: f64,
    pub initial_total_water_mm: f64,
    /// 城市树冠的 `fwet_snow`（`UrbanTHERMAL` 的截留写下，下一步 `alburban` 与重启读它）。
    pub fwet_snow: f64,
}

/// Port of `CoLMMAIN_Urban`（非 SNICAR、非动态湖、非 CaMa）。
pub fn urban_step(
    input: StandardLctSnowSoilInput<'_>,
    site: &UrbanSite,
    clock: UrbanClock,
    state: &mut StandardLctSnowSoilState,
    urban: &mut UrbanPatchState,
) -> Result<UrbanStepOutput> {
    let ground = input.energy.ground_temperature;
    let dt = ground.time_step_seconds;
    let forcing = input.energy.forcing;
    let froof = site.froof;
    let fgper = site.fgper;
    let flake = site.flake;
    let fveg = site.fveg;
    let one_minus_froof = 1.0 - froof;
    let one_minus_fgper = 1.0 - fgper;
    let one_minus_flake = 1.0 - flake;
    let lai = state.energy.canopy.leaf_area_index;
    let sai = state.energy.canopy.stem_area_index;
    let sigf = state.energy.canopy.vegetation_free_fraction;

    // `:713`
    let theta = clock.cosine_zenith.max(0.01).acos();
    let shortwave = urban_net_solar(
        UrbanNetSolarInput {
            forcing: input.energy.solar.forcing,
            greenwich_time: input.energy.solar.greenwich_time,
            seconds_of_day: input.energy.solar.seconds_of_day,
            time_step_seconds: input.energy.solar.time_step_seconds,
            longitude_radians: input.energy.solar.longitude_radians,
        },
        &urban.radiation,
    )?;
    let precipitation = forcing.partition_precipitation(1, input.energy.precipitation_scheme)?;
    let prc_rain = precipitation.convective_rain_kg_m2_s;
    let prl_rain = precipitation.large_scale_rain_kg_m2_s;
    let prc_snow = precipitation.convective_snow_kg_m2_s;
    let prl_snow = precipitation.large_scale_snow_kg_m2_s;
    let forc_rain = prc_rain + prl_rain;
    let forc_snow = prc_snow + prl_snow;
    let sabv = shortwave.vegetation_absorbed_w_m2;
    // `:738` `(fveg*sabv)*(1-flake)`
    let sabvsun = (fveg * sabv) * one_minus_flake;

    // 四个面的界面深度、`scvold`、`fiold`（`:764-864`）
    let roof_zi = cumulative_interfaces(&site.dz_roof);
    let zi_wall = cumulative_interfaces(&site.dz_wall);
    for surface in [
        &mut urban.roof,
        &mut urban.impervious,
        &mut urban.pervious,
        &mut urban.lake_bed,
    ] {
        rebuild_snow_interfaces(&mut surface.snow);
        remember_ice_fraction(&mut surface.snow);
    }
    let scvold = [
        urban.roof.snow.water_equivalent_kg_m2,
        urban.impervious.snow.water_equivalent_kg_m2,
        urban.pervious.snow.water_equivalent_kg_m2,
        urban.lake_bed.snow.water_equivalent_kg_m2,
    ];
    // `:859` `w_old = sum(wliq_lakesno(snll+1:))`
    let w_old = lake_liquid(&urban.lake_bed);
    // `:867-868` `totwb = .FMA ((1-froof)*wa, fgper, .FMA (fveg, ldew, (scv + sum(wice+wliq))))`
    let total_water_before = total_water(state, fveg, one_minus_froof, fgper);

    // 截留（`:882-891`）
    let interception = intercept_canopy(
        CanopyInterceptionInput {
            convective_rain_kg_m2_s: prc_rain,
            convective_snow_kg_m2_s: prc_snow,
            large_scale_rain_kg_m2_s: prl_rain,
            large_scale_snow_kg_m2_s: prl_snow,
            leaf_temperature_k: state.energy.leaf.leaf_temperature_k,
            leaf_area_index: lai,
            stem_area_index: sai,
            sprinkler_irrigation_kg_m2_s: 0.0,
            ..input.energy.interception
        },
        &mut state.energy.leaf.canopy_water,
    )?;
    let qintr = (interception.retained_kg_m2_s * fveg) * one_minus_flake;
    let mut pgper_rain = interception.ground_rain_kg_m2_s;
    let mut pgper_snow = interception.ground_snow_kg_m2_s;
    let qdrip_gper = pgper_rain + pgper_snow;
    // `:891` `.FMA (forc_rain+forc_snow, .FNMA (1-flake, fveg, 1), (1-flake)*(fveg*qdrip_gper))`
    let qdrip = (forc_rain + forc_snow).contract(
        (-one_minus_flake).contract(fveg, 1.0),
        one_minus_flake * (fveg * qdrip_gper),
    );

    // 降水在各面的分配（`:894-920`）
    let pg_rain = prc_rain + prl_rain;
    let pg_snow = prc_snow + prl_snow;
    let fveg_gper = if fgper > 0.0 {
        fveg / (one_minus_froof * fgper)
    } else {
        0.0
    };
    let fveg_gimp = if fgper < 1.0 {
        (fveg - one_minus_froof * fgper) / (one_minus_froof * one_minus_fgper)
    } else {
        0.0
    };
    let (pgimp_rain, pgimp_snow);
    if fveg_gper <= 1.0 {
        pgper_rain = pgper_rain.contract(fveg_gper, pg_rain * (1.0 - fveg_gper));
        pgper_snow = pgper_snow.contract(fveg_gper, pg_snow * (1.0 - fveg_gper));
        pgimp_rain = pg_rain;
        pgimp_snow = pg_snow;
    } else {
        pgimp_rain = pgper_rain.contract(fveg_gimp, pg_rain * (1.0 - fveg_gimp));
        pgimp_snow = pgper_snow.contract(fveg_gimp, pg_snow * (1.0 - fveg_gimp));
    }

    // 新雪（`:926-959`）
    let new_snow = |snow: &mut RuntimeSnowColumn, surface_t: f64, snowfall: f64| {
        add_new_snow(
            NewSnowInput {
                patch_type: 1,
                time_step_seconds: dt,
                ground_temperature_k: surface_t,
                ground_snowfall_kg_m2_s: snowfall,
                new_snow_bulk_density_kg_m3: precipitation.new_snow_bulk_density_kg_m3,
                precipitation_temperature_k: precipitation.precipitation_temperature_k,
                variably_saturated_flow: input.soil_water.variably_saturated,
            },
            snow,
        )
    };
    let troof = surface_temperature(&urban.roof);
    let tgimp = surface_temperature(&urban.impervious);
    let tgper = surface_temperature(&urban.pervious);
    new_snow(&mut urban.roof.snow, troof, pg_snow)?;
    new_snow(&mut urban.impervious.snow, tgimp, pgimp_snow)?;
    new_snow(&mut urban.pervious.snow, tgper, pgper_snow)?;
    let lake = state
        .lake
        .as_mut()
        .context("an urban patch needs its water-body state")?;
    let lake_precip = add_lake_new_snow(
        LakeNewSnowInput {
            use_dynamic_lake: false,
            time_step_seconds: dt,
            rainfall_kg_m2_s: pg_rain,
            snowfall_kg_m2_s: pg_snow,
            precipitation_temperature_k: precipitation.precipitation_temperature_k,
            new_snow_bulk_density_kg_m3: precipitation.new_snow_bulk_density_kg_m3,
        },
        &mut urban.lake_bed.snow,
        &mut lake.column,
    )?;

    // ---- UrbanTHERMAL ----
    let soil_zi = &site.soil_interface_depth_m;
    let mut roof = pack(&urban.roof, &site.dz_roof, &site.z_roof, &roof_zi);
    let mut impervious = pack(
        &urban.impervious,
        &site.soil_layer_thickness_m,
        &site.soil_node_depth_m,
        soil_zi,
    );
    let mut pervious = pack(
        &urban.pervious,
        &site.soil_layer_thickness_m,
        &site.soil_node_depth_m,
        soil_zi,
    );
    let mut lake_bed = pack(
        &urban.lake_bed,
        &site.soil_layer_thickness_m,
        &site.soil_node_depth_m,
        soil_zi,
    );
    let snow_counts = [
        snow_layers(&urban.roof),
        snow_layers(&urban.impervious),
        snow_layers(&urban.pervious),
        snow_layers(&urban.lake_bed),
    ];
    let mut swe = [
        urban.roof.snow.water_equivalent_kg_m2,
        urban.impervious.snow.water_equivalent_kg_m2,
        urban.pervious.snow.water_equivalent_kg_m2,
        urban.lake_bed.snow.water_equivalent_kg_m2,
    ];
    let mut depth = [
        urban.roof.snow.depth_m,
        urban.impervious.snow.depth_m,
        urban.pervious.snow.depth_m,
        urban.lake_bed.snow.depth_m,
    ];
    let leaf = input.energy.leaf_temperature;
    let mut tree = UrbanTreeState {
        tl: state.energy.leaf.leaf_temperature_k,
        ldew: state.energy.leaf.canopy_water.total_mm,
        ldew_rain: state.energy.leaf.canopy_water.rain_mm,
        ldew_snow: state.energy.leaf.canopy_water.snow_mm,
        fwet_snow: 0.0,
    };
    let mut fwsun = urban.radiation.sunlit_wall_fraction;
    let [swe_roof, swe_gimp, swe_gper, swe_lake] = &mut swe;
    let [depth_roof, depth_gimp, depth_gper, depth_lake] = &mut depth;
    let thermal = urban_thermal(
        UrbanThermalContext {
            time_step_seconds: dt,
            latitude_radians: site.latitude_radians,
            longitude_radians: clock.longitude_radians,
            time: clock.time,
            greenwich: clock.greenwich,
            wind_height_m: input.energy.ground_flux.wind_height_m,
            temperature_height_m: input.energy.ground_flux.temperature_height_m,
            humidity_height_m: input.energy.ground_flux.humidity_height_m,
            absolute_heights: site.absolute_heights,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            air_temperature_k: forcing.air_temperature_k,
            specific_humidity: forcing.specific_humidity,
            surface_pressure_pa: forcing.surface_pressure_pa,
            air_density_kg_m3: forcing.air_density_kg_m3,
            downward_longwave_w_m2: forcing.downward_longwave_w_m2,
            po2m: leaf.oxygen_partial_pressure_pa,
            pco2m: leaf.atmospheric_co2_pa,
            shortwave: input.energy.solar.forcing,
            boundary_layer_height_m: forcing.boundary_layer_height_m,
            surface_layer_scheme: input.energy.ground_flux.surface_layer_scheme,
            zenith_angle_radians: theta,
            sabroof: shortwave.roof_absorbed_w_m2,
            sabwsun: shortwave.sunlit_wall_absorbed_w_m2,
            sabwsha: shortwave.shaded_wall_absorbed_w_m2,
            sabgimp: shortwave.impervious_absorbed_w_m2,
            sabgper: shortwave.pervious_absorbed_w_m2,
            sablake: shortwave.lake_absorbed_w_m2,
            sabv,
            par: shortwave.vegetation_par_w_m2,
            froof,
            flake,
            hroof: site.hroof,
            hlr: site.hlr,
            fgper,
            eroof: site.em_roof,
            ewall: site.em_wall,
            egimp: site.em_gimp,
            egper: site.em_gper,
            trsmx0: input.energy.root_uptake.maximum_transpiration_mm_s,
            zlnd: input.energy.ground_flux.soil_roughness_m,
            zsno: input.energy.ground_flux.snow_roughness_m,
            capr: ground.surface_temperature_factor,
            cnfac: ground.crank_nicolson_factor,
            soil_thermal_inputs: ground.soil_thermal_inputs,
            thermal_conductivity_scheme: ground.thermal_conductivity_scheme,
            soil_porosity: ground.soil_porosity,
            soil_residual_water: ground.soil_residual_water,
            soil_suction_mm: ground.soil_suction_mm,
            soil_hydraulic_model: ground.soil_hydraulic_model,
            campbell: site.campbell,
            supercool_water: ground.supercool_water,
            cv_roof: &site.cv_roof,
            tk_roof: &site.tk_roof,
            cv_wall: &site.cv_wall,
            tk_wall: &site.tk_wall,
            cv_gimp: &site.cv_gimp,
            tk_gimp: &site.tk_gimp,
            dz_wall: &site.dz_wall,
            z_wall: &site.z_wall,
            zi_wall: &zi_wall,
            lake_depth_m: site.lake_depth_m,
            dewmx: input.energy.interception.maximum_dew_mm,
            sqrtdi: leaf.inverse_sqrt_leaf_dimension_m_neg_half,
            root_fraction: input.energy.root_uptake.root_fraction,
            root_stress_scheme: input.energy.root_uptake.stress_scheme,
            biochemistry: leaf.biochemistry,
            stomata_options: leaf.options.stomata,
            wue_lambda: leaf.wue_lambda,
            vegetation_snow: leaf.options.vegetation_snow,
            lai,
            sai,
            htop: site.htop,
            hbot: site.hbot,
            fveg,
            sigf,
            extkd: urban.radiation.diffuse_extinction,
            fsno_roof: urban.roof.snow.ground_snow_fraction,
            fsno_gimp: urban.impervious.snow.ground_snow_fraction,
            fsno_gper: urban.pervious.snow.ground_snow_fraction,
            t_roommax: site.t_roommax,
            t_roommin: site.t_roommin,
            fix_holiday: &site.fix_holiday,
            week_holiday: &site.week_holiday,
            hum_prof: &site.hum_prof,
            wdh_prof: &site.wdh_prof,
            weh_prof: &site.weh_prof,
            pop_den: site.pop_den,
            vehicle: &site.vehicle,
        },
        UrbanThermalState {
            roof: UrbanSurfaceRef {
                column: &mut roof,
                snow_layers: snow_counts[0],
                snow_water_equivalent_kg_m2: swe_roof,
                snow_depth_m: depth_roof,
            },
            impervious: UrbanSurfaceRef {
                column: &mut impervious,
                snow_layers: snow_counts[1],
                snow_water_equivalent_kg_m2: swe_gimp,
                snow_depth_m: depth_gimp,
            },
            pervious: UrbanSurfaceRef {
                column: &mut pervious,
                snow_layers: snow_counts[2],
                snow_water_equivalent_kg_m2: swe_gper,
                snow_depth_m: depth_gper,
            },
            lake_bed: UrbanSurfaceRef {
                column: &mut lake_bed,
                snow_layers: snow_counts[3],
                snow_water_equivalent_kg_m2: swe_lake,
                snow_depth_m: depth_lake,
            },
            lake: &mut lake.column,
            saved_tke: &mut lake.saved_tke,
            t_wallsun: &mut urban.t_wallsun,
            t_wallsha: &mut urban.t_wallsha,
            fwsun: &mut fwsun,
            dfwsun: urban.radiation.change_in_sunlit_wall_fraction,
            lwsun: &mut urban.lwsun,
            lwsha: &mut urban.lwsha,
            lgimp: &mut urban.lgimp,
            lgper: &mut urban.lgper,
            lveg: &mut urban.lveg,
            tree: &mut tree,
            t_room: &mut urban.t_room,
            troof_inner: &mut urban.troof_inner,
            twsun_inner: &mut urban.twsun_inner,
            twsha_inner: &mut urban.twsha_inner,
            tafu: &mut urban.tafu,
            fhac: &mut urban.fhac,
            fwst: &mut urban.fwst,
            fach: &mut urban.fach,
            fahe: &mut urban.fahe,
            fhah: &mut urban.fhah,
            vehc: &mut urban.vehc,
            meta: &mut urban.meta,
        },
    )?;
    urban.radiation.sunlit_wall_fraction = fwsun;
    unpack(&mut urban.roof, &roof, swe[0], depth[0]);
    unpack(&mut urban.impervious, &impervious, swe[1], depth[1]);
    unpack(&mut urban.pervious, &pervious, swe[2], depth[2]);
    unpack(&mut urban.lake_bed, &lake_bed, swe[3], depth[3]);
    if thermal.fsen_urbl.is_some() {
        urban.fsen_urbl = thermal.fsen_urbl;
        urban.lfevp_urbl = thermal.lfevp_urbl;
    }
    urban.t_wall = thermal.twall;
    state.energy.leaf.leaf_temperature_k = tree.tl;
    state.energy.leaf.canopy_water.total_mm = tree.ldew;
    state.energy.leaf.canopy_water.rain_mm = tree.ldew_rain;
    state.energy.leaf.canopy_water.snow_mm = tree.ldew_snow;
    let mut fseng = thermal.fseng;
    let mut fgrnd = thermal.fgrnd;

    // 城市灌溉（`:1055-1063`）
    let etr_deficit = thermal.etr_deficit;
    let etrgper = if fveg > 0.0 {
        ((thermal.etr - etr_deficit) / one_minus_froof) / fgper
    } else {
        0.0
    };
    pgper_rain += (etr_deficit / one_minus_froof) / fgper;
    // `urb_irrig = etr_deficit + wst_irrig*etr_deficit`，`wst_irrig = 1` 折成 `*2`
    let urb_irrig = etr_deficit * 2.0;

    // ---- UrbanHydrology ----
    // [1] 透水地面：与土壤同一个 `WATER_2014`
    // `rootflux = rootr*etr`（`MOD_Urban_Hydrology.F90:269`）里的 `etr` 是**哑元**，
    // `CoLMMAIN_Urban.F90:1070` 传进去的实参是 `etrgper`，不是树冠总蒸腾 `etr`
    let root_flux = thermal
        .rootr
        .iter()
        .map(|root| root * etrgper)
        .collect::<Vec<_>>();
    let mut soil = Water2014SoilState {
        liquid_water_kg_m2: urban.pervious.liquid_water_kg_m2.clone(),
        ice_water_kg_m2: urban.pervious.ice_water_kg_m2.clone(),
        ..state.soil_water.clone()
    };
    let water = crate::water_2014_snow_soil_step(
        Water2014SnowSoilInput {
            snow: SnowWaterInput {
                time_step_seconds: dt,
                rainfall_kg_m2_s: pgper_rain,
                evaporation_kg_m2_s: thermal.qseva_gper,
                dew_kg_m2_s: thermal.qsdew_gper,
                sublimation_kg_m2_s: thermal.qsubl_gper,
                frost_kg_m2_s: thermal.qfros_gper,
                ..input.snow_water
            },
            soil: Water2014SoilInput {
                patch_type: 1,
                urban_run: true,
                // `UrbanHydrology` 无条件 `CALL WATER_2014`（`MOD_Urban_Hydrology.F90:271`），
                // 不看 `DEF_USE_VariablySaturatedFlow` —— 城市透水面永远是 Campbell/2014 那一支。
                variably_saturated: false,
                time_step_seconds: dt,
                fluxes: Water2014SoilFluxes {
                    ground_rain_kg_m2_s: pgper_rain,
                    snowmelt_kg_m2_s: thermal.sm_gper,
                    ground_evaporation_kg_m2_s: thermal.qseva_gper,
                    transpiration_kg_m2_s: etrgper,
                    soil_dew_kg_m2_s: thermal.qsdew_gper,
                    soil_frost_kg_m2_s: thermal.qfros_gper,
                    soil_sublimation_kg_m2_s: thermal.qsubl_gper,
                    total_ground_evaporation_kg_m2_s: thermal.qseva_gper + thermal.qsubl_gper
                        - thermal.qsdew_gper
                        - thermal.qfros_gper,
                },
                temperature_k: &urban.pervious.temperature_k,
                // `soilwater` 的 `etr*rootr(j)` 用的是本步 `eroot` 的输出，不是静态 `rootfr`
                root_fraction: &thermal.rootr,
                root_flux_mm_s: &root_flux,
                snow_layers: snow_layers(&urban.pervious),
                ..input.soil_water
            },
            split: None,
        },
        &mut urban.pervious.snow,
        &mut soil,
    )?;
    urban.pervious.liquid_water_kg_m2 = soil.liquid_water_kg_m2.clone();
    urban.pervious.ice_water_kg_m2 = soil.ice_water_kg_m2.clone();
    state.soil_water.water_table_depth_m = soil.water_table_depth_m;
    state.soil_water.aquifer_water_mm = soil.aquifer_water_mm;
    state.soil_water.surface_water_mm = soil.surface_water_mm;
    state.soil_water.matric_potential_mm = soil.matric_potential_mm.clone();
    state.soil_water.hydraulic_conductivity_mm_s = soil.hydraulic_conductivity_mm_s.clone();
    let rsur_gper = water.soil.surface_runoff_mm_s;
    let rnof_gper = water.soil.total_runoff_mm_s;

    // [2] 屋顶与不透水面（`:300-358`）
    let sealed = |surface: &mut UrbanSurface,
                  rainfall: f64,
                  melt: f64,
                  qseva: f64,
                  qsdew: f64,
                  qsubl: f64,
                  qfros: f64|
     -> Result<f64> {
        let gwat = if surface.snow.layer_count >= 0 {
            (rainfall + melt) - qseva
        } else {
            snow_water(
                SnowWaterInput {
                    time_step_seconds: dt,
                    rainfall_kg_m2_s: rainfall,
                    evaporation_kg_m2_s: qseva,
                    dew_kg_m2_s: qsdew,
                    sublimation_kg_m2_s: qsubl,
                    frost_kg_m2_s: qfros,
                    ..input.snow_water
                },
                &mut surface.snow,
            )?
            .bottom_drainage_kg_m2_s
        };
        // `:308` `.FMA (gwat, deltim, wliq(1))`
        surface.liquid_water_kg_m2[0] = gwat.contract(dt, surface.liquid_water_kg_m2[0]);
        if surface.snow.layer_count >= 0 {
            surface.liquid_water_kg_m2[0] =
                qsdew.contract(dt, surface.liquid_water_kg_m2[0]).max(0.0);
            surface.ice_water_kg_m2[0] = (qfros - qsubl)
                .contract(dt, surface.ice_water_kg_m2[0])
                .max(0.0);
        }
        let mut xs1 = surface.liquid_water_kg_m2[0] - 1.0;
        if xs1 > 0.0 {
            surface.liquid_water_kg_m2[0] = 1.0;
        } else {
            xs1 = 0.0;
        }
        Ok(xs1 / dt)
    };
    let rsur_roof = sealed(
        &mut urban.roof,
        pg_rain,
        thermal.sm_roof,
        thermal.qseva_roof,
        thermal.qsdew_roof,
        thermal.qsubl_roof,
        thermal.qfros_roof,
    )?;
    let rsur_gimp = sealed(
        &mut urban.impervious,
        pgimp_rain,
        thermal.sm_gimp,
        thermal.qseva_gimp,
        thermal.qsdew_gimp,
        thermal.qsubl_gimp,
        thermal.qfros_gimp,
    )?;

    // [3] 水体（`:364-404`）
    let mut lake_fluxes = LakeSnowWaterFluxes {
        sensible_heat_w_m2: 0.0,
        ground_heat_w_m2: 0.0,
        snow_melt_kg_m2_s: thermal.sm_lake,
    };
    let lake_snow_layers = snow_layers(&urban.lake_bed);
    let melted = thermal.imelt_lake[..lake_snow_layers]
        .iter()
        .map(|flag| *flag == 1)
        .collect::<Vec<_>>();
    let mut lake_soil = LakeSnowWaterSoil {
        thickness_m: site.soil_layer_thickness_m.clone(),
        porosity: ground.soil_porosity.to_vec(),
        liquid_water_kg_m2: urban.lake_bed.liquid_water_kg_m2.clone(),
        ice_water_kg_m2: urban.lake_bed.ice_water_kg_m2.clone(),
    };
    let lake = state.lake.as_mut().expect("checked above");
    lake_snow_water(
        LakeSnowWaterInput {
            use_dynamic_lake: false,
            time_step_seconds: dt,
            irreducible_saturation: input.snow_water.irreducible_saturation,
            impermeable_porosity: input.snow_water.impermeable_porosity,
            rainfall_kg_m2_s: lake_precip.rainfall_kg_m2_s,
            evaporation_kg_m2_s: thermal.qseva_lake,
            sublimation_kg_m2_s: thermal.qsubl_lake,
            dew_kg_m2_s: thermal.qsdew_lake,
            frost_kg_m2_s: thermal.qfros_lake,
            eastward_wind_m_s: forcing.eastward_wind_m_s,
            northward_wind_m_s: forcing.northward_wind_m_s,
            melted: &melted,
        },
        &mut urban.lake_bed.snow,
        &mut lake.column,
        &mut lake_soil,
        &mut lake_fluxes,
    )?;
    urban.lake_bed.liquid_water_kg_m2 = lake_soil.liquid_water_kg_m2;
    urban.lake_bed.ice_water_kg_m2 = lake_soil.ice_water_kg_m2;
    clear_empty_snow_slots(&mut urban.lake_bed.snow);
    let _ = w_old;
    // `:403-404` `.FMA (flake, dfseng, fseng)`
    fseng = flake.contract(lake_fluxes.sensible_heat_w_m2, fseng);
    fgrnd = flake.contract(lake_fluxes.ground_heat_w_m2, fgrnd);
    let fg = 1.0 - froof;
    // `:411` `.FMA (fgper, rsur_gper*fg, .FMA (froof, rsur_roof, (fg*rsur_gimp)*(1-fgper)))`
    let sealed_runoff = froof.contract(rsur_roof, (fg * rsur_gimp) * one_minus_fgper);
    let rsur = fgper.contract(rsur_gper * fg, sealed_runoff);
    let rnof = fgper.contract(rnof_gper * fg, sealed_runoff);

    // 雪层压实/合并/分裂（`:1108-1215`）
    let melted_of = |flags: &[i32], count: usize| {
        flags[..count]
            .iter()
            .map(|flag| *flag == 1)
            .collect::<Vec<_>>()
    };
    for (surface, flags) in [
        (&mut urban.roof, &thermal.imelt_roof),
        (&mut urban.impervious, &thermal.imelt_gimp),
        (&mut urban.pervious, &thermal.imelt_gper),
    ] {
        let count = snow_layers(surface);
        if count > 0 {
            compact_snow_layers(
                &mut surface.snow,
                dt,
                forcing.eastward_wind_m_s,
                forcing.northward_wind_m_s,
                &melted_of(flags, count),
            )?;
            let mut top = SnowToSoilTransfer {
                liquid_water_kg_m2: surface.liquid_water_kg_m2[0],
                ice_water_kg_m2: surface.ice_water_kg_m2[0],
            };
            combine_snow_layers(&mut surface.snow, &mut top)?;
            surface.liquid_water_kg_m2[0] = top.liquid_water_kg_m2;
            surface.ice_water_kg_m2[0] = top.ice_water_kg_m2;
            if surface.snow.layer_count < 0 {
                divide_snow_layers(&mut surface.snow)?;
            }
        }
        clear_empty_snow_slots(&mut surface.snow);
    }
    let troof = surface_temperature(&urban.roof);
    // `troof` 在 `CoLMMAIN_Urban` 里被赋两次：热力学之后（`:929`）与屋顶雪层合并之后（`:1141`）。history 的
    // `t_roof` 读的是后者 —— 屋顶薄雪在本步化完、雪层被并掉时，它是屋顶第一层的温度而不是雪温。
    urban.t_roof = troof;
    let tgimp = surface_temperature(&urban.impervious);
    let tgper = surface_temperature(&urban.pervious);
    let lake = state.lake.as_mut().expect("checked above");
    let tlake = if urban.lake_bed.snow.layer_count < 0 {
        surface_temperature(&urban.lake_bed)
    } else {
        lake.column.temperature_k[0]
    };

    // 聚合进主状态（`:1218-1254`）
    state.soil_temperature_k = urban.pervious.temperature_k.clone();
    aggregate_columns(state, urban, froof, fgper, one_minus_froof, one_minus_fgper);
    state.snow.water_equivalent_kg_m2 = aggregate3(
        froof,
        fgper,
        one_minus_froof,
        one_minus_fgper,
        urban.roof.snow.water_equivalent_kg_m2,
        urban.pervious.snow.water_equivalent_kg_m2,
        urban.impervious.snow.water_equivalent_kg_m2,
    );
    let total_water_after = total_water(state, fveg, one_minus_froof, fgper);
    // `:1259` `.FNMA ((((prc+prl)+urb_irrig)-fevpa)-rnof, deltim, endwb-totwb)`
    let water_balance_error_mm = (-((((forcing.convective_precipitation_kg_m2_s
        + forcing.large_scale_precipitation_kg_m2_s)
        + urb_irrig)
        - thermal.fevpa)
        - rnof))
        .contract(dt, total_water_after - total_water_before);
    let xerr = water_balance_error_mm / dt;

    // 下一步的雪盖、雪龄与反照率（`:1278-1320`）
    let z0m = thermal.z0m;
    let zlnd = input.energy.ground_flux.soil_roughness_m;
    let exponent = site.snow_cover_exponent;
    let fsno_lake = snow_fraction(
        0.0,
        0.0,
        z0m,
        zlnd,
        urban.lake_bed.snow.water_equivalent_kg_m2,
        urban.lake_bed.snow.depth_m,
        exponent,
    )?;
    urban.lake_bed.snow.ground_snow_fraction = fsno_lake.ground_snow_fraction;
    for surface in [&mut urban.roof, &mut urban.impervious] {
        surface.snow.ground_snow_fraction = snow_fraction(
            0.0,
            0.0,
            z0m,
            zlnd,
            surface.snow.water_equivalent_kg_m2,
            surface.snow.depth_m,
            exponent,
        )?
        .ground_snow_fraction;
    }
    let gper_fraction = snow_fraction(
        lai,
        sai,
        z0m,
        zlnd,
        urban.pervious.snow.water_equivalent_kg_m2,
        urban.pervious.snow.depth_m,
        exponent,
    )?;
    urban.pervious.snow.ground_snow_fraction = gper_fraction.ground_snow_fraction;
    let sigf = gper_fraction.vegetation_free_fraction;
    let new_lai = state.energy.temporal_canopy.leaf_area_index;
    let new_sai = state.energy.temporal_canopy.stem_area_index * sigf;
    for ((surface, t), old) in [
        (&mut urban.roof, troof),
        (&mut urban.impervious, tgimp),
        (&mut urban.pervious, tgper),
        (&mut urban.lake_bed, tlake),
    ]
    .into_iter()
    .zip(scvold)
    {
        if surface.snow.layer_count == 0 {
            surface.snow.age = 0.0;
        }
        surface.snow.age = update_snow_age(
            dt,
            t,
            surface.snow.water_equivalent_kg_m2,
            old,
            surface.snow.age,
        )?;
    }
    state.snow.depth_m = aggregate3(
        froof,
        fgper,
        one_minus_froof,
        one_minus_fgper,
        urban.roof.snow.depth_m,
        urban.pervious.snow.depth_m,
        urban.impervious.snow.depth_m,
    );
    state.snow.ground_snow_fraction = aggregate3(
        froof,
        fgper,
        one_minus_froof,
        one_minus_fgper,
        urban.roof.snow.ground_snow_fraction,
        urban.pervious.snow.ground_snow_fraction,
        urban.impervious.snow.ground_snow_fraction,
    );
    state.snow.age = aggregate3(
        froof,
        fgper,
        one_minus_froof,
        one_minus_fgper,
        urban.roof.snow.age,
        urban.pervious.snow.age,
        urban.impervious.snow.age,
    );
    urban.radiation = cold_start_urban_radiation(UrbanRadiationInput {
        roof_fraction: froof,
        pervious_ground_fraction: fgper,
        water_fraction: flake,
        building_height_to_length: site.hlr,
        roof_height_m: site.hroof,
        roof_albedo: site.alb_roof,
        wall_albedo: site.alb_wall,
        impervious_albedo: site.alb_gimp,
        pervious_albedo: site.alb_gper,
        leaf_optics: site.leaf_optics,
        vegetation_fraction: fveg,
        vegetation_center_height_m: (site.htop + site.hbot) / 2.0,
        lai: new_lai,
        sai: new_sai,
        wet_snow_fraction: tree.fwet_snow,
        vegetation_snow: leaf.options.vegetation_snow,
        cosine_zenith: clock.surface_cosine_zenith,
        previous_sunlit_wall_fraction: urban.radiation.sunlit_wall_fraction,
        lake_temperature_k: tlake,
        roof_snow_fraction: urban.roof.snow.ground_snow_fraction,
        impervious_snow_fraction: urban.impervious.snow.ground_snow_fraction,
        pervious_snow_fraction: urban.pervious.snow.ground_snow_fraction,
        lake_snow_fraction: urban.lake_bed.snow.ground_snow_fraction,
        roof_snow_water_mm: urban.roof.snow.water_equivalent_kg_m2,
        impervious_snow_water_mm: urban.impervious.snow.water_equivalent_kg_m2,
        pervious_snow_water_mm: urban.pervious.snow.water_equivalent_kg_m2,
        lake_snow_water_mm: urban.lake_bed.snow.water_equivalent_kg_m2,
        roof_snow_age: urban.roof.snow.age,
        impervious_snow_age: urban.impervious.snow.age,
        pervious_snow_age: urban.pervious.snow.age,
        lake_snow_age: urban.lake_bed.snow.age,
    })?;
    state.energy.canopy.leaf_area_index = new_lai;
    state.energy.canopy.stem_area_index = new_sai;
    state.energy.canopy.vegetation_free_fraction = sigf;
    let lake = state.lake.as_mut().expect("checked above");
    lake.ground_temperature_k = thermal.t_grnd;

    // 诊断（`:1322-1352`）
    let h2osoi = state
        .soil_water
        .liquid_water_kg_m2
        .iter()
        .zip(&state.soil_water.ice_water_kg_m2)
        .zip(&site.soil_layer_thickness_m)
        .map(|((liquid, ice), dz)| liquid / (dz * DENH2O) + ice / (dz * DENICE))
        .collect::<Vec<_>>();
    let wat = total_water(state, fveg, one_minus_froof, fgper);
    aggregate_snow_geometry(
        state,
        urban,
        froof,
        fgper,
        one_minus_froof,
        one_minus_fgper,
        flake,
    );
    let qsat_tref = saturation_specific_humidity(thermal.tref, forcing.surface_pressure_pa)?;
    let qref = thermal.qref / qsat_tref.specific_humidity;
    ensure!(xerr.is_finite(), "the urban water balance is not finite");
    Ok(UrbanStepOutput {
        precipitation,
        shortwave,
        sabvsun,
        qintr,
        qdrip,
        rsur,
        rnof,
        qinfl: water.soil.infiltration_mm_s,
        qcharge: water.soil.recharge_mm_s,
        xerr,
        zerr: thermal.errore,
        urb_irrig,
        qref,
        h2osoi,
        wat,
        initial_total_water_mm: total_water_before,
        fwet_snow: tree.fwet_snow,
        thermal: UrbanThermalOutput {
            fseng,
            fgrnd,
            ..thermal
        },
    })
}

fn snow_layers(surface: &UrbanSurface) -> usize {
    surface.snow.layer_count.unsigned_abs() as usize
}

fn surface_temperature(surface: &UrbanSurface) -> f64 {
    if surface.snow.layer_count < 0 {
        surface.snow.temperature_k[snow_layer_slot(surface.snow.layer_count + 1)]
    } else {
        surface.temperature_k[0]
    }
}

/// `zi(0) = 0`、`zi(j) = zi(j-1) + dz(j)`，返回 `zi(0..nl)`。
fn cumulative_interfaces(thickness: &[f64]) -> Vec<f64> {
    let mut interfaces = Vec::with_capacity(thickness.len() + 1);
    interfaces.push(0.0);
    for dz in thickness {
        let last = *interfaces.last().expect("non-empty");
        interfaces.push(last + dz);
    }
    interfaces
}

/// `zi(0) = 0`、`zi(j) = zi(j+1) - dz(j+1)`（`:777-782`）。
fn rebuild_snow_interfaces(snow: &mut RuntimeSnowColumn) {
    snow.interface_depth_m[snow_interface_slot(0)] = 0.0;
    let mut index = -1;
    while index >= snow.layer_count {
        let above = snow.interface_depth_m[snow_interface_slot(index + 1)];
        snow.interface_depth_m[snow_interface_slot(index)] =
            above - snow.thickness_m[snow_layer_slot(index + 1)];
        index -= 1;
    }
}

fn remember_ice_fraction(snow: &mut RuntimeSnowColumn) {
    for slot in 0..MAX_SNOW_LAYERS {
        snow.previous_ice_fraction[slot] = 0.0;
    }
    for index in snow.layer_count + 1..=0 {
        let slot = snow_layer_slot(index);
        snow.previous_ice_fraction[slot] = snow.ice_water_kg_m2[slot]
            / (snow.liquid_water_kg_m2[slot] + snow.ice_water_kg_m2[slot]);
    }
}

fn clear_empty_snow_slots(snow: &mut RuntimeSnowColumn) {
    for index in -(MAX_SNOW_LAYERS as i32) + 1..=snow.layer_count {
        let slot = snow_layer_slot(index);
        snow.ice_water_kg_m2[slot] = 0.0;
        snow.liquid_water_kg_m2[slot] = 0.0;
        snow.temperature_k[slot] = 0.0;
        snow.node_depth_m[slot] = 0.0;
        snow.thickness_m[slot] = 0.0;
    }
}

/// `sum(wliq_lakesno(snll+1:))`：水体雪层与其下土层的液水。
fn lake_liquid(surface: &UrbanSurface) -> f64 {
    let mut sum = 0.0;
    for index in surface.snow.layer_count + 1..=0 {
        sum += surface.snow.liquid_water_kg_m2[snow_layer_slot(index)];
    }
    for value in &surface.liquid_water_kg_m2 {
        sum += value;
    }
    sum
}

/// `totwb`/`endwb`/`wat`：`.FMA ((1-froof)*wa, fgper, .FMA (fveg, ldew, scv + sum(wice+wliq)))`。
fn total_water(
    state: &StandardLctSnowSoilState,
    fveg: f64,
    one_minus_froof: f64,
    fgper: f64,
) -> f64 {
    let sum = state
        .soil_water
        .ice_water_kg_m2
        .iter()
        .zip(&state.soil_water.liquid_water_kg_m2)
        .fold(0.0, |acc, (ice, liquid)| (ice + liquid) + acc);
    (one_minus_froof * state.soil_water.aquifer_water_mm).contract(
        fgper,
        fveg.contract(
            state.energy.leaf.canopy_water.total_mm,
            state.snow.water_equivalent_kg_m2 + sum,
        ),
    )
}

/// `x_roof*froof + x_gper*(1-froof)*fgper + x_gimp*(1-froof)*(1-fgper)`：
/// `.FMA ((1-froof)*x_gimp, 1-fgper, .FMA (froof, x_roof, fgper*((1-froof)*x_gper)))`（`:1254`）。
fn aggregate3(
    froof: f64,
    fgper: f64,
    one_minus_froof: f64,
    one_minus_fgper: f64,
    roof: f64,
    pervious: f64,
    impervious: f64,
) -> f64 {
    (one_minus_froof * impervious).contract(
        one_minus_fgper,
        froof.contract(roof, fgper * (one_minus_froof * pervious)),
    )
}

/// `wliq_soisno`/`wice_soisno` 的面积加权（`:1242-1251`）：屋顶只到第 1 层，
/// 透水地面全柱，不透水地面也只到第 1 层（`(:1)` 包括全部雪槽）。
fn aggregate_columns(
    state: &mut StandardLctSnowSoilState,
    urban: &UrbanPatchState,
    froof: f64,
    fgper: f64,
    one_minus_froof: f64,
    one_minus_fgper: f64,
) {
    let snow = &mut state.snow;
    for slot in 0..MAX_SNOW_LAYERS {
        for (target, roof, gper, gimp) in [
            (
                &mut snow.liquid_water_kg_m2[slot],
                urban.roof.snow.liquid_water_kg_m2[slot],
                urban.pervious.snow.liquid_water_kg_m2[slot],
                urban.impervious.snow.liquid_water_kg_m2[slot],
            ),
            (
                &mut snow.ice_water_kg_m2[slot],
                urban.roof.snow.ice_water_kg_m2[slot],
                urban.pervious.snow.ice_water_kg_m2[slot],
                urban.impervious.snow.ice_water_kg_m2[slot],
            ),
        ] {
            let acc = fgper.contract(one_minus_froof * gper, froof * roof);
            *target = (one_minus_froof * gimp).contract(one_minus_fgper, acc);
        }
        snow.temperature_k[slot] = urban.pervious.snow.temperature_k[slot];
    }
    let soil = &mut state.soil_water;
    for layer in 0..soil.liquid_water_kg_m2.len() {
        let pairs = [
            (
                &mut soil.liquid_water_kg_m2[layer],
                &urban.roof.liquid_water_kg_m2,
                &urban.pervious.liquid_water_kg_m2,
                &urban.impervious.liquid_water_kg_m2,
            ),
            (
                &mut soil.ice_water_kg_m2[layer],
                &urban.roof.ice_water_kg_m2,
                &urban.pervious.ice_water_kg_m2,
                &urban.impervious.ice_water_kg_m2,
            ),
        ];
        for (target, roof, gper, gimp) in pairs {
            let mut acc = if layer == 0 { froof * roof[0] } else { 0.0 };
            acc = fgper.contract(one_minus_froof * gper[layer], acc);
            if layer == 0 {
                acc = (one_minus_froof * gimp[0]).contract(one_minus_fgper, acc);
            }
            *target = acc;
        }
    }
}

/// `z_sno`/`dz_sno` 的聚合（`:1336-1344`），最后与水体按面积混合。
#[allow(clippy::too_many_arguments)]
fn aggregate_snow_geometry(
    state: &mut StandardLctSnowSoilState,
    urban: &UrbanPatchState,
    froof: f64,
    fgper: f64,
    one_minus_froof: f64,
    one_minus_fgper: f64,
    flake: f64,
) {
    let snow = &mut state.snow;
    for slot in 0..MAX_SNOW_LAYERS {
        for (target, roof, gper, gimp, lake) in [
            (
                &mut snow.node_depth_m[slot],
                urban.roof.snow.node_depth_m[slot],
                urban.pervious.snow.node_depth_m[slot],
                urban.impervious.snow.node_depth_m[slot],
                urban.lake_bed.snow.node_depth_m[slot],
            ),
            (
                &mut snow.thickness_m[slot],
                urban.roof.snow.thickness_m[slot],
                urban.pervious.snow.thickness_m[slot],
                urban.impervious.snow.thickness_m[slot],
                urban.lake_bed.snow.thickness_m[slot],
            ),
        ] {
            let mut acc = froof * roof;
            acc = fgper.contract(one_minus_froof * gper, acc);
            acc = (one_minus_froof * gimp).contract(one_minus_fgper, acc);
            *target = (1.0 - flake).contract(acc, flake * lake);
        }
    }
}

/// 把雪列与材料层拼成雪层在前的打包列。
fn pack(surface: &UrbanSurface, dz: &[f64], z: &[f64], zi: &[f64]) -> GlacierColumn {
    let snow = &surface.snow;
    let mut column = GlacierColumn {
        thickness_m: Vec::new(),
        node_depth_m: Vec::new(),
        interface_depth_m: Vec::new(),
        temperature_k: Vec::new(),
        liquid_water_kg_m2: Vec::new(),
        ice_water_kg_m2: Vec::new(),
    };
    for index in snow.layer_count..=0 {
        column
            .interface_depth_m
            .push(snow.interface_depth_m[snow_interface_slot(index)]);
    }
    for index in snow.layer_count + 1..=0 {
        let slot = snow_layer_slot(index);
        column.thickness_m.push(snow.thickness_m[slot]);
        column.node_depth_m.push(snow.node_depth_m[slot]);
        column.temperature_k.push(snow.temperature_k[slot]);
        column
            .liquid_water_kg_m2
            .push(snow.liquid_water_kg_m2[slot]);
        column.ice_water_kg_m2.push(snow.ice_water_kg_m2[slot]);
    }
    column.thickness_m.extend_from_slice(dz);
    column.node_depth_m.extend_from_slice(z);
    column.interface_depth_m.extend_from_slice(&zi[1..]);
    column
        .temperature_k
        .extend_from_slice(&surface.temperature_k);
    column
        .liquid_water_kg_m2
        .extend_from_slice(&surface.liquid_water_kg_m2);
    column
        .ice_water_kg_m2
        .extend_from_slice(&surface.ice_water_kg_m2);
    column
}

fn unpack(surface: &mut UrbanSurface, column: &GlacierColumn, swe: f64, depth: f64) {
    let snow_layers = snow_layers(surface);
    for (relative, index) in (surface.snow.layer_count + 1..=0).enumerate() {
        let slot = snow_layer_slot(index);
        surface.snow.temperature_k[slot] = column.temperature_k[relative];
        surface.snow.liquid_water_kg_m2[slot] = column.liquid_water_kg_m2[relative];
        surface.snow.ice_water_kg_m2[slot] = column.ice_water_kg_m2[relative];
    }
    surface.temperature_k = column.temperature_k[snow_layers..].to_vec();
    surface.liquid_water_kg_m2 = column.liquid_water_kg_m2[snow_layers..].to_vec();
    surface.ice_water_kg_m2 = column.ice_water_kg_m2[snow_layers..].to_vec();
    surface.snow.water_equivalent_kg_m2 = swe;
    surface.snow.depth_m = depth;
}

#[cfg(test)]
#[path = "urban_step_tests.rs"]
mod urban_step_tests;
