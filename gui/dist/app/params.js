//! 基本设定与过程参数字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { renderHistVars } from './histvars.js';
import { renderTiming } from './timing.js';
import { editTarget } from './batch.js';
import { wizardFieldNames } from './domain.js';
import { urbanEnabled } from './kernel.js';

// 分类在后端从 MOD_Namelist.F90 的字段名与 namelist 组推导，并有测试保证
// 新字段不能掉进「其他」。基本设定与过程参数各自只认这一份归属表。
const BASIC_PAGES = [
  { id: 'basic-site', target: 'basic-site-fields', sections: ['站点'] },
  { id: 'basic-grid', target: 'basic-grid-fields', sections: ['网格与并行'] },
  { id: 'basic-surface', target: 'basic-surface-fields', sections: ['地表数据'] },
  { id: 'basic-initial', target: 'basic-initial-fields', sections: ['初始场'] },
  { id: 'basic-forcing', target: 'basic-forcing-fields', sections: ['强迫场'] },
];
const PARAM_PAGES = [
  { id: 'params-water', target: 'param-water-fields', sections: ['水热过程'] },
  { id: 'params-eco', target: 'param-eco-fields', sections: ['生态与生地化'],
    enabled: () => !!state.wizard?.physics?.bgc },
  { id: 'params-river', target: 'param-river-fields', sections: ['河道与水库'] },
  { id: 'params-da', target: 'param-da-fields', sections: ['数据同化'] },
  { id: 'params-tracer', target: 'param-tracer-fields', sections: ['示踪剂'],
    enabled: () => !!state.wizard?.physics?.tracer },
  { id: 'params-urban', target: 'param-urban-fields', sections: ['城市'],
    enabled: urbanEnabled },
];

// MOD_Namelist 会在 Urban 打开时无条件关掉这六项。让用户看见并修改一组
// 必然被覆盖的开关，只会造成“明明设了却没生效”。
const URBAN_DISABLED_FIELDS = new Set([
  'DEF_USE_WUEST', 'DEF_USE_SUPERCOOL_WATER', 'DEF_USE_PLANTHYDRAULICS',
  'DEF_USE_OZONESTRESS', 'DEF_USE_OZONEDATA', 'DEF_SPLIT_SOILSNOW',
]);


// 少数几个字段光看名字会理解反，在这里补一句。
//
// **不是给每个字段配说明** —— schema 里 108 个字段已经带着 CoLM 自己的
// 行尾注释（`meta.doc`），那些直接显示就够了。这张表只收「名字会误导人」
// 的那几个，保持短。
const HINTS = {
  'DEF_forcing_namelist':
    '强迫场 namelist 的路径。CoLM 会直接打开并读取这个文件（MOD_Namelist.F90:1392），不能删除。',
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

/** 向导已定下的字段不在主界面重复出现。 */
export function withoutWizardFields(entries) {
  const owned = new Set(wizardFieldNames());
  return entries.filter(e => !owned.has(e.path));
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
  const output = $('output-fields');
  const hist = $('hist-fields');
  const basics = BASIC_PAGES.map(p => [p, $(p.target)]);
  const processes = PARAM_PAGES.map(p => [p, $(p.target)]);
  const flows = new Set(['basic-files']);
  output.textContent = '';
  hist.textContent = '';
  for (const [, basic] of basics) basic.textContent = '';
  for (const [, process] of processes) process.textContent = '';
  // 时间在基本设定，输出在运行页，但仍写回同一份 case.nml。
  await renderTiming();
  if (!state.text) {
    output.innerHTML = '<p class="muted">先选一个算例</p>';
    for (const [, basic] of basics) {
      basic.innerHTML = '<p class="muted">先在“文件与目录”里选择站点并建算例</p>';
    }
    for (const [, process] of processes) {
      process.innerHTML = '<p class="muted">先在“文件与目录”里选择站点并建算例</p>';
    }
    publishFlows(flows);
    return;
  }
  let entries;
  try { entries = await invoke('read_case', { text: state.text }); }
  catch (e) {
    for (const [, target] of basics.concat(processes)) target.textContent = String(e);
    status(e);
    publishFlows(flows);
    return;
  }
  // 这一批里取值不一致的字段。**必须标出来** —— 一个显示着某个值的输入框
  // 其实代表着 20 个不同的值，而改它会把另外 19 个悄悄抹平。
  try {
    state.varies = new Set(await invoke('varying_fields', { dirs: editTarget() }));
  } catch (e) { state.varies = new Set(); status(e); }

  // 列表来自源码 schema，而不是只列 case.nml 里已经写过的项；
  // 再按向导自动匹配的编译产物过滤。
  const have = new Set(entries.map(e => e.path));
  const extra = state.fields
    .filter(f => !have.has(f.name))
    // 这里只编辑 case.nml 的 nl_colm 组。forcing/history 是另外的 namelist；
    // 派生项虽然不属于任何组，但仍要显示 —— 它们回答「这个值现在是多少」，
    // 排在各分节末尾，只读。
    .filter(f => f.group === 'nl_colm' || f.derived)
    .map(f => ({ path: f.name, value: f.default, known: true, group: f.group,
                 derived: f.derived, unset: true }));
  const entriesAll = withoutWizardFields(entries.concat(extra));
  const inGroup = entriesAll.filter(e => !e.path.startsWith('DEF_hist_vars%'));
  // 严格跟随所选内核。**只读派生项不再藏在专家模式后面** ——
  // 全仓库只有 6 个（DEF_dir_landdata/restart/history、DEF_USE_USGS/IGBP、
  // DEF_wetland_finundation_scheme），它们是「这个值现在是多少」的答案，
  // 而那是个常规问题。
  const shown = inGroup
    .filter(e => !state.irrelevant.has(e.path))
    .filter(e => !urbanEnabled() || !URBAN_DISABLED_FIELDS.has(e.path));
  const sectionOf = e => state.fields.find(f => f.name === e.path)?.section;
  const outputFields = shown.filter(e => sectionOf(e) === '输出与重启');
  flows.add('basic-timing');

  for (const [page, basic] of basics) {
    const rows = shown.filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (!rows.length) {
      basic.innerHTML = '<p class="muted">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    renderScope(basic);
    basic.appendChild(table(rows));
  }

  for (const [page, process] of processes) {
    const rows = (page.enabled && !page.enabled() ? [] : shown)
      .filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (!rows.length) {
      process.innerHTML = '<p class="muted">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    renderScope(process);
    process.appendChild(table(rows));
  }

  if (outputFields.length) {
    renderScope(output);
    // 与参数页各分节同一条规矩：可编辑的在前，只读派生项排到末尾。
    // `DEF_dir_restart` 与 `DEF_dir_history` 归的是「输出与重启」——
    // `config.rs` 里**显式列举**了这两个名字，早于 `DEF_DIR` 前缀规则，
    // 所以它们落在这个分支，而不是上面那个 PARAM_SECTIONS 循环里。
    output.appendChild(table(
      outputFields.slice().sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0))));
  } else {
    output.innerHTML = '<p class="muted">当前配置没有可配置的输出参数。</p>';
  }
  publishFlows(flows);
  await renderHistVars(hist);
}

function publishFlows(flows) {
  state.availableFlows = flows;
  globalThis.dispatchEvent?.(new Event('colm:flows'));
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
