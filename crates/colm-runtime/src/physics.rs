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

use anyhow::{bail, Context, Result};
use colm_core::{
    HydraulicModel, LandCoverScheme, ObservationHeightMode, PrecipitationPhaseScheme,
    RootFractionScheme, StomataOptions, SurfaceLayerScheme, ThermalConductivityScheme,
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
    if logical(document, "DEF_USE_IRRIGATION")? {
        // 上游的喷灌率由 `DEF_TUNING_IRRIGATION_*` 与作物物候逐步算出，
        // 不是 namelist 里的一个常数。这里给 0 会让开启喷灌的算例静默变成不灌溉。
        bail!(
            "DEF_USE_IRRIGATION is on, but the Rust runtime has no sprinkler schedule yet; \
             the rate is derived per step from DEF_TUNING_IRRIGATION_* and the crop phenology, \
             so substituting zero would silently run the case unirrigated"
        );
    }
    // 上游 `MOD_Namelist.F90:1767-1772`：选了 van Genuchten 就把
    // `DEF_USE_VariablySaturatedFlow` 强制置真。它的声明默认值也是真，所以**默认
    // 配置跑的是 VSF**，经典 Richards 路径要显式关掉才走得到。
    let campbell = logical(document, "DEF_USE_Campbell_SOIL_MODEL")?;
    let variably_saturated_flow = logical(document, "DEF_USE_VariablySaturatedFlow")? || !campbell;
    // `DEF_USE_PLANTHYDRAULICS` 的声明默认值是 `.true.`，所以**默认配置开着 PHS**，
    // 本仓库的 standard-LCT 分支则是硬关的。调用方要能看出这个不匹配。
    let plant_hydraulics = logical(document, "DEF_USE_PLANTHYDRAULICS")?;
    // 同上：`DEF_VEG_SNOW` 的声明默认值也是 `.true.`。
    let vegetation_snow = logical(document, "DEF_VEG_SNOW")?;
    Ok(LandPhysicsParameters {
        hydraulic_model: if campbell {
            HydraulicModel::Campbell
        } else {
            HydraulicModel::VanGenuchten
        },
        variably_saturated_flow,
        plant_hydraulics,
        vegetation_snow,
        land_cover_scheme,
        root_fraction_scheme: ROOT_FRACTION_SCHEME,
        timestep_seconds,
        precipitation_scheme: precipitation_scheme(&text(
            document,
            "DEF_precip_phase_discrimination_scheme",
        )?)?,
        surface_resistance_scheme: scheme_index(document, "DEF_RSS_SCHEME", 0, 5)?,
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
        stomata: stomata(document)?,
        soil_ice_impedance: real(document, "DEF_TUNING_SOIL_ICE_IMPEDANCE")?,
        snow_irreducible_saturation: real(document, "DEF_TUNING_SSI")?,
        impermeable_porosity: real(document, "DEF_TUNING_WIMP")?,
        ponding_limit_mm: real(document, "DEF_TUNING_PONDMX")?,
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
        supercool_water: logical(document, "DEF_USE_SUPERCOOL_WATER")?,
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
pub fn unported_branches(physics: &LandPhysicsParameters) -> Vec<&'static str> {
    let mut missing = Vec::new();
    if physics.variably_saturated_flow {
        missing.push(
            "DEF_USE_VariablySaturatedFlow：土壤水文走 WATER_VSF（van Genuchten），\
             内核已移植但缺编排；上游在选 van Genuchten 时强制打开，且自身默认即为真。\
             写 DEF_USE_Campbell_SOIL_MODEL = .true. 与 \
             DEF_USE_VariablySaturatedFlow = .false. 可走已编排的 WATER_2014",
        );
    }
    if physics.plant_hydraulics {
        missing.push(
            "DEF_USE_PLANTHYDRAULICS：植物水力（默认真）改的是 ET 的分层分配、冠层阻力\
             的来源与 vegwp 状态；写 DEF_USE_PLANTHYDRAULICS = .false. 可走已移植的那条",
        );
    }
    if physics.vegetation_snow {
        missing.push(
            "DEF_VEG_SNOW：植被上的雪（默认真）决定 fwet_snow 与冠层水的雪/雨分配；\
             实测一步之后 Fortran 的 fwet_snow = 0.061 而本仓库是 0。\
             写 DEF_VEG_SNOW = .false. 可走已移植的那条",
        );
    }
    missing
}

/// `MOD_SoilSnowHydrology.F90:315-348` 的 `DEF_Runoff_SCHEME` 派发。
fn runoff_scheme(scheme: i64) -> Result<StandardLctRunoffScheme> {
    Ok(match scheme {
        0 => StandardLctRunoffScheme::Topmodel,
        2 => StandardLctRunoffScheme::XinAnJiang,
        3 => StandardLctRunoffScheme::SimpleVic,
        1 => bail!(
            "DEF_Runoff_SCHEME=1 selects the VIC runoff scheme, which the Rust runtime has not \
             ported; 0 (TOPMODEL), 2 (XinAnJiang) and 3 (Simple VIC) are available"
        ),
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
fn stomata(document: &Document) -> Result<StomataOptions> {
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
