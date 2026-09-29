//! 算例 namelist → [`LandPhysicsParameters`] 的映射。
//!
//! 装配层刻意把物理参数留给调用方显式传入（见 `assembly.rs` 的模块注释），
//! 于是"从哪读"这件事一直没有落地：调用方要么照抄测试夹具，要么凭字段名猜。
//! 这一步把每个字段的**出处**写死在这里，出处不明的宁可报错也不给默认值。
//!
//! 三条取值纪律：
//!
//! 1. **缺省来自 `colm-schema`（即 `MOD_Namelist.F90` 的声明值），不来自这里的
//!    `unwrap_or`。** 两处各写一份默认值，改了一处另一处不会响。
//! 2. **不是 namelist 字段的量不在这里。** `hvap` 是 `MOD_Const_Physical` 的常数，
//!    `emg` 由雪水当量与 patch 类型逐步推出，WUE 的基准 `lambda` 来自地类表 ——
//!    它们混进这张表就会变成"看起来可调、实际是装配期定死"的参数。
//! 3. **上游有、本仓库没移植的分支一律报错。** 目前两处：`DEF_Runoff_SCHEME=1`
//!    （VIC 产流）与 `DEF_USE_IRRIGATION`（喷灌）。

use anyhow::{bail, ensure, Context, Result};
use colm_core::{
    HydraulicModel, LandCoverScheme, ObservationHeightMode, PlantHydraulicOverrides,
    PlantHydraulicParameters, PrecipitationPhaseScheme, RootFractionScheme, StomataOptions,
    SurfaceLayerScheme, ThermalConductivityScheme,
};
use colm_namelist::{Document, Value};
use colm_schema::{find, Default as SchemaDefault};

use crate::assembly::{LandPhysicsParameters, StandardLctRunoffScheme};

/// `MOD_Const_Physical.F90:17` 的 `hvap`。
///
/// 是**常数**，不随温度变；它留在这里而不是每步绑定里，与上游一致。
/// 写成 `LandPhysicsParameters` 的字段只是因为内核接口收它。
const VAPORIZATION_HEAT_J_KG: f64 = 2.5104e6;

/// `MOD_Const_LC.F90:717` 的 `ROOTFR_SCHEME`。
///
/// 它是**模块私有常数**，不是 namelist 字段，所以我们只能钉住这个取值而不是
/// 去读它。上游那一版是 `1`（Schenk & Jackson）。改了上游这个数就得同步改这里 ——
/// 这也是为什么它单独写成一个常量而不是散在 match 里。
const ROOT_FRACTION_SCHEME: RootFractionScheme = RootFractionScheme::SchenkJackson;

/// 把一份已解析的算例 namelist 映射成装配层要的物理参数。
///
/// `land_cover_scheme` 由调用方给：它**不是** namelist 决定的，而是内核编译期的
/// `LULC_IGBP` / `LULC_USGS`。`MOD_Namelist.F90:163` 明说 `DEF_USE_USGS`/`DEF_USE_IGBP`
/// 只是那个选择的只读镜像 —— 从 namelist 读它会在默认算例上拿到两个 `.false.`，
/// 然后只能靠猜挑一个。
/// `forc_hgt_u/t/q` 三个参考高度。它们有**三级**来源，所以不在本函数里解析。
///
/// 见 [`PointRuntimeConfig::wind_height_m`]：文件优先、其次 forcing namelist、
/// 最后 schema 声明默认值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ObservationHeights {
    pub wind_m: f64,
    pub temperature_m: f64,
    pub humidity_m: f64,
}

pub fn land_physics_parameters(
    document: &Document,
    land_cover_scheme: LandCoverScheme,
    heights: ObservationHeights,
) -> Result<LandPhysicsParameters> {
    let timestep_seconds = real(document, "DEF_simulation_time%timestep")?;
    if timestep_seconds <= 0.0 {
        bail!("DEF_simulation_time%timestep must be positive, got {timestep_seconds}");
    }
    let runoff_scheme = runoff_scheme(integer(document, "DEF_Runoff_SCHEME")?)?;
    // `DEF_SPLIT_SOILSNOW`：能量侧（比湿、`t_grnd`、凝结拆分、`fgrnd`）与水分侧
    // （`snowwater` 只拿雪面那份、土面走 `pg_rain*(1-fsno) - qseva_soil`）都已接上。
    // `DEF_URBAN_RUN`（`MOD_Namelist.F90:2248-2257`）：打开城市模型时上游把 WUEST、
    // SUPERCOOL_WATER、PLANTHYDRAULICS、OZONESTRESS、SPLIT_SOILSNOW 一律关掉。
    let urban_run = logical(document, "DEF_URBAN_RUN")?;
    let split_soil_snow = logical(document, "DEF_SPLIT_SOILSNOW")? && !urban_run;
    // `DEF_Interception_scheme`：`main/` 的 `LEAF_interception_wrap` 只接受 1（CoLM2014）
    // 与 8（CoLM2024，CoLM2014 外加按冠层结构算的雨容量），其余档位在非扩展构建里
    // `CALL abort`（`MOD_LeafInterception.F90:614-617`）。这里照样拒绝。
    let interception_scheme = integer(document, "DEF_Interception_scheme")?;
    let colm2024_interception = match interception_scheme {
        1 => false,
        8 => true,
        other => bail!(
            "DEF_Interception_scheme = {other}: the kernel (main/, non-extended build) supports only \
             schemes 1 (CoLM2014) and 8 (CoLM2024) and aborts on any other value"
        ),
    };
    // `DEF_Optimize_Baseflow` 不在这里：它不改物理参数，而是在主循环里逐年改写
    // `scale_baseflow`，由 `colm-rs` 装一个 `BaseflowOptimizer` 给运行时（`MOD_Opt_Baseflow`）。
    // `DEF_USE_Dynamic_Wetland`：湿地按土壤地面算地面湿度（`MOD_Thermal.F90:601-602`），VSF 下走土壤水分支
    // （`MOD_SoilSnowHydrology.F90:946-947`、`:1384-1392`）。
    let dynamic_wetland = logical(document, "DEF_USE_Dynamic_Wetland")?;
    if logical(document, "DEF_USE_SNICAR")? {
        // `DEF_USE_SNICAR` 与 `DEF_SPLIT_SOILSNOW` 同属"最坏的一类"：它此前**根本
        // 没被读过**，而 `assembly.rs` 把 `snow_layer_absorption_w_m2` 硬写成 `None`
        // —— 也就是说 `ground_temperature.rs`/`phase_change.rs` 里那套 SNICAR 分支
        // 永远不会被选中，写 `.true.` 的算例会**静默按标准雪算完**
        // （雪粒径增长、分层吸收、融化能量都会与上游的 `SNICAR_AD_RT` 不同）。
        //
        // 冷启动那边（`colm-init`）**不拦**：它确实能按 SNICAR 生成重启，
        // 只是运行期还没有把那条支路接上；拦在运行期才是"接不上就不跑"。
        bail!(
            "DEF_USE_SNICAR is on, but the Rust runtime assembles only the standard snow \
             branch: `assembly.rs` pins `snow_layer_absorption_w_m2` to None and the SNICAR \
             cold start is not carried into the time loop, so the case would silently run \
             with the non-SNICAR snow albedo and layer absorption"
        );
    }
    // 上游 `MOD_Namelist.F90:1932-1944`：`DEF_USE_LCT`/`DEF_USE_PFT`/`DEF_USE_PC`
    // **恰好一个**必须为真，否则 `CoLM_stop`；三者的声明默认值是
    // `.true.`/`.false.`/`.false.`，也就是默认走 LCT。
    //
    // 本仓库只实现了 LCT 这一条编排（`assembly.rs:1379` 的注释、`assembly.rs`
    // 只装配植被 patch）。**不读这三个开关会让一个选 PFT/PC 的算例静默按 LCT
    // 算完** —— 那是最坏的一类分支不匹配：算式对、结构错、还不报错。
    // 它们不是默认打开的分支，所以按本文件的纪律 #3 直接报错，
    // 而不是进 `unported_branches` 那张"默认配置会撞上"的表。
    let subgrid = [
        ("DEF_USE_LCT", logical(document, "DEF_USE_LCT")?),
        ("DEF_USE_PFT", logical(document, "DEF_USE_PFT")?),
        ("DEF_USE_PC", logical(document, "DEF_USE_PC")?),
    ];
    let selected: Vec<&str> = subgrid
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect();
    if selected.len() != 1 {
        bail!(
            "exactly one of DEF_USE_LCT / DEF_USE_PFT / DEF_USE_PC must be .true. \
             (MOD_Namelist.F90:1942 stops the model otherwise), got {selected:?}"
        );
    }
    // PFT 与 PC 都已移植：PFT 逐 PFT 解单冠层，PC 走 `LeafTemperaturePC` 与 `ThreeDCanopy`；
    // 二者都在聚合后复用 LCT 的地面与水分路径。
    // `DEF_USE_BGC`：`CoLMDRIVER.F90:238` 每步在 `CoLMMAIN` 之后调 `bgc_driver`（`crate::bgc_step`）。
    // 上游 `MOD_Namelist.F90:1900` 要求 PFT 或 PC 子网格；BGC 关闭时 `LAIFEEDBACK`/`SASU`/
    // `DiagMatrix`/`NITRIF`/`FIRE` 被强制关掉，`CROP` 关闭时 `FERT`/`CNSOYFIXN` 同理。
    let bgc = if logical(document, "DEF_USE_BGC")? {
        ensure!(
            selected[0] != "DEF_USE_LCT",
            "DEF_USE_BGC requires DEF_USE_PFT or DEF_USE_PC (MOD_Namelist.F90:1900 stops the model)"
        );
        let switches = colm_core::bgc_driver::BgcSwitches {
            // `DEF_USE_CROP` 是内核宏的只读映射、namelist 里没有：由 `colm-rs --crop` 置位（见那里）。
            crop: false,
            nitrif: logical(document, "DEF_USE_NITRIF")?,
            fire: logical(document, "DEF_USE_FIRE")?,
            sasu: logical(document, "DEF_USE_SASU")?,
            diag_matrix: logical(document, "DEF_USE_DiagMatrix")?,
            cnsoyfixn: false,
            fert: false,
            irrigation: false,
            laifeedback: logical(document, "DEF_USE_LAIFEEDBACK")?,
            nostressnitrogen: logical(document, "DEF_USE_NOSTRESSNITROGEN")?,
            campbell: logical(document, "DEF_USE_Campbell_SOIL_MODEL")?,
            rstfac: scheme_index(document, "DEF_RSTFAC", 1, 2)?,
        };
        Some(switches)
    } else {
        None
    };
    let use_pc = selected[0] == "DEF_USE_PC";
    let use_pft = selected[0] == "DEF_USE_PFT" || use_pc;

    // 上游 `MOD_Namelist.F90:1767-1772`：选了 van Genuchten 就把
    // `DEF_USE_VariablySaturatedFlow` 强制置真。它的声明默认值也是真，所以**默认
    // 配置跑的是 VSF**，经典 Richards 路径要显式关掉才走得到。
    let campbell = logical(document, "DEF_USE_Campbell_SOIL_MODEL")?;
    let variably_saturated_flow = logical(document, "DEF_USE_VariablySaturatedFlow")? || !campbell;
    // `DEF_USE_PLANTHYDRAULICS` 的声明默认值是 `.true.`，所以**默认配置开着 PHS**，
    // 本仓库的 standard-LCT 分支则是硬关的。调用方要能看出这个不匹配。
    let plant_hydraulics = logical(document, "DEF_USE_PLANTHYDRAULICS")? && !urban_run;
    // `DEF_VEG_SNOW` 的声明默认值也是 `.true.`。**这一支已经移植并可运行**：
    // 冠层雨/雪分开记（`CanopyWater::rain_mm`/`snow_mm`）、`fwet_rain`/`fwet_snow`、
    // 以及 `radiation.rs`/`high_res_radiation.rs` 里按月-雪的两套反照率，
    // 四处的分支逻辑都在，装配也把本字段透传下去（`assembly.rs` 三处）。
    // 实测（Jan 1-3、`DEF_VEG_SNOW = .true.`）与 Fortran 的一致性
    // 与已验收的 `DEF_VEG_SNOW = .false.` 配置**同一水平**：`f_scv`/`f_snowdp`/`f_fsno`
    // 两边恒为 0，`f_t_grnd` 0.075 K、`f_tleaf` 0.127 K、`f_etr` 1.0e-8。
    let vegetation_snow = logical(document, "DEF_VEG_SNOW")?;
    // 植物水力的七个常数。逐项对 `MOD_Namelist.F90:628-634` 的声明默认值 ——
    // 它们与 `PlantHydraulicParameters::default()` 相同，`physics_tests.rs` 有一条
    // 空算例断言把这件事钉在 schema 上（纪律 #1：缺省来自 schema，不在这里写第二份）。
    // 上游只在 `MOD_PlantHydraulic.F90:162-200` 用到这七个，用法见
    // `plant_hydraulics.rs`：`CROOT_LATERAL_LENGTH` 是侧根平均长度、
    // `K_AXS` 是轴向导度系数、`FROOT_CARBON`/`ROOT_DENSITY`/`ROOT_RADIUS`
    // 一起定细根长度密度、`FROOT_LEAF` 是细根-叶面积分配、`KRMAX` 是单位长度
    // 单位面积的最大径向导度。
    // `DEF_LC_*` 的九个植物水力覆盖。schema 的声明默认值是 `-1.e36`，就是上游的
    // `LC_OVERRIDE_UNSET` —— 读到它表示 namelist 没写这一列，用地类表的值。
    //
    // 用 `==` 比而不是容差比：两边都是同一个十进制字面量解析出来的 `f64`，
    // 位模式必然一致；给这样一个"哨兵值"加容差反而会把一个刚好接近 -1e36 的
    // 真实覆盖误判成未设置。
    // 其余 `DEF_LC_*` 地类表逐列覆盖（`apply_lc_scalar_overrides`）：上游只在单点、`DEF_USE_LCT` 时生效，
    // PFT/PC 下读了也不用。`DEF_LC_C3C4` 是整数列，哨兵是 `-1`，只许 0/1（`MOD_Namelist.F90:2070`）。
    let mut land_class_overrides = colm_core::LandClassOverrides::default();
    if selected[0] == "DEF_USE_LCT" {
        for name in colm_core::LandClassOverrides::REAL_NAMES {
            if let Some(value) = land_cover_override(document, name)? {
                land_class_overrides.set_real(name, value)?;
            }
        }
        land_class_overrides.c3c4 = match integer(document, "DEF_LC_C3C4")? {
            -1 => None,
            value @ (0 | 1) => Some(i32::try_from(value)?),
            other => bail!("DEF_LC_C3C4 must be -1, 0, or 1, got {other}"),
        };
        // 上游只覆盖本站地类 `SITE_landtype`；城市单点里还有别的地类的 patch，Rust 没有按地类区分，拒绝。
        ensure!(
            land_class_overrides == colm_core::LandClassOverrides::default() || !urban_run,
            "DEF_LC_* land-class overrides together with DEF_URBAN_RUN are not verified: upstream \
             overrides only SITE_landtype, and an urban site mixes land classes"
        );
    }
    // 声明里有、本仓库**不读**的物理开关（默认值取自 schema 的声明）。判据是
    // "算例把它设成了非声明默认值" —— 只写默认值不算设过（那正是黄金窗口验证过的路径）。
    // 这张表来自 `oracle/scripts/audit_namelist_reads.sh` 的快照
    // （`oracle/nml-unread.snapshot`）：里面还有一大批本来就与 SinglePoint/LCT 无关的
    // 开关（CaMa 河网、湿地、DA、TRACER、crop/urban、以及调试/输出类），
    // 那些**不列**在这里 —— 它们在 Golden 内核里同样不生效，忽略是对的。
    // 列的是"设了就会改变物理、而本仓库不会照做"的那些，其中 `DEF_TUNING_SIMPLE_VIC_*`
    // 尤其要紧：黄金配置的产流方案正是 SimpleVIC。
    const UNREAD_PHYSICS_SWITCHES: [&str; 14] = [
        "DEF_TUNING_CSOILC",
        "DEF_TUNING_SMPMAX",
        "DEF_TUNING_SMPMAX_HR",
        "DEF_TUNING_SMPMIN_HR",
        "DEF_TUNING_SIMPLE_VIC_DS",
        "DEF_TUNING_SIMPLE_VIC_WS",
        "DEF_SOIL_REFL_SCHEME",
        "DEF_TOPMOD_method",
        "DEF_USE_BEDROCK",
        "DEF_USE_SOILPAR_UPS_FIT",
        "DEF_LANDONLY",
        "DEF_USE_DOMINANT_PATCHTYPE",
        "DEF_LAI_START_YEAR",
        "DEF_LAI_END_YEAR",
    ];
    for name in UNREAD_PHYSICS_SWITCHES {
        if sets_non_default(document, name) {
            bail!(
                "{name} is set to a non-default value, but the Rust runtime never reads it, so \
                 the case would silently run with the runtime's own hard-coded behaviour \
                 instead (see oracle/nml-unread.snapshot)"
            );
        }
    }
    let plant_hydraulic_overrides = PlantHydraulicOverrides {
        maximum_sunlit_leaf_conductance: land_cover_override(document, "DEF_LC_KMAX_SUN")?,
        maximum_shaded_leaf_conductance: land_cover_override(document, "DEF_LC_KMAX_SHA")?,
        maximum_xylem_conductance: land_cover_override(document, "DEF_LC_KMAX_XYL")?,
        maximum_root_conductance: land_cover_override(document, "DEF_LC_KMAX_ROOT")?,
        sunlit_leaf_psi50_mm: land_cover_override(document, "DEF_LC_PSI50_SUN")?,
        shaded_leaf_psi50_mm: land_cover_override(document, "DEF_LC_PSI50_SHA")?,
        xylem_psi50_mm: land_cover_override(document, "DEF_LC_PSI50_XYL")?,
        root_psi50_mm: land_cover_override(document, "DEF_LC_PSI50_ROOT")?,
        vulnerability_shape: land_cover_override(document, "DEF_LC_CK")?,
    };
    let plant_hydraulic_parameters = PlantHydraulicParameters {
        coarse_root_lateral_length_m: real(document, "DEF_PH_CROOT_LATERAL_LENGTH")?,
        axial_root_conductivity: real(document, "DEF_PH_K_AXS")?,
        fine_root_carbon_g_c_m2: real(document, "DEF_PH_FROOT_CARBON")?,
        fine_root_radius_m: real(document, "DEF_PH_ROOT_RADIUS")?,
        root_tissue_density_g_m3: real(document, "DEF_PH_ROOT_DENSITY")?,
        fine_root_to_leaf_area: real(document, "DEF_PH_FROOT_LEAF")?,
        maximum_radial_root_conductance: real(document, "DEF_PH_KRMAX")?,
    };
    // `DEF_USE_IRRIGATION`：上游 CROP 关闭时强制关掉（`MOD_Namelist.F90:1973`）；是否 CROP 内核只有
    // `colm-rs --crop` 知道，所以这里先读，由调用方在非 CROP 时清掉。
    // `DEF_USE_Dynamic_Lake`：湖深随水量变、`dz_lake` 进时间重启、`wdsrf` 记湖水。上游非 VSF 时强制关掉
    // （`MOD_Namelist.F90:2339-2342`，单点没有 CATCHMENT）。
    let dynamic_lake = logical(document, "DEF_USE_Dynamic_Lake")? && variably_saturated_flow;
    ensure!(
        !(dynamic_lake && urban_run),
        "DEF_USE_Dynamic_Lake together with DEF_URBAN_RUN is not verified: the urban water body \
         calls the lake snow routines with the dynamic switch, which the Rust urban step does not pass"
    );
    let irrigation = if logical(document, "DEF_USE_IRRIGATION")? {
        Some(colm_core::IrrigationSettings {
            start_seconds: real(document, "DEF_TUNING_IRRIGATION_START_SEC")?,
            duration_seconds: real(document, "DEF_TUNING_IRRIGATION_DURATION_SEC")?,
            max_depth_m: real(document, "DEF_TUNING_IRRIGATION_MAX_DEPTH")?,
            threshold_fraction: real(document, "DEF_TUNING_IRRIGATION_THRESHOLD_FRACTION")?,
            supply_fraction: real(document, "DEF_TUNING_IRRIGATION_SUPPLY_FRACTION")?,
            min_crop_phase: real(document, "DEF_TUNING_IRRIGATION_MIN_CPHASE")?,
            max_crop_phase: real(document, "DEF_TUNING_IRRIGATION_MAX_CPHASE")?,
            paddy_ponding_limit_mm: real(document, "DEF_TUNING_IRRIGATION_PONDMX")?,
            allocation: scheme_index(document, "DEF_IRRIGATION_ALLOCATION", 1, 3)?,
            variably_saturated_flow,
            campbell,
            greenwich: logical(document, "DEF_simulation_time%greenwich")?,
        })
    } else {
        None
    };
    Ok(LandPhysicsParameters {
        use_pft,
        use_pc,
        bgc,
        irrigation,
        land_class_overrides,
        dynamic_wetland,
        dynamic_lake,
        hydraulic_model: if campbell {
            HydraulicModel::Campbell
        } else {
            HydraulicModel::VanGenuchten
        },
        variably_saturated_flow,
        plant_hydraulics,
        plant_hydraulic_parameters,
        plant_hydraulic_overrides,
        vegetation_snow,
        split_soil_snow,
        colm2024_interception,
        land_cover_scheme,
        root_fraction_scheme: ROOT_FRACTION_SCHEME,
        timestep_seconds,
        precipitation_scheme: precipitation_scheme(&text(
            document,
            "DEF_precip_phase_discrimination_scheme",
        )?)?,
        surface_resistance_scheme: soil_surface_resistance_scheme(document, campbell, use_pft)?,
        stress_scheme: scheme_index(document, "DEF_RSTFAC", 1, 2)?,
        surface_layer_scheme: if logical(document, "DEF_USE_CBL_HEIGHT")? {
            SurfaceLayerScheme::LargeEddy
        } else {
            SurfaceLayerScheme::Standard
        },
        thermal_conductivity_scheme: thermal_conductivity_scheme(integer(
            document,
            "DEF_THERMAL_CONDUCTIVITY_SCHEME",
        )?)?,
        observation_height_mode: observation_height_mode(&text(
            document,
            "DEF_forcing%HEIGHT_mode",
        )?)?,
        stomata: stomata(document, urban_run)?,
        soil_ice_impedance: real(document, "DEF_TUNING_SOIL_ICE_IMPEDANCE")?,
        snow_irreducible_saturation: real(document, "DEF_TUNING_SSI")?,
        impermeable_porosity: real(document, "DEF_TUNING_WIMP")?,
        ponding_limit_mm: real(document, "DEF_TUNING_PONDMX")?,
        wetland_water_capacity_mm: real(document, "DEF_TUNING_WETWATMAX")?,
        minimum_soil_potential_mm: real(document, "DEF_TUNING_SMPMIN")?,
        maximum_dew_mm: real(document, "DEF_TUNING_DEWMX")?,
        maximum_transpiration_mm_s: real(document, "DEF_TUNING_TRSMX0")?,
        surface_temperature_factor: real(document, "DEF_TUNING_CAPR")?,
        crank_nicolson_factor: real(document, "DEF_TUNING_CNFAC")?,
        soil_roughness_m: real(document, "DEF_TUNING_ZLND")?,
        // 超冷土壤水（Niu & Yang 2006）：`MOD_Namelist.F90:281` 默认**开**，
        // 只有 `DEF_URBAN_RUN` 会把它强关（`:2337`）。它决定冰点以下土层的
        // 液相保留量 —— 关掉会让表层土壤在冰点以下全部结冰，`ssw` 归零，
        // 地面反照率顶到上限。
        supercool_water: logical(document, "DEF_USE_SUPERCOOL_WATER")? && !urban_run,
        urban_run,
        // `snowfraction` 的指数（`MOD_Namelist.F90:618`，默认 1）。
        snow_cover_exponent: real(document, "DEF_TUNING_SNOW_COVER_EXPONENT")?,
        snow_roughness_m: real(document, "DEF_TUNING_ZSNO")?,
        // 观测高度**不在这里解析**：它挂在 `nl_forcing_type` 上（在 forcing namelist 里），
        // 而且 POINT 下文件里的 `reference_height_*` 会覆盖它
        // （`MOD_Forcing.F90:297-311`）。三级优先级由 `PointRuntimeConfig` 负责，
        // 调用方把结果经 [`ObservationHeights`] 传进来 —— 在这里读 case 文档只会拿到
        // schema 的 100/50/50，实测那会让 `zol` 差几十倍。
        wind_height_m: heights.wind_m,
        temperature_height_m: heights.temperature_m,
        humidity_height_m: heights.humidity_m,
        vaporization_heat_j_kg: VAPORIZATION_HEAT_J_KG,
        sprinkler_irrigation_kg_m2_s: 0.0,
        runoff_scheme,
        topmodel_decay_tuning: real(document, "DEF_TUNING_TOPMOD_DECAY")?,
    })
}

/// 这个算例要、但**本仓库的运行时尚且没有实现**的分支。
///
/// 每一项在上游都是**声明默认打开**或由另一项推出，所以"什么都没写"的算例几乎必然
/// 落进来。列成一张表而不是四处 `ensure!`，是因为调用方需要一次看全 —— 修一个再撞下一个
/// 的体验会让"到底能跑什么"变成一个移动靶。
///
/// 每一项都附上实测或上游出处，见 `docs/implementation-verification.md` 的对应小节。
pub fn unported_branches(_physics: &LandPhysicsParameters) -> Vec<&'static str> {
    // **现在是空的。** 最后一对是 VSF（`DEF_USE_VariablySaturatedFlow`）：
    // `variably_saturated_flow_step` 接上之后，上游有、本仓库没有的编排分支
    // 只剩**在别处**按 patch 拒绝的（如灌溉只接了 CROP BGC 土壤 patch，见
    // `StandardLctRestartTemplate::with_irrigation`），它们各自在装配期就报错，
    // 不需要在这里再列一遍。
    Vec::new()
}

/// `MOD_SoilSnowHydrology.F90:315-348` 的 `DEF_Runoff_SCHEME` 派发。
fn runoff_scheme(scheme: i64) -> Result<StandardLctRunoffScheme> {
    Ok(match scheme {
        0 => StandardLctRunoffScheme::Topmodel,
        2 => StandardLctRunoffScheme::XinAnJiang,
        1 => StandardLctRunoffScheme::Vic,
        3 => StandardLctRunoffScheme::SimpleVic,
        other => bail!("DEF_Runoff_SCHEME={other} is not one of the four upstream schemes 0..=3"),
    })
}

/// `MOD_Namelist.F90:522` 的三档降水相态判据。
///
/// 上游没有第四档 —— 历史上那个线性气温判据在 `LULC_*` 时期就没了，
/// `PrecipitationPhaseScheme::Legacy` 是内核给旧测试留的分支，namelist 选不到它。
fn precipitation_scheme(name: &str) -> Result<PrecipitationPhaseScheme> {
    Ok(match name {
        "I" => PrecipitationPhaseScheme::WetBulb,
        "II" => PrecipitationPhaseScheme::AirTemperature,
        "III" => PrecipitationPhaseScheme::HydrometeorTemperature,
        other => bail!(
            "DEF_precip_phase_discrimination_scheme={other:?} is not one of \"I\", \"II\", \"III\""
        ),
    })
}

/// `MOD_Namelist.F90:271` 的八档土壤热导率方案，取值与枚举同序。
fn thermal_conductivity_scheme(scheme: i64) -> Result<ThermalConductivityScheme> {
    Ok(match scheme {
        1 => ThermalConductivityScheme::Oleson,
        2 => ThermalConductivityScheme::Johansen,
        3 => ThermalConductivityScheme::CoteKonrad,
        4 => ThermalConductivityScheme::BallandArp,
        5 => ThermalConductivityScheme::Lu,
        6 => ThermalConductivityScheme::TarnawskiLeong,
        7 => ThermalConductivityScheme::DeVries,
        8 => ThermalConductivityScheme::YanHe,
        other => bail!(
            "DEF_THERMAL_CONDUCTIVITY_SCHEME={other} is outside the eight upstream schemes 1..=8"
        ),
    })
}

/// `DEF_forcing%HEIGHT_mode`：观测高度是绝对高度还是相对冠层顶。
fn observation_height_mode(name: &str) -> Result<ObservationHeightMode> {
    // 上游按大小写不敏感比较，且只认这两种写法（`MOD_Forcing.F90` 的
    // `TRIM(ADJUSTL(...)) == 'absolute'` 那一组）。第三种拼法一律报错，
    // 否则一个笔误会静默落进另一支。
    match name.trim().to_ascii_lowercase().as_str() {
        "absolute" => Ok(ObservationHeightMode::Absolute),
        "relative" | "relative_to_canopy" => Ok(ObservationHeightMode::RelativeToCanopy),
        other => bail!(
            "DEF_forcing%HEIGHT_mode={other:?} is neither \"absolute\" nor \"relative\"; \
             a typo here silently switches every reference height"
        ),
    }
}

/// `MOD_AssimStomataConductance.F90:196-205` 的气孔方案与四个覆盖。
///
/// 阈值判定在内核里（`selected_parameters`），这里**原样**把 namelist 的值带过去，
/// 不重复实现一遍 `>= 0.0` / `> 1.6` 的分界 —— 两份阈值迟早会分叉。
fn stomata(document: &Document, urban_run: bool) -> Result<StomataOptions> {
    let mut use_medlyn = logical(document, "DEF_USE_MEDLYNST")?;
    let mut use_wue = logical(document, "DEF_USE_WUEST")?;
    // 两个都开时上游**不报错**：`MOD_Namelist.F90:2080-2088` 把两者都置为
    // `.false.`，于是落回 Ball-Berry。映射必须照做 —— 拒绝它等于拒绝一个上游
    // 能跑的算例，而"照上游跑成 Ball-Berry"正是移植的目标。
    //
    // 这条也说明为什么不能靠"两个开关互斥"来校验：`DEF_USE_WUEST` 的声明默认值
    // 就是 `.true.`，所以任何只写 `DEF_USE_MEDLYNST = .true.` 的算例都会落进这里。
    if use_medlyn && use_wue {
        use_medlyn = false;
        use_wue = false;
    }
    // 城市模型在冲突处理之后再把 WUEST 关掉（`MOD_Namelist.F90:2252`）。
    if urban_run {
        use_wue = false;
    }
    Ok(StomataOptions {
        use_medlyn,
        use_wue,
        medlyn_g1_override: Some(real(document, "DEF_MEDLYN_G1")?),
        medlyn_g0_override: Some(real(document, "DEF_MEDLYN_G0")?),
        wue_lambda_override: Some(real(document, "DEF_WUE_LAMBDA")?),
        ball_berry_slope_override: Some(real(document, "DEF_BALL_BERRY_GRADM")?),
        ball_berry_intercept_override: Some(real(document, "DEF_BALL_BERRY_BINTER")?),
    })
}

/// 字段的有效值：算例里写了就用算例的，否则用 `colm-schema` 的声明默认值。
///
/// `DEF_RSS_SCHEME`，含上游那条**与 Campbell 绑定**的强制规则。
///
/// `MOD_Namelist.F90:1946-1950`：在 `DEF_USE_LCT` 分支里，
/// `DEF_USE_Campbell_SOIL_MODEL` 为假时把 `DEF_RSS_SCHEME` **强制置 0**，
/// 并打印 "Soil resistance is automaticlly turned off for VG soil + USGS|IGBP scheme"。
/// 也就是 van Genuchten 土壤下土壤表面阻力恒为 0，算例里写什么都不算数。
///
/// **`DEF_USE_LCT` 那道门在本仓库恒成立**：`LandCoverScheme` 只有 `Usgs`/`Igbp`
/// 两种地类分类，没有 PFT/PC 子网格，运行时走的也只有 standard-LCT 这一条链。
/// 所以这里只需要判 Campbell。
/// `MOD_Namelist.F90:1860-1867`：只有 **LCT** 在 van Genuchten 土壤下把
/// `DEF_RSS_SCHEME` 强制置 0（"Soil resistance is automaticlly turned off for VG soil +
/// USGS|IGBP scheme"）；PFT/PC 保留 namelist 值，默认 1。3-PFT AT-Neu 算例漏掉这一条时
/// 第一条 history 的 `f_rss` 是 0 对 Fortran 的 0.00885。
fn soil_surface_resistance_scheme(
    document: &Document,
    campbell: bool,
    use_pft: bool,
) -> Result<i32> {
    if !campbell && !use_pft {
        return Ok(0);
    }
    scheme_index(document, "DEF_RSS_SCHEME", 0, 5)
}

/// 每一档都在**类型**上钉住 schema 的声明类型：`DEF_RSS_SCHEME` 在 schema 里是
/// 整数，算例里写成 `1.0` 就该报错，而不是被 `as_f64` 悄悄收下。
/// schema 不认识这个路径同样报错 —— 那说明字段名拼错了，而继续走下去只会拿到
/// 一个凭空来的值。
fn integer(document: &Document, path: &str) -> Result<i64> {
    if let Some(value) = document.get(path) {
        return match value {
            Value::Int(value) => Ok(*value),
            other => bail!("{path} must be an integer, got {other:?}"),
        };
    }
    match &field(path)?.default {
        SchemaDefault::Integer(value) => Ok(*value),
        other => bail!("{path} is declared as {other:?}, not an integer"),
    }
}

/// 同 [`integer`]，但按上游声明的合法区间校验。
fn scheme_index(document: &Document, path: &str, low: i64, high: i64) -> Result<i32> {
    let value = integer(document, path)?;
    if !(low..=high).contains(&value) {
        bail!("{path}={value} is outside the upstream range {low}..={high}");
    }
    i32::try_from(value).with_context(|| format!("{path} does not fit an i32"))
}

fn real(document: &Document, path: &str) -> Result<f64> {
    let text = if let Some(value) = document.get(path) {
        match value {
            Value::Real { text } => text.clone(),
            Value::Int(value) => return Ok(*value as f64),
            other => bail!("{path} must be a real, got {other:?}"),
        }
    } else {
        match &field(path)?.default {
            SchemaDefault::Real(text) => (*text).to_owned(),
            SchemaDefault::Integer(value) => return Ok(*value as f64),
            other => bail!("{path} is declared as {other:?}, not a real"),
        }
    };
    parse_fortran_real(&text).with_context(|| format!("{path} is not a readable real: {text:?}"))
}

/// 上游 `MOD_Namelist.F90` 的 `LC_OVERRIDE_UNSET = -1.e36_r8`。
///
/// 它是 `DEF_LC_*` 的声明默认值，含义是"namelist 没写这一列，用地类表的值"——
/// 而不是一个真实的物理量。所以这里必须把它映射成 `None`，不能当成一个
/// `-1e36` 的叶导度传下去。
const LC_OVERRIDE_UNSET: f64 = -1.0e36;

/// 读一个 `DEF_LC_*`，把 `LC_OVERRIDE_UNSET` 折成 `None`。
fn land_cover_override(document: &Document, path: &str) -> Result<Option<f64>> {
    let value = real(document, path)?;
    if value == LC_OVERRIDE_UNSET {
        return Ok(None);
    }
    Ok(Some(value))
}

/// 算例是否把 `name` 设成了**非声明默认值**（`colm-schema` 里那份声明就是判据）。
///
/// 只写默认值不算"设过" —— 那正是黄金窗口验证过的路径；写别的值才是"要一份
/// 本仓库不会照做的行为"。实数按**解析后的 `f64` 比**（namelist 里的 `0.004` 与声明里的
/// `"0.004_r8"` 是同一个值），类型对不上或解析不了就按"设过"处理（宁可报错，别静默丢）。
fn sets_non_default(document: &Document, name: &str) -> bool {
    let (Ok(field), Some(value)) = (field(name), document.get(name)) else {
        return false;
    };
    match (&field.default, value) {
        (SchemaDefault::Logical(declared), Value::Bool(actual)) => declared != actual,
        (SchemaDefault::Integer(declared), Value::Int(actual)) => declared != actual,
        (SchemaDefault::Real(declared), Value::Real { text }) => {
            match (
                declared.trim_end_matches("_r8").parse::<f64>(),
                text.trim_end_matches("_r8").parse::<f64>(),
            ) {
                (Ok(declared), Ok(actual)) => declared != actual,
                _ => true,
            }
        }
        (SchemaDefault::Str(declared), Value::Str(actual)) => declared != actual,
        _ => true,
    }
}

fn logical(document: &Document, path: &str) -> Result<bool> {
    if let Some(value) = document.get(path) {
        return match value {
            Value::Bool(value) => Ok(*value),
            other => bail!("{path} must be a logical, got {other:?}"),
        };
    }
    match &field(path)?.default {
        SchemaDefault::Logical(value) => Ok(*value),
        other => bail!("{path} is declared as {other:?}, not a logical"),
    }
}

fn text(document: &Document, path: &str) -> Result<String> {
    if let Some(value) = document.get(path) {
        return match value {
            Value::Str(value) => Ok(value.clone()),
            other => bail!("{path} must be a character value, got {other:?}"),
        };
    }
    match &field(path)?.default {
        SchemaDefault::Str(value) => Ok((*value).to_owned()),
        other => bail!("{path} is declared as {other:?}, not a character value"),
    }
}

fn field(path: &str) -> Result<&'static colm_schema::Field> {
    find(path).with_context(|| {
        format!("{path} is not a field in MOD_Namelist.F90, so it has no declared default")
    })
}

/// Fortran 实数字面量转 `f64`。`_r8` 后缀与 `d` 指数都要处理 ——
/// `MOD_Namelist.F90` 里两种写法都有。
fn parse_fortran_real(text: &str) -> Result<f64> {
    let cleaned = text.trim().trim_end_matches("_r8").replace(['d', 'D'], "e");
    cleaned
        .parse()
        .with_context(|| format!("{cleaned:?} is not a Fortran real literal"))
}

#[cfg(test)]
#[path = "physics_tests.rs"]
mod physics_tests;
