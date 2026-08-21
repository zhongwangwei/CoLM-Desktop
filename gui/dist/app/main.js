//! 启动。**这里只做接线**，逻辑都在各自的模块里。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { initShell, renderSteps, setStatus } from './shell.js';
import { refreshKernels, watchRun } from './runner.js';
import { refreshPresets } from './presets.js';
import { restoreRecent, wirePickers } from './recent.js';
import { showDomainGate } from './domain.js';
import { renderCases, checkRootSpace } from './sites.js';
// 只为它们顶层的 `$('fprobe').onclick = …` / `$('smake').onclick = …`
// 接线而 import —— 前处理页两个子栏（强迫场、站点属性）的状态都是各自
// 模块内部的闭包变量，没有别的模块要用它们的导出。
import './forcing.js';
import './sitedata.js';

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
    // `restoreRecent` 对文本框只赋值、不派发 `change`（见那里的注释），
    // 而 `checkRootSpace` 挂在 `#root` 的 `change`/`input` 上 ——
    // 恢复出来的旧路径若含空格，不补这一次调用就要等用户自己碰一下
    // 那个框才会被标出来。
    checkRootSpace();
    // 恢复出来的算例根目录里可能已经有算例 —— 不扫的话基本设定是个空盒子，
    // 而用户上次的工作就在那里。
    const root = $('root').value.trim();
    if (root) {
      try {
        state.cases = await invoke('list_cases', { root });
        renderCases();
      } catch (e) { /* 目录没了就算了，扫描按钮还在 */ }
    }
    wirePickers();
    renderSteps();
  } catch (e) { setStatus('后端出错：' + e); }
  await watchRun();
}
