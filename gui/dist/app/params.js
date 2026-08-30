//! 基本设定与过程参数字段表格。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { renderHistVars } from './histvars.js';
import { markResultsStale } from './results.js';
import { renderTiming } from './timing.js';
import { editTarget, currentCases } from './batch.js';
import { wizardFieldNames } from './domain.js';
import { language } from './i18n.js';
import { go } from './shell.js';
import { landCoverClasses, landCoverLabel } from './land-cover.js';
import {
  fieldLabel, fieldOptions, fortranNumberInputValue, optionLabel, technicalFieldHint,
} from './param-presentation.js';

// 分类在后端从 MOD_Namelist.F90 的字段名与 namelist 组推导，并有测试保证
// 新字段不能掉进「其他」。基本设定与过程参数各自只认这一份归属表。
const BASIC_PAGES = [
  { id: 'basic-site', target: 'basic-site-fields', sections: ['站点'], scoped: true },
  { id: 'basic-grid', target: 'basic-grid-fields', sections: ['网格与并行'] },
  { id: 'basic-surface', target: 'basic-surface-fields', sections: ['地表数据'], scoped: true },
  { id: 'basic-initial', target: 'basic-initial-fields', sections: ['初始场'], scoped: true },
  { id: 'basic-forcing', target: 'basic-forcing-fields', sections: ['强迫场'], scoped: true },
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
  DEF_file_GIEMS: '卫星淹水模式必需的 GIEMS-MC 月湿地比例 NetCDF 文件。',
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
    .concat([['DEF_file_Ozone', 'file'], ['DEF_file_GIEMS', 'file']])));

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
const EXPERT_ALL = '__all__';
const EXPERT_SAME_LCT = '__same_lct__';

const PFT_SITE_CACHE = new Map();
const PFT_IDENTITY_FIELDS = new Set([
  'SITE_fsitedata', 'SITE_landtype', 'DEF_USE_LCT', 'DEF_USE_PFT', 'DEF_USE_PC',
]);

const SECTION_FLOW = Object.freeze({
  '水热过程': 'params-water',
  '生态与生地化': 'params-eco',
  '河道与水库': 'params-river',
  '数据同化': 'params-da',
  '示踪剂': 'params-tracer',
  '城市': 'params-urban',
  '站点': 'basic-site',
  '地表数据': 'basic-surface',
  '初始场': 'basic-initial',
  '强迫场': 'basic-forcing',
  '网格与并行': 'basic-grid',
});

const normalizedSearch = value => String(value ?? '').toLocaleLowerCase()
  .normalize('NFKC').replace(/[\s_\-–—℃°%]/g, '');

function catalogVisibility(rawKey) {
  const rows = state.parameterCatalog.filter(item => item.raw_key === rawKey);
  return rows.some(item => item.visibility === 'editable-common')
    ? 'editable-common' : 'editable-expert';
}

function currentCatalogRows() {
  const mode = String(state.subgrid ?? '').toUpperCase();
  return state.parameterCatalog.filter(item => {
    const id = String(item.id);
    if (id.startsWith('lct:IGBP:')) return !mode || mode === 'IGBP';
    if (id.startsWith('lct:USGS:')) return !mode || mode === 'USGS';
    if (id.startsWith('pft:')) return !mode || mode === 'PFT';
    if (id.startsWith('pc-pft:')) return !mode || mode === 'PC';
    return true;
  });
}

function searchState(item) {
  const field = state.fieldStates.get(item.raw_key)
    ?? ((String(item.id).startsWith('pft:') || String(item.id).startsWith('pc-pft:'))
      ? state.parameterPftStates.get(item.raw_key) : null);
  const inactive = field?.mode === 'hidden';
  const modified = field?.override_value != null || field?.value != null;
  const mixed = Boolean(field?.mixed || field?.override_mixed
    || field?.effective_mixed || field?.default_mixed);
  return { inactive, modified, mixed, field };
}

function matchesStatus(item, current) {
  switch (state.parameterStatusFilter) {
    case 'modified': return current.modified;
    case 'inherited': return !current.modified && !current.inactive;
    case 'inactive': return current.inactive;
    case 'tunable': return Boolean(item.calibration_eligible);
    case 'mixed': return current.mixed;
    default: return true;
  }
}

function searchText(item) {
  return normalizedSearch([
    item.id, item.raw_key, item.label_zh, item.label_en,
    fieldLabel(item.raw_key, 'zh'), fieldLabel(item.raw_key, 'en'),
    item.subgroup_zh, item.subgroup_en, item.subgroup,
    ...(item.aliases ?? []), item.source_location,
  ].filter(Boolean).join(' '));
}

async function markChanged(result, dirs) {
  if (result.changed) await markResultsStale(dirs);
}

function renderParameterSearch() {
  const box = $('parameter-search-results');
  if (!box) return;
  const query = normalizedSearch(state.parameterSearch);
  if (!query && state.parameterStatusFilter === 'all') {
    box.textContent = language() === 'en'
      ? 'Search by Chinese, English, CoLM key, or aliases such as vcmax, D50, P50, g1, and beta.'
      : '输入中文、英文、CoLM 原始键或 vcmax、D50、P50、g1、beta 等别名。';
    return;
  }
  const matches = currentCatalogRows()
    .filter(item => state.expert || item.visibility !== 'editable-expert')
    .filter(item => !query || searchText(item).includes(query))
    .map(item => ({ item, current: searchState(item) }))
    .filter(({ item, current }) => matchesStatus(item, current))
    .slice(0, 200);
  box.textContent = '';
  if (!matches.length) {
    box.textContent = language() === 'en' ? 'No matching parameters.' : '没有匹配的参数。';
    return;
  }
  for (const { item, current } of matches) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'parameter-search-hit';
    const title = document.createElement('span');
    const strong = document.createElement('strong');
    strong.textContent = item.scope === 'pft-type' || item.scope === 'pc-pft-component'
      ? (language() === 'en' ? item.label_en : item.label_zh)
      : fieldLabel(item.raw_key, language());
    const raw = document.createElement('small');
    raw.textContent = item.raw_key;
    title.append(strong, raw);
    const where = document.createElement('span');
    where.textContent = [item.section,
      language() === 'en' ? item.subgroup_en : item.subgroup_zh,
    ].filter(Boolean).join(' · ');
    const badges = document.createElement('span');
    badges.className = 'parameter-badges';
    const labels = [
      [String(item.scope_label ?? item.scope ?? '').replaceAll('_', '-'), ''],
      [current.modified ? (language() === 'en' ? 'Explicit override' : '已显式修改')
        : (language() === 'en' ? 'Built-in inherited' : '继承内置值'), current.modified ? 'modified' : ''],
      [current.inactive ? (language() === 'en' ? 'Inactive' : '当前不生效') : '', 'inactive'],
      [item.visibility === 'editable-expert' && !state.expert
        ? (language() === 'en' ? 'Expert locked' : '专家锁定') : '', 'locked'],
      [current.mixed ? (language() === 'en' ? 'Mixed' : '混合值') : '', 'inactive'],
    ];
    for (const [label, kind] of labels) {
      if (!label) continue;
      const badge = document.createElement('span');
      badge.className = `parameter-badge ${kind}`.trim();
      badge.textContent = label;
      badges.appendChild(badge);
    }
    button.append(title, where, badges);
    button.onclick = () => {
      const flow = SECTION_FLOW[item.section];
      if (!flow || !state.availableFlows.has(flow)) {
        status(current.field?.reason ?? `${item.raw_key} 当前配置下不生效`);
        return;
      }
      go(flow);
      queueMicrotask(() => {
        const row = [...document.querySelectorAll('[data-parameter-key]')]
          .find(element => element.dataset.parameterKey === item.raw_key);
        row?.closest('details')?.setAttribute('open', '');
        row?.scrollIntoView?.({ block: 'center' });
      });
    };
    box.appendChild(button);
  }
}

function wireParameterSearch() {
  const input = $('parameter-search');
  const filter = $('parameter-status-filter');
  if (!input || input.dataset.wired) return;
  input.dataset.wired = 'true';
  input.value = state.parameterSearch;
  filter.value = state.parameterStatusFilter;
  input.oninput = () => { state.parameterSearch = input.value; renderParameterSearch(); };
  filter.onchange = () => { state.parameterStatusFilter = filter.value; renderParameterSearch(); };
}

function wireParameterTransfer() {
  const exportButton = $('parameter-export');
  const importButton = $('parameter-import');
  const summary = $('parameter-import-summary');
  if (!exportButton || exportButton.dataset.wired) return;
  exportButton.dataset.wired = 'true';
  exportButton.onclick = async () => {
    const dirs = editTarget();
    if (!dirs.length) return status('没有可导出的算例');
    try {
      const bundle = await invoke('export_parameter_overrides', {
        dirs, kernelDir: $('kernel').value || null,
      });
      const blob = new Blob([`${JSON.stringify(bundle, null, 2)}\n`], { type: 'application/json' });
      const href = URL.createObjectURL(blob);
      const link = document.createElement('a');
      link.href = href;
      link.download = 'colm-parameter-overrides.json';
      link.click();
      URL.revokeObjectURL(href);
      status(`已导出 ${bundle.cases.reduce((n, item) => n + item.records.length, 0)} 条显式覆盖；未包含继承默认值`);
    } catch (error) { status(error); }
  };
  importButton.onclick = async () => {
    const dirs = editTarget();
    if (!dirs.length) return status('没有可导入的算例');
    try {
      const file = await invoke('pick_file', { key: 'parameter-overrides', filter: 'json' });
      if (!file) return;
      const args = { dirs, file, kernelDir: $('kernel').value || null };
      const preview = await invoke('preview_import_parameter_overrides', args);
      const applicable = preview.items.filter(item => item.status !== 'incompatible').length;
      const incompatible = preview.items.length - applicable;
      summary.textContent = `预览：${applicable} 条可应用，${incompatible} 条不兼容，${preview.files.length} 个文件会变化。`;
      if (!preview.can_apply) {
        const reasons = preview.items.filter(item => item.reason).slice(0, 3)
          .map(item => `${item.parameter_id}: ${item.reason}`).join('；');
        return status(`导入预检失败：${reasons}`);
      }
      if (!globalThis.confirm(`将应用 ${applicable} 条显式覆盖并修改 ${preview.files.length} 个文件。继续吗？`)) return;
      const result = await invoke('apply_import_parameter_overrides', {
        ...args, expectedVersion: preview.version_token,
      });
      await markChanged(result, dirs);
      status(result.changed
        ? `已原子导入 ${result.records} 条覆盖，修改 ${result.changed} 个文件`
        : '导入内容与当前显式覆盖相同，没有写文件');
      await renderFields();
    } catch (error) { status(error); }
  };
}

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
  inp.value = kind.startsWith('Real') ? (fortranNumberInputValue(raw) ?? '') : raw;
  return inp;
}

/** 批量编辑区先说清楚会动几个文件；过程参数改用独立的站点下拉。 */
function renderScope(box, dirs = editTarget()) {
  if (dirs.length < 2) return;
  const bar = document.createElement('div');
  bar.className = 'expert-note';
  bar.style.marginBottom = '10px';
  const names = dirs.map(baseName);
  bar.append('除逐站点数据文件外，下面的改动会写进 ');
  const count = document.createElement('b');
  count.textContent = `${dirs.length} 个算例`;
  bar.append(count, '：', names.slice(0, 6).join('、'));
  if (names.length > 6) bar.append(` 等 ${names.length} 个`);
  box.appendChild(bar);
}

export async function renderFields() {
  wireParameterSearch();
  wireParameterTransfer();
  renderParameterSearch();
  const output = $('output-fields');
  const hist = $('hist-fields');
  const basics = BASIC_PAGES.map(p => [p, $(p.target)]);
  const processes = PARAM_PAGES.map(p => [p, $(p.target)]);
  // 文件/预热总是基本设定；其余分栏只在后端确认有可编辑/可见字段后加入。
  // SinglePoint/Urban 会把“网格与并行”等整栏隐藏，避免空分栏误导用户。
  const flows = new Set(['basic-files', 'basic-timing']);
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
  // 批量写命令返回批次第一份文本；基本设定仍应显示算例列表当前站点。
  if (state.selected) {
    try { state.text = await invoke('read_text', { path: state.selected.dir + '/case.nml' }); }
    catch (e) { status(e); return; }
  }
  let entries;
  try { entries = await invoke('read_case', { text: state.text }); }
  catch (e) {
    for (const [, target] of basics.concat(processes)) target.textContent = String(e);
    status(e);
    publishFlows(flows);
    return;
  }
  // 列表来自源码 schema，而不是只列 case.nml 里已经写过的项；
  // 再按向导自动匹配的编译产物过滤。
  const complete = source => {
    const have = new Set(source.map(e => e.path));
    const extra = state.fields
      .filter(f => !have.has(f.name))
      // 这里只编辑 case.nml 的 nl_colm 组。forcing/history 是另外的 namelist；
      // 派生项虽然不属于任何组，但仍要显示 —— 它们回答「这个值现在是多少」，
      // 排在各分节末尾，只读。
      .filter(f => f.group === 'nl_colm' || f.derived)
      .map(f => ({ path: f.name, value: f.default, known: true, group: f.group,
                   derived: f.derived, unset: true }));
    return withoutWizardFields(source.concat(extra))
      .filter(e => !e.path.startsWith('DEF_hist_vars%'));
  };
  const inGroup = complete(entries);
  // 参数页默认编辑一个站点，也可明确切到“全部站点”。输出和预热继续使用
  // 本次批次范围；二者不是同一个选择，避免改一个站点参数时误伤整批。
  const parameterCases = expertCases();
  const batchDirs = editTarget();
  const kernelDir = $('kernel').value;
  try {
    state.parameterLctContexts = kernelDir
      ? await invoke('land_cover_contexts', {
        dirs: parameterCases.map(item => item.dir), kernelDir,
      }) : [];
  } catch (error) {
    state.parameterLctContexts = [];
    status(error);
  }
  const selectedProcessCase = expertCase();
  const processDirs = expertDirs();
  // 只在明确选择多站点时提示差异；单站编辑不需要拿其他站点的值干扰当前行。
  try {
    state.varies = new Set(await invoke('varying_fields', {
      dirs: [...new Set(batchDirs.concat(processDirs))],
    }));
  } catch (e) { state.varies = new Set(); status(e); }
  const representativeDir = selectedProcessCase?.dir ?? processDirs[0];
  let processInGroup = inGroup;
  if (representativeDir && representativeDir !== state.selected?.dir) {
    try {
      const text = await invoke('read_text', { path: representativeDir + '/case.nml' });
      processInGroup = complete(await invoke('read_case', { text }));
    } catch (e) { status(e); return; }
  }
  let fieldStates = new Map();
  let processFieldStates = new Map();
  try {
    if (!kernelDir) throw new Error('请先选择或安装 CoLM 内核');
    const runtimeStates = await invoke('field_states_batch', { dirs: batchDirs, kernelDir });
    fieldStates = new Map(runtimeStates.map(item => [item.name, item]));
    if (fieldStates.size !== state.fields.length) {
      throw new Error(`字段状态不完整：后端返回 ${fieldStates.size}/${state.fields.length}`);
    }
    if (processDirs.length === batchDirs.length
        && processDirs.every((dir, i) => dir === batchDirs[i])) {
      processFieldStates = fieldStates;
    } else {
      const processStates = await invoke('field_states_batch', { dirs: processDirs, kernelDir });
      processFieldStates = new Map(processStates.map(item => [item.name, item]));
      if (processFieldStates.size !== state.fields.length) {
        throw new Error(`过程字段状态不完整：后端返回 ${processFieldStates.size}/${state.fields.length}`);
      }
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
  renderParameterSearch();
  const withContextDefaults = (items, states) => items.map(e => {
    const value = states.get(e.path)?.context_default;
    return e.unset && value != null ? { ...e, value } : e;
  });
  // **只读派生项不再藏在专家模式后面** ——
  // 全仓库只有 6 个（DEF_dir_landdata/restart/history、DEF_USE_USGS/IGBP、
  // DEF_wetland_finundation_scheme），它们是「这个值现在是多少」的答案，
  // 而那是个常规问题。
  const shown = withContextDefaults(inGroup, fieldStates)
    // 未知字段仍需显示为错误，已知字段则一律服从后端；不保留第二套前端规则。
    .filter(e => !e.known || fieldStates.get(e.path)?.mode !== 'hidden');
  const processShown = withContextDefaults(processInGroup, processFieldStates)
    .filter(e => !e.known || processFieldStates.get(e.path)?.mode !== 'hidden');
  const sectionOf = e => state.fields.find(f => f.name === e.path)?.section;
  const outputFields = shown.filter(e => sectionOf(e) === '输出与重启');
  for (const [page, basic] of basics) {
    const scoped = page.scoped;
    const rows = (scoped ? processShown : shown)
      .filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (!rows.length) {
      basic.innerHTML = '<p class="muted">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    if (scoped) {
      renderProcessPicker(basic, parameterCases);
      basic.appendChild(table(
        rows, processFieldStates, processDirs, processDirs.length > 1, false,
      ));
    } else {
      renderScope(basic, batchDirs);
      basic.appendChild(table(rows, fieldStates, batchDirs));
    }
  }

  for (const [page, process] of processes) {
    let rows = processShown.filter(e => page.sections.includes(sectionOf(e)))
      .sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0));
    if (page.id === 'params-eco') rows = collapseStomatal(rows);
    const commonField = e => e.derived || e.synthetic || !e.known
      || catalogVisibility(e.path) === 'editable-common' || HINTS[e.path];
    const common = rows.filter(commonField);
    const expert = rows.filter(e => !commonField(e));
    if (!common.length && !expert.length) {
      process.innerHTML = '<p class="muted empty-params">当前配置没有这一类可设置项。</p>';
      continue;
    }
    flows.add(page.id);
    renderProcessPicker(process, parameterCases);
    if (page.id === 'params-eco') {
      renderLandCoverContext(process, processFieldStates, processDirs);
    }
    if (common.length) process.appendChild(table(
      common, processFieldStates, processDirs, processDirs.length > 1, false,
    ));
    if (state.expert && expert.length) {
      process.appendChild(renderTierFields(
        expert, processFieldStates, processDirs,
        language() === 'en' ? 'Tunable parameters' : '可调参数',
      ));
    }
  }
  if (state.expert) {
    await renderExpertProcessFiles(processes, flows);
    await renderPftParameters(processes, flows);
  }

  if (outputFields.length) {
    renderScope(output);
    // 与参数页各分节同一条规矩：可编辑的在前，只读派生项排到末尾。
    // `DEF_dir_restart` 与 `DEF_dir_history` 归的是「输出与重启」——
    // `config.rs` 里**显式列举**了这两个名字，早于 `DEF_DIR` 前缀规则，
    // 所以它们落在这个分支，而不是上面那个 PARAM_SECTIONS 循环里。
    output.appendChild(table(
      outputFields.slice().sort((a, b) => (a.derived ? 1 : 0) - (b.derived ? 1 : 0)),
      fieldStates, batchDirs));
  } else {
    output.innerHTML = '<p class="muted">当前配置没有可配置的输出参数。</p>';
  }
  publishFlows(flows);
  await renderHistVars(hist);
  renderParameterSearch();
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

function renderTierFields(rows, fieldStates, dirs, label) {
  const wrap = document.createElement('details');
  wrap.className = 'expert-param-file';
  const summary = document.createElement('summary');
  const groups = new Map();
  for (const row of rows) {
    const descriptor = currentCatalogRows().find(item => item.raw_key === row.path);
    const group = language() === 'en' ? descriptor?.subgroup_en : descriptor?.subgroup_zh;
    if (group) groups.set(group, (groups.get(group) ?? 0) + 1);
  }
  const breakdown = [...groups].map(([group, count]) => `${group} ${count}`).join('；');
  summary.textContent = `${label}（${rows.length}${breakdown ? `；${breakdown}` : ''}）`;
  wrap.appendChild(summary);
  wrap.appendChild(table(rows, fieldStates, dirs, dirs.length > 1, false, true));
  return wrap;
}


function expertCases() {
  const cases = currentCases();
  return cases.length ? cases : (state.selected ? [state.selected] : []);
}

function expertCase() {
  const cases = expertCases();
  if (!cases.length) return null;
  if (state.expertCaseDir === EXPERT_ALL && cases.length > 1) return cases[0];
  if (state.expertCaseDir === EXPERT_SAME_LCT) {
    return state.selected && cases.some(c => c.dir === state.selected.dir)
      ? state.selected : cases[0];
  }
  if (!cases.some(c => c.dir === state.expertCaseDir)) {
    state.expertCaseDir = state.selected && cases.some(c => c.dir === state.selected.dir)
      ? state.selected.dir : cases[0].dir;
  }
  return cases.find(c => c.dir === state.expertCaseDir) ?? cases[0];
}

function expertDirs() {
  const cases = expertCases();
  if (state.expertCaseDir === EXPERT_ALL && cases.length > 1) return cases.map(c => c.dir);
  if (state.expertCaseDir === EXPERT_SAME_LCT) {
    const reference = expertCase();
    const context = state.parameterLctContexts.find(item => item.dir === reference?.dir);
    if (context) return state.parameterLctContexts
      .filter(item => item.scheme === context.scheme && item.class_index === context.class_index)
      .map(item => item.dir);
  }
  const selected = expertCase();
  return selected ? [selected.dir] : editTarget().slice(0, 1);
}

function renderLandCoverContext(box, fieldStates, dirs) {
  const rows = [...fieldStates.values()].filter(item =>
    item.name.startsWith('DEF_LC_') && item.mode !== 'hidden');
  if (!rows.length) return;
  const context = state.parameterLctContexts.find(item => dirs.includes(item.dir));
  const card = document.createElement('div');
  card.className = 'expert-note land-cover-context';
  if (!context || rows.some(item => item.scope_label === 'mixed')) {
    card.textContent = language() === 'en'
      ? `LCT batch · ${dirs.length} cases · mixed IGBP/USGS classes and contextual defaults. Values are never flattened to the first case.`
      : `LCT 批量范围 · ${dirs.length} 个算例 · IGBP/USGS 地类或上下文默认值不一致，不会用第一个算例伪装为全局值。`;
    box.appendChild(card);
    return;
  }
  const item = landCoverClasses(context.scheme)
    .find(candidate => candidate.value === context.class_index);
  const label = item ? landCoverLabel(item, language()) : String(context.class_index);
  const explicit = rows.some(row => row.override_value != null || row.override_mixed);
  card.textContent = language() === 'en'
    ? `Current subgrid: LCT · Scheme: ${context.scheme} · Class: ${label} · Default source: MOD_Const_LC.F90 / current classification table · Explicit override: ${explicit ? 'yes' : 'no'} · Effective values are shown per row.`
    : `当前次网格：LCT · 分类体系：${context.scheme} · 当前地类：${label} · 默认值来源：MOD_Const_LC.F90 / 当前分类表 · 显式覆盖：${explicit ? '有' : '无'} · 当前有效值见各参数行。`;
  box.appendChild(card);
}

function renderProcessPicker(box, cases) {
  if (cases.length < 2 || box.querySelector('.process-site-picker')) return;
  const row = document.createElement('div');
  row.className = 'expert-site-picker process-site-picker';
  const label = document.createElement('label');
  label.textContent = language() === 'en' ? 'Edit site' : '修改站点';
  const pick = document.createElement('select');
  pick.className = 'select';
  if (cases.length > 1) {
    const all = document.createElement('option');
    all.value = EXPERT_ALL;
    all.textContent = language() === 'en' ? 'All sites' : '全部站点';
    pick.appendChild(all);
    const reference = state.selected && cases.some(c => c.dir === state.selected.dir)
      ? state.selected : cases[0];
    const lct = state.parameterLctContexts.find(item => item.dir === reference.dir);
    const same = lct && state.parameterLctContexts.filter(item =>
      item.scheme === lct.scheme && item.class_index === lct.class_index).length;
    if (same > 1) {
      const option = document.createElement('option');
      option.value = EXPERT_SAME_LCT;
      option.textContent = language() === 'en'
        ? `Same ${lct.scheme} class as current (${same} sites)`
        : `仅当前相同 ${lct.scheme} 地类（${same} 个站点）`;
      pick.appendChild(option);
    }
  }
  for (const c of cases) {
    const o = document.createElement('option');
    o.value = c.dir;
    o.textContent = c.name;
    pick.appendChild(o);
  }
  pick.value = [EXPERT_ALL, EXPERT_SAME_LCT].includes(state.expertCaseDir)
    && [...pick.options].some(option => option.value === state.expertCaseDir)
    ? state.expertCaseDir : (expertCase()?.dir ?? '');
  pick.onchange = async () => {
    state.expertCaseDir = pick.value;
    await renderFields();
  };
  label.appendChild(pick);
  row.appendChild(label);
  box.appendChild(row);
}

function processControl(entry) {
  const raw = entry.value.replace(/^'|'$/g, '');
  if (entry.kind === 'logical') {
    const s = document.createElement('select');
    s.className = 'select';
    for (const value of ['.true.', '.false.']) {
      const o = document.createElement('option');
      o.value = value;
      o.textContent = optionLabel(entry.path, value, language());
      s.appendChild(o);
    }
    s.value = /true|\.t\./i.test(entry.value) ? '.true.' : '.false.';
    return s;
  }
  const options = fieldOptions(entry.path);
  if (options.length && entry.kind !== 'list' && entry.path !== 'DEF_METHANE%ch4_history_vars') {
    const s = document.createElement('select');
    s.className = 'select';
    for (const value of options) {
      const o = document.createElement('option');
      o.value = value;
      o.textContent = optionLabel(entry.path, value, language());
      s.appendChild(o);
    }
    if (!options.includes(raw)) {
      const o = document.createElement('option');
      o.value = raw;
      o.textContent = optionLabel(entry.path, raw, language()) + '（不在已知取值里）';
      s.appendChild(o);
    }
    s.value = raw;
    return s;
  }
  const inp = document.createElement('input');
  inp.className = 'input';
  if (entry.path === 'DEF_METHANE%ch4_history_vars') inp.setAttribute('list', 'ch4-history-presets');
  inp.value = entry.kind === 'list' ? entry.value.replace(/\s+/g, ', ') : raw;
  if (entry.kind === 'integer' || entry.kind === 'real') {
    const number = entry.kind === 'real' ? fortranNumberInputValue(raw) : raw;
    if (number !== null && Number.isFinite(Number(number))) {
      inp.type = 'number';
      inp.step = entry.kind === 'integer' ? '1' : 'any';
      inp.value = number;
    } else if (entry.unset) {
      inp.value = '';
      inp.placeholder = raw;
      inp.inputMode = 'decimal';
    }
  }
  return inp;
}

function defaultValueText(path, value, kind = '') {
  if (value == null) return null;
  if (String(kind).toLowerCase() === 'real') {
    const number = fortranNumberInputValue(value);
    if (number != null) return number;
  }
  return value === ''
    ? (language() === 'en' ? '(empty)' : '（空）')
    : optionLabel(path, value, language());
}

function appendDefaultValue(cell, path, value, kind = '') {
  const contextual = path.startsWith('DEF_LC_') || ['DEF_BALL_BERRY_GRADM', 'DEF_BALL_BERRY_BINTER', 'DEF_MEDLYN_G1', 'DEF_MEDLYN_G0', 'DEF_WUE_LAMBDA'].includes(path);
  const text = defaultValueText(path, value, kind);
  if (text == null) return;
  const note = document.createElement('div');
  note.className = 'parameter-default';
  note.textContent = `${contextual ? (language() === 'en' ? 'Current land-cover default' : '当前地类默认值') : (language() === 'en' ? 'Default' : '默认值')}：${text}`;
  cell.appendChild(note);
}

function renderExpertTable(file, dirs) {
  const wrap = document.createElement('details');
  wrap.className = 'expert-param-file';
  const summary = document.createElement('summary');
  summary.textContent = language() === 'en'
    ? `Advanced process parameters: ${file.title} (${file.entries.length})`
    : `高级过程参数：${file.title}（${file.entries.length}）`;
  wrap.appendChild(summary);
  const tbl = document.createElement('table');
  tbl.className = 'parameter-table expert-parameter-table';
  for (const entry of file.entries) {
    const tr = document.createElement('tr');
    tr.dataset.parameterKey = entry.path;
    const k = document.createElement('td');
    k.textContent = fieldLabel(entry.path, language());
    const defaultText = defaultValueText(entry.path, entry.default, entry.kind);
    k.title = `${entry.path}\n&${entry.group}`
      + (defaultText == null ? '' : `\n${language() === 'en' ? 'Code default' : '代码默认值'}：${defaultText}`)
      + (entry.doc ? `\n${entry.doc}` : '');
    const v = document.createElement('td');
    const inp = processControl(entry);
    inp.title = defaultText == null
      ? '' : `${language() === 'en' ? 'Code default' : '代码默认值'}：${defaultText}`;
    if (entry.unset) {
      inp.style.opacity = '0.55';
      inp.title += (inp.title ? '\n' : '') + '当前文件未设置，显示代码默认值';
    }
    inp.onchange = async () => {
      if (entry.unset && !inp.value.trim()) return;
      try {
        const r = await invoke('set_process_parameter_field_batch', {
          dirs, file: file.file, path: entry.path, value: inp.value,
        });
        if (state.selected && dirs.includes(state.selected.dir)) state.text = r.text || state.text;
        await markChanged(r, dirs);
        status(r.written > 1
          ? `已写入 ${r.written} 个站点：${entry.path}`
          : `已保存 ${baseName(dirs[0])}：${entry.path}`);
        await renderFields();
      } catch (e) {
        status(e);
        const raw = entry.value.replace(/^'|'$/g, '');
        inp.value = entry.kind === 'real' ? (fortranNumberInputValue(raw) ?? raw) : raw;
      }
    };
    v.appendChild(inp);
    const reset = document.createElement('button');
    reset.type = 'button';
    reset.className = 'btn-ghost';
    reset.style.marginLeft = '8px';
    reset.textContent = language() === 'en' ? 'Use code default' : '恢复代码默认值';
    reset.disabled = entry.unset;
    reset.onclick = async () => {
      try {
        const r = await invoke('reset_process_parameter_field_batch', {
          dirs, file: file.file, path: entry.path,
        });
        if (state.selected && dirs.includes(state.selected.dir)) state.text = r.text || state.text;
        await markChanged(r, dirs);
        status(r.changed
          ? `${entry.path} 已删除显式覆盖`
          : `${entry.path} 已继承代码默认值`);
        await renderFields();
      } catch (e) { status(e); }
    };
    v.appendChild(reset);
    appendDefaultValue(v, entry.path, entry.default, entry.kind);
    appendCatalogDetails(v, entry.path, entry);
    tr.appendChild(k); tr.appendChild(v); tbl.appendChild(tr);
  }
  wrap.appendChild(tbl);
  return wrap;
}

async function renderExpertProcessFiles(processes, flows) {
  const cases = expertCases();
  const dirs = expertDirs();
  if (!dirs.length) return;
  let files = [];
  try {
    const lists = await Promise.all(dirs.map(dir => invoke('process_parameter_files', { dir })));
    files = commonProcessFiles(lists);
  } catch (e) {
    status(e);
    return;
  }
  for (const [page, target] of processes) {
    const mine = files.filter(file => page.sections.includes(file.section));
    if (!mine.length) continue;
    target.querySelector('.empty-params')?.remove();
    const note = document.createElement('div');
    note.className = 'expert-note';
    note.textContent = '专家模式：这些值来自算例本地过程参数文件，不写入公共 case.nml。';
    target.appendChild(note);
    renderProcessPicker(target, cases);
    for (const file of mine) target.appendChild(renderExpertTable(file, dirs));
    flows.add(page.id);
  }
}

function commonProcessFiles(lists) {
  if (!lists.length) return [];
  const tail = lists.slice(1);
  return lists[0].map(file => {
    const entries = file.entries.filter(entry => tail.every(list => {
      const peer = list.find(other => other.file === file.file);
      return peer?.entries.some(other => other.path === entry.path);
    }));
    return entries.length ? { ...file, entries } : null;
  }).filter(Boolean);
}

async function pftSites(cases) {
  const kernelDir = $('kernel').value;
  return Promise.all(cases.map(async siteCase => {
    const key = `${kernelDir}\u001f${siteCase.dir}`;
    if (!PFT_SITE_CACHE.has(key)) {
      const pending = invoke('site_pfts', { dir: siteCase.dir, kernelDir })
        .catch(error => {
          PFT_SITE_CACHE.delete(key);
          throw error;
        });
      PFT_SITE_CACHE.set(key, pending);
    }
    const components = await PFT_SITE_CACHE.get(key);
    return { siteCase, components };
  }));
}

function invalidatePftSites(dirs, changes) {
  if (!changes.some(change => PFT_IDENTITY_FIELDS.has(change.path))) return;
  for (const key of PFT_SITE_CACHE.keys()) {
    if (dirs.some(dir => key.endsWith(`\u001f${dir}`))) PFT_SITE_CACHE.delete(key);
  }
}

function pftParameterControl(parameter) {
  if (parameter.allowed_values?.length) {
    const select = document.createElement('select');
    select.className = 'select';
    for (const value of parameter.allowed_values) {
      const option = document.createElement('option');
      option.value = value;
      option.textContent = parameter.name === 'DEF_PFT_C3C4'
        ? (value === '1' ? 'C3' : 'C4') : value;
      select.appendChild(option);
    }
    select.value = parameter.value ?? parameter.default;
    return select;
  }
  const input = document.createElement('input');
  input.className = 'input';
  input.type = 'number';
  input.step = parameter.kind === 'integer' ? '1' : 'any';
  input.value = fortranNumberInputValue(parameter.value ?? parameter.default)
    ?? (parameter.value ?? parameter.default);
  return input;
}

function appendCatalogDetails(cell, rawKey, runtime = null) {
  const descriptor = currentCatalogRows().find(item => item.raw_key === rawKey)
    ?? state.parameterCatalog.find(item => item.raw_key === rawKey);
  if (!descriptor) return;
  const details = document.createElement('details');
  details.className = 'parameter-details';
  const summary = document.createElement('summary');
  summary.textContent = language() === 'en' ? 'Details' : '详情';
  details.appendChild(summary);
  const lines = [
    `${language() === 'en' ? 'English name' : '英文名'}：${descriptor.label_en}`,
    `${language() === 'en' ? 'CoLM key' : 'CoLM 原始键'}：${descriptor.raw_key}`,
    `${language() === 'en' ? 'Stable ID' : '稳定 ID'}：${descriptor.id}`,
    `${language() === 'en' ? 'Source' : 'Fortran 来源'}：${descriptor.source_location}`,
    `${language() === 'en' ? 'Default provider' : '默认值来源'}：${descriptor.default_provider}`,
    `${language() === 'en' ? 'Effective provenance' : '当前值来源'}：${runtime?.provenance ?? descriptor.default_provider}`,
    `${language() === 'en' ? 'Activation' : '生效条件'}：${descriptor.activation?.join(', ') || (language() === 'en' ? 'always/contextual' : '始终/按上下文')}`,
    `${language() === 'en' ? 'Calibration' : '可用于调优'}：${descriptor.calibration_eligible ? (language() === 'en' ? 'eligible; range required' : '可选；必须自行提供范围') : (language() === 'en' ? 'no' : '否')}`,
  ];
  const text = document.createElement('div');
  text.className = 'parameter-default';
  text.textContent = lines.join('\n');
  text.style.whiteSpace = 'pre-line';
  details.appendChild(text);
  cell.appendChild(details);
}

function renderPftContextMatrix(types, ids, usable) {
  const details = document.createElement('details');
  details.className = 'expert-param-file pft-context-matrix';
  const summary = document.createElement('summary');
  summary.textContent = language() === 'en' ? `PFT comparison matrix (${ids.length})` : `PFT 对比矩阵（${ids.length}）`;
  details.appendChild(summary);
  const tbl = document.createElement('table');
  tbl.className = 'parameter-table expert-parameter-table';
  const head = document.createElement('tr');
  for (const text of [language() === 'en' ? 'PFT' : 'PFT 类型', language() === 'en' ? 'Sites' : '站点', language() === 'en' ? 'Fractions' : '比例']) {
    const th = document.createElement('th');
    th.textContent = text;
    head.appendChild(th);
  }
  tbl.appendChild(head);
  for (const id of ids) {
    const item = types.get(id);
    const row = document.createElement('tr');
    row.dataset.parameterKey = 'DEF_PFT_VMAX25';
    const name = language() === 'en' ? item.name_en : item.name_zh;
    for (const text of [
      `${id} · ${name}`,
      item.sites.map(site => baseName(site.dir)).join('、'),
      item.fractions.map(value => `${(value * 100).toFixed(1)}%`).join('、'),
    ]) {
      const td = document.createElement('td');
      td.textContent = text || (usable.length ? '—' : '0');
      row.appendChild(td);
    }
    tbl.appendChild(row);
  }
  details.appendChild(tbl);
  return details;
}

function renderPftParameterGroup(group, parameters, dirs, pftType) {
  const details = document.createElement('details');
  details.className = 'expert-param-file';
  const summary = document.createElement('summary');
  summary.textContent = `${group}（${parameters.length}）`;
  details.appendChild(summary);
  const table = document.createElement('table');
  table.className = 'parameter-table expert-parameter-table';
  for (const parameter of parameters) {
    const row = document.createElement('tr');
    row.dataset.parameterKey = parameter.name;
    const key = document.createElement('td');
    const label = language() === 'en' ? parameter.label_en : parameter.label_zh;
    key.textContent = label + (parameter.unit ? `（${parameter.unit}）` : '');
    key.title = `${parameter.name}(${Number(pftType) + 1})`;
    const badges = document.createElement('div');
    badges.className = 'parameter-badges';
    for (const [text, kind] of [
      [parameter.scope_label, ''],
      [parameter.value == null ? (language() === 'en' ? 'Built-in inherited' : '继承内置值')
        : (language() === 'en' ? 'Explicit override' : '已显式修改'),
      parameter.value == null ? '' : 'modified'],
    ]) {
      const badge = document.createElement('span');
      badge.className = `parameter-badge ${kind}`.trim();
      badge.textContent = text;
      badges.appendChild(badge);
    }
    key.appendChild(badges);
    const warnings = [];
    if (parameter.mixed) warnings.push(language() === 'en'
      ? 'Selected sites have different explicit overrides; editing makes them equal.'
      : '所选站点的显式覆盖不同；修改后会统一为同一个值。');
    if (parameter.default_mixed) warnings.push(language() === 'en'
      ? 'Built-in defaults differ because the selected cases use different schemes.'
      : '所选算例的内置方案不同，因此默认值不同。');
    if (warnings.length) {
      key.textContent += ' ⚠';
      key.className = 'warn';
      key.title += `\n${warnings.join('\n')}`;
    }
    const valueCell = document.createElement('td');
    const control = pftParameterControl(parameter);
    if (parameter.value == null) control.style.opacity = '0.55';
    control.onchange = async () => {
      try {
        const result = await invoke('set_pft_parameter_batch', {
          dirs, pftType: Number(pftType), name: parameter.name,
          value: control.value, kernelDir: $('kernel').value,
        });
        await markChanged(result, dirs);
        status(result.written > 1
          ? `${parameter.name} 已写入 ${result.written} 个站点`
          : `${parameter.name} 已保存`);
        await renderFields();
      } catch (error) {
        status(error);
        control.value = fortranNumberInputValue(parameter.value ?? parameter.default)
          ?? (parameter.value ?? parameter.default);
      }
    };
    valueCell.appendChild(control);
    const reset = document.createElement('button');
    reset.type = 'button';
    reset.className = 'btn-ghost';
    reset.style.marginLeft = '8px';
    reset.textContent = language() === 'en' ? 'Use built-in' : '恢复内置值';
    reset.disabled = parameter.value == null && !parameter.mixed;
    reset.onclick = async () => {
      try {
        const result = await invoke('set_pft_parameter_batch', {
          dirs, pftType: Number(pftType), name: parameter.name,
          value: null, kernelDir: $('kernel').value,
        });
        await markChanged(result, dirs);
        status(result.written > 1
          ? `${parameter.name} 已在 ${result.written} 个站点恢复内置值`
          : `${parameter.name} 已恢复内置值`);
        await renderFields();
      } catch (error) { status(error); }
    };
    valueCell.appendChild(reset);
    const note = document.createElement('div');
    note.className = 'parameter-default';
    const currentDefault = parameter.scope_kind === 'pc-pft'
      ? (language() === 'en' ? 'Current PC default' : '当前 PC 内置值')
      : (language() === 'en' ? 'Built-in default' : '当前 PFT 内置值');
    note.textContent = `${currentDefault}：${parameter.default}`;
    valueCell.appendChild(note);
    if (parameter.normal_pft_default != null) {
      const normal = document.createElement('div');
      normal.className = 'parameter-default';
      normal.textContent = `${language() === 'en' ? 'Normal PFT default' : '普通 PFT 内置值'}：${parameter.normal_pft_default}`;
      valueCell.appendChild(normal);
    }
    const effective = document.createElement('div');
    effective.className = 'parameter-default';
    effective.textContent = `${language() === 'en' ? 'Effective' : '当前有效值'}：${parameter.effective_value}`
      + ` · ${parameter.provenance}`;
    valueCell.appendChild(effective);
    appendCatalogDetails(valueCell, parameter.name, parameter);
    row.append(key, valueCell);
    table.appendChild(row);
  }
  details.appendChild(table);
  return details;
}

async function renderPftParameterMatrix(target, types, ids, group) {
  const byType = new Map(await Promise.all(ids.map(async id => {
    const dirs = types.get(id).sites.map(item => item.dir);
    const parameters = await invoke('pft_parameter_states', {
      dirs, pftType: Number(id), kernelDir: $('kernel').value,
    });
    return [id, parameters.filter(parameter => (
      language() === 'en' ? parameter.group_en : parameter.group_zh
    ) === group)];
  })));
  const names = [...new Set([...byType.values()].flat().map(parameter => parameter.name))];
  if (!names.length) return;

  const details = document.createElement('details');
  details.className = 'expert-param-file';
  details.open = true;
  const summary = document.createElement('summary');
  summary.textContent = `${language() === 'en' ? 'Parameter comparison matrix' : '参数对比矩阵'} · ${group}（${names.length}）`;
  details.appendChild(summary);
  const table = document.createElement('table');
  table.className = 'parameter-table expert-parameter-table pft-parameter-matrix';
  const head = document.createElement('tr');
  const first = document.createElement('th');
  first.textContent = language() === 'en' ? 'Parameter' : '参数';
  head.appendChild(first);
  const selectedColumns = new Map();
  for (const id of ids) {
    const th = document.createElement('th');
    const checkbox = document.createElement('input');
    checkbox.type = 'checkbox';
    checkbox.checked = true;
    checkbox.ariaLabel = `${language() === 'en' ? 'Select PFT' : '选择 PFT'} ${id}`;
    selectedColumns.set(id, checkbox);
    th.append(checkbox, ` PFT-${id}`);
    head.appendChild(th);
  }
  const batchHead = document.createElement('th');
  batchHead.textContent = language() === 'en' ? 'Selected columns' : '选定列批量值';
  head.appendChild(batchHead);
  table.appendChild(head);

  for (const name of names) {
    const row = document.createElement('tr');
    row.dataset.parameterKey = name;
    const representative = [...byType.values()].flat().find(parameter => parameter.name === name);
    const label = document.createElement('td');
    label.textContent = language() === 'en' ? representative.label_en : representative.label_zh;
    label.title = name;
    row.appendChild(label);
    for (const id of ids) {
      const cell = document.createElement('td');
      const parameter = byType.get(id).find(item => item.name === name);
      if (!parameter) {
        cell.textContent = '—';
        cell.className = 'muted';
      } else {
        const control = pftParameterControl(parameter);
        control.title = `${parameter.scope_label} · ${parameter.provenance}`;
        if (parameter.value == null) control.style.opacity = '0.55';
        control.onchange = async () => {
          const dirs = types.get(id).sites.map(item => item.dir);
          try {
            const result = await invoke('set_pft_parameter_batch', {
              dirs, pftType: Number(id), name, value: control.value,
              kernelDir: $('kernel').value,
            });
            await markChanged(result, dirs);
            await renderFields();
          } catch (error) { status(error); }
        };
        const reset = document.createElement('button');
        reset.type = 'button';
        reset.className = 'btn-ghost';
        reset.textContent = '↺';
        reset.title = language() === 'en' ? 'Use built-in' : '恢复内置值';
        reset.disabled = parameter.value == null && !parameter.mixed;
        reset.onclick = async () => {
          const dirs = types.get(id).sites.map(item => item.dir);
          try {
            const result = await invoke('set_pft_parameter_batch', {
              dirs, pftType: Number(id), name, value: null,
              kernelDir: $('kernel').value,
            });
            await markChanged(result, dirs);
            await renderFields();
          } catch (error) { status(error); }
        };
        cell.append(control, reset);
      }
      row.appendChild(cell);
    }
    const batchCell = document.createElement('td');
    const batchInput = document.createElement('input');
    batchInput.className = 'input';
    batchInput.type = 'number';
    batchInput.step = representative.kind === 'integer' ? '1' : 'any';
    batchInput.placeholder = language() === 'en' ? 'value' : '统一值';
    const apply = document.createElement('button');
    apply.type = 'button';
    apply.className = 'btn-ghost';
    apply.textContent = language() === 'en' ? 'Apply' : '应用';
    apply.onclick = async () => {
      if (!batchInput.value.trim()) return status(`${name} 需要批量值`);
      const selected = ids.filter(id => selectedColumns.get(id).checked
        && byType.get(id).some(parameter => parameter.name === name));
      if (!selected.length) return status('没有选中的 PFT 列');
      const changes = selected.map(id => ({
        dirs: types.get(id).sites.map(item => item.dir),
        pftType: Number(id), name, value: batchInput.value,
      }));
      const dirs = [...new Set(changes.flatMap(change => change.dirs))];
      try {
        const result = await invoke('set_pft_parameters_batch', {
          changes, kernelDir: $('kernel').value,
        });
        await markChanged(result, dirs);
        status(`${name} 已原子写入 ${selected.length} 个 PFT 类型、${result.written} 个算例`);
        await renderFields();
      } catch (error) { status(error); }
    };
    batchCell.append(batchInput, apply);
    row.appendChild(batchCell);
    table.appendChild(row);
  }
  details.appendChild(table);
  target.appendChild(details);
}

async function renderPftParameters(processes, flows) {
  const eco = processes.find(([page]) => page.id === 'params-eco');
  if (!eco) return;
  const [, target] = eco;
  const allCases = expertCases();
  if (!allCases.length) return;
  const selectedCases = state.expertCaseDir === EXPERT_ALL && allCases.length > 1
    ? allCases : [expertCase()].filter(Boolean);
  let usable;
  try {
    usable = await pftSites(selectedCases);
  } catch (error) {
    status(error);
    return;
  }

  const types = new Map();
  for (const { siteCase, components } of usable) {
    for (const component of components) {
      const current = types.get(component.pft_type) ?? {
        ...component, sites: [], fractions: [],
      };
      current.sites.push(siteCase);
      current.fractions.push(component.fraction);
      types.set(component.pft_type, current);
    }
  }
  if (!types.size) return;
  const ids = [...types.keys()].filter(id => id !== 0).sort((a, b) => a - b);
  if (!ids.length) return;
  state.parameterPftContexts = ids.map(id => ({
    ...types.get(id),
    pft_type: id,
  }));
  if (!ids.includes(Number(state.expertPftType))) state.expertPftType = ids[0];
  const selected = types.get(Number(state.expertPftType));
  const dirs = selected.sites.map(item => item.dir);
  let parameters;
  try {
    parameters = await invoke('pft_parameter_states', {
      dirs, pftType: Number(state.expertPftType), kernelDir: $('kernel').value,
    });
  } catch (error) {
    status(error);
    return;
  }
  if (!parameters.length) return;
  state.parameterPftStates = new Map(parameters.map(item => [item.name, item]));
  const pcMode = parameters[0].scope_kind === 'pc-pft';

  target.querySelector('.empty-params')?.remove();
  const wrap = document.createElement('div');
  wrap.className = 'expert-note pft-expert-editor';
  const picker = document.createElement('label');
  picker.textContent = pcMode
    ? (language() === 'en' ? 'PFT component in current PC' : '当前 PC 的 PFT 组分')
    : (language() === 'en' ? 'Plant functional type' : '植被功能型');
  const select = document.createElement('select');
  select.className = 'select';
  for (const id of ids) {
    const item = types.get(id);
    const option = document.createElement('option');
    option.value = id;
    const name = language() === 'en' ? item.name_en : item.name_zh;
    const suffix = usable.length === 1
      ? ` · ${(item.fractions[0] * 100).toFixed(1)}%`
      : ` · ${item.sites.length}/${usable.length} ${language() === 'en' ? 'sites' : '个站点'}`;
    option.textContent = `${id} · ${name}${suffix}`;
    select.appendChild(option);
  }
  select.value = String(state.expertPftType);
  select.onchange = async () => {
    state.expertPftType = Number(select.value);
    await renderFields();
  };
  picker.appendChild(select);
  wrap.appendChild(picker);
  const viewPicker = document.createElement('label');
  viewPicker.textContent = language() === 'en' ? 'View' : '视图';
  const view = document.createElement('select');
  view.className = 'select';
  for (const [value, label] of [
    ['single', language() === 'en' ? 'Single type' : '单类型'],
    ['matrix', language() === 'en' ? 'Comparison matrix' : '对比矩阵'],
  ]) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = label;
    view.appendChild(option);
  }
  view.value = state.parameterPftView;
  view.onchange = async () => {
    state.parameterPftView = view.value;
    await renderFields();
  };
  viewPicker.appendChild(view);
  wrap.appendChild(viewPicker);
  const explanation = document.createElement('div');
  explanation.className = 'parameter-default';
  const composition = ids.map(id => {
    const item = types.get(id);
    const fraction = usable.length === 1 ? ` ${(item.fractions[0] * 100).toFixed(1)}%` : '';
    return `${id} ${language() === 'en' ? item.name_en : item.name_zh}${fraction}`;
  }).join('；');
  if (language() === 'en') {
    explanation.textContent = `${pcMode ? `PC components: ${composition}. ` : ''}Sparse overrides are written only for PFT ${state.expertPftType}; sites without this PFT are left unchanged.`;
  } else {
    explanation.textContent = (pcMode ? `当前为 PC 模式，组分为：${composition}。` : '')
      + `仅为 PFT ${state.expertPftType} 写入稀疏覆盖；不含该 PFT 的站点不会被修改。`;
  }
  wrap.appendChild(explanation);
  target.appendChild(wrap);
  target.appendChild(renderPftContextMatrix(types, ids, usable));

  const groups = new Map();
  for (const parameter of parameters) {
    const group = language() === 'en' ? parameter.group_en : parameter.group_zh;
    if (!groups.has(group)) groups.set(group, []);
    groups.get(group).push(parameter);
  }
  if (!groups.has(state.parameterPftGroup)) state.parameterPftGroup = groups.keys().next().value;
  if (state.parameterPftView === 'matrix') {
    const groupPicker = document.createElement('label');
    groupPicker.textContent = language() === 'en' ? 'Loaded subgroup' : '当前加载子组';
    const groupSelect = document.createElement('select');
    groupSelect.className = 'select';
    for (const group of groups.keys()) {
      const option = document.createElement('option');
      option.value = group;
      option.textContent = group;
      groupSelect.appendChild(option);
    }
    groupSelect.value = state.parameterPftGroup;
    groupSelect.onchange = async () => {
      state.parameterPftGroup = groupSelect.value;
      await renderFields();
    };
    groupPicker.appendChild(groupSelect);
    wrap.appendChild(groupPicker);
    await renderPftParameterMatrix(target, types, ids, state.parameterPftGroup);
    flows.add('params-eco');
    return;
  }
  for (const [group, items] of groups) {
    target.appendChild(renderPftParameterGroup(group, items, dirs, state.expertPftType));
  }
  flows.add('params-eco');
}

function publishFlows(flows) {
  state.availableFlows = flows;
  globalThis.dispatchEvent?.(new Event('colm:flows'));
}

/** 一组字段渲染成一张表。分节之后每节各调一次。 */
function table(
  shown, fieldStates = new Map(), dirs = editTarget(), showVaries = true, syncText = true,
  showDefaults = false, forceDisabled = false,
) {
  const tbl = document.createElement('table');
  tbl.className = 'parameter-table';
  for (const e of shown) {
    const tr = document.createElement('tr');
    tr.dataset.parameterKey = e.path;
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
        k.textContent = fieldLabel(e.path, language()) + ' ⓘ';
        k.style.cursor = 'help';
      }
      if (meta) {
        const details = [technicalFieldHint(e.path, language())];
        if (hint) details.push(hint);
        if (meta.doc) details.push(meta.doc);
        const defaultText = defaultValueText(
          e.path, fieldState?.context_default ?? meta.default, meta.kind,
        );
        if (defaultText != null) {
          details.push(`${language() === 'en' ? 'Default' : '默认'}：${defaultText}`);
        }
        k.title = details.join('\n\n');
      }
      if (fieldState?.reason) {
        k.title = (k.title ? k.title + '\n\n' : '') + fieldState.reason;
      }
      if (e.path === 'DEF_USE_CBL_HEIGHT' && expertCases().length > 1) {
        const site = dirs.length === 1 ? baseName(dirs[0]) : null;
        k.textContent += language() === 'en'
          ? (site ? ` (file for ${site})` : ' (select one site)')
          : (site ? `（当前站点：${site}）` : '（请选择单个站点）');
      }
    }
    const warnings = [];
    if (fieldState?.mixed) {
      warnings.push('这一批算例对该字段的适用条件不同；它只对其中一部分算例生效。');
    }
    if (fieldState?.default_mixed) {
      warnings.push('所选站点的内置默认值随地类不同；批量值不显示代表值。请选择单站或相同地类范围后修改。');
    }
    if (showVaries
        && (state.varies.has(e.path) || e.sourcePaths?.some(path => state.varies.has(path)))) {
      // 这一行显示的是代表算例的值，别的算例不是这个值。改它会抹平全部。
      warnings.push('这一批算例在这个字段上取值不同，显示的是第一个的值。改它会把全部改成同一个值。');
    }
    if (warnings.length) {
      k.textContent += ' ⚠';
      k.className = 'warn';
      k.title = (k.title ? k.title + '\n\n' : '') + warnings.join('\n');
    }
    if (fieldState?.scope_label || fieldState?.override_value != null) {
      const badges = document.createElement('div');
      badges.className = 'parameter-badges';
      for (const [text, kind] of [
        [fieldState.scope_label, ''],
        [fieldState.override_value != null
          ? (language() === 'en' ? 'Explicit override' : '已显式修改')
          : (language() === 'en' ? 'Built-in inherited' : '继承内置值'),
        fieldState.override_value != null ? 'modified' : ''],
      ]) {
        if (!text) continue;
        const badge = document.createElement('span');
        badge.className = `parameter-badge ${kind}`.trim();
        badge.textContent = text;
        badges.appendChild(badge);
      }
      k.appendChild(badges);
    }
    const v = document.createElement('td');
    if (e.derived) {
      // 有声明有默认值，但不在任何 namelist 组里 —— 用户设了也没用。
      // 给一个改了没用的输入框比只读地显示更糟。
      v.textContent = optionLabel(e.path, e.value, language()) + '（派生值，改不了）';
      v.className = 'muted';
    } else {
      const inp = control(e, meta, fieldState);
      if (forceDisabled) inp.disabled = true;
      if (fieldState?.mode === 'disabled') inp.disabled = true;
      // 一批算例对字段的适用条件不同，就不能拿第一个算例的控件覆盖整批。
      // 需要修改时先让配置一致；后端也会做同一条最终防线。
      if (fieldState?.mixed) inp.disabled = true;
      if (fieldState?.default_mixed || fieldState?.effective_mixed) {
        inp.disabled = true;
        if (inp.tagName === 'SELECT') inp.selectedIndex = -1;
        else {
          inp.value = '';
          inp.placeholder = language() === 'en' ? 'mixed values' : '混合值';
        }
      }
      // 边界层高度是逐站点数据；同一份文件不能安全地套到整批站点。
      if (e.path === 'DEF_USE_CBL_HEIGHT' && dirs.length > 1) inp.disabled = true;
      // 路径必须经原生选择器写入，避免手填一个不存在或类型不对的路径。
      if (PATH_FIELDS[e.path]) inp.readOnly = true;
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
              dirs,
              fields,
              kernelDir: $('kernel').value,
            });
            if (syncText) state.text = r.text;
            await markChanged(r, dirs);
            state.varies.delete('DEF_USE_MEDLYNST');
            state.varies.delete('DEF_USE_WUEST');
            status(r.written > 1
              ? `已为 ${r.written} 个算例设置气孔导度方案`
              : '已保存气孔导度方案');
            await renderFields();
            return;
          }
          if (enabled(inp.value) && e.path === 'DEF_USE_CBL_HEIGHT') {
            if (dirs.length !== 1) throw new Error('请先在“修改站点”中选择一个站点');
            const picked = await pickParameterPath('DEF_USE_CBL_HEIGHT', 'file');
            if (!picked) { inp.value = before; return; }
            const r = await invoke('configure_cbl_batch', {
              dirs, file: picked, kernelDir: $('kernel').value,
            });
            if (syncText) state.text = r.text;
            await markChanged(r, dirs);
            status(`已为 ${baseName(dirs[0])} 校验并接入边界层高度文件`);
            await renderFields();
            return;
          }
          if (enabled(inp.value)
              && (e.path === 'DEF_USE_OZONESTRESS' || e.path === 'DEF_USE_OZONEDATA')) {
            const picked = await pickParameterPath('DEF_file_Ozone', 'file');
            if (!picked) { inp.value = before; return; }
            const r = await invoke('configure_ozone_batch', {
              dirs, file: picked, kernelDir: $('kernel').value,
            });
            if (syncText) state.text = r.text;
            await markChanged(r, dirs);
            status('已校验臭氧数据，并启用臭氧胁迫与数据读取');
            await renderFields();
            return;
          }
          if (e.path === 'DEF_file_Ozone') {
            const r = await invoke('configure_ozone_batch', {
              dirs, file: inp.value, kernelDir: $('kernel').value,
            });
            if (syncText) state.text = r.text;
            await markChanged(r, dirs);
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
            ? await invoke('set_fields_batch', {
              dirs, fields: changes, kernelDir: $('kernel').value,
            })
            : await invoke('set_field_batch',
              { dirs, path: e.path, value: inp.value, kernelDir: $('kernel').value });
          if (syncText) state.text = r.text;
          await markChanged(r, dirs);
          invalidatePftSites(dirs, changes);
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
          const raw = e.value.replace(/^'|'$/g, '');
          inp.value = meta?.kind?.startsWith('Real')
            ? (fortranNumberInputValue(raw) ?? '') : raw;
        }
      };
      v.appendChild(inp);
      if (PATH_FIELDS[e.path]) {
        const pick = document.createElement('button');
        pick.type = 'button';
        pick.className = 'btn-ghost';
        pick.disabled = inp.disabled;
        pick.style.marginLeft = '8px';
        pick.textContent = PATH_FIELDS[e.path] === 'file' ? '选择文件…' : '选择目录…';
        pick.onclick = async () => {
          const chosen = await pickParameterPath(e.path, PATH_FIELDS[e.path]);
          if (!chosen) return;
          if (e.path === 'DEF_file_Ozone') {
            try {
              const r = await invoke('configure_ozone_batch', {
                dirs, file: chosen, kernelDir: $('kernel').value,
              });
              if (syncText) state.text = r.text;
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
      if (!e.synthetic && (fieldState?.override_value != null || fieldState?.override_mixed)) {
        const reset = document.createElement('button');
        reset.type = 'button';
        reset.className = 'btn-ghost';
        reset.style.marginLeft = '8px';
        reset.textContent = language() === 'en' ? 'Use built-in' : '恢复内置值';
        reset.disabled = forceDisabled || fieldState?.mixed;
        reset.onclick = async () => {
          try {
            const r = await invoke('reset_field_batch', {
              dirs, path: e.path, kernelDir: $('kernel').value,
            });
            if (syncText) state.text = r.text;
            await markChanged(r, dirs);
            status(r.changed ? `${e.path} 已删除显式覆盖` : `${e.path} 已继承内置值`);
            await renderFields();
          } catch (err) { status(err); }
        };
        v.appendChild(reset);
      }
      if (showDefaults || fieldState?.built_in_default != null) appendDefaultValue(
        v, e.path, fieldState?.built_in_default ?? fieldState?.context_default ?? meta?.default,
        meta?.kind,
      );
      if (fieldState?.effective_value != null) {
        const note = document.createElement('div');
        note.className = 'parameter-default';
        note.textContent = `${language() === 'en' ? 'Effective' : '当前有效值'}：${fieldState.effective_value}`
          + ` · ${fieldState.provenance}`;
        v.appendChild(note);
      }
      appendCatalogDetails(v, e.path, fieldState);
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
globalThis.addEventListener?.('colm:mode', () => {
  if (state.text) renderFields();
});
