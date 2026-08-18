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
mod project;
mod sidecar;
mod sites;

use config::*;
use project::*;
use sidecar::*;
use sites::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(RunLog::default())
        .invoke_handler(tauri::generate_handler![
            backend_ready,
            describe_fields,
            list_cases,
            list_kernels,
            scan_sites,
            new_case,
            read_text,
            read_case,
            set_field,
            run_case,
            run_batch,
            run_log_tail,
            series,
            write_text,
            unknown_fields,
        ])
        .run(tauri::generate_context!())
        .expect("error running CoLM Desktop");
}
