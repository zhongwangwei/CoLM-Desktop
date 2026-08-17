use super::*;

#[test]
fn path_parses_a_plain_field() {
    let p = Path::parse("DEF_CASE_NAME").unwrap();
    assert_eq!(p.segments, vec![Segment::Field("DEF_CASE_NAME".into())]);
    assert_eq!(p.to_string(), "DEF_CASE_NAME");
}

#[test]
fn path_parses_a_derived_type_member() {
    // 50/55 个真实文件里有这种写法
    let p = Path::parse("DEF_forcing%dataset").unwrap();
    assert_eq!(
        p.segments,
        vec![
            Segment::Field("DEF_forcing".into()),
            Segment::Member("dataset".into())
        ]
    );
    assert_eq!(p.to_string(), "DEF_forcing%dataset");
}

#[test]
fn path_parses_a_subscript() {
    // 24/55 个真实文件里有这种写法，且正是 forcing namelist 必需的
    let p = Path::parse("DEF_forcing%fprefix(1)").unwrap();
    assert_eq!(
        p.segments,
        vec![
            Segment::Field("DEF_forcing".into()),
            Segment::Member("fprefix".into()),
            Segment::Index(1)
        ]
    );
    assert_eq!(p.to_string(), "DEF_forcing%fprefix(1)");
}

#[test]
fn path_rejects_a_slice_rather_than_guessing() {
    // 数组切片在 55 个文件里出现 0 次。不支持，且必须明确报错 ——
    // 猜一个语义比拒绝更危险。
    let e = Path::parse("DEF_x(1:3)").unwrap_err();
    assert!(format!("{e:#}").contains("slice"), "{e:#}");
}

#[test]
fn values_render_in_fortran_form() {
    assert_eq!(Value::Bool(true).to_string(), ".true.");
    assert_eq!(Value::Bool(false).to_string(), ".false.");
    assert_eq!(Value::Int(-8).to_string(), "-8");
    assert_eq!(Value::Str("POINT".into()).to_string(), "'POINT'");
}

#[test]
fn real_keeps_the_exact_text_it_was_read_from() {
    // 1800. 与 1800.0 与 1.8e3 在 Fortran 里等价，但往返必须还原原样，
    // 否则每次保存都会把用户的写法改掉，diff 里全是噪声。
    let v = Value::Real {
        text: "1800.".into(),
    };
    assert_eq!(v.to_string(), "1800.");
    assert_eq!(v.as_f64(), Some(1800.0));
}

#[test]
fn a_list_renders_space_separated_like_the_files_do() {
    // 26/55 个文件用空格分隔多字符串：vname = 'Tair' 'Qair'
    let v = Value::List(vec![Value::Str("Tair".into()), Value::Str("Qair".into())]);
    assert_eq!(v.to_string(), "'Tair' 'Qair'");
}
