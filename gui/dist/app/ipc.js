//! 与后端说话的唯一出口。
//!
//! **全部 `invoke` / `listen` 都从这里出去。** `xtask check-gui` 靠扫这些
//! 字面量核对前后端接口；散落各处扫得到，但集中一处才看得出「一共有哪些」。
//! 更要紧的是下面那个 `hasBackend` —— 用浏览器直接打开页面时没有 IPC，
//! 每个模块各自判一次会得到不一致的降级行为。

import { state } from './state.js';

const T = window.__TAURI__;
const rawInvoke = T?.core?.invoke;

// 改算例配置的命令。正在运行的算例读的就是这些文件：中途改了，这次运行会混用新旧设定，
// 而跑完后 `colm-cli` 清掉"需重跑"标记，结果看起来像是按新设定算的。所以在出口统一拦下。
const CASE_WRITES = new Set([
  'set_field_batch', 'set_fields_batch', 'reset_field_batch', 'set_spinup',
  'set_process_parameter_field_batch', 'reset_process_parameter_field_batch',
  'set_pft_parameter_batch', 'set_pft_parameters_batch',
  'apply_import_parameter_overrides', 'configure_ozone_batch', 'configure_cbl_batch',
  'hybrid_install', 'hybrid_remove',
]);

function writeTargets(args) {
  if (Array.isArray(args?.dirs)) return args.dirs;
  if (Array.isArray(args?.changes)) return args.changes.flatMap(change => change.dirs ?? []);
  return [];
}

export const invoke = rawInvoke && ((cmd, args) => {
  if (CASE_WRITES.has(cmd) && state.runningCases?.size) {
    const busy = writeTargets(args).filter(dir => state.runningCases.has(dir));
    if (busy.length) {
      return Promise.reject(`${busy.length} 个算例正在运行，现在改设定会混进这次运行；请等它跑完或取消后再改`);
    }
  }
  return rawInvoke(cmd, args);
});
export const listen = T?.event?.listen;
export const hasBackend = !!invoke;
