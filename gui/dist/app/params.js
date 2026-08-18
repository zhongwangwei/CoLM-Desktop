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

// 控件按 schema 的类型选，不一律给文本框。
//
// 顶层 202 个字段里 **99 个是 logical** —— 差不多一半的界面在让人手打
// `.true.` / `.false.`，而拼错要等 CoLM 读 namelist 时才报。另有 12 个字段
// 有固定取值集合（从 CoLM 自己的 `select case` 与 `==` 分支扫出来）。
//
// **写回文件的仍是 Fortran 字面量** —— `colm-namelist` 的往返保证不能因为
// 界面换了控件就破掉。
function control(e, meta) {
  const raw = e.value.replace(/^'|'$/g, '');
  const kind = meta?.kind ?? '';
  if (meta?.values?.length) {
    const s = document.createElement('select');
    for (const v of meta.values) {
      const o = document.createElement('option');
      o.value = v; o.textContent = v;
      s.appendChild(o);
    }
    // 文件里的值可能不在集合里（上游加了新取值，或者用户手写的）。
    // 那时把它作为一项补进去并选中 —— 悄悄改成第一项是最糟的做法。
    if (!meta.values.includes(raw)) {
      const o = document.createElement('option');
      o.value = raw; o.textContent = raw + '（不在已知取值里）';
      s.appendChild(o);
    }
    s.value = raw;
    return s;
  }
  if (kind.startsWith('Logical')) {
    const s = document.createElement('select');
    for (const [v, label] of [['.true.', '是（.true.）'], ['.false.', '否（.false.）']]) {
      const o = document.createElement('option');
      o.value = v; o.textContent = label;
      s.appendChild(o);
    }
    s.value = /true|\.t\./i.test(raw) ? '.true.' : '.false.';
    return s;
  }
  const inp = document.createElement('input');
  if (kind.startsWith('Integer') || kind.startsWith('Real')) {
    inp.type = 'number';
    // 实数不限步长；整数按 1。`any` 让浏览器不对小数报警。
    inp.step = kind.startsWith('Integer') ? '1' : 'any';
  }
  inp.value = raw;
  return inp;
}

export async function renderFields() {
  const box = $('fields');
  box.textContent = '';
  if (!state.text) { box.innerHTML = '<p class="muted">先在左边选一个算例</p>'; return; }
  let entries;
  try { entries = await invoke('read_case', { text: state.text }); }
  catch (e) { box.textContent = String(e); return; }

  const inGroup = entries.filter(e => (e.group ?? 'nl_colm') === state.group);
  // 当前内核编不进去的字段默认不显示 —— 用户设了不会有任何效果。
  const hidden = inGroup.filter(e => state.irrelevant.has(e.path));
  const shown = state.expert ? inGroup : inGroup.filter(e => !state.irrelevant.has(e.path));
  if (!shown.length) { box.innerHTML = '<p class="muted">这一组里这份配置没有设任何字段</p>'; return; }
  const tbl = document.createElement('table');
  for (const e of shown) {
    const tr = document.createElement('tr');
    const k = document.createElement('td');
    k.textContent = e.path;
    // schema 元数据在下面选控件时也要用，所以在这里取一次，
    // 不放进 else 分支里 —— 放进去的话 `control(e, meta)` 就取不到它了。
    const meta = state.fields.find(f => f.name === e.path);
    if (!e.known) {
      k.className = 'warn';
      k.title = 'CoLM 不认识这个字段';
    } else {
      // schema 里 713 个字段有 108 个带 CoLM 自己的行尾注释。有就显示出来，
      // 顺带把声明的默认值也放上去 —— 用户最常问的就是「不改会怎样」。
      if (meta) k.title = (meta.doc ? meta.doc + '\n' : '') + '默认 ' + meta.default;
      if (state.irrelevant.has(e.path)) {
        k.className = 'muted';
        k.title = `本内核未编入（需要 ${meta?.requires?.join('、') ?? '某个宏'}），设了也没用\n` + (k.title ?? '');
      }
    }
    const v = document.createElement('td');
    if (e.derived) {
      // 有声明有默认值，但不在任何 namelist 组里 —— 用户设了也没用。
      // 给一个改了没用的输入框比只读地显示更糟。
      v.textContent = e.value + '（派生值，改不了）';
      v.className = 'muted';
    } else {
      const inp = control(e, meta);
      inp.onchange = async () => {
        try {
          state.text = await invoke('set_field',
            { text: state.text, path: e.path, value: inp.value });
          await invoke('write_text', { path: state.selected.dir + '/case.nml', text: state.text });
          status(`已保存 ${e.path}`);
        } catch (err) {
          // 类型不对在后端就被拦下了，原样报出来 —— 它说得比我们编的具体
          status(err);
          inp.value = e.value.replace(/^'|'$/g, '');
        }
      };
      v.appendChild(inp);
    }
    tr.appendChild(k); tr.appendChild(v); tbl.appendChild(tr);
  }
  box.appendChild(tbl);

  // **藏起来不等于假装不存在。** 换个内核这些字段就该回来，
  // 而看不见又找不到会让人以为程序坏了。
  if (hidden.length && !state.expert) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.style.cssText = 'font-size:11px;cursor:pointer';
    p.textContent = `+ ${hidden.length} 个字段本内核未编入（${hidden.map(h => h.path).slice(0, 3).join('、')}${hidden.length > 3 ? ' 等' : ''}），点此展开`;
    p.onclick = () => { state.expert = true; renderFields(); };
    box.appendChild(p);
  } else if (state.expert && hidden.length) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.style.cssText = 'font-size:11px;cursor:pointer';
    p.textContent = `专家模式：正在显示 ${hidden.length} 个本内核未编入的字段，点此收起`;
    p.onclick = () => { state.expert = false; renderFields(); };
    box.appendChild(p);
  }
}
