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
    /// `DEF_forcing%HEIGHT_mode`。它和三个高度一样在 **forcing namelist** 里，
    /// 由调用方用 [`observation_height_mode`] 从那份文档解出；case 文档里没有这一项，
    /// 照读只会拿到声明默认的 `'absolute'`（ERA5LAND 的 `'relative'` 曾因此被静默丢掉）。
    pub mode: ObservationHeightMode,
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
    // 臭氧胁迫（`MOD_Ozone.F90`）：`DEF_URBAN_RUN` 下上游把 OZONESTRESS/OZONEDATA 一起关掉
    // （`MOD_Namelist.F90:2360-2361`），OZONESTRESS 关时 OZONEDATA 也被强制关（`:2092-2097`）。
    // LCT 的 `LeafTemperature` 收的 `ivt` 是字面量 1（`MOD_Thermal.F90:718`），所以 patch 级参数
    // 恒取 PFT 1（温带常绿针叶林）的 `isevg`/`leaf_long`；`DEF_PFT_LEAF_LONG` 只在 PFT/PC 下覆盖
    // （`Init_PFT_Const` 的 `IF (DEF_USE_PFT .or. DEF_USE_PC)`），这里取表值。
    let ozone = if logical(document, "DEF_USE_OZONESTRESS")? && !urban_run {
        let ko3 = real(document, "DEF_OZONE_KO3")?;
        ensure!(
            ko3.is_finite() && ko3 >= 0.0,
            "DEF_OZONE_KO3 must be finite and non-negative (MOD_Namelist.F90:2310 stops), got {ko3}"
        );
        let campbell = logical(document, "DEF_USE_Campbell_SOIL_MODEL")?;
        Some(colm_core::OzoneParameters {
            vegetation_type: 1,
            evergreen: colm_case::pft::fixed_value("isevg", 1)? != 0.0,
            leaf_longevity_years: colm_case::pft::default_value(
                "DEF_PFT_LEAF_LONG",
                1,
                campbell,
                false,
            )?
            .context("MOD_Const_PFT has no leaf_long")?,
            stomatal_resistance_factor: ko3,
            use_data: logical(document, "DEF_USE_OZONEDATA")?,
        })
    } else {
        None
    };
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
    // `DEF_USE_SNICAR`：雪反照率与分层吸收走 SNICAR（`SnowAlbedo`/`SNICAR_AD_RT`），雪层携带粒径与
    // 气溶胶。`read_namelist` 在 SNICAR 关闭时把 `DEF_Aerosol_Readin` 强制置假（`MOD_Namelist.F90:2229-2235`）。
    let snicar = logical(document, "DEF_USE_SNICAR")?;
    // `DEF_Aerosol_Readin`：读 `DEF_dir_runtime/aerosol/` 的月度沉降（`DEF_Aerosol_Clim` 选气候态）；
    // 关闭时 `forc_aer = 0`（`CoLMMAIN.F90:745-750`）。
    let aerosol_readin = snicar && logical(document, "DEF_Aerosol_Readin")?;
    let aerosol_climatology = logical(document, "DEF_Aerosol_Clim")?;
    // 城市 patch 上 SNICAR 不起作用：`CoLMMAIN_Urban` 与城市各模块里没有一处读 `DEF_USE_SNICAR`，
    // `WATER_2014` 对 `patchtype == 1 .and. DEF_URBAN_RUN` 显式走普通 `snowwater`，城市水体带
    // `urban_call`，`alburban` 不做雪粒老化。城市 patch 的 `snw_rds`/`mss_*`/`ssno_lyr` 原样保留。
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
            runoff_scheme: i32::try_from(integer(document, "DEF_Runoff_SCHEME")?)?,
            // 单点构建把它强制为 0（`MOD_Namelist.F90:1901`），`assemble_bgc` 在单点里再置 0。
            topmod_method: i32::try_from(integer(document, "DEF_TOPMOD_method")?)?,
            variably_saturated: logical(document, "DEF_USE_VariablySaturatedFlow")?
                || !logical(document, "DEF_USE_Campbell_SOIL_MODEL")?,
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
    // 这里曾有一张"设了非默认值就拒绝"的表（`oracle/nml-unread.snapshot` 里会改变物理、而运行期不读的
    // 开关）。第 443 轮逐个核对上游后全部清空，理由记在 `docs/implementation-verification.md`：
    // - 运行期死参数：`DEF_TUNING_CSOILC`（`rd_opt` 恒为 3，`MOD_LeafTemperature.F90:438`）、
    //   `DEF_TUNING_SMPMAX`/`DEF_TUNING_SIMPLE_VIC_DS/WS`（唯一读者的调用是注释，
    //   `MOD_SoilSnowHydrology.F90:1031`）；
    // - 已由重启或别处接通：`DEF_TUNING_SMPMAX_HR/SMPMIN_HR`（常数重启 → BGC）、`DEF_LAI_START/END_YEAR`
    //   （城市 LAI；`USE_SITE_LAI` 不看它们）；
    // - 只影响前处理（colm-init / colm-srfdata 已实现）：`DEF_USE_BEDROCK`、`DEF_SOIL_REFL_SCHEME`、
    //   `DEF_USE_DOMINANT_PATCHTYPE`、`DEF_USE_SOILPAR_UPS_FIT`、`DEF_LANDONLY`；
    // - `DEF_TOPMOD_method`：TOPMODEL 只在 `DEF_Runoff_SCHEME == 0` 时调用，而单点构建在那时把它强制为 0
    //   （`MOD_Namelist.F90:1899-1904`），这里给 0；空间入口按 namelist 覆盖成 0/1/2
    //   （`MOD_Runoff.F90:90-130,215-219`，方法 2 含 `GRATIO`），见 [`topmodel_method`]。
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
        snicar,
        aerosol_readin,
        aerosol_climatology,
        hydraulic_model: if campbell {
            HydraulicModel::Campbell
        } else {
            HydraulicModel::VanGenuchten
        },
        variably_saturated_flow,
        plant_hydraulics,
        plant_hydraulic_parameters,
        plant_hydraulic_overrides,
        ozone,
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
        observation_height_mode: heights.mode,
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
        // 单点构建没有 `GridRiverLakeFlow`；空间入口在装配前置真。
        river_lake_flow_build: false,
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
        // 单点构建：`DEF_Runoff_SCHEME == 0` 时上游强制 `DEF_TOPMOD_method = 0`
        // （`MOD_Namelist.F90:1899-1904`），其余产流方案不读它。空间入口再用 [`topmodel_method`] 覆盖。
        topmodel_method: 0,
    })
}

/// 空间构建的 `DEF_TOPMOD_method`。上游只认 0/1/2：`SurfaceRunoff_TOPMOD` 把"不是 0/1"一律当方法 2，
/// `SubsurfaceRunoff_TOPMOD` 却把"不是 1/2"当方法 0 —— 别的值两边不一致，这里拒绝。
pub fn topmodel_method(document: &Document) -> Result<u8> {
    match integer(document, "DEF_TOPMOD_method")? {
        value @ 0..=2 => Ok(value as u8),
        other => bail!("DEF_TOPMOD_method must be 0, 1 or 2, got {other}"),
    }
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

/// `DEF_forcing%HEIGHT_mode`：观测高度是绝对高度还是相对冠层顶。`forcing` 是 forcing namelist。
pub fn observation_height_mode(forcing: &Document) -> Result<ObservationHeightMode> {
    // 上游是 `trim(HEIGHT_mode) == 'absolute'`（`MOD_LeafTemperature.F90:573` 等四处），
    // 区分大小写，其余任何写法都进 relative 分支。这里只认上游 namelist 里出现的两种
    // 写法，其余一律报错：一个笔误（或 `'Absolute'`）在上游会静默落进 relative 分支。
    match text(forcing, "DEF_forcing%HEIGHT_mode")?.trim_end() {
        "absolute" => Ok(ObservationHeightMode::Absolute),
        "relative" => Ok(ObservationHeightMode::RelativeToCanopy),
        other => bail!(
            "DEF_forcing%HEIGHT_mode={other:?} is neither \"absolute\" nor \"relative\"; \
             upstream would silently treat it as \"relative\""
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
pub(crate) fn integer(document: &Document, path: &str) -> Result<i64> {
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

pub(crate) fn real(document: &Document, path: &str) -> Result<f64> {
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

pub(crate) fn logical(document: &Document, path: &str) -> Result<bool> {
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

pub(crate) fn text(document: &Document, path: &str) -> Result<String> {
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
