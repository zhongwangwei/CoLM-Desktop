//! 配置页签与字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderHistVars } from './histvars.js';

export function renderTabs() {
  const groups = ['nl_colm', 'nl_colm_forcing', 'nl_colm_history'];
  const t = $('tabs');
  t.textContent = '';
  for (const g of groups) {
    const b = document.createElement('button');
    b.textContent = { nl_colm: '算例', nl_colm_forcing: '强迫场', nl_colm_history: '输出变量' }[g];
    b.setAttribute('aria-pressed', String(state.group === g));
    b.onclick = () => { state.group = g; renderTabs(); renderFields(); };
    // 输出变量那一组有 482 个开关，走自己的渲染（§1.1）—— 铺进字段表
    // 会把这一页变成 482 行，而它们全是 logical，不需要各配一个输入框。
    t.appendChild(b);
  }
}


// 九个功能分类。**判据是字段名前缀，不是主观的「重要性」** —— 前缀是 CoLM
// 自己的命名，会随上游一起演进；主观分类会在下一次上游加字段时立刻过时。
//
// 顺序即优先级，每个字段只进第一个匹配上的分类。`other` 是兜底且**必须在
// 最后**：新字段进来不会消失，而是显眼地堆在「其他」里 —— 那正是提醒
// 有人该给它归类的信号。
export const GROUPS = [
  { id: 'site',    label: '站点',     match: n => n.startsWith('SITE_') || n.startsWith('USE_SITE_') },
  { id: 'time',    label: '时间',     match: n => n.startsWith('DEF_simulation_time') },
  { id: 'dirs',    label: '路径',     match: n => n.startsWith('DEF_dir') || n === 'DEF_forcing_namelist' },
  { id: 'urban',   label: '城市',     match: n => n.includes('URBAN') || n.includes('Urban') },
  { id: 'soil',    label: '土壤',     match: n => /SOIL|Soil|soil/.test(n) },
  { id: 'physics', label: '物理开关', match: n => n.startsWith('DEF_USE_') },
  { id: 'forcing', label: '强迫场',   match: (n, f) => f?.group === 'nl_colm_forcing' },
  { id: 'output',  label: '输出',     match: n => /^DEF_(HIST|hist|WRST)/.test(n) },
  { id: 'other',   label: '其他',     match: () => true },
];

function groupOf(name, meta) {
  return GROUPS.find(g => g.match(name, meta)) ?? GROUPS[GROUPS.length - 1];
}

// 「常改项」白名单：一个人跑一个新站点时几乎一定要看的东西。
// **保持短** —— 长白名单等于没有普通模式。
const ALWAYS_SHOWN = [
  'DEF_simulation_time%start_year', 'DEF_simulation_time%end_year',
  'DEF_HIST_FREQ', 'DEF_dir_output',
];


// 少数几个字段光看名字会理解反，在这里补一句。
//
// **不是给每个字段配说明** —— schema 里 108 个字段已经带着 CoLM 自己的
// 行尾注释（`meta.doc`），那些直接显示就够了。这张表只收「名字会误导人」
// 的那几个，保持短。
const HINTS = {
  'DEF_simulation_time%spinup_repeat':
    '预热轮数：起始日**之前**那段反复跑几遍，让土壤温湿等状态趋于平衡。\n' +
    '预热期不写 history（MOD_Hist.F90:235 在 itstamp <= ptstamp 时直接 RETURN），' +
    '所以它不会污染输出，也不会被算进指标。\n' +
    '与结果页的「丢弃前 N 条记录」不是一回事：那个丢的是输出记录，单位是条。',
  'DEF_simulation_time%spinup_year':
    '预热截止时刻。起始时刻早于它，中间那段就是预热期。四项（年月日秒）一起决定。',
  'DEF_simulation_time%spinup_month': '预热截止时刻的月，见 spinup_repeat 的说明。',
  'DEF_simulation_time%spinup_day': '预热截止时刻的日，见 spinup_repeat 的说明。',
  'DEF_simulation_time%spinup_sec': '预热截止时刻的当天秒数，见 spinup_repeat 的说明。',
};

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

  // 专家模式：把这份配置**没设过**的字段也补进来，显示 schema 默认值并标灰。
  // 判据用 `minimal::required` 已有的那条分界（设了的 vs 等于默认值的），
  // 不另发明一套 —— 它可解释，而且和 .nml 文件里看到的一致。
  let entriesAll = entries;
  if (state.expert) {
    const have = new Set(entries.map(e => e.path));
    const extra = state.fields
      .filter(f => !have.has(f.name))
      .filter(f => f.group === state.group)
      // 482 个 `DEF_hist_vars%*` 不在这里露面 —— 它们有自己的一页（§1.1）。
      // 铺进这张表的话，「输出变量」页签在专家模式下会变成 482 行。
      .filter(f => !f.name.startsWith('DEF_hist_vars%'))
      .map(f => ({ path: f.name, value: f.default, known: true, group: f.group,
                   derived: f.derived, unset: true }));
    entriesAll = entries.concat(extra);
  }
  if (state.group === 'nl_colm_history') { await renderHistVars(box); return; }
  const inGroup = entriesAll.filter(e => (e.group ?? 'nl_colm') === state.group);
  // 当前内核编不进去的字段默认不显示 —— 用户设了不会有任何效果。
  const hidden = inGroup.filter(e => state.irrelevant.has(e.path));
  const shown = state.showIrrelevant ? inGroup : inGroup.filter(e => !state.irrelevant.has(e.path));
  if (!shown.length) { box.innerHTML = '<p class="muted">这一组里这份配置没有设任何字段</p>'; return; }
  renderToolbar(box, inGroup.length, shown.length);

  // 按九分类分节。分节而不是分页签：`nl_colm` 那一组有 214 个字段，
  // 而另外两组只有 35 与 482（后者另有专页），只有它需要再分。
  // 页签仍按 namelist 组分 —— 那决定**写进哪个文件**，是另一回事。
  const filter = state.fieldFilter?.trim().toLowerCase() ?? '';
  const visible = filter ? shown.filter(e => e.path.toLowerCase().includes(filter)) : shown;
  const buckets = new Map(GROUPS.map(g => [g.id, []]));
  for (const e of visible) {
    buckets.get(groupOf(e.path, state.fields.find(f => f.name === e.path)).id).push(e);
  }

  for (const g of GROUPS) {
    const rows = buckets.get(g.id);
    if (!rows.length) continue;
    const h = document.createElement('h2');
    h.textContent = `${g.label}（${rows.length}）`;
    h.style.marginTop = '14px';
    box.appendChild(h);
    box.appendChild(table(rows));
  }
  if (!visible.length) {
    box.insertAdjacentHTML('beforeend', `<p class="muted">没有名字含「${filter}」的字段</p>`);
  }

  // **藏起来不等于假装不存在。** 换个内核这些字段就该回来，
  // 而看不见又找不到会让人以为程序坏了。
  if (hidden.length && !state.showIrrelevant) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.style.cssText = 'font-size:11px;cursor:pointer';
    p.textContent = `+ ${hidden.length} 个字段本内核未编入（${hidden.map(h => h.path).slice(0, 3).join('、')}${hidden.length > 3 ? ' 等' : ''}），点此展开`;
    p.onclick = () => { state.showIrrelevant = true; renderFields(); };
    box.appendChild(p);
  } else if (state.showIrrelevant && hidden.length) {
    const p = document.createElement('p');
    p.className = 'muted';
    p.style.cssText = 'font-size:11px;cursor:pointer';
    p.textContent = `专家模式：正在显示 ${hidden.length} 个本内核未编入的字段，点此收起`;
    p.onclick = () => { state.showIrrelevant = false; renderFields(); };
    box.appendChild(p);
  }
}

/** 顶部一行：普通/专家切换 + 过滤框。 */
function renderToolbar(box, total, shown) {
  const bar = document.createElement('div');
  bar.className = 'row';
  bar.style.marginBottom = '8px';
  const b = document.createElement('button');
  b.textContent = state.expert ? `专家模式（${shown} 项）` : `普通模式（${shown} 项）`;
  b.title = state.expert
    ? '正在显示全部字段，含这份配置没设过的（灰色，值是 CoLM 的默认值）'
    : '只显示这份配置实际设了的字段。点击查看全部。';
  b.setAttribute('aria-pressed', String(state.expert));
  b.onclick = () => { state.expert = !state.expert; renderFields(); };
  bar.appendChild(b);
  const f = document.createElement('input');
  f.placeholder = '过滤字段名';
  f.value = state.fieldFilter ?? '';
  f.style.flex = '1';
  // input 而不是 change：202 个字段时边打边筛才有用。
  f.oninput = () => { state.fieldFilter = f.value; renderFields(); };
  bar.appendChild(f);
  box.appendChild(bar);
  // 过滤框重绘后会失焦，补回去 —— 否则打第二个字符就得再点一次。
  if (state.fieldFilter) { f.focus(); f.setSelectionRange(f.value.length, f.value.length); }
}

/** 一组字段渲染成一张表。分节之后每节各调一次。 */
function table(shown) {
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
      const hint = HINTS[e.path];
      if (hint) {
        k.title = hint;
        // 有说明的字段要看得出来有说明 —— 一个只在悬停时才出现的提示，
        // 等于没有。
        k.textContent = e.path + ' ⓘ';
        k.style.cursor = 'help';
      }
      if (meta) {
        k.title = (hint ? hint + '\n\n' : '') + (meta.doc ? meta.doc + '\n' : '') + '默认 ' + meta.default;
      }
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
      // 未设过的字段标灰：它显示的是 CoLM 的默认值，不是这份文件里的内容。
      if (e.unset) { inp.style.opacity = '0.55'; v.title = '这份配置没设它，显示的是默认值'; }
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
  return tbl;
}
