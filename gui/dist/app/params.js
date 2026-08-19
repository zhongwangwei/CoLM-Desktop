//! 配置页签与字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { renderHistVars } from './histvars.js';
import { renderTiming } from './timing.js';
import { editTarget } from './batch.js';



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

/** 顶上一条横幅，说清楚"改一下会动几个文件"。
 *
 *  **不能只在状态栏事后说。** 状态栏是改完之后才出现的，而这里要回答的是
 *  改之前那个问题：我现在改的是一个还是二十个。旁边给一个立刻缩回单个的
 *  按钮 —— 想给某一个站点单独设个值时，不用退回上一步重来。 */
function renderScope(box) {
  const dirs = editTarget();
  if (dirs.length < 2) return;
  const bar = document.createElement('div');
  bar.className = 'expert-note';
  bar.style.marginBottom = '10px';
  const names = dirs.map(baseName);
  bar.innerHTML = `下面的改动会写进 <b>${dirs.length} 个算例</b>：`
    + names.slice(0, 6).join('、') + (names.length > 6 ? ` 等 ${names.length} 个` : '');
  const b = document.createElement('button');
  b.className = 'btn-ghost';
  b.style.marginLeft = '10px';
  b.textContent = `只改 ${state.selected?.name ?? names[0]}`;
  b.onclick = () => {
    state.batch = state.selected ? [state.selected.dir] : [dirs[0]];
    renderFields();
  };
  bar.appendChild(b);
  box.appendChild(bar);
}

export async function renderFields() {
  const box = $('fields');
  box.textContent = '';
  // 时间与预热在页签**之上**，两个子页签下都看得见 —— 它不属于"参数"
  // 或"输出变量"里的任何一个，而是这份算例跑多久、从哪天开始出结果。
  await renderTiming();
  if (!state.text) { box.innerHTML = '<p class="muted">先在左边选一个算例</p>'; return; }
  let entries;
  try { entries = await invoke('read_case', { text: state.text }); }
  catch (e) { box.textContent = String(e); return; }
  // 这一批里取值不一致的字段。**必须标出来** —— 一个显示着某个值的输入框
  // 其实代表着 20 个不同的值，而改它会把另外 19 个悄悄抹平。
  try {
    state.varies = new Set(await invoke('varying_fields', { dirs: editTarget() }));
  } catch (e) { state.varies = new Set(); status(e); }

  // 专家模式：把这份配置**没设过**的字段也补进来，显示 schema 默认值并标灰。
  // 判据用 `minimal::required` 已有的那条分界（设了的 vs 等于默认值的），
  // 不另发明一套 —— 它可解释，而且和 .nml 文件里看到的一致。
  let entriesAll = entries;
  if (state.expert) {
    const have = new Set(entries.map(e => e.path));
    const extra = state.fields
      .filter(f => !have.has(f.name))
      // 482 个 `DEF_hist_vars%*` 不在这里露面 —— 它们有自己的一页（§1.1）。
      // 铺进这张表的话，「输出变量」页签在专家模式下会变成 482 行。
      .filter(f => !f.name.startsWith('DEF_hist_vars%'))
      .map(f => ({ path: f.name, value: f.default, known: true, group: f.group,
                   derived: f.derived, unset: true }));
    entriesAll = entries.concat(extra);
  }
  // 子页签换成「参数 / 输出变量」—— 原来那三个页签是 CoLM 的 namelist
  // 分组名（决定字段写进哪个文件），那是**程序的内部结构，不是用户要做的事**。
  if (state.ptab === 'hist') { await renderHistVars(box); return; }
  // 三个 namelist 组一起显示。`group` 仍然有用（决定写进哪个文件），
  // 但它不再是导航轴 —— 分类由下面九个功能节承担。
  // 482 个 DEF_hist_vars 例外：它们在「输出变量」子页签里。
  const inGroup = entriesAll.filter(e => !e.path.startsWith('DEF_hist_vars%'));
  // 当前内核编不进去的字段默认不显示 —— 用户设了不会有任何效果。
  const hidden = inGroup.filter(e => state.irrelevant.has(e.path));
  const shown = state.showIrrelevant ? inGroup : inGroup.filter(e => !state.irrelevant.has(e.path));
  if (!shown.length) { box.innerHTML = '<p class="muted">这一组里这份配置没有设任何字段</p>'; return; }
  renderScope(box);
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
    if (state.varies.has(e.path)) {
      // 这一行显示的是代表算例的值，别的算例不是这个值。改它会抹平全部。
      k.textContent += ' ⚠';
      k.className = 'warn';
      k.title = (k.title ? k.title + '\n\n' : '')
        + '这一批算例在这个字段上取值不同，显示的是第一个的值。改它会把全部改成同一个值。';
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
          // 后端读改写全部算例，成功后把**代表算例**的新内容带回来。
          // 前端不再自己 write_text —— 那条路只写得动一个文件。
          const r = await invoke('set_field_batch',
            { dirs: editTarget(), path: e.path, value: inp.value });
          state.text = r.text;
          status(r.written > 1 ? `已写入 ${r.written} 个算例：${e.path}` : `已保存 ${e.path}`);
          // 改过之后这个字段就一致了，标记要跟着消失。
          if (state.varies.delete(e.path)) renderFields();
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
