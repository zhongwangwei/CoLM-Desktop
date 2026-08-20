use super::*;

const SAMPLE: &str = "\
&nl_colm

! 用户自己的笔记 —— 保存一次不该把它冲掉
   DEF_CASE_NAME = 'CN-Cng'          ! 冬季窗口
   DEF_USE_OZONEDATA = .FALSE.
   DEF_simulation_time%start_year = 2008
   DEF_simulation_time%timestep = 1800.
   USE_SITE_topostd = .false.
/
";

#[test]
fn reading_a_case_marks_what_the_schema_knows() {
    let e = read_case(SAMPLE.into()).expect("parses");
    let by = |n: &str| e.iter().find(|x| x.path == n).expect(n);
    assert!(by("DEF_CASE_NAME").known);
    assert_eq!(by("DEF_CASE_NAME").group, Some("nl_colm"));
    assert!(!by("DEF_CASE_NAME").derived);
    // 上游删掉的字段。CoLM 读到会 `Cannot match namelist object name` 然后停 ——
    // 界面该在开跑前点名它，而不是让用户对着那句报错发呆。
    assert!(!by("USE_SITE_topostd").known);
}

#[test]
fn unknown_fields_names_the_ones_colm_would_reject() {
    // USE_SITE_topostd 与 USE_SITE_BVIC 都在上游自己发布的单点示例
    // run/examples/SiteSYSUAtmos_IGBP_VG.nml 里，而两者都已从
    // MOD_Namelist.F90 删除 —— 那个示例现在根本跑不了。
    let u = unknown_fields(SAMPLE.into()).expect("parses");
    assert_eq!(u, ["USE_SITE_topostd"]);
}

#[test]
fn changing_one_field_leaves_every_other_line_byte_identical() {
    // colm-namelist 的往返保证。用户的注释与对齐是他们自己的东西。
    let out = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), "CN-Cng-wet".into()).expect("sets");
    let differing: Vec<usize> = SAMPLE
        .lines()
        .zip(out.lines())
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i + 1)
        .collect();
    assert_eq!(differing.len(), 1, "changed lines: {differing:?}");
    assert_eq!(SAMPLE.lines().count(), out.lines().count());
    assert!(out.contains("'CN-Cng-wet'"));
    // 行尾注释还在
    assert!(out.contains("! 冬季窗口"));
}

#[test]
fn a_value_of_the_wrong_type_is_refused_before_it_reaches_the_file() {
    // 前端只送字符串；类型由 schema 决定。送错了要在这里就被拦下，
    // 而不是写进文件、等 CoLM 跑起来才报。
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%start_year".into(),
        "早点".into(),
    )
    .unwrap_err();
    assert!(e.contains("integer"), "{e}");
    let e = set_field(SAMPLE.into(), "DEF_USE_OZONEDATA".into(), "yes".into()).unwrap_err();
    assert!(e.contains("logical"), "{e}");
    let e = set_field(
        SAMPLE.into(),
        "DEF_simulation_time%timestep".into(),
        "half".into(),
    )
    .unwrap_err();
    assert!(e.contains("real"), "{e}");
}

#[test]
fn a_real_keeps_the_spelling_it_was_given() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，但往返要还原用户写的那种，
    // 否则每次保存都改写一遍用户没动过的写法。
    for spelling in ["3600.", "3600.0", "3.6e3"] {
        let out = set_field(
            SAMPLE.into(),
            "DEF_simulation_time%timestep".into(),
            spelling.into(),
        )
        .expect("sets");
        assert!(out.contains(spelling), "{spelling} not found in output");
    }
}

#[test]
fn a_string_longer_than_the_declared_length_is_refused() {
    // DEF_CASE_NAME 是 character(len=256)。超长会被 Fortran 悄悄截断，
    // 于是产物目录名与用户以为的不同 —— 在这里拦下说得清楚得多。
    let long = "x".repeat(300);
    let e = set_field(SAMPLE.into(), "DEF_CASE_NAME".into(), long).unwrap_err();
    assert!(e.contains("256") && e.contains("300"), "{e}");
}

#[test]
fn setting_a_field_the_file_does_not_have_is_an_error_not_an_append() {
    // 静默追加会写出一个同名字段出现两次的文件，Fortran 取最后一个，
    // 而用户在界面上看到的是第一个。
    let e = set_field(SAMPLE.into(), "DEF_HIST_FREQ".into(), "HOURLY".into()).unwrap_err();
    assert!(e.contains("no such field"), "{e}");
}

/// 造 n 个算例目录，每个一份 case.nml。`tag` 让各测试互不干扰 ——
/// 本项目没有引入 tempfile，与 `sites_tests` 一样自己套一层。
fn batch(tag: &str, texts: &[&str]) -> Vec<String> {
    let tmp = std::env::temp_dir().join(format!("colm-batch-{tag}"));
    let _ = std::fs::remove_dir_all(&tmp);
    batch_in(&tmp, texts)
}

fn batch_in(tmp: &std::path::Path, texts: &[&str]) -> Vec<String> {
    texts
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let d = tmp.join(format!("case{i}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("case.nml"), t).unwrap();
            d.display().to_string()
        })
        .collect()
}

const NML_A: &str = "&nl_colm\n   DEF_simulation_time%start_year = 2002\n   DEF_simulation_time%end_year = 2013\n   DEF_HIST_FREQ = 'HOURLY'\n/\n";
const NML_B: &str = "&nl_colm\n   DEF_simulation_time%start_year = 2005\n   DEF_simulation_time%end_year = 2008\n   DEF_HIST_FREQ = 'DAILY'\n/\n";

#[test]
fn one_change_lands_in_every_case_of_the_batch() {
    // 勾了 20 个站点是要配"这一次运行"，不是配其中第一个。只改第一个的话，
    // 另外 19 个会带着未改的配置跑完，而界面上看不出任何异常。
    let dirs = batch("every", &[NML_A, NML_B]);
    let r = super::set_field_batch(dirs.clone(), "DEF_HIST_FREQ".into(), "MONTHLY".into()).unwrap();
    assert_eq!(r.written, 2);
    for d in &dirs {
        let t = std::fs::read_to_string(std::path::Path::new(d).join("case.nml")).unwrap();
        assert!(t.contains("MONTHLY"), "{d} 没被改到：{t}");
    }
    // 回传的是代表算例（第一个）的新内容，前端拿它继续显示。
    assert!(r.text.contains("MONTHLY"));
    assert!(r.text.contains("2002"), "代表算例应当是第一个");
}

#[test]
fn a_batch_write_that_cannot_finish_writes_nothing() {
    // 半批配置好的算例与整批配置好的在界面上长得一样，而它们跑出来的
    // 东西不一样 —— 所以宁可一份都不写。
    let dirs = batch("nothing", &[NML_A, "&nl_colm\n   这不是 namelist"]);
    let before = std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap();
    let e = super::set_field_batch(dirs.clone(), "DEF_HIST_FREQ".into(), "MONTHLY".into())
        .expect_err("坏文件必须让整批失败");
    assert!(e.contains("case1"), "错误要说清是哪一个：{e}");
    assert_eq!(
        std::fs::read_to_string(std::path::Path::new(&dirs[0]).join("case.nml")).unwrap(),
        before,
        "第一个算例不该被动过"
    );
}

#[test]
fn wizard_fields_are_written_together() {
    let dir = batch("wizard", &[SAMPLE]).remove(0);
    super::apply_fields(
        &dir,
        &[
            FieldChange {
                path: "DEF_USE_PFT".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_LCT".into(),
                value: ".false.".into(),
            },
            FieldChange {
                path: "DEF_USE_BGC".into(),
                value: ".true.".into(),
            },
        ],
    )
    .unwrap();
    let text = std::fs::read_to_string(std::path::Path::new(&dir).join("case.nml")).unwrap();
    let doc = colm_namelist::parse(&text).unwrap();
    for (name, value) in [
        ("DEF_USE_PFT", ".true."),
        ("DEF_USE_LCT", ".false."),
        ("DEF_USE_BGC", ".true."),
    ] {
        assert_eq!(doc.get(name).unwrap().to_string(), value, "{name}");
    }
}

#[test]
fn invalid_wizard_field_leaves_the_case_unchanged() {
    let dir = batch("wizard-invalid", &[SAMPLE]).remove(0);
    let path = std::path::Path::new(&dir).join("case.nml");
    let before = std::fs::read_to_string(&path).unwrap();
    let err = super::apply_fields(
        &dir,
        &[
            FieldChange {
                path: "DEF_USE_PFT".into(),
                value: ".true.".into(),
            },
            FieldChange {
                path: "DEF_USE_RangeCheck".into(),
                value: "yes".into(),
            },
        ],
    )
    .unwrap_err();
    assert!(err.contains("logical"), "{err}");
    assert_eq!(std::fs::read_to_string(path).unwrap(), before);
}

#[test]
fn fields_the_batch_disagrees_on_are_reported() {
    let dirs = batch("varies", &[NML_A, NML_B]);
    let v = super::varying_fields(dirs).unwrap();
    assert!(v.iter().any(|p| p == "DEF_HIST_FREQ"), "{v:?}");
    assert!(
        v.iter().any(|p| p == "DEF_simulation_time%start_year"),
        "{v:?}"
    );

    // 单个算例永远没有分歧 —— 而不是"每个字段都算分歧"。
    assert!(super::varying_fields(batch("alone", &[NML_A]))
        .unwrap()
        .is_empty());
}

#[test]
fn spin_up_is_computed_per_case_not_shared() {
    // 各站点的强迫场起点不同。用同一个绝对年份会让一部分算例的预热落在
    // 窗口之外（等于没预热），另一部分落得过深（等于把输出砍掉一大截）。
    let dirs = batch("spinup", &[NML_A, NML_B]);
    super::set_spinup(dirs.clone(), 1, 10).unwrap();
    let year = |d: &str| {
        let t = std::fs::read_to_string(std::path::Path::new(d).join("case.nml")).unwrap();
        t.lines()
            .find(|l| l.contains("spinup_year"))
            .map(|l| l.split('=').nth(1).unwrap().trim().to_string())
            .expect("写了 spinup_year")
    };
    assert_eq!(year(&dirs[0]), "2003", "起点 2002 的算例预热到 2003");
    assert_eq!(year(&dirs[1]), "2006", "起点 2005 的算例预热到 2006");

    // 读回来：窗口不一致要报出来，预热设置一致则不报。
    let t = super::read_timing(dirs.clone()).unwrap();
    assert_eq!(t.count, 2);
    assert!(t.window_varies, "两个算例的窗口本来就不同");
    assert!(!t.spinup_varies);
    assert_eq!(t.spinup_years, 1);
    assert_eq!(t.spinup_repeat, 10);
    // 输出从预热结束处才开始 —— 这是预热的代价，界面必须能说出来。
    assert_eq!(t.output_start, "2003-01-01");
}

#[test]
fn every_source_namelist_field_has_a_named_ui_section() {
    // 参数页不再用「其他」兜底。上游新增字段时应该在 CI 里
    // 点名，迫使我们读它的 namelist 语义后再归类，而不是随手堆进杂物箱。
    let missing: Vec<_> = colm_schema::all()
        .iter()
        .filter(|f| !f.name.starts_with("DEF_hist_vars%"))
        .filter(|f| super::field_section(f.name, f.group).is_none())
        .map(|f| f.name)
        .collect();
    assert!(missing.is_empty(), "还没归类的源码字段：{missing:?}");
}

#[test]
fn forcing_namelist_path_is_used_by_colm_and_stays_visible() {
    let f = colm_schema::find("DEF_forcing_namelist").expect("源码里还有这个字段");
    assert_eq!(super::field_section(f.name, f.group), Some("文件与目录"));

    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../vendor/CoLM202X/share/MOD_Namelist.F90");
    let Ok(text) = std::fs::read_to_string(source) else {
        return; // 未取 submodule 时由 schema drift 测试负责
    };
    assert!(
        text.contains("file=trim(DEF_forcing_namelist)"),
        "CoLM 已不再用 DEF_forcing_namelist，重新评估是否从界面和算例里删掉"
    );
}

#[test]
fn kernel_macros_decide_which_parameters_are_relevant() {
    let default = ["SinglePoint"].into_iter().collect();
    let with_da = ["SinglePoint", "DataAssimilation"].into_iter().collect();

    let relevant = |name, have| {
        let f = colm_schema::find(name).expect(name);
        super::field_is_relevant(f, have)
    };
    assert!(relevant("DEF_CASE_NAME", &default));
    assert!(relevant("DEF_dir_output", &default));
    // LULC/BGC/CROP/URBAN/LULCC 那组改造之后：URBAN_MODEL 与 BGC 不再是
    // 编译期宏（main/URBAN/、main/BGC/ 始终编译进去，DEF_URBAN_RUN/
    // DEF_USE_BGC 在 MOD_Namelist.F90 里改成运行时开关），所以
    // `DEF_URBAN_RUN`/`DEF_USE_CN_INIT` 现在在**每个**内核下都相关——
    // 不再有一个「URBAN_MODEL 没编进去」或「BGC 没编进去」的内核让它们
    // 变得不相关，用户设了就会在运行时生效（或不生效，那是 DEF_URBAN_RUN/
    // DEF_USE_BGC 本身的值决定的，不是这个函数管的编译期相关性）。
    assert!(relevant("DEF_URBAN_RUN", &default));
    assert!(relevant("DEF_USE_CN_INIT", &default));
    assert!(relevant("DEF_USE_TRACER", &default));
    // DataAssimilation 仍然是真正的编译期宏（这组改造没有碰它），
    // 用来验证「宏决定相关性」这条机制本身还成立。
    assert!(!relevant("DEF_DA_TWS", &default));
    assert!(relevant("DEF_DA_TWS", &with_da));
}
