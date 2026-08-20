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
    /// 这个字段的**合法取值**，为空表示不是枚举型。
    ///
    /// 从 CoLM 自己的分支里扫出来：`SELECT CASE (trim(adjustl(DEF_x)))` 的
    /// 各个 `CASE ('…')`，以及 `trim(DEF_x) == '…'`。实测 12 个字段有，
    /// 其中 `DEF_HIST_mode` 两种写法都用到 —— **只扫一种会漏掉 7 个**。
    ///
    /// GUI 据此把文本框换成下拉框：这些字段拼错了要等 CoLM 读 namelist
    /// 时才报错，而那时人已经在等一次运行了。
    pub values: &'static [&'static str],
    /// 这个字段要**编译期开了哪些宏**才有意义；为空表示任何配置下都可能用到。
    ///
    /// 两种来源，都从源码扫：字段的全部用法都在某个 `#ifdef` 之内（实测 63 个），
    /// 或全部落在某个可选子系统的目录里（`*Catch*`、`CaMa/`、`main/DA/`，
    /// 实测 56 个，与前者有重叠）。**`main/TRACER/`、`*Urban*`、`main/BGC/`
    /// 不再在这一类里**——示踪物 / URBAN_MODEL / BGC 那两轮改造把它们的
    /// 编译期宏都改成了运行时开关，这三个目录现在永远编译进去，见
    /// `xtask/src/usage.rs` 里 `SUBSYSTEMS`/`BY_NAME` 头上的说明。
    ///
    /// **两种都覆盖不到的还有一类**：字段本身没被守，守护在**调用点**。
    /// 那一类只能人工列，见 `curated.rs`，而且每条都要带出处。目前是空的——
    /// 唯一进过这张表的例子 `DEF_URBAN_type_scheme`（当时守护在
    /// `landurban_build` 唯一调用者外的 `#ifdef URBAN_MODEL`）已经过时了：
    /// URBAN_MODEL 改成运行时开关之后，那个调用点的守护变成了
    /// `IF (DEF_URBAN_RUN)`（`MKSRFDATA.F90`），不再是编译期宏。
    ///
    /// 判据落到界面上很便宜：内核的 `manifest.json` 已经记了它编进了哪些宏。
    pub requires: &'static [&'static str],
    /// `MOD_Namelist.F90` 中的行号，便于回查
    pub line: u32,
}

#[cfg(test)]
#[path = "field_tests.rs"]
mod field_tests;
