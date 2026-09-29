//! 多 patch 单点（CROP 多作物站点）的续跑写出：各 patch 独立生成的覆盖量合成整变量。
//!
//! 每个 patch 的模板只知道自己：它给出的覆盖量要么是"以输入重启为底、只换了自己那一块"的整变量
//! （主重启的写法），要么只有自己那一块（BGC/PFT 状态直接导出的写法）。这里按变量最外层的
//! `patch`/`pft` 维把各 patch 的那一块拼回去；没有这两维的变量取第一个 patch 的值。

use std::ops::Range;

use anyhow::{ensure, Context, Result};
use colm_init::{RestartFile, RestartOverride};

/// 一个参与写出的 patch：在重启里的下标与它的 PFT 区间。
#[derive(Debug, Clone)]
pub struct PatchSlot {
    pub patch: usize,
    pub pfts: Range<usize>,
}

/// 把各 patch 的覆盖量（与 `slots` 一一对应）合成对 `source` 整变量的覆盖。
pub fn merge_overrides(
    source: &RestartFile,
    slots: &[PatchSlot],
    per_patch: Vec<Vec<RestartOverride>>,
) -> Result<Vec<RestartOverride>> {
    ensure!(
        slots.len() == per_patch.len() && !slots.is_empty(),
        "every written patch needs its own overrides"
    );
    let patches = source.dimensions().get("patch").copied();
    let pfts = source.dimensions().get("pft").copied();
    let mut per_patch = per_patch.into_iter();
    let first = per_patch.next().context("checked above")?;
    let others: Vec<Vec<RestartOverride>> = per_patch.collect();
    let mut merged = Vec::with_capacity(first.len());
    for (index, head) in first.into_iter().enumerate() {
        let name = head.name.clone();
        let dims = source
            .variable_dimensions(&name)
            .with_context(|| format!("the restart has no variable {name} to override"))?;
        let outer = dims.first().map(String::as_str);
        let block = match outer {
            Some("patch") => patches.map(|count| (count, "patch")),
            Some("pft") => pfts.map(|count| (count, "pft")),
            _ => None,
        };
        let Some((count, dim)) = block else {
            merged.push(head);
            continue;
        };
        let whole = whole_values(source, &name)?;
        ensure!(
            count > 0 && whole.len() % count == 0,
            "{name} does not divide by its {dim} dimension"
        );
        let chunk = whole.len() / count;
        let mut values = whole;
        let mut place = |slot: &PatchSlot, given: &RestartOverride| -> Result<()> {
            let range = if dim == "patch" {
                slot.patch * chunk..(slot.patch + 1) * chunk
            } else {
                slot.pfts.start * chunk..slot.pfts.end * chunk
            };
            let own = if given.values.len() == values.len() {
                &given.values[range.clone()]
            } else {
                ensure!(
                    given.values.len() == range.len(),
                    "{name} from patch {} holds {} values; expected the whole variable ({}) or \
                     its own block ({})",
                    slot.patch,
                    given.values.len(),
                    values.len(),
                    range.len()
                );
                &given.values[..]
            };
            values[range].copy_from_slice(own);
            Ok(())
        };
        place(&slots[0], &head)?;
        for (slot, list) in slots[1..].iter().zip(&others) {
            let given = list
                .get(index)
                .filter(|given| given.name == name)
                .or_else(|| list.iter().find(|given| given.name == name))
                .with_context(|| format!("patch {} did not override {name}", slot.patch))?;
            place(slot, given)?;
        }
        merged.push(RestartOverride::new(name, values));
    }
    Ok(merged)
}

fn whole_values(source: &RestartFile, name: &str) -> Result<Vec<f64>> {
    if let Ok(values) = source.floats(name) {
        return Ok(values.to_vec());
    }
    Ok(source
        .integers(name)
        .with_context(|| format!("the restart has no variable {name}"))?
        .iter()
        .map(|&value| value as f64)
        .collect())
}

#[cfg(test)]
#[path = "multi_patch_tests.rs"]
mod tests;
