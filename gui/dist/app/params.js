//! 基本设定与过程参数字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { renderHistVars } from './histvars.js';
import { renderTiming } from './timing.js';
import { editTarget } from './batch.js';
import { wizardFieldNames } from './domain.js';
import { language } from './i18n.js';
import { fieldLabel, optionLabel, technicalFieldHint } from './param-presentation.js';

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
  { id: 'params-eco', target: 'param-eco-fields', sections: ['生态与生地化'] },
  { id: 'params-river', target: 'param-river-fields', sections: ['河道与水库'] },
  { id: 'params-da', target: 'param-da-fields', sections: ['数据同化'] },
  { id: 'params-tracer', target: 'param-tracer-fields', sections: ['示踪剂'] },
  { id: 'params-urban', target: 'param-urban-fields', sections: ['城市'] },
];


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

// 打开这些父开关时，缺少路径就不是“以后再补”的半成品，而是下一阶段
// 必然失败的配置。选择 true 后立即打开原生选择器；取消则保持原值不变。
const PATH_ON_ENABLE = Object.freeze({
  DEF_USE_SoilInit: { path: 'DEF_file_SoilInit', kind: 'file' },
  DEF_USE_SnowInit: { path: 'DEF_file_SnowInit', kind: 'file' },
  DEF_USE_CN_INIT: { path: 'DEF_file_cn_init', kind: 'file' },
  DEF_USE_WaterTableInit: { path: 'DEF_file_WaterTable', kind: 'file' },
  DEF_USE_Forcing_Downscaling: { path: 'DEF_DS_HiresTopographyDataDir', kind: 'folder' },
});
const PATH_FIELDS = Object.freeze(Object.fromEntries(
  Object.values(PATH_ON_ENABLE).map(spec => [spec.path, spec.kind])
    .concat([['DEF_file_Ozone', 'file']])));

// CoLM 对这两个开关分别分配同名、不同形状的数组，不能同时为 true。
// 用户启用一个时在同一次原子写入里关闭另一个，不要求先手工关闭另一方案。
const MUTEX_ON_ENABLE = Object.freeze({
  DEF_USE_Forcing_Downscaling: 'DEF_USE_Forcing_Downscaling_Simple',
  DEF_USE_Forcing_Downscaling_Simple: 'DEF_USE_Forcing_Downscaling',
});

// 这两项只改变模型取值，不参与 `field_runtime_state` 的显隐约束。保存后保留
// 当前控件而不是重建整张参数表，避免用户刚选完方案，行位置和滚动位置就跳动。
const STABLE_IN_PLACE_FIELDS = new Set([
  'DEF_precip_phase_discrimination_scheme',
  'DEF_SSP',
]);

const enabled = value => /true|\.t\./i.test(String(value));

async function pickParameterPath(path, kind) {
  try {
    const picked = kind === 'file'
      ? await invoke('pick_file', { key: path, filter: 'nc,nc4' })
      : await invoke('pick_folder', { key: path });
    if (picked) await invoke('save_recent', { key: path, value: picked });
    return picked;
  } catch (e) {
    status(e);
    return null;
  }
}

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
function control(e, meta, fieldState) {
  if (e.synthetic === 'stomatal') {
    const s = document.createElement('select');
    const values = e.value === 'INVALID'
      ? ['INVALID', 'BALL_BERRY', 'MEDLYN', 'WUE']
      : ['BALL_BERRY', 'MEDLYN', 'WUE'];
    for (const value of values) {
      const o = document.createElement('option');
      o.value = value;
      o.textContent = optionLabel(e.path, value, language());
      s.appendChild(o);
    }
    s.value = e.value;
    s.className = 'select';
    return s;
  }
  const raw = e.value.replace(/^'|'$/g, '');
  const kind = meta?.kind ?? '';
  const runtimeValues = fieldState?.allowed_values ?? [];
  const knownValues = runtimeValues.length ? runtimeValues : (meta?.values ?? []);
  if (kind.startsWith('Logical')) {
    // 互斥降尺度由 onchange 原子地切换，所以即使另一项当前已打开，也必须
    // 让用户直接选“启用”，不能逼他先去另一行手工关闭。
    const allowed = MUTEX_ON_ENABLE[e.path]
      ? ['.true.', '.false.']
      : (runtimeValues.length ? runtimeValues : ['.true.', '.false.']);
    const s = document.createElement('select');
    for (const v of allowed) {
      const yes = /true|\.t\./i.test(v);
      const o = document.createElement('option');
      o.value = yes ? '.true.' : '.false.';
      o.textContent = optionLabel(e.path, o.value, language());
      s.appendChild(o);
    }
    const current = /true|\.t\./i.test(raw) ? '.true.' : '.false.';
    // 已有配置可能违反新约束（例如两个降尺度开关都为 true）。保留当前值
    // 作为可见选项，用户才能看见并把它改回合法状态。
    if (![...s.children].some(o => o.value === current)) {
      const o = document.createElement('option');
      o.value = current;
      o.textContent = optionLabel(e.path, current, language()) + '（当前值不满足约束）';
      s.appendChild(o);
    }
    s.value = current;
    s.className = 'select';
    return s;
  }
  if (knownValues.length) {
    const s = document.createElement('select');
    for (const v of knownValues) {
      const o = document.createElement('option');
      o.value = v; o.textContent = optionLabel(e.path, v, language());
      s.appendChild(o);
    }
    // 文件里的值可能不在集合里（上游加了新取值，或者用户手写的）。
    // 那时把它作为一项补进去并选中 —— 悄悄改成第一项是最糟的做法。
    if (!knownValues.includes(raw)) {
      const o = document.createElement('option');
      o.value = raw;
      o.textContent = optionLabel(e.path, raw, language()) + '（不在已知取值里）';
      s.appendChild(o);
    }
    s.value = raw;
    s.className = 'select';
    return s;
  }
  const inp = document.createElement('input');
  inp.className = 'input';
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
  const flows = new Set(['basic-files', 'basic-timing', 'basic-grid']);
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
  // 内核宏和 case.nml 当前值统一在 Rust 配置层判定。这里不再复制城市、BGC、
  // SinglePoint 等规则；父字段保存后重新调用，子字段会立刻出现或消失。
  let fieldStates = new Map();
  const kernelDir = $('kernel').value;
  try {
    if (!kernelDir) throw new Error('请先选择或安装 CoLM 内核');
    const runtimeStates = await invoke('field_states_batch', { dirs: editTarget(), kernelDir });
    fieldStates = new Map(runtimeStates.map(item => [item.name, item]));
    if (fieldStates.size !== state.fields.length) {
      throw new Error(`字段状态不完整：后端返回 ${fieldStates.size}/${state.fields.length}`);
    }
  } catch (e) {
    // 运行时规则拿不到时必须 fail closed。退回编译期过滤会把 SinglePoint、
    // 城市或 BGC 下无效的参数重新露出来，让用户以为它们会生效。
    const message = `无法核实当前配置下哪些参数有效：${e}`;
    const showError = target => {
      const p = document.createElement('p');
      p.className = 'warn';
      p.textContent = message;
      target.replaceChildren(p);
    };
    for (const [, target] of basics.concat(processes)) {
      showError(target);
    }
    showError(output);
    showError(hist);
    state.fieldStates = new Map();
    status(message);
    publishFlows(flows);
    return;
  }
  state.fieldStates = fieldStates;
  // **只读派生项不再藏在专家模式后面** ——
  // 全仓库只有 6 个（DEF_dir_landdata/restart/history、DEF_USE_USGS/IGBP、
  // DEF_wetland_finundation_scheme），它们是「这个值现在是多少」的答案，
  // 而那是个常规问题。
  const shown = inGroup
    // 未知字段仍需显示为错误，已知字段则一律服从后端；不保留第二套前端规则。
    .filter(e => !e.known || fieldStates.get(e.path)?.mode !== 'hidden');
  const sectionOf = e => state.fields.find(f => f.name === e.path)?.section;
  const outputFields = shown.filter(e => sectionOf(e) === '输出与重启');
  for (const [page, basic] of basics) {
    const rows = shown.filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (!rows.length) {
      basic.innerHTML = '<p class="muted">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    renderScope(basic);
    basic.appendChild(table(rows, fieldStates));
  }

  for (const [page, process] of processes) {
    let rows = shown.filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (page.id === 'params-eco') rows = collapseStomatal(rows);
    if (!rows.length) {
      process.innerHTML = '<p class="muted">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    renderScope(process);
    process.appendChild(table(rows, fieldStates));
  }

  if (outputFields.length) {
    renderScope(output);
    // 与参数页各分节同一条规矩：可编辑的在前，只读派生项排到末尾。
    // `DEF_dir_restart` 与 `DEF_dir_history` 归的是「输出与重启」——
    // `config.rs` 里**显式列举**了这两个名字，早于 `DEF_DIR` 前缀规则，
    // 所以它们落在这个分支，而不是上面那个 PARAM_SECTIONS 循环里。
    output.appendChild(table(
      outputFields.slice().sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0)),
      fieldStates));
  } else {
    output.innerHTML = '<p class="muted">当前配置没有可配置的输出参数。</p>';
  }
  publishFlows(flows);
  await renderHistVars(hist);
}

/** 两个底层 logical 实际表示一个三选一方案：都关是 Ball–Berry。 */
function collapseStomatal(rows) {
  const medlyn = rows.find(e => e.path === 'DEF_USE_MEDLYNST');
  const wue = rows.find(e => e.path === 'DEF_USE_WUEST');
  if (!medlyn && !wue) return rows;
  const medlynOn = medlyn && enabled(medlyn.value);
  const wueOn = wue && enabled(wue.value);
  const value = medlynOn && wueOn ? 'INVALID'
    : medlynOn ? 'MEDLYN'
      : wueOn ? 'WUE' : 'BALL_BERRY';
  const first = rows.findIndex(e => e.path === 'DEF_USE_MEDLYNST' || e.path === 'DEF_USE_WUEST');
  const collapsed = rows.filter(e => e.path !== 'DEF_USE_MEDLYNST' && e.path !== 'DEF_USE_WUEST');
  collapsed.splice(first, 0, {
    path: 'GUI_STOMATAL_CONDUCTANCE_SCHEME',
    value,
    known: true,
    synthetic: 'stomatal',
    sourcePaths: ['DEF_USE_MEDLYNST', 'DEF_USE_WUEST'],
    unset: Boolean(medlyn?.unset && wue?.unset),
  });
  return collapsed;
}

function publishFlows(flows) {
  state.availableFlows = flows;
  globalThis.dispatchEvent?.(new Event('colm:flows'));
}

/** 一组字段渲染成一张表。分节之后每节各调一次。 */
function table(shown, fieldStates = new Map()) {
  const tbl = document.createElement('table');
  tbl.className = 'parameter-table';
  for (const e of shown) {
    const tr = document.createElement('tr');
    const k = document.createElement('td');
    // schema 元数据在下面选控件时也要用，所以在这里取一次，
    // 不放进 else 分支里 —— 放进去的话 `control(e, meta)` 就取不到它了。
    const meta = state.fields.find(f => f.name === e.path);
    const fieldState = fieldStates.get(e.path);
    if (!e.known) {
      k.textContent = e.path;
      k.className = 'warn';
      k.title = 'CoLM 不认识这个字段';
    } else {
      // 主标签说人话，CoLM 原始键保留在 tooltip 供查文档和排错。
      k.textContent = fieldLabel(e.path, language());
      k.title = e.synthetic === 'stomatal'
        ? '由 DEF_USE_MEDLYNST 与 DEF_USE_WUEST 两个底层开关原子写入；两者都关闭时使用 Ball–Berry。'
        : technicalFieldHint(e.path, language());
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
        const details = [technicalFieldHint(e.path, language())];
        if (hint) details.push(hint);
        if (meta.doc) details.push(meta.doc);
        details.push('默认：' + optionLabel(e.path, meta.default, language()));
        k.title = details.join('\n\n');
      }
      if (fieldState?.reason) {
        k.title = (k.title ? k.title + '\n\n' : '') + fieldState.reason;
      }
    }
    const warnings = [];
    if (fieldState?.mixed) {
      warnings.push('这一批算例对该字段的适用条件不同；它只对其中一部分算例生效。');
    }
    if (state.varies.has(e.path) || e.sourcePaths?.some(path => state.varies.has(path))) {
      // 这一行显示的是代表算例的值，别的算例不是这个值。改它会抹平全部。
      warnings.push('这一批算例在这个字段上取值不同，显示的是第一个的值。改它会把全部改成同一个值。');
    }
    if (warnings.length) {
      k.textContent += ' ⚠';
      k.className = 'warn';
      k.title = (k.title ? k.title + '\n\n' : '') + warnings.join('\n');
    }
    const v = document.createElement('td');
    if (e.derived) {
      // 有声明有默认值，但不在任何 namelist 组里 —— 用户设了也没用。
      // 给一个改了没用的输入框比只读地显示更糟。
      v.textContent = optionLabel(e.path, e.value, language()) + '（派生值，改不了）';
      v.className = 'muted';
    } else {
      const inp = control(e, meta, fieldState);
      if (fieldState?.mode === 'disabled') inp.disabled = true;
      // 未设过的字段标灰：它显示的是 CoLM 的默认值，不是这份文件里的内容。
      if (e.unset) { inp.style.opacity = '0.55'; v.title = '这份配置没设它，显示的是默认值'; }
      inp.onchange = async () => {
        const before = e.value.replace(/^'|'$/g, '');
        try {
          if (e.synthetic === 'stomatal') {
            if (inp.value === 'INVALID') return;
            const fields = [
              { path: 'DEF_USE_MEDLYNST', value: inp.value === 'MEDLYN' ? '.true.' : '.false.' },
              { path: 'DEF_USE_WUEST', value: inp.value === 'WUE' ? '.true.' : '.false.' },
            ];
            const r = await invoke('set_fields_batch', {
              dirs: editTarget(),
              fields,
            });
            state.text = r.text;
            state.varies.delete('DEF_USE_MEDLYNST');
            state.varies.delete('DEF_USE_WUEST');
            status(r.written > 1
              ? `已为 ${r.written} 个算例设置气孔导度方案`
              : '已保存气孔导度方案');
            await renderFields();
            return;
          }
          if (enabled(inp.value) && e.path === 'DEF_USE_CBL_HEIGHT') {
            const picked = await pickParameterPath('DEF_USE_CBL_HEIGHT', 'file');
            if (!picked) { inp.value = before; return; }
            const r = await invoke('configure_cbl_batch', { dirs: editTarget(), file: picked });
            state.text = r.text;
            status('已校验并接入边界层高度文件');
            await renderFields();
            return;
          }
          if (enabled(inp.value)
              && (e.path === 'DEF_USE_OZONESTRESS' || e.path === 'DEF_USE_OZONEDATA')) {
            const picked = await pickParameterPath('DEF_file_Ozone', 'file');
            if (!picked) { inp.value = before; return; }
            const r = await invoke('configure_ozone_batch', { dirs: editTarget(), file: picked });
            state.text = r.text;
            status('已校验臭氧数据，并启用臭氧胁迫与数据读取');
            await renderFields();
            return;
          }
          if (e.path === 'DEF_file_Ozone') {
            const r = await invoke('configure_ozone_batch', { dirs: editTarget(), file: inp.value });
            state.text = r.text;
            status('已校验并更换臭氧数据文件');
            await renderFields();
            return;
          }
          const changes = [{ path: e.path, value: inp.value }];
          if (enabled(inp.value) && PATH_ON_ENABLE[e.path]) {
            const spec = PATH_ON_ENABLE[e.path];
            const picked = await pickParameterPath(spec.path, spec.kind);
            // 取消选择就不打开父开关，避免留下 true + null。
            if (!picked) { inp.value = before; return; }
            changes.push({ path: spec.path, value: picked });
          }
          if (enabled(inp.value) && MUTEX_ON_ENABLE[e.path]) {
            changes.push({ path: MUTEX_ON_ENABLE[e.path], value: '.false.' });
          }
          if (!enabled(inp.value) && e.path === 'DEF_USE_OZONESTRESS') {
            changes.push({ path: 'DEF_USE_OZONEDATA', value: '.false.' });
          }
          // 独立地下水位文件在 SoilInit 下会被 CoLM 忽略；打开完整土壤初始场
          // 时顺手关掉这个无效父开关，配置文件也与实际执行保持一致。
          if (enabled(inp.value) && e.path === 'DEF_USE_SoilInit') {
            changes.push({ path: 'DEF_USE_WaterTableInit', value: '.false.' });
          }
          // 后端读改写全部算例，成功后把**代表算例**的新内容带回来。
          // 前端不再自己 write_text —— 那条路只写得动一个文件。
          const r = changes.length > 1
            ? await invoke('set_fields_batch', { dirs: editTarget(), fields: changes })
            : await invoke('set_field_batch',
              { dirs: editTarget(), path: e.path, value: inp.value });
          state.text = r.text;
          status(r.written > 1 ? `已写入 ${r.written} 个算例：${e.path}` : `已保存 ${e.path}`);
          // 父开关会改变其他行是否有效，通常保存后要重新读取统一状态；
          // 明确不参与显隐规则的枚举则留在原位，避免无意义的整页跳动。
          state.varies.delete(e.path);
          if (STABLE_IN_PLACE_FIELDS.has(e.path)) {
            e.value = inp.value;
            e.unset = false;
            inp.style.opacity = '';
            v.title = '';
            return;
          }
          await renderFields();
        } catch (err) {
          // 类型不对在后端就被拦下了，原样报出来 —— 它说得比我们编的具体
          status(err);
          inp.value = e.value.replace(/^'|'$/g, '');
        }
      };
      v.appendChild(inp);
      if (PATH_FIELDS[e.path]) {
        const pick = document.createElement('button');
        pick.type = 'button';
        pick.className = 'btn-ghost';
        pick.style.marginLeft = '8px';
        pick.textContent = PATH_FIELDS[e.path] === 'file' ? '选择文件…' : '选择目录…';
        pick.onclick = async () => {
          const chosen = await pickParameterPath(e.path, PATH_FIELDS[e.path]);
          if (!chosen) return;
          if (e.path === 'DEF_file_Ozone') {
            try {
              const r = await invoke('configure_ozone_batch', { dirs: editTarget(), file: chosen });
              state.text = r.text;
              status('已校验并更换臭氧数据文件');
              await renderFields();
            } catch (err) { status(err); }
            return;
          }
          inp.value = chosen;
          inp.dispatchEvent(new Event('change'));
        };
        v.appendChild(pick);
      }
    }
    tr.appendChild(k); tr.appendChild(v); tbl.appendChild(tr);
  }
  return tbl;
}

// 切换中英文时方案名也要跟着变；它们不是静态 HTML，通用文本替换无法知道
// 同一个 `I` 在两个不同方案字段里分别代表什么。
globalThis.addEventListener?.('colm:language', () => {
  if (state.text) renderFields();
});
