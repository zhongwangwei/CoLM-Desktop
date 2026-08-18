
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
    assert!(out.contains("   DEF_simulation_time%spinup_year = 2003"), "{out}");
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
        .insert("DEF_hist_vars%xy", crate::Value::Bool(true), "nl_colm_history")
        .expect_err("没有那个组就该报错");
    assert!(e.to_string().contains("nl_colm_history"), "{e}");
}
