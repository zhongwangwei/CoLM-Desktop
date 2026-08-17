use super::*;
use crate::check::MetSummary;
use crate::civil::Stamp;

fn spec() -> ForcingSpec {
    ForcingSpec {
        dir: "/data/PLUMBER2s/Forcing/".into(),
        file: "CN-Cng_2008-2009_FLUXNET2015_Met.nc".into(),
        met: MetSummary {
            time_units: "seconds since 2008-01-01 00:00:00".into(),
            start: Stamp {
                year: 2008,
                month: 1,
                day: 1,
                hour: 0,
                minute: 0,
                second: 0,
            },
            steps: 35041,
            step_seconds: 1800.0,
            step_uniform: true,
            height_v: 6.0,
            height_t: 6.0,
            height_q: 6.0,
            variables: crate::check::REQUIRED_VARS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        },
    }
}

/// 把生成的文本用 colm-namelist 解析回来，取一个字段。
///
/// 这样断言而不是比字符串，是因为要验的是**它说了什么**，不是它长什么样。
fn field(text: &str, path: &str) -> String {
    let doc = colm_namelist::parse(text).expect("our own output must parse");
    doc.get(path)
        .unwrap_or_else(|| panic!("{path} missing from:\n{text}"))
        .to_string()
}

#[test]
fn our_own_output_parses() {
    // 生成器写出的东西必须能被本仓库的解析器读回来。两边都是自己的代码，
    // 但它们是独立写的，互相验证比各自自证强。
    let text = render(&spec());
    colm_namelist::parse(&text).expect("must parse");
}

#[test]
fn the_slot_map_is_colms_fixed_one() {
    // 槽位固定为 1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW。
    // PLUMBER2 只有标量 Wind，所以第 5 槽是 NULL，Wind 进第 6 槽。
    let text = render(&spec());
    assert_eq!(
        field(&text, "DEF_forcing%vname"),
        "'Tair' 'Qair' 'Psurf' 'Precip' 'NULL' 'Wind' 'SWdown' 'LWdown'"
    );
    assert_eq!(
        field(&text, "DEF_forcing%tintalgo"),
        "'linear' 'linear' 'linear' 'nearest' 'NULL' 'linear' 'linear' 'linear'"
    );
}

#[test]
fn the_window_comes_from_the_time_axis_not_from_the_filename() {
    // 文件名里的 2008-2009 只是个约定；覆盖范围由时间轴决定。
    let text = render(&spec());
    assert_eq!(field(&text, "DEF_forcing%startyr"), "2008");
    assert_eq!(field(&text, "DEF_forcing%startmo"), "1");
    assert_eq!(field(&text, "DEF_forcing%endyr"), "2009");
    assert_eq!(field(&text, "DEF_forcing%endmo"), "12");
}

#[test]
fn only_the_first_fprefix_slot_is_written() {
    // POINT 下 CoLM 只读 fprefix(1)（MOD_UserSpecifiedForcing.F90:683）。
    // 先前的模板把 8 个槽都填成同一个文件名 —— 无害，但会让人以为
    // 它们各有用处。
    let text = render(&spec());
    assert_eq!(
        field(&text, "DEF_forcing%fprefix(1)"),
        "'CN-Cng_2008-2009_FLUXNET2015_Met.nc'"
    );
    // 只数字段本身；注释里也提到 fprefix，那是有意的。
    assert_eq!(text.matches("DEF_forcing%fprefix").count(), 1);
}

#[test]
fn the_heights_come_from_the_file_and_say_so() {
    // namelist 里的 HEIGHT_* 在 POINT 下会被文件里的 reference_height_*
    // 覆盖（MOD_Forcing.F90:294-310），所以这三行是给人看的。
    // 写文件里的真值而不是一个常数，才不会误导下一个读它的人。
    let mut s = spec();
    s.met.height_v = 12.1;
    s.met.height_t = 1.5;
    s.met.height_q = 1.5;
    let text = render(&s);
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_V"), "12.1");
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_T"), "1.5");
    assert!(
        text.contains("overwritten"),
        "the note about CoLM overwriting these must survive: {text}"
    );
}

#[test]
fn an_integer_valued_height_still_looks_like_a_real() {
    // Rust 的 Display 把 6.0 打成 "6"，写进 namelist 就会被读成整数。
    // CoLM 那三个字段是 real(r8)，Fortran 读得进去，但逐字段比对时
    // Int(6) 与 Real("6.0") 是两回事 —— 而这正是 Task 8 要做的比对。
    // 实测不少站点的高度是整数值（AU-Lit 是 31 / 33 / 33）。
    let mut s = spec();
    s.met.height_v = 6.0;
    s.met.height_t = 33.0;
    let text = render(&s);
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_V"), "6.0");
    assert_eq!(field(&text, "DEF_forcing%HEIGHT_T"), "33.0");
}

#[test]
fn the_constants_colm_needs_are_present() {
    let text = render(&spec());
    assert_eq!(field(&text, "DEF_forcing%dataset"), "'POINT'");
    assert_eq!(field(&text, "DEF_forcing%NVAR"), "8");
    assert_eq!(field(&text, "DEF_forcing%solarin_all_band"), ".true.");
    assert_eq!(
        field(&text, "DEF_dir_forcing"),
        "'/data/PLUMBER2s/Forcing/'"
    );
}

#[test]
fn a_directory_without_a_trailing_slash_still_works() {
    // CoLM 拼路径是 dir//fprefix，中间不补斜杠。少一个斜杠会让它去找
    // ForcingCN-Cng_....nc，报的错与真正的原因无关。
    let mut s = spec();
    s.dir = "/data/PLUMBER2s/Forcing".into();
    let text = render(&s);
    assert_eq!(
        field(&text, "DEF_dir_forcing"),
        "'/data/PLUMBER2s/Forcing/'"
    );
}
