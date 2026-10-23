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

mod assistant;
mod config;
#[cfg(feature = "e2e-driver")]
mod e2e_driver;
mod example;
mod forcing;
mod history;
mod history_delete;
mod histvars;
mod model;
mod observation;
mod project;
mod recent;
mod remote;
mod remote_cases;
mod sidecar;
mod simulation_draft;
mod sitedata;
mod sites;
mod workspace;

use assistant::*;
use config::*;
use example::*;
use forcing::*;
use history::*;
use history_delete::*;
use histvars::*;
use model::*;
use observation::*;
use project::*;
use recent::*;
use remote::*;
use remote_cases::*;
use sidecar::*;
use simulation_draft::*;
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
/// 关窗与退出走同一套停止规则。原来两处各写一份，退出路径漏了 AI 助手（第 732 轮）。
/// 两个事件在单窗口下都会触发；每一步都是幂等的，重复调用无害。
fn stop_everything(app: &tauri::AppHandle, when: &str) {
    if let Err(error) = app.state::<RunProcesses>().cancel_on_shutdown() {
        eprintln!("failed to stop every run while {when}: {error}");
    }
    if let Err(error) = app.state::<CaseCreations>().cancel_on_shutdown() {
        eprintln!("failed to stop case creation while {when}: {error}");
    }
    // 远程提交/编引擎与数据准备不在 RunProcesses 里，但同样要停，否则界面承诺不成立。
    if let Err(error) = crate::sidecar::cancel_detached_children() {
        eprintln!("failed to stop remote submissions and data preparation while {when}: {error}");
    }
    app.state::<AssistantProcess>().stop();
}

pub fn run() {
    let run_processes = RunProcesses::default();
    crate::sidecar::install_run_registry(&run_processes);
    let builder = tauri::Builder::default();
    #[cfg(feature = "e2e-driver")]
    let builder = builder.setup(|app| {
        e2e_driver::start(app.handle());
        Ok(())
    });
    builder
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
                stop_everything(window.app_handle(), "closing");
            }
        })
        .manage(run_processes)
        .manage(CaseCreations::default())
        .manage(remote_cases::RemoteCreationBatches::default())
        .manage(AssistantProcess::default())
        // 注意：这里**没有**单份写入的命令（`set_field` / `write_text` /
        // 单份写入）。参数改动一律走 `*_batch` —— 前端只有一条写入路径，
        // 因为"改一个字段"与"改这一批的一个字段"必须是同一件事。
        .invoke_handler(tauri::generate_handler![
            backend_ready,
            probe_log,
            describe_fields,
            parameter_catalog,
            parameter_type_defaults,
            standard_process_parameters,
            export_parameter_overrides,
            preview_import_parameter_overrides,
            apply_import_parameter_overrides,
            list_cases,
            project_wizard_seed,
            project_last_runs,
            open_case,
            set_case_run_host,
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
            site_classifications,
            new_case,
            new_spatial_case,
            redirect_output_dir,
            forcing_datasets,
            probe_forcing_directory,
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
            cancel_preparation,
            cancel_remote_submission,
            case_creation_cancel,
            history_catalog,
            history_map,
            model_list,
            model_drafts,
            model_fields,
            open_path,
            model_get,
            model_delete,
            model_prune,
            model_create,
            model_draft,
            model_save_draft,
            model_freeze,
            model_freeze_check,
            model_changes,
            model_usage,
            model_delete_draft,
            model_branch_code,
            model_code_files,
            model_code_file,
            model_migrate_case,
            model_case_runs,
            model_case_context,
            model_case_profiles,
            model_configuration,
            model_attach_network,
            model_import_process,
            model_import_study,
            model_refresh_code,
            case_data_save,
            case_forcing_input,
            case_namelist_preview,
            history_list,
            history_detail,
            history_delete_plan,
            history_clear_plan,
            history_clear,
            history_project_delete_plan,
            history_project_delete,
            history_delete,
            history_export,
            history_replay,
            history_compare,
            new_simulation_draft_save,
            new_simulation_draft_close,
            study_save_version,
            study_parameter_contexts,
            study_type_choices,
            study_clone,
            study_compare,
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
            hybrid_info,
            assistant_settings,
            assistant_save_settings,
            assistant_set_key,
            assistant_has_key,
            assistant_delete_key,
            assistant_start,
            assistant_send,
            assistant_approve,
            assistant_cancel,
            assistant_new_session,
            assistant_sessions,
            assistant_input_history,
            assistant_transcript,
            assistant_delete_session,
            assistant_resume,
            assistant_ui_result,
            assistant_backend_status,
            assistant::assistant_codex_models,
            assistant::assistant_opencode_models,
            assistant::assistant_api_models,
            hybrid_studies,
            hybrid_fit,
            remote_config,
            remote_save_config,
            remote_remember_data_paths,
            remote_remember_kind_paths,
            remote_forget_kind_path,
            remote_set_password,
            remote_ssh_hosts,
            remote_probe,
            remote_scan_sites,
            remote_prepare_engine,
            remote_engine_progress,
            remote_load,
            remote_create_case,
            remote_create_cases,
            remote_create_cases_cancel,
            remote_browse,
            remote_mkdir,
            remote_engines,
            remote_engine_clean,
            remote_engine_recover,
            remote_run,
            remote_study_run,
            remote_study_status,
            remote_study_cancel,
            remote_study_fetch,
            remote_preview,
            remote_kernels,
            workspace::workspace_list,
            workspace::workspace_create,
            workspace::workspace_remote,
            workspace::workspace_status,
            workspace::workspace_snapshot,
            workspace::workspace_open,
            workspace::workspace_export,
            workspace::workspace_revert,
            workspace::workspace_delete,
            workspace::version_check,
            workspace::workspace_note,
            workspace::workspace_check,
            workspace::workspace_import_patch,
            workspace::workspace_diff,
            workspace::workspace_files,
            workspace::workspace_file,
            workspace::workspace_write_file,
            workspace::workspace_put,
            workspace::workspace_remove,
            workspace::workspace_revert_file,
            model::case_runs,
            remote_build_kernel,
            remote_status,
            remote_job_record,
            remote_cancel,
            remote_fetch,
            hybrid_check,
            hybrid_climate,
            hybrid_install,
            hybrid_remove,
            series,
            energy_summary,
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
            export_text_file,
            print_report,
        ])
        .build(tauri::generate_context!())
        .expect("error building CoLM Desktop")
        .run(|app, event| {
            if matches!(
                event,
                tauri::RunEvent::ExitRequested { .. } | tauri::RunEvent::Exit
            ) {
                stop_everything(app, "exiting");
            }
        });
}
