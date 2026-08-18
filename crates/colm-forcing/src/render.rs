//! 生成 `nl_colm_forcing`。
//!
//! 生成的是文本而不是结构，因为这份 namelist 也要给人看：注释里那几句
//! 「为什么第 5 槽是 NULL」「为什么 HEIGHT_* 会被覆盖」比字段本身更容易丢。
//!
//! 但产物会被 `colm-namelist` 解析回来做断言（见 `render_tests.rs`），
//! 所以它不只是拼字符串 —— 拼错了测试会红。

use crate::check::MetSummary;

/// 生成一份 namelist 所需的一切。
#[derive(Debug, Clone)]
pub struct ForcingSpec {
    /// 强迫场目录。CoLM 拼路径时不补斜杠，所以这里保证结尾有一个。
    pub dir: String,
    /// 强迫场文件名（不含目录）。
    pub file: String,
    pub met: MetSummary,
}

/// 渲染成 namelist 文本。
pub fn render(s: &ForcingSpec) -> String {
    let dir = if s.dir.ends_with('/') {
        s.dir.clone()
    } else {
        format!("{}/", s.dir)
    };
    let end = s.met.end();
    // 槽位按文件里实际有的变量填，不写死 —— PLUMBER2 是标量风（第 5 槽空），
    // Urban-PLUMBER 是分量风（第 5 槽是东风分量）。见 `slots`。
    let (slots, _missing) = crate::slots::resolve(&s.met.variables);
    let vnames = slots.names();
    let tints = slots.tintalgo();
    let quoted = |xs: &[&str]| {
        xs.iter()
            .map(|x| format!("'{x}'"))
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "&nl_colm_forcing\n\
         \n\
         ! 由 colm-forcing 生成。CoLM 直接读 PLUMBER2 的 Met 文件，不做转换。\n\
         \n\
         \x20  DEF_dir_forcing              = '{dir}'\n\
         \n\
         \x20  DEF_forcing%dataset          = 'POINT'\n\
         \x20  DEF_forcing%solarin_all_band = .true.\n\
         \n\
         ! HEIGHT_* 取自强迫场文件的 reference_height_v/t/q。CoLM 在 POINT 下会用\n\
         ! 文件里的值 overwritten 掉这三行（MOD_Forcing.F90:294-310），所以它们是\n\
         ! 给人看的；写文件里的真值而不是常数，才不会误导下一个读它的人。\n\
         \x20  DEF_forcing%HEIGHT_V         = {hv:?}\n\
         \x20  DEF_forcing%HEIGHT_T         = {ht:?}\n\
         \x20  DEF_forcing%HEIGHT_Q         = {hq:?}\n\
         \n\
         \x20  DEF_forcing%NVAR             = 8\n\
         \x20  DEF_forcing%startyr          = {sy}\n\
         \x20  DEF_forcing%startmo          = {sm}\n\
         \x20  DEF_forcing%endyr            = {ey}\n\
         \x20  DEF_forcing%endmo            = {em}\n\
         \n\
         ! POINT 下 CoLM 只读 fprefix(1)（MOD_UserSpecifiedForcing.F90:683），\n\
         ! 其余 7 个槽从不使用。\n\
         \x20  DEF_forcing%fprefix(1)       = '{file}'\n\
         \n\
         ! 槽位固定为 1=T 2=q 3=psrf 4=precip 5=u 6=v 7=SW 8=LW，\n\
         ! 名字则按这份文件里实际有的变量填。\n\
         {windnote}\
         \x20  DEF_forcing%vname            = {vname}\n\
         \x20  DEF_forcing%tintalgo         = {tint}\n\
         /\n",
        dir = dir,
        hv = s.met.height_v,
        ht = s.met.height_t,
        hq = s.met.height_q,
        sy = s.met.start.year,
        sm = s.met.start.month,
        ey = end.year,
        em = end.month,
        file = s.file,
        vname = quoted(&vnames),
        tint = quoted(&tints),
        windnote = if slots.wind_is_vector() {
            "! 这份数据是**分量风**：第 5 槽东风、第 6 槽北风。\n"
        } else {
            "! 这份数据只有**标量风**，故第 5 槽为 'NULL'，风进第 6 槽。\n"
        },
    )
}

#[cfg(test)]
#[path = "render_tests.rs"]
mod render_tests;
