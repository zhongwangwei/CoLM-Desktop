//! 与后端说话的唯一出口。
//!
//! **全部 `invoke` / `listen` 都从这里出去。** `xtask check-gui` 靠扫这些
//! 字面量核对前后端接口；散落各处扫得到，但集中一处才看得出「一共有哪些」。
//! 更要紧的是下面那个 `hasBackend` —— 用浏览器直接打开页面时没有 IPC，
//! 每个模块各自判一次会得到不一致的降级行为。

const T = window.__TAURI__;

export const invoke = T?.core?.invoke;
export const listen = T?.event?.listen;
export const hasBackend = !!invoke;
