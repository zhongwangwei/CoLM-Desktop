//! 按 `tolerances.toml` 的分层比较两个 history 文件。
//!
//! 与 [`crate::judge::compare`] 的分工：那个是**逐位**判官（里程碑 1 的判据），
//! 这个按变量查分层规则。schema 一侧两者一样严 —— 维度顺序、存储类型、变量级属性
//! 都必须相同，因为它们是文件契约，不是数值问题。
//!
//! 三条刻意的取值：
//!
//! 1. **整数永远逐位比。** 容差是给浮点的；把整型坐标或计数放进容差里，等于允许
//!    "差不多是同一层"。
//! 2. **没有归属的变量报错**，不算通过。安静地跳过它，正是容差表要防的那种失效。
//! 3. **`statistical` 层的变量报错**：整场统计判据逐变量不可判，`tolerances.toml`
//!    当前也刻意没给这一层分配任何变量。

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::tolerances::{Rule, Tolerances};

/// 一个超出容差的变量。
#[derive(Debug, Clone, PartialEq)]
pub struct Failure {
    pub variable: String,
    pub tier: String,
    pub rule: String,
    /// 超出容差的取值个数。
    pub differing: usize,
    /// 该变量比较了多少个值。
    pub total: usize,
    /// 偏差最大的一处：`(扁平下标, golden, produced, 允许差值)`。
    pub worst: Option<(usize, f64, f64, f64)>,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} [{} {}]: {}/{} values outside tolerance",
            self.variable, self.tier, self.rule, self.differing, self.total
        )?;
        if let Some((index, golden, produced, allowed)) = self.worst {
            write!(
                f,
                "; worst at index {index}: {golden:.17e} vs {produced:.17e} (allowed {allowed:.3e})"
            )?;
        }
        Ok(())
    }
}

/// 分层比较的结果。`failures`、`schema_problems`、`undecidable` 全空才算通过。
#[derive(Debug, Default)]
pub struct TierReport {
    pub compared: usize,
    pub dimensions: usize,
    /// 结构性问题：维度、类型、属性、变量集合。任何一条都是硬失败。
    pub schema_problems: Vec<String>,
    /// 超出容差的变量。
    pub failures: Vec<Failure>,
    /// 无法逐变量判定的变量（没有归属，或落在 `statistical` 层）。
    pub undecidable: Vec<String>,
}

impl TierReport {
    pub fn passed(&self) -> bool {
        self.schema_problems.is_empty() && self.failures.is_empty() && self.undecidable.is_empty()
    }
}

/// 按分层表比较两个文件。
pub fn compare_within(
    golden_path: &Path,
    produced_path: &Path,
    tolerances: &Tolerances,
) -> Result<TierReport> {
    let golden = netcdf::open(golden_path)
        .with_context(|| format!("cannot open {}", golden_path.display()))?;
    let produced = netcdf::open(produced_path)
        .with_context(|| format!("cannot open {}", produced_path.display()))?;
    let mut report = TierReport::default();

    for dimension in golden.dimensions() {
        let name = dimension.name();
        match produced.dimension(&name) {
            Some(other) if other.len() == dimension.len() => {}
            Some(other) => report.schema_problems.push(format!(
                "dimension {name}: golden {} vs produced {}",
                dimension.len(),
                other.len()
            )),
            None => report
                .schema_problems
                .push(format!("dimension only in golden: {name}")),
        }
    }
    for dimension in produced.dimensions() {
        if golden.dimension(&dimension.name()).is_none() {
            report
                .schema_problems
                .push(format!("dimension only in produced: {}", dimension.name()));
        }
    }
    report.dimensions = golden.dimensions().count();

    let names_golden: BTreeSet<String> = golden.variables().map(|v| v.name()).collect();
    let names_produced: BTreeSet<String> = produced.variables().map(|v| v.name()).collect();
    for name in names_golden.difference(&names_produced) {
        report
            .schema_problems
            .push(format!("variable only in golden: {name}"));
    }
    for name in names_produced.difference(&names_golden) {
        report
            .schema_problems
            .push(format!("variable only in produced: {name}"));
    }

    for name in names_golden.intersection(&names_produced) {
        let a = golden.variable(name).expect("name came from this file");
        let b = produced.variable(name).expect("name came from this file");
        // 维度顺序、类型都是契约：维度名与长度的有序列表必须相同（只比秩会漏掉
        // `patch = 1` 时前两轴对调这种换轴），存储类型也必须相同。
        let dims_a: Vec<(String, usize)> =
            a.dimensions().iter().map(|d| (d.name(), d.len())).collect();
        let dims_b: Vec<(String, usize)> =
            b.dimensions().iter().map(|d| (d.name(), d.len())).collect();
        if dims_a != dims_b {
            report
                .schema_problems
                .push(format!("{name}: dimensions {dims_a:?} vs {dims_b:?}"));
            continue;
        }
        if a.vartype() != b.vartype() {
            report.schema_problems.push(format!(
                "{name}: type {:?} vs {:?}",
                a.vartype(),
                b.vartype()
            ));
            continue;
        }
        let is_integer = matches!(a.vartype(), netcdf::types::NcVariableType::Int(_));
        if is_integer {
            // 整数不经 f64：相邻的 64 位 ID 在 2^53 之后会舍入成同一个值。
            let xa = a.get_raw_values(netcdf::Extents::All)?;
            let xb = b.get_raw_values(netcdf::Extents::All)?;
            if xa != xb {
                report.failures.push(Failure {
                    variable: name.clone(),
                    tier: "integers".to_string(),
                    rule: "bitwise".to_string(),
                    differing: xa
                        .iter()
                        .zip(xb.iter())
                        .filter(|(left, right)| left != right)
                        .count(),
                    total: xa.len(),
                    worst: None,
                });
            }
            report.compared += 1;
            continue;
        }

        let (tier, rule) = match tolerances.rule_for(name) {
            Ok(entry) => entry,
            Err(_) => {
                report
                    .undecidable
                    .push(format!("{name}: no tier assignment in the tolerance table"));
                continue;
            }
        };
        if rule == Rule::Statistical {
            report.undecidable.push(format!(
                "{name}: {tier} uses the statistical rule, which has no per-variable form"
            ));
            continue;
        }
        let xa = a.get_values::<f64, _>(netcdf::Extents::All)?;
        let xb = b.get_values::<f64, _>(netcdf::Extents::All)?;
        if xa.len() != xb.len() {
            report
                .schema_problems
                .push(format!("{name}: {} values vs {}", xa.len(), xb.len()));
            continue;
        }
        let mut differing = 0usize;
        let mut worst: Option<(usize, f64, f64, f64)> = None;
        let mut worst_ratio = 0.0f64;
        for (index, (golden_value, produced_value)) in xa.iter().zip(xb.iter()).enumerate() {
            if !golden_value.is_finite() || !produced_value.is_finite() {
                if golden_value.is_nan() && produced_value.is_nan() {
                    continue;
                }
                if golden_value != produced_value {
                    differing += 1;
                }
                continue;
            }
            if rule.accepts(*golden_value, *produced_value) {
                continue;
            }
            differing += 1;
            let allowed = rule.allowed(*golden_value, *produced_value);
            let excess = (*golden_value - *produced_value).abs();
            let ratio = if allowed > 0.0 {
                excess / allowed
            } else {
                f64::INFINITY
            };
            if worst.is_none() || ratio > worst_ratio {
                worst_ratio = ratio;
                worst = Some((index, *golden_value, *produced_value, allowed));
            }
        }
        if differing > 0 {
            report.failures.push(Failure {
                variable: name.clone(),
                tier,
                rule: rule.label(),
                differing,
                total: xa.len(),
                worst,
            });
        }
        report.compared += 1;
    }

    report.failures.sort_by(|left, right| {
        right
            .differing
            .cmp(&left.differing)
            .then_with(|| left.variable.cmp(&right.variable))
    });
    report.schema_problems.sort();
    report.undecidable.sort();
    Ok(report)
}

/// 打印分层比较的结果，并在不通过时返回错误。
pub fn report(report: &TierReport, tolerances: &Tolerances) -> Result<()> {
    let counts = tolerances
        .counts()
        .into_iter()
        .map(|(tier, count)| format!("{tier}={count}"))
        .collect::<Vec<_>>()
        .join(" ");
    if report.passed() {
        println!(
            "within tolerance: {} variables, {} dimensions ({counts})",
            report.compared, report.dimensions
        );
        return Ok(());
    }
    if !report.schema_problems.is_empty() {
        eprintln!("{} schema problem(s):", report.schema_problems.len());
        for problem in &report.schema_problems {
            eprintln!("  {problem}");
        }
    }
    if !report.undecidable.is_empty() {
        eprintln!(
            "{} variable(s) cannot be judged per variable:",
            report.undecidable.len()
        );
        for problem in &report.undecidable {
            eprintln!("  {problem}");
        }
    }
    if !report.failures.is_empty() {
        eprintln!("{} variable(s) outside tolerance:", report.failures.len());
        for failure in &report.failures {
            eprintln!("  {failure}");
        }
    }
    let by_tier: BTreeMap<String, usize> =
        report
            .failures
            .iter()
            .fold(BTreeMap::new(), |mut counts, failure| {
                *counts.entry(failure.tier.clone()).or_insert(0) += 1;
                counts
            });
    if !by_tier.is_empty() {
        eprintln!("failures by tier: {by_tier:?}");
    }
    bail!("tier comparison failed")
}
