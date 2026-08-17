//! 比对两个 CoLM history 文件：变量数据、维度、属性。
//!
//! 不做字节比对。实测重跑会产生 8 字节差异，全部来自全局属性 create_time
//! （CoLM 写入的墙上时钟）；129 个变量的数据逐位相同。
//!
//! 本模块是库，`golden-compare` 只是它的薄壳。这样拆是因为判官已经两次
//! 静默放行过真实回归（第一次漏比变量级属性，第二次漏比逐变量维度顺序），
//! 而两次都只有靠手工命令才发现。库形态让 `oracle/tests/judge.rs` 能把
//! 每一类差异都钉成自动化测试。

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result};

/// 允许不同的属性名。**精确名单，不做前缀或通配匹配。**
/// 新增条目必须说明为什么该属性天然易变。
pub const VOLATILE_ATTRIBUTES: &[&str] = &[
    // CoLM 写入的文件创建墙上时钟，例如 "20260817-16:16:27 UTC+08:00"
    "create_time",
];

/// 一次比对的结果。`problems` 为空即视为相同。
#[derive(Debug)]
pub struct Report {
    pub problems: Vec<String>,
    /// 逐值比较完成的变量数（用于「identical: N variables」）
    pub compared: usize,
    /// 文件级维度数
    pub dimensions: usize,
}

impl Report {
    pub fn is_identical(&self) -> bool {
        self.problems.is_empty()
    }
}

/// 比对两个 NetCDF 文件，返回全部差异描述。
///
/// 打不开文件是 `Err`，不是「没有差异」—— 那个区别很重要：把读不到当成相同，
/// 正是回归门禁最典型的失效方式。
pub fn compare(a_path: &Path, b_path: &Path) -> Result<Report> {
    let a = netcdf::open(a_path).with_context(|| format!("cannot open {}", a_path.display()))?;
    let b = netcdf::open(b_path).with_context(|| format!("cannot open {}", b_path.display()))?;

    let mut problems: Vec<String> = Vec::new();

    // --- 维度 ---
    let dims_a: BTreeSet<(String, usize)> = a.dimensions().map(|d| (d.name(), d.len())).collect();
    let dims_b: BTreeSet<(String, usize)> = b.dimensions().map(|d| (d.name(), d.len())).collect();
    for d in dims_a.difference(&dims_b) {
        problems.push(format!("dimension only in golden: {d:?}"));
    }
    for d in dims_b.difference(&dims_a) {
        problems.push(format!("dimension only in produced: {d:?}"));
    }

    // --- 全局属性 ---
    compare_attrs(
        "global",
        &a.attributes()
            .map(|x| x.name().to_string())
            .collect::<Vec<_>>(),
        &b.attributes()
            .map(|x| x.name().to_string())
            .collect::<Vec<_>>(),
        |n| a.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
        |n| b.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
        &mut problems,
    );

    // --- 变量 ---
    let names_a: BTreeSet<String> = a.variables().map(|v| v.name()).collect();
    let names_b: BTreeSet<String> = b.variables().map(|v| v.name()).collect();
    for n in names_a.difference(&names_b) {
        problems.push(format!("variable only in golden: {n}"));
    }
    for n in names_b.difference(&names_a) {
        problems.push(format!("variable only in produced: {n}"));
    }

    let mut compared = 0usize;
    for name in names_a.intersection(&names_b) {
        let va = a.variable(name).expect("name came from this file");
        let vb = b.variable(name).expect("name came from this file");

        // 逐变量比对**维度名与长度的有序列表**，而不只是秩。
        //
        // 只比秩是不够的：patch = 1，所以把 (time, patch, ...) 换成 (patch, time, ...)
        // 之后扁平化的字节序完全不变，逐值比较会全部通过。实测：把 119 个变量的
        // 前两个维度对调，判官报 identical。而 colm-hist 做时间轴还原与抽稀时是
        // **按轴位置索引**的，那样的文件会让它静默读错轴。
        let da: Vec<(String, usize)> = va
            .dimensions()
            .iter()
            .map(|d| (d.name(), d.len()))
            .collect();
        let db: Vec<(String, usize)> = vb
            .dimensions()
            .iter()
            .map(|d| (d.name(), d.len()))
            .collect();
        if da != db {
            problems.push(format!("{name}: dimensions {da:?} vs {db:?}"));
            continue;
        }

        // 存储类型也是契约的一部分：把 int 坐标变量改写成 double，值读出来一样，
        // 但下游按整数索引的代码会变。
        if va.vartype() != vb.vartype() {
            problems.push(format!(
                "{name}: type {:?} vs {:?}",
                va.vartype(),
                vb.vartype()
            ));
            continue;
        }

        // 变量级属性也要比。units / long_name / missing_value 是文件契约的一部分，
        // 下游的评估与绘图都依赖它们；只比全局属性会让这类回归静默通过。
        compare_attrs(
            name,
            &va.attributes()
                .map(|x| x.name().to_string())
                .collect::<Vec<_>>(),
            &vb.attributes()
                .map(|x| x.name().to_string())
                .collect::<Vec<_>>(),
            |n| va.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
            |n| vb.attribute(n).and_then(|x| x.value().ok()).map(fmt_attr),
            &mut problems,
        );

        // 全部按 f64 读出后逐位比较。NaN 视为相等（两边都 NaN 才算相等）。
        let xa: Vec<f64> = va
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read golden {name}"))?;
        let xb: Vec<f64> = vb
            .get_values(netcdf::Extents::All)
            .with_context(|| format!("cannot read produced {name}"))?;
        if xa.len() != xb.len() {
            problems.push(format!("{name}: length {} vs {}", xa.len(), xb.len()));
            continue;
        }
        let mut first_bad: Option<(usize, f64, f64)> = None;
        let mut n_bad = 0usize;
        for (i, (p, q)) in xa.iter().zip(xb.iter()).enumerate() {
            let same = (p.is_nan() && q.is_nan()) || p.to_bits() == q.to_bits();
            if !same {
                n_bad += 1;
                if first_bad.is_none() {
                    first_bad = Some((i, *p, *q));
                }
            }
        }
        if let Some((i, p, q)) = first_bad {
            problems.push(format!(
                "{name}: {n_bad}/{} values differ; first at index {i}: {p:?} vs {q:?}",
                xa.len()
            ));
        }
        compared += 1;
    }

    Ok(Report {
        problems,
        compared,
        dimensions: dims_a.len(),
    })
}

fn compare_attrs<FA, FB>(
    scope: &str,
    names_a: &[String],
    names_b: &[String],
    get_a: FA,
    get_b: FB,
    problems: &mut Vec<String>,
) where
    FA: Fn(&str) -> Option<String>,
    FB: Fn(&str) -> Option<String>,
{
    let sa: BTreeSet<&String> = names_a.iter().collect();
    let sb: BTreeSet<&String> = names_b.iter().collect();
    for n in sa.symmetric_difference(&sb) {
        problems.push(format!("{scope} attribute present on only one side: {n}"));
    }
    for n in sa.intersection(&sb) {
        if VOLATILE_ATTRIBUTES.contains(&n.as_str()) {
            continue;
        }
        let (x, y) = (get_a(n), get_b(n));
        if x != y {
            problems.push(format!("{scope} attribute {n}: {x:?} vs {y:?}"));
        }
    }
}

fn fmt_attr(v: netcdf::AttributeValue) -> String {
    format!("{v:?}")
}
