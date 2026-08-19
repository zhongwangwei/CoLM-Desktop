//! 启动。**这里只做接线**，逻辑都在各自的模块里。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { initShell, renderSteps, setStatus } from './shell.js';
import { renderFields } from './params.js';
import { refreshKernels, watchRun } from './runner.js';
import { refreshPresets } from './presets.js';
import { restoreRecent, wirePickers } from './recent.js';
import { showDomainGate } from './domain.js';

initShell();

// 门先立起来，后台初始化在它后面照常跑 —— 用户点完站点时界面已经就绪。
// **门不拦后台的错误**：list_kernels 失败、示例数据装不上，照常落状态栏，
// 选完站点就看得见。把错误藏在门后面等于延迟暴露。
showDomainGate();

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
  addEventListener('colm:mode', () => { if (state.selected) renderFields(); });
}
