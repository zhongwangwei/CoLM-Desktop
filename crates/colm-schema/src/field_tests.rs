use crate::{all, find, Default, FieldKind};

#[test]
fn the_table_has_the_measured_number_of_fields() {
    // 实测：206 个顶层标量 + 4 个派生类型共 535 个成员，合计 741。
    // 三个调试宏改成运行时开关后新增了 DEF_USE_CoLMDEBUG /
    // DEF_USE_RangeCheck / DEF_USE_SrfdataDiag，顶层数从 202 变 205；
    // 土壤水力方案改成运行时开关后新增了 DEF_USE_Campbell_SOIL_MODEL
    // （extend_interception 没有对应的开关，见 MOD_Namelist.F90 里的说明），
    // 顶层数从 205 变 206。
    // 若这个数再变了，要么上游改了，要么生成器漏了 —— 两种都必须有人看一眼。
    let total = all().len();
    assert!(
        (700..=760).contains(&total),
        "expected roughly 740 fields, got {total}"
    );
    let top = all().iter().filter(|f| f.owner.is_none()).count();
    assert_eq!(top, 206, "top-level count changed");
}

#[test]
fn a_known_scalar_is_described_correctly() {
    let f = find("DEF_CASE_NAME").expect("DEF_CASE_NAME must be in the schema");
    assert!(matches!(f.kind, FieldKind::Character { .. }));
    assert!(f.owner.is_none());
}

#[test]
fn a_derived_type_member_carries_its_owner() {
    let f = find("DEF_forcing%dataset").expect("must be in the schema");
    assert_eq!(f.owner, Some("nl_forcing_type"));
}

#[test]
fn an_array_field_records_its_arity() {
    // fprefix(8) —— GUI 要知道它有 8 槽，且第 5 槽在 POINT 下是 'NULL'
    let f = find("DEF_forcing%fprefix").expect("must be in the schema");
    assert_eq!(f.arity, Some(8));
}

#[test]
fn defaults_are_recorded_exactly_as_colm_declares_them() {
    // 这两个默认值都假设 HPC 数据树存在（见 design.md §2.5）：臭氧要 2.8 GB
    // 的全球场，Simple VIC 要站点文件里有 soil_texture。处置并不相同 ——
    // 臭氧是本项目唯一必须显式关掉的，产流方案则沿用 CoLM 的 3 并补数据。
    // 但那都是上层的决定：schema 只负责如实记录 CoLM 声明的原值，
    // 不在这里偷偷改掉，否则「CoLM 的默认」与「我们建议的默认」就分不清了。
    assert_eq!(
        find("DEF_USE_OZONEDATA").map(|f| f.default),
        Some(Default::Logical(true))
    );
    assert_eq!(
        find("DEF_Runoff_SCHEME").map(|f| f.default),
        Some(Default::Integer(3))
    );
}

#[test]
fn no_local_variable_leaked_into_the_schema() {
    // MOD_Namelist.F90 里有 8 个不含 '=' 的声明（7 个不同名字），
    // 它们是子程序局部变量与哑元
    // （nlfile / fexists / ivar / ierr / iomesg / set_defaults / onoff），
    // 不是配置字段。生成器必须靠作用域排除它们 —— 靠 intent(...) 属性过滤
    // 是不够的，因为 fexists / ivar / ierr / iomesg 都没有 intent。
    for leaked in [
        "nlfile",
        "fexists",
        "ivar",
        "ierr",
        "iomesg",
        "set_defaults",
        "onoff",
    ] {
        assert!(
            find(leaked).is_none(),
            "{leaked} is a subroutine local, not a config field"
        );
    }
}

#[test]
fn the_history_type_contributes_the_bulk_of_the_table() {
    let n = all()
        .iter()
        .filter(|f| f.owner == Some("history_var_type"))
        .count();
    assert_eq!(n, 482, "history_var_type member count changed");
}

#[test]
fn lookup_ignores_case_the_way_fortran_does() {
    // MOD_Namelist.F90 声明的是 DEF_HIST_vars_out_default，而 CoLM 自己入库的
    // 算例文件多数写成 DEF_hist_vars_out_default —— 两种写法它都能跑。
    let want = find("DEF_HIST_vars_out_default").expect("declared in MOD_Namelist.F90");
    for probe in [
        "DEF_hist_vars_out_default",
        "def_hist_vars_out_default",
        "DEF_HIST_VARS_OUT_DEFAULT",
    ] {
        assert_eq!(
            find(probe).map(|f| f.name),
            Some(want.name),
            "{probe} should resolve to the same field"
        );
    }
}

#[test]
fn no_two_fields_differ_only_in_case() {
    // 大小写不敏感的查找只有在名字集合本身无歧义时才是对的。
    // 这条守住那个前提：上游哪天加一个只差大小写的重名字段，
    // find 会静默地只返回其中一个，而这里会先炸。
    let mut seen = std::collections::HashMap::new();
    let mut clashes = Vec::new();
    for f in all() {
        if let Some(prev) = seen.insert(f.name.to_ascii_lowercase(), f.name) {
            clashes.push(format!("{prev} vs {}", f.name));
        }
    }
    assert!(clashes.is_empty(), "names collide once folded: {clashes:?}");
}

#[test]
fn the_single_point_section_is_in_the_table() {
    // MOD_Namelist.F90 的 Part 3 用 SITE_ / USE_SITE_ 前缀，而生成器原先
    // 按 `DEF_` 白名单收字段，于是把整个单点段滤掉了 —— 在一个专做单点的
    // 项目里。这 21 个是那一段的全部。
    let want = [
        "SITE_fsitedata",
        "SITE_lon_location",
        "SITE_lat_location",
        "SITE_landtype",
        "USE_SITE_landtype",
        "USE_SITE_pctpfts",
        "USE_SITE_pctcrop",
        "USE_SITE_htop",
        "USE_SITE_LAI",
        "USE_SITE_lakedepth",
        "USE_SITE_soilreflectance",
        "USE_SITE_soilparameters",
        "USE_SITE_dbedrock",
        "USE_SITE_topography",
        "USE_SITE_urban_geometry",
        "USE_SITE_urban_ecology",
        "USE_SITE_urban_radiation",
        "USE_SITE_urban_thermal",
        "USE_SITE_urban_human",
        "USE_SITE_HistWriteBack",
        "USE_SITE_ForcingReadAhead",
    ];
    for n in want {
        let f = find(n).unwrap_or_else(|| panic!("{n} missing from the schema"));
        assert_eq!(f.group, Some("nl_colm"), "{n}");
    }
}

#[test]
fn the_three_aggregation_switches_are_in_the_table() {
    // 与单点段一样，只因为不叫 DEF_ 就被滤掉了。
    for n in [
        "USE_srfdata_from_3D_gridded_data",
        "USE_srfdata_from_larger_region",
        "USE_zip_for_aggregation",
    ] {
        assert_eq!(find(n).and_then(|f| f.group), Some("nl_colm"), "{n}");
    }
}

#[test]
fn a_field_nobody_can_set_is_marked_as_such() {
    // 这 6 个有声明、有默认值，但不在任何 namelist 组里。
    // DEF_dir_history 更进一步：MOD_Namelist.F90:1406 用 DEF_dir_output 与
    // DEF_CASE_NAME 无条件把它覆盖掉。GUI 给这种字段一个输入框就是在骗人。
    for n in [
        "DEF_dir_history",
        "DEF_dir_landdata",
        "DEF_dir_restart",
        "DEF_USE_IGBP",
        "DEF_USE_USGS",
        "DEF_Wetland_finundation_scheme",
    ] {
        let f = find(n).unwrap_or_else(|| panic!("{n} should still be in the table"));
        assert_eq!(f.group, None, "{n} is not settable from any namelist");
    }
}

#[test]
fn members_inherit_the_group_of_their_container() {
    // 这条决定 GUI 把一个字段写进哪个文件：强迫场字段进 nl_colm_forcing，
    // 输出变量开关进 nl_colm_history，其余进主 namelist。
    assert_eq!(
        find("DEF_forcing%dataset").unwrap().group,
        Some("nl_colm_forcing")
    );
    assert_eq!(
        find("DEF_hist_vars%xy_us").unwrap().group,
        Some("nl_colm_history")
    );
    assert_eq!(
        find("DEF_simulation_time%start_year").unwrap().group,
        Some("nl_colm")
    );
    assert_eq!(find("DEF_domain%edges").unwrap().group, Some("nl_colm"));
}

#[test]
fn the_macro_guarded_member_is_still_a_field() {
    // DEF_file_GIEMS 在 namelist 语句里被 #if (defined TRACER) && (defined BGC)
    // 包着。它是真字段，只是仅在那个预设下可设 —— 不能因为解析器看见
    // 一行 `#if` 就把它丢掉。
    assert_eq!(
        find("DEF_file_GIEMS").and_then(|f| f.group),
        Some("nl_colm")
    );
}

#[test]
fn a_use_statement_is_not_a_field() {
    // `USE, intrinsic :: ieee_arithmetic` 会被声明扫描器当成一个声明。
    // 原先靠 `DEF_` 白名单顺带挡住；改用 namelist 判据之后，它自然落选 ——
    // 因为它不在任何 namelist 组里。这条守住那个「顺带」不再是巧合。
    assert!(find("ieee_arithmetic").is_none());
}
