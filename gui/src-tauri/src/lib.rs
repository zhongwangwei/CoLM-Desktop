//! CoLM Desktop 的 Tauri 后端。
//!
//! **这个进程不链接 netcdf/hdf5。** 凡要读 NetCDF 的一律走 `colm-cli` sidecar ——
//! 窗口进程不该为了画一条曲线把整个静态 HDF5 拖进来。实测各层的 netcdf 依赖
//! 节点数：`colm-namelist` / `colm-schema` / `colm-case` / `colm-kernel` /
//! `colm-hist`（默认）全是 0，而 `colm-forcing` 7、`colm-srfdata` 7、`colm-cli` 9。
//! 这个分界不是强加的，是已有分层里自然掉出来的。
//!
//! 前端是纯静态 HTML/CSS/JS，无 npm、无打包器。`withGlobalTauri: true`，
//! 所以页面用 `window.__TAURI__.core.invoke(...)` 跟这里说话。

mod config;
mod example;
mod forcing;
mod histvars;
mod presets;
mod project;
mod recent;
mod sidecar;
mod sitedata;
mod sites;

use config::*;
use example::*;
use forcing::*;
use histvars::*;
use presets::*;
use project::*;
use recent::*;
use sidecar::*;
use sitedata::*;
use sites::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RunLog::default())
        // 注意：这里**没有**单份写入的命令（`set_field` / `write_text` /
        // `apply_preset`）。参数改动一律走 `*_batch` —— 前端只有一条写入路径，
        // 因为"改一个字段"与"改这一批的一个字段"必须是同一件事。
        .invoke_handler(tauri::generate_handler![
            backend_ready,
            describe_fields,
            list_cases,
            list_kernels,
            install_example,
            scan_sites,
            probe_forcing,
            convert_forcing,
            make_site,
            new_case,
            read_text,
            read_case,
            read_timing,
            set_spinup,
            set_field_batch,
            varying_fields,
            apply_preset_batch,
            run_case,
            run_batch,
            run_log_tail,
            series,
            metrics,
            unknown_fields,
            irrelevant_fields,
            hist_vars,
            save_preset,
            list_presets,
            delete_preset,
            load_recent,
            save_recent,
            pick_folder,
            pick_file,
        ])
        .run(tauri::generate_context!())
        .expect("error running CoLM Desktop");
}
