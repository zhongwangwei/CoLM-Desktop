//! 「这一段能不能跳过」的判据。
//!
//! **只看产物在不在是不够的。** 改了 `SITE_fsitedata` 或 `DEF_dir_rawdata`，
//! `srfdata.nc` 就失效了，而文件还好好躺在那儿 —— 跳过它等于拿旧地表数据
//! 算新算例，而且没有任何迹象。所以每段跑完记一份**输入指纹**，
//! 下次比对不上就必须重跑，并说出是哪一项变了。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// 一段完成时，它的输入长什么样。
#[derive(Serialize, Deserialize, PartialEq, Eq)]
pub struct Fingerprint {
    /// 相关 namelist 字段及其值（原文）
    pub inputs: BTreeMap<String, String>,
    /// 站点文件的 sha256。**必须算内容而不是记路径** ——
    /// 同一个路径下换一份站点文件是最容易发生、也最容易漏掉的一种变更。
    pub site_sha256: String,
    /// 内核身份：预设 + 上游 commit。换个预设就是换了一套编译期宏，
    /// 地表数据也跟着不同。
    pub kernel: String,
}

/// 一段**不**依赖的字段前缀。
///
/// 反过来列是有理由的：正着列「这一段依赖哪些字段」要枚举两百个名字里的
/// 大部分，漏一个就是静默算错；反着列只需要说清楚「时间窗口与输出设置
/// 影响不到地表数据」这件事，短得多，也容易辩护。
///
/// `DEF_dir_output` 也在里面：它变了，产物就落到别处，
/// 而「产物在不在」那一关本来就会发现。
fn ignored(stage: &str, path: &str) -> bool {
    let output_side = path.starts_with("DEF_HIST")
        || path.starts_with("DEF_hist")
        || path.starts_with("DEF_WRST")
        || path == "DEF_dir_output";
    match stage {
        // 地表数据与时间窗口无关：它描述的是这个点长什么样，不是跑哪一段。
        "mksrfdata" => output_side || path.starts_with("DEF_simulation_time"),
        // 初始场取决于起始时刻，但与结束时刻、spin-up 轮数无关。
        "mkinidata" => {
            output_side
                || (path.starts_with("DEF_simulation_time")
                    && !path.starts_with("DEF_simulation_time%start"))
        }
        // 主程序什么都依赖。
        _ => false,
    }
}

pub fn compute(stage: &str, case_nml: &Path, kernel: &str) -> Result<Fingerprint> {
    let text = std::fs::read_to_string(case_nml)
        .with_context(|| format!("cannot read {}", case_nml.display()))?;
    let doc = colm_namelist::parse(&text)?;
    let mut inputs = BTreeMap::new();
    for p in doc.paths() {
        if ignored(stage, &p) {
            continue;
        }
        if let Some(v) = doc.get(&p) {
            inputs.insert(p, v.to_string());
        }
    }
    // 站点文件的内容。读不到就用空串 —— 那时下一关（产物是否存在）会接手，
    // 而在这里报错会让「站点文件还没生成」的正常情形变成硬失败。
    let site = doc
        .get("SITE_fsitedata")
        .and_then(|v| match v {
            colm_namelist::Value::Str(s) => Some(s.clone()),
            _ => None,
        })
        .and_then(|p| std::fs::read(p).ok())
        .map(|b| colm_kernel::sha256_hex(&b))
        .unwrap_or_default();

    Ok(Fingerprint {
        inputs,
        site_sha256: site,
        kernel: kernel.to_string(),
    })
}

/// 算例目录里那份记录。整体读写，不做增量 —— 三段而已。
pub fn load(case: &Path) -> BTreeMap<String, Fingerprint> {
    std::fs::read_to_string(case.join("stages.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn save(case: &Path, all: &BTreeMap<String, Fingerprint>) -> Result<()> {
    let p = case.join("stages.json");
    std::fs::write(&p, serde_json::to_string_pretty(all)?)
        .with_context(|| format!("cannot write {}", p.display()))
}

/// 两份指纹的第一处差异，用人话说。
///
/// 只报**第一处**：界面上要的是「为什么要重跑」，一个具体原因比一张
/// 差异表更有用，而全列出来在字段多时会淹没重点。
pub fn first_difference(old: &Fingerprint, new: &Fingerprint) -> Option<String> {
    if old.kernel != new.kernel {
        return Some(format!("内核换了：{} -> {}", old.kernel, new.kernel));
    }
    if old.site_sha256 != new.site_sha256 {
        return Some("站点文件的内容变了".to_string());
    }
    for (k, v) in &new.inputs {
        match old.inputs.get(k) {
            None => return Some(format!("新设了 {k}")),
            Some(o) if o != v => return Some(format!("{k}：{o} -> {v}")),
            _ => {}
        }
    }
    for k in old.inputs.keys() {
        if !new.inputs.contains_key(k) {
            return Some(format!("{k} 被删掉了"));
        }
    }
    None
}

#[cfg(test)]
#[path = "fingerprint_tests.rs"]
mod fingerprint_tests;
