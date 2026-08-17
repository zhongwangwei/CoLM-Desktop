//! 一个配置字段的元数据。手写；字段表本身是生成的。

/// 字段的存储类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Logical,
    Integer,
    Real,
    /// Fortran 的 `character(len=N)`，N 一并记下来：GUI 要用它限制输入长度
    Character {
        len: usize,
    },
}

/// 字段的默认值，保留 Fortran 原文。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Default {
    Logical(bool),
    Integer(i64),
    /// 原始文本，如 `"1800."`
    Real(&'static str),
    Str(&'static str),
    /// 数组字面量的原文，如 `"(/ 'a','b' /)"`
    Array(&'static str),
}

/// 一个 `DEF_*` 字段。
#[derive(Debug, Clone, Copy)]
pub struct Field {
    /// 全名，如 `DEF_forcing%dataset`
    pub name: &'static str,
    pub kind: FieldKind,
    pub default: Default,
    /// 声明处 `=` 之后的行尾注释，可作为 GUI 的字段说明。713 个字段里 108 个有。
    pub doc: Option<&'static str>,
    /// 数组长度，如 `fprefix(8)` 是 `Some(8)`
    pub arity: Option<usize>,
    /// 所属派生类型名；顶层字段为 `None`
    pub owner: Option<&'static str>,
    /// 这个字段可以从**哪个 namelist 组**设置。
    ///
    /// `None` 意味着它在 `MOD_Namelist.F90` 里有声明、有默认值，但不出现在
    /// 任何 `namelist /.../` 语句里 —— 也就是**用户改不了它**。实测 6 个：
    /// `DEF_dir_history` / `DEF_dir_landdata` / `DEF_dir_restart` 由
    /// `DEF_dir_output` 派生（`MOD_Namelist.F90:1406` 无条件覆盖），
    /// `DEF_USE_IGBP` / `DEF_USE_USGS` / `DEF_Wetland_finundation_scheme` 由宏决定。
    /// GUI 应当把它们显示成只读的派生值，而不是给一个改了没用的输入框。
    ///
    /// 派生类型成员继承容器所在的组，所以 `DEF_forcing%dataset` 是
    /// `nl_colm_forcing`、`DEF_hist_vars%*` 是 `nl_colm_history`。
    /// **这正是 GUI 需要知道的「这个字段该写进哪个文件」。**
    pub group: Option<&'static str>,
    /// `MOD_Namelist.F90` 中的行号，便于回查
    pub line: u32,
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod field_tests;
