use super::*;
use std::path::PathBuf;

use colm_namelist::parse;

/// 一个只有 namelist 组头的算例：每一项都落在 schema 的声明默认值上。
/// 测试用的观测高度。真实算例里这三个数有三级来源（见 `PointRuntimeConfig`），
/// 但映射本身只把它们照抄进参数表，所以测试用一个固定值即可。
const HEIGHTS: ObservationHeights = ObservationHeights {
    wind_m: 6.0,
    temperature_m: 6.0,
    humidity_m: 6.0,
};

fn empty_case() -> Document {
    parse("&nl_colm\n/\n").expect("an empty nl_colm group parses")
}

fn case_with(body: &str) -> Document {
    parse(&format!("&nl_colm\n{body}\n/\n")).expect("the case parses")
}

fn golden_case(name: &str) -> Document {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../oracle/cases")
        .join(name)
        .join("case.nml");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    parse(&text).unwrap_or_else(|error| panic!("cannot parse {}: {error}", path.display()))
}

/// 空算例必须逐项等于 `MOD_Namelist.F90` 的声明默认值。
///
/// 这条测试的价值在于**它读的是 schema**：映射里任何一处写成字面量（例如把
/// `DEF_TUNING_CAPR` 的 0.34 抄进代码）都会在默认值变动后与 schema 分叉，
/// 而那时候只有这条会响。
/// 子网格结构开关：LCT/PFT/PC 三选一，PFT 与 PC 都已移植。
///
/// 上游 `MOD_Namelist.F90:1932-1944` 要求 `DEF_USE_LCT`/`DEF_USE_PFT`/`DEF_USE_PC`
/// 恰好一个为真，默认是 LCT。以前**根本不读这三个开关** —— 一个选 PFT/PC 的算例会一路
/// 按 LCT 跑完，算式对、结构错、不报错。
///
/// 子网格还牵动 `DEF_RSS_SCHEME`：`:1860-1867` 只在 LCT 下把 VG 土壤的土壤表面阻力关掉，
/// PFT/PC 保留 namelist 值（默认 1）。
#[test]
fn subgrid_switches_select_pft_or_pc_and_keep_soil_resistance() {
    let pft = land_physics_parameters(
        &case_with("DEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_PC=.false."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect("a PFT case maps to physics parameters");
    assert!(pft.use_pft && !pft.use_pc);
    assert_eq!(pft.surface_resistance_scheme, 1);
    let pc = land_physics_parameters(
        &case_with("DEF_USE_LCT=.false.\nDEF_USE_PFT=.false.\nDEF_USE_PC=.true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect("a PC case maps to physics parameters");
    assert!(pc.use_pft && pc.use_pc);
    assert_eq!(pc.surface_resistance_scheme, 1);
    let lct = land_physics_parameters(&empty_case(), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(!lct.use_pft && !lct.use_pc);
    assert_eq!(lct.surface_resistance_scheme, 0);
    // 两个或三个同时为真也要报错（上游 `CoLM_stop`）。
    let error = land_physics_parameters(
        &case_with("DEF_USE_LCT=.true.\nDEF_USE_PFT=.true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect_err("two subgrid structures at once must be refused");
    assert!(error.to_string().contains("exactly one"), "{error}");
    // 默认（只有 LCT）照常。
    assert!(land_physics_parameters(&empty_case(), LandCoverScheme::Igbp, HEIGHTS).is_ok());
}

#[test]
fn an_empty_case_maps_every_declared_default() {
    let physics = land_physics_parameters(&empty_case(), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert_eq!(physics.hydraulic_model, HydraulicModel::VanGenuchten);
    // 七个 `DEF_PH_*` 的 namelist 声明默认值必须与
    // `PlantHydraulicParameters::default()` 逐位相同 —— 这正是纪律 #1
    // （缺省来自 schema，不在映射里写第二份）。手抄一次就会在
    // `MOD_Namelist.F90` 改默认值时与 schema 分叉，而只有这条会响。
    assert_eq!(
        physics.plant_hydraulic_parameters,
        PlantHydraulicParameters::default()
    );
    assert_eq!(physics.land_cover_scheme, LandCoverScheme::Igbp);
    assert_eq!(physics.timestep_seconds, 1800.0);
    assert_eq!(
        physics.precipitation_scheme,
        PrecipitationPhaseScheme::WetBulb
    );
    // `DEF_RSS_SCHEME` 声明默认 1，但空算例是 van Genuchten 土壤，
    // 于是上游那条与 Campbell 绑定的强制规则把它压到 **0**
    // （`MOD_Namelist.F90:1946-1950`）。下面另有一条断言证明 Campbell 时不压。
    assert_eq!(physics.surface_resistance_scheme, 0);
    assert_eq!(physics.stress_scheme, 1);
    assert_eq!(physics.surface_layer_scheme, SurfaceLayerScheme::Standard);
    // 声明默认是 4，而 Balland-Arp 在枚举里排第四。
    assert_eq!(
        physics.thermal_conductivity_scheme,
        ThermalConductivityScheme::BallandArp
    );
    assert_eq!(
        physics.observation_height_mode,
        ObservationHeightMode::Absolute
    );
    // 默认 3 是 Simple VIC（不是 XinAnJiang）。
    assert_eq!(physics.runoff_scheme, StandardLctRunoffScheme::SimpleVic);
    assert_eq!(physics.topmodel_decay_tuning, 2.0);
    // 观测高度来自传进来的 `ObservationHeights`（真实算例里由三级优先级解出），
    // 不是本函数从 case 文档里读的 —— case 文档里根本没有 `DEF_forcing%HEIGHT_*`，
    // 照读只会拿到 schema 的 100/50/50，实测那会让 `zol` 差几十倍。
    assert_eq!(physics.wind_height_m, HEIGHTS.wind_m);
    assert_eq!(physics.temperature_height_m, HEIGHTS.temperature_m);
    assert_eq!(physics.humidity_height_m, HEIGHTS.humidity_m);
    assert_eq!(physics.soil_roughness_m, 0.01);
    assert_eq!(physics.snow_roughness_m, 0.0024);
    // `snowfraction` 的雪密度指数（`MOD_Namelist.F90:618`，默认 1）。
    assert_eq!(physics.snow_cover_exponent, 1.0);
    // `DEF_USE_SUPERCOOL_WATER` 默认**开**（`MOD_Namelist.F90:281`）。
    assert!(physics.supercool_water);
    assert_eq!(physics.maximum_dew_mm, 0.1);
    assert_eq!(physics.surface_temperature_factor, 0.34);
    assert_eq!(physics.crank_nicolson_factor, 0.5);
    assert_eq!(physics.snow_irreducible_saturation, 0.033);
    assert_eq!(physics.impermeable_porosity, 0.05);
    assert_eq!(physics.ponding_limit_mm, 10.0);
    assert_eq!(physics.minimum_soil_potential_mm, -1.0e8);
    assert_eq!(physics.maximum_transpiration_mm_s, 2.0e-4);
    assert_eq!(physics.soil_ice_impedance, 6.0);
    // `hvap` 是 `MOD_Const_Physical` 的常数，namelist 里没有它。
    assert_eq!(physics.vaporization_heat_j_kg, 2.5104e6);
    // 默认不开喷灌，所以速率是 0 —— 但它不是从 namelist 读来的。
    assert_eq!(physics.sprinkler_irrigation_kg_m2_s, 0.0);
    assert!(physics.stomata.use_wue);
    assert!(!physics.stomata.use_medlyn);
}

/// `DEF_USE_VariablySaturatedFlow` 的**生效值**：上游 `MOD_Namelist.F90:1767-1772`
/// 在选了 van Genuchten 时强制置真，而它的声明默认值本来就是真。
///
/// 这三条决定了默认配置到底走 `WATER_2014`（Campbell/Richards）还是 `WATER_VSF`
/// （van Genuchten）—— 而本仓库只编排了前者，所以这个值必须是可读的，
/// 不能靠调用方自己推。
#[test]
fn the_effective_vsf_switch_follows_the_soil_model() {
    // 默认：van Genuchten + 声明默认 true。
    let physics = land_physics_parameters(&empty_case(), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert_eq!(physics.hydraulic_model, HydraulicModel::VanGenuchten);
    assert!(physics.variably_saturated_flow);

    // 选了 Campbell 但没关 VSF：上游不强制置真，声明默认仍是 true，所以还是 VSF。
    let physics = land_physics_parameters(
        &case_with("DEF_USE_Campbell_SOIL_MODEL = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(physics.variably_saturated_flow);

    // 只有显式关掉才落回 WATER_2014，也就是本仓库已经编排的那条。
    let physics = land_physics_parameters(
        &case_with("DEF_USE_Campbell_SOIL_MODEL = .true.\nDEF_USE_VariablySaturatedFlow = .false."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert_eq!(physics.hydraulic_model, HydraulicModel::Campbell);
    assert!(!physics.variably_saturated_flow);

    // van Genuchten 关不掉：上游会把它强制打开。
    let physics = land_physics_parameters(
        &case_with("DEF_USE_VariablySaturatedFlow = .false."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(physics.variably_saturated_flow);
}

/// `DEF_Runoff_SCHEME` 的编号必须照 `MOD_SoilSnowHydrology.F90:315-348` 的派发，
/// 而不是照枚举上原先那两条写反了的注释。
#[test]
fn runoff_scheme_numbers_follow_the_upstream_dispatch() {
    for (number, expected) in [
        (0, StandardLctRunoffScheme::Topmodel),
        (2, StandardLctRunoffScheme::XinAnJiang),
        (3, StandardLctRunoffScheme::SimpleVic),
    ] {
        let physics = land_physics_parameters(
            &case_with(&format!("DEF_Runoff_SCHEME = {number}")),
            LandCoverScheme::Igbp,
            HEIGHTS,
        )
        .unwrap();
        assert_eq!(
            physics.runoff_scheme, expected,
            "DEF_Runoff_SCHEME={number}"
        );
    }
}

/// 1 是 VIC 产流，必须映射到 VIC 而不是相邻的方案（上游派发是 0=TOPMODEL、1=VIC、
/// 2=XinAnJiang、3=SimpleVIC；挑错一个也能跑完，只是给出别的产流）。
#[test]
fn runoff_scheme_one_selects_vic() {
    let parameters = land_physics_parameters(
        &case_with("DEF_Runoff_SCHEME = 1"),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert_eq!(parameters.runoff_scheme, StandardLctRunoffScheme::Vic);
}

/// `DEF_USE_IRRIGATION` 读进灌溉设置（`DEF_TUNING_IRRIGATION_*` 取声明默认值），关闭时为 `None`。
#[test]
fn irrigation_settings_are_read_from_the_namelist() {
    let on = land_physics_parameters(
        &case_with("DEF_USE_IRRIGATION = .true.\n   DEF_IRRIGATION_ALLOCATION = 3"),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap()
    .irrigation
    .expect("irrigation on");
    assert_eq!(on.allocation, 3);
    assert_eq!(
        (on.start_seconds, on.duration_seconds),
        (21_600.0, 14_400.0)
    );
    assert_eq!(on.paddy_ponding_limit_mm, 100.0);
    let off = land_physics_parameters(&case_with(""), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(off.irrigation.is_none());
}

/// `DEF_SPLIT_SOILSNOW` 现在真的接上了：它必须被读进 `split_soil_snow`，
/// 而不是像从前那样被装配层硬写成 `false`（那样算例会静默按非 split 跑完）。
#[test]
fn split_soil_snow_is_read_into_the_physics_parameters() {
    let split = land_physics_parameters(
        &case_with("DEF_SPLIT_SOILSNOW = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(split.split_soil_snow);
    let default = land_physics_parameters(&case_with(""), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(!default.split_soil_snow);
}

/// 基流优化器（`MOD_Opt_Baseflow`）在预热期迭代 `scale_baseflow`，
/// 而本仓库把它钉成 1.0。开着它跑等于静默地不优化。
/// `DEF_USE_SNICAR` 此前同样没被读过，而 `assembly.rs` 把 `snow_layer_absorption_w_m2`
/// 钉成 `None` —— 开着它跑会静默用非 SNICAR 的雪光学。
#[test]
fn snicar_switches_follow_read_namelist() {
    // SNICAR 打开：`DEF_Aerosol_Readin` 的声明默认值 `.true.` 生效，`DEF_Aerosol_Clim` 原样读。
    let physics = land_physics_parameters(
        &case_with("DEF_USE_SNICAR = .true.\n   DEF_Aerosol_Clim = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect("SNICAR runs in the time loop");
    assert!(physics.snicar && physics.aerosol_readin && physics.aerosol_climatology);
    // SNICAR 关闭：`read_namelist` 把 `DEF_Aerosol_Readin` 强制置假（`MOD_Namelist.F90:2229-2235`）。
    let physics = land_physics_parameters(
        &case_with("DEF_Aerosol_Readin = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(!physics.snicar && !physics.aerosol_readin);
    // 城市的雪面没有 SNICAR 分支。
    let error = land_physics_parameters(
        &case_with("DEF_USE_SNICAR = .true.\n   DEF_URBAN_RUN = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect_err("urban SNICAR is not ported");
    assert!(error.to_string().contains("DEF_URBAN_RUN"), "{error}");
}

/// 优化器由运行时承担（`baseflow_optimizer`），物理参数映射不再拦它。
#[test]
fn baseflow_optimization_is_accepted_by_the_physics_mapping() {
    land_physics_parameters(
        &case_with("DEF_Optimize_Baseflow = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect("the baseflow optimizer runs in the runtime loop");
}

/// 金标准算例用的是 `DEF_Runoff_SCHEME = 3`，也就是 Simple VIC。
/// 这条把映射接到**仓库里那份真实算例**上，而不是只有构造出来的输入。
#[test]
fn the_checked_in_golden_case_selects_simple_vic() {
    let physics =
        land_physics_parameters(&golden_case("CN-Cng"), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert_eq!(physics.runoff_scheme, StandardLctRunoffScheme::SimpleVic);
    assert_eq!(physics.land_cover_scheme, LandCoverScheme::Igbp);
    assert_eq!(physics.timestep_seconds, 1800.0);
}

#[test]
fn canopy_settings_can_be_overridden_from_the_case() {
    // `DEF_USE_WUEST` 必须显式关掉：它的声明默认是 `.true.`，留着就会与
    // `DEF_USE_MEDLYNST` 撞上，而上游对撞车的处置是把两者都关掉（见
    // `both_stomata_switches_on_fall_back_to_ball_berry`）。
    let physics = land_physics_parameters(
        &case_with(
            "DEF_USE_MEDLYNST = .true.\n\
             DEF_USE_WUEST = .false.\n\
             DEF_MEDLYN_G1 = 4.5\n\
             DEF_WUE_LAMBDA = 1200.\n\
             DEF_USE_Campbell_SOIL_MODEL = .true.\n\
             DEF_USE_CBL_HEIGHT = .true.\n\
             DEF_forcing%HEIGHT_mode = 'relative'\n\
             DEF_TUNING_CAPR = 0.4",
        ),
        LandCoverScheme::Usgs,
        HEIGHTS,
    )
    .unwrap();
    assert_eq!(physics.hydraulic_model, HydraulicModel::Campbell);
    assert_eq!(physics.surface_layer_scheme, SurfaceLayerScheme::LargeEddy);
    assert_eq!(
        physics.observation_height_mode,
        ObservationHeightMode::RelativeToCanopy
    );
    assert_eq!(physics.surface_temperature_factor, 0.4);
    assert_eq!(physics.land_cover_scheme, LandCoverScheme::Usgs);
    assert!(physics.stomata.use_medlyn);
    // 阈值判定在内核里，所以映射把 namelist 的值原样带过去。
    assert_eq!(physics.stomata.medlyn_g1_override, Some(4.5));
    assert_eq!(physics.stomata.wue_lambda_override, Some(1200.0));
}

/// 两个气孔开关都开时**上游不报错**：`MOD_Namelist.F90:2080-2088` 把两者都置为
/// `.false.`，落回 Ball-Berry。映射照做，而不是拒绝这个算例。
#[test]
fn both_stomata_switches_on_fall_back_to_ball_berry() {
    let physics = land_physics_parameters(
        &case_with("DEF_USE_MEDLYNST = .true.\nDEF_USE_WUEST = .true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect("upstream resolves this conflict instead of failing");
    assert!(!physics.stomata.use_medlyn);
    assert!(!physics.stomata.use_wue);
}

/// 观测高度模式拼错一个字母就该报错：它决定每个参考高度是绝对高度还是相对冠层，
/// 静默落进另一支会让整条湍流交换算错而没有任何迹象。
#[test]
fn a_misspelled_height_mode_is_refused() {
    let error = land_physics_parameters(
        &case_with("DEF_forcing%HEIGHT_mode = 'absolut'"),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect_err("a misspelled mode must not fall through");
    assert!(error.to_string().contains("absolut"));
}

#[test]
fn unknown_and_out_of_range_fields_are_refused() {
    // 拼错的字段名没有声明默认值可退，必须报错而不是猜。
    assert!(land_physics_parameters(
        &case_with("DEF_Runoff_SCHEME = 9"),
        LandCoverScheme::Igbp,
        HEIGHTS
    )
    .is_err());
    assert!(land_physics_parameters(
        &case_with("DEF_THERMAL_CONDUCTIVITY_SCHEME = 9"),
        LandCoverScheme::Igbp,
        HEIGHTS
    )
    .is_err());
    assert!(land_physics_parameters(
        &case_with("DEF_precip_phase_discrimination_scheme = 'IV'"),
        LandCoverScheme::Igbp,
        HEIGHTS
    )
    .is_err());
}

#[test]
fn fortran_real_literals_parse_with_suffixes_and_d_exponents() {
    assert_eq!(parse_fortran_real("1800.").unwrap(), 1800.0);
    assert_eq!(parse_fortran_real("-1.e36_r8").unwrap(), -1.0e36);
    assert_eq!(parse_fortran_real("2.e-008").unwrap(), 2.0e-8);
    assert_eq!(parse_fortran_real("1.5D3").unwrap(), 1500.0);
    assert!(parse_fortran_real("not a number").is_err());
}

/// `DEF_RSS_SCHEME` 只在 van Genuchten 土壤上被强制置 0。
///
/// 上游 `MOD_Namelist.F90:1946-1950`：`DEF_USE_LCT` 分支里，
/// `DEF_USE_Campbell_SOIL_MODEL` 为假时把它压到 0（"Soil resistance is
/// automaticlly turned off for VG soil + USGS|IGBP scheme"）。
/// 这条测试钉住的是**条件性**：只断言"VG 时是 0"抓不住一个无条件覆盖，
/// 所以同一份算例再打开 Campbell，必须回到 namelist/默认写的那个值。
#[test]
fn the_soil_resistance_scheme_is_forced_to_zero_only_for_van_genuchten() {
    use colm_namelist::parse;

    // 空算例：VG，默认 1 → 被压到 0。
    let vg = land_physics_parameters(&empty_case(), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert_eq!(vg.hydraulic_model, HydraulicModel::VanGenuchten);
    assert_eq!(vg.surface_resistance_scheme, 0);

    // 同一份算例 + Campbell：不压，用文档里的值。
    let campbell =
        parse("&nl_colm\n  DEF_USE_Campbell_SOIL_MODEL = .true.\n  DEF_RSS_SCHEME = 3\n/\n")
            .unwrap();
    let physics = land_physics_parameters(&campbell, LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert_eq!(physics.hydraulic_model, HydraulicModel::Campbell);
    assert_eq!(physics.surface_resistance_scheme, 3);
}

/// `DEF_VEG_SNOW` 已经移植完，不再进"未移植分支"清单。
///
/// 它的分支逻辑在 `interception.rs`/`leaf_temperature.rs`/`radiation.rs`/
/// `high_res_radiation.rs` 四处都写了，缺的只是 `assembly.rs` 里那两个
/// 硬写死的 `vegetation_snow: false`（无雪那一支的两个装配点；有雪那一支
/// 本来就在透传）。改完之后实测 Jan 1-3、`DEF_VEG_SNOW = .true.` 与 Fortran
/// 的一致性，与已验收的 `.false.` 配置同一水平（`f_t_grnd` 0.075 K、
/// `f_tleaf` 0.127 K、`f_etr` 1.0e-8，结构量 `f_scv`/`f_snowdp`/`f_fsno` 两边恒 0）。
///
/// 这条断言钉住的是**默认配置可跑**：`DEF_VEG_SNOW` 的声明默认就是 `.true.`，
/// 所以它留在清单里意味着"什么都不写的算例一律被拒"。
#[test]
fn vegetation_snow_is_no_longer_an_unported_branch() {
    use colm_namelist::parse;

    let document = parse("&nl_colm\n  DEF_VEG_SNOW = .true.\n/\n").unwrap();
    let physics = land_physics_parameters(&document, LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(physics.vegetation_snow);
    assert!(
        !unported_branches(&physics)
            .iter()
            .any(|branch| branch.starts_with("DEF_VEG_SNOW")),
        "vegetation snow must not be reported as unported"
    );
}

/// 截获方案：`main/` 只接受 1 与 8，其余档位内核 `CALL abort`，这里同样拒绝。
#[test]
fn interception_scheme_eight_is_read_and_other_schemes_are_refused() {
    let colm2024 = land_physics_parameters(
        &case_with("DEF_Interception_scheme = 8"),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(colm2024.colm2024_interception);
    let default = land_physics_parameters(&case_with(""), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(!default.colm2024_interception);
    let error = land_physics_parameters(
        &case_with("DEF_Interception_scheme = 5"),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect_err("scheme 5 aborts in the kernel");
    assert!(error.to_string().contains("schemes 1"), "{error}");
}

/// BGC 需要 PFT/PC 子网格；已验证的分支（NITRIF 关）读出开关，未验证的分支当场拒绝，
/// 不能静默跑成另一个模式。
#[test]
fn bgc_switches_follow_the_namelist_and_unverified_branches_are_refused() {
    let lct = land_physics_parameters(
        &case_with("DEF_USE_BGC=.true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .expect_err("BGC on LCT stops the model");
    assert!(
        lct.to_string().contains("DEF_USE_PFT or DEF_USE_PC"),
        "{lct}"
    );

    let ported = land_physics_parameters(
        &case_with(
            "DEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_BGC=.true.\nDEF_USE_NITRIF=.false.",
        ),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    let switches = ported.bgc.expect("BGC switches");
    assert!(!switches.nitrif && !switches.fire && !switches.crop);

    // `DEF_USE_NITRIF` 的声明默认值是 .true.：默认 BGC 配置走硝化分支。
    let nitrif = land_physics_parameters(
        &case_with("DEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_BGC=.true."),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(nitrif.bgc.expect("BGC switches").nitrif);
    let fire = land_physics_parameters(
        &case_with(
            "DEF_USE_LCT=.false.\nDEF_USE_PFT=.true.\nDEF_USE_BGC=.true.\nDEF_USE_FIRE=.true.",
        ),
        LandCoverScheme::Igbp,
        HEIGHTS,
    )
    .unwrap();
    assert!(fire.bgc.expect("BGC switches").fire);

    let off = land_physics_parameters(&case_with(""), LandCoverScheme::Igbp, HEIGHTS).unwrap();
    assert!(off.bgc.is_none());
}
