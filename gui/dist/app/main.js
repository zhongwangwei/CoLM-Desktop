//! 启动。**这里只做接线**，逻辑都在各自的模块里。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderTabs } from './params.js';
import { refreshKernels, watchRun } from './runner.js';
import { refreshPresets } from './presets.js';
import { restoreRecent } from './recent.js';

if (!hasBackend) {
  // 直接用浏览器打开这个文件时没有 IPC。说清楚而不是渲染成一片空白。
  status('没有 IPC 后端 —— 这个页面不在 Tauri 里运行');
} else {
  boot();
}

async function boot() {
  try {
    status(await invoke('backend_ready'));
    state.fields = await invoke('describe_fields');
    renderTabs();
    await refreshKernels();
    await refreshPresets();
    await restoreRecent();
  } catch (e) { status('后端出错：' + e); }
  await watchRun();
}
