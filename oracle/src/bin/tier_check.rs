//! 校验 oracle/tolerances.toml 覆盖了黄金文件里的每一个变量。
//!
//! 用法: tier-check <golden.nc> [<golden.nc> ...]
//!
//! 这不是容差比较器（里程碑 1 只做逐位）。它保证 CoLM 增删 history 变量时
//! 分类不会静默变得不完整。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use oracle::tolerances::Tolerances;

const TIER_DEPENDENCIES: &[(&str, &[&str])] = &[
    ("f_z0m", &["f_tleaf", "f_t_grnd"]),
    ("f_lai", &["f_sigf"]),
    ("f_sai", &["f_sigf"]),
    ("f_sigf", &["f_scv", "f_snowdp"]),
    ("f_fsno", &["f_scv", "f_snowdp"]),
    ("f_olrg", &["f_t_soisno", "f_tleaf", "f_t_grnd"]),
    ("f_trad", &["f_t_soisno", "f_tleaf", "f_t_grnd"]),
];

fn main() -> Result<()> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        bail!("usage: tier-check <golden.nc> [...]");
    }
    let t = Tolerances::load("oracle/tolerances.toml").context("run from the repository root")?;

    // 重复归属由 `Tolerances::parse` 直接拒绝，这里不再重复检查。
    let mut assigned: BTreeMap<String, String> = BTreeMap::new();
    for variable in t.variables() {
        let (tier, _) = t
            .rule_for(variable)
            .expect("the variable came from the table");
        assigned.insert(variable.to_string(), tier);
    }
    let duplicates: Vec<String> = Vec::new();

    let mut present: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        let nc = netcdf::open(f).with_context(|| format!("cannot open {f}"))?;
        for v in nc.variables() {
            // 坐标/维度变量（`lat`/`lon`/`time`/`soil`/`soilsnow`/`vegnodes`/…）
            // **要收**：容差表里给它们单列了一组（`tolerances.toml` 的轴那一行），
            // 逐位比的是两边各自写出的坐标轴。漏收会让它们被判成"表里的名字不存在"。
            present.insert(v.name());
        }
    }

    let conditional: BTreeSet<String> = colm_hist::all()
        .iter()
        .filter(|v| v.runtime.is_some())
        .map(|v| format!("f_{}", v.name))
        .collect();
    let (unclassified, stale, unexercised) = classify(&assigned, &present, &conditional);
    let inversions = tier_dependency_inversions(&assigned);

    let mut bad = false;
    if !duplicates.is_empty() {
        eprintln!("variables assigned to more than one tier:");
        for d in &duplicates {
            eprintln!("  {d}");
        }
        bad = true;
    }
    if !unclassified.is_empty() {
        eprintln!(
            "{} variable(s) in the golden files have no tier assignment:",
            unclassified.len()
        );
        for v in &unclassified {
            eprintln!("  {v}");
        }
        eprintln!("add each to a tier in oracle/tolerances.toml (see design.md §8.1)");
        bad = true;
    }
    if !stale.is_empty() {
        eprintln!(
            "{} tier entry/entries name variables that no longer exist:",
            stale.len()
        );
        for v in &stale {
            eprintln!("  {v}");
        }
        bad = true;
    }
    if !unexercised.is_empty() {
        println!(
            "note: {} tier entry/entries are not produced by the supplied golden file(s) \
             (runtime-conditional, not an error):",
            unexercised.len()
        );
        for v in &unexercised {
            println!("  {v}");
        }
    }
    if !inversions.is_empty() {
        eprintln!("tolerance tier inversion(s): derived variable is stricter than its input:");
        for v in &inversions {
            eprintln!("  {v}");
        }
        bad = true;
    }
    if bad {
        bail!("tolerance classification is incomplete");
    }
    println!(
        "all {} golden variables have a tier assignment",
        present.len()
    );
    Ok(())
}

/// 分类完整性的三条判据，返回 `(黄金里有却无归属, 表里有却已不存在, 表里有但这次没跑到)`。
///
/// 第三条**不是错误**。原先 `stale` 只对着传进来的黄金文件判
/// （`!present.contains(v)`），于是 `f_qcharge` 被判成"变量没了"。它没消失，
/// 只是这几份黄金文件对应的那一次运行没有产出它：`MOD_Hist.F90:698` 把它写在
/// `IF (.not. DEF_USE_VariablySaturatedFlow)` 里，而 `oracle/golden/` 两份都是
/// `CN-Cng` 的默认配置、都走 VSF。"名字写错了"与"这个窗口没跑到"混在一个错误里，
/// 只会让人去删一个正确的条目。
///
/// 判据取并集：黄金文件里有的 ∪ 闸门表标了**运行时条件**的。前者保证"改名/删变量"
/// 照样抓得住（它两边都没有），后者放过"这个窗口没触发"。闸门表里那段条件原文
/// **不求值** —— 求值要么重写一个 namelist 解释器，要么在这里塞一份开关的副本，
/// 两条都不如"有运行时条件就不算缺失"来得诚实。
fn classify(
    assigned: &BTreeMap<String, String>,
    present: &BTreeSet<String>,
    conditional: &BTreeSet<String>,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let unclassified = present
        .iter()
        .filter(|v| !assigned.contains_key(*v))
        .cloned()
        .collect();
    let stale = assigned
        .keys()
        .filter(|v| !present.contains(*v) && !conditional.contains(*v))
        .cloned()
        .collect();
    let unexercised = assigned
        .keys()
        .filter(|v| !present.contains(*v) && conditional.contains(*v))
        .cloned()
        .collect();
    (unclassified, stale, unexercised)
}

fn tier_dependency_inversions(assigned: &BTreeMap<String, String>) -> Vec<String> {
    TIER_DEPENDENCIES
        .iter()
        .flat_map(|(derived, inputs)| {
            inputs.iter().filter_map(move |input| {
                let derived_tier = assigned.get(*derived)?;
                let input_tier = assigned.get(*input)?;
                (tier_rank(derived_tier) < tier_rank(input_tier)).then(|| {
                    format!("{derived} is {derived_tier} but input {input} is {input_tier}")
                })
            })
        })
        .collect()
}

fn tier_rank(tier: &str) -> u8 {
    match tier {
        "tier0" => 0,
        "tier1" => 1,
        "tier2" => 2,
        "tier3" => 3,
        _ => u8::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(xs: &[(&str, &str)]) -> BTreeMap<String, String> {
        xs.iter()
            .map(|(name, tier)| ((*name).to_string(), (*tier).to_string()))
            .collect()
    }

    fn set(xs: &[&str]) -> BTreeSet<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn a_runtime_conditional_tier_entry_is_not_mistaken_for_a_deleted_variable() {
        // `f_qcharge` 回归：它挂在 `IF (.not. DEF_USE_VariablySaturatedFlow)` 里
        // （`MOD_Hist.F90:698`），而 `oracle/golden/` 里两份黄金都走 VSF，
        // 所以它两边都不在 —— 这不是"变量没了"，是"这个窗口没跑到"。
        let assigned = map(&[("f_qcharge", "tier2"), ("f_tleaf", "tier2")]);
        let present = set(&["f_tleaf"]);
        let conditional = set(&["f_qcharge"]);
        let (unclassified, stale, unexercised) = classify(&assigned, &present, &conditional);
        assert!(unclassified.is_empty());
        assert!(
            stale.is_empty(),
            "f_qcharge must not be reported as deleted"
        );
        assert_eq!(unexercised, vec!["f_qcharge".to_string()]);
    }

    #[test]
    fn a_renamed_variable_is_still_reported_as_stale() {
        // 并集判据不能把"改名/删变量"一起放过去：它既不在黄金里，
        // 也没有运行时条件。
        let assigned = map(&[("f_typo", "tier2")]);
        let present = set(&["f_tleaf"]);
        let conditional = set(&["f_qcharge"]);
        let (unclassified, stale, unexercised) = classify(&assigned, &present, &conditional);
        assert_eq!(unclassified, vec!["f_tleaf".to_string()]);
        assert_eq!(stale, vec!["f_typo".to_string()]);
        assert!(unexercised.is_empty());
    }

    #[test]
    fn coordinate_axes_are_classifiable_too() {
        // 容差表给维度轴单列了一组（`band`/`lake`/`lat`/…），它们没有 `f_` 前缀，
        // 所以收取 `present` 时**不能**按前缀过滤，否则它们会被判成"表里的名字不存在"。
        let assigned = map(&[("lat", "tier0"), ("vegnodes", "tier0")]);
        let present = set(&["lat", "vegnodes"]);
        let (unclassified, stale, unexercised) = classify(&assigned, &present, &set(&[]));
        assert!(unclassified.is_empty() && stale.is_empty() && unexercised.is_empty());
    }

    #[test]
    fn tier_order_allows_equal_or_looser_derived_variables() {
        let assigned = map(&[
            ("f_sigf", "tier2"),
            ("f_scv", "tier2"),
            ("f_snowdp", "tier2"),
        ]);
        assert!(tier_dependency_inversions(&assigned).is_empty());
    }

    #[test]
    fn tier_order_rejects_a_minimal_inversion() {
        let assigned = map(&[("f_sigf", "tier1"), ("f_scv", "tier2")]);
        let bad = tier_dependency_inversions(&assigned);
        assert_eq!(bad, vec!["f_sigf is tier1 but input f_scv is tier2"]);
    }
}
