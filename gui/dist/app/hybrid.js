//! AI 混合建模（docs/design-hybrid.md）：研究里的“AI 学习参数”与“AI 替换过程”两个入口（同一个页面的两种模式），
//! 运行页的算例卡片、导入表单，以及参数率定里“同时训练网络”的表单。
//!
//! 只依赖 ipc/state/ui/batch/shell/engine/i18n：runner.js 与 results.js 都导入它，反过来导入会成环。
//! 算例上的配置由 `colm-cli hybrid-info` 读出（JSON），这里不解析 TOML。

import { invoke, hasBackend } from './ipc.js';
import { $, appConfirm, status, baseName } from './ui.js';
import { state } from './state.js';
import { batchTarget } from './batch.js';
import { go } from './shell.js';
import { modelEngine } from './engine.js';
import { language, translateZh } from './i18n.js';

const t = text => (language() === 'en' ? translateZh(text) : text);

// ---- 纯函数（tests/hybrid.mjs）----------------------------------------------------------

/** 地表模式决定插槽：LCT 按地类（`land_class`），PFT/PC 按植物功能型（`pft`）。 */
export function slotForMode(mode) {
  return mode === 'lct' ? 'land_class' : 'pft';
}

/** 过程插槽：网络替换 `eroot` 的土壤水分胁迫 β（只有一个输出 `beta`）。 */
export const SOIL_STRESS = 'soil_stress';

/** β 插槽每步现算的特征（colm-core 给出；其余特征与参数插槽同一套取法）。 */
export const STRESS_FEATURES = 'beta_physics, root_saturation, root_temperature, frozen_root_fraction';

/** 两种模式各用哪些插槽：学习参数是参数插槽（按 PFT 或按地类），替换过程是过程插槽。 */
export const MODE_SLOTS = { params: ['pft', 'land_class'], process: [SOIL_STRESS] };

/** 研究入口决定模式：`hybrid-process` 是替换过程，其余（`hybrid-learn`）是学习参数。 */
export function modeForStep(step) {
  return step === 'hybrid-process' ? 'process' : 'params';
}

/** 这个模式能用的预设（“自定义”两种模式都有）。 */
export function presetsForMode(mode) {
  const slots = MODE_SLOTS[mode] ?? MODE_SLOTS.params;
  return PRESETS.filter(preset => !preset.slot || slots.includes(preset.slot));
}

/** 这个模式能用的训练方法：两步法拟合“特征 → 率定出的参数”，只属于学习参数；可微训练（计划中）是给 β 的。 */
export function methodsForMode(mode) {
  return mode === 'process' ? ['de', 'gradient'] : ['de', 'two-step'];
}

/** 插槽能驱动的参数前缀；β 插槽的输出就叫 `beta`。 */
export function outputPrefix(slot) {
  if (slot === SOIL_STRESS) return 'beta';
  return slot === 'land_class' ? 'DEF_LC_' : 'DEF_PFT_';
}

/** 新插槽的默认特征与输出（Vcmax25，常见取值 10–150 μmol m⁻² s⁻¹）。 */
export function slotDefaults(slot) {
  if (slot === SOIL_STRESS) {
    return { features: STRESS_FEATURES, output: { name: 'beta', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true } };
  }
  return slot === 'land_class'
    ? { features: 'patchclass', output: { name: 'DEF_LC_VMAX25', lo: 10, hi: 150, transform: 'sigmoid' } }
    : { features: 'pftclass, pftfrac', output: { name: 'DEF_PFT_VMAX25', lo: 10, hi: 150, transform: 'sigmoid' } };
}

/** 逗号分隔的特征名。 */
export function parseFeatures(text) {
  return String(text ?? '').split(/[,，\s]+/).map(name => name.trim()).filter(Boolean);
}

/** 校验一行输出，返回 `{name, lo, hi, transform}`；不合法时抛出说明。 */
export function checkOutput(slot, { name, lo, hi, transform, relative = false }) {
  const prefix = outputPrefix(slot);
  const stress = slot === SOIL_STRESS;
  const field = stress ? String(name ?? '').trim().toLowerCase() : String(name ?? '').trim().toUpperCase();
  if (stress && field !== 'beta') throw new Error('土壤水分胁迫插槽只有一个输出：beta');
  if (!stress && (!field.startsWith(prefix) || field.length === prefix.length)) {
    throw new Error(`输出参数必须以 ${prefix} 开头`);
  }
  // 空格子不是 0：`Number('')` 会把没填的下限当成 0。
  const number = value => (String(value ?? '').trim() === '' ? NaN : Number(value));
  const low = number(lo);
  const high = number(hi);
  if (!Number.isFinite(low) || !Number.isFinite(high) || low >= high) {
    throw new Error(`${field} 的范围必须是有限数且下限小于上限`);
  }
  return { name: field, lo: low, hi: high, transform: transform || 'sigmoid', relative: !!relative };
}

/** `colm-cli hybrid-install --output` 的写法。 */
export function outputArg({ name, lo, hi, transform, relative = false }) {
  return `${name}:${lo}:${hi}:${transform}${relative ? ':relative' : ''}`;
}

/** 网络规模选项 → 隐藏层宽度。 */
export const NETWORK_SIZES = { linear: [], h4: [4], h8: [8] };

/** 权重个数：逐层 输入×输出 + 偏置（与 `colm-hybrid` 的 `Mlp::parameter_count` 一致）。 */
export function weightCount(features, hidden, outputs) {
  const widths = [features, ...hidden, outputs];
  let count = 0;
  for (let i = 1; i < widths.length; i++) count += widths[i - 1] * widths[i] + widths[i];
  return count;
}

/** Study spec 的 `hybrid` 段。 */
export function studySection({ slot, features, outputs, size }) {
  const names = parseFeatures(features);
  if (!names.length) throw new Error('AI 模型至少需要一个输入特征');
  if (!outputs.length) throw new Error('AI 模型至少需要一个输出参数');
  const checked = outputs.map(output => checkOutput(slot, output));
  if (new Set(checked.map(o => o.name)).size !== checked.length) throw new Error('AI 模型的输出参数不能重复');
  for (const output of checked) {
    if (!['sigmoid', 'clamp'].includes(output.transform)) throw new Error('训练时输出只能用 S 形映射或截断');
  }
  return {
    slot,
    features: names,
    outputs: checked.map(o => ({
      name: o.name, range: [o.lo, o.hi], transform: o.transform, ...(o.relative ? { relative: true } : {}),
    })),
    hidden: NETWORK_SIZES[size] ?? [],
  };
}

// ---- 训练：预设、两步法 ----------------------------------------------------------------

/** “要学什么”的预设：点一下就把作用方式、特征、输出和网络规模填好。 */
export const PRESETS = [
  {
    id: 'vcmax-pft', title: 'Vcmax25 随气候变化', note: 'PFT/PC 模式 · 按植物功能型 · 乘数 0.5–2',
    slot: 'pft', features: 'clim_tair, clim_vpd, clim_prec', size: 'linear',
    outputs: [{ name: 'DEF_PFT_VMAX25', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true }],
  },
  {
    id: 'vcmax-lc', title: '地类 Vcmax25 随气候变化', note: 'LCT 模式 · 按地类 · 乘数 0.5–2',
    slot: 'land_class', features: 'clim_tair, clim_vpd, clim_prec', size: 'linear',
    outputs: [{ name: 'DEF_LC_VMAX25', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true }],
  },
  {
    id: 'beta', title: '土壤水分胁迫 β', note: '过程槽位 · LCT/PFT · 需关闭植物水力 · 乘数 0.5–2',
    slot: SOIL_STRESS, features: STRESS_FEATURES, size: 'linear',
    outputs: [{ name: 'beta', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true }],
  },
  { id: 'custom', title: '自定义', note: '自己选作用方式、特征与输出' },
];

/** 预设在这个算例上能不能用；不能用时返回原因。`info` 是 `colm-cli hybrid-info` 的结果。 */
export function presetBlocked(preset, info) {
  if (preset.slot !== SOIL_STRESS || !info) return '';
  if (info.land_mode === 'pc') return 'PC 模式不支持：PC 冠层的水分胁迫来自植物水力';
  if (info.plant_hydraulics) return '要先关闭植物水力（DEF_USE_PLANTHYDRAULICS）';
  return '';
}

/** 地表模式对应的默认预设。 */
export function presetForMode(mode) {
  return mode === 'lct' ? 'vcmax-lc' : 'vcmax-pft';
}

/** 率定任务能不能用作两步法的第一步：已完成、本身没训练网络、率定了网络的全部输出。 */
export function studyUsable(study, outputNames) {
  if (!['completed', 'completed_with_failures'].includes(study.status)) return { usable: false, reason: '还没跑完' };
  if (study.trains_network) return { usable: false, reason: '这个任务本身在训练网络' };
  const missing = outputNames.filter(name => !study.parameters.includes(name));
  if (missing.length) return { usable: false, reason: `没有率定 ${missing.join('、')}` };
  if (!study.best_member) return { usable: false, reason: '没有最优成员' };
  return { usable: true, reason: '' };
}

/** 两步法报告的摘要：每个留出任务一行（网络与均值基准的误差），以及合起来的门槛结论。 */
export function fitSummary(report) {
  const mean = values => (Array.isArray(values) && values.length ? values.reduce((a, b) => a + b, 0) / values.length : null);
  const rows = (report.held_out_rmse ?? []).map(entry => {
    const network = mean(entry.rmse);
    const baseline = mean(entry.mean_predictor_rmse);
    return { study: entry.study, network, baseline, better: network != null && baseline != null && network < baseline };
  });
  const validation = report.validation;
  return {
    rows,
    samples: report.rows,
    network: mean(validation?.network_rmse),
    baseline: mean(validation?.mean_predictor_rmse),
    // 只有一个任务时没有交叉验证（null）：能装，但要提醒没有经过检验。
    passed: validation ? validation.passed === true : null,
  };
}

// ---- 共用：输出参数行 -----------------------------------------------------------------

const TRANSFORMS = [
  ['sigmoid', 'S 形映射到范围（推荐）'],
  ['clamp', '截断到范围'],
  ['identity', '原样（超出范围报错）'],
];

function element(tag, cls = '', text = '') {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text !== '') el.textContent = String(text);
  return el;
}

function input(cls, value, placeholder = '', type = 'text') {
  const el = element('input', cls);
  el.type = type;
  el.value = value;
  if (placeholder) el.placeholder = placeholder;
  return el;
}

/** 一行输出：参数名、下限、上限、变换、删除。 */
function outputRow(host, output, transforms, onChange) {
  const row = element('div', 'hybrid-output-row');
  const name = input('input hybrid-output-name', output.name, 'DEF_PFT_VMAX25');
  const lo = input('input mini-input', output.lo, '下限', 'number');
  const hi = input('input mini-input', output.hi, '上限', 'number');
  const transform = element('select', 'select');
  for (const [value, label] of TRANSFORMS.filter(([value]) => transforms.includes(value))) {
    const option = element('option', '', label);
    option.value = value;
    transform.appendChild(option);
  }
  transform.value = output.transform;
  const relativeLabel = element('label', 'check mini');
  const relative = element('input');
  relative.type = 'checkbox';
  relative.checked = !!output.relative;
  relativeLabel.title = '勾选后网络给出乘数，参数取“查表值 × 乘数”，范围是乘数的范围（如 0.5–2）';
  relativeLabel.append(relative, element('span', '', '乘数'));
  const remove = element('button', 'btn-ghost', '删除');
  remove.type = 'button';
  remove.onclick = () => { row.remove(); onChange?.(); };
  for (const el of [name, lo, hi, transform]) el.addEventListener('input', () => onChange?.());
  transform.addEventListener('change', () => onChange?.());
  relative.addEventListener('change', () => onChange?.());
  row.append(name, lo, element('span', 'muted', '–'), hi, transform, relativeLabel, remove);
  row.read = () => ({
    name: name.value, lo: lo.value, hi: hi.value, transform: transform.value, relative: relative.checked,
  });
  host.appendChild(row);
}

function readOutputs(host) {
  return [...host.children].filter(row => typeof row.read === 'function').map(row => row.read());
}

// ---- 运行页卡片 ---------------------------------------------------------------------

const infoCache = new Map();
let refreshToken = 0;

async function caseInfo(dir) {
  const info = JSON.parse(await invoke('hybrid_info', { case: dir }));
  infoCache.set(dir, info);
  return info;
}

const MODE_LABELS = { lct: 'LCT（地类）', pft: 'PFT', pc: 'PC' };

function statusCell(info) {
  if (!info) return element('td', 'muted', '读取失败');
  if (!info.installed) return element('td', 'muted', '未安装');
  if (info.error) {
    const cell = element('td', 'warn', '模型校验失败');
    cell.title = info.error;
    return cell;
  }
  const cell = element('td', '');
  cell.textContent = info.slots.some(slot => slot.trained_by_study) ? '已安装（参数率定训练）' : '已安装';
  return cell;
}

function renderCaseTable(cases, infos) {
  const table = element('table');
  const head = element('tr');
  for (const label of ['算例', '地表模式', 'AI 模型', '作用参数', '模型文件', '气候特征', '']) head.appendChild(element('th', '', label));
  table.appendChild(head);
  cases.forEach((c, index) => {
    const info = infos[index];
    const row = element('tr');
    const slots = info?.slots ?? [];
    const actions = element('td');
    const climate = element('button', 'btn-ghost', info?.climate ? '重算气候特征' : '计算气候特征');
    climate.type = 'button';
    climate.onclick = () => computeClimate(c, climate).catch(e => status(e));
    actions.appendChild(climate);
    if (info?.installed) {
      const check = element('button', 'btn-ghost', '检查');
      check.type = 'button';
      check.onclick = () => checkCase(c).catch(e => status(e));
      const remove = element('button', 'btn-ghost', '移除');
      remove.type = 'button';
      remove.onclick = () => removeCase(c).catch(e => status(e));
      actions.append(check, remove);
    }
    row.append(
      element('td', '', c.name || baseName(c.dir)),
      element('td', '', MODE_LABELS[info?.land_mode] ?? '—'),
      statusCell(info),
      element('td', '', slots.flatMap(slot => slot.outputs.map(o => o.name)).join(', ') || '—'),
      element('td', '', slots.map(slot => slot.model).filter(Boolean).join(', ') || '—'),
      climateCell(info),
      actions,
    );
    table.appendChild(row);
  });
  return table;
}

function climateCell(info) {
  if (info?.climate) return element('td', '', '已计算');
  // 模型用到 clim_* 却还没算：运行会失败，标成警告。
  return element('td', info?.uses_climate ? 'warn' : 'muted', info?.uses_climate ? '未计算（模型需要）' : '未计算');
}

async function computeClimate(c, button) {
  const kernelDir = $('kernel')?.value;
  if (!kernelDir) throw new Error('先在基本设定里选好内核。');
  button.disabled = true;
  status('正在计算气候特征…');
  try {
    await invoke('hybrid_climate', { dirs: [c.dir], kernelDir });
    status('已计算气候特征');
  } finally {
    button.disabled = false;
  }
  await refreshHybridCard();
}

/** 有模型的算例碰上 Fortran 内核：提前在卡片里说，运行前再拦一次（见 `fortranBlockedCases`）。 */
function syncEngineWarning() {
  const warning = $('hybrid-engine-warning');
  if (!warning) return;
  const dirs = batchTarget().map(c => c.dir);
  warning.hidden = !(modelEngine() === 'fortran' && dirs.some(dir => infoCache.get(dir)?.installed));
}

/** 重新读本次算例的配置并重画卡片。 */
export async function refreshHybridCard() {
  const host = $('hybrid-cases');
  if (!host || !hasBackend) return;
  const token = ++refreshToken;
  if (!$('hybrid-outputs').children.length) setImportSlot($('hybrid-slot').value);
  const cases = batchTarget();
  if (!cases.length) {
    host.replaceChildren(element('div', 'result-empty', '还没有算例；先在基本设定中创建或打开算例。'));
    syncEngineWarning();
    return;
  }
  const infos = await Promise.all(cases.map(c => caseInfo(c.dir).catch(() => null)));
  if (token !== refreshToken) return;
  host.replaceChildren(renderCaseTable(cases, infos));
  syncEngineWarning();
  syncImportSlot(infos);
}

/** Fortran 内核运行前：返回装了模型的算例目录（空数组表示可以运行）。 */
export async function fortranBlockedCases(dirs) {
  if (modelEngine() !== 'fortran' || !hasBackend) return [];
  const infos = await Promise.all(dirs.map(dir => caseInfo(dir).catch(() => null)));
  return dirs.filter((_, index) => infos[index]?.installed);
}

function statRow(kind, column) {
  const row = element('tr');
  const fmt = value => (Number.isFinite(value) ? Number(value.toPrecision(4)).toString() : '—');
  row.append(element('td', '', kind), element('td', '', column.name), element('td', '', fmt(column.min)),
    element('td', '', fmt(column.max)), element('td', '', fmt(column.mean)), element('td', '', fmt(column.std)));
  return row;
}

async function checkCase(c) {
  const host = $('hybrid-check-result');
  const kernelDir = $('kernel')?.value;
  if (!kernelDir) throw new Error('先在基本设定里选好内核，检查要用它读取算例。');
  host.replaceChildren(element('p', 'muted mini', '正在检查…'));
  let summary;
  try {
    summary = JSON.parse(await invoke('hybrid_check', { case: c.dir, kernelDir }));
  } catch (error) {
    host.replaceChildren(element('p', 'warn mini', String(error?.message || error)));
    return;
  }
  const title = element('div', 'hybrid-check-title');
  title.append(element('h4', '', '检查结果'), element('span', 'muted mini', c.name || baseName(c.dir)));
  const blocks = [title,
    element('p', 'muted mini', '模型按正式运行的方式取特征并推理，但不模拟。“行”是模型作用到的单元：LCT 是土壤 patch，PFT/PC 是其中的每个植物功能型。输出范围明显不合理，或特征超出训练时见过的范围时，结果不可信。')];
  for (const slot of summary) {
    const table = element('table');
    const head = element('tr');
    for (const label of ['类型', '名称', '最小', '最大', '均值', '标准差']) head.appendChild(element('th', '', label));
    table.appendChild(head);
    for (const column of slot.features) table.appendChild(statRow('输入特征', column));
    for (const column of slot.outputs) table.appendChild(statRow('输出参数', column));
    const rows = element('p', 'mini');
    rows.append(element('span', '', '作用行数：'), element('span', '', String(slot.rows)));
    let outside;
    if (slot.outside_training == null) {
      outside = element('p', 'muted mini', '模型没有记录训练范围，无法判断是否外推。');
    } else {
      outside = element('p', slot.outside_training > 0 ? 'warn mini' : 'mini');
      outside.append(element('span', '', '超出训练范围的行：'), element('span', '', String(slot.outside_training)));
    }
    const wrap = element('div', 'result-table-wrap');
    wrap.appendChild(table);
    blocks.push(rows, outside, wrap);
  }
  host.replaceChildren(...blocks);
}

async function removeCase(c) {
  const question = '移除这个算例的 AI 模型？算例回到纯物理参数；之后的运行会按新设定重跑。';
  if (!(await appConfirm(language() === 'en' ? translateZh(question) : question))) return;
  await invoke('hybrid_remove', { dirs: [c.dir] });
  status('已移除 AI 模型');
  $('hybrid-check-result')?.replaceChildren();
  await refreshHybridCard();
}

// ---- 导入表单 ------------------------------------------------------------------------

let importSlotChosen = false;

function syncImportSlot(infos) {
  const select = $('hybrid-slot');
  if (!select || importSlotChosen || hybridMode() !== 'params') return;
  const modes = new Set(infos.map(info => info?.land_mode).filter(Boolean));
  if (modes.size === 1) setImportSlot(slotForMode([...modes][0]));
}

function setImportSlot(slot, force = false) {
  const select = $('hybrid-slot');
  if (!select || !force && select.value === slot && $('hybrid-outputs')?.children.length) return;
  select.value = slot;
  const defaults = slotDefaults(slot);
  $('hybrid-features').value = defaults.features;
  const host = $('hybrid-outputs');
  host.replaceChildren();
  outputRow(host, defaults.output, ['sigmoid', 'clamp', 'identity']);
}

async function pickInto(id, key, filter) {
  const path = await invoke('pick_file', { key, filter });
  if (path) $(id).value = path;
}

async function installModel() {
  const dirs = batchTarget().map(c => c.dir);
  if (!dirs.length) throw new Error('还没有算例；先在基本设定中创建或打开算例。');
  const model = $('hybrid-model-path').value.trim();
  if (!model) throw new Error('先选择模型文件（.onnx 或 .mlp.json）。');
  const slot = $('hybrid-slot').value;
  const features = parseFeatures($('hybrid-features').value);
  if (!features.length) throw new Error('至少要一个输入特征');
  const outputs = readOutputs($('hybrid-outputs')).map(output => outputArg(checkOutput(slot, output)));
  if (!outputs.length) throw new Error('至少要一个输出参数');
  const normalize = $('hybrid-normalize-path').value.trim() || null;
  await invoke('hybrid_install', {
    dirs, model, slot, features, outputs, normalize,
    force: $('hybrid-force').checked, outsidePhysics: $('hybrid-outside').checked,
  });
  status(dirs.length === 1 ? '已导入模型' : `已为 ${dirs.length} 个算例导入模型`);
  await refreshHybridCard();
}

function wireRunCard() {
  if (!$('hybrid-card')) return;
  $('hybrid-slot').onchange = () => { importSlotChosen = true; setImportSlot($('hybrid-slot').value, true); };
  $('hybrid-add-output').onclick = () => {
    outputRow($('hybrid-outputs'), { name: outputPrefix($('hybrid-slot').value), lo: '', hi: '', transform: 'sigmoid' }, ['sigmoid', 'clamp', 'identity']);
  };
  $('hybrid-pick-model').onclick = () => pickInto('hybrid-model-path', 'hybrid-model', 'onnx,json').catch(e => status(e));
  $('hybrid-pick-normalize').onclick = () => pickInto('hybrid-normalize-path', 'hybrid-normalize', 'json').catch(e => status(e));
  $('hybrid-install').onclick = () => installModel().catch(e => status(e?.message || e));
  // 训练在参数率定里：跳过去并打开“同时训练 AI 模型”。
  $('hybrid-go-tuning').onclick = () => {
    go('result-tuning');
    const toggle = $('tune-hybrid-on');
    if (toggle && !toggle.checked) toggle.click();
  };
  addEventListener('colm:step', () => refreshHybridCard().catch(e => status(e)));
  // 算例勾选与引擎下拉框的 change 冒泡到窗口；在这里统一接，不依赖那两个元素何时建好。
  addEventListener('change', event => {
    if (event.target?.closest?.('#cases-run')) refreshHybridCard().catch(e => status(e));
    else if (event.target?.id === 'model-engine') syncEngineWarning();
  });
}

// ---- 参数率定：同时训练 AI 模型 ------------------------------------------------------------

/** 勾了“训练 AI 模型”时返回 spec 的 `hybrid` 段，否则 `null`；填写不合法时抛出说明。 */
export function tuneHybridSection() {
  if (!$('tune-hybrid-on')?.checked) return null;
  return studySection({
    slot: $('tune-hybrid-slot').value,
    features: $('tune-hybrid-features').value,
    outputs: readOutputs($('tune-hybrid-outputs')),
    size: $('tune-hybrid-size').value,
  });
}

/** 权重数与种群大小的提示。 */
export function renderTuneHybridWeights(population) {
  const count = $('tune-hybrid-weight-count');
  if (!count) return;
  const on = $('tune-hybrid-on').checked;
  const n = weightCount(parseFeatures($('tune-hybrid-features').value).length,
    NETWORK_SIZES[$('tune-hybrid-size').value] ?? [], readOutputs($('tune-hybrid-outputs')).length);
  count.textContent = String(n);
  $('tune-hybrid-weight-warning').hidden = !(on && Number.isFinite(population) && n > population);
}

/** `onChange` 在表单任何改动后调用（results.js 用它作废已生成的任务并刷新预算）。 */
export function wireTuneHybrid(onChange) {
  if (!$('tune-hybrid-on')) return;
  // 表单在 AI 混合建模页，摘要在参数率定页：任何改动都要同时作废已生成的率定任务、刷新摘要与两步法的任务列表。
  const changed = () => {
    onChange?.();
    renderTuneSummary();
    if (studies.length) renderStudies();
  };
  tuneChanged = changed;
  const resetOutputs = () => {
    const defaults = slotDefaults($('tune-hybrid-slot').value);
    $('tune-hybrid-features').value = defaults.features;
    const host = $('tune-hybrid-outputs');
    host.replaceChildren();
    outputRow(host, defaults.output, ['sigmoid', 'clamp'], changed);
  };
  // 第一次打开时才填默认特征与输出（模块加载时不碰 DOM 内容）。
  $('tune-hybrid-on').onchange = () => {
    if ($('tune-hybrid-on').checked && !$('tune-hybrid-outputs').children.length) resetOutputs();
    changed();
  };
  $('tune-hybrid-slot').onchange = () => { resetOutputs(); changed(); };
  $('tune-hybrid-size').onchange = changed;
  $('tune-hybrid-features').oninput = changed;
  $('tune-hybrid-add-output').onclick = () => {
    outputRow($('tune-hybrid-outputs'), { name: outputPrefix($('tune-hybrid-slot').value), lo: '', hi: '', transform: 'sigmoid' }, ['sigmoid', 'clamp'], changed);
    changed();
  };
}

wireRunCard();

// ---- 研究 · AI 混合建模：模式与页内面板 ------------------------------------------------------

let selectedPreset = null;
let fitResult = null;
let hybridPane = 'overview';

const isHybridStep = () => state.step === 'hybrid-learn' || state.step === 'hybrid-process';
export const hybridMode = () => modeForStep(state.step);

const MODE_TEXT = {
  params: {
    title: 'AI 学习参数',
    intro: '用训练好的小网络，按每个 patch 的地表与气候特征给出物理参数（例如 Vcmax25），代替查表值。只有 Rust 引擎能运行；不装模型的算例照常按纯物理运行。',
  },
  process: {
    title: 'AI 替换过程',
    intro: '用训练好的小网络替换模型里的一个过程量。目前的过程插槽是土壤水分胁迫 β（替换 eroot 那一段），要先关闭植物水力，不支持 PC 模式。只有 Rust 引擎能运行；不装模型的算例照常按纯物理运行。',
  },
};

/** 只留下这个模式的插槽选项；当前值不在其中时换成第一个。 */
function filterSlotSelect(select, mode) {
  if (!select) return false;
  const slots = MODE_SLOTS[mode];
  for (const option of select.options ?? []) {
    const allowed = slots.includes(option.value);
    option.hidden = !allowed;
    option.disabled = !allowed;
  }
  if (slots.includes(select.value)) return false;
  select.value = mode === 'params' ? slotForMode(caseMode()) : slots[0];
  return true;
}

function showPane(name) {
  hybridPane = name;
  for (const pane of document.querySelectorAll('[data-hybrid-pane]')) pane.hidden = pane.dataset.hybridPane !== name;
  for (const b of document.querySelectorAll('[data-hybrid-pane-go]')) {
    const on = b.dataset.hybridPaneGo === name;
    b.classList.toggle('on', on);
    b.setAttribute('aria-pressed', String(on));
  }
  if (name === 'train') enterTraining();
}

/** 进入或切换模式：标题、插槽、预设与训练方法都跟着模式走。 */
function applyMode() {
  const mode = hybridMode();
  const text = MODE_TEXT[mode];
  if ($('hybrid-title')) $('hybrid-title').textContent = t(text.title);
  if ($('hybrid-intro')) $('hybrid-intro').textContent = t(text.intro);
  if (filterSlotSelect($('hybrid-slot'), mode)) {
    importSlotChosen = false;
    setImportSlot($('hybrid-slot').value, true);
  }
  const preset = PRESETS.find(p => p.id === selectedPreset);
  if (preset?.slot && !MODE_SLOTS[mode].includes(preset.slot)) selectedPreset = null;
  // 训练表单同理：插槽一换，特征与输出也要换成新插槽的（“自定义”预设没有插槽，上面那一句清不掉它）。
  if (filterSlotSelect($('tune-hybrid-slot'), mode)) {
    applyPreset(mode === 'process' ? 'beta' : presetForMode(caseMode()));
  }
  const methods = methodsForMode(mode);
  let current = null;
  for (const b of $('hybrid-method')?.querySelectorAll('button') ?? []) {
    b.hidden = !methods.includes(b.dataset.method);
    if (b.classList.contains('on')) current = b.dataset.method;
  }
  if (!methods.includes(current)) setMethod('de');
  showPane(hybridPane);
}

function enterTraining() {
  if (!selectedPreset) applyPreset(hybridMode() === 'process' ? 'beta' : presetForMode(caseMode()));
  // 每次进来都重画：β 预设能不能用取决于算例（地表模式、植物水力），算例信息是异步读到的。
  else renderPresets();
  if (!$('hybrid-fit-root').value) $('hybrid-fit-root').value = $('root')?.value ?? '';
}

function cachedCaseInfo() {
  return batchTarget().map(c => infoCache.get(c.dir)).find(Boolean);
}

/** 本次第一个算例上 β 插槽能不能用（研究总览的准备情况）；不能用时返回原因，没有算例时返回空串。 */
export async function processSlotBlocked() {
  const c = batchTarget()[0];
  if (!c || !hasBackend) return '';
  const info = infoCache.get(c.dir) ?? await caseInfo(c.dir).catch(() => null);
  return presetBlocked(PRESETS.find(preset => preset.slot === SOIL_STRESS), info);
}

function caseMode() {
  const info = cachedCaseInfo();
  return info?.land_mode ?? (state.subgrid === 'LCT' || ['USGS', 'IGBP'].includes(state.subgrid) ? 'lct' : 'pc');
}

function renderPresets() {
  const host = $('hybrid-presets');
  if (!host) return;
  host.replaceChildren(...presetsForMode(hybridMode()).map(preset => {
    const card = element('button', 'domain-card');
    card.type = 'button';
    card.dataset.preset = preset.id;
    card.setAttribute('aria-selected', String(selectedPreset === preset.id));
    const blocked = presetBlocked(preset, cachedCaseInfo());
    card.append(element('span', 'dt', preset.title), element('span', 'dd', preset.note));
    if (blocked) card.append(element('span', 'dsoon', blocked));
    if (preset.disabled || blocked) {
      card.disabled = true;
      card.className += ' disabled';
    } else {
      card.onclick = () => applyPreset(preset.id);
    }
    return card;
  }));
}

/** 套用预设：填好“查看或修改设置”里的表单；“自定义”只展开表单。 */
function applyPreset(id) {
  selectedPreset = id;
  const preset = PRESETS.find(p => p.id === id);
  if (preset?.outputs) {
    $('tune-hybrid-slot').value = preset.slot;
    $('tune-hybrid-features').value = preset.features;
    $('tune-hybrid-size').value = preset.size;
    const host = $('tune-hybrid-outputs');
    host.replaceChildren();
    for (const output of preset.outputs) outputRow(host, output, ['sigmoid', 'clamp'], tuneChanged);
  }
  $('hybrid-custom').open = id === 'custom';
  renderPresets();
  tuneChanged();
}

function presetTitle() {
  return PRESETS.find(p => p.id === selectedPreset)?.title ?? '自定义';
}

function setMethod(method) {
  for (const b of $('hybrid-method').querySelectorAll('button')) {
    const on = b.dataset.method === method;
    b.classList.toggle('on', on);
    b.setAttribute('aria-pressed', String(on));
  }
  for (const pane of document.querySelectorAll('[data-method-pane]')) pane.hidden = pane.dataset.methodPane !== method;
  if (method === 'two-step' && !$('hybrid-fit-studies').childElementCount) refreshStudies().catch(e => status(e?.message || e));
}

/** 当前“要学什么”的网络设置（与 Study 的 hybrid 段同格式）；不合法时抛出说明。 */
function currentNetwork() {
  return studySection({
    slot: $('tune-hybrid-slot').value,
    features: $('tune-hybrid-features').value,
    outputs: readOutputs($('tune-hybrid-outputs')),
    size: $('tune-hybrid-size').value,
  });
}

let studies = [];

async function refreshStudies() {
  const root = $('hybrid-fit-root').value.trim() || $('root')?.value?.trim();
  if (!root) {
    $('hybrid-fit-studies').replaceChildren(element('div', 'result-empty', '先填项目目录'));
    return;
  }
  $('hybrid-fit-root').value = root;
  $('hybrid-fit-studies').replaceChildren(element('div', 'result-empty', '正在查找…'));
  studies = JSON.parse(await invoke('hybrid_studies', { root }));
  renderStudies();
}

function renderStudies() {
  const host = $('hybrid-fit-studies');
  let names = [];
  try { names = currentNetwork().outputs.map(o => o.name); } catch { /* 表单还没填好：只列出任务 */ }
  if (!studies.length) {
    host.replaceChildren(element('div', 'result-empty', '这个目录里没有参数率定任务。先在“研究 → 参数率定”里为每个站点建一个，率定上面的输出参数。'));
    return;
  }
  const table = element('table', 'result-table');
  const head = element('tr');
  for (const title of ['', '站点', '率定的参数', '状态', '']) head.appendChild(element('th', '', title));
  table.appendChild(head);
  for (const study of studies) {
    const check = studyUsable(study, names);
    const row = element('tr');
    const box = element('input');
    box.type = 'checkbox';
    box.dataset.study = study.dir;
    box.checked = check.usable;
    box.disabled = !check.usable;
    const cell = element('td');
    cell.appendChild(box);
    row.append(cell, element('td', '', study.sites.join(', ')), element('td', 'mini', study.parameters.join(', ')),
      element('td', 'mini', study.status), element('td', 'muted mini', check.reason));
    table.appendChild(row);
  }
  const usable = studies.filter(s => studyUsable(s, names).usable).length;
  host.replaceChildren(element('p', 'mini muted', `找到 ${studies.length} 个率定任务，可用 ${usable} 个。`), table);
}

function selectedStudies() {
  return [...$('hybrid-fit-studies').querySelectorAll('input[data-study]:checked')].map(b => b.dataset.study);
}

async function runFit() {
  const network = currentNetwork();
  const chosen = selectedStudies();
  if (!chosen.length) throw new Error('至少勾选一个可用的率定任务');
  const root = $('hybrid-fit-root').value.trim();
  const linear = !network.hidden.length;
  const request = {
    studies: chosen,
    network,
    kernel_dir: $('kernel').value,
    out_dir: `${root.replace(/\/+$/, '')}/ai-models`,
    name: $('hybrid-fit-name').value.trim() || 'ai-model',
    ridge: linear ? Number($('hybrid-fit-ridge').value) : null,
    epochs: linear ? null : Number($('hybrid-fit-epochs').value),
  };
  const box = $('hybrid-fit-result');
  box.replaceChildren(element('p', 'mini muted', `正在拟合（${chosen.length} 个任务，每个都要在基础算例上取特征，可能要几分钟）…`));
  $('hybrid-fit-run').disabled = true;
  try {
    fitResult = { ...(await invoke('hybrid_fit', { request })), network };
    renderFitResult();
  } finally {
    $('hybrid-fit-run').disabled = false;
  }
}

function fmt(value) {
  return value == null ? '—' : Number(value).toPrecision(3);
}

function renderFitResult() {
  const box = $('hybrid-fit-result');
  const summary = fitSummary(fitResult.report);
  const verdict = summary.passed === true
    ? element('p', 'assistant-key-ok', `✓ 通过：留一站交叉验证误差 ${fmt(summary.network)}，低于均值基准 ${fmt(summary.baseline)}。`)
    : summary.passed === false
      ? element('p', 'assistant-fail', `✗ 没通过：网络的留一站误差 ${fmt(summary.network)} 不低于均值基准 ${fmt(summary.baseline)}，到新站点上大概率不如直接用均值。不建议装到算例。`)
      : element('p', 'warn mini', '只有一个任务，做不了交叉验证；装之前请自己确认效果。');
  const parts = [verdict, element('p', 'mini muted', `样本 ${summary.samples} 行；模型写在 ${fitResult.model}`)];
  if (summary.rows.length) {
    const table = element('table', 'result-table');
    const head = element('tr');
    for (const title of ['留出的任务', '网络误差', '均值基准', '']) head.appendChild(element('th', '', title));
    table.appendChild(head);
    for (const row of summary.rows) {
      const tr = element('tr');
      const study = studies.find(s => s.dir === row.study);
      tr.append(element('td', '', study ? study.sites.join(', ') : baseName(row.study)), element('td', '', fmt(row.network)),
        element('td', '', fmt(row.baseline)), element('td', row.better ? 'assistant-key-ok' : 'muted', row.better ? '更好' : '不如'));
      table.appendChild(tr);
    }
    parts.push(table);
  }
  const install = element('button', 'run-btn', '装到本次算例');
  install.type = 'button';
  install.disabled = summary.passed === false;
  install.onclick = () => installFitted().catch(e => status(e?.message || e));
  parts.push(element('div', 'pill-row'));
  parts.at(-1).appendChild(install);
  box.replaceChildren(...parts);
}

async function installFitted() {
  const dirs = batchTarget().map(c => c.dir);
  if (!dirs.length) throw new Error('本次还没有算例；先在基本设定里建算例');
  const { network, model, normalize } = fitResult;
  const outputs = network.outputs.map(o => outputArg({ name: o.name, lo: o.range[0], hi: o.range[1], transform: o.transform, relative: o.relative }));
  await invoke('hybrid_install', {
    dirs, model, slot: network.slot, features: network.features, outputs, normalize, force: true, outsidePhysics: true,
  });
  status(dirs.length === 1 ? '已把拟合的模型装到算例' : `已把拟合的模型装到 ${dirs.length} 个算例`);
  await refreshHybridCard();
  showPane('overview');
}

/** 参数率定里只显示一行摘要，设置都在 AI 混合建模页。 */
function renderTuneSummary() {
  const text = $('tune-hybrid-summary-text');
  if (!text) return;
  if (!$('tune-hybrid-on').checked) {
    text.textContent = t('本次率定不训练 AI 模型。要训练，请到“研究 → AI 混合建模”的“训练”。');
    return;
  }
  let detail = '';
  try {
    const network = currentNetwork();
    detail = ` · ${t('特征')} ${network.features.join(', ')} · ${t('输出')} ${network.outputs.map(o => o.name).join(', ')}`;
  } catch (error) {
    detail = ` · ${String(error?.message || error)}`;
  }
  text.textContent = `${t('本次率定同时训练 AI 模型')}：${t(presetTitle())}${detail}`;
}

let tuneChanged = () => {};

// 模块加载时不碰 DOM（测试的 DOM 替身很简陋，见第 610 轮）：按钮用窗口上的一个委托点击监听，
// 预设卡片第一次进训练页时才画。
function wireTraining() {
  addEventListener('click', event => {
    const target = event.target?.closest?.('#hybrid-method [data-method], [data-hybrid-go], [data-hybrid-pane-go], #hybrid-fit-refresh, #hybrid-fit-run, #tune-hybrid-edit');
    if (!target) return;
    if (target.dataset.method) setMethod(target.dataset.method);
    else if (target.dataset.hybridGo) showPane(target.dataset.hybridGo);
    else if (target.dataset.hybridPaneGo) showPane(target.dataset.hybridPaneGo);
    else if (target.id === 'hybrid-fit-refresh') refreshStudies().catch(e => status(e?.message || e));
    else if (target.id === 'hybrid-fit-run') {
      runFit().catch(e => $('hybrid-fit-result').replaceChildren(element('p', 'assistant-fail', String(e?.message || e))));
    } else if (target.id === 'tune-hybrid-edit') {
      hybridPane = 'train';
      go($('tune-hybrid-slot')?.value === SOIL_STRESS ? 'hybrid-process' : 'hybrid-learn');
      setMethod('de');
    }
  });
  addEventListener('colm:step', () => {
    if (isHybridStep()) applyMode();
    renderTuneSummary();
  });
}

wireTraining();
