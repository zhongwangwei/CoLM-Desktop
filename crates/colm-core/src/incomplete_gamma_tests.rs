//! 与 gfortran 编译的 `MOD_IncompleteGamma.F90` 逐位比对。
//!
//! 金标准 `tests/data/incomplete_gamma_gfortran.txt` 由一个小 Fortran 驱动生成：
//! 用 `gfortran -fdefault-real-8 -fdefault-double-8 -ffree-form -O2` 编译 vendor
//! 模块（其 GIMPLE 与 CoLM 构建的转储逐条相同），对手挑的分支点和数千组随机
//! (a, x) 调 `GRATIO`，并直接调模块内的 `GAMMA/GAM1/RLOG/REXP/ERF/ERFC1/GLOG`，
//! 输出十六进制位模式。

use super::*;

const FIXTURE: &str = include_str!("../tests/data/incomplete_gamma_gfortran.txt");
const SENTINEL: u64 = 0x7FF8_DEAD_BEEF_0001;

fn h64(s: &str) -> u64 {
    u64::from_str_radix(s, 16).expect("十六进制 f64 位")
}

fn hf(s: &str) -> f64 {
    f64::from_bits(h64(s))
}

/// macOS 逐位、其它平台 16 ULP（见 [`crate::reference_bits_match`]）。
fn close(got: f64, want: u64) -> bool {
    crate::reference_bits_match(got, f64::from_bits(want), 0.0)
}

fn records(tag: &str) -> Vec<Vec<&'static str>> {
    FIXTURE
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| l.split_whitespace().collect::<Vec<_>>())
        .filter(|f| f[0].starts_with(tag))
        .collect()
}

/// 汇报并断言不一致数为 0。
fn report(name: &str, total: usize, bad: &[String]) {
    eprintln!("{name}: {total} 组，按位不一致 {} 组", bad.len());
    for b in bad.iter().take(10) {
        eprintln!("  {b}");
    }
    assert!(total > 0, "{name}: 金标准中没有数据");
    assert!(
        bad.is_empty(),
        "{name}: {} / {total} 组与 gfortran 不一致",
        bad.len()
    );
}

#[test]
fn gratio_matches_gfortran_bitwise() {
    let rows = records("P");
    let mut bad = Vec::new();
    let mut errors = 0;
    for f in &rows {
        // P ind a x ans qans
        let ind: i32 = f[1].parse().unwrap();
        let (a, x) = (hf(f[2]), hf(f[3]));
        let (want_p, want_q) = (h64(f[4]), h64(f[5]));
        let mut ans = f64::from_bits(SENTINEL);
        let mut qans = f64::from_bits(SENTINEL);
        gratio_fortran(a, x, &mut ans, &mut qans, ind);
        if !(close(ans, want_p) && close(qans, want_q)) {
            bad.push(format!(
                "ind={ind} a={a:e} x={x:e}: P {:016X} vs {want_p:016X}, Q {:016X} vs {want_q:016X}",
                ans.to_bits(),
                qans.to_bits()
            ));
        }
        if ind == 0 {
            let (p, q) = gratio(a, x);
            if want_p == 2.0_f64.to_bits() {
                errors += 1;
                assert_eq!(p, 2.0);
                assert!(q.is_nan());
            } else if !(close(p, want_p) && close(q, want_q)) {
                bad.push(format!("gratio({a:e}, {x:e}) 与 gratio_fortran 不一致"));
            }
        }
    }
    eprintln!("其中 IND=0 出错返回（ANS=2）{errors} 组");
    report("GRATIO", rows.len(), &bad);
    assert!(rows.len() > 5000);
}

#[test]
fn gratio_representative_points() {
    // 取自金标准的代表点，覆盖 a<1、A=0.5、整数 A、连分式、Temme 与出错返回。
    let cases: [(f64, f64, u64, u64); 6] = [
        (hf("3FE0000000000000"), hf("3FB999999999999A"), 0, 0), // A = 0.5, ERF 分支
        (1.0, 1.0, 0, 0),
        (
            hf("401F9FB72237A12C"),
            hf("4011146EDA85A193"),
            0x3FB2EECDC6AEF70D,
            0x3FEDA226472A211E,
        ),
        (
            hf("401D182BF05ADB0F"),
            hf("3FF7061DAA3A204A"),
            0x3F3E12D7A469F520,
            0x3FEFFC3DA50B72C2,
        ),
        (
            hf("412E848000000000"),
            hf("412E8C4FFFFFFFFF"),
            0x3FEAEC4BE6C3959D,
            0x3FC44ED064F1A98C,
        ),
        (
            hf("40F20CBF7F557790"),
            hf("40F20CBF7F56E660"),
            0x3FE00401A98D5112,
            0x3FDFF7FCACE55DDC,
        ),
    ];
    for (a, x, p_bits, q_bits) in cases {
        let (p, q) = gratio(a, x);
        if p_bits != 0 {
            assert_eq!((p.to_bits(), q.to_bits()), (p_bits, q_bits), "a={a} x={x}");
        }
        assert!((p + q - 1.0).abs() < 1e-14, "a={a} x={x}: P+Q={}", p + q);
    }
    // P(1, x) = 1 - exp(-x)
    let (p, _) = gratio(1.0, 1.0);
    assert!((p - (1.0 - (-1.0_f64).exp())).abs() < 1e-14);
    // 出错返回：只写 ANS，QANS 保留旧值
    let mut ans = 0.0;
    let mut qans = 0.25;
    gratio_fortran(1.0, -1.0, &mut ans, &mut qans, 0);
    assert_eq!((ans, qans), (2.0, 0.25));
}

fn check_unary(tag: &str, name: &str, f: impl Fn(f64) -> f64) {
    let rows = records(tag);
    let mut bad = Vec::new();
    for r in &rows {
        let x = hf(r[1]);
        let want = h64(r[2]);
        let got = f(x).to_bits();
        if !close(f64::from_bits(got), want) {
            bad.push(format!("{name}({x:e}) = {got:016X}，gfortran {want:016X}"));
        }
    }
    report(name, rows.len(), &bad);
}

#[test]
fn gamma_matches_gfortran_bitwise() {
    check_unary("G", "GAMMA", gamma);
}

#[test]
fn gam1_matches_gfortran_bitwise() {
    check_unary("H", "GAM1", gam1);
}

#[test]
fn rlog_matches_gfortran_bitwise() {
    check_unary("L", "RLOG", rlog);
}

#[test]
fn rexp_matches_gfortran_bitwise() {
    check_unary("E", "REXP", rexp);
}

#[test]
fn erf_matches_gfortran_bitwise() {
    check_unary("F", "ERF", erf);
}

#[test]
fn erfc1_matches_gfortran_bitwise() {
    let rows = records("C");
    let mut bad = Vec::new();
    for r in &rows {
        let ind: i32 = r[0][1..].parse().unwrap();
        let x = hf(r[1]);
        let want = h64(r[2]);
        let got = erfc1(ind, x).to_bits();
        if !close(f64::from_bits(got), want) {
            bad.push(format!(
                "ERFC1({ind}, {x:e}) = {got:016X}，gfortran {want:016X}"
            ));
        }
    }
    report("ERFC1", rows.len(), &bad);
}

#[test]
fn glog_matches_gfortran_bitwise() {
    check_unary("N", "GLOG", glog);
}
