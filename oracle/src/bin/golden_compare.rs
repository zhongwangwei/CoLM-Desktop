//! 比对两个 CoLM history 文件：变量数据、维度、属性。
//!
//! 用法: golden-compare <golden.nc> <produced.nc>
//!
//! 不做字节比对。实测重跑会产生 8 字节差异，全部来自全局属性 create_time
//! （CoLM 写入的墙上时钟）；129 个变量的数据逐位相同。

use std::collections::BTreeSet;
use std::path::PathBuf;

use anyhow::{bail, Context, Result};

/// 允许不同的属性名。**精确名单，不做前缀或通配匹配。**
/// 新增条目必须说明为什么该属性天然易变。
const VOLATILE_ATTRIBUTES: &[&str] = &[
    // CoLM 写入的文件创建墙上时钟，例如 "20260817-16:16:27 UTC+08:00"
    "create_time",
];

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let a_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );
    let b_path = PathBuf::from(
        args.next()
            .context("usage: golden-compare <golden> <produced>")?,
    );

    let a = netcdf::open(&a_path).with_context(|| format!("cannot open {}", a_path.display()))?;
    let b = netcdf::open(&b_path).with_context(|| format!("cannot open {}", b_path.display()))?;

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
        let va = a.variable(name).unwrap();
        let vb = b.variable(name).unwrap();

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

    if problems.is_empty() {
        println!(
            "identical: {compared} variables, {} dimensions (ignoring {:?})",
            dims_a.len(),
            VOLATILE_ATTRIBUTES
        );
        return Ok(());
    }
    eprintln!("{} problem(s):", problems.len());
    for p in &problems {
        eprintln!("  {p}");
    }
    bail!("golden comparison failed");
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
