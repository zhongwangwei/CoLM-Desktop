use super::*;

/// `oracle/scripts/binary128/gen.f90 600 20261005` 的输出（gfortran `real(16)`）。
const FIXTURE: &str = include_str!("../tests/data/binary128_gfortran.txt");

fn quad(hi: &str, lo: &str) -> Quad {
    let hi = u64::from_str_radix(hi, 16).expect("hex");
    let lo = u64::from_str_radix(lo, 16).expect("hex");
    Quad((u128::from(hi) << 64) | u128::from(lo))
}

/// 逐行核对加、减、乘、除、转 f64 与比较；返回不一致的行数与第一条不一致。
fn check(text: &str) -> (usize, usize, Option<String>) {
    let mut lines = 0;
    let mut bad = 0;
    let mut first = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        lines += 1;
        let f: Vec<&str> = line.split_whitespace().collect();
        let a = quad(f[0], f[1]);
        let b = quad(f[2], f[3]);
        let expect = [
            ("add", a + b, quad(f[4], f[5])),
            ("sub", a - b, quad(f[6], f[7])),
            ("mul", a * b, quad(f[8], f[9])),
            ("div", a / b, quad(f[10], f[11])),
        ];
        let mut wrong = expect
            .iter()
            .filter(|(_, got, want)| got != want)
            .map(|(op, got, want)| format!("{op}: got {:032X} want {:032X}", got.0, want.0))
            .collect::<Vec<_>>();
        let f64_bits = u64::from_str_radix(f[12], 16).expect("hex");
        if a.to_f64().to_bits() != f64_bits {
            wrong.push(format!(
                "to_f64: got {:016X} want {f64_bits:016X}",
                a.to_f64().to_bits()
            ));
        }
        let cmp: i32 = f[13].parse().expect("cmp");
        let got_cmp = match a.partial_cmp(&b) {
            Some(Ordering::Less) => -1,
            Some(Ordering::Greater) => 1,
            _ => 0,
        };
        if got_cmp != cmp {
            wrong.push(format!("cmp: got {got_cmp} want {cmp}"));
        }
        if !wrong.is_empty() {
            bad += 1;
            first.get_or_insert_with(|| format!("{line}\n  {}", wrong.join("\n  ")));
        }
    }
    (lines, bad, first)
}

#[test]
fn matches_gfortran_real16_bit_for_bit() {
    let (lines, bad, first) = check(FIXTURE);
    assert!(lines >= 600);
    assert_eq!(bad, 0, "first mismatch:\n{}", first.unwrap_or_default());
}

/// 大规模核对：`BINARY128_VECTORS=<file> cargo test -p colm-core --release -- --ignored binary128`。
#[test]
#[ignore = "needs a large gfortran vector file"]
fn matches_gfortran_real16_on_a_large_vector_file() {
    let path = std::env::var("BINARY128_VECTORS").expect("BINARY128_VECTORS");
    let text = std::fs::read_to_string(path).expect("vector file");
    let (lines, bad, first) = check(&text);
    assert_eq!(
        bad,
        0,
        "{bad} of {lines} lines differ; first:\n{}",
        first.unwrap_or_default()
    );
}

#[test]
fn f64_round_trip_is_exact() {
    for value in [
        0.0,
        -0.0,
        1.0,
        -2.5,
        f64::MIN_POSITIVE,
        5e-324,
        f64::MAX,
        1.0 / 3.0,
    ] {
        assert_eq!(Quad::from_f64(value).to_f64().to_bits(), value.to_bits());
    }
}
