//! namelist 的值与字段寻址。
//!
//! `Value::Real` 保存**原始文本**而不是 `f64`：`1800.` `1800.0` `1.8e3`
//! 在 Fortran 里等价，但往返必须还原用户写的那一种，否则每次保存都会
//! 改写用户的文件，让 diff 里全是与改动无关的噪声。

use std::fmt;

use anyhow::{bail, Result};

/// 字段路径的一段。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Segment {
    /// 顶层名字，如 `DEF_CASE_NAME`
    Field(String),
    /// 派生类型成员，如 `%dataset`
    Member(String),
    /// 下标，如 `(1)`。Fortran 下标从 1 起，这里原样保存不做换算。
    Index(usize),
}

/// 一个字段的完整路径，如 `DEF_forcing%fprefix(1)`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    pub segments: Vec<Segment>,
}

impl Path {
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.is_empty() {
            bail!("empty field path");
        }
        if s.contains(':') {
            bail!("array slice syntax is not supported: {s}");
        }
        let mut segments = Vec::new();
        for (i, part) in s.split('%').enumerate() {
            let (name, index) = match part.find('(') {
                Some(p) => {
                    if !part.ends_with(')') {
                        bail!("unclosed subscript in {s}");
                    }
                    let inner = &part[p + 1..part.len() - 1];
                    let n: usize = inner
                        .trim()
                        .parse()
                        .map_err(|_| anyhow::anyhow!("bad subscript {inner:?} in {s}"))?;
                    (&part[..p], Some(n))
                }
                None => (part, None),
            };
            let name = name.trim();
            if name.is_empty() {
                bail!("empty path segment in {s}");
            }
            segments.push(if i == 0 {
                Segment::Field(name.to_string())
            } else {
                Segment::Member(name.to_string())
            });
            if let Some(n) = index {
                segments.push(Segment::Index(n));
            }
        }
        Ok(Self { segments })
    }
}

impl fmt::Display for Path {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for seg in &self.segments {
            match seg {
                Segment::Field(n) => write!(f, "{n}")?,
                Segment::Member(n) => write!(f, "%{n}")?,
                Segment::Index(n) => write!(f, "({n})")?,
            }
        }
        Ok(())
    }
}

/// 一个 namelist 值。
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    /// 保留原始文本，见模块文档。
    Real {
        text: String,
    },
    Str(String),
    /// 空格或逗号分隔的多值。分隔符由 `Document` 在序列化时按原文还原。
    List(Vec<Value>),
}

impl Value {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            // Fortran 用 d 表示双精度指数，Rust 不认，换成 e
            Value::Real { text } => text.replace(['d', 'D'], "e").parse().ok(),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(true) => write!(f, ".true."),
            Value::Bool(false) => write!(f, ".false."),
            Value::Int(i) => write!(f, "{i}"),
            Value::Real { text } => write!(f, "{text}"),
            Value::Str(s) => write!(f, "'{s}'"),
            Value::List(items) => {
                let parts: Vec<String> = items.iter().map(|v| v.to_string()).collect();
                write!(f, "{}", parts.join(" "))
            }
        }
    }
}

#[cfg(test)]
#[path = "value_tests.rs"]
mod value_tests;
