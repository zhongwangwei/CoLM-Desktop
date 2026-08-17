//! 保留原文的 namelist 文档模型。
//!
//! 文档是一个**按行的项列表**。每个项都记着它的原始文本，序列化时
//! 未被修改的项原样吐回，被修改的项只替换值那一段 —— 缩进、等号位置、
//! 行尾注释都从原始文本里切出来复用。
//!
//! 这样做而不是「解析成结构再重新排版」，是因为重新排版必然改写用户
//! 没有动过的行，让保存后的 diff 淹没在无关噪声里。
//!
//! 关键是 `Entry` 连**值本身的原文**也保留，而不只是缩进与注释。理由是
//! 同一个值在 Fortran 里有多种等价写法，而 `Value` 只能渲染出其中一种。
//! 实测 55 个真实文件里：`.TRUE.` 大写形式 198 处（`Value::Bool` 渲染成
//! `.true.`）、双引号字符串 156 处（渲染成单引号）、逗号分隔多值 15 处
//! （渲染成空格分隔）—— 合计约 369 行会在「读进来再写回去」时被改写。
//!
//! 于是分界是：**没被 `set` 过的行逐字节不动；被 `set` 过的行才按 `Value`
//! 的规范形式重写。** 用户改了哪一行，diff 里就只出现哪一行。

use anyhow::{bail, Result};

use crate::value::{Path, Value};

/// 文档里的一行。
#[derive(Debug, Clone)]
pub enum Item {
    /// 空行或整行注释，原样保存
    Verbatim(String),
    /// `&group_name`
    GroupStart(String),
    /// 单独成行的 `/`
    GroupEnd(String),
    /// 一个赋值
    Entry(Entry),
}

/// 一个 `name = value ! comment` 行。
#[derive(Debug, Clone)]
pub struct Entry {
    pub path: Path,
    pub value: Value,
    /// 值那一段的**原文**。`set` 会用新值的渲染结果覆盖它；
    /// 没被改过就原样吐回，见模块文档。
    pub text: String,
    /// 从行首到 `=` 之后的那一段原文（含缩进与对齐空格）
    pub prefix: String,
    /// 值之后到行尾的原文（含空格与行尾注释）
    pub suffix: String,
}

#[derive(Debug, Clone, Default)]
pub struct Document {
    pub items: Vec<Item>,
}

impl Document {
    /// 按路径取值。路径写法与文件里一致，如 `DEF_forcing%fprefix(1)`。
    pub fn get(&self, path: &str) -> Option<&Value> {
        let want = Path::parse(path).ok()?;
        self.items.iter().find_map(|i| match i {
            Item::Entry(e) if e.path == want => Some(&e.value),
            _ => None,
        })
    }

    /// 就地改值。**字段不存在时报错，不追加** —— 静默追加会让调用方
    /// 以为改动生效，而 CoLM 读到的是另一回事。
    pub fn set(&mut self, path: &str, value: Value) -> Result<()> {
        let want = Path::parse(path)?;
        for item in &mut self.items {
            if let Item::Entry(e) = item {
                if e.path == want {
                    e.text = value.to_string();
                    e.value = value;
                    return Ok(());
                }
            }
        }
        bail!("no such field in this namelist: {path}")
    }

    /// 列出全部字段路径，按出现顺序。
    pub fn paths(&self) -> Vec<String> {
        self.items
            .iter()
            .filter_map(|i| match i {
                Item::Entry(e) => Some(e.path.to_string()),
                _ => None,
            })
            .collect()
    }
}

impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for item in &self.items {
            match item {
                Item::Verbatim(s) | Item::GroupStart(s) | Item::GroupEnd(s) => writeln!(f, "{s}")?,
                Item::Entry(e) => writeln!(f, "{}{}{}", e.prefix, e.text, e.suffix)?,
            }
        }
        Ok(())
    }
}
