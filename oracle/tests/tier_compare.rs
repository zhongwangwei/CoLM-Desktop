//! 分层比较器的行为测试。
//!
//! 与 `judge.rs` 同样的理由：一个只会说"在容差内"的比较器比没有更糟 —— 它会让
//! 回归带着绿色的 CI 通过。所以每一类规则、每一条拒绝路径各写一条负向测试。
//!
//! 测试自造小文件，不用黄金文件：跑得快，也不会因为黄金文件重新生成而失效。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use oracle::tier_compare::compare_within;
use oracle::tolerances::{Rule, Tolerances};

const TABLE: &str = r#"
[tier0]
rule = "bitwise"
variables = ["f_bit"]
[tier1]
rule = "relative"
rtol = 1e-12
variables = ["f_rel", "f_int"]
[tier2]
rule = "absolute_and_relative"
atol = 1e-7
rtol = 1e-7
variables = ["f_abs"]
[tier3]
rule = "statistical"
variables = ["f_stat"]
"#;

fn workdir(name: &str) -> PathBuf {
    let directory = std::env::temp_dir().join(format!(
        "oracle-tier-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("create workdir");
    directory
}

/// 一个变量：名字、浮点的三个值，或整数的三个值。
enum Values {
    Float([f64; 3]),
    Integer([i32; 3]),
}

fn write_file(path: &Path, entries: &[(&str, Values)]) {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
    let mut file = netcdf::create(path).expect("create");
    file.add_dimension("time", 3).unwrap();
    file.add_dimension("patch", 1).unwrap();
    for (name, values) in entries {
        match values {
            Values::Float(values) => {
                let mut variable = file.add_variable::<f64>(name, &["time", "patch"]).unwrap();
                variable.put_values(values, netcdf::Extents::All).unwrap();
                variable.put_attribute("units", "W/m2").unwrap();
            }
            Values::Integer(values) => {
                let mut variable = file.add_variable::<i32>(name, &["time", "patch"]).unwrap();
                variable.put_values(values, netcdf::Extents::All).unwrap();
                variable.put_attribute("units", "1").unwrap();
            }
        }
    }
}

/// 造一对文件：基准用给定取值，对照把同一个位置改成另一个值。
fn pair(name: &str, golden: f64, produced: f64) -> (PathBuf, PathBuf) {
    let directory = workdir(name);
    let a = directory.join("golden.nc");
    let b = directory.join("produced.nc");
    let mut base = [0.0f64; 3];
    base[0] = golden;
    let mut other = base;
    other[0] = produced;
    let entries = |values: [f64; 3]| -> Vec<(&'static str, Values)> {
        vec![
            ("f_bit", Values::Float(values)),
            ("f_rel", Values::Float(values)),
            ("f_abs", Values::Float(values)),
            ("f_stat", Values::Float(values)),
            ("f_int", Values::Integer([1, 2, 3])),
        ]
    };
    write_file(&a, &entries(base));
    write_file(&b, &entries(other));
    (a, b)
}

/// 把某个变量从表里去掉，用来测"没有归属"这条拒绝路径。
fn table_without(variable: &str) -> Tolerances {
    let table = TABLE.replace(&format!("\"{variable}\", "), "");
    Tolerances::parse(&table).unwrap()
}

#[test]
fn identical_values_pass_every_tier() {
    let (a, b) = pair("identical", 1.0, 1.0);
    let tolerances = Tolerances::parse(TABLE).unwrap();
    let report = compare_within(&a, &b, &tolerances).unwrap();
    // `f_stat` 落在 statistical 层，逐变量不可判 —— 所以即使取值相同也不通过，
    // 这正是"不把统计层当成通过"的意思。
    assert_eq!(report.undecidable.len(), 1, "{:?}", report.undecidable);
    assert!(report.failures.is_empty());
    assert!(report.schema_problems.is_empty());
    assert_eq!(report.compared, 4);
}

#[test]
fn a_relative_rule_accepts_within_and_rejects_beyond() {
    let tolerances = Tolerances::parse(TABLE).unwrap();
    // 1e-13 相对差：在 rtol=1e-12 之内。
    let (a, b) = pair("rel-pass", 1.0, 1.0 + 1.0e-13);
    let report = compare_within(&a, &b, &tolerances).unwrap();
    // 同一个改动会写进每个变量，所以 `f_bit`（逐位层）必然失败；这里只问
    // 相对层与绝对层有没有放过它。
    let names: Vec<&str> = report
        .failures
        .iter()
        .map(|failure| failure.variable.as_str())
        .collect();
    assert!(
        !names.contains(&"f_rel") && !names.contains(&"f_abs"),
        "1e-13 should pass rtol=1e-12: {:?}",
        report.failures
    );
    // 1e-9 相对差：超出。
    let (a, b) = pair("rel-fail", 1.0, 1.0 + 1.0e-9);
    let report = compare_within(&a, &b, &tolerances).unwrap();
    let names: Vec<&str> = report
        .failures
        .iter()
        .map(|failure| failure.variable.as_str())
        .collect();
    assert!(names.contains(&"f_rel"), "{:?}", report.failures);
    // `f_bit` 是逐位层，同样会失败；把它的层名报出来。
    let bit = report
        .failures
        .iter()
        .find(|failure| failure.variable == "f_bit")
        .expect("bitwise tier must fail on any difference");
    assert_eq!(bit.tier, "tier0");
}

#[test]
fn an_absolute_rule_covers_a_near_zero_value() {
    let tolerances = Tolerances::parse(TABLE).unwrap();
    // 基准 0 与 1e-8：相对规则会失败，`atol = 1e-7` 兜住。
    let (a, b) = pair("abs-pass", 0.0, 1.0e-8);
    let report = compare_within(&a, &b, &tolerances).unwrap();
    let abs = report
        .failures
        .iter()
        .find(|failure| failure.variable == "f_abs");
    assert!(
        abs.is_none(),
        "atol should cover 1e-8: {:?}",
        report.failures
    );
    // 1e-5 远超 atol。
    let (a, b) = pair("abs-fail", 0.0, 1.0e-5);
    let report = compare_within(&a, &b, &tolerances).unwrap();
    assert!(report
        .failures
        .iter()
        .any(|failure| failure.variable == "f_abs"));
}

#[test]
fn integers_are_compared_bitwise_even_under_a_float_tier() {
    let directory = workdir("integer");
    let a = directory.join("golden.nc");
    let b = directory.join("produced.nc");
    write_file(&a, &[("f_int", Values::Integer([1, 2, 3]))]);
    write_file(&b, &[("f_int", Values::Integer([1, 2, 4]))]);
    let tolerances = Tolerances::parse(TABLE).unwrap();
    let report = compare_within(&a, &b, &tolerances).unwrap();
    let failure = report
        .failures
        .iter()
        .find(|failure| failure.variable == "f_int")
        .expect("an integer difference must fail");
    assert_eq!(failure.rule, "bitwise");
    assert_eq!(failure.differing, 1);
}

#[test]
fn an_unassigned_variable_is_undecidable_not_passing() {
    let (a, b) = pair("unassigned", 1.0, 1.0);
    // `f_rel` 在表里带尾逗号，去掉它之后的表仍然是合法的 TOML。
    let tolerances = table_without("f_rel");
    let report = compare_within(&a, &b, &tolerances).unwrap();
    assert!(!report.passed());
    assert!(
        report
            .undecidable
            .iter()
            .any(|entry| entry.contains("f_rel") && entry.contains("no tier assignment")),
        "{:?}",
        report.undecidable
    );
}

#[test]
fn a_statistical_tier_variable_is_undecidable() {
    let (a, b) = pair("statistical", 1.0, 1.0);
    let tolerances = Tolerances::parse(TABLE).unwrap();
    let report = compare_within(&a, &b, &tolerances).unwrap();
    assert!(report
        .undecidable
        .iter()
        .any(|entry| entry.contains("f_stat") && entry.contains("statistical")));
}

#[test]
fn a_schema_difference_is_reported_even_when_the_values_agree() {
    let directory = workdir("schema");
    let a = directory.join("golden.nc");
    let b = directory.join("produced.nc");
    write_file(&a, &[("f_bit", Values::Float([1.0, 2.0, 3.0]))]);
    // 同一批值，但轴序换了 —— 与判官那边同样的理由：`patch = 1` 时扁平序不变。
    {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        let _guard = LOCK.get_or_init(|| Mutex::new(())).lock().unwrap();
        let mut file = netcdf::create(&b).unwrap();
        file.add_dimension("time", 3).unwrap();
        file.add_dimension("patch", 1).unwrap();
        let mut variable = file
            .add_variable::<f64>("f_bit", &["patch", "time"])
            .unwrap();
        variable
            .put_values(&[1.0f64, 2.0, 3.0], netcdf::Extents::All)
            .unwrap();
    }
    let tolerances = Tolerances::parse(TABLE).unwrap();
    let report = compare_within(&a, &b, &tolerances).unwrap();
    assert!(!report.schema_problems.is_empty());
    assert!(report
        .schema_problems
        .iter()
        .any(|problem| problem.contains("dimensions")));
}

#[test]
fn the_repository_table_assigns_rules_that_the_comparator_can_evaluate() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tolerances.toml");
    let tolerances = Tolerances::load(&path).unwrap();
    for variable in tolerances.variables() {
        let (tier, rule) = tolerances.rule_for(variable).unwrap();
        // 每一层要么可逐变量判，要么是刻意的 statistical（当前没有变量）。
        assert!(
            matches!(
                rule,
                Rule::Bitwise | Rule::Relative { .. } | Rule::AbsoluteAndRelative { .. }
            ) || tier == "tier3",
            "{variable} is in {tier} with rule {rule:?}"
        );
    }
}
