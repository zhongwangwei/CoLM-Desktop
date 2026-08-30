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
mod observation;
mod project;
mod recent;
mod sidecar;
mod sitedata;
mod sites;

use config::*;
use example::*;
use forcing::*;
use histvars::*;
use observation::*;
use project::*;
use recent::*;
use sidecar::*;
use sitedata::*;
use sites::*;
use tauri::{Emitter, Manager};

#[tauri::command]
fn print_report(window: tauri::WebviewWindow) -> Result<(), String> {
    window
        .print()
        .map_err(|e| format!("无法打开系统打印窗口：{e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .menu(|app| {
            use tauri::menu::{Menu, MenuItem};

            let menu = Menu::default(app)?;
            let about =
                MenuItem::with_id(app, "about-colm", "About CoLM Desktop", true, None::<&str>)?;

            #[cfg(target_os = "macos")]
            let parent = menu
                .items()?
                .into_iter()
                .next()
                .and_then(|item| item.as_submenu().cloned());
            #[cfg(not(target_os = "macos"))]
            let parent = menu
                .get(tauri::menu::HELP_SUBMENU_ID)
                .and_then(|item| item.as_submenu().cloned());
            if let Some(parent) = parent {
                parent.remove_at(0)?;
                parent.insert(&about, 0)?;
            }
            Ok(menu)
        })
        .on_menu_event(|app, event| {
            if event.id() == "about-colm" {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                    let _ = window.emit("colm-about", app.package_info().version.to_string());
                }
            }
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                if let Err(error) = window
                    .app_handle()
                    .state::<RunProcesses>()
                    .cancel_on_shutdown()
                {
                    eprintln!("failed to stop every run while closing: {error}");
                }
            }
        })
        .manage(RunProcesses::default())
        // 注意：这里**没有**单份写入的命令（`set_field` / `write_text` /
        // 单份写入）。参数改动一律走 `*_batch` —— 前端只有一条写入路径，
        // 因为"改一个字段"与"改这一批的一个字段"必须是同一件事。
        .invoke_handler(tauri::generate_handler![
            backend_ready,
            probe_log,
            describe_fields,
            parameter_catalog,
            export_parameter_overrides,
            preview_import_parameter_overrides,
            apply_import_parameter_overrides,
            list_cases,
            mark_results_stale,
            list_kernels,
            install_example,
            scan_sites,
            probe_forcing,
            probe_forcing_table,
            convert_forcing_table,
            probe_observation_table,
            convert_observation_table,
            probe_forcing_gaps,
            repair_forcing,
            download_era5land,
            convert_forcing,
            configure_cbl_batch,
            configure_ozone_batch,
            make_site,
            install_prepared_pair,
            site_pfts,
            new_case,
            read_text,
            read_case,
            read_timing,
            set_spinup,
            set_field_batch,
            reset_field_batch,
            set_fields_batch,
            varying_fields,
            run_case,
            run_batch,
            cancel_runs,
            history_catalog,
            study_params,
            study_parameter_contexts,
            study_create_json,
            study_preflight_json,
            study_run,
            study_status,
            study_pause,
            study_resume,
            study_cancel,
            study_retry,
            study_export,
            study_apply,
            study_apply_preview,
            study_result,
            series,
            evaluation_catalog,
            evaluation_plan,
            metrics,
            unknown_fields,
            irrelevant_fields,
            field_states_batch,
            land_cover_contexts,
            pft_parameter_states,
            set_pft_parameter_batch,
            set_pft_parameters_batch,
            process_parameter_files,
            set_process_parameter_field_batch,
            reset_process_parameter_field_batch,
            hist_vars,
            load_recent,
            save_recent,
            pick_folder,
            pick_file,
            print_report,
        ])
        .build(tauri::generate_context!())
        .expect("error building CoLM Desktop")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                if let Err(error) = app.state::<RunProcesses>().cancel_on_shutdown() {
                    eprintln!("failed to stop every run while exiting: {error}");
                }
            }
        });
}
