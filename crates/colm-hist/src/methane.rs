//! CH4 history 写哪些变量（`MOD_Tracer_Reactive_Methane_Hist`）：由 CH4 参数文件里的
//! `DEF_METHANE%write_ch4_history` 与 `DEF_METHANE%ch4_history_vars` 决定。引擎与 GUI 共用这份规则，
//! 输出变量页显示的开关才与实际写出的一致。

/// `core` 选择写出的 18 个变量（写进文件时加前缀 `f_`，与 [`crate::generated::VARS`] 同名）。
pub const METHANE_CORE_HISTORY: [&str; 18] = [
    "methane_surf_flux_tot_active",
    "methane_surf_flux_tot_phys",
    "methane_balance_residual",
    "methane_ch4_clip_credit",
    "o2_cap_loss",
    "o2_cap_gain",
    "methane_prod_tot",
    "methane_oxid_tot",
    "totcol_methane",
    "methane_surf_flux_tot",
    "methane_surf_flux_wetland",
    "methane_surf_flux_soil",
    "methane_surf_flux_lake",
    "methane_surf_flux_rice",
    "methane_surf_flux_global_total_with_lake",
    "methane_surf_flux_global_phys_with_lake",
    "methane_balance_residual_global_with_lake",
    "methane_ch4_clip_credit_global_with_lake",
];

/// `methane_history_accumulation_mode`：0 不写，1 只写 `core`，2 其它选择（`diagnostic`、`all` 或逐个列名）。
pub fn methane_history_mode(write_ch4_history: bool, ch4_history_vars: &str) -> i32 {
    if !write_ch4_history {
        return 0;
    }
    match ch4_history_vars.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "false" | ".false." => 0,
        "core" | "default" | "minimal" | "fast" => 1,
        _ => 2,
    }
}
