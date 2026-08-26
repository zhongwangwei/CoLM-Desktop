//! 482 个输出变量：搜索 + 开关 + 「勾了到底写不写得出来」。
//!
//! 不与其余字段共用那张表（见 plan-gui2.md §1.1）。它们全是 logical，
//! 各配一个输入框既没必要也没法看；真正有价值的信息是**这一个勾上了会不会
//! 真的出现在输出里** —— 那要同时看编译期的宏与运行时的开关，
//! `colm-hist` 的闸门表两层都记着。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { markResultsStale } from './results.js';
import { editTarget } from './batch.js';
import { $, status } from './ui.js';

export async function renderHistVars(box) {
  const kernel = $('kernel').value;
  if (!kernel) { box.innerHTML = '<p class="muted">当前安装缺少与向导配置匹配的运行产物</p>'; return; }
  let vars;
  try { vars = await invoke('hist_vars', { text: state.text, kernelDir: kernel }); }
  catch (e) { box.textContent = String(e); return; }

  const n = { on: 0, blocked: 0, unknown: 0 };
  for (const v of vars) {
    if (v.on) n.on++;
    if (v.writable === false) n.blocked++;
    if (v.writable === null) n.unknown++;
  }

  const bar = document.createElement('div');
  bar.className = 'row';
  const f = document.createElement('input');
  f.placeholder = `搜索 ${vars.length} 个输出变量`;
  f.value = state.histFilter ?? '';
  f.style.flex = '1';
  f.oninput = () => { state.histFilter = f.value; renderHistVars(box); };
  bar.appendChild(f);
  const only = document.createElement('button');
  only.textContent = state.histOnlyOn ? '只看已勾选' : '全部';
  only.setAttribute('aria-pressed', String(!!state.histOnlyOn));
  only.onclick = () => { state.histOnlyOn = !state.histOnlyOn; renderHistVars(box); };
  bar.appendChild(only);
  box.textContent = '';
  box.appendChild(bar);

  const sum = document.createElement('p');
  sum.className = 'muted';
  sum.style.fontSize = '11px';
  // 「勾了 N 个」不是用户真正想知道的；「其中 M 个写不出来」才是。
  sum.textContent =
    `已勾选 ${n.on} 个` +
    (n.blocked ? ` · ${n.blocked} 个在当前配置下写不出来` : '') +
    (n.unknown ? ` · ${n.unknown} 个未知` : '');
  box.appendChild(sum);

  const q = (state.histFilter ?? '').trim().toLowerCase();
  let shown = q ? vars.filter(v => v.name.toLowerCase().includes(q)) : vars;
  if (state.histOnlyOn) shown = shown.filter(v => v.on);
  if (!shown.length) { box.insertAdjacentHTML('beforeend', '<p class="muted">没有匹配的变量</p>'); return; }

  const tbl = document.createElement('table');
  for (const v of shown) {
    const tr = document.createElement('tr');
    const c = document.createElement('td');
    const cb = document.createElement('input');
    cb.type = 'checkbox';
    cb.checked = v.on;
    cb.disabled = v.settable === false;
    cb.title = v.settable === false ? '由模型内置输出选择器控制' : '';
    cb.style.width = 'auto';
    cb.onchange = async () => {
      try {
        // 输出变量与其他参数走同一条批量路径 —— 勾选的 20 个算例要输出
        // 同一批变量，否则结果页拿它们做比对时会缺变量。
        const dirs = editTarget();
        const r = await invoke('set_field_batch', {
          dirs, path: `DEF_hist_vars%${v.name}`,
          value: cb.checked ? '.true.' : '.false.',
          kernelDir: kernel,
        });
        state.text = r.text;
        await markResultsStale(dirs);
        status(r.written > 1 ? `已写入 ${r.written} 个算例：${v.name}` : `已保存 ${v.name}`);
      } catch (e) { status(e); cb.checked = v.on; }
    };
    c.appendChild(cb);
    const nm = document.createElement('td');
    nm.textContent = v.name;
    const why = document.createElement('td');
    if (v.writable === false) {
      // 勾了却没有输出是这一页最该防的事。**说出原因**，而不是只标个灰。
      why.className = 'warn';
      why.textContent = v.blocked_by ?? '写不出来';
    } else if (v.writable === null) {
      why.className = 'muted';
      why.textContent = '未知';
      why.title = v.blocked_by ?? '';
    } else if (v.settable === false) {
      why.className = 'muted';
      why.textContent = '由甲烷输出选择器控制';
    }
    tr.appendChild(c); tr.appendChild(nm); tr.appendChild(why);
    tbl.appendChild(tr);
  }
  box.appendChild(tbl);
}
