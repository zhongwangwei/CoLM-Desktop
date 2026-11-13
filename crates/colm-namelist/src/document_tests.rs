#[test]
fn a_field_the_file_lacks_can_be_inserted_into_its_group() {
    // 专家模式下用户能改一个这份文件没设过的字段，而 `set` 对那种字段是
    // 报错的 —— 于是"改了却报 no such field"。预热更甚：关掉预热时
    // 截止时刻那四项都不在文件里，要打开预热就必须能插进去。
    let src = "&nl_colm\n   DEF_CASE_NAME = 'x'\n/\n&nl_colm_forcing\n   DEF_forcing%dataset = 'POINT'\n/\n";
    let mut d = crate::parse(src).unwrap();
    d.insert(
        "DEF_simulation_time%spinup_year",
        crate::Value::Int(2003),
        "nl_colm",
    )
    .unwrap();
    let out = d.to_string();
    // 插在**自己那个组**的 `/` 之前，不是文件末尾。
    let at = out.find("spinup_year").unwrap();
    assert!(at < out.find("&nl_colm_forcing").unwrap(), "{out}");
    assert!(
        out.contains("   DEF_simulation_time%spinup_year = 2003"),
        "{out}"
    );
    // 别的行一个字节都没动。
    assert!(out.contains("   DEF_forcing%dataset = 'POINT'"));

    // 已经在文件里的字段走 set，不会插出第二行来。
    d.insert("DEF_CASE_NAME", crate::Value::Str("y".into()), "nl_colm")
        .unwrap();
    let out = d.to_string();
    assert_eq!(out.matches("DEF_CASE_NAME").count(), 1, "{out}");
    assert!(out.contains("'y'"));

    // 组不在文件里就报错 —— 静默插到别处等于没设，而 CoLM 按组读。
    let e = d
        .insert(
            "DEF_hist_vars%xy",
            crate::Value::Bool(true),
            "nl_colm_history",
        )
        .expect_err("没有那个组就该报错");
    assert!(e.to_string().contains("nl_colm_history"), "{e}");
}

#[test]
fn insert_refuses_to_update_a_field_in_the_wrong_group() {
    let src = "&nl_a\n   DUP = 1\n/\n&nl_b\n/\n";
    let mut d = crate::parse(src).unwrap();
    let e = d
        .insert("DUP", crate::Value::Int(2), "nl_b")
        .expect_err("不能把 nl_a 里的 DUP 当成 nl_b 字段改掉");
    assert!(e.to_string().contains("nl_b"), "{e}");
    assert!(d.to_string().contains("DUP = 1"), "{}", d);
}

#[test]
fn insert_updates_the_requested_group_when_names_repeat() {
    let src = "&nl_a\n   DUP = 1\n/\n&nl_b\n   DUP = 2\n/\n";
    let mut d = crate::parse(src).unwrap();
    d.insert("DUP", crate::Value::Int(3), "nl_b").unwrap();
    let out = d.to_string();
    assert!(out.contains("&nl_a\n   DUP = 1"), "{out}");
    assert!(out.contains("&nl_b\n   DUP = 3"), "{out}");
}

#[test]
fn removes_only_the_requested_subscripted_assignment() {
    let mut d =
        crate::parse("&nl_colm\n DEF_PFT_HTOP0(2)=17.0\n DEF_PFT_HTOP0(3)=18.0\n/\n").unwrap();
    assert!(d.remove("DEF_PFT_HTOP0(2)").unwrap());
    assert!(!d.remove("DEF_PFT_HTOP0(2)").unwrap());
    assert!(d.get("DEF_PFT_HTOP0(2)").is_none());
    assert!(d.get("DEF_PFT_HTOP0(3)").is_some());
}

#[test]
fn reads_and_updates_the_last_assignment_like_fortran() {
    let src = "&nl_colm\n a = 1 ! earlier\n A = 2 ! effective\n/\n";
    let mut d = crate::parse(src).unwrap();
    assert_eq!(d.get("a"), Some(&crate::Value::Int(2)));
    d.set("a", crate::Value::Int(3)).unwrap();
    assert_eq!(d.to_string(), src.replace("A = 2", "A = 3"));
    d.insert("a", crate::Value::Int(4), "nl_colm").unwrap();
    assert_eq!(d.to_string(), src.replace("A = 2", "A = 4"));
    assert_eq!(d.get("a"), Some(&crate::Value::Int(4)));
}

#[test]
fn insertion_accepts_comments_on_group_delimiters() {
    let src = "&nl_colm ! case settings\n a = 1 ! comment ending in &\n/ ! end\n";
    let mut d = crate::parse(src).unwrap();
    assert_eq!(d.to_string(), src);
    d.insert("b", crate::Value::Int(2), "nl_colm").unwrap();
    assert_eq!(d.to_string(), src.replace("/ ! end", "   b = 2\n/ ! end"));
}

#[test]
fn a_missing_group_can_be_appended_explicitly_before_inserting() {
    // 上游把材料拆进 `&nl_colm_material`，而建例只写 `&nl_colm`；用户第一次在
    // 已有算例上开材料时，插入路径必须先显式补一个空组（2026-11-12 实测：
    // 不补会报 "this namelist has no group &nl_colm_material"）。
    let src = "&nl_colm\n   DEF_CASE_NAME = 'x'\n/\n";
    let mut d = crate::parse(src).unwrap();
    d.ensure_group("nl_colm_material").unwrap();
    let out = d.to_string();
    assert!(out.starts_with(src), "原有内容必须逐字节保留：{out}");
    assert!(out.contains("&nl_colm_material\n/\n"), "{out}");
    d.insert(
        "DEF_USE_METHANE",
        crate::Value::Bool(true),
        "nl_colm_material",
    )
    .unwrap();
    let out = d.to_string();
    let at = out.find("DEF_USE_METHANE").unwrap();
    assert!(at > out.find("&nl_colm_material").unwrap(), "{out}");
    assert_eq!(out.matches("&nl_colm_material").count(), 1, "{out}");
    // 重复调用不补第二个空组。
    d.ensure_group("nl_colm_material").unwrap();
    assert_eq!(d.to_string().matches("&nl_colm_material").count(), 1);
}

#[test]
fn an_existing_group_with_a_comment_is_not_duplicated() {
    let src = "&nl_colm\n/\n&nl_colm_material ! material settings\n/\n";
    let mut d = crate::parse(src).unwrap();
    d.ensure_group("NL_COLM_MATERIAL").unwrap();
    assert_eq!(d.to_string(), src);
}

#[test]
fn an_invalid_group_name_is_refused_instead_of_being_rendered() {
    let mut d = crate::parse("&nl_colm\n/\n").unwrap();
    for bad in ["", "1nl", "nl colm", "nl_colm\n&evil"] {
        assert!(d.ensure_group(bad).is_err(), "{bad:?}");
    }
    assert_eq!(d.to_string(), "&nl_colm\n/\n");
}
