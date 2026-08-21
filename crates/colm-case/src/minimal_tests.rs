use super::*;

#[test]
fn a_value_equal_to_the_declared_default_is_omitted() {
    // DEF_Runoff_SCHEME 默认 3（Simple VIC）。算例照抄 3 就不必写。
    assert_eq!(is_default("DEF_Runoff_SCHEME", &Value::Int(3)), Some(true));
    assert_eq!(is_default("DEF_Runoff_SCHEME", &Value::Int(0)), Some(false));
}

#[test]
fn reals_compare_numerically_not_textually() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里是同一个数。按文本比的话
    // 生成的每个算例都会多带一行 timestep，纯噪声。
    for t in ["1800.", "1800.0", "1.8e3", " 1800. "] {
        assert_eq!(
            is_default(
                "DEF_simulation_time%timestep",
                &Value::Real { text: t.into() }
            ),
            Some(true),
            "{t} should equal the declared default 1800."
        );
    }
}

#[test]
fn the_two_hourly_sites_must_write_their_timestep() {
    // 实测 90 个强迫场里 US-Ne3 与 US-MMS 是 3600 秒。漏写的话模型按
    // 1800 秒推进而强迫场是 3600 秒 —— 跑得完，结果全错。
    assert_eq!(
        is_default(
            "DEF_simulation_time%timestep",
            &Value::Real {
                text: "3600.".into()
            }
        ),
        Some(false)
    );
}

#[test]
fn an_unknown_field_is_written_out_rather_than_dropped() {
    // schema 不认识它，可能是上游新加的，也可能是用户拼错了。
    // 两种情况都该让 CoLM 自己去表态 —— 它对未声明的变量会明确报
    // `Cannot match namelist object name` 然后停，那是有用的报错。
    // 静默丢弃则会让用户以为自己设了。
    assert_eq!(is_default("DEF_NOT_A_REAL_FIELD", &Value::Bool(true)), None);
    let f = vec![("DEF_NOT_A_REAL_FIELD".to_string(), Value::Bool(true))];
    assert_eq!(required(&f).len(), 1);
}

#[test]
fn the_single_point_block_is_understood() {
    // 里程碑 5b 之前 schema 认不得整个 SITE_ 段，这些会全部落进
    // 「不认识」而被无条件写出。这条守住那次修复没有回退。
    assert_eq!(
        is_default("USE_SITE_landtype", &Value::Bool(false)),
        Some(true)
    );
    assert_eq!(
        is_default("USE_SITE_landtype", &Value::Bool(true)),
        Some(false)
    );
    assert_eq!(is_default("SITE_landtype", &Value::Int(-1)), Some(true));
    assert_eq!(is_default("SITE_landtype", &Value::Int(10)), Some(false));
}

#[test]
fn required_keeps_the_order_it_was_given() {
    // 生成的 namelist 里字段顺序应当稳定，否则每次重生成都是一个大 diff。
    let f = vec![
        ("DEF_CASE_NAME".to_string(), Value::Str("X".into())),
        ("DEF_Runoff_SCHEME".to_string(), Value::Int(3)), // 等于默认，会被滤掉
        ("DEF_VEG_SNOW".to_string(), Value::Bool(false)),
    ];
    let r = required(&f);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].0, "DEF_CASE_NAME");
    assert_eq!(r[1].0, "DEF_VEG_SNOW");
}
