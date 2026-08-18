//! 内核选择、运行控制、进度与日志。

import { invoke, listen } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderCases } from './sites.js';
import { refreshVars } from './results.js';

// 「选个目录」而是「选一套物理」—— 让它列出来，而不是让人记住路径。
export async function refreshKernels() {
  const s = $('kernel');
  state.kernels = await invoke('list_kernels');
  s.textContent = '';
  if (!state.kernels.length) {
    // 只在开发树里可能发生：装出来的程序自带三个预设。
    const o = document.createElement('option');
    o.textContent = '没有找到内核'; o.value = '';
    s.appendChild(o);
    showKernelMeta();
    return;
  }
  for (const k of state.kernels) {
    const o = document.createElement('option');
    o.value = k.dir; o.textContent = k.preset;
    s.appendChild(o);
  }
  s.onchange = showKernelMeta;
  showKernelMeta();
}

// 目录名不是身份，宏组合才是。把它显示出来，免得「我选的 urban 到底编没编
// URBAN」这种问题要靠读 manifest.json 才能答。
function showKernelMeta() {
  const k = state.kernels?.find(k => k.dir === $('kernel').value);
  $('kernelmeta').textContent = k
    ? `${k.generator_args}  ·  CoLM ${k.colm_git_sha}  ·  ${k.platform}`
    : '\u00a0';
}

$('run').onclick = async () => {
  if (!state.selected) return;
  $('run').disabled = true;
  $('log').textContent = '';
  $('prog').style.width = '0';
  $('progtext').textContent = '启动…';
  try {
    await invoke('run_case', { case: state.selected.dir, kernel: $('kernel').value });
  } catch (e) {
    // run://done 只在子进程真的起来之后才会发。起不来的话这里是唯一的收尾点，
    // 不写的话进度文字会永远停在「启动…」。
    $('status').textContent = String(e);
    $('progtext').textContent = '没能启动 —— ' + e;
    $('run').disabled = false;
  }
};

/** 订阅三个运行事件。由 `main.js` 在启动时调一次。 */
export async function watchRun() {
    await listen('run://progress', e => {
      // 进度只到「第几步、模型时间到哪天」为止。总步数要等算例配置解析出来才知道，
      // 在那之前不假装知道百分比 —— 一条走到一半又跳回去的进度条比没有更糟。
      $('progtext').textContent = `第 ${e.payload.step} 步 · ${e.payload.date}`;
      const w = Math.min(96, 6 + Math.log10(e.payload.step + 1) * 30);
      $('prog').style.width = w + '%';
    });
    await listen('run://lines', e => {
      const el = $('log');
      // 事件是**成批**到的（后端每 100 毫秒合并一次），所以这里一次追加一批。
      el.textContent += e.payload.join('\n') + '\n';
      if (el.textContent.length > 60000) el.textContent = el.textContent.slice(-40000);
      el.scrollTop = el.scrollHeight;
    });
    await listen('run://done', e => {
      const d = e.payload;
      $('prog').style.width = d.code === 0 ? '100%' : '0';
      $('progtext').textContent =
        `${d.code === 0 ? '完成' : '失败（退出码 ' + d.code + '）'} · ` +
        `子进程打了 ${d.total} 行，丢弃 ${d.dropped} 行噪声`;
      $('run').disabled = false;
      if (d.code === 0 && state.selected) {
        // list_cases 是运行**之前**扫的，这个标记那时还是 false ——
        // 不在这里更新的话，跑完第一次「画图」仍然是灰的，
        // 而用户完全看不出为什么。
        state.selected.has_history = true;
        renderCases();
        refreshVars();
      }
    });
}
