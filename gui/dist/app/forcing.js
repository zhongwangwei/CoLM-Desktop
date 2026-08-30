//! 前处理页：强迫场探测与转换（`docs/design-prep.md` §2.1 阶段 A）。
//!
//! NetCDF 走单站路径；CSV/TXT/TSV 可以是一站，也可以是带站点列的长表。
//! 表格先按站点拆成 UTC 标准时间轴，再让每个站点复用同一套缺测诊断、
//! 插值、ERA5-Land 订正和 QC 代码。两种输入最终都只产出标准单站 NetCDF，
//! 下游不需要知道原始文件是哪一种格式。
//!
//! 逐项编辑状态留在本模块；只有完成后的站点/强迫场清单进入共享 `state`，
//! 供“就绪检查”和“基本设定”扫描，不让半成品污染下游。
//!
//! **确认映射是必经一步**（`design-prep.md` §2.1）：变量名猜错的后果是
//! 「跑得完、结果全错」——模型照样跑完，曲线照样是曲线，界面上什么都看
//! 不出来。④ 的转换按钮默认禁用，只有在②上点过「这些映射我看过了」才
//! 会启用；改了②的任何一行（变量、源单位、合并变量）都会把确认打回去，
//! 否则「我看过了」说的是改之前那一版。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status, joinPath, baseName, forcingDirectoryForSiteDirectory } from './ui.js';
import {
  forcingOutputName, missingForcingHeights, prepMode, siteOutputName,
} from './prep-state.js';
import { scanPreparedSites } from './sites.js';

/** 探测结果。没探过是 `null`。 */
let probe = null;
/** 源文件路径，探测成功那一刻记下来 —— 转换时还要用它算产物文件名。 */
let srcPath = '';
/** 每个槽位当前选的变量名，下标对应 `probe.slots`。空字符串是「（不用）」。 */
let picks = [];
/** 每个槽位的源单位（可能是探出来的，也可能是用户补的）。 */
let unitsInput = [];
/** 每个槽位要合并进去的额外变量（目前只有降水槽用得到，至多一个）。 */
let extras = [];
/** 三个观测高度，`null` 表示源文件没有、也还没有人填。 */
let heights = { v: null, t: null, q: null };
/** 用户是否在当前这版映射上点过「确认」。改了任何一行映射就清掉。 */
let confirmed = false;
/** 产物目录，卡片重画时要保留用户已经打的字。 */
let dstDir = '';
/** 上一次转换成功的产物路径。`null` 表示还没转换过，或者刚探了新文件。 */
let lastResult = null;
/** 最近一次缺测诊断；映射或修复设置变化就失效。 */
let gapReport = null;
/** 已修复中间文件。没有缺测时保持 null，转换直接读原文件。 */
let repairedSource = null;
/** 非空表示当前输入是 CSV/TXT/TSV，而不是既有 NetCDF。 */
let tableProbe = null;
/** 表格列配置；自动探测只负责预填，用户确认的值才交给导入器。 */
let tableSettings = null;
/** 按站点拆分后的诊断/修复状态。每一项永远只对应一个站点。 */
let tableBatch = [];
let tableBusy = false;
let era5Busy = false;
const defaultGapSettings = () => ({
  shortGap: 3,
  utcOffset: '',
  latitude: '',
  longitude: '',
  era5: '',
  minOverlap: 24,
});
let gapSettings = defaultGapSettings();

function tableRow(cells) {
  const row = document.createElement('tr');
  for (const cell of cells) {
    const td = document.createElement(cell.header ? 'th' : 'td');
    td.textContent = cell.text ?? '';
    if (cell.className) td.className = cell.className;
    row.appendChild(td);
  }
  return row;
}

globalThis.addEventListener?.('colm:prep-site-invalidated', () => {
  lastResult = null;
  if (probe) renderCards();
});

const MEANING_ZH = {
  'air temperature': '气温',
  'specific humidity': '比湿',
  'surface pressure': '气压',
  precipitation: '降水',
  'eastward wind': '东风',
  'northward or scalar wind': '北风 / 标量风',
  'downward shortwave': '短波辐射',
  'downward longwave': '长波辐射',
};
const zh = m => MEANING_ZH[m] ?? m;

const TABLE_CANONICAL_VARIABLES = {
  1: 'Tair',
  2: 'Qair',
  3: 'Psurf',
  4: 'Precip',
  5: 'Wind_E',
  6: 'Wind_N',
  7: 'SWdown',
  8: 'LWdown',
};

const isTabularPath = path => /\.(?:csv|txt|tsv)$/i.test(path);

function resetForcingSourceState() {
  gapSettings = defaultGapSettings();
}

function resetOutputs() {
  confirmed = false;
  lastResult = null;
  gapReport = null;
  repairedSource = null;
  tableBatch = [];
  Object.assign(state.prepArtifacts, {
    forcingFile: null,
    forcingDir: null,
    batchSites: [],
  });
  globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
}

function initializeMappings(result, tabular) {
  probe = tabular
    ? {
        variables: result.columns.map(column => column.name),
        slots: result.slots.map(slot => ({ ...slot, guessed: slot.column })),
      }
    : result;
  picks = probe.slots.map(slot => slot.guessed ?? '');
  unitsInput = probe.slots.map(slot => slot.units ?? '');
  extras = probe.slots.map(() => []);
}

$('fprobe').onclick = async () => {
  const path = $('fsrc').value.trim();
  if (!path) { status('先选一份强迫场文件'); return; }
  $('fprobe').disabled = true;
  try {
    const tabular = isTabularPath(path);
    const result = tabular
      ? await invoke('probe_forcing_table', { path })
      : await invoke('probe_forcing', { path });
    srcPath = path;
    resetForcingSourceState();
    tableProbe = tabular ? result : null;
    initializeMappings(result, tabular);
    heights = tabular
      ? { v: null, t: null, q: null }
      : { v: result.height_v, t: result.height_t, q: result.height_q };
    tableSettings = tabular ? {
      timeColumn: result.time_column ?? '',
      siteColumn: result.site_column ?? '',
      latitudeColumn: result.latitude_column ?? '',
      longitudeColumn: result.longitude_column ?? '',
      landtypeColumn: result.landtype_column ?? '',
      offsetColumn: result.utc_offset_column ?? '',
      latitude: '',
      longitude: '',
      utcOffset: '',
      stepSeconds: commonTableStep(result),
      createSites: true,
      siteDir: $('soutdir')?.value.trim() || state.prepArtifacts.siteDir || '',
      rawdata: $('srawdata')?.value.trim() || state.prepArtifacts.rawdataDir || '',
    } : null;
    // 经纬度/时区/ERA5 缓存不沿用上一站点；后端会优先读文件坐标，读不到才用这里的人填值。
    // 产物目录只在还没填过时用后端建议的那个 —— 用户改过就别再动它，
    // 换一份源文件重新探测不该把他填的路径冲掉。
    if (!dstDir) {
      dstDir = state.prepArtifacts.siteDir
        ? forcingDirectoryForSiteDirectory(state.prepArtifacts.siteDir)
        : (result.suggest_dst ?? '');
    }
    resetOutputs();
    renderCards();
    status(tabular
      ? `已探测 ${baseName(path)}：${result.rows} 行，${result.sites.length} 个站点`
      : `已探测 ${baseName(path)}：${probe.variables.length} 个变量，${probe.steps} 步`);
  } catch (e) { status(e); }
  finally { $('fprobe').disabled = false; }
};

function renderCards() {
  const box = $('forcing-cards');
  if (!box) return;
  box.textContent = '';
  if (!probe) return;
  if (tableProbe) {
    box.appendChild(tableStructureCard());
    box.appendChild(slotsCard());
    box.appendChild(tableBatchCard());
    return;
  }
  box.appendChild(slotsCard());
  box.appendChild(timingCard());
  box.appendChild(gapCard());
  box.appendChild(convertCard());
}

function invalidateGap() {
  gapReport = null;
  repairedSource = null;
  lastResult = null;
  tableBatch = [];
}

function commonTableStep(result) {
  const values = result.sites
    .map(site => site.step_seconds)
    .filter(value => value != null);
  return values.length && values.every(value => value === values[0]) ? String(values[0]) : '';
}

function selectedSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ i }) => picks[i])
    .map(({ s, i }) => ({
      index: s.index,
      name: picks[i],
      units: unitsInput[i].trim(),
      also_add: extras[i] ?? [],
    }));
}

function optionalNumber(value) {
  if (value === '' || value == null) return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

function gapOptions(includeEra5 = true) {
  return {
    short_gap: Math.max(0, Math.trunc(Number(gapSettings.shortGap) || 0)),
    utc_offset: optionalNumber(gapSettings.utcOffset),
    latitude: optionalNumber(gapSettings.latitude),
    longitude: optionalNumber(gapSettings.longitude),
    era5: includeEra5 && gapSettings.era5.trim() ? gapSettings.era5.trim() : null,
    min_overlap: Math.max(1, Math.trunc(Number(gapSettings.minOverlap) || 24)),
  };
}

// ------------------------------------------------------------ ② 槽位映射

function missingRequiredSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ s, i }) => !s.optional && !picks[i]);
}

function missingUnitSlots() {
  return probe.slots
    .map((s, i) => ({ s, i }))
    .filter(({ i }) => picks[i] && !unitsInput[i].trim());
}

function unitsForVariable(name, slot) {
  if (!name) return '';
  if (tableProbe) {
    return tableProbe.columns.find(column => column.name === name)?.units
      ?? (name === slot.guessed ? slot.units ?? '' : '');
  }
  return name === slot.guessed ? (slot.units ?? '') : '';
}

function slotsCard() {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>槽位映射</h3>
    <div class="ch">CoLM 认死了这八个槽位（<code>MOD_UserSpecifiedForcing.F90</code>：
      1 气温 2 比湿 3 气压 4 降水 5 东风 6 北风/标量风 7 短波辐射 8 长波辐射）。
      自动猜的对不对、单位要不要换，都要你确认一遍 ——
      <b>变量名猜错的后果是「跑得完、结果全错」</b>，模型照样跑完，曲线照样是曲线，
      界面上什么都看不出来。</div>
    <table>
      <tr><th>槽位</th><th>含义</th><th>变量</th><th>源单位</th><th>目标单位</th></tr>
    </table>`;
  const table = card.querySelector('table');
  for (let i = 0; i < probe.slots.length; i++) table.appendChild(slotRow(i));

  const bar = document.createElement('div');
  bar.className = 'pill-row';
  bar.style.marginTop = '12px';
  const confirmBtn = document.createElement('button');
  confirmBtn.className = 'btn-ghost';
  confirmBtn.textContent = '这些映射我看过了';
  confirmBtn.onclick = () => {
    confirmed = true;
    status('映射已确认，下面「转换」可以按了');
    renderCards();
  };
  bar.appendChild(confirmBtn);
  const st = document.createElement('span');
  st.className = 'mini ' + (confirmed ? 'muted' : 'warn');
  st.textContent = confirmed
    ? '已确认 —— 再改任何一行都会打回未确认'
    : '还没确认，下面「转换」按钮不会亮';
  bar.appendChild(st);
  card.appendChild(bar);

  const missing = missingRequiredSlots();
  if (missing.length) {
    const p = document.createElement('p');
    p.className = 'fail mini';
    p.style.marginTop = '8px';
    p.textContent = '必需槽位还没选变量：' +
      missing.map(({ s }) => `第 ${s.index} 槽（${zh(s.meaning)}）`).join('、');
    card.appendChild(p);
  }
  return card;
}

/** 画第 `i` 个槽位那一行。**恒画 8 行**——不是每个数据集都用得上第 5 槽
 *  （PLUMBER2 只有标量 `Wind`），但那是「这一槽空着」，不是「这一槽不存在」。
 *  写死成 7 行会在 Urban-PLUMBER 的 `Wind_E` 上漏掉一整个变量。 */
function slotRow(i) {
  const s = probe.slots[i];
  const tr = document.createElement('tr');

  const tdIdx = document.createElement('td');
  tdIdx.textContent = String(s.index);
  tr.appendChild(tdIdx);

  const tdMeaning = document.createElement('td');
  tdMeaning.textContent = zh(s.meaning);
  tr.appendChild(tdMeaning);

  const tdVar = document.createElement('td');
  // 没猜到、又是必需槽位：这一行需要人立刻处理。「猜到了」不上色 ——
  // 默认色就是「正常」，只有这两种状态色（warn/fail）才值得标出来。
  if (!s.guessed && !s.optional) tdVar.className = 'fail';
  const sel = document.createElement('select');
  sel.className = 'select';
  const opt0 = document.createElement('option');
  opt0.value = '';
  opt0.textContent = '（不用）';
  sel.appendChild(opt0);
  for (const v of probe.variables) {
    const o = document.createElement('option');
    o.value = v;
    o.textContent = v;
    sel.appendChild(o);
  }
  sel.value = picks[i];
  sel.onchange = () => {
    picks[i] = sel.value;
    // 探测阶段量到的单位只对**猜出来的那个变量**有效。换成别的变量之后
    // 留着旧单位会让人以为它还对 —— 只有选回原来那个猜测时才恢复，
    // 其余一律清空，逼用户自己填。
    unitsInput[i] = unitsForVariable(sel.value, s);
    if (s.index !== 4) extras[i] = []; // 只有降水槽用得到 also_add
    confirmed = false;
    invalidateGap();
    renderCards();
  };
  tdVar.appendChild(sel);
  if (s.optional) {
    const note = document.createElement('div');
    note.className = 'muted mini';
    note.textContent = '这一槽可以空着 —— 标量风的数据集没有它，模型照样能跑。';
    tdVar.appendChild(note);
  }
  // 降水槽（第 4 槽）能再加一个同单位的变量合并进去：Urban-PLUMBER 把
  // 降水拆成 Rainf + Snowf，不合并就丢掉全部降雪（实测 FI-Kumpula 少 24.7%）。
  if (s.index === 4 && picks[i]) {
    const note = document.createElement('div');
    note.className = 'muted mini';
    note.style.marginTop = '4px';
    note.textContent = '再加一个同单位的变量合并进这一槽（降水常拆成雨 + 雪两个变量）：';
    tdVar.appendChild(note);
    const extraSel = document.createElement('select');
    extraSel.className = 'select';
    const oNone = document.createElement('option');
    oNone.value = '';
    oNone.textContent = '（不加）';
    extraSel.appendChild(oNone);
    for (const v of probe.variables) {
      if (v === picks[i]) continue;
      const o = document.createElement('option');
      o.value = v;
      o.textContent = v;
      extraSel.appendChild(o);
    }
    // **主变量换成了原来那个额外变量时，额外变量要清掉。**
    //
    // 不清的话下面这行赋值会静默失败 —— 上面的循环已经把主变量从选项里
    // 排除了，给 `<select>` 赋一个不存在的选项值，它会变成空串。于是
    // **界面显示「（不加）」，而 `extras[i]` 里还留着那个名字**，
    // 转换时发出去的是 `4=Snowf:kg/m2/s+Snowf`，后端把它加两次，
    // 降水翻倍而模型照样跑完。
    //
    // 校验放在渲染这里而不是 `sel.onchange` 里：这一行守的是
    // 「显示出来的必须等于将要发出去的」，而渲染是唯一能保证覆盖
    // 所有改动路径的地方。
    if (extras[i][0] === picks[i]) extras[i] = [];
    extraSel.value = extras[i][0] ?? '';
    extraSel.onchange = () => {
      extras[i] = extraSel.value ? [extraSel.value] : [];
      confirmed = false;
      invalidateGap();
      renderCards();
    };
    tdVar.appendChild(extraSel);
  }
  tr.appendChild(tdVar);

  const tdUnits = document.createElement('td');
  const uInp = document.createElement('input');
  uInp.className = 'input';
  uInp.style.width = '7em';
  uInp.value = unitsInput[i];
  uInp.disabled = !picks[i];
  uInp.placeholder = picks[i] ? '必填' : '—';
  const unitsMissing = !!picks[i] && !unitsInput[i].trim();
  const unitsDiffer = !!picks[i] && !!unitsInput[i].trim() && unitsInput[i].trim() !== s.wants;
  if (unitsMissing) tdUnits.className = 'fail';
  else if (unitsDiffer) tdUnits.className = 'warn';
  uInp.onchange = () => {
    unitsInput[i] = uInp.value.trim();
    confirmed = false;
    invalidateGap();
    renderCards();
  };
  tdUnits.appendChild(uInp);
  tr.appendChild(tdUnits);

  const tdWant = document.createElement('td');
  tdWant.textContent = s.wants;
  tr.appendChild(tdWant);

  return tr;
}

// ---------------------------------------- CSV / TXT：结构确认与多站点批处理

function tableColumnField(label, key, required = false) {
  const field = document.createElement('div');
  field.className = 'field';
  const lab = document.createElement('label');
  lab.textContent = label;
  field.appendChild(lab);
  const select = document.createElement('select');
  select.id = `forcing-table-${key}`;
  lab.htmlFor = select.id;
  select.className = 'select';
  const empty = document.createElement('option');
  empty.value = '';
  const mustKeepSiteColumn = key === 'siteColumn' && tableProbe.sites.length > 1;
  empty.textContent = required || mustKeepSiteColumn ? '请选择一列' : '（没有 / 不使用）';
  empty.disabled = mustKeepSiteColumn;
  select.appendChild(empty);
  for (const column of tableProbe.columns) {
    const option = document.createElement('option');
    option.value = column.name;
    option.textContent = column.units ? `${column.name} [${column.units}]` : column.name;
    select.appendChild(option);
  }
  select.value = tableSettings[key];
  select.onchange = () => {
    tableSettings[key] = select.value;
    invalidateTableBatch();
    renderCards();
  };
  field.appendChild(select);
  return field;
}

function tableNumberField(label, key, options = {}) {
  const field = document.createElement('div');
  field.className = 'field';
  const lab = document.createElement('label');
  lab.textContent = label;
  field.appendChild(lab);
  const input = document.createElement('input');
  input.id = `forcing-table-${key}`;
  lab.htmlFor = input.id;
  input.className = 'input';
  input.type = 'number';
  input.step = options.step ?? 'any';
  if (options.min != null) input.min = String(options.min);
  if (options.max != null) input.max = String(options.max);
  input.placeholder = options.placeholder ?? '';
  input.value = tableSettings[key] ?? '';
  input.onchange = () => {
    tableSettings[key] = input.value;
    invalidateTableBatch();
    renderCards();
  };
  field.appendChild(input);
  return field;
}

function resetBatchArtifacts() {
  const hadBatch = (state.prepArtifacts.batchSites?.length ?? 0) > 0;
  Object.assign(state.prepArtifacts, {
    forcingFile: null,
    forcingDir: null,
    batchSites: [],
    ...(hadBatch ? { siteFile: null, siteReport: null } : {}),
  });
  globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
}

function invalidateTableBatch() {
  tableBatch = [];
  resetBatchArtifacts();
}

function tableStructureCard() {
  const landScheme = prepMode(state);
  const landColumnLabel = landScheme === 'urban'
    ? '局地气候区（LCZ）列'
    : `${landScheme === 'usgs' ? 'USGS' : 'IGBP'} 地表覆盖类型列`;
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>CSV / TXT 表格结构与站点</h3>
    <div class="ch">一份表格可以只含一个站点，也可以用站点列保存多个站点。
      软件会先按站点拆分，不会把不同站点混进同一个文件；所有时间统一换算为 UTC，
      缺少的整条时间记录会作为真实缺口交给后面的诊断和修复。</div>
    <table style="margin-top:12px"></table>`;
  const summary = card.querySelector('table');
  summary.append(
    tableRow([{ header: true, text: '分隔方式' }, { text: tableProbe.delimiter }]),
    tableRow([{ header: true, text: '数据行' }, { text: String(tableProbe.rows) }]),
    tableRow([{ header: true, text: '识别到的站点' }, { text: String(tableProbe.sites.length) }]),
  );

  const columns = document.createElement('div');
  columns.className = 'table-grid';
  columns.style.marginTop = '12px';
  for (const field of [
    tableColumnField('时间列', 'timeColumn', true),
    tableColumnField('站点名称列', 'siteColumn'),
    tableColumnField('纬度列', 'latitudeColumn'),
    tableColumnField('经度列', 'longitudeColumn'),
    tableColumnField(landColumnLabel, 'landtypeColumn'),
    tableColumnField('UTC 偏移列（小时）', 'offsetColumn'),
  ]) columns.appendChild(field);
  card.appendChild(columns);

  const fallbacks = document.createElement('div');
  fallbacks.className = 'table-grid';
  fallbacks.style.marginTop = '10px';
  for (const field of [
    tableNumberField('单站纬度（表格无纬度列时）', 'latitude', { min: -90, max: 90 }),
    tableNumberField('单站经度（表格无经度列时）', 'longitude', { min: -180, max: 180 }),
    tableNumberField('人工 UTC 偏移（小时）', 'utcOffset', { min: -12, max: 14, step: 0.25, placeholder: '自动判断' }),
    tableNumberField('时间步长（秒）', 'stepSeconds', { min: 1, step: 1, placeholder: '自动推断' }),
  ]) fallbacks.appendChild(field);
  card.appendChild(fallbacks);

  const heightsRow = document.createElement('div');
  heightsRow.className = 'table-grid';
  heightsRow.style.marginTop = '10px';
  for (const [key, label] of [
    ['v', '观测高度 V（风速，米）'],
    ['t', '观测高度 T（气温，米）'],
    ['q', '观测高度 Q（湿度，米）'],
  ]) {
    const field = document.createElement('div');
    field.className = 'field';
    const lab = document.createElement('label');
    lab.textContent = label;
    field.appendChild(lab);
    const input = document.createElement('input');
    input.id = `forcing-height-${key}`;
    lab.htmlFor = input.id;
    input.className = 'input';
    input.type = 'number';
    input.step = 'any';
    input.min = '0.000001';
    input.value = heights[key] ?? '';
    input.onchange = () => {
      const value = Number(input.value);
      heights[key] = input.value !== '' && Number.isFinite(value) && value > 0 ? value : null;
      invalidateTableBatch();
      renderCards();
    };
    field.appendChild(input);
    heightsRow.appendChild(field);
  }
  card.appendChild(heightsRow);

  const repairRow = document.createElement('div');
  repairRow.className = 'table-grid';
  repairRow.style.marginTop = '10px';
  for (const [label, key, min] of [
    ['短缺口上限（时间步）', 'shortGap', 0],
    ['订正最少重叠样本', 'minOverlap', 1],
  ]) {
    const field = document.createElement('div');
    field.className = 'field';
    const labelEl = document.createElement('label');
    labelEl.textContent = label;
    field.appendChild(labelEl);
    const input = document.createElement('input');
    input.id = `forcing-gap-${key}`;
    labelEl.htmlFor = input.id;
    input.className = 'input';
    input.type = 'number';
    input.min = String(min);
    input.step = '1';
    input.value = gapSettings[key];
    input.onchange = () => {
      gapSettings[key] = input.value;
      invalidateTableBatch();
      renderCards();
    };
    field.appendChild(input);
    repairRow.appendChild(field);
  }
  card.appendChild(repairRow);

  if (tableProbe.sites.length) {
    const heading = document.createElement('p');
    heading.className = 'muted mini';
    heading.style.marginTop = '12px';
    heading.textContent = '自动识别预览（修改上面的列选择后，以转换时的选择为准）：';
    card.appendChild(heading);
    const table = document.createElement('table');
    table.innerHTML = '<tr><th>站点</th><th>行数</th><th>经纬度</th><th>地类</th><th>推断步长</th><th>缺少整行</th><th>时间范围</th></tr>';
    for (const site of tableProbe.sites) {
      table.appendChild(tableRow([
        { text: site.id },
        { text: String(site.rows) },
        { text: site.latitude != null && site.longitude != null ? `${site.latitude}, ${site.longitude}` : '—' },
        { text: site.landtype ?? '—' },
        { text: site.step_seconds ? `${site.step_seconds} 秒` : '需填写' },
        { text: String(site.inserted_steps) },
        { text: `${site.start ?? '—'} — ${site.end ?? '—'}` },
      ]));
    }
    card.appendChild(table);
  }
  return card;
}

function validOptionalNumber(value, min, max) {
  if (value === '' || value == null) return false;
  const number = Number(value);
  return Number.isFinite(number) && number >= min && number <= max;
}

function tableReadinessReasons() {
  const reasons = [];
  if (!confirmed) reasons.push('先确认槽位映射');
  const missingReq = missingRequiredSlots();
  if (missingReq.length) reasons.push('必需槽位没有映射完整');
  if (missingUnitSlots().length) reasons.push('已选变量缺少源单位');
  const missingHeights = missingForcingHeights(heights);
  if (missingHeights.length) reasons.push(`缺少观测高度：${missingHeights.join('、')}`);
  if (!tableSettings.timeColumn) reasons.push('请选择时间列');
  if (tableProbe.sites.length > 1 && !tableSettings.siteColumn) {
    reasons.push('多个站点必须保留站点名称列，不能合并成一个站点');
  }
  if (tableProbe.sites.length > 1 && (!tableSettings.latitudeColumn || !tableSettings.longitudeColumn)) {
    reasons.push('多个站点必须各自提供纬度列和经度列，不能共用一个回退坐标');
  }
  if (!tableSettings.latitudeColumn && !validOptionalNumber(tableSettings.latitude, -90, 90)) {
    reasons.push('需要纬度列，或为单站表格填写纬度');
  }
  if (!tableSettings.longitudeColumn && !validOptionalNumber(tableSettings.longitude, -180, 180)) {
    reasons.push('需要经度列，或为单站表格填写经度');
  }
  if (!dstDir.trim()) reasons.push('请选择强迫场产物目录');
  if (tableSettings.createSites && !tableSettings.siteDir.trim()) reasons.push('请选择站点数据产物目录');
  if (tableSettings.stepSeconds !== ''
      && (!Number.isInteger(Number(tableSettings.stepSeconds)) || Number(tableSettings.stepSeconds) <= 0)) {
    reasons.push('时间步长必须是正整数秒');
  }
  if (tableSettings.utcOffset !== ''
      && !validOptionalNumber(tableSettings.utcOffset, -12, 14)) {
    reasons.push('人工 UTC 偏移必须在 -12 到 +14 小时之间');
  }
  return reasons;
}

function tableBatchCard() {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>按站点拆分、诊断与修复</h3>
    <div class="ch">先把表格拆成每站一份暂存 NetCDF，再逐站点检查缺测。
      短缺口采用统计插值；长缺口需要 ERA5-Land 对应格点并做偏差订正。
      最终每个站点独立生成 <code>&lt;site-name&gt;_Met.nc</code> 和 QC 标记。</div>
    <div class="field" style="margin-top:12px"><label for="table-forcing-dir">强迫场产物目录</label>
      <div class="browse"><input class="input" id="table-forcing-dir" placeholder="…/Forcing"><button class="btn-ghost" id="table-forcing-pick">选择…</button></div>
    </div>
    <label class="check" style="margin-top:12px"><input type="checkbox" id="table-create-sites"> 同时批量生成或更新站点文件</label>
    <div id="table-site-options"></div>
    <div class="pill-row" style="margin-top:12px"><button class="btn-next" id="table-split">拆分并诊断全部站点</button></div>
    <p class="mini" id="table-why"></p>
    <div id="table-batch-result"></div>`;

  const forcingDir = card.querySelector('#table-forcing-dir');
  forcingDir.value = dstDir;
  forcingDir.onchange = () => {
    dstDir = forcingDir.value.trim();
    invalidateTableBatch();
    renderCards();
  };
  card.querySelector('#table-forcing-pick').onclick = async () => {
    try {
      const picked = await invoke('pick_folder', { key: 'table-forcing-dir' });
      if (!picked) return;
      dstDir = picked;
      invalidateTableBatch();
      renderCards();
    } catch (error) { status(error); }
  };

  const createSites = card.querySelector('#table-create-sites');
  createSites.checked = tableSettings.createSites;
  createSites.onchange = () => {
    tableSettings.createSites = createSites.checked;
    invalidateTableBatch();
    renderCards();
  };
  if (tableSettings.createSites) renderTableSiteOptions(card.querySelector('#table-site-options'));

  const reasons = tableReadinessReasons();
  const split = card.querySelector('#table-split');
  split.disabled = tableBusy || reasons.length > 0;
  split.onclick = splitAndDiagnoseTable;
  const why = card.querySelector('#table-why');
  why.className = (reasons.length ? 'fail' : 'muted') + ' mini';
  why.textContent = reasons.length
    ? reasons.join('；')
    : '就绪：将处理表格里的全部站点，原始 CSV/TXT 不会被修改。';
  if (tableBatch.length) renderTableBatchResult(card.querySelector('#table-batch-result'));
  return card;
}

function renderTableSiteOptions(box) {
  box.innerHTML = `
    <div class="field" style="margin-top:10px"><label for="table-site-dir">站点数据产物目录</label>
      <div class="browse"><input class="input" id="table-site-dir" placeholder="…/Sitedata"><button class="btn-ghost" id="table-site-pick">选择…</button></div>
    </div>
    <div class="field" style="margin-top:10px"><label for="table-rawdata">CoLM rawdata 目录（站点文件有缺项时需要）</label>
      <div class="browse"><input class="input" id="table-rawdata" placeholder="…/rawdata"><button class="btn-ghost" id="table-rawdata-pick">选择…</button></div>
    </div>`;
  const siteDir = box.querySelector('#table-site-dir');
  siteDir.value = tableSettings.siteDir;
  siteDir.onchange = () => {
    tableSettings.siteDir = siteDir.value.trim();
    invalidateTableBatch();
    renderCards();
  };
  const rawdata = box.querySelector('#table-rawdata');
  rawdata.value = tableSettings.rawdata;
  rawdata.onchange = () => {
    tableSettings.rawdata = rawdata.value.trim();
    invalidateTableBatch();
    renderCards();
  };
  box.querySelector('#table-site-pick').onclick = async () => {
    try {
      const picked = await invoke('pick_folder', { key: 'table-site-dir' });
      if (!picked) return;
      tableSettings.siteDir = picked;
      invalidateTableBatch();
      renderCards();
    } catch (error) { status(error); }
  };
  box.querySelector('#table-rawdata-pick').onclick = async () => {
    try {
      const picked = await invoke('pick_folder', { key: 'table-rawdata' });
      if (!picked) return;
      tableSettings.rawdata = picked;
      invalidateTableBatch();
      renderCards();
    } catch (error) { status(error); }
  };
}

function canonicalTableSlots() {
  return selectedSlots().map(slot => ({
    index: slot.index,
    name: TABLE_CANONICAL_VARIABLES[slot.index],
    units: probe.slots.find(candidate => candidate.index === slot.index)?.wants ?? slot.units,
    also_add: [],
  }));
}

function tableImportOptions() {
  const numberOrNull = value => value === '' ? null : optionalNumber(value);
  const optionalText = value => value?.trim() || null;
  return {
    time_column: tableSettings.timeColumn,
    site_column: optionalText(tableSettings.siteColumn),
    latitude_column: optionalText(tableSettings.latitudeColumn),
    longitude_column: optionalText(tableSettings.longitudeColumn),
    landtype_column: optionalText(tableSettings.landtypeColumn),
    land_cover_scheme: prepMode(state),
    utc_offset_column: optionalText(tableSettings.offsetColumn),
    utc_offset: numberOrNull(tableSettings.utcOffset),
    latitude: numberOrNull(tableSettings.latitude),
    longitude: numberOrNull(tableSettings.longitude),
    step_seconds: tableSettings.stepSeconds === '' ? null : Math.trunc(Number(tableSettings.stepSeconds)),
    heights: [Number(heights.v), Number(heights.t), Number(heights.q)],
  };
}

function tableGapOptions(item, includeEra5) {
  return {
    short_gap: Math.max(0, Math.trunc(Number(gapSettings.shortGap) || 0)),
    // 拆分文件的时间轴已由导入器转成 UTC，不能再次应用原表时区。
    utc_offset: 0,
    latitude: item.latitude,
    longitude: item.longitude,
    era5: includeEra5 && item.report?.needs_era5 && gapSettings.era5.trim()
      ? gapSettings.era5.trim() : null,
    min_overlap: Math.max(1, Math.trunc(Number(gapSettings.minOverlap) || 24)),
  };
}

async function runPool(items, limit, operation) {
  let next = 0;
  const workers = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const index = next++;
      await operation(items[index], index);
    }
  });
  const results = await Promise.allSettled(workers);
  const failure = results.find(result => result.status === 'rejected');
  if (failure) throw failure.reason;
}

async function splitAndDiagnoseTable() {
  const reasons = tableReadinessReasons();
  if (reasons.length || tableBusy) { status(reasons.join('；')); return; }
  tableBusy = true;
  tableBatch = [];
  resetBatchArtifacts();
  renderCards();
  try {
    status('正在按站点拆分 CSV/TXT…');
    const imported = await invoke('convert_forcing_table', {
      src: srcPath,
      dst: dstDir.trim(),
      slots: selectedSlots(),
      options: tableImportOptions(),
    });
    tableBatch = imported.map(item => ({ ...item, phase: '诊断中', report: null, error: null, siteReport: null }));
    let finished = 0;
    await runPool(tableBatch, 4, async item => {
      try {
        item.report = await invoke('probe_forcing_gaps', {
          src: item.staged_path,
          slots: canonicalTableSlots(),
          options: tableGapOptions(item, false),
        });
        item.phase = item.report.needs_era5 ? '需要 ERA5-Land' : '可修复';
      } catch (error) {
        item.error = String(error);
        item.phase = '诊断失败';
      }
      finished += 1;
      status(`正在诊断站点 ${finished}/${tableBatch.length}：${item.site}`);
    });
    const failed = tableBatch.filter(item => item.error).length;
    status(failed
      ? `已拆分 ${tableBatch.length} 个站点，其中 ${failed} 个诊断失败`
      : `已拆分并诊断 ${tableBatch.length} 个站点`);
  } catch (error) {
    tableBatch = [];
    status(error);
  } finally {
    tableBusy = false;
    renderCards();
  }
}

function renderTableBatchResult(box) {
  const needsEra5 = tableBatch.some(item => item.report?.needs_era5);
  const failed = tableBatch.some(item => item.error);
  const allComplete = tableBatch.every(item => item.phase === '完成');
  const table = document.createElement('table');
  table.style.marginTop = '14px';
  table.innerHTML = '<tr><th>站点</th><th>状态</th><th>原始行</th><th>补入时间步</th><th>缺测/不合格</th><th>QC 剔除</th><th>ERA5-Land</th><th>产物</th></tr>';
  for (const item of tableBatch) {
    table.appendChild(tableRow([
      { text: item.site },
      {
        text: item.error ? `${item.phase}：${item.error}` : item.phase,
        className: item.error ? 'fail' : (item.report?.needs_era5 ? 'warn' : ''),
      },
      { text: String(item.rows) },
      { text: String(item.inserted_steps) },
      { text: item.report ? String(item.report.missing) : '—' },
      {
        text: item.report
          ? String(item.report.variables.reduce((sum, variable) => sum + variable.quality_rejected, 0)) : '—',
      },
      { text: item.report?.needs_era5 ? '需要' : '不需要' },
      { text: item.phase === '完成' ? item.final_path : '—' },
    ]));
  }
  box.appendChild(table);

  if (needsEra5) {
    if (!gapSettings.era5 && dstDir) gapSettings.era5 = joinPath(dstDir, '.era5land');
    const field = document.createElement('div');
    field.className = 'field';
    field.style.marginTop = '12px';
    field.innerHTML = '<label for="table-era5">全部站点共用的 ERA5-Land 缓存目录</label><div class="browse"><input class="input" id="table-era5" placeholder="…/ERA5-Land"><button class="btn-ghost" id="table-era5-pick">选择…</button></div>';
    const input = field.querySelector('#table-era5');
    input.value = gapSettings.era5;
    input.onchange = () => {
      gapSettings.era5 = input.value.trim();
      renderCards();
    };
    field.querySelector('#table-era5-pick').onclick = async () => {
      try {
        const picked = await invoke('pick_folder', { key: 'table-era5' });
        if (!picked) return;
        gapSettings.era5 = picked;
        renderCards();
      } catch (error) { status(error); }
    };
    box.appendChild(field);
    const queueNote = document.createElement('p');
    queueNote.className = 'muted mini';
    queueNote.textContent = '各站点按格点共享缓存并依次提交；CDS 服务器可能排队，长序列请等待，不要重复点击。';
    box.appendChild(queueNote);
  }

  const bar = document.createElement('div');
  bar.className = 'pill-row';
  bar.style.marginTop = '12px';
  if (needsEra5) {
    const download = document.createElement('button');
    download.className = 'btn-ghost';
    download.textContent = '下载全部缺失站点的 ERA5-Land';
    download.disabled = tableBusy || !gapSettings.era5.trim() || failed;
    download.onclick = downloadTableEra5;
    bar.appendChild(download);
  }
  const repair = document.createElement('button');
  repair.className = 'btn-next';
  repair.textContent = allComplete
    ? '全部站点已完成'
    : (tableSettings.createSites ? '修复并生成全部站点数据' : '修复并生成全部强迫场');
  repair.disabled = tableBusy || allComplete || failed || (needsEra5 && !gapSettings.era5.trim());
  repair.onclick = repairTableBatch;
  bar.appendChild(repair);
  box.appendChild(bar);
}

async function downloadTableEra5() {
  const targets = tableBatch.filter(item => item.report?.needs_era5 && !item.error);
  if (!targets.length || !gapSettings.era5.trim() || tableBusy) return;
  tableBusy = true;
  renderCards();
  let finished = 0;
  try {
    // 同一 ERA5 格点可能被多个站点复用；串行下载避免两个 sidecar 同时写
    // 同一个缓存文件。逐站点诊断和修复仍保持最多 4 个并发。
    await runPool(targets, 1, async item => {
      status(`正在提交 ${finished + 1}/${targets.length}：${item.site}；CDS 服务器可能排队，请等待…`);
      await invoke('download_era5land', {
        dst: gapSettings.era5.trim(),
        latitude: item.report.latitude,
        longitude: item.report.longitude,
        start: item.report.start_date,
        end: item.report.end_date,
      });
      finished += 1;
      status(`ERA5-Land 下载 ${finished}/${targets.length}：${item.site}`);
    });
    status(`已缓存 ${targets.length} 个站点需要的 ERA5-Land 数据`);
  } catch (error) { showEra5DownloadError(error); }
  finally { tableBusy = false; renderCards(); }
}

function showEra5DownloadError(error) {
  const message = String(error);
  status(message);
  if (message.includes('CDS API 配置')) globalThis.alert?.(message);
}

async function repairTableBatch() {
  if (!tableBatch.length || tableBusy) return;
  tableBusy = true;
  renderCards();
  let finished = 0;
  try {
    await runPool(tableBatch, 4, async item => {
      try {
        if (tableSettings.createSites) {
          item.phase = '生成站点文件';
          item.siteFinalPath = joinPath(tableSettings.siteDir, siteOutputName(item.safe_site));
          item.siteStagedPath = joinPath(tableSettings.siteDir, `.${item.safe_site}.colm-site-stage`);
          item.forcingStagedPath = joinPath(dstDir.trim(), `.${item.safe_site}.colm-forcing-stage`);
          item.siteReport = await invoke('make_site', {
            out: item.siteStagedPath,
            lon: item.longitude,
            lat: item.latitude,
            landtype: item.landtype,
            rawdata: tableSettings.rawdata || null,
            mode: prepMode(state),
            crop: !!state.wizard?.physics?.crop,
          });
          if (item.siteReport.readiness === 'blocked') {
            throw new Error(`站点数据仍缺 ${item.siteReport.needs_external.length} 项当前模式必需数据`);
          }
        }
        item.phase = '修复中';
        item.report = await invoke('repair_forcing', {
          src: item.staged_path,
          dst: tableSettings.createSites ? item.forcingStagedPath : item.final_path,
          slots: canonicalTableSlots(),
          options: tableGapOptions(item, true),
        });
        if (item.report.unresolved) {
          throw new Error(`仍有 ${item.report.unresolved} 个缺测值没有解决`);
        }
        if (tableSettings.createSites) {
          await invoke('install_prepared_pair', {
            siteStaged: item.siteStagedPath,
            siteFinal: item.siteFinalPath,
            forcingStaged: item.forcingStagedPath,
            forcingFinal: item.final_path,
          });
          item.siteReport.path = item.siteFinalPath;
        }
        item.phase = '完成';
        item.error = null;
      } catch (error) {
        item.error = String(error);
        item.phase = '处理失败';
      }
      finished += 1;
      status(`正在生成站点产物 ${finished}/${tableBatch.length}：${item.site}`);
    });
    const completed = tableBatch.filter(item => item.phase === '完成');
    const failed = tableBatch.length - completed.length;
    $('forcingdir').value = dstDir.trim();
    Object.assign(state.prepArtifacts, {
      forcingDir: dstDir.trim(),
      forcingFile: completed.length === 1 ? completed[0].final_path : null,
      siteDir: tableSettings.createSites ? tableSettings.siteDir : state.prepArtifacts.siteDir,
      siteFile: tableSettings.createSites && completed.length === 1 ? completed[0].siteReport?.path ?? null : null,
      siteReport: tableSettings.createSites && completed.length === 1 ? completed[0].siteReport : null,
      rawdataDir: tableSettings.rawdata || null,
      batchSites: tableBatch.map(item => ({
        site: item.site,
        siteFile: item.phase === '完成' ? item.siteReport?.path ?? null : null,
        siteReport: item.phase === '完成' ? item.siteReport : null,
        forcingFile: item.phase === '完成' ? item.final_path : null,
        error: item.error,
      })),
    });
    if (tableSettings.createSites) {
      $('sitedir').value = tableSettings.siteDir;
      if ($('rawdata')) $('rawdata').value = tableSettings.rawdata;
      await scanPreparedSites();
    }
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    status(failed
      ? `批量处理完成：${completed.length}/${tableBatch.length} 个站点成功`
      : `批量处理完成：${completed.length} 个站点均已生成`);
  } catch (error) {
    status(error);
  } finally {
    tableBusy = false;
    renderCards();
  }
}

// -------------------------------------------------------- ③ 时间轴与高度

function timingCard() {
  const card = document.createElement('div');
  card.className = 'card';
  const uniformWarn = !probe.step_uniform;
  card.innerHTML = `
    <h3>时间轴与观测高度</h3>
    <div class="ch">步长与观测高度会写进产物；模拟用哪一段时间范围仍以强迫场
      覆盖范围为准，由建例时自动确定，不需要手动填写。</div>
    <table></table>`;
  const table = card.querySelector('table');
  table.append(
    tableRow([{ header: true, text: '步长' }, { text: `${probe.step_seconds} 秒` }]),
    tableRow([{ header: true, text: '步数' }, { text: String(probe.steps) }]),
    tableRow([
      { header: true, text: '是否等间隔' },
      { text: uniformWarn ? '不是 —— 重采样不在这一阶段，请先自己处理' : '是', className: uniformWarn ? 'warn' : '' },
    ]),
  );
  const row = document.createElement('div');
  row.className = 'row';
  row.style.marginTop = '12px';
  for (const [key, label] of [
    ['v', '观测高度 V（风速，米）'],
    ['t', '观测高度 T（气温，米）'],
    ['q', '观测高度 Q（湿度，米）'],
  ]) {
    const f = document.createElement('div');
    f.className = 'field';
    const lab = document.createElement('label');
    lab.textContent = label;
    f.appendChild(lab);
    const inp = document.createElement('input');
    inp.id = `forcing-netcdf-height-${key}`;
    lab.htmlFor = inp.id;
    inp.className = 'input';
    inp.type = 'number';
    inp.step = 'any';
    inp.min = '0.000001';
    inp.value = heights[key] ?? '';
    inp.onchange = () => {
      const n = parseFloat(inp.value);
      heights[key] = Number.isFinite(n) && n > 0 ? n : null;
      renderCards();
    };
    f.appendChild(inp);
    row.appendChild(f);
  }
  card.appendChild(row);
  if (heights.v == null || heights.t == null || heights.q == null) {
    const note = document.createElement('div');
    note.className = 'expert-note';
    note.innerHTML =
      'CoLM 要观测高度填 <code>DEF_forcing%HEIGHT_V/T/Q</code>。这份文件里没有，' +
      '不填的话模型会拿到 <b>NaN</b> 然后直接崩，而报出来的错看不出是这里的问题。';
    card.appendChild(note);
  }
  return card;
}

// ----------------------------------------------- ④ 缺测诊断、时区与 ERA5-Land

function gapCard() {
  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>缺测诊断与修复</h3>
    <div class="ch">先把源单位转换为 CoLM 标准单位并做宽松物理范围 QC；不合格值按缺测处理。
      短缺口按变量类型插值；长缺口在把站点时间换算到 UTC 后，
      读取 ERA5-Land 最近 0.1° 格点，并只用观测重叠期做偏差订正。原始文件不会被覆盖，
      产物逐时记录观测、插值或 ERA5-Land 来源。</div>
    <p class="muted mini">QC 范围：气温 180–350 K、比湿 0–0.1 kg/kg、气压 30–110 kPa、降水 0–0.1 kg/m²/s、风速/分量不超过 100 m/s、短波 0–1800 W/m²、长波 0–800 W/m²。</p>
    <div class="row" style="margin-top:12px">
      <div class="field"><label for="gap-short">短缺口上限（时间步）</label><input class="input" id="gap-short" type="number" min="0" step="1"></div>
      <div class="field"><label for="gap-overlap">订正最少重叠样本</label><input class="input" id="gap-overlap" type="number" min="1" step="1"></div>
    </div>
    <div class="row" style="margin-top:10px">
      <div class="field"><label for="gap-lat">站点纬度</label><input class="input" id="gap-lat" type="number" min="-90" max="90" step="any" placeholder="优先读取文件"></div>
      <div class="field"><label for="gap-lon">站点经度</label><input class="input" id="gap-lon" type="number" min="-180" max="180" step="any" placeholder="优先读取文件"></div>
      <div class="field"><label for="gap-offset">人工 UTC 偏移（小时）</label><input class="input" id="gap-offset" type="number" min="-12" max="14" step="0.25" placeholder="自动判断"></div>
    </div>
    <div class="pill-row" style="margin-top:12px"><button class="btn-ghost" id="gap-probe">诊断缺测与时区</button></div>
    <div id="gap-result"></div>`;

  const bind = (selector, key, fallback = '') => {
    const input = card.querySelector(selector);
    input.value = gapSettings[key] ?? fallback;
    input.onchange = () => {
      gapSettings[key] = input.value;
      invalidateGap();
      renderCards();
    };
  };
  bind('#gap-short', 'shortGap', 3);
  bind('#gap-overlap', 'minOverlap', 24);
  bind('#gap-lat', 'latitude');
  bind('#gap-lon', 'longitude');
  bind('#gap-offset', 'utcOffset');
  card.querySelector('#gap-probe').onclick = diagnoseGaps;

  const result = card.querySelector('#gap-result');
  if (gapReport) result.appendChild(gapReportView());
  return card;
}

function gapReportView() {
  const box = document.createElement('div');
  const qualityRejected = gapReport.variables.reduce((sum, row) => sum + row.quality_rejected, 0);
  const timezoneLabels = {
    manual_override: '人工覆盖',
    file_metadata: '文件元数据',
    solar_noon_confirmed_utc: '短波辐射太阳正午确认 UTC',
    solar_noon_inferred_offset: '短波辐射太阳正午推断',
    longitude_inferred_offset: '按经度推断（不是行政时区）',
  };
  const confidenceLabels = { high: '高', medium: '中', low: '低' };
  const solarEvidence = gapReport.solar_noon_hour == null
    ? '无可用太阳正午证据'
    : `${gapReport.solar_noon_hour.toFixed(2)} 时（逐日标准差 ${gapReport.solar_noon_std_hours.toFixed(2)} 小时）`;
  const summary = document.createElement('table');
  summary.style.marginTop = '12px';
  summary.append(
    tableRow([
      { header: true, text: 'UTC 偏移' },
      { text: `UTC${gapReport.timezone_offset_hours >= 0 ? '+' : ''}${gapReport.timezone_offset_hours} · ${timezoneLabels[gapReport.timezone_source] ?? gapReport.timezone_source}` },
    ]),
    tableRow([
      { header: true, text: '时区证据' },
      {
        text: `${confidenceLabels[gapReport.timezone_confidence] ?? gapReport.timezone_confidence}置信度 · ${solarEvidence}${gapReport.timezone_conflict ? ' · 与人工/文件声明冲突' : ''}`,
        className: gapReport.timezone_conflict ? 'warn' : '',
      },
    ]),
    tableRow([{ header: true, text: 'ERA5-Land 格点定位' }, { text: `${gapReport.latitude}, ${gapReport.longitude}` }]),
    tableRow([{ header: true, text: '数据范围（UTC 日期）' }, { text: `${gapReport.start_date} — ${gapReport.end_date}` }]),
    tableRow([{ header: true, text: '缺测/不合格总数' }, { text: String(gapReport.missing), className: gapReport.missing ? 'warn' : '' }]),
    tableRow([{ header: true, text: '其中 QC 剔除' }, { text: String(qualityRejected), className: qualityRejected ? 'warn' : '' }]),
  );
  box.appendChild(summary);
  const table = document.createElement('table');
  table.style.marginTop = '10px';
  table.innerHTML = '<tr><th>槽位</th><th>变量</th><th>缺测/不合格</th><th>QC 剔除</th><th>短缺口</th><th>需 ERA5</th><th>最长</th><th>已插值</th><th>ERA5-Land</th></tr>';
  box.appendChild(table);
  for (const row of gapReport.variables) {
    table.appendChild(tableRow([
      { text: String(row.slot) },
      { text: row.variable },
      { text: String(row.missing) },
      { text: String(row.quality_rejected) },
      { text: String(row.short_missing) },
      { text: String(row.long_missing), className: row.long_missing ? 'warn' : '' },
      { text: String(row.longest_gap) },
      { text: String(row.interpolated) },
      { text: String(row.era5_corrected) },
    ]));
  }

  if (gapReport.missing === 0) {
    const ready = document.createElement('p');
    ready.className = 'muted mini';
    ready.textContent = '没有缺测，原文件可直接进入标准化转换；时区判定仍会保留在诊断记录中。';
    box.appendChild(ready);
    return box;
  }

  if (gapReport.needs_era5) {
    const field = document.createElement('div');
    field.className = 'field';
    field.style.marginTop = '12px';
    field.innerHTML = `<label for="gap-era5">ERA5-Land 缓存目录</label><div class="browse"><input class="input" id="gap-era5" placeholder="…/ERA5-Land"><button class="btn-ghost" id="gap-era5-pick">选择…</button></div>`;
    const input = field.querySelector('#gap-era5');
    if (!gapSettings.era5 && dstDir) gapSettings.era5 = joinPath(dstDir, '.era5land');
    input.value = gapSettings.era5;
    input.onchange = () => {
      gapSettings.era5 = input.value.trim();
      repairedSource = null;
      renderCards();
    };
    field.querySelector('#gap-era5-pick').onclick = async () => {
      try {
        const picked = await invoke('pick_folder', { key: 'gap-era5' });
        if (!picked) return;
        gapSettings.era5 = picked;
        repairedSource = null;
        renderCards();
      } catch (error) { status(error); }
    };
    box.appendChild(field);
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = '可选择已有 ERA5-Land NetCDF 缓存；也可用本机 CDS API 一次下载该站点完整时间段。CDS 服务器可能排队，长序列请等待且不要重复点击。下载前需配置 ~/.cdsapirc 并接受数据许可。';
    box.appendChild(note);
  }

  const bar = document.createElement('div');
  bar.className = 'pill-row';
  bar.style.marginTop = '10px';
  if (gapReport.needs_era5) {
    const download = document.createElement('button');
    download.className = 'btn-ghost';
    download.textContent = era5Busy ? 'CDS 排队或下载中…' : '下载对应 ERA5-Land 格点';
    download.disabled = era5Busy || !gapSettings.era5.trim();
    download.onclick = downloadEra5;
    bar.appendChild(download);
  }
  const repair = document.createElement('button');
  repair.className = 'btn-next';
  repair.textContent = '生成已修复中间文件';
  repair.disabled = era5Busy || (gapReport.needs_era5 && !gapSettings.era5.trim());
  repair.onclick = repairGaps;
  bar.appendChild(repair);
  box.appendChild(bar);
  if (repairedSource) {
    const done = document.createElement('p');
    done.className = 'mini';
    done.append('修复完成：');
    const code = document.createElement('code');
    code.textContent = repairedSource;
    done.appendChild(code);
    box.appendChild(done);
  }
  return box;
}

async function diagnoseGaps() {
  if (!confirmed) { status('先确认槽位映射，再诊断缺测'); return; }
  const missingUnits = missingUnitSlots();
  if (missingUnits.length) { status('先补齐所有已选变量的源单位'); return; }
  try {
    gapReport = await invoke('probe_forcing_gaps', {
      src: srcPath,
      slots: selectedSlots(),
      options: gapOptions(false),
    });
    repairedSource = null;
    status(gapReport.missing ? `发现 ${gapReport.missing} 个缺测或 QC 不合格值` : '缺测与基础 QC 均通过');
  } catch (error) {
    gapReport = null;
    status(error);
  }
  renderCards();
}

async function downloadEra5() {
  if (!gapReport || !gapSettings.era5.trim() || era5Busy) return;
  era5Busy = true;
  renderCards();
  try {
    status('ERA5-Land 请求已提交；CDS 服务器可能排队，长时间序列请耐心等待…');
    await invoke('download_era5land', {
      dst: gapSettings.era5.trim(),
      latitude: gapReport.latitude,
      longitude: gapReport.longitude,
      start: gapReport.start_date,
      end: gapReport.end_date,
    });
    status('ERA5-Land 对应格点已缓存，可以生成修复文件');
  } catch (error) { showEra5DownloadError(error); }
  finally { era5Busy = false; renderCards(); }
}

async function repairGaps() {
  if (!gapReport || !dstDir.trim()) { status('先诊断缺测并填写产物目录'); return; }
  const stem = state.prepArtifacts.siteStem;
  if (!stem) { status('先生成站点文件'); return; }
  const repaired = joinPath(joinPath(dstDir.trim(), '.colm-gapfill'), `${stem}_Met_repaired.nc`);
  try {
    gapReport = await invoke('repair_forcing', {
      src: srcPath,
      dst: repaired,
      slots: selectedSlots(),
      options: gapOptions(true),
    });
    if (gapReport.unresolved) throw new Error(`仍有 ${gapReport.unresolved} 个缺测值没有解决`);
    repairedSource = repaired;
    status('缺测与 QC 修复完成，逐时来源已写入 *_gapfill_qc');
  } catch (error) {
    repairedSource = null;
    status(error);
  }
  renderCards();
}

// --------------------------------------------------------------- ⑤ 转换

function convertCard() {
  const card = document.createElement('div');
  card.className = 'card';
  const reasons = [];
  if (!confirmed) reasons.push('先在上面「槽位映射」卡片点一次「这些映射我看过了」');
  const missingReq = missingRequiredSlots();
  if (missingReq.length) {
    reasons.push('必需槽位还没选变量：' + missingReq.map(({ s }) => `第 ${s.index} 槽`).join('、'));
  }
  const missingU = missingUnitSlots();
  if (missingU.length) {
    reasons.push('选了变量但没填源单位：' + missingU.map(({ s }) => `第 ${s.index} 槽`).join('、'));
  }
  const missingHeights = missingForcingHeights(heights);
  if (missingHeights.length) {
    reasons.push(`缺少观测高度：${missingHeights.join('、')}`);
  }
  if (!state.prepArtifacts.siteStem) reasons.push('先在“站点数据”子步骤填写站点名并生成站点文件');
  if (!dstDir.trim()) reasons.push('先填写强迫场产物目录');
  if (!gapReport) reasons.push('先完成缺测与时区诊断');
  else if (gapReport.missing > 0 && !repairedSource) reasons.push('先生成已修复中间文件');
  else if (gapReport.unresolved > 0) reasons.push(`仍有 ${gapReport.unresolved} 个缺测值未解决`);

  card.innerHTML = `
    <h3>转换</h3>
    <div class="ch">按上面确认过的映射写出一份 CoLM 认的标准文件。
      <b>产物目录不能与源文件所在目录相同</b> —— 原始数据要原样留着，
      选了同一个目录后端会直接拒绝。</div>
    <div class="browse"><input class="input" id="fdst" placeholder="…/converted"></div>
    <p class="muted mini" id="fdst-note"></p>
    <div class="pill-row" style="margin-top:12px">
      <button class="btn-ghost" id="fconvert">转换</button>
    </div>
    <p class="mini" id="fconvert-why"></p>
    <div id="fconvert-result"></div>`;

  const dstInp = card.querySelector('#fdst');
  dstInp.value = dstDir;
  dstInp.onchange = () => { dstDir = dstInp.value.trim(); renderCards(); };
  card.querySelector('#fdst-note').textContent =
    state.prepArtifacts.siteStem
      ? `标准文件名：${forcingOutputName(state.prepArtifacts.siteStem)}，可与站点文件自动配对。`
      : '先生成站点文件，强迫场将沿用同一个站点名。';

  const btn = card.querySelector('#fconvert');
  btn.disabled = reasons.length > 0;
  btn.onclick = doConvert;
  const why = card.querySelector('#fconvert-why');
  why.className = (reasons.length ? 'fail' : 'muted') + ' mini';
  why.textContent = reasons.length ? reasons.join('；') : '就绪，可以转换。';

  const resultBox = card.querySelector('#fconvert-result');
  if (lastResult) {
    const p1 = document.createElement('p');
    p1.className = 'mini';
    p1.style.marginTop = '10px';
    const code = document.createElement('code');
    code.textContent = lastResult;
    p1.append('已转换：', code);
    const p2 = document.createElement('p');
    p2.className = 'muted mini';
    p2.textContent = '已自动写入基本设定的强迫场目录，并与刚生成的站点重新配对。';
    resultBox.appendChild(p1);
    resultBox.appendChild(p2);
  }
  return card;
}

async function ensureRepairedSource(stem) {
  if (repairedSource) return repairedSource;
  const repaired = joinPath(joinPath(dstDir.trim(), '.colm-gapfill'), `${stem}_Met_repaired.nc`);
  gapReport = await invoke('repair_forcing', {
    src: srcPath,
    dst: repaired,
    slots: selectedSlots(),
    options: gapOptions(true),
  });
  if (gapReport.unresolved) throw new Error(`仍有 ${gapReport.unresolved} 个缺测值没有解决`);
  repairedSource = repaired;
  return repairedSource;
}

async function doConvert() {
  const dir = $('fdst').value.trim();
  if (!dir) { status('先填产物放哪个目录'); return; }
  const stem = state.prepArtifacts.siteStem;
  if (!stem) { status('先在“站点数据”子步骤生成站点文件'); return; }
  const missingHeights = missingForcingHeights(heights);
  if (missingHeights.length) { status(`先补齐观测高度：${missingHeights.join('、')}`); return; }
  const dst = joinPath(dir, forcingOutputName(stem));
  const btn = $('fconvert');
  if (btn) btn.disabled = true;
  try {
    const slots = selectedSlots();
    const sourceForConvert = await ensureRepairedSource(stem);
    const heightsReady = heights.v != null && heights.t != null && heights.q != null;
    const heightsArg = heightsReady ? [heights.v, heights.t, heights.q] : null;
    lastResult = await invoke('convert_forcing', {
      src: sourceForConvert,
      dst,
      slots,
      heights: heightsArg,
    });
    Object.assign(state.prepArtifacts, { forcingFile: lastResult, forcingDir: dir });
    $('forcingdir').value = dir;
    if (state.prepArtifacts.siteFile) {
      const selected = await scanPreparedSites(state.prepArtifacts.siteFile);
      if (!selected?.met_file) throw new Error('转换已写出，但重新扫描未能把站点与强迫场配对；请检查强迫场目录');
    }
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    status('转换完成：' + lastResult);
  } catch (e) {
    lastResult = null;
    Object.assign(state.prepArtifacts, { forcingFile: null, forcingDir: null });
    globalThis.dispatchEvent?.(new Event('colm:prep-artifacts'));
    status(e);
  } finally {
    renderCards();
  }
}
