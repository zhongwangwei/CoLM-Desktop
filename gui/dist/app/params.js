//! 配置页签与字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';

export function renderTabs() {
  const groups = ['nl_colm', 'nl_colm_forcing', 'nl_colm_history'];
  const t = $('tabs');
  t.textContent = '';
  for (const g of groups) {
    const b = document.createElement('button');
    b.textContent = { nl_colm: '算例', nl_colm_forcing: '强迫场', nl_colm_history: '输出变量' }[g];
    b.setAttribute('aria-pressed', String(state.group === g));
    b.onclick = () => { state.group = g; renderTabs(); renderFields(); };
    t.appendChild(b);
  }
}

export async function renderFields() {
  const box = $('fields');
  box.textContent = '';
  if (!state.text) { box.innerHTML = '<p class="muted">先在左边选一个算例</p>'; return; }
  let entries;
  try { entries = await invoke('read_case', { text: state.text }); }
  catch (e) { box.textContent = String(e); return; }

  const shown = entries.filter(e => (e.group ?? 'nl_colm') === state.group);
  if (!shown.length) { box.innerHTML = '<p class="muted">这一组里这份配置没有设任何字段</p>'; return; }
  const tbl = document.createElement('table');
  for (const e of shown) {
    const tr = document.createElement('tr');
    const k = document.createElement('td');
    k.textContent = e.path;
    if (!e.known) {
      k.className = 'warn';
      k.title = 'CoLM 不认识这个字段';
    } else {
      // schema 里 713 个字段有 108 个带 CoLM 自己的行尾注释。有就显示出来，
      // 顺带把声明的默认值也放上去 —— 用户最常问的就是「不改会怎样」。
      const meta = state.fields.find(f => f.name === e.path);
      if (meta) k.title = (meta.doc ? meta.doc + '\n' : '') + '默认 ' + meta.default;
    }
    const v = document.createElement('td');
    if (e.derived) {
      // 有声明有默认值，但不在任何 namelist 组里 —— 用户设了也没用。
      // 给一个改了没用的输入框比只读地显示更糟。
      v.textContent = e.value + '（派生值，改不了）';
      v.className = 'muted';
    } else {
      const inp = document.createElement('input');
      inp.value = e.value.replace(/^'|'$/g, '');
      inp.onchange = async () => {
        try {
          state.text = await invoke('set_field',
            { text: state.text, path: e.path, value: inp.value });
          await invoke('write_text', { path: state.selected.dir + '/case.nml', text: state.text });
          $('status').textContent = `已保存 ${e.path}`;
        } catch (err) {
          // 类型不对在后端就被拦下了，原样报出来 —— 它说得比我们编的具体
          $('status').textContent = String(err);
          inp.value = e.value.replace(/^'|'$/g, '');
        }
      };
      v.appendChild(inp);
    }
    tr.appendChild(k); tr.appendChild(v); tbl.appendChild(tr);
  }
  box.appendChild(tbl);
}
