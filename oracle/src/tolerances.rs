//! `oracle/tolerances.toml` 的共享解析：分层规则与变量归属。
//!
//! `tier-check` 只关心"每个黄金变量都有归属"，而容差比较器还要拿规则去判值 ——
//! 两边各写一份解析就会漂开（一处改了 `rule` 的名字，另一处继续按旧名字放行）。
//! 所以类型放在库里，两个二进制都从 `load` 进来。
//!
//! 规则名与语义直接对应 `docs/design.md` §8.1：
//!
//! - `bitwise`：逐位相同，任何差异都是 bug；
//! - `relative`：`|a-b| <= rtol * max(|a|,|b|)`；
//! - `absolute_and_relative`：`|a-b| <= atol + rtol * max(|a|,|b|)`；
//! - `statistical`：整场统计判据，**逐变量不可判** —— 落在这一层的变量一律拒绝
//!   （`tolerances.toml` 里它当前没有变量，正是这个意思）。
//!
//! 认不出的 `rule` 报错：静默当成 bitwise 会让一个本该宽松的量变成硬失败，反过来
//! 静默放宽则更糟。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

/// 一个分层的判据。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rule {
    Bitwise,
    Relative { rtol: f64 },
    AbsoluteAndRelative { atol: f64, rtol: f64 },
    Statistical,
}

impl Rule {
    /// 两个值是否落在本规则内。整数永远逐位比（见 `compare_integers_only`）。
    pub fn accepts(self, golden: f64, produced: f64) -> bool {
        match self {
            Self::Bitwise => golden.to_bits() == produced.to_bits(),
            Self::Relative { rtol } => {
                (golden - produced).abs() <= rtol * golden.abs().max(produced.abs())
            }
            Self::AbsoluteAndRelative { atol, rtol } => {
                (golden - produced).abs() <= atol + rtol * golden.abs().max(produced.abs())
            }
            // 逐变量不可判：调用方在 `accepts` 之前就该拒绝它。
            Self::Statistical => false,
        }
    }

    /// 容差比较里用的"允许差值"，只用于报告。
    pub fn allowed(self, golden: f64, produced: f64) -> f64 {
        match self {
            Self::Bitwise => 0.0,
            Self::Relative { rtol } => rtol * golden.abs().max(produced.abs()),
            Self::AbsoluteAndRelative { atol, rtol } => {
                atol + rtol * golden.abs().max(produced.abs())
            }
            Self::Statistical => 0.0,
        }
    }

    pub fn label(self) -> String {
        match self {
            Self::Bitwise => "bitwise".to_string(),
            Self::Relative { rtol } => format!("relative(rtol={rtol:e})"),
            Self::AbsoluteAndRelative { atol, rtol } => {
                format!("absolute_and_relative(atol={atol:e}, rtol={rtol:e})")
            }
            Self::Statistical => "statistical".to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Document {
    tier0: Tier,
    tier1: Tier,
    tier2: Tier,
    tier3: Tier,
}

#[derive(Debug, Deserialize)]
struct Tier {
    rule: String,
    #[serde(default)]
    rtol: Option<f64>,
    #[serde(default)]
    atol: Option<f64>,
    variables: Vec<String>,
    /// 只有 `[tier3]` 有：整场统计判据的基线（`docs/design.md` §8.1 的 Tier 3）。
    #[serde(default)]
    baselines: Option<Baselines>,
}

/// Tier 3 的整场统计基线。
///
/// 与 `Rule::Statistical` 是一对：那个只说明"这一层不能用逐变量判据"，
/// 这个给出实际的门槛值。没有它，tier3 就只是一句"暂不判定"。
///
/// `history_suffixes` 是**判据的作用域**，不是装饰：`docs/design.md` §2.8 的
/// 冬季窗口自己那三行（Rnet R² 0.986 / Qle 0.047）是"冷启动无预热的预期值"，
/// 不是基线；拿 §2.8b 的 0.999/0.85 去卡它必然红，而那个红是**用法错**、
/// 不是回归。
///
/// 按**后缀**而不是全名认：同一个窗口的两个产出前缀不同 —— Fortran 那份是
/// `CN-Cng-wet_hist_2008-07.nc`，colm-rs 那份是 `colm-rs_hist_2008-07.nc`
/// （`colm-rs` 的 history stem 不带算例名，`--history-stem` 才带）。共同的
/// 只有时期后缀，而时期恰好就是窗口的定义。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Baselines {
    pub history_suffixes: Vec<String>,
    /// 剔除多少小时预热。它与基线同源，所以放在表里而不是调用方命令行上 ——
    /// n 会随它变，两边分开写迟早对不上。
    pub spinup_hours: f64,
    pub rnet_r2_min: f64,
    pub qle_r2_min: f64,
}

impl Baselines {
    /// 这份 history 文件名（不含目录）是否落在基线的作用域内。
    ///
    /// 只比文件名：`oracle/golden/` 与 `oracle/work/<case>/` 下的同名文件是
    /// 同一个窗口的两个产出，本就该用同一把尺子。
    pub fn covers(&self, history: &Path) -> bool {
        history
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                self.history_suffixes
                    .iter()
                    .any(|suffix| name.ends_with(suffix))
            })
    }
}

/// 分层表：变量 → (层名, 规则)，层名 → 规则。
#[derive(Debug, Clone)]
pub struct Tolerances {
    rule_of_variable: BTreeMap<String, (String, Rule)>,
    rules: BTreeMap<String, Rule>,
    /// 仓库表必须给 `[tier3.baselines]`；单元测试里的小表可以没有。
    tier3_baselines: Option<Baselines>,
}

impl Tolerances {
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        Self::parse(&text)
    }

    pub fn parse(text: &str) -> Result<Self> {
        let document: Document =
            toml::from_str(text).context("cannot parse the tolerance table")?;
        let mut rule_of_variable = BTreeMap::new();
        let mut rules = BTreeMap::new();
        let mut tier3_baselines = None;
        for (name, tier) in [
            ("tier0", document.tier0),
            ("tier1", document.tier1),
            ("tier2", document.tier2),
            ("tier3", document.tier3),
        ] {
            let rule = parse_rule(name, &tier)?;
            rules.insert(name.to_string(), rule);
            if name == "tier3" {
                tier3_baselines = tier.baselines;
            } else if tier.baselines.is_some() {
                bail!("{name} must not declare [baselines]; only tier3 has whole-run criteria");
            }
            for variable in tier.variables {
                if let Some((previous, _)) =
                    rule_of_variable.insert(variable.clone(), (name.to_string(), rule))
                {
                    bail!("{variable} is assigned to both {previous} and {name}");
                }
            }
        }
        Ok(Self {
            rule_of_variable,
            rules,
            tier3_baselines,
        })
    }

    /// `[tier3.baselines]`。仓库表里它必须存在 —— tier3 是设计文档 §8.1 的
    /// 一整层验收判据，缺了它那三个变量就没有判据，静默跳过等于假通过。
    pub fn tier3_baselines(&self) -> Result<Baselines> {
        self.tier3_baselines
            .clone()
            .context("the tolerance table has no [tier3.baselines] section")
    }

    /// 变量归属的层名与规则。没有归属就是错误 —— 静默放行一个未分类的变量，正是
    /// 容差表想避免的事。
    pub fn rule_for(&self, variable: &str) -> Result<(String, Rule)> {
        match self.rule_of_variable.get(variable) {
            Some((tier, rule)) => Ok((tier.clone(), *rule)),
            None => bail!("{variable} has no tier assignment in the tolerance table"),
        }
    }

    pub fn variables(&self) -> impl Iterator<Item = &str> {
        self.rule_of_variable.keys().map(String::as_str)
    }

    pub fn counts(&self) -> BTreeMap<String, usize> {
        let mut counts = BTreeMap::new();
        for (tier, _) in self.rule_of_variable.values() {
            *counts.entry(tier.clone()).or_insert(0) += 1;
        }
        for tier in self.rules.keys() {
            counts.entry(tier.clone()).or_insert(0);
        }
        counts
    }
}

fn parse_rule(tier: &str, entry: &Tier) -> Result<Rule> {
    match entry.rule.as_str() {
        "bitwise" => Ok(Rule::Bitwise),
        "relative" => Ok(Rule::Relative {
            rtol: entry
                .rtol
                .with_context(|| format!("{tier} uses rule relative but has no rtol"))?,
        }),
        "absolute_and_relative" => Ok(Rule::AbsoluteAndRelative {
            atol: entry.atol.with_context(|| {
                format!("{tier} uses rule absolute_and_relative but has no atol")
            })?,
            rtol: entry.rtol.with_context(|| {
                format!("{tier} uses rule absolute_and_relative but has no rtol")
            })?,
        }),
        "statistical" => Ok(Rule::Statistical),
        other => bail!("{tier} uses unknown rule {other:?}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = r#"
[tier0]
rule = "bitwise"
variables = ["f_a"]
[tier1]
rule = "relative"
rtol = 1e-12
variables = ["f_b"]
[tier2]
rule = "absolute_and_relative"
atol = 1e-7
rtol = 1e-7
variables = ["f_c"]
[tier3]
rule = "statistical"
variables = []
"#;

    #[test]
    fn rules_are_parsed_and_applied() {
        let t = Tolerances::parse(TABLE).unwrap();
        assert_eq!(t.rule_for("f_a").unwrap().1, Rule::Bitwise);
        assert_eq!(t.rule_for("f_b").unwrap().1, Rule::Relative { rtol: 1e-12 });
        assert_eq!(
            t.rule_for("f_c").unwrap().1,
            Rule::AbsoluteAndRelative {
                atol: 1e-7,
                rtol: 1e-7
            }
        );
        // 逐位：一位之差即失败。
        assert!(Rule::Bitwise.accepts(1.0, 1.0));
        assert!(!Rule::Bitwise.accepts(1.0, 1.0 + f64::EPSILON));
        // relative：1e-12 之内通过，之外失败。
        let rule = Rule::Relative { rtol: 1e-12 };
        assert!(rule.accepts(1.0, 1.0 + 1.0e-13));
        assert!(!rule.accepts(1.0, 1.0 + 1.0e-11));
        // absolute_and_relative：atol 把近零值也兜住。
        let rule = Rule::AbsoluteAndRelative {
            atol: 1e-7,
            rtol: 1e-7,
        };
        assert!(rule.accepts(0.0, 1.0e-8));
        assert!(!rule.accepts(0.0, 1.0e-6));
    }

    #[test]
    fn an_unassigned_variable_is_refused() {
        let t = Tolerances::parse(TABLE).unwrap();
        let error = t.rule_for("f_missing").unwrap_err();
        assert!(
            format!("{error:#}").contains("no tier assignment"),
            "{error:#}"
        );
    }

    #[test]
    fn a_duplicate_assignment_is_refused() {
        let table = TABLE.replace(r#"variables = ["f_b"]"#, r#"variables = ["f_a"]"#);
        let error = Tolerances::parse(&table).unwrap_err();
        assert!(
            format!("{error:#}").contains("both tier0 and tier1"),
            "{error:#}"
        );
    }

    #[test]
    fn an_unknown_rule_is_refused() {
        let table = TABLE.replace(r#"rule = "bitwise""#, r#"rule = "close_enough""#);
        let error = Tolerances::parse(&table).unwrap_err();
        assert!(format!("{error:#}").contains("unknown rule"), "{error:#}");
    }

    #[test]
    fn the_repository_table_parses_and_assigns_every_variable() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tolerances.toml");
        let t = Tolerances::load(&path).unwrap();
        let counts = t.counts();
        assert!(counts["tier0"] > 0 && counts["tier1"] > 0 && counts["tier2"] > 0);
        // tier3 当前没有逐变量判据 —— 它不该被当成"通过"。
        assert_eq!(counts["tier3"], 0);
        // 但它有整场判据，而且必须从表里读出来，不能内联在调用方。
        let baselines = t.tier3_baselines().unwrap();
        assert_eq!(baselines.rnet_r2_min, 0.998);
        assert_eq!(baselines.qle_r2_min, 0.85);
        assert_eq!(baselines.spinup_hours, 96.0);
        // 作用域必须点名湿季窗口，且**不能**点名冬季窗口 —— 后者没有基线。
        let covers = |name: &str| baselines.covers(Path::new(name));
        assert!(covers("/x/golden/CN-Cng-wet_hist_2008-07.nc"));
        assert!(covers("/tmp/wet/colm-rs_hist_2008-07.nc"));
        assert!(!covers("/x/golden/CN-Cng_hist_2008-01.nc"));
        assert!(!covers("/tmp/vsf6/colm-rs_hist_2008-01.nc"));
    }

    #[test]
    fn a_table_without_tier3_baselines_is_refused_by_the_reader() {
        let t = Tolerances::parse(TABLE).unwrap();
        let error = t.tier3_baselines().unwrap_err();
        assert!(
            format!("{error:#}").contains("no [tier3.baselines]"),
            "{error:#}"
        );
    }

    #[test]
    fn baselines_on_any_other_tier_are_refused() {
        // `[baselines]` 挂在 tier2 上：整场 R² 是 tier3 的判据，挂错了层
        // 会让 `tier3_baselines()` 悄悄读不到、而表看起来"有基线"。
        let table = TABLE.replace(
            "[tier2]",
            "[tier2.baselines]\nhistory_suffixes = [\"_x.nc\"]\nspinup_hours = 0.0\nrnet_r2_min = 1.0\nqle_r2_min = 1.0\n[tier2]",
        );
        let error = Tolerances::parse(&table).unwrap_err();
        assert!(
            format!("{error:#}").contains("must not declare [baselines]"),
            "{error:#}"
        );
    }
}
