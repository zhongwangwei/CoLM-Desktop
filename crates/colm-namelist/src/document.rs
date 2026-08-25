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

    /// 就地改值。**字段不存在时报错，不追加** —— 追加要知道往哪个
    /// namelist 组里加，而这个函数不知道。要新增字段用 [`Document::insert`]，
    /// 它要求调用方把组名说出来：插错组的字段 CoLM 根本不读。
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

    /// 加一个这份文件里还没有的字段，插在指定 namelist 组的 `/` 之前。
    ///
    /// **组名必须由调用方给。** CoLM 按组读 namelist，插错组等于没设 ——
    /// 而"没设"与"设了但值不对"在运行时长得完全不一样。组名的来源是
    /// `colm_schema::Field::group`，那是从 CoLM 自己的声明里扫出来的。
    ///
    /// 组不在这份文件里就报错，不新建：一份 case.nml 少了整个组，
    /// 通常说明它不是一份完整的算例配置，而那个问题不该被一次插入掩盖。
    pub fn insert(&mut self, path: &str, value: Value, group: &str) -> Result<()> {
        let want = Path::parse(path)?;
        let mut inside = false;
        let mut found_outside = false;
        for item in &mut self.items {
            match item {
                Item::GroupStart(s) => {
                    inside = s.trim().trim_start_matches('&').eq_ignore_ascii_case(group);
                }
                Item::GroupEnd(_) => inside = false,
                Item::Entry(e) if e.path == want => {
                    if inside {
                        e.text = value.to_string();
                        e.value = value;
                        return Ok(());
                    }
                    found_outside = true;
                }
                _ => {}
            }
        }
        if found_outside {
            bail!("{path} already exists outside &{group}");
        }
        // 找那个组的 `/`。GroupStart 的原文形如 `&nl_colm`（可能带缩进）。
        let mut inside = false;
        for (i, item) in self.items.iter().enumerate() {
            match item {
                Item::GroupStart(s) => {
                    inside = s.trim().trim_start_matches('&').eq_ignore_ascii_case(group);
                }
                Item::GroupEnd(_) if inside => {
                    // 缩进跟着仓库风格（3 空格），与 `render` 写出来的一致。
                    self.items.insert(
                        i,
                        Item::Entry(Entry {
                            path: want,
                            text: value.to_string(),
                            value,
                            prefix: format!("   {path} = "),
                            suffix: String::new(),
                        }),
                    );
                    return Ok(());
                }
                _ => {}
            }
        }
        bail!("this namelist has no group &{group} to put {path} in")
    }

    /// Remove one exact assignment. Missing fields are already at their
    /// declared default, so removing an absent field is a harmless no-op.
    pub fn remove(&mut self, path: &str) -> Result<bool> {
        let want = Path::parse(path)?;
        let before = self.items.len();
        self.items
            .retain(|item| !matches!(item, Item::Entry(entry) if entry.path == want));
        Ok(self.items.len() != before)
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

#[cfg(test)]
#[path = "document_tests.rs"]
mod document_tests;
