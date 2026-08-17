//! 校验 oracle/tolerances.toml 覆盖了黄金文件里的每一个变量。
//!
//! 用法: tier-check <golden.nc> [<golden.nc> ...]
//!
//! 这不是容差比较器（里程碑 1 只做逐位）。它保证 CoLM 增删 history 变量时
//! 分类不会静默变得不完整。

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Deserialize)]
struct Tolerances {
    tier0: Tier,
    tier1: Tier,
    tier2: Tier,
    tier3: Tier,
}

#[derive(Deserialize)]
struct Tier {
    variables: Vec<String>,
}

fn main() -> Result<()> {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        bail!("usage: tier-check <golden.nc> [...]");
    }
    let text = std::fs::read_to_string("oracle/tolerances.toml")
        .context("run from the repository root")?;
    let t: Tolerances = toml::from_str(&text)?;

    let mut assigned: BTreeMap<String, &str> = BTreeMap::new();
    let mut duplicates = Vec::new();
    for (tier, list) in [
        ("tier0", &t.tier0.variables),
        ("tier1", &t.tier1.variables),
        ("tier2", &t.tier2.variables),
        ("tier3", &t.tier3.variables),
    ] {
        for v in list {
            if let Some(prev) = assigned.insert(v.clone(), tier) {
                duplicates.push(format!("{v} in both {prev} and {tier}"));
            }
        }
    }

    let mut present: BTreeSet<String> = BTreeSet::new();
    for f in &files {
        let nc = netcdf::open(f).with_context(|| format!("cannot open {f}"))?;
        for v in nc.variables() {
            present.insert(v.name());
        }
    }

    let unclassified: Vec<&String> =
        present.iter().filter(|v| !assigned.contains_key(*v)).collect();
    let stale: Vec<&String> = assigned
        .keys()
        .filter(|v| !present.contains(*v))
        .collect();

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
        eprintln!("{} tier entry/entries name variables that no longer exist:", stale.len());
        for v in &stale {
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
