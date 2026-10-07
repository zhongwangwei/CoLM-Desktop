//! AI 参数化（混合模型，docs/design-hybrid.md）：运行页的算例卡片、导入表单，以及参数调优里
//! “同时训练 AI 参数化”的表单。
//!
//! 只依赖 ipc/state/ui/batch/engine：runner.js 与 results.js 都导入它，反过来导入会成环。
//! 算例上的配置由 `colm-cli hybrid-info` 读出（JSON），这里不解析 TOML。

import { invoke, hasBackend } from './ipc.js';
import { $, status, baseName } from './ui.js';
import { batchTarget } from './batch.js';
import { modelEngine } from './engine.js';
import { language, translateZh } from './i18n.js';

// ---- 纯函数（tests/hybrid.mjs）----------------------------------------------------------

/** 地表模式决定插槽：LCT 按地类（`land_class`），PFT/PC 按植物功能型（`pft`）。 */
export function slotForMode(mode) {
  return mode === 'lct' ? 'land_class' : 'pft';
}

/** 插槽能驱动的参数前缀。 */
export function outputPrefix(slot) {
  return slot === 'land_class' ? 'DEF_LC_' : 'DEF_PFT_';
}

/** 新插槽的默认特征与输出（Vcmax25，常见取值 10–150 μmol m⁻² s⁻¹）。 */
export function slotDefaults(slot) {
  return slot === 'land_class'
    ? { features: 'patchclass', output: { name: 'DEF_LC_VMAX25', lo: 10, hi: 150, transform: 'sigmoid' } }
    : { features: 'pftclass, pftfrac', output: { name: 'DEF_PFT_VMAX25', lo: 10, hi: 150, transform: 'sigmoid' } };
}

/** 逗号分隔的特征名。 */
export function parseFeatures(text) {
  return String(text ?? '').split(/[,，\s]+/).map(name => name.trim()).filter(Boolean);
}

/** 校验一行输出，返回 `{name, lo, hi, transform}`；不合法时抛出说明。 */
export function checkOutput(slot, { name, lo, hi, transform }) {
  const prefix = outputPrefix(slot);
  const field = String(name ?? '').trim().toUpperCase();
  if (!field.startsWith(prefix) || field.length === prefix.length) {
    throw new Error(`输出参数必须以 ${prefix} 开头`);
  }
  // 空格子不是 0：`Number('')` 会把没填的下限当成 0。
  const number = value => (String(value ?? '').trim() === '' ? NaN : Number(value));
  const low = number(lo);
  const high = number(hi);
  if (!Number.isFinite(low) || !Number.isFinite(high) || low >= high) {
    throw new Error(`${field} 的范围必须是有限数且下限小于上限`);
  }
  return { name: field, lo: low, hi: high, transform: transform || 'sigmoid' };
}

/** `colm-cli hybrid-install --output` 的写法。 */
export function outputArg({ name, lo, hi, transform }) {
  return `${name}:${lo}:${hi}:${transform}`;
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
  if (!names.length) throw new Error('AI 参数化至少需要一个输入特征');
  if (!outputs.length) throw new Error('AI 参数化至少需要一个输出参数');
  const checked = outputs.map(output => checkOutput(slot, output));
  if (new Set(checked.map(o => o.name)).size !== checked.length) throw new Error('AI 参数化的输出参数不能重复');
  for (const output of checked) {
    if (!['sigmoid', 'clamp'].includes(output.transform)) throw new Error('训练时输出只能用 S 形映射或截断');
  }
  return {
    slot,
    features: names,
    outputs: checked.map(o => ({ name: o.name, range: [o.lo, o.hi], transform: o.transform })),
    hidden: NETWORK_SIZES[size] ?? [],
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
  const remove = element('button', 'btn-ghost', '删除');
  remove.type = 'button';
  remove.onclick = () => { row.remove(); onChange?.(); };
  for (const el of [name, lo, hi, transform]) el.addEventListener('input', () => onChange?.());
  transform.addEventListener('change', () => onChange?.());
  row.append(name, lo, element('span', 'muted', '–'), hi, transform, remove);
  row.read = () => ({ name: name.value, lo: lo.value, hi: hi.value, transform: transform.value });
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
  cell.textContent = info.slots.some(slot => slot.trained_by_study) ? '已安装（参数调优训练）' : '已安装';
  return cell;
}

function renderCaseTable(cases, infos) {
  const table = element('table');
  const head = element('tr');
  for (const label of ['算例', '地表模式', 'AI 参数化', '作用参数', '模型文件', '气候特征', '']) head.appendChild(element('th', '', label));
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
    const wrap = element('div', 'result-table-wrap');
    wrap.appendChild(table);
    blocks.push(rows, wrap);
  }
  host.replaceChildren(...blocks);
}

async function removeCase(c) {
  const question = '移除这个算例的 AI 参数化模型？算例回到纯物理参数；之后的运行会按新设定重跑。';
  if (!globalThis.confirm(language() === 'en' ? translateZh(question) : question)) return;
  await invoke('hybrid_remove', { dirs: [c.dir] });
  status('已移除 AI 参数化模型');
  $('hybrid-check-result')?.replaceChildren();
  await refreshHybridCard();
}

// ---- 导入表单 ------------------------------------------------------------------------

let importSlotChosen = false;

function syncImportSlot(infos) {
  const select = $('hybrid-slot');
  if (!select || importSlotChosen) return;
  const modes = new Set(infos.map(info => info?.land_mode).filter(Boolean));
  if (modes.size === 1) setImportSlot(slotForMode([...modes][0]));
}

function setImportSlot(slot) {
  const select = $('hybrid-slot');
  if (!select || select.value === slot && $('hybrid-outputs')?.children.length) return;
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
  await invoke('hybrid_install', { dirs, model, slot, features, outputs, normalize, force: $('hybrid-force').checked });
  status(dirs.length === 1 ? '已导入模型' : `已为 ${dirs.length} 个算例导入模型`);
  await refreshHybridCard();
}

function wireRunCard() {
  if (!$('hybrid-card')) return;
  $('hybrid-slot').onchange = () => { importSlotChosen = true; setImportSlot($('hybrid-slot').value); };
  $('hybrid-add-output').onclick = () => {
    outputRow($('hybrid-outputs'), { name: outputPrefix($('hybrid-slot').value), lo: '', hi: '', transform: 'sigmoid' }, ['sigmoid', 'clamp', 'identity']);
  };
  $('hybrid-pick-model').onclick = () => pickInto('hybrid-model-path', 'hybrid-model', 'onnx,json').catch(e => status(e));
  $('hybrid-pick-normalize').onclick = () => pickInto('hybrid-normalize-path', 'hybrid-normalize', 'json').catch(e => status(e));
  $('hybrid-install').onclick = () => installModel().catch(e => status(e?.message || e));
  addEventListener('colm:step', () => refreshHybridCard().catch(e => status(e)));
  // 算例勾选与引擎下拉框的 change 冒泡到窗口；在这里统一接，不依赖那两个元素何时建好。
  addEventListener('change', event => {
    if (event.target?.closest?.('#cases-run')) refreshHybridCard().catch(e => status(e));
    else if (event.target?.id === 'model-engine') syncEngineWarning();
  });
}

// ---- 参数调优：同时训练 AI 参数化 ------------------------------------------------------------

/** 勾了“训练 AI 参数化”时返回 spec 的 `hybrid` 段，否则 `null`；填写不合法时抛出说明。 */
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
  $('tune-hybrid-form').hidden = !on;
  const n = weightCount(parseFeatures($('tune-hybrid-features').value).length,
    NETWORK_SIZES[$('tune-hybrid-size').value] ?? [], readOutputs($('tune-hybrid-outputs')).length);
  count.textContent = String(n);
  $('tune-hybrid-weight-warning').hidden = !(on && Number.isFinite(population) && n > population);
}

/** `onChange` 在表单任何改动后调用（results.js 用它作废已生成的任务并刷新预算）。 */
export function wireTuneHybrid(onChange) {
  if (!$('tune-hybrid-on')) return;
  const changed = () => onChange?.();
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
