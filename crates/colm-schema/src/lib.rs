//! CoLM `DEF_*` 配置字段的元数据：类型、默认值、所属 group、说明。
//!
//! **本 crate 的字段表是代码生成的，不是手写的。** 生成器是
//! `xtask gen-schema`，输入是 `vendor/CoLM202X/share/MOD_Namelist.F90`，
//! 产物 `generated.rs` 入库，并由 `tests/drift.rs` 守住：重新生成必须
//! 与入库产物逐字节一致。
//!
//! 这样做的理由是 CoLM 会持续演进。手写的字段表在上游加一个 `DEF_` 之后
//! 不会报错，只会静默地少一项 —— 而 GUI 依赖这张表决定渲染什么，
//! 少一项意味着用户永远看不到那个选项。
//!
//! `all()` / `find()` 与重导出在 Task 8 里加上，那时字段表才存在。

pub mod field;
pub mod generated;

pub use field::{Default, Field, FieldKind};

/// 全部字段，按声明顺序。
pub fn all() -> &'static [Field] {
    generated::FIELDS
}

/// 按全名查找，例如 `"DEF_forcing%dataset"`。
pub fn find(name: &str) -> Option<&'static Field> {
    generated::FIELDS.iter().find(|f| f.name == name)
}
