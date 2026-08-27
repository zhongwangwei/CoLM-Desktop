//! 启动。**这里只做接线**，逻辑都在各自的模块里。

import { invoke, hasBackend, listen } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { initShell, renderSteps, setStatus } from './shell.js';
import { refreshKernels, watchRun } from './runner.js';
import { restoreRecent, wirePickers } from './recent.js';
import { renderCases, checkRootSpace } from './sites.js';
import { initI18n } from './i18n.js';
// 只为它们顶层的 `$('fprobe').onclick = …` / `$('smake').onclick = …`
// 接线而 import —— 前处理页两个子栏（强迫场、站点属性）的状态都是各自
// 模块内部的闭包变量，没有别的模块要用它们的导出。
import './forcing.js';
import './sitedata.js';

initI18n();
initShell();

function beginLiveResize(e) {
  const live = document.querySelector('.live');
  if (!live || document.querySelector('.app')?.classList.contains('live-collapsed')) return;
  const stacked = innerWidth <= 1240;
  const apply = ev => {
    const size = stacked
      ? Math.min(innerHeight - 220, Math.max(160, innerHeight - 34 - ev.clientY))
      : Math.min(720, Math.max(260, innerWidth - ev.clientX));
    document.documentElement.style.setProperty(stacked ? '--live-h' : '--live-w', `${size}px`);
  };
  const up = () => {
    document.body.classList.remove('resizing-live');
    window.removeEventListener('pointermove', apply);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', up);
  };
  e.preventDefault();
  e.stopPropagation();
  e.currentTarget?.setPointerCapture?.(e.pointerId);
  document.body.classList.add('resizing-live');
  apply(e);
  window.addEventListener('pointermove', apply);
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', up);
}

$('live-resizer').addEventListener('pointerdown', beginLiveResize);

if (listen) {
  listen('colm-about', event => {
    $('about-version').textContent = event.payload;
    $('aboutDialog').showModal();
  });
}

if (!hasBackend) {
  // 直接用浏览器打开这个文件时没有 IPC。说清楚而不是渲染成一片空白。
  setStatus('没有 IPC 后端 —— 这个页面不在 Tauri 里运行');
}

// 启动页只等这一件事；不要再维护第二套“加载完成”状态。
export const ready = hasBackend ? boot() : Promise.resolve();

async function boot() {
  try {
    setStatus(await invoke('backend_ready'));
    state.fields = await invoke('describe_fields');
    await refreshKernels();
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
  } catch (e) {
    setStatus('后端出错：' + e);
    throw e;
  }
  await watchRun();
}
