//! 从内核日志里抽出 CoLM 的静默覆盖。
//!
//! CoLM 会在不声不响地改掉你的配置之后打印一行 `Note:` 或 `Warning:`，
//! 然后继续跑。实测一次 CN-Cng 运行有 9 种这样的消息，其中两条是真正的覆盖
//! （变饱和流被自动打开、VG + IGBP 下土壤阻抗被自动关掉），两条是站点坐标
//! 与 namelist 不一致而**以数据文件为准**。
//!
//! 抽取只认前缀，不认消息文本。CoLM 把 automatically 拼成了 automaticlly，
//! 而上游哪天改回来，按文本匹配的代码就会静默失效。整行原样交给上层，
//! 由上层决定怎么呈现 —— design.md §6.4 要的是「你要求了 X，模型实际用了 Y」。

use std::collections::BTreeSet;

/// 覆盖消息的类别。只按前缀分，不解释内容。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Note,
    Warning,
}

/// 一条覆盖消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Override {
    pub kind: Kind,
    /// 整行原文，已去掉两端空白。
    pub text: String,
}

/// 扫全文，按出现顺序返回去重后的覆盖消息。
pub fn extract(stdout: &str) -> Vec<Override> {
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for line in stdout.lines() {
        let t = line.trim();
        // 冒号前可能有空格：实测有 `Warning :` 这种写法。
        let Some(kind) = prefix_kind(t) else { continue };
        if seen.insert(t.to_string()) {
            out.push(Override {
                kind,
                text: t.to_string(),
            });
        }
    }
    out
}

fn prefix_kind(line: &str) -> Option<Kind> {
    for (word, kind) in [("Note", Kind::Note), ("Warning", Kind::Warning)] {
        if let Some(rest) = line.strip_prefix(word) {
            if rest.trim_start().starts_with(':') {
                return Some(kind);
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "overrides_tests.rs"]
mod overrides_tests;
