//! 算例库扫描、选中、新建。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderFields } from './params.js';
import { refreshVars } from './results.js';

$('rescan').onclick = async () => {
  try {
    state.cases = await invoke('list_cases', { root: $('root').value.trim() });
    renderCases();
  } catch (e) { $('status').textContent = String(e); }
};

export function renderCases() {
  const box = $('cases');
  box.textContent = '';
  if (!state.cases.length) {
    box.innerHTML = '<p class="muted" style="font-size:11px">这个目录下没有算例</p>';
    return;
  }
  for (const c of state.cases) {
    const d = document.createElement('div');
    d.className = 'case';
    d.setAttribute('aria-selected', String(state.selected?.dir === c.dir));
    const s = document.createElement('small');
    s.textContent = c.has_history ? '已跑过' : '未跑';
    d.textContent = c.name;
    d.appendChild(s);
    d.onclick = () => selectCase(c);
    box.appendChild(d);
  }
}

async function selectCase(c) {
  state.selected = c;
  renderCases();
  $('run').disabled = false;
  try {
    state.text = await invoke('read_text', { path: c.dir + '/case.nml' });
    const unknown = await invoke('unknown_fields', { text: state.text });
    const u = $('unknown');
    u.textContent = '';
    if (unknown.length) {
      // 上游自己发布的单点示例就设了两个已删除的字段，CoLM 读到会
      // `Cannot match namelist object name` 然后停。在开跑前说，别让人对着报错发呆。
      const p = document.createElement('p');
      p.className = 'warn';
      p.style.fontSize = '12px';
      p.textContent =
        `这份配置有 ${unknown.length} 个 CoLM 已经不认识的字段，会让运行在读取时就停：` +
        unknown.join('、');
      u.appendChild(p);
    }
    renderFields();
    refreshVars();
  } catch (e) { $('status').textContent = String(e); }
}

$('create').onclick = async () => {
  const site = $('w-site').value.trim();
  const root = $('root').value.trim();
  if (!site || !root) { $('status').textContent = '要先填站点文件与算例目录'; return; }
    // Windows 上的分隔符是反斜杠，两种都认
  const stem = site.split(/[\\/]/).pop();
  const name = $('w-name').value.trim() || stem.split('_')[0];
  $('create').disabled = true;
  try {
    const msg = await invoke('new_case', {
      site, out: root + '/' + name, name,
      start: $('w-start').value.trim() || null,
      end: $('w-end').value.trim() || null,
    });
    $('log').textContent = msg;
    state.cases = await invoke('list_cases', { root });
    renderCases();
    $('status').textContent = '算例已建好';
  } catch (e) { $('status').textContent = String(e); }
  finally { $('create').disabled = false; }
};
