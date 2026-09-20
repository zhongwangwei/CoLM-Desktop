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
            present.insert(v.name());
        }
    }

    let unclassified: Vec<&String> = present
        .iter()
        .filter(|v| !assigned.contains_key(*v))
        .collect();
    let stale: Vec<&String> = assigned.keys().filter(|v| !present.contains(*v)).collect();
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
