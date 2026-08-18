//! 启动。**这里只做接线**，逻辑都在各自的模块里。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { initShell, renderSteps, setStatus, go } from './shell.js';
import { renderFields } from './params.js';
import { refreshKernels, watchRun } from './runner.js';
import { refreshPresets } from './presets.js';
import { restoreRecent, wirePickers } from './recent.js';

initShell();

if (!hasBackend) {
  // 直接用浏览器打开这个文件时没有 IPC。说清楚而不是渲染成一片空白。
  setStatus('没有 IPC 后端 —— 这个页面不在 Tauri 里运行');
} else {
  boot();
}

async function boot() {
  try {
    setStatus(await invoke('backend_ready'));
    state.fields = await invoke('describe_fields');
    await refreshKernels();
    await refreshPresets();
    await restoreRecent();
    wirePickers();
    renderSteps();
  } catch (e) { setStatus('后端出错：' + e); }
  await watchRun();
  // 参数页的两个子页签（参数 / 输出变量）共用一块渲染区。
  addEventListener('colm:ptab', e => { state.ptab = e.detail; renderFields(); });
  addEventListener('colm:mode', () => { if (state.step === 'params') renderFields(); });
  go('data');
}
