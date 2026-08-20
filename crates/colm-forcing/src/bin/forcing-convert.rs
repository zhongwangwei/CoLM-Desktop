//! 把一份变量名/单位与 PLUMBER2 不同的强迫场，转成 CoLM 认的约定。
//!
//! 用法: forcing-convert <源文件> <产物> [--slot N=名字:单位 ...] [--height V,T,Q]
//!
//! 没给 `--slot` 的槽位走自动匹配。匹配不上就**列出文件里有哪些变量**
//! 再退出 —— 那正是用户下一步要用的信息，只说「缺第 3 槽」帮不上忙。
//!
//! `--height V,T,Q` 给源文件里没有的 `reference_height_v/t/q` 兜底
//! （Urban-PLUMBER 的站点都缺这三个标量）。源文件带着的不会被它覆盖。

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use colm_forcing::convert::{convert, Heights, Plan, SlotPlan};
use colm_forcing::{resolve_with, summarize, SLOTS};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let src = PathBuf::from(
        args.next()
            .context("usage: forcing-convert <src.nc> <dst.nc> [--slot N=name:units ...]")?,
    );
    let dst = PathBuf::from(
        args.next()
            .context("usage: forcing-convert <src.nc> <dst.nc>")?,
    );

    // --slot 1=TA_F:degC
    let mut given: Vec<SlotPlan> = Vec::new();
    let mut heights = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--slot" => {
                let spec = args.next().context("--slot needs N=name:units")?;
                let (idx, rest) = spec
                    .split_once('=')
                    .with_context(|| format!("--slot {spec:?} is not N=name:units"))?;
                let (name, units) = rest
                    .split_once(':')
                    .with_context(|| format!("--slot {spec:?} is missing :units"))?;
                // `--slot 4=Rainf:kg/m2/s+Snowf` —— 加号后面是要合并进
                // 同一个槽位的变量（见 Task 4b：不合并就丢掉全部降雪）。
                let (units, extra) = match units.split_once('+') {
                    Some((u, e)) => (u, e.split('+').map(str::to_string).collect()),
                    None => (units, Vec::new()),
                };
                given.push(SlotPlan {
                    index: idx
                        .parse()
                        .with_context(|| format!("{idx:?} is not a slot number"))?,
                    source_name: name.to_string(),
                    source_units: units.to_string(),
                    also_add: extra,
                });
            }
            "--height" => {
                let spec = args.next().context("--height needs V,T,Q")?;
                let n: Vec<f64> = spec
                    .split(',')
                    .map(|x| x.trim().parse::<f64>())
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("--height {spec:?} is not V,T,Q"))?;
                let [v, t, q] = n[..] else {
                    bail!("--height needs exactly three numbers, got {}", n.len());
                };
                heights = Some(Heights { v, t, q });
            }
            other => bail!("unknown argument: {other}"),
        }
    }

    let summary = summarize(&src)?;

    // 没给的槽位交给自动匹配。
    let overrides: Vec<(usize, String)> = given
        .iter()
        .map(|s| (s.index, s.source_name.clone()))
        .collect();
    let (resolved, missing) = resolve_with(&summary.variables, &overrides);
    if !missing.is_empty() {
        for m in &missing {
            eprintln!("  {m}");
        }
        // **把文件里有什么列出来。** 只说「缺第 3 槽」用户无从下手。
        eprintln!("  {} has: {}", src.display(), summary.variables.join(", "));
        bail!("{} slot(s) unresolved", missing.len());
    }

    // 自动匹配到的槽位，单位取文件自己的 `units` 属性。
    let mut plan = Plan {
        slots: given,
        heights,
    };
    let f = netcdf::open(&src)?;
    for (i, slot) in SLOTS.iter().enumerate() {
        if plan.slots.iter().any(|s| s.index == slot.index) {
            continue;
        }
        let Some(name) = resolved.vname[i] else {
            continue;
        };
        // **用 `attribute_value` 而不是 `attribute`。** 后者的签名是
        // `fn attribute<'a>(&'a self, ..) -> Option<Attribute<'a>>` —— 借用
        // 那个 `Variable`。而这里 `v` 是**按值移进闭包**的，闭包一结束就
        // drop，返回的 `Attribute` 就悬垂了，编译不过。
        // `attribute_value` 返回 owned 的 `AttributeValue`，没有这个问题。
        let units = f
            .variable(name)
            .and_then(|v| v.attribute_value("units"))
            .and_then(|r| r.ok())
            .and_then(|v| match v {
                netcdf::AttributeValue::Str(s) => Some(s),
                _ => None,
            })
            .unwrap_or_default();
        plan.slots.push(SlotPlan {
            index: slot.index,
            source_name: name.to_string(),
            source_units: units,
            // 自动匹配不做合成：合成要用户说清楚哪两个变量是同一个量。
            also_add: Vec::new(),
        });
    }

    // **缺测拦在入口，不在转换里悄悄处理。** -999 乘个系数还是个数，
    // 模型不会因此报错。这里报出来，用户才知道要先补数据。
    for sp in &plan.slots {
        for name in std::iter::once(&sp.source_name).chain(sp.also_add.iter()) {
            let Some(v) = f.variable(name) else { continue };
            let fill = match v.attribute_value("_FillValue").and_then(|r| r.ok()) {
                Some(netcdf::AttributeValue::Float(x)) => f64::from(x),
                Some(netcdf::AttributeValue::Double(x)) => x,
                _ => continue,
            };
            let vals: Vec<f64> = v.get_values(netcdf::Extents::All)?;
            let n = vals.iter().filter(|x| (**x - fill).abs() < 1e-6).count();
            if n > 0 {
                bail!(
                    "{name} has {n} missing value(s) (_FillValue = {fill}); \
                     fill them before converting — a fill value survives unit \
                     conversion as a plausible-looking number and the model will \
                     run to completion with it"
                );
            }
        }
    }

    convert(&src, &dst, &plan)?;
    println!("wrote {}", dst.display());
    for s in &plan.slots {
        println!(
            "  slot {} <- {} ({})",
            s.index, s.source_name, s.source_units
        );
    }
    Ok(())
}
