//! 结果分析工作台。
//!
//! 参数编辑用 `state.selected`，结果浏览用 `state.resultCaseDir`。两者故意分开：
//! 切一张图不该把 20 个算例的参数编辑目标悄悄换成其中一个。

import { hasBackend, invoke, listen } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { sourceSite } from './batch.js';
import { go, renderSteps } from './shell.js';
import { metricText } from './metric-format.js';
import { language, translateZh } from './i18n.js';
import { fieldLabel } from './param-presentation.js';
import { aggregateStudy, MAX_STUDY_CANDIDATES, paginate, replaceScopedStudyDirs, scopedStudyDirs, studyBudget, studySiteId } from './study-model.js';
import {
  LruCache, METRIC_META, boundedMap, finite, metricKey, ranking, resultCases,
  rowsToCsv, seriesKey, seriesStats,
} from './result-model.js';

const COMMON_VARIABLES = {
  f_rnet: ['净辐射 Rnet', 'W/m²', '能量'],
  f_fsena: ['感热 Qh', 'W/m²', '能量'],
  f_lfevpa: ['潜热 Qle', 'W/m²', '能量'],
  f_fgrnd: ['地表热通量 Qg', 'W/m²', '能量'],
  f_sr: ['反射短波 SWup', 'W/m²', '辐射'],
  f_xy_t: ['参考高度气温', 'K', '大气'],
  f_t_soisno: ['土壤与积雪温度', 'K', '土壤/积雪'],
  f_wliq_soisno: ['液态土壤水', 'kg/m²', '土壤/积雪'],
  f_wice_soisno: ['土壤冰', 'kg/m²', '土壤/积雪'],
  f_rnof: ['总产流', 'mm/s', '水文'],
  f_zwt: ['地下水位', 'm', '水文'],
  f_qinfl: ['入渗', 'mm/s', '水文'],
  f_qover: ['地表产流', 'mm/s', '水文'],
  f_qintr: ['冠层截留', 'mm/s', '植被'],
  f_etr: ['蒸腾', 'mm/s', '植被'],
  f_evplwet: ['湿冠层蒸发', 'mm/s', '植被'],
  f_vegt: ['植被温度', 'K', '植被'],
  f_lai: ['叶面积指数', 'm²/m²', '植被'],
  f_sai: ['茎面积指数', 'm²/m²', '植被'],
  f_assim: ['光合作用', 'µmol/m²/s', '碳循环'],
  f_respc: ['植物呼吸', 'µmol/m²/s', '碳循环'],
  f_gpp: ['总初级生产力 GPP', 'gC/m²/s', '碳循环'],
  f_grainc: ['籽粒碳库', 'gC/m²', '作物'],
  f_cropprod1c: ['一年期作物产品碳库', 'gC/m²', '作物'],
  f_cropprod1c_loss: ['作物产品碳损失', 'gC/m²/s', '作物'],
  f_cropseedc_deficit: ['作物种子碳亏缺', 'gC/m²/s', '作物'],
  f_cropprodc_rainfed_temp_corn: ['雨养温带玉米产量碳', 'gC/m²/s', '作物'],
  f_plantdate_rainfed_temp_corn: ['雨养温带玉米播种日', 'day', '作物'],
  f_gddplant: ['播种后积温', 'degree days', '作物'],
  f_gddmaturity: ['成熟所需积温', 'degree days', '作物'],
  f_hui: ['热量单位指数 HUI', '—', '作物'],
  f_fert_to_sminn: ['施肥氮输入', 'gN/m²/s', '作物'],
  f_methane_surf_flux_tot: ['甲烷总地表通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_tot_active: ['活动地表甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_tot_phys: ['物理地表甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_soil: ['土壤甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_wetland: ['湿地甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_lake: ['湖泊甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_rice: ['稻田甲烷通量', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_global_total_with_lake: ['全地表平均甲烷通量（含湖泊）', 'mol/m²/s', '甲烷'],
  f_methane_surf_flux_global_phys_with_lake: ['全地表平均物理甲烷通量（含湖泊）', 'mol/m²/s', '甲烷'],
  f_methane_prod_tot: ['甲烷总产生率', 'mol/m²/s', '甲烷'],
  f_methane_oxid_tot: ['甲烷总氧化率', 'mol/m²/s', '甲烷'],
  f_totcol_methane: ['土柱甲烷储量', 'mol/m²', '甲烷'],
  f_methane_balance_residual: ['甲烷质量平衡残差', 'mol/m²/s', '甲烷'],
  f_methane_balance_residual_global_with_lake: ['全地表平均甲烷质量平衡残差（含湖泊）', 'mol/m²/s', '甲烷'],
  f_methane_ch4_clip_credit: ['甲烷数值截断修正', 'mol/m²/s', '甲烷'],
  f_methane_ch4_clip_credit_global_with_lake: ['全地表平均甲烷数值截断修正（含湖泊）', 'mol/m²/s', '甲烷'],
  f_o2_cap_gain: ['氧气上限修正增益', 'mol/m²/s', '甲烷'],
  f_o2_cap_loss: ['氧气上限修正损失', 'mol/m²/s', '甲烷'],
  f_CONC_O2_UNSAT: ['非淹水区土壤氧气浓度', 'mol/m³', '甲烷'],
  f_O2_DECOMP_DEPTH_UNSAT: ['非淹水区土壤耗氧率', 'mol/m³/s', '甲烷'],
  f_fach: ['空调制冷显热', 'W/m²', '城市'],
  f_fhac: ['空调供热显热', 'W/m²', '城市'],
  f_fsenroof: ['屋顶感热通量', 'W/m²', '城市'],
  f_fvehc: ['车辆人为热', 'W/m²', '城市'],
  f_lfevproof: ['屋顶潜热通量', 'W/m²', '城市'],
  f_t_roof: ['屋顶温度', 'K', '城市'],
  f_t_room: ['室内空气温度', 'K', '城市'],
  f_t_wall: ['墙体温度', 'K', '城市'],
  f_xy_snow: ['降雪', 'mm/s', '积雪'],
  f_snowdp: ['雪深', 'm', '积雪'],
  f_scv: ['雪水当量', 'kg/m²', '积雪'],
};
const PLANNED_PROFILE_VARIABLES = new Set(['f_t_soisno', 'f_wliq_soisno', 'f_wice_soisno']);

const catalogCache = new LruCache(16);
const evaluationCatalogCache = new LruCache(64);
const seriesCache = new LruCache(12);
const dialogText = text => (language() === 'en' ? translateZh(text) : text);
const metricsCache = new LruCache(180);
const charts = new Map();
let currentMetricRows = [];
let currentComparisonSummary = null;
let currentEvaluationCatalog = [];
let batchEvaluationCatalogs = [];
let batchEvaluationCatalogFailures = [];
let comparisonController = null;
let activeSeriesRequest = 0;
let activeMetricRequest = 0;
let activeMetricChartRequest = 0;
let activePaneRequest = 0;
let activeDataBrowserRequest = 0;
let activeBatchEvaluationCatalogRequest = 0;
let reportText = '';
let reportExtension = 'md';

const node = (tag, cls = '', text = '') => {
  const element = document.createElement(tag);
  if (cls) element.className = cls;
  if (text !== '') element.textContent = String(text);
  return element;
};
const td = (text, cls = '') => node('td', cls, text);
const th = (text, cls = '') => node('th', cls, text);
const badge = (text, kind = '') => node('span', `result-badge ${kind}`.trim(), text);
const historyHealth = new Map();
const isStaleResult = c => state.runState[c.dir] === '需重跑';
const isActiveResult = c => ['待运行', '运行中'].includes(state.runState[c.dir]);
const hasValidatedHistory = c => c.has_history && !isStaleResult(c) && !isActiveResult(c)
  && historyHealth.get(c.dir)?.ok === true;
const completed = () => resultCases(state.cases, state.createdCases, false).filter(hasValidatedHistory);
const allCurrent = () => resultCases(state.cases, state.createdCases, false);
const resultScope = () => {
  const done = completed();
  if (!state.resultSelectionTouched) return done;
  return done.filter(c => state.resultSelection.has(c.dir));
};
const resultScopeKey = () => resultScope()
  .map(c => `${c.dir}\u001f${observationFor(c)}`)
  .join('\u001e');

function activeCase() {
  const done = completed();
  let found = done.find(c => c.dir === state.resultCaseDir);
  if (!found && state.selected?.has_history) found = done.find(c => c.dir === state.selected.dir);
  found ??= done[0] ?? null;
  state.resultCaseDir = found?.dir ?? null;
  return found;
}

function observationFor(c) {
  return state.resultObsOverrides.get(c.dir) ?? sourceSite(c)?.obs_file ?? '';
}

function caseState(c) {
  const running = state.runState[c.dir];
  if (running === '失败') return 'failed';
  if (running === '运行中') return 'running';
  const health = historyHealth.get(c.dir);
  if (isStaleResult(c)) return 'stale';
  if (c.has_history && health?.ok === false && !health.pending) return 'invalid';
  if (hasValidatedHistory(c) || (running === '已完成' && !isStaleResult(c))) return 'done';
  return 'waiting';
}

function stateBadge(c) {
  const value = caseState(c);
  return value === 'done' ? badge('已完成', 'pass')
    : value === 'stale' ? badge('需重跑', 'warn')
    : value === 'invalid' ? badge('结果异常', 'fail')
    : value === 'failed' ? badge('失败', 'fail')
    : value === 'running' ? badge('运行中', 'on') : badge('未完成');
}

function utcText(seconds) {
  if (!Number.isFinite(seconds) || seconds === 0) return '—';
  return new Intl.DateTimeFormat(undefined, {
    year: 'numeric', month: '2-digit', day: '2-digit', timeZone: 'UTC',
  }).format(new Date(seconds * 1000));
}

function variableMeta(name, units = '') {
  const known = COMMON_VARIABLES[name];
  if (known) return { label: known[0], units: units || known[1], group: known[2] };
  const bare = name.replace(/^f_/, '').replaceAll('_', ' ');
  let group = '其他';
  if (/methane|ch4|(^|_)o2(_|$)/i.test(name)) group = '甲烷';
  else if (/snow|sno|ice/i.test(name)) group = '积雪';
  else if (/soil|soi|zwt|runoff|rnof|qinfl|qover|water/i.test(name)) group = '水文/土壤';
  else if (/urban|roof|wall|room|fach|fhac|fvehc|imper|perv/i.test(name)) group = '城市';
  else if (/crop|grain|fert|plantdate|gdd|hui/i.test(name)) group = '作物';
  else if (/lai|sai|veg|assim|resp|gpp|npp|leaf/i.test(name)) group = '植被/碳氮';
  else if (/rad|rnet|solar|long|short|albedo|fsena|lfevpa|fgrnd/i.test(name)) group = '能量/辐射';
  return { label: bare, units: units || '—', group };
}

function evaluationLabel(variable) {
  return language() === 'en' ? variable.label_en : variable.label_zh;
}

function selectedEvaluationNames() {
  return [...state.evaluationVariables];
}

function evaluationMissingReason(variable) {
  const missing = [];
  if (variable.missing_model?.length) missing.push(`${language() === 'en' ? 'model' : '模型'}: ${variable.missing_model.join(', ')}`);
  if (variable.missing_observation?.length) missing.push(`${language() === 'en' ? 'observation' : '观测'}: ${variable.missing_observation.join(', ')}`);
  return missing.join(' · ') || (language() === 'en' ? 'No valid paired samples' : '没有有效配对样本');
}

function resultKpi(value, label, kind = '') {
  const card = node('div', `result-kpi ${kind}`.trim());
  card.append(node('div', 'value', value), node('div', 'label', label));
  return card;
}

function renderOverview() {
  const cases = allCurrent();
  validateHistoryCases(cases).catch(status);
  const done = completed();
  const invalid = cases.filter(c => {
    const health = historyHealth.get(c.dir);
    return c.has_history && health?.ok === false && !health.pending;
  });
  if (!state.resultSelectionTouched) {
    state.resultSelection.clear();
    done.forEach(c => state.resultSelection.add(c.dir));
  }
  const failed = cases.filter(c => caseState(c) === 'failed');
  const observed = done.filter(c => observationFor(c));
  const kpis = $('result-kpis');
  kpis.textContent = '';
  kpis.append(
    resultKpi(cases.length, '本次算例'),
    resultKpi(done.length, '已有结果', done.length ? 'pass' : ''),
    resultKpi(failed.length, '运行失败', failed.length ? 'fail' : ''),
    resultKpi(invalid.length, '结果异常', invalid.length ? 'fail' : ''),
    resultKpi(observed.length, '可与观测评估'),
    resultKpi(Math.max(0, done.length - observed.length), '缺少观测', done.length > observed.length ? 'warn' : ''),
  );

  const query = state.resultCaseSearch.toLowerCase();
  const filter = state.resultStatusFilter;
  const shown = cases.filter(c => !query || c.name.toLowerCase().includes(query))
    .filter(c => filter === 'all'
      || (filter === 'done' && caseState(c) === 'done')
      || (filter === 'failed' && caseState(c) === 'failed')
      || (filter === 'waiting' && ['waiting', 'running'].includes(caseState(c)))
      || (filter === 'no-observation' && hasValidatedHistory(c) && !observationFor(c)));
  const host = $('result-case-matrix');
  host.textContent = '';
  if (!shown.length) {
    host.appendChild(node('div', 'result-empty', cases.length ? '没有符合筛选条件的站点。' : '本次还没有创建算例。'));
    return;
  }
  const wrap = node('div', 'result-table-wrap');
  const table = document.createElement('table');
  const head = document.createElement('tr');
  for (const label of ['分析', '站点', '运行状态', 'mksrfdata', 'mkinidata', 'colm', 'History', '观测']) head.appendChild(th(label));
  table.appendChild(head);
  for (const c of shown) {
    const row = document.createElement('tr');
    row.dataset.case = c.dir;
    const choose = td('');
    const check = document.createElement('input');
    check.type = 'checkbox';
    check.checked = state.resultSelection.has(c.dir);
    check.disabled = !hasValidatedHistory(c);
    check.title = '加入多站点分析范围';
    check.onchange = event => {
      event.stopPropagation();
      state.resultSelectionTouched = true;
      if (check.checked) state.resultSelection.add(c.dir); else state.resultSelection.delete(c.dir);
      updateComparisonButton();
    };
    choose.appendChild(check);
    row.appendChild(choose);
    row.appendChild(td(c.name));
    const run = td(''); run.appendChild(stateBadge(c)); row.appendChild(run);
    for (const stage of ['mksrfdata', 'mkinidata', 'colm']) {
      const value = state.runStages[c.dir]?.[stage];
      const cell = td('');
      cell.appendChild(value === 'ok' || value === 'skipped' ? badge(value === 'ok' ? '成功' : '跳过', value === 'ok' ? 'pass' : '')
        : value === 'failed' ? badge('失败', 'fail') : value === 'begin' ? badge('运行中', 'on') : badge('—'));
      row.appendChild(cell);
    }
    const health = historyHealth.get(c.dir);
    const hist = td('');
    hist.appendChild(!c.has_history ? badge('无')
      : health?.pending ? badge('检查中', 'on')
        : health?.ok === true ? badge('可用', 'pass')
          : badge('损坏', 'fail'));
    if (health?.error) hist.title = health.error;
    row.appendChild(hist);
    const obs = td(''); obs.appendChild(observationFor(c) ? badge('已匹配', 'pass') : badge('缺少', hasValidatedHistory(c) ? 'warn' : '')); row.appendChild(obs);
    row.onclick = () => {
      if (!hasValidatedHistory(c)) {
        status(historyHealth.get(c.dir)?.error ?? `${c.name} 还没有可分析的 history 结果`);
        return;
      }
      setResultCase(c.dir);
      go('result-series');
    };
    table.appendChild(row);
  }
  wrap.appendChild(table);
  host.appendChild(wrap);
}

function syncResultCaseSelects() {
  const done = completed();
  const current = activeCase();
  for (const select of document.querySelectorAll('[data-result-case]')) {
    const previous = current?.dir;
    select.textContent = '';
    for (const c of done) {
      const option = document.createElement('option');
      option.value = c.dir;
      option.textContent = c.name;
      select.appendChild(option);
    }
    select.value = previous ?? '';
    select.disabled = !done.length;
    if (!select.dataset.wired) {
      select.dataset.wired = '1';
      select.onchange = () => setResultCase(select.value);
    }
  }
}

function setResultCase(dir) {
  if (!completed().some(c => c.dir === dir)) return;
  const changed = state.resultCaseDir !== dir;
  state.resultCaseDir = dir;
  if (changed) {
    // A time window belongs to the selected case. Carrying it into another site can
    // silently request an empty range when the forcing periods differ.
    $('series-from').value = '';
    $('series-to').value = '';
    activeSeriesRequest += 1;
    activeMetricRequest += 1;
    activeMetricChartRequest += 1;
    currentMetricRows = [];
    currentComparisonSummary = null;
    destroyChart($('charts'));
    destroyChartsInside($('evaluation-charts'));
    $('series-stats').textContent = '';
    $('metrics').textContent = '';
    $('series-png').disabled = true;
    $('series-csv').disabled = true;
    $('evaluation-chart-refresh').disabled = true;
    $('evaluation-png').disabled = true;
  }
  syncResultCaseSelects();
  syncObservation();
  prepareActivePane();
}

function syncObservation() {
  const c = activeCase();
  const obs = $('obs');
  if (obs) obs.value = c ? observationFor(c) : '';
  updateButtons();
}

async function validateHistoryCases(cases = allCurrent()) {
  const pending = cases.filter(c => c.has_history && !historyHealth.has(c.dir));
  if (!pending.length) return;
  if (!hasBackend) {
    pending.forEach(c => historyHealth.set(c.dir, { ok: true }));
    return;
  }
  pending.forEach(c => historyHealth.set(c.dir, { ok: false, pending: true }));
  const width = Math.max(1, Math.min(4, Number(navigator.hardwareConcurrency) || 1));
  const results = await boundedMap(pending, width, async c => {
    const catalog = JSON.parse(await invoke('history_catalog', { case: c.dir }));
    assertUsableCatalog(catalog);
    catalogCache.set(c.dir, catalog);
    return catalog;
  });
  results.forEach((result, index) => {
    const c = pending[index];
    if (result.ok) historyHealth.set(c.dir, { ok: true });
    else historyHealth.set(c.dir, { ok: false, error: result.error });
  });
  if (['result-data', 'result-series', 'result-evaluation', 'result-comparison'].includes(state.step)) {
    await prepareActivePane();
  } else {
    syncResultCaseSelects();
    syncObservation();
    renderOverview();
    updateButtons();
  }
}

async function loadCatalog(c) {
  const cached = catalogCache.get(c.dir);
  if (cached) return cached;
  try {
    const catalog = JSON.parse(await invoke('history_catalog', { case: c.dir }));
    assertUsableCatalog(catalog);
    historyHealth.set(c.dir, { ok: true });
    catalogCache.set(c.dir, catalog);
    return catalog;
  } catch (error) {
    if (c?.has_history) historyHealth.set(c.dir, { ok: false, error: String(error) });
    throw error;
  }
}

function assertUsableCatalog(catalog) {
  if (!catalog?.steps || !catalog.variables?.some(variable => variable.name !== 'time')) {
    throw new Error(language() === 'en'
      ? 'history file is damaged or incomplete'
      : 'history 文件损坏或不完整');
  }
}

async function loadEvaluationCatalog(c, obs = observationFor(c)) {
  if (!c || !obs) return [];
  const key = `${c.dir}\u001f${obs}`;
  const cached = evaluationCatalogCache.get(key);
  if (cached) return cached;
  const rows = JSON.parse(await invoke('evaluation_catalog', { case: c.dir, obs }));
  return evaluationCatalogCache.set(key, rows);
}

function resetEvaluationResults(message = '') {
  currentMetricRows = [];
  currentComparisonSummary = null;
  state.resultMetrics = [];
  state.resultFailures = [];
  state.resultMetricMissing = [];
  batchEvaluationCatalogFailures = [];
  $('metrics').textContent = '';
  destroyChartsInside($('evaluation-charts'));
  $('evaluation-chart-refresh').disabled = true;
  $('evaluation-png').disabled = true;
  renderComparison();
  if (message) status(message);
}

function setEvaluationVariable(name, checked) {
  state.evaluationSelectionTouched = true;
  if (checked) state.evaluationVariables.add(name);
  else state.evaluationVariables.delete(name);
  resetEvaluationResults(language() === 'en'
    ? 'Evaluation contents changed; run the evaluation again.'
    : '评估内容已更改，请重新运行评估。');
  renderEvaluationSelector();
  renderBatchEvaluationSelector();
  updateButtons();
}

function replaceEvaluationSelection(names) {
  state.evaluationSelectionTouched = true;
  state.evaluationVariables = new Set(names);
  resetEvaluationResults(language() === 'en'
    ? 'Evaluation contents changed; run the evaluation again.'
    : '评估内容已更改，请重新运行评估。');
  renderEvaluationSelector();
  renderBatchEvaluationSelector();
  updateButtons();
}

function initializeEvaluationSelection(catalog) {
  if (state.evaluationSelectionTouched || state.evaluationVariables.size) return;
  catalog.filter(variable => variable.available)
    .forEach(variable => state.evaluationVariables.add(variable.name));
}

function evaluationVariableNode(variable, availabilityText = '') {
  const label = document.createElement('label');
  label.className = `evaluation-variable${variable.available ? '' : ' unavailable'}`;
  const input = document.createElement('input');
  input.type = 'checkbox';
  input.checked = state.evaluationVariables.has(variable.name);
  input.disabled = !variable.available;
  input.onchange = () => setEvaluationVariable(variable.name, input.checked);
  const text = node('span');
  text.append(node('b', '', `${evaluationLabel(variable)} · ${variable.name}`));
  const qc = variable.quality_control === 'measured_only'
    ? (language() === 'en' ? 'QC=0 measured values' : '仅 QC=0 实测值')
    : (language() === 'en' ? 'No QC field; all finite values' : '无 QC 字段；使用全部有限值');
  text.append(node('small', '', `${variable.model_var} ↔ ${variable.obs_var} · ${variable.units} · ${qc}`));
  if (!variable.available) text.append(node('small', 'warn', evaluationMissingReason(variable)));
  label.append(input, text, node('span', 'availability', availabilityText));
  return label;
}

function renderEvaluationSelector() {
  const host = $('evaluation-variable-selector');
  host.textContent = '';
  if (!currentEvaluationCatalog.length) {
    host.appendChild(node('div', 'result-empty', language() === 'en'
      ? 'Choose an observation file to list evaluable variables.'
      : '选择观测文件后显示可评估变量。'));
    return;
  }
  initializeEvaluationSelection(currentEvaluationCatalog);
  currentEvaluationCatalog.forEach(variable => host.appendChild(evaluationVariableNode(variable)));
}

function mergedEvaluationCatalog() {
  const byName = new Map();
  for (const entry of batchEvaluationCatalogs) {
    for (const variable of entry.catalog) byName.set(variable.name, variable);
  }
  for (const variable of currentEvaluationCatalog) if (!byName.has(variable.name)) byName.set(variable.name, variable);
  return [...byName.values()];
}

function renderBatchEvaluationSelector() {
  const host = $('batch-evaluation-variable-selector');
  host.textContent = '';
  const definitions = mergedEvaluationCatalog();
  if (!definitions.length) {
    host.appendChild(node('div', 'result-empty', language() === 'en'
      ? 'No evaluation catalogs are available in the current scope.'
      : '当前分析范围还没有可用的评估目录。'));
    if (batchEvaluationCatalogFailures.length) {
      host.appendChild(node('div', 'warn mini', `${batchEvaluationCatalogFailures.length} 个站点评估目录读取失败：${batchEvaluationCatalogFailures.map(item => item.site).join('、')}`));
    }
    return;
  }
  initializeEvaluationSelection(definitions);
  const total = resultScope().length;
  for (const definition of definitions) {
    const available = batchEvaluationCatalogs.filter(entry => entry.catalog
      .some(variable => variable.name === definition.name && variable.available)).length;
    host.appendChild(evaluationVariableNode({ ...definition, available: available > 0 },
      language() === 'en' ? `${available}/${total} sites` : `${available}/${total} 站点`));
  }
  if (batchEvaluationCatalogFailures.length) {
    host.appendChild(node('div', 'warn mini', `${batchEvaluationCatalogFailures.length}/${total} 个站点评估目录读取失败：${batchEvaluationCatalogFailures.map(item => `${item.site}（${item.reason}）`).join('、')}`));
  }
}

async function refreshCurrentEvaluationCatalog() {
  const c = activeCase();
  const obs = $('obs').value.trim();
  const isCurrent = () => state.step === 'result-evaluation'
    && activeCase()?.dir === c?.dir && $('obs').value.trim() === obs;
  currentEvaluationCatalog = [];
  renderEvaluationSelector();
  if (!c || !obs) return;
  try {
    const rows = await loadEvaluationCatalog(c, obs);
    if (!isCurrent()) return;
    currentEvaluationCatalog = rows;
    renderEvaluationSelector();
    updateButtons();
  } catch (error) { if (isCurrent()) status(error); }
}

async function refreshBatchEvaluationCatalogs() {
  const token = ++activeBatchEvaluationCatalogRequest;
  const scopeKey = resultScopeKey();
  const scope = resultScope();
  const width = Math.max(1, Math.min(4, Number(navigator.hardwareConcurrency) || 1));
  const results = await boundedMap(scope, width, async c => {
    const obs = observationFor(c);
    if (!obs) return { case: c, catalog: [] };
    return { case: c, catalog: await loadEvaluationCatalog(c, obs) };
  });
  if (token !== activeBatchEvaluationCatalogRequest || state.step !== 'result-comparison' || resultScopeKey() !== scopeKey) return false;
  batchEvaluationCatalogs = [];
  batchEvaluationCatalogFailures = [];
  results.forEach((result, index) => {
    const c = scope[index];
    if (result.ok) batchEvaluationCatalogs.push(result.value);
    else batchEvaluationCatalogFailures.push({
      site: c.name, case_dir: c.dir, reason: result.error,
    });
  });
  renderBatchEvaluationSelector();
  return true;
}

function kindLabel(kind) {
  return { series: '时间序列', profile: '垂直剖面', category: '分类维度', scalar: '标量' }[kind] ?? kind;
}

async function renderDataBrowser() {
  const token = ++activeDataBrowserRequest;
  const c = activeCase();
  const table = $('result-variable-table');
  table.textContent = '';
  if (!c) return;
  try {
    const catalog = await loadCatalog(c);
    if (token !== activeDataBrowserRequest || state.step !== 'result-data' || activeCase()?.dir !== c.dir) return;
    $('result-catalog-summary').textContent = `${catalog.files} 个 history 文件 · ${catalog.steps} 步 · ${utcText(catalog.start)} 至 ${utcText(catalog.end)} · ${catalog.variables.length} 个变量`;
    const query = $('result-variable-search').value.trim().toLowerCase();
    const kind = $('result-variable-kind').value;
    const variables = catalog.variables.filter(v => v.name !== 'time')
      .filter(v => kind === 'all' || v.kind === kind)
      .filter(v => {
        const meta = variableMeta(v.name, v.units);
        return !query || `${v.name} ${meta.label} ${meta.units} ${meta.group}`.toLowerCase().includes(query);
      });
    const head = document.createElement('tr');
    for (const label of ['变量', '名称', '分组', '单位', '维度', '类型']) head.appendChild(th(label));
    table.appendChild(head);
    for (const variable of variables) {
      const meta = variableMeta(variable.name, variable.units);
      const row = document.createElement('tr');
      row.dataset.variable = variable.name;
      row.append(td(variable.name), td(meta.label), td(meta.group), td(meta.units),
        td(variable.dimensions.map(d => `${d.name}=${d.len}`).join(' × ') || '—'));
      const kindCell = td(''); kindCell.appendChild(badge(kindLabel(variable.kind), variable.kind === 'series' ? 'pass' : '')); row.appendChild(kindCell);
      if (variable.kind === 'series') row.onclick = () => {
        $('var').value = variable.name;
        go('result-series');
        plotSeries();
      };
      else row.title = '当前变量是多维结果，请在后续剖面/分类展示器中查看；不会错误地压成一条折线。';
      table.appendChild(row);
    }
    fillVariableSelect(catalog);
  } catch (error) {
    if (token === activeDataBrowserRequest && state.step === 'result-data' && activeCase()?.dir === c.dir) {
      table.appendChild(node('caption', 'warn', String(error)));
    }
  }
}

function fillVariableSelect(catalog) {
  const select = $('var');
  const previous = select.value;
  const series = catalog.variables.filter(v => v.kind === 'series' && v.name !== 'time');
  series.sort((a, b) => Number(!COMMON_VARIABLES[a.name]) - Number(!COMMON_VARIABLES[b.name]) || a.name.localeCompare(b.name));
  select.textContent = '';
  for (const variable of series) {
    const meta = variableMeta(variable.name, variable.units);
    const option = document.createElement('option');
    option.value = variable.name;
    option.textContent = `${meta.label} · ${variable.name} [${meta.units}]`;
    select.appendChild(option);
  }
  select.value = series.some(v => v.name === previous) ? previous
    : series.some(v => v.name === 'f_rnet') ? 'f_rnet' : (series[0]?.name ?? '');
  $('plot').disabled = !select.value;
}

function parseLocalClock(value) {
  if (!value) return null;
  const normalized = value.length === 16 ? `${value}:00Z` : `${value}Z`;
  const ms = Date.parse(normalized);
  return Number.isFinite(ms) ? Math.trunc(ms / 1000) : null;
}

function inputClock(seconds) {
  if (!seconds) return '';
  return new Date(seconds * 1000).toISOString().slice(0, 16);
}

async function getSeries(c, variable, options = {}) {
  const maxPoints = Object.hasOwn(options, 'maxPoints') ? options.maxPoints : 2400;
  const request = {
    caseDir: c.dir, variable,
    from: options.from ?? '', to: options.to ?? '', maxPoints: maxPoints ?? 'all',
  };
  const key = seriesKey(request);
  const cached = maxPoints === null ? undefined : seriesCache.get(key);
  if (cached) return cached;
  const json = await invoke('series', {
    case: c.dir, vars: variable,
    from: options.from ?? null, to: options.to ?? null,
    maxPoints,
  });
  const data = JSON.parse(json);
  return maxPoints === null ? data : seriesCache.set(key, data);
}

function destroyChart(host) {
  if (!host) return;
  const current = charts.get(host);
  if (current) current.destroy();
  charts.delete(host);
  chartResize?.unobserve(host);
  host.textContent = '';
}

function destroyChartsInside(host) {
  if (!host) return;
  for (const chartHost of [...charts.keys()]) {
    if (chartHost === host || host.contains(chartHost)) destroyChart(chartHost);
  }
}

const chartResize = typeof ResizeObserver === 'undefined' ? null : new ResizeObserver(entries => {
  for (const entry of entries) {
    const chart = charts.get(entry.target);
    const width = Math.floor(entry.contentRect.width);
    if (chart && width > 120) chart.setSize({ width, height: chart.height });
  }
});

function makeChart(host, options, data, height = 230) {
  destroyChart(host);
  const width = Math.max(320, Math.floor(host.clientWidth || host.parentElement?.clientWidth || 640));
  const chart = new uPlot({
    width, height,
    tzDate: ts => uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'),
    ...options,
  }, data, host);
  chart.height = height;
  charts.set(host, chart);
  chartResize?.observe(host);
  return chart;
}

function chartColors() {
  const dark = document.documentElement.dataset.theme === 'dark';
  return {
    model: dark ? '#8fd3a6' : '#1e6b3a',
    obs: dark ? '#e0a45e' : '#a5610d',
    residual: dark ? '#8fb9ed' : '#1f6feb',
    guide: dark ? '#7d8590' : '#a8afb8',
  };
}

async function plotSeries() {
  const c = activeCase();
  const variable = $('var').value;
  if (!c || !variable) return;
  const token = ++activeSeriesRequest;
  $('plot').disabled = true;
  status(`读取 ${c.name} · ${variable}…`);
  try {
    const [data, catalog] = await Promise.all([
      getSeries(c, variable, {
        from: parseLocalClock($('series-from').value),
        to: parseLocalClock($('series-to').value),
        maxPoints: 2400,
      }),
      loadCatalog(c),
    ]);
    if (token !== activeSeriesRequest || activeCase()?.dir !== c.dir) return;
    const values = data.vars[variable];
    const catalogVariable = catalog.variables.find(item => item.name === variable);
    const meta = variableMeta(variable, catalogVariable?.units);
    const colors = chartColors();
    makeChart($('charts'), {
      title: `${c.name} · ${meta.label} · ${data.n}/${data.source_n ?? data.n} 点`,
      series: [{ label: '时间' }, { label: `${meta.label} · ${variable}`, stroke: colors.model, width: 1.3 }],
      axes: [{}, { label: meta.units }],
    }, [data.time, values], 300);
    renderSeriesStats(data, values, meta);
    $('series-png').disabled = false;
    $('series-csv').disabled = false;
    status(`${c.name} · ${meta.label} 已绘制${data.downsampled ? '（显示已保极值降采样）' : ''}`);
  } catch (error) { status(error); }
  finally { if (token === activeSeriesRequest) $('plot').disabled = false; }
}

function renderSeriesStats(data, values, meta) {
  const stats = seriesStats(values);
  const host = $('series-stats');
  host.textContent = '';
  for (const [label, value] of [
    ['显示点数', `${data.n}/${data.source_n ?? data.n}`],
    ['缺测点数', stats.missing],
    ['最小值', metricText(stats.min, 3)], ['最大值', metricText(stats.max, 3)],
    ['平均值', metricText(stats.mean, 3)], ['标准差', metricText(stats.sd, 3)],
    ['单位', meta.units],
  ]) {
    const card = node('div', 'result-stat');
    card.append(node('b', '', value), node('span', '', label));
    host.appendChild(card);
  }
}

async function exportFullSeries() {
  const c = activeCase();
  const variable = $('var').value;
  if (!c || !variable) return;
  $('series-csv').disabled = true;
  status(`读取 ${c.name} · ${variable} 完整序列…`);
  try {
    const data = await getSeries(c, variable, {
      from: parseLocalClock($('series-from').value),
      to: parseLocalClock($('series-to').value),
      maxPoints: null,
    });
    const rows = data.time.map((time, index) => ({
      time_utc: new Date(time * 1000).toISOString(), unix_seconds: time,
      variable, value: data.vars[variable][index],
    }));
    downloadText(rowsToCsv(rows, ['time_utc', 'unix_seconds', 'variable', 'value']),
      `${c.name}-${variable}.csv`, 'text/csv;charset=utf-8');
    status(`${c.name} · ${variable} 完整 CSV 已导出：${data.n} 行`);
  } catch (error) { status(error); }
  finally { $('series-csv').disabled = !activeCase() || !$('var').value; }
}

async function getMetrics(c, obs, spinup, corrected, summaryOnly = false, pairVars = [], maxPoints = null) {
  const variables = [...new Set(pairVars)].sort();
  const key = metricKey({ caseDir: c.dir, obs, spinup, corrected, summaryOnly, pairVars: variables, maxPoints: maxPoints ?? '' });
  const cached = metricsCache.get(key);
  if (cached) return cached;
  const rows = JSON.parse(await invoke('metrics', {
    case: c.dir, obs, spinup, corrected, summaryOnly,
    pairVars: variables.length ? variables : null, maxPoints,
  }));
  return metricsCache.set(key, rows);
}

async function evaluateCurrent() {
  const c = activeCase();
  const obs = $('obs').value.trim();
  if (!c || !obs) { status('要先给当前站点选择观测文件'); return; }
  const selected = currentEvaluationCatalog
    .filter(variable => variable.available && state.evaluationVariables.has(variable.name))
    .map(variable => variable.name);
  if (!selected.length) {
    status(language() === 'en' ? 'Select at least one variable available at this site.' : '请至少选择一个当前站点可用的评估变量');
    return;
  }
  state.resultObsOverrides.set(c.dir, obs);
  const token = ++activeMetricRequest;
  $('evaluate').disabled = true;
  try {
    const rows = await getMetrics(c, obs, Number($('spinup').value) || 0, $('corrected').checked, true, selected);
    if (token !== activeMetricRequest || activeCase()?.dir !== c.dir) return;
    const returned = new Set(rows.map(row => row.name));
    const missing = selected.filter(variable => !returned.has(variable));
    currentMetricRows = rows;
    state.resultMetrics = state.resultMetrics.filter(row => row.case_dir !== c.dir);
    rows.forEach(row => state.resultMetrics.push({ site: c.name, case_dir: c.dir, ...row }));
    renderMetrics(rows, missing);
    if (rows.length) drawComparison(rows[0]);
    status(language() === 'en'
      ? `${c.name}: ${rows.length} variable(s) evaluated${missing.length ? `; ${missing.length} had no valid paired samples` : ''}`
      : `${c.name} 评估完成：${rows.length} 个变量${missing.length ? `；${missing.length} 个没有有效配对样本` : ''}`);
  } catch (error) { status(error); }
  finally { if (token === activeMetricRequest) $('evaluate').disabled = false; }
}

function renderMetrics(rows, missing = []) {
  const box = $('metrics');
  box.textContent = '';
  destroyChartsInside($('evaluation-charts'));
  $('evaluation-charts').textContent = '';
  $('evaluation-chart-refresh').disabled = !rows.length;
  $('evaluation-png').disabled = true;
  if (!rows.length) {
    box.appendChild(node('div', 'result-empty', missing.length
      ? (language() === 'en' ? `No valid paired samples: ${missing.join(', ')}` : `当前时段没有有效配对样本：${missing.join('、')}`)
      : (language() === 'en' ? 'No variables can be paired.' : '没有可配对的变量。')));
    return;
  }
  const table = document.createElement('table');
  const head = document.createElement('tr');
  for (const label of ['变量', 'n', 'RMSE', 'MAE', 'Bias', 'R²', 'r', 'NSE', 'KGE', 'α', 'β']) head.appendChild(th(label, label === '变量' ? '' : 'n'));
  table.appendChild(head);
  for (const r of rows) {
    const tr = document.createElement('tr');
    tr.dataset.variable = r.model_var;
    const display = `${language() === 'en' ? r.label_en : r.label_zh} · ${r.model_var} ↔ ${r.obs_var ?? r.name}`;
    const cells = [display, r.n, metricText(r.rmse, 2), metricText(r.mae, 2),
      metricText(r.bias, 2, true), metricText(r.r2), metricText(r.correlation), metricText(r.nse, 3, true),
      metricText(r.kge, 3, true), metricText(r.alpha), metricText(r.beta)];
    cells.forEach((value, index) => tr.appendChild(td(value, index ? 'n' : '')));
    const qc = r.quality_control === 'measured_only'
      ? (language() === 'en' ? 'QC=0 measured observations only' : '仅使用 QC=0 的实测观测')
      : (language() === 'en' ? 'Observation has no QC field; all finite values were used' : '观测没有 QC 字段，使用全部有限值');
    tr.title = [qc, r.beta_warning].filter(Boolean).join('\n');
    if (r.beta_warning) tr.lastChild.className = 'n warn';
    tr.onclick = () => drawComparison(r);
    table.appendChild(tr);
  }
  box.appendChild(table);
  if (missing.length) box.appendChild(node('p', 'warn mini', language() === 'en'
    ? `No valid paired samples: ${missing.join(', ')}`
    : `当前时段没有有效配对样本：${missing.join('、')}`));
}

async function drawComparison(summaryRow) {
  currentComparisonSummary = summaryRow;
  const c = activeCase();
  const obs = $('obs').value.trim();
  if (!c || !obs) return;
  const token = ++activeMetricChartRequest;
  status(`读取 ${c.name} · ${summaryRow.name} 配对点…`);
  let rows;
  try {
    rows = await getMetrics(c, obs, Number($('spinup').value) || 0, $('corrected').checked,
      false, [summaryRow.name], 2400);
  } catch (error) { status(error); return; }
  if (token !== activeMetricChartRequest || activeCase()?.dir !== c.dir) return;
  const row = rows[0];
  if (!row?.time || !row?.model || !row?.obs) { status(`${summaryRow.name} 没有可绘制的配对点`); return; }
  renderComparisonCharts(row);
  status(`${c.name} · ${row.name} 图形诊断已更新`);
}

function renderComparisonCharts(row) {
  const host = $('evaluation-charts');
  destroyChartsInside(host);
  host.textContent = '';
  const summary = node('p', 'muted mini result-chart-summary',
    `${evaluationLabel(row)} · ${row.name} · n=${row.n} · RMSE=${metricText(row.rmse, 2)} · Bias=${metricText(row.bias, 2, true)} · R²=${metricText(row.r2)} · NSE=${metricText(row.nse, 3, true)} · KGE=${metricText(row.kge, 3, true)} · 图形点 ${row.pair_n ?? row.time.length}/${row.pair_source_n ?? row.time.length}`);
  const timeHost = node('div', 'chart');
  const scatterHost = node('div', 'chart');
  const residualHost = node('div', 'chart');
  const stats = node('div', 'result-stat-grid');
  for (const [label, value] of [
    ['模型均值', metricText(row.model_mean, 3)], ['观测均值', metricText(row.obs_mean, 3)],
    ['模型标准差', metricText(row.model_sd, 3)], ['观测标准差', metricText(row.obs_sd, 3)],
    ['KGE r', metricText(row.correlation)], ['KGE α', metricText(row.alpha)], ['KGE β', metricText(row.beta)],
  ]) {
    const card = node('div', 'result-stat');
    card.append(node('b', '', value), node('span', '', label));
    stats.appendChild(card);
  }
  host.append(summary, stats, timeHost, scatterHost, residualHost);
  const colors = chartColors();
  makeChart(timeHost, {
    title: `${row.name} · 模型与观测`,
    series: [{ label: '时间' }, { label: `CoLM · ${row.model_var}`, stroke: colors.model, width: 1.2 }, { label: `${language() === 'en' ? 'Observation' : '观测'} · ${row.obs_var}`, stroke: colors.obs, width: 1.2 }],
    axes: [{}, { label: row.units }],
  }, [row.time, row.model, row.obs]);
  const order = row.obs.map((value, index) => [value, row.model[index]]).sort((a, b) => a[0] - b[0]);
  makeChart(scatterHost, {
    title: `${row.name} · 观测（横）与模型（纵）`, scales: { x: { time: false } },
    series: [{ label: '观测' }, { label: '模型', stroke: 'transparent', points: { show: true, size: 3, stroke: colors.model } },
      { label: '1:1', stroke: colors.guide, width: 1, dash: [4, 4] }],
  }, [order.map(x => x[0]), order.map(x => x[1]), order.map(x => x[0])]);
  const residual = row.model.map((value, index) => value - row.obs[index]);
  makeChart(residualHost, {
    title: `${row.name} · 残差（模型 − 观测）`,
    series: [{ label: '时间' }, { label: '残差', stroke: colors.residual, width: 1.1 }], axes: [{}, {}],
  }, [row.time, residual]);
  $('evaluation-png').disabled = false;
}

function updateComparisonButton() {
  const scope = resultScope();
  $('eval-all').disabled = !scope.length || !state.evaluationVariables.size || !!comparisonController;
  $('eval-all').textContent = `评估分析范围内的 ${scope.length} 个站点`;
}

async function evaluateAll() {
  const todo = resultScope();
  if (!todo.length) return;
  const selected = selectedEvaluationNames();
  if (!selected.length) {
    status(language() === 'en' ? 'Select at least one variable for batch evaluation.' : '请至少选择一个批量评估变量');
    return;
  }
  comparisonController = new AbortController();
  $('eval-cancel').disabled = false;
  updateComparisonButton();
  $('eval-progress').max = todo.length;
  $('eval-progress').value = 0;
  const spinup = Number($('spinup').value) || 0;
  const corrected = $('corrected').checked;
  const width = Math.max(1, Math.min(4, Number(navigator.hardwareConcurrency) || 1));
  status(`并发评估 ${todo.length} 个站点（最多 ${width} 个同时进行）`);
  const results = await boundedMap(todo, width, async c => {
    const obs = observationFor(c);
    if (!obs) throw new Error('没有观测文件');
    const catalog = await loadEvaluationCatalog(c, obs);
    const requested = catalog.filter(variable => selected.includes(variable.name));
    const available = requested.filter(variable => variable.available).map(variable => variable.name);
    const missing = requested.filter(variable => !variable.available)
      .map(variable => ({ variable: variable.name, reason: evaluationMissingReason(variable) }));
    const rows = available.length
      ? await getMetrics(c, obs, spinup, corrected, true, available) : [];
    const returned = new Set(rows.map(row => row.name));
    for (const variable of available) {
      if (!returned.has(variable)) missing.push({
        variable,
        reason: language() === 'en' ? 'No valid paired samples' : '没有有效配对样本',
      });
    }
    return { case: c, rows, missing };
  }, {
    signal: comparisonController.signal,
    onProgress: progress => {
      $('eval-progress').value = progress.completed;
      $('eval-progress-text').textContent = `${progress.completed}/${progress.total}`;
    },
  });
  state.resultMetrics = [];
  state.resultFailures = [];
  state.resultMetricMissing = [];
  results.forEach((result, index) => {
    const c = todo[index];
    if (result.ok) {
      result.value.rows.forEach(row => state.resultMetrics.push({ site: c.name, case_dir: c.dir, ...row }));
      result.value.missing.forEach(item => state.resultMetricMissing.push({
        site: c.name, case_dir: c.dir, ...item,
      }));
    }
    else state.resultFailures.push({ site: c.name, case_dir: c.dir, reason: result.error, cancelled: !!result.cancelled });
  });
  const cancelled = comparisonController.signal.aborted;
  comparisonController = null;
  $('eval-cancel').disabled = true;
  updateComparisonButton();
  renderComparison();
  status(cancelled
    ? `批量评估已取消：保留 ${new Set(state.resultMetrics.map(row => row.case_dir)).size} 个已完成站点`
    : `批量评估完成：${new Set(state.resultMetrics.map(row => row.case_dir)).size}/${todo.length} 个站点有结果`);
}

function renderComparison() {
  const rows = state.resultMetrics;
  const controls = $('comparison-controls');
  const rankingCard = $('summary-ranking-card');
  controls.hidden = !rows.length;
  rankingCard.hidden = !rows.length;
  const summary = $('summary');
  summary.textContent = '';
  if (!rows.length && !state.resultFailures.length && !state.resultMetricMissing.length && !batchEvaluationCatalogFailures.length) {
    summary.appendChild(node('div', 'result-empty', '尚未运行多站点评估。'));
    return;
  }
  const catalog = mergedEvaluationCatalog();
  const selected = selectedEvaluationNames();
  const variables = selected.length ? selected : [...new Set(rows.map(row => row.name))];
  state.summaryVar = variables.includes(state.summaryVar) ? state.summaryVar
    : variables.includes('Rnet') ? 'Rnet' : variables[0];
  const variableSelect = $('summary-var');
  variableSelect.textContent = '';
  for (const variable of variables) {
    const definition = catalog.find(item => item.name === variable);
    const option = document.createElement('option'); option.value = variable;
    option.textContent = definition ? `${evaluationLabel(definition)} · ${variable}` : variable;
    variableSelect.appendChild(option);
  }
  variableSelect.value = state.summaryVar ?? '';
  const metric = $('summary-metric').value || state.summarySort || 'r2';
  state.summarySort = metric;
  const search = $('summary-search').value.trim().toLowerCase();
  const valueRows = rows.filter(row => row.name === state.summaryVar)
    .filter(row => !search || row.site.toLowerCase().includes(search));
  renderRanking(valueRows, metric);
  const byCase = new Map(valueRows.map(row => [row.case_dir, row]));
  const shown = resultScope().filter(c => !search || c.name.toLowerCase().includes(search)).map(c => {
    const value = byCase.get(c.dir);
    if (value) return { ...value, availability: language() === 'en' ? 'Available' : '可用' };
    const missing = state.resultMetricMissing.find(item => item.case_dir === c.dir && item.variable === state.summaryVar);
    const failed = state.resultFailures.find(item => item.case_dir === c.dir);
    return {
      site: c.name, case_dir: c.dir, name: state.summaryVar,
      availability: language() === 'en' ? 'Unavailable' : '不可用',
      reason: missing?.reason ?? failed?.reason ?? (language() === 'en' ? 'Not evaluated' : '未评估'),
    };
  });
  const table = document.createElement('table');
  const columns = [['站点', 'site'], ['状态', 'availability'], ['n', 'n'], ['RMSE', 'rmse'], ['MAE', 'mae'], ['Bias', 'bias'], ['R²', 'r2'], ['r', 'correlation'], ['NSE', 'nse'], ['KGE', 'kge']];
  const head = document.createElement('tr'); columns.forEach(([label], index) => head.appendChild(th(label, index ? 'n' : ''))); table.appendChild(head);
  const meta = METRIC_META[metric] ?? { better: 'high' };
  const badness = row => meta.better === 'low' ? Number(row[metric])
    : meta.better === 'zero' ? Math.abs(Number(row[metric])) : -Number(row[metric]);
  shown.sort((a, b) => Number(!a.n) - Number(!b.n) || badness(b) - badness(a));
  for (const row of shown) {
    const tr = document.createElement('tr');
    columns.forEach(([, key], index) => {
      const raw = row[key];
      const value = ['site', 'availability'].includes(key) ? raw
        : metricText(raw, key === 'n' ? 0 : 3, key === 'bias');
      tr.appendChild(td(value, index > 1 ? 'n' : ''));
    });
    tr.title = [row.reason, row.beta_warning].filter(Boolean).join('\n');
    if (row.n) tr.onclick = async () => {
      setResultCase(row.case_dir);
      go('result-evaluation');
      await refreshCurrentEvaluationCatalog();
      await evaluateCurrent();
    };
    table.appendChild(tr);
  }
  const wrap = node('div', 'result-table-wrap'); wrap.appendChild(table); summary.appendChild(wrap);
  if (state.resultFailures.length) {
    const failure = node('div', 'warn mini');
    failure.textContent = `${state.resultFailures.length} 个站点未完成评估：` + state.resultFailures.map(item => `${item.site}（${item.reason}）`).join('、');
    summary.appendChild(failure);
  }
  if (batchEvaluationCatalogFailures.length) {
    const failure = node('div', 'warn mini');
    failure.textContent = `${batchEvaluationCatalogFailures.length} 个站点评估目录读取失败：` + batchEvaluationCatalogFailures.map(item => `${item.site}（${item.reason}）`).join('、');
    summary.appendChild(failure);
  }
}

function renderRanking(rows, metric) {
  const host = $('summary-ranking');
  host.textContent = '';
  const ranked = ranking(rows, metric).reverse(); // 最差在上，优先暴露异常站点。
  for (const row of ranked) {
    const item = node('div', 'metric-rank-row');
    const track = node('div', 'metric-rank-track');
    const bar = document.createElement('i'); bar.style.width = `${Math.max(2, row.rankFraction * 100)}%`; track.appendChild(bar);
    item.append(node('span', 'name', row.site), track, node('span', 'value', metricText(row[metric], 3, metric === 'bias')));
    host.appendChild(item);
  }
}

async function diagnoseCurrent() {
  const c = activeCase();
  if (!c) return;
  $('diagnose').disabled = true;
  const host = $('diagnostics');
  host.textContent = '';
  try {
    const catalog = await loadCatalog(c);
    const names = new Set(catalog.variables.map(v => v.name));
    host.appendChild(diagnosticCard('时间与产物', 'pass', [
      ['History 文件', catalog.files], ['时间步', catalog.steps],
      ['起始', utcText(catalog.start)], ['结束', utcText(catalog.end)],
      ['变量', catalog.variables.length],
    ]));
    const required = ['f_rnet', 'f_fsena', 'f_lfevpa', 'f_fgrnd'];
    const missing = required.filter(name => !names.has(name));
    if (missing.length) {
      host.appendChild(diagnosticCard('能量平衡', 'warn', [['状态', '无法计算'], ['缺少变量', missing.join('、')]], '只有所需通量全部存在时才计算，不用零值补缺项。'));
    } else {
      const data = await getSeries(c, required.join(','), { maxPoints: 20000 });
      const residual = data.time.map((_, index) => {
        const values = required.map(name => finite(data.vars[name][index]));
        if (values.some(value => value == null)) return null;
        return values[0] - values[1] - values[2] - values[3];
      });
      const stats = seriesStats(residual);
      host.appendChild(diagnosticCard('能量平衡残差', Math.abs(stats.mean ?? 0) < 20 ? 'pass' : 'warn', [
        ['有效点', stats.n], ['缺测点', stats.missing], ['平均残差', `${metricText(stats.mean, 2, true)} W/m²`],
        ['标准差', `${metricText(stats.sd, 2)} W/m²`], ['最大绝对值', `${metricText(Math.max(Math.abs(stats.min ?? 0), Math.abs(stats.max ?? 0)), 2)} W/m²`],
      ], data.downsampled ? '诊断基于保极值抽样点；导出原始数据可做完整审计。' : '按 Rnet − Qh − Qle − Qg 计算。'));
    }
    const rangeChecks = [
      ['f_xy_t', 150, 350, '参考高度气温'], ['f_zwt', -200, 20, '地下水位'], ['f_snowdp', 0, 20, '雪深'],
    ].filter(([name]) => names.has(name));
    const rangeRows = [];
    for (const [name, lo, hi, label] of rangeChecks) {
      const data = await getSeries(c, name, { maxPoints: 10000 });
      const stats = seriesStats(data.vars[name]);
      const bad = data.vars[name].filter(value => Number.isFinite(value) && (value < lo || value > hi)).length;
      rangeRows.push([label, bad ? `${bad} 个抽样点越界` : '抽样点均在合理范围']);
    }
    host.appendChild(diagnosticCard('物理范围', rangeRows.some(([, value]) => value.includes('越界')) ? 'warn' : 'pass', rangeRows.length ? rangeRows : [['状态', '没有已定义范围检查的变量']]));
    const profile = catalog.variables.filter(v => v.kind === 'profile').length;
    const category = catalog.variables.filter(v => v.kind === 'category').length;
    host.appendChild(diagnosticCard('变量结构', '', [['时间序列', catalog.variables.filter(v => v.kind === 'series').length], ['垂直剖面', profile], ['分类维度', category]], profile || category ? '多维变量已识别，不会被错误压成一维折线。' : '当前 history 以标量时间序列为主。'));
  } catch (error) {
    host.appendChild(diagnosticCard('诊断失败', 'fail', [['原因', String(error)]]));
  } finally { $('diagnose').disabled = false; }
}

function diagnosticCard(title, kind, rows, note = '') {
  const card = node('div', 'diagnostic-card');
  const head = node('h3'); head.append(node('span', '', title), badge(kind === 'pass' ? '正常' : kind === 'warn' ? '注意' : kind === 'fail' ? '失败' : '信息', kind)); card.appendChild(head);
  const list = document.createElement('dl');
  for (const [label, value] of rows) list.append(node('dt', '', label), node('dd', '', value));
  card.appendChild(list);
  if (note) card.appendChild(node('p', kind === 'warn' ? 'warn mini' : 'muted mini', note));
  return card;
}

function reportData() {
  const scope = resultScope();
  const scopeDirs = new Set(scope.map(c => c.dir));
  const runFailures = allCurrent()
    .filter(c => caseState(c) === 'failed')
    .map(c => ({
      site: c.name, case_dir: c.dir, phase: 'run',
      reason: state.runProgress[c.dir]?.reason ?? (language() === 'en' ? 'Run failed' : '运行失败'),
    }));
  const evaluationFailures = state.resultFailures
    .filter(item => scopeDirs.has(item.case_dir))
    .map(item => ({ ...item, phase: 'evaluation' }));
  const catalogFailures = batchEvaluationCatalogFailures
    .filter(item => scopeDirs.has(item.case_dir))
    .map(item => ({ ...item, phase: 'catalog' }));
  const invalidHistoryFailures = allCurrent()
    .filter(c => {
      const health = historyHealth.get(c.dir);
      return c.has_history && health?.ok === false && !health.pending;
    })
    .map(c => ({
      site: c.name, case_dir: c.dir, phase: 'history',
      reason: historyHealth.get(c.dir)?.error ?? (language() === 'en' ? 'Invalid history file' : 'history 文件损坏或不完整'),
    }));
  const metricMissing = state.resultMetricMissing
    .filter(item => scopeDirs.has(item.case_dir))
    .map(item => ({ ...item, phase: 'variable', reason: `${item.variable}: ${item.reason}` }));
  return {
    product: 'CoLM Desktop', version: $('about-version')?.textContent?.trim() || 'unknown', generated_at: new Date().toISOString(),
    copyright: 'CoLM LSM Development Team, School of Atmospheric Sciences, SYSU',
    settings: {
      domain: state.domain,
      subgrid: state.subgrid,
      wizard: state.wizard,
      kernel: $('kernel')?.value || null,
      discarded_records: Number($('spinup').value) || 0,
      energy_closure_corrected: $('corrected').checked,
      evaluation_variables: selectedEvaluationNames(),
      analysis_sites: scope.length,
    },
    cases: allCurrent().map(c => ({
      name: c.name, dir: c.dir, status: caseState(c), in_analysis_scope: scopeDirs.has(c.dir),
      has_history: !!c.has_history, observation: observationFor(c) || null,
    })),
    metrics: $('export-metrics').checked
      ? state.resultMetrics.filter(row => scopeDirs.has(row.case_dir)).map(stripMetricPairs) : [],
    failures: $('export-failures').checked ? [...runFailures, ...invalidHistoryFailures, ...catalogFailures, ...evaluationFailures, ...metricMissing] : [],
  };
}

function stripMetricPairs(row) {
  const { time, model, obs, ...summary } = row;
  return summary;
}

function markdownReport(data) {
  if (language() === 'en') {
    const lines = ['# CoLM Desktop Results Analysis Report', '', `Software version: ${data.version}`,
      `Generated: ${data.generated_at}`, `Analysis scope: ${data.settings.analysis_sites} sites`,
      `Subgrid scheme: ${data.settings.subgrid ?? '—'}`, `Discarded output records: ${data.settings.discarded_records}`,
      `Energy-closure correction: ${data.settings.energy_closure_corrected ? 'Yes' : 'No'}`,
      `Evaluation variables: ${data.settings.evaluation_variables.join(', ') || '—'}`,
      '', '## Cases', '', '| Site | Status | Analysis scope | History | Observation |', '|---|---|---:|---:|---|'];
    data.cases.forEach(c => lines.push(`| ${c.name} | ${c.status} | ${c.in_analysis_scope ? 'Yes' : 'No'} | ${c.has_history ? 'Yes' : 'No'} | ${c.observation ?? '—'} |`));
    if (data.metrics.length) {
      lines.push('', '## Evaluation metrics', '', '| Site | Variable | n | RMSE | MAE | Bias | R² | r | NSE | KGE |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|');
      data.metrics.forEach(row => lines.push(`| ${row.site} | ${row.name} | ${row.n} | ${metricText(row.rmse)} | ${metricText(row.mae)} | ${metricText(row.bias)} | ${metricText(row.r2)} | ${metricText(row.correlation)} | ${metricText(row.nse)} | ${metricText(row.kge)} |`));
    }
    if (data.failures.length) {
      lines.push('', '## Incomplete items', '');
      const phases = { run: 'run', evaluation: 'evaluation', variable: 'variable' };
      data.failures.forEach(item => lines.push(`- ${item.site} [${phases[item.phase] ?? item.phase}]: ${item.reason}`));
    }
    lines.push('', '---', '', 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU');
    return lines.join('\n') + '\n';
  }
  const lines = ['# CoLM Desktop 结果分析报告', '', `软件版本：${data.version}`, `生成时间：${data.generated_at}`,
    `分析范围：${data.settings.analysis_sites} 个站点`, `次网格方案：${data.settings.subgrid ?? '—'}`,
    `丢弃输出记录：${data.settings.discarded_records}`, `能量闭合订正：${data.settings.energy_closure_corrected ? '是' : '否'}`,
    `评估变量：${data.settings.evaluation_variables.join('、') || '—'}`,
    '', '## 算例', '', '| 站点 | 状态 | 分析范围 | History | 观测 |', '|---|---|---:|---:|---|'];
  const statusLabels = { done: '已完成', invalid: '结果异常', failed: '失败', running: '运行中', waiting: '未完成' };
  data.cases.forEach(c => lines.push(`| ${c.name} | ${statusLabels[c.status] ?? c.status} | ${c.in_analysis_scope ? '是' : '否'} | ${c.has_history ? '是' : '否'} | ${c.observation ?? '—'} |`));
  if (data.metrics.length) {
    lines.push('', '## 评估指标', '', '| 站点 | 变量 | n | RMSE | MAE | Bias | R² | r | NSE | KGE |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|');
    data.metrics.forEach(row => lines.push(`| ${row.site} | ${row.name} | ${row.n} | ${metricText(row.rmse)} | ${metricText(row.mae)} | ${metricText(row.bias)} | ${metricText(row.r2)} | ${metricText(row.correlation)} | ${metricText(row.nse)} | ${metricText(row.kge)} |`));
  }
  if (data.failures.length) {
    lines.push('', '## 未完成项', '');
    const phases = { run: '运行', evaluation: '评估', variable: '变量' };
    data.failures.forEach(item => lines.push(`- ${item.site} [${phases[item.phase] ?? item.phase}]：${item.reason}`));
  }
  lines.push('', '---', '', 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU');
  return lines.join('\n') + '\n';
}

function escapeHtml(value) {
  return String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
}

function printableReportHtml(data) {
  const en = language() === 'en';
  const h = value => escapeHtml(value ?? '—');
  const yes = value => value ? (en ? 'Yes' : '是') : (en ? 'No' : '否');
  const statusLabels = en
    ? { done: 'Completed', invalid: 'Invalid result', failed: 'Failed', running: 'Running', waiting: 'Pending' }
    : { done: '已完成', invalid: '结果异常', failed: '失败', running: '运行中', waiting: '未完成' };
  const caseRows = data.cases.map(c => `<tr><td>${h(c.name)}</td><td>${h(statusLabels[c.status] ?? c.status)}</td><td>${yes(c.in_analysis_scope)}</td><td>${yes(c.has_history)}</td><td>${h(c.observation)}</td></tr>`).join('');
  const metricRows = data.metrics.map(row => `<tr><td>${h(row.site)}</td><td>${h(language() === 'en' ? row.label_en : row.label_zh)} · ${h(row.name)}</td><td>${h(row.n)}</td><td>${h(metricText(row.rmse))}</td><td>${h(metricText(row.mae))}</td><td>${h(metricText(row.bias))}</td><td>${h(metricText(row.r2))}</td><td>${h(metricText(row.correlation))}</td><td>${h(metricText(row.nse))}</td><td>${h(metricText(row.kge))}</td></tr>`).join('');
  const failures = data.failures.map(item => `<li><b>${h(item.site)}</b> · ${h(item.reason)}</li>`).join('');
  return `
    <h1>${en ? 'CoLM Desktop Results Analysis Report' : 'CoLM Desktop 结果分析报告'}</h1>
    <p>${en ? 'Generated' : '生成时间'}: ${h(data.generated_at)}</p>
    <div class="print-meta">
      <div><b>${en ? 'Analysis scope' : '分析范围'}</b>${h(data.settings.analysis_sites)} ${en ? 'sites' : '个站点'}</div>
      <div><b>${en ? 'Evaluation variables' : '评估变量'}</b>${h(data.settings.evaluation_variables.join(', ') || '—')}</div>
      <div><b>${en ? 'Energy closure correction' : '能量闭合订正'}</b>${yes(data.settings.energy_closure_corrected)}</div>
      <div><b>${en ? 'Subgrid scheme' : '次网格方案'}</b>${h(data.settings.subgrid)}</div>
      <div><b>${en ? 'Discarded records' : '丢弃输出记录'}</b>${h(data.settings.discarded_records)}</div>
      <div><b>${en ? 'Software version' : '软件版本'}</b>${h(data.version)}</div>
    </div>
    <h2>${en ? 'Cases' : '算例'}</h2>
    <table><thead><tr><th>${en ? 'Site' : '站点'}</th><th>${en ? 'Status' : '状态'}</th><th>${en ? 'In scope' : '分析范围'}</th><th>History</th><th>${en ? 'Observation' : '观测'}</th></tr></thead><tbody>${caseRows}</tbody></table>
    ${data.metrics.length ? `<h2>${en ? 'Evaluation metrics' : '评估指标'}</h2><table><thead><tr><th>${en ? 'Site' : '站点'}</th><th>${en ? 'Variable' : '变量'}</th><th>n</th><th>RMSE</th><th>MAE</th><th>Bias</th><th>R²</th><th>r</th><th>NSE</th><th>KGE</th></tr></thead><tbody>${metricRows}</tbody></table>` : ''}
    ${data.failures.length ? `<h2>${en ? 'Incomplete items' : '未完成项'}</h2><ul>${failures}</ul>` : ''}
    <footer>${en ? 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU' : '版权所有：CoLM陆面模式开发团队，中山大学大气科学学院'}</footer>`;
}

async function exportPdfReport() {
  document.querySelector('#print-report')?.remove();
  const report = document.createElement('article');
  report.id = 'print-report';
  report.innerHTML = printableReportHtml(reportData());
  document.body.appendChild(report);
  const cleanup = () => report.remove();
  addEventListener('afterprint', cleanup, { once: true });
  await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  try {
    if (hasBackend) await invoke('print_report');
    else if (typeof window.print === 'function') window.print();
    else throw new Error(language() === 'en' ? 'Printing is unavailable.' : '当前环境不支持打印。');
    // 某些 WebView 不触发 afterprint；留足打印对话框时间后再清理隐藏节点。
    setTimeout(cleanup, 300_000);
    status(language() === 'en'
      ? 'Print dialog opened; choose Save as PDF.'
      : '已打开打印窗口，请选择“另存为 PDF”。');
  } catch (error) {
    cleanup();
    status(error);
  }
}

function generateReport() {
  const data = reportData();
  const format = $('export-format').value;
  if (format === 'pdf') { exportPdfReport(); return; }
  if (format === 'json') { reportText = JSON.stringify(data, null, 2) + '\n'; reportExtension = 'json'; }
  else if (format === 'csv') {
    reportText = rowsToCsv(data.metrics, ['site', 'name', 'n', 'rmse', 'mae', 'bias', 'r2', 'correlation', 'nse', 'kge', 'alpha', 'beta']);
    reportExtension = 'csv';
  } else if (format === 'html') {
    const markdown = markdownReport(data);
    const title = language() === 'en' ? 'CoLM Desktop Results Analysis Report' : 'CoLM Desktop 结果分析报告';
    reportText = `<!doctype html><meta charset="utf-8"><title>${title}</title><style>body{font:14px system-ui;max-width:1100px;margin:40px auto;padding:0 20px;color:#17251d}pre{white-space:pre-wrap;line-height:1.55;background:#f4f8f5;border:1px solid #d9e5dd;border-radius:12px;padding:20px}</style><h1>${title}</h1><pre>${escapeHtml(markdown.replace(/^# .*\n/, ''))}</pre>`;
    reportExtension = 'html';
  } else { reportText = markdownReport(data); reportExtension = 'md'; }
  $('export-preview').textContent = reportText;
  $('export-copy').disabled = false;
  $('export-download').disabled = false;
  status(`已生成 ${format.toUpperCase()} 报告`);
}

async function copyText(text) {
  try { await navigator.clipboard.writeText(text); }
  catch {
    const area = document.createElement('textarea'); area.value = text; document.body.appendChild(area); area.select(); document.execCommand('copy'); area.remove();
  }
}

function downloadText(text, filename, type = 'text/plain') {
  const link = document.createElement('a');
  link.href = URL.createObjectURL(new Blob([text], { type }));
  link.download = filename;
  link.click();
  setTimeout(() => URL.revokeObjectURL(link.href), 5000);
}

function exportChart(host, filename) {
  const canvas = host.querySelector('canvas');
  if (!canvas) return;
  const link = document.createElement('a'); link.href = canvas.toDataURL('image/png'); link.download = filename; link.click();
}

export async function markResultsStale(dirs) {
  const target = new Set(dirs);
  for (const c of state.cases) {
    if (!target.has(c.dir)) continue;
    c.has_history = false;
    state.runState[c.dir] = '需重跑';
    invalidateResultCase(c.dir);
  }
  if (hasBackend && target.size) await invoke('mark_results_stale', { dirs: [...target] });
}

export function invalidateResultCase(dir) {
  catalogCache.delete(dir);
  historyHealth.delete(dir);
  evaluationCatalogCache.deleteWhere(key => key.startsWith(`${dir}\u001f`));
  seriesCache.deleteWhere(key => key.startsWith(`${dir}\u001f`));
  metricsCache.deleteWhere(key => key.startsWith(`${dir}\u001f`));
  state.resultMetrics = state.resultMetrics.filter(row => row.case_dir !== dir);
  state.resultFailures = state.resultFailures.filter(row => row.case_dir !== dir);
  state.resultMetricMissing = state.resultMetricMissing.filter(row => row.case_dir !== dir);
  batchEvaluationCatalogFailures = batchEvaluationCatalogFailures.filter(row => row.case_dir !== dir);
}

function updateButtons() {
  const c = activeCase();
  $('plot').disabled = !c || !$('var').value;
  $('series-csv').disabled = !c || !$('var').value;
  const hasSelectedAvailable = currentEvaluationCatalog.some(variable =>
    variable.available && state.evaluationVariables.has(variable.name));
  $('evaluate').disabled = !c || !$('obs').value.trim() || !hasSelectedAvailable;
  $('diagnose').disabled = !c;
  updateComparisonButton();
}

async function prepareActivePane() {
  if (!state.step.startsWith('result-')) return;
  const token = ++activePaneRequest;
  const step = state.step;
  renderOverview();
  syncResultCaseSelects();
  syncObservation();
  // 两类 Study 是基本设定后的独立分支，不要求原算例先有 history。
  // 因此必须在 `activeCase()`（只返回已完成结果）这个早退之前准备页面。
  if (step === 'result-uncertainty' || step === 'result-tuning') {
    const kind = step === 'result-tuning' ? 'tuning' : 'uq';
    const scopeKey = studyScopeKey();
    const isCurrent = () => token === activePaneRequest && state.step === step && studyScopeKey() === scopeKey;
    syncStudyKernelLabels();
    renderStudyWizard(kind);
    try {
      await loadStudyParams(isCurrent);
      if (!isCurrent()) return;
      if (kind === 'tuning') await renderTuningTargets(isCurrent);
      else { await renderUqSpinup(isCurrent); await renderStudyOutputs(isCurrent); }
      if (!isCurrent()) return;
      if (activeStudyDirs(kind).length) await refreshStudy(kind);
    } catch (error) {
      if (token === activePaneRequest && state.step === step) status(error);
    }
    if (token === activePaneRequest && state.step === step) {
      renderStudyBudget(kind);
      updateButtons();
    }
    return;
  }
  const c = activeCase();
  const isCurrent = () => token === activePaneRequest && state.step === step && activeCase()?.dir === c?.dir;
  if (!c) { updateButtons(); return; }
  if (['result-data', 'result-series'].includes(step)) {
    try {
      const catalog = await loadCatalog(c);
      if (!isCurrent()) return;
      fillVariableSelect(catalog);
      if (!$('series-from').value) $('series-from').value = inputClock(catalog.start);
      if (!$('series-to').value) $('series-to').value = inputClock(catalog.end);
      if (step === 'result-data') await renderDataBrowser();
      if (!isCurrent()) return;
    } catch (error) { if (isCurrent()) status(error); }
  }
  if (step === 'result-evaluation') {
    await refreshCurrentEvaluationCatalog();
    if (!isCurrent()) return;
  }
  if (step === 'result-comparison') {
    if (await refreshBatchEvaluationCatalogs()) renderComparison();
    if (!isCurrent()) return;
  }
  if (isCurrent()) updateButtons();
}

/** 外部仍调用旧名字 `refreshVars`；现在它刷新的是整个结果索引。 */
export async function refreshVars() {
  renderOverview();
  syncResultCaseSelects();
  syncObservation();
  renderSteps();
  await prepareActivePane();
}

$('result-case-search').oninput = () => { state.resultCaseSearch = $('result-case-search').value; renderOverview(); };
$('result-status-filter').onchange = () => { state.resultStatusFilter = $('result-status-filter').value; renderOverview(); };
$('result-refresh').onclick = async () => {
  allCurrent().forEach(c => invalidateResultCase(c.dir));
  await refreshVars();
  status('结果索引已刷新');
};
$('result-variable-search').oninput = renderDataBrowser;
$('result-variable-kind').onchange = renderDataBrowser;
$('plot').onclick = plotSeries;
$('series-reset').onclick = async () => {
  const c = activeCase(); if (!c) return;
  const catalog = await loadCatalog(c);
  $('series-from').value = inputClock(catalog.start); $('series-to').value = inputClock(catalog.end);
  plotSeries();
};
$('series-png').onclick = () => exportChart($('charts'), `${activeCase()?.name ?? 'colm'}-${$('var').value}.png`);
$('series-csv').onclick = exportFullSeries;
$('obs').onchange = async () => {
  const c = activeCase();
  if (c) state.resultObsOverrides.set(c.dir, $('obs').value.trim());
  resetEvaluationResults();
  await refreshCurrentEvaluationCatalog();
  updateButtons();
};
$('eval-select-all').onclick = () => replaceEvaluationSelection(
  currentEvaluationCatalog.filter(variable => variable.available).map(variable => variable.name));
$('eval-select-none').onclick = () => replaceEvaluationSelection([]);
$('batch-eval-select-all').onclick = () => replaceEvaluationSelection(
  mergedEvaluationCatalog().filter(variable => batchEvaluationCatalogs.some(entry => entry.catalog
    .some(item => item.name === variable.name && item.available))).map(variable => variable.name));
$('batch-eval-select-none').onclick = () => replaceEvaluationSelection([]);
$('evaluate').onclick = evaluateCurrent;
$('corrected').onchange = () => { if (currentMetricRows.length) evaluateCurrent(); };
$('evaluation-chart-refresh').onclick = () => {
  const row = currentComparisonSummary ?? currentMetricRows[0];
  if (row) drawComparison(row);
  else status(language() === 'en'
    ? 'Run the evaluation and select a variable first.'
    : '请先运行评估并选择一个变量。');
};
$('evaluation-png').onclick = () => exportChart($('evaluation-charts'), `${activeCase()?.name ?? 'colm'}-evaluation.png`);
$('eval-all').onclick = evaluateAll;
$('eval-cancel').onclick = () => comparisonController?.abort();
$('summary-var').onchange = () => { state.summaryVar = $('summary-var').value; renderComparison(); };
$('summary-metric').onchange = () => { state.summarySort = $('summary-metric').value; renderComparison(); };
$('summary-search').oninput = renderComparison;
$('summary-copy').onclick = async () => {
  const values = new Map(state.resultMetrics.filter(row => row.name === state.summaryVar)
    .map(row => [row.case_dir, stripMetricPairs(row)]));
  const rows = resultScope().map(c => values.get(c.dir) ?? {
    site: c.name, case_dir: c.dir, name: state.summaryVar, status: 'unavailable',
    reason: state.resultMetricMissing.find(item => item.case_dir === c.dir && item.variable === state.summaryVar)?.reason
      ?? state.resultFailures.find(item => item.case_dir === c.dir)?.reason ?? 'not evaluated',
  });
  await copyText(rowsToCsv(rows)); status('多站点指标 CSV 已复制');
};
$('diagnose').onclick = diagnoseCurrent;
$('export-report').onclick = generateReport;
$('export-pdf').onclick = exportPdfReport;
$('export-copy').onclick = async () => { await copyText(reportText); status('报告已复制'); };
$('export-download').onclick = () => downloadText(reportText, `colm-results-${new Date().toISOString().slice(0, 10)}.${reportExtension}`);

addEventListener('colm:step', event => {
  // Hidden result panes must not retain uPlot instances or ResizeObserver targets.
  for (const pane of document.querySelectorAll('[data-flow-pane^="result-"][hidden]')) destroyChartsInside(pane);
  if (event.detail?.startsWith('result-')) prepareActivePane();
});
addEventListener('colm:language', () => {
  if (!reportText) $('export-preview').textContent = language() === 'en'
    ? 'No report has been generated yet.' : '还没有生成报告。';
  if (state.step.startsWith('result-')) refreshVars();
});
addEventListener('colm:theme', () => {
  if (state.step === 'result-series' && charts.has($('charts'))) plotSeries();
  if (state.step === 'result-evaluation' && currentComparisonSummary) drawComparison(currentComparisonSummary);
});

// Study panes: keep the browser thin; Rust sidecar validates the science contract.
let studyParamCatalog = [];
let studyParamCasesKey = '';
let tuningDatesInitialized = '';
const studyDirs = { uq: [], tuning: [] };
const studyEvents = { uq: [], tuning: [] };
const studyRunning = { uq: false, tuning: false };
const studyViews = { uq: null, tuning: null };
const studyPages = { uq: 1, tuning: 1 };
const studyWizardPages = { uq: 0, tuning: 0 };
const studyAsyncRequests = { params: 0, outputs: 0, targets: 0 };
const studyWizardTitles = {
  uq: ['试验设计', '输出变量', '参数范围', '预算预览', '创建与运行', '运行状态', '结果'],
  tuning: ['优化设计', '目标变量', '参数范围', '预算预览', '创建与运行', '运行状态', '结果'],
};
const studyWizardHelp = {
  uq: [
    ['先明确要评估哪些参数假设会让哪些输出发生多大变化，再选择方法、预热、样本数、随机种子和并发。', '设计决定成员如何覆盖参数空间、计算成本与可重复性；站点固定共享参数，分析使用每个站点预热后实际写出的全部 history。结果是给定范围内的有限样本情景分位带，不是自动校准或统计置信区间。'],
    ['从当前配置预计写出的标量 history 中，选择要形成不确定性包络的输出。', '只分析与科学问题相关且各站点可用的变量，可避免无效运行和无法比较的结果。'],
    ['选择参与扰动的参数，填写有限采样上下界与线性/对数尺度，并确认范围责任。', '参数范围就是不确定性假设；过宽会产生非物理解，过窄则会低估结果敏感性。'],
    ['汇总参数数、候选数、站点数、运行阶段和并发，预览本次 Study 的计算规模。', '在创建前看清任务量，便于控制计算时间、磁盘占用和样本预算。'],
    ['先创建冻结内核与算例副本的 Study，再启动、暂停、恢复、重试或导出任务。', '创建和运行分开可先审查可复现清单；所有成员独立运行，不会修改原算例。'],
    ['查看 baseline 与各成员的阶段进度、成功/失败状态和筛选后的事件日志。', '运行状态用于定位失败成员并安全暂停或重试，而不是盲目重复整批计算。'],
    ['查看输出分位数包络、参数影响排序和成员明细，并按站点与变量加载图表。', '这些结果用于判断预测区间、主要不确定性来源，以及下一步应优先约束哪些参数。'],
  ],
  tuning: [
    ['选择目标指标、最少有效配对数、站点组织方式、校准/验证时段、种群和代数。', '这些设置定义优化问题、数据证据门槛和搜索成本；验证期独立检验参数泛化能力。'],
    ['对照模型计划输出与观测文件，选择参与目标函数的变量并设置权重。', '目标与权重决定优化器在不同过程间如何取舍；缺测或不可评估变量不能当作零误差。'],
    ['选择当前物理方案真正读取的参数，填写有限范围和采样尺度，并确认范围责任。', '有效且物理合理的搜索边界能减少无效候选，避免优化器找到数值上好但不可解释的解。'],
    ['预览种群 ×（代数 + 1）形成的候选数，以及多站点和三阶段带来的总运行量。', '调优成本会快速放大；预算预览帮助在搜索充分性与可用算力之间做取舍。'],
    ['创建冻结输入与内核的调优 Study，再运行差分进化；最佳候选只能另存为新算例。', '冻结与复制保证每个候选可复现，并保护原算例不被搜索过程覆盖。'],
    ['跟踪 baseline、候选成员、校准与评估状态，查看失败原因并执行暂停、恢复或重试。', '实时状态让你区分模型失败、数据不足和正常搜索进展，避免错误应用未完成结果。'],
    ['比较候选目标函数、最佳成员、校准/验证表现和成员表，并预览最佳参数改动。', '结果页用于确认改进是否跨验证期成立，再决定是否把最佳候选另存并进入后续模拟。'],
  ],
};
const studyLogFilters = { uq: { study_dir: '', member: '', site: '', stage: '' }, tuning: { study_dir: '', member: '', site: '', stage: '' } };
const studyCpuCapacity = Math.max(1, Number(globalThis.navigator?.hardwareConcurrency) || 1);
const parentDir = p => String(p || '').replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, '');
const caseName = c => c.name || String(c.dir || '').split(/[\\/]/).pop();
const oneDay = 86400;
// Study 自己运行 baseline；它的输入是本次已经建好的算例，不是“已经有结果”的
// 子集。若用户在结果总览明确缩小过分析范围，则继续尊重那个选择。
const studyScope = () => {
  const cases = allCurrent();
  return state.resultSelectionTouched
    ? cases.filter(c => state.resultSelection.has(c.dir))
    : cases;
};
const studyScopeKey = () => studyScope()
  .map(c => `${c.dir}\u001f${observationFor(c)}`)
  .join('\u001e');
const activeStudyDirs = kind => scopedStudyDirs(studyDirs[kind] || [], studyScope().map(item => item.dir));
const setActiveStudyDirs = (kind, dirs) => {
  studyDirs[kind] = replaceScopedStudyDirs(studyDirs[kind] || [], studyScope().map(item => item.dir), dirs);
};
const currentKernel = () => $('kernel')?.value || '';
const uqSpinupTarget = () => studyScope().map(c => c.dir);
const setPreview = (kind, text) => { const el = $(kind === 'tuning' ? 'tune-preview' : 'uq-preview'); if (el) el.textContent = text; };
const studyLabel = kind => kind === 'tuning' ? '参数调优' : '不确定性分析';

async function renderUqSpinup(stillCurrent = () => true) {
  const note = $('uq-spinup-note');
  const dirs = uqSpinupTarget();
  if (!note) return;
  if (!dirs.length) {
    for (const id of ['uq-spinup-years', 'uq-spinup-repeat']) if ($(id)) $(id).value = '0';
    note.textContent = '创建算例后显示预热设置。';
    return;
  }
  if (!hasBackend) return;
  try {
    const t = await invoke('read_timing', { dirs });
    if (!stillCurrent()) return;
    if ($('uq-spinup-years')) $('uq-spinup-years').value = String(Math.max(0, Number(t.spinup_years) || 0));
    if ($('uq-spinup-repeat')) $('uq-spinup-repeat').value = String(Math.max(0, Number(t.spinup_repeat) || 0));
    note.textContent = t.spinup_varies
      ? `这 ${t.count} 个算例的预热设置不一致；应用后会统一。`
      : (t.spinup_repeat ? `当前：每轮 ${t.spinup_years} 年，重复 ${t.spinup_repeat} 轮；预热期不写 history。` : '当前未启用模型预热。');
  } catch (error) { if (stillCurrent()) note.textContent = `无法读取预热设置：${error?.message || error}`; }
}

async function applyUqSpinup() {
  const dirs = uqSpinupTarget();
  if (!dirs.length) return status('没有已建算例可设置预热。');
  const years = Number($('uq-spinup-years')?.value);
  const repeat = Number($('uq-spinup-repeat')?.value);
  if (!Number.isSafeInteger(years) || years < 0 || !Number.isSafeInteger(repeat) || repeat < 0) {
    throw new Error('预热年数和重复轮数必须是非负整数。');
  }
  const r = await invoke('set_spinup', { dirs, years, repeat, kernelDir: currentKernel() });
  state.text = r.text;
  await markResultsStale(dirs);
  await renderUqSpinup();
  status(repeat > 0 && years > 0 ? `预热：每轮 ${years} 年，共重复 ${repeat} 轮` : '已关闭预热');
}

function studyWizardIssue(kind, page) {
  const tuning = kind === 'tuning';
  const prefix = tuning ? 'tune' : 'uq';
  const cases = studyScope();
  if (page === 0) {
    if (!cases.length) return '先在“基本设定 / 文件与目录”创建算例';
    if (new Set(cases.map(c => parentDir(c.dir))).size !== 1) return 'Study 中的算例必须位于同一个项目目录';
    if (!currentKernel()) return '当前配置没有匹配的内核运行产物';
    if (tuning && cases.some(c => !observationFor(c))) return '参数调优要求分析范围内每个算例都有观测文件。';
    try { studyDesign(kind); } catch (error) { return error?.message || String(error); }
  }
  if (page === 1) {
    const selections = [...document.querySelectorAll(tuning
      ? '[data-tune-target]:checked:not(:disabled)'
      : '[data-uq-output]:checked:not(:disabled)')];
    if (!selections.length) return tuning ? '至少选择一个可评估目标' : '至少选择一个输出变量';
    if (tuning && $('tune-site-mode')?.value === 'independent') {
      const uncovered = cases.filter(c => !selections.some(input => (input.dataset[tuning ? 'targetSites' : 'outputSites'] || '')
        .split('\u001f').includes(studySiteId(c))));
      if (uncovered.length) return `以下算例没有选中的适用${tuning ? '目标' : '输出变量'}：${uncovered.map(caseName).join('、')}`;
    }
  }
  if (page === 2) {
    try {
      if (!selectedStudyParams(`${prefix}-params`).length) return '至少选择一个参数并填写有限范围';
    } catch (error) { return error?.message || String(error); }
    if (!$(`${prefix}-range-confirm`)?.checked) return '检查范围后勾选责任确认';
  }
  if ((page === 4 || page === 5) && !activeStudyDirs(kind).length) return '请先创建 Study。';
  return '';
}

function renderStudyWizard(kind) {
  const prefix = kind === 'tuning' ? 'tune' : 'uq';
  const steps = [...document.querySelectorAll(`[data-study-wizard="${kind}"]`)]
    .sort((a, b) => Number(a.dataset.studyStep) - Number(b.dataset.studyStep));
  if (!steps.length) return;
  const page = Math.max(0, Math.min(steps.length - 1, studyWizardPages[kind] || 0));
  studyWizardPages[kind] = page;
  steps.forEach((step, index) => { step.hidden = index !== page; });
  const title = studyWizardTitles[kind][page];
  const help = studyWizardHelp[kind][page];
  $(`${prefix}-step-progress`).textContent = language() === 'en'
    ? `Page ${page + 1}/${steps.length} · ${dialogText(title)}`
    : `第 ${page + 1}/${steps.length} 页 · ${title}`;
  $(`${prefix}-step-do`).textContent = dialogText(help[0]);
  $(`${prefix}-step-why`).textContent = dialogText(help[1]);
  const previous = $(`${prefix}-step-prev`);
  const next = $(`${prefix}-step-next`);
  const issue = page < steps.length - 1 ? studyWizardIssue(kind, page) : '';
  previous.disabled = page === 0;
  next.disabled = page === steps.length - 1 || !!issue;
  next.title = issue ? dialogText(issue) : '';
  $(`${prefix}-step-note`).textContent = issue ? dialogText(issue) : '';
}

function setStudyWizardPage(kind, page) {
  studyWizardPages[kind] = page;
  renderStudyWizard(kind);
  document.querySelector(`[data-study-wizard="${kind}"][data-study-step="${page}"]`)?.scrollIntoView({ block: 'start' });
}

function studyJobCount(kind) {
  const input = $(kind === 'tuning' ? 'tune-jobs' : 'uq-jobs');
  const jobs = Math.max(1, Math.min(studyCpuCapacity, Math.trunc(Number(input?.value)) || 1));
  if (input) { input.max = String(studyCpuCapacity); input.value = String(jobs); }
  return jobs;
}

function saveStudyDirs() {
  try { localStorage.setItem('colm.studyDirs', JSON.stringify(studyDirs)); } catch {}
}
function restoreStudyDirs() {
  try {
    const raw = JSON.parse(localStorage.getItem('colm.studyDirs') || '{}');
    for (const kind of ['uq', 'tuning']) studyDirs[kind] = Array.isArray(raw[kind]) ? raw[kind] : (raw[kind] ? [raw[kind]] : []);
  } catch {}
}
restoreStudyDirs();

function kindForStudyDir(dir) {
  for (const kind of ['uq', 'tuning']) if ((studyDirs[kind] || []).includes(dir)) return kind;
  return state.step === 'result-tuning' ? 'tuning' : 'uq';
}

async function loadStudyParams(stillCurrent = () => true) {
  const request = ++studyAsyncRequests.params;
  const current = () => request === studyAsyncRequests.params && stillCurrent();
  if (!hasBackend) throw new Error('后端未连接');
  const cases = studyScope();
  const key = cases.map(c => c.dir).join('\u001f') + '\u001e' + currentKernel() + `\u001e${state.expert}`;
  if (!studyParamCatalog.length || studyParamCasesKey !== key) {
    if (studyParamCasesKey && studyParamCasesKey !== key) {
      for (const id of ['uq-range-confirm', 'tune-range-confirm']) if ($(id)) $(id).checked = false;
    }
    const catalog = JSON.parse(await invoke('study_params'));
    if (!current()) return studyParamCatalog;
    let states = new Map();
    if (cases.length && currentKernel()) {
      const rows = await invoke('field_states_batch', { dirs: cases.map(c => c.dir), kernelDir: currentKernel() });
      if (!current()) return studyParamCatalog;
      states = new Map(rows.map(item => [item.name, item]));
    }
    studyParamCatalog = catalog
      .map(p => ({ ...p, state: states.get(p.name) }))
      .filter(p => !p.state || (p.state.mode === 'editable' && !p.state.mixed))
      .filter(p => state.expert || p.review !== 'expert_range_only');
    studyParamCasesKey = key;
  }
  if (!current()) return studyParamCatalog;
  renderStudyParams('uq-params');
  renderStudyParams('tune-params');
  renderStudyBudget('uq');
  renderStudyBudget('tuning');
  return studyParamCatalog;
}

/**
 * 在 baseline 尚未运行时，从 case.nml + 内核闸门得到“计划写出”的标量变量。
 * 复用运行页同一个 `hist_vars` 后端；只保留结果工作台已知可画成单序列的变量，
 * 避免把土层数组误交给只接受标量时间序列的 Study 聚合器。
 */
async function plannedHistoryCatalog(c) {
  const text = await invoke('read_text', { path: `${c.dir}/case.nml` });
  const variables = await invoke('hist_vars', { text, kernelDir: currentKernel() });
  return {
    planned: true,
    variables: variables
      .filter(variable => variable.on && variable.writable === true)
      .map(variable => `f_${variable.name}`)
      .filter(name => COMMON_VARIABLES[name] && !PLANNED_PROFILE_VARIABLES.has(name))
      .map(name => ({ name, units: COMMON_VARIABLES[name][1], kind: 'series' })),
  };
}

async function initializeTuningDatesFromCases(cases, isCurrent = () => true) {
  if (!cases.length || tuningDatesInitialized === studyScopeKey()) return;
  try {
    const timing = await invoke('read_timing', { dirs: cases.map(c => c.dir) });
    if (!isCurrent()) return;
    const start = Date.parse(`${timing.output_start || timing.start}T00:00:00Z`) / 1000;
    const end = Date.parse(`${timing.end}T00:00:00Z`) / 1000;
    if (Number.isFinite(start) && Number.isFinite(end) && start <= end) {
      initializeTuningDates([{ start, end }]);
    }
  } catch {}
}

function renderStudyParams(hostId) {
  const host = $(hostId);
  if (!host) return;
  const previous = new Map([...host.querySelectorAll('[data-study-param]')].map(input => [input.dataset.studyParam, {
    checked: input.checked,
    min: host.querySelector(`[data-study-min="${CSS.escape(input.dataset.studyParam)}"]`)?.value ?? '',
    max: host.querySelector(`[data-study-max="${CSS.escape(input.dataset.studyParam)}"]`)?.value ?? '',
    scale: host.querySelector(`[data-study-scale="${CSS.escape(input.dataset.studyParam)}"]`)?.value ?? '',
  }]));
  const tuning = hostId.startsWith('tune');
  const invalidateConfirmation = () => {
    const confirmation = $(tuning ? 'tune-range-confirm' : 'uq-range-confirm');
    if (confirmation) confirmation.checked = false;
    renderStudyBudget(tuning ? 'tuning' : 'uq');
  };
  host.textContent = '';
  for (const p of studyParamCatalog) {
    const saved = previous.get(p.name);
    const row = node('label', 'evaluation-variable study-param-option');
    const label = fieldLabel(p.name, language());
    const input = document.createElement('input');
    input.type = 'checkbox'; input.dataset.studyParam = p.name;
    input.checked = saved?.checked ?? false;
    input.onchange = invalidateConfirmation;
    const text = node('span');
    const lower = p.min == null ? '−∞' : `${p.min_inclusive ? '≥' : '>'} ${metricText(p.min)}`;
    const upper = p.max == null ? '+∞' : `${p.max_inclusive ? '≤' : '<'} ${metricText(p.max)}`;
    const sentinel = p.sentinel == null ? '' : ` · 哨兵 ${metricText(p.sentinel)}（${p.sentinel_meaning || '使用内核默认值'}，不可采样）`;
    text.append(node('b', '', label), node('small', '', `${p.name} · 代码默认 ${metricText(p.default)} · 硬边界 ${lower}, ${upper}${sentinel} · 仅专家自定义范围`));
    const min = node('input', 'input'); min.type = 'number'; min.step = 'any'; min.placeholder = '最小值'; min.dataset.studyMin = p.name; min.value = saved?.min ?? '';
    const max = node('input', 'input'); max.type = 'number'; max.step = 'any'; max.placeholder = '最大值'; max.dataset.studyMax = p.name; max.value = saved?.max ?? '';
    const scale = node('select', 'select'); scale.dataset.studyScale = p.name;
    for (const [value, label] of [['linear', language() === 'en' ? 'Linear' : '线性'], ['log', language() === 'en' ? 'Logarithmic' : '对数']]) {
      const option = document.createElement('option'); option.value = value; option.textContent = label; scale.appendChild(option);
    }
    scale.value = saved?.scale || p.scale || 'linear';
    min.oninput = max.oninput = scale.onchange = invalidateConfirmation;
    row.append(input, text, scale, min, max);
    host.appendChild(row);
  }
  if (!studyParamCatalog.length) host.appendChild(node('div', 'result-empty', language() === 'en'
    ? 'No reviewed normal-mode ranges are available. Switch to Expert mode to enter explicit finite ranges.'
    : '当前没有已审核的普通模式采样范围；请切换到专家模式并显式填写有限范围。'));
}

function selectedStudyParams(hostId) {
  return [...$(hostId).querySelectorAll('[data-study-param]:checked')].map(cb => {
    const name = cb.dataset.studyParam;
    const meta = studyParamCatalog.find(parameter => parameter.name === name);
    const label = fieldLabel(name, language());
    const minInput = $(hostId).querySelector(`[data-study-min="${CSS.escape(name)}"]`);
    const maxInput = $(hostId).querySelector(`[data-study-max="${CSS.escape(name)}"]`);
    if (!minInput?.value.trim() || !maxInput?.value.trim()) throw new Error(`${label} 需要填写上下界。`);
    const min = Number(minInput.value);
    const max = Number(maxInput.value);
    if (!Number.isFinite(min) || !Number.isFinite(max) || min >= max) throw new Error(`${label} 需要有限且 min < max 的范围。`);
    const scale = $(hostId).querySelector(`[data-study-scale="${CSS.escape(name)}"]`)?.value || 'linear';
    if (scale === 'log' && (min <= 0 || max <= 0)) throw new Error(`${label} 使用对数采样时上下界必须大于 0。`);
    const below = meta?.min != null && (min < meta.min || (!meta.min_inclusive && min === meta.min));
    const above = meta?.max != null && (max > meta.max || (!meta.max_inclusive && max === meta.max));
    if (below || above) throw new Error(`${label} 的采样范围超出代码硬边界。`);
    if (meta?.sentinel != null && (min === meta.sentinel || max === meta.sentinel)) throw new Error(`${label} 不能把哨兵值用作采样边界。`);
    return { name, sample_min: min, sample_max: max, scale };
  });
}

async function renderStudyOutputs(stillCurrent = () => true) {
  const request = ++studyAsyncRequests.outputs;
  const current = () => request === studyAsyncRequests.outputs && stillCurrent();
  const host = $('uq-outputs');
  if (!host) return;
  const previous = new Set([...host.querySelectorAll('[data-uq-output]:checked')].map(input => input.dataset.uqOutput));
  const hadSelection = host.querySelector('[data-uq-output]') !== null;
  host.textContent = '';
  const byName = new Map();
  const failures = [];
  const cases = studyScope();
  for (const c of cases) {
    try {
      const catalog = !c.has_history || isStaleResult(c) || isActiveResult(c)
        ? await plannedHistoryCatalog(c)
        : await loadCatalog(c);
      if (!current()) return;
      for (const v of catalog.variables || []) {
        if (v.kind !== 'series' || v.name === 'time') continue;
        const current = byName.get(v.name) || { variable: v, n: 0, sites: [], planned: false };
        current.n += 1;
        current.sites.push(studySiteId(c));
        current.planned ||= catalog.planned === true;
        byName.set(v.name, current);
      }
    } catch (error) { failures.push(`${caseName(c)}：${error?.message || error}`); }
  }
  if (!current()) return;
  const rows = [...byName.values()]
    .filter(row => row.n === cases.length)
    .sort((a, b) => Number(!COMMON_VARIABLES[a.variable.name]) - Number(!COMMON_VARIABLES[b.variable.name]) || a.variable.name.localeCompare(b.variable.name));
  for (const { variable: v, n, sites, planned } of rows) {
    const meta = variableMeta(v.name, v.units);
    const row = node('label', 'evaluation-variable');
    const input = document.createElement('input');
    input.type = 'checkbox'; input.dataset.uqOutput = v.name;
    input.dataset.outputSites = sites.join('\u001f');
    input.checked = hadSelection ? previous.has(v.name) : v.name === 'f_rnet';
    input.onchange = () => renderStudyBudget('uq');
    const text = node('span');
    text.append(node('b', '', meta.label), node('small', '', `${v.name} · ${meta.units} · 覆盖 ${n}/${cases.length} · ${planned ? '按当前配置预计写出' : '已在 history 中确认'}`));
    row.append(input, text);
    host.appendChild(row);
  }
  if (!rows.length) host.appendChild(node('div', 'result-empty', '当前分析范围没有所有站点共同的 history 时间序列变量。'));
  if (failures.length) host.appendChild(node('div', 'warn mini', `以下已有结果未通过 history 检查，不会退回计划值：${failures.join('；')}`));
}

async function renderTuningTargets(stillCurrent = () => true) {
  const request = ++studyAsyncRequests.targets;
  const current = () => request === studyAsyncRequests.targets && stillCurrent();
  const host = $('tune-targets');
  if (!host) return;
  const previous = new Map([...host.querySelectorAll('[data-tune-target]')].map(input => [input.dataset.tuneTarget, {
    checked: input.checked,
    weight: host.querySelector(`[data-tune-weight="${CSS.escape(input.dataset.tuneTarget)}"]`)?.value ?? '1',
  }]));
  const hadSelection = previous.size > 0;
  const cases = studyScope();
  if (!cases.length || cases.some(c => !observationFor(c))) {
    host.innerHTML = '<div class="result-empty">参数调优要求分析范围内每个算例都有观测文件。</div>';
    renderStudyBudget('tuning');
    return;
  }
  const rows = await boundedMap(cases, Math.min(4, cases.length), async c => ({
    case: c,
    catalog: !c.has_history || isStaleResult(c) || isActiveResult(c)
      ? JSON.parse(await invoke('evaluation_plan', {
          case: c.dir, obs: observationFor(c), kernelDir: currentKernel(),
        }))
      : await loadEvaluationCatalog(c, observationFor(c)),
  }));
  if (!current()) return;
  const historyCatalogs = (await Promise.all(cases.map(c => loadCatalog(c).catch(() => null)))).filter(Boolean);
  if (!current()) return;
  initializeTuningDates(historyCatalogs);
  if (!historyCatalogs.length) await initializeTuningDatesFromCases(cases, current);
  if (!current()) return;
  const counts = new Map();
  for (const result of rows.filter(r => r.ok)) for (const v of result.value.catalog) {
    const current = counts.get(v.name) || { variable: v, n: 0, sites: [], reasons: new Set() };
    if (v.available) {
      current.n += 1;
      current.sites.push(studySiteId(result.value.case));
    } else {
      const reason = evaluationMissingReason(v);
      if (reason) current.reasons.add(reason);
    }
    counts.set(v.name, current);
  }
  const failures = rows.filter(row => !row.ok).map(row => `${caseName(cases[row.index])}：${row.error}`);
  host.textContent = '';
  const independent = $('tune-site-mode')?.value === 'independent';
  for (const { variable: v, n, sites, reasons } of [...counts.values()].sort((a, b) => a.variable.name.localeCompare(b.variable.name))) {
    const label = language() === 'en' ? v.label_en : v.label_zh;
    const saved = previous.get(v.name);
    const row = node('div', 'evaluation-variable');
    const input = document.createElement('input');
    input.type = 'checkbox'; input.dataset.tuneTarget = v.name;
    input.checked = hadSelection ? saved?.checked === true : v.name === 'Qle' || v.name === 'Qh';
    input.dataset.targetSites = sites.join('\u001f');
    input.disabled = independent ? n === 0 : n !== cases.length;
    if (input.disabled) input.checked = false;
    input.onchange = () => renderStudyBudget('tuning');
    const text = node('span');
    const why = reasons.size ? ` · ${[...reasons].join('；')}` : '';
    text.append(node('b', '', label), node('small', '', `${v.name} · ${v.model_var} ↔ ${v.obs_var} · ${n}/${cases.length} 站点${why}`));
    const weightLabel = node('label', 'study-target-weight', '权重 ');
    const weight = node('input', 'input mini-input');
    weight.type = 'number'; weight.min = '0.000001'; weight.step = '0.1'; weight.value = saved?.weight ?? '1';
    weight.dataset.tuneWeight = v.name; weight.setAttribute('aria-label', `${label} 权重`);
    weight.oninput = () => renderStudyBudget('tuning');
    weightLabel.appendChild(weight);
    row.append(input, text, weightLabel);
    host.appendChild(row);
  }
  if (!counts.size) host.appendChild(node('div', 'result-empty', '当前观测文件没有共同可评估变量。'));
  if (failures.length) host.appendChild(node('div', 'warn mini', `以下已有结果未通过评估目录检查，不会退回计划值：${failures.join('；')}`));
  renderStudyBudget('tuning');
}

function unixDate(id, message = '日期窗口需要开始和结束日期。') {
  const value = $(id)?.value;
  if (!value) throw new Error(message);
  return Math.trunc(new Date(`${value}T00:00:00Z`).getTime() / 1000);
}

function dateValue(unix) { return new Date(unix * 1000).toISOString().slice(0, 10); }

function initializeTuningDates(catalogs) {
  const scopeKey = studyScopeKey();
  if (tuningDatesInitialized === scopeKey || !catalogs.length) return;
  const start = Math.max(...catalogs.map(c => Number(c.start)).filter(Number.isFinite));
  const last = Math.min(...catalogs.map(c => Number(c.end)).filter(Number.isFinite));
  if (!Number.isFinite(start) || !Number.isFinite(last) || start >= last) return;
  const end = last + oneDay;
  const split = Math.trunc(start + (end - start) * 0.75);
  $('tune-from').value = dateValue(start);
  $('tune-to').value = dateValue(split);
  $('tune-val-from').value = dateValue(split);
  $('tune-val-to').value = dateValue(end);
  tuningDatesInitialized = scopeKey;
}

function renderStudyBudget(kind) {
  const tuning = kind === 'tuning';
  const host = $(tuning ? 'tune-budget' : 'uq-budget');
  if (!host) return;
  let paramCount = 0;
  try { paramCount = selectedStudyParams(tuning ? 'tune-params' : 'uq-params').length; } catch {}
  const siteCount = studyScope().length;
  const budget = tuning
    ? studyBudget({ method: 'de', paramCount, siteCount, population: Number($('tune-pop')?.value), generations: Number($('tune-gen')?.value), jobs: studyJobCount('tuning') })
    : studyBudget({ method: $('uq-method')?.value, paramCount, siteCount, candidates: $('uq-method')?.value === 'lhs' ? Number($('uq-count')?.value) : null, jobs: studyJobCount('uq') });
  host.textContent = `参数 ${paramCount} · 站点 ${siteCount} · 候选 ${budget.candidateCount} · 成员×站点 ${budget.memberSiteTasks} · 阶段运行 ${budget.totalStageRuns} · 并发 ${budget.jobs} · 预计时间未知（暂无基准实测） · 磁盘需求未知（暂无基准产物大小）`;
  renderStudyReadiness(kind);
}

function renderStudyReadiness(kind) {
  const tuning = kind === 'tuning';
  const en = language() === 'en';
  const prefix = tuning ? 'tune' : 'uq';
  const host = $(`${prefix}-readiness`);
  if (!host) return;
  const cases = studyScope();
  const roots = new Set(cases.map(c => parentDir(c.dir)));
  let parameterCount = 0;
  try { parameterCount = selectedStudyParams(`${prefix}-params`).length; } catch {}
  const selections = [...document.querySelectorAll(tuning
    ? '[data-tune-target]:checked:not(:disabled)'
    : '[data-uq-output]:checked:not(:disabled)')];
  const selectionCount = selections.length;
  const independent = tuning && $('tune-site-mode')?.value === 'independent';
  const uncovered = independent ? cases.filter(c => !selections.some(input => {
    const sites = input.dataset[tuning ? 'targetSites' : 'outputSites'] || '';
    return sites.split('\u001f').includes(studySiteId(c));
  })) : [];
  const confirmed = $(`${prefix}-range-confirm`)?.checked === true;
  const observations = tuning ? cases.filter(c => observationFor(c)).length : cases.length;
  const dateValues = tuning
    ? ['tune-from', 'tune-to', ...($('tune-validation')?.checked === false ? [] : ['tune-val-from', 'tune-val-to'])]
    : [];
  const datesReady = dateValues.every(id => $(id)?.value);
  const checks = [
    { ok: cases.length > 0, text: cases.length ? (en ? `${cases.length} base case(s) selected` : `已选择 ${cases.length} 个基础算例`) : (en ? 'Create a case in Basic setup / Files and directories first' : '先在“基本设定 / 文件与目录”创建算例') },
    { ok: roots.size === 1, text: roots.size === 1 ? (en ? 'Cases share one project directory' : '算例位于同一个项目目录') : (en ? 'Study cases must share one project directory' : 'Study 中的算例必须位于同一个项目目录') },
    { ok: !!currentKernel(), text: currentKernel() ? (en ? 'Matching physics kernel is available' : '已匹配当前物理内核') : (en ? 'No matching kernel build is available' : '当前配置没有匹配的内核运行产物') },
  ];
  if (tuning) checks.push({
    ok: observations === cases.length && cases.length > 0,
    text: observations === cases.length && cases.length > 0
      ? (en ? `Observations matched for all ${cases.length} case(s)` : `全部 ${cases.length} 个算例已匹配观测`)
      : (en ? `Observation files ${observations}/${cases.length}; every case requires one` : `观测文件 ${observations}/${cases.length}；每个算例都必须有观测`),
  });
  checks.push(
    { ok: parameterCount > 0, text: parameterCount ? (en ? `${parameterCount} parameter(s) have valid ranges` : `已选择 ${parameterCount} 个参数并填写有效范围`) : (en ? 'Select at least one parameter and enter finite ranges' : '至少选择一个参数并填写有限范围') },
    { ok: selectionCount > 0 && uncovered.length === 0, text: uncovered.length
      ? (en ? `No selected applicable ${tuning ? 'target' : 'output'} for: ${uncovered.map(caseName).join(', ')}` : `以下算例没有选中的适用${tuning ? '目标' : '输出变量'}：${uncovered.map(caseName).join('、')}`)
      : selectionCount ? (en ? `${selectionCount} ${tuning ? 'target(s)' : 'output variable(s)'} selected` : `已选择 ${selectionCount} 个${tuning ? '目标' : '输出变量'}`) : (en ? `Select at least one ${tuning ? 'evaluable target' : 'output variable'}` : `至少选择一个${tuning ? '可评估目标' : '输出变量'}`) },
    { ok: confirmed, text: confirmed ? (en ? 'Sampling-range responsibility confirmed' : '已确认采样范围责任') : (en ? 'Review the ranges and confirm responsibility' : '检查范围后勾选责任确认') },
  );
  if (tuning) checks.push({ ok: datesReady, text: datesReady ? (en ? 'Calibration/validation windows are filled' : '校准/验证窗口已填写') : (en ? 'Fill the calibration period and any enabled validation period' : '填写校准期及启用的验证期') });
  host.replaceChildren(...checks.map(check => node('div', `study-ready-item ${check.ok ? 'pass' : 'warn'}`, check.text)));
  const create = $(`${prefix}-create`);
  if (create) create.disabled = checks.some(check => !check.ok);
  const hasStudy = activeStudyDirs(kind).length > 0;
  for (const action of ['run', 'status', 'retry', 'pause', 'resume', 'cancel', 'export-study']) {
    const button = $(`${prefix}-${action}`);
    if (button) button.disabled = !hasStudy;
  }
  if (tuning && $('tune-apply-best')) $('tune-apply-best').disabled = !hasStudy;
  renderStudyWizard(kind);
}

function studyDesign(kind) {
  if (kind === 'tuning') {
    const from = unixDate('tune-from', '调优目标需要校准期开始和结束日期。');
    const to = unixDate('tune-to', '调优目标需要校准期开始和结束日期。');
    const useValidation = $('tune-validation')?.checked !== false;
    const validation_from = useValidation ? unixDate('tune-val-from', '调优目标需要验证期开始和结束日期。') : undefined;
    const validation_to = useValidation ? unixDate('tune-val-to', '调优目标需要验证期开始和结束日期。') : undefined;
    if (from >= to || (useValidation && validation_from >= validation_to)) throw new Error('校准期/验证期必须满足开始 < 结束。');
    if (useValidation && !(validation_to <= from || validation_from >= to)) throw new Error('校准期与验证期不能重叠。');
    const minPairs = Number($('tune-min-pairs')?.value);
    if (!Number.isInteger(minPairs) || minPairs < 2) throw new Error('最少配对样本数必须是至少 2 的整数。');
    const population = Number($('tune-pop')?.value);
    const generations = Number($('tune-gen')?.value);
    if (!Number.isInteger(population) || population < 4) throw new Error('种群必须是至少 4 的整数。');
    if (!Number.isInteger(generations) || generations < 1) throw new Error('代数必须是至少 1 的整数。');
    if (!Number.isSafeInteger(population * (generations + 1)) || population * (generations + 1) > MAX_STUDY_CANDIDATES) {
      throw new Error(`候选成员数必须不超过 ${MAX_STUDY_CANDIDATES}。`);
    }
    return { from, to, validation_from, validation_to, minPairs, population, generations };
  }
  const method = $('uq-method')?.value || 'lhs';
  const candidate_count = method === 'lhs' ? Number($('uq-count')?.value) : undefined;
  if (method === 'lhs' && (!Number.isInteger(candidate_count) || candidate_count < 1 || candidate_count > MAX_STUDY_CANDIDATES)) {
    throw new Error(`样本数必须是 1 至 ${MAX_STUDY_CANDIDATES} 的整数。`);
  }
  const seedText = $('uq-seed')?.value.trim() || '';
  const seed = method === 'lhs' ? Number(seedText) : 1;
  if (method === 'lhs' && (!seedText || !Number.isSafeInteger(seed) || seed < 0)) throw new Error('随机种子必须是非负安全整数。');
  return { method, candidate_count, seed };
}

function studySpec(kind, cases, independent = false) {
  const tuning = kind === 'tuning';
  const design = studyDesign(kind);
  const kernel_dir = currentKernel() || undefined;
  const observations = Object.fromEntries(cases.map(c => [studySiteId(c), observationFor(c)]).filter(([, obs]) => obs));
  if (tuning && Object.keys(observations).length !== cases.length) throw new Error('参数调优需要每个站点都有观测文件。');
  const parameters = selectedStudyParams(tuning ? 'tune-params' : 'uq-params');
  if (!parameters.length) throw new Error('请至少勾选一个参数，并填写有限的最小/最大值。');
  if (!$(tuning ? 'tune-range-confirm' : 'uq-range-confirm')?.checked) throw new Error('请确认采样范围由用户负责。');
  if (tuning) {
    const targets = [...document.querySelectorAll('[data-tune-target]:checked')]
      .filter(x => cases.every(c => (x.dataset.targetSites || '').split('\u001f').includes(studySiteId(c))))
      .map(input => {
        const name = input.dataset.tuneTarget;
        const weight = Number(document.querySelector(`[data-tune-weight="${CSS.escape(name)}"]`)?.value);
        if (!Number.isFinite(weight) || weight <= 0) throw new Error(`${name} 的权重必须是正数。`);
        return { key: name, variable: name, metric: $('tune-metric')?.value || 'nrmse', weight, min_pairs: design.minPairs, from: design.from, to: design.to, validation_from: design.validation_from, validation_to: design.validation_to };
      });
    if (!targets.length) throw new Error('参数调优至少选择一个目标变量。');
    return {
      kind: 'tuning', method: 'differential-evolution', seed: Number($('tune-seed')?.value || 1), kernel_dir,
      base_cases: cases.map(c => c.dir), observations, parameters, site_mode: independent ? 'independent' : 'shared',
      targets,
      budget: { population: design.population, generations: design.generations, jobs: studyJobCount('tuning') },
    };
  }
  const outputs = [...document.querySelectorAll('[data-uq-output]:checked')]
    .filter(input => cases.every(c => (input.dataset.outputSites || '').split('\u001f').includes(studySiteId(c))))
    .map(input => input.dataset.uqOutput);
  if (!outputs.length) throw new Error(`不确定性分析至少需要一个适用于 ${cases.map(caseName).join('、')} 的输出变量。`);
  return {
    kind: 'uncertainty', method: design.method, seed: design.seed, kernel_dir,
    base_cases: cases.map(c => c.dir), parameters, outputs, site_mode: 'shared',
    budget: { candidate_count: design.candidate_count, jobs: studyJobCount('uq') },
  };
}

async function createStudy(kind) {
  const cases = studyScope();
  if (!cases.length) return status('没有已建算例可用于 Study。');
  const roots = new Set(cases.map(c => parentDir(c.dir)));
  if (roots.size !== 1) return status('Study 需要同一算例根目录下的算例。');
  await loadStudyParams();
  if (kind === 'tuning') await renderTuningTargets(); else await renderStudyOutputs();
  const independent = kind === 'tuning' && $('tune-site-mode')?.value === 'independent';
  const groups = independent ? cases.map(c => [c]) : [cases];
  const plans = groups.map(group => ({
    caseRoot: parentDir(group[0].dir),
    specJson: JSON.stringify(studySpec(kind, group, independent)),
  }));
  const candidateCounts = plans.map(plan => {
    const spec = JSON.parse(plan.specJson);
    return spec.method === 'differential-evolution'
      ? Number(spec.budget.population) * (Number(spec.budget.generations) + 1)
      : Number(spec.budget.candidate_count || (spec.method === 'oat' ? spec.parameters.length * 2 : Math.max(40, spec.parameters.length * 10)));
  });
  const maxCandidates = Math.max(...candidateCounts);
  const totalCandidates = candidateCounts.reduce((sum, count) => sum + count, 0);
  if (!Number.isSafeInteger(maxCandidates) || maxCandidates > MAX_STUDY_CANDIDATES) {
    throw new Error(`候选成员数必须不超过 ${MAX_STUDY_CANDIDATES}。`);
  }
  if (totalCandidates > 200 && !globalThis.confirm?.(`本次共会创建 ${totalCandidates} 个候选成员，可能耗时很长。是否继续？`)) return;
  for (const plan of plans) await invoke('study_preflight_json', plan);
  const dirs = [];
  try {
    for (const plan of plans) {
      const out = await invoke('study_create_json', plan);
      dirs.push(out.trim());
    }
  } catch (error) {
    const suffix = dirs.length ? `\n已创建但未登记的 Study：\n${dirs.join('\n')}` : '';
    throw new Error(`${error?.message || error}${suffix}`);
  }
  setActiveStudyDirs(kind, dirs);
  saveStudyDirs();
  renderStudyReadiness(kind);
  setPreview(kind, dirs.join('\n'));
  await refreshStudy(kind);
  status(`${studyLabel(kind)} Study 已创建。`);
}

function renderStudyEnvelope(kind, envelope) {
  const flowKind = envelope.kind_hint || kind || (state.step === 'result-tuning' ? 'tuning' : 'uq');
  const previous = studyViews[flowKind] || {};
  const view = {
    ...previous,
    ...envelope,
    manifest: envelope.manifest || previous.manifest,
    state: envelope.state || previous.state,
    events: envelope.events || previous.events || studyEvents[flowKind],
    kind_hint: flowKind,
  };
  delete view.event_only;
  studyViews[flowKind] = view;
  const summary = aggregateStudy(view.state || view.manifest || view);
  const box = node('div', 'study-status-box');
  box.append(resultKpi(summary.status || '—', '状态'), resultKpi(`${summary.done || 0}/${summary.total || 0}`, '成员'), resultKpi(`${Math.round((summary.progress || 0) * 100)}%`, '进度'));
  const filters = studyLogFilters[flowKind];
  const eventRows = view.events || [];
  const filterBar = node('div', 'result-tools study-log-filters');
  for (const [key, label] of [['study_dir', 'Study'], ['member', '成员'], ['site', '站点'], ['stage', '阶段']]) {
    const select = node('select', 'select');
    const all = document.createElement('option'); all.value = ''; all.textContent = `${label}：全部`; select.appendChild(all);
    const values = [...new Set(eventRows.map(item => typeof item === 'object' ? item?.[key] : '').filter(Boolean))].sort();
    for (const value of values) { const option = document.createElement('option'); option.value = value; option.textContent = value; select.appendChild(option); }
    select.value = values.includes(filters[key]) ? filters[key] : '';
    filters[key] = select.value;
    select.onchange = () => { filters[key] = select.value; renderStudyEnvelope(flowKind, studyViews[flowKind]); };
    filterBar.appendChild(select);
  }
  const log = node('div', 'study-log');
  const visibleEvents = eventRows.filter(item => typeof item !== 'object'
    || Object.entries(filters).every(([key, value]) => !value || item?.[key] === value));
  for (const item of visibleEvents.slice(-120)) log.append(node('div', '', typeof item === 'string' ? item : JSON.stringify(item)));
  if (!visibleEvents.length) log.append(node('div', '', '没有符合筛选条件的日志。'));
  const candidates = view.state?.candidates || view.candidates || {};
  const members = (summary.members || []).map(m => {
    const candidateKey = m.study_key ? `${m.study_key}\u001f${m.member}` : m.member;
    return { id: m.id || m.member, status: m.status, objective: candidates[candidateKey]?.calibration ?? candidates[m.member]?.calibration ?? m.objective ?? m.score, sites: (m.sites || []).length };
  });
  const page = paginate(members, studyPages[flowKind], 40);
  studyPages[flowKind] = page.page;
  const table = document.createElement('table');
  const head = document.createElement('tr'); ['成员', '状态', '目标函数', '站点'].forEach(x => head.appendChild(th(x))); table.appendChild(head);
  for (const m of page.items) { const tr = document.createElement('tr'); tr.append(td(m.id), td(m.status || '—'), td(metricText(m.objective)), td(m.sites)); table.appendChild(tr); }
  const pager = node('div', 'result-tools study-pager');
  if (page.pages > 1) {
    const previousButton = node('button', 'btn-ghost', '上一页'); previousButton.disabled = page.page <= 1;
    const nextButton = node('button', 'btn-ghost', '下一页'); nextButton.disabled = page.page >= page.pages;
    previousButton.onclick = () => { studyPages[flowKind] -= 1; renderStudyEnvelope(flowKind, studyViews[flowKind]); };
    nextButton.onclick = () => { studyPages[flowKind] += 1; renderStudyEnvelope(flowKind, studyViews[flowKind]); };
    pager.append(previousButton, node('span', 'muted mini', `${page.page}/${page.pages}`), nextButton);
  }
  const host = $(flowKind === 'tuning' ? 'tune-study-view' : 'uq-study-view');
  host?.replaceChildren(box, table, pager, filterBar, log);
  if (!envelope.event_only) setPreview(flowKind, JSON.stringify(view, null, 2));
}

function mergeStudyEvent(kind, payload) {
  const view = studyViews[kind];
  const tasks = view?.state?.tasks;
  if (tasks && payload.member && payload.site) {
    const statusByEvent = { task_started: 'running', task_done: 'succeeded', task_failed: 'failed' };
    for (const task of Object.values(tasks)) {
      if (task.member !== payload.member || task.site !== payload.site) continue;
      if (payload.study_dir && task.study_dir && payload.study_dir !== task.study_dir) continue;
      task.status = statusByEvent[payload.kind || payload.type] || task.status;
      if (payload.stage) task.stage = payload.stage;
      if (payload.reason) task.reason = payload.reason;
      if (payload.objective != null) task.objective = payload.objective;
    }
  }
  const eventKind = payload.kind || payload.type;
  if (view?.state && eventKind === 'study_done' && payload.status) view.state.status = payload.status;
  if (view?.state && eventKind === 'study_cancelled') view.state.status = 'cancelled';
  if (view?.state && eventKind === 'study_failed') view.state.status = 'failed';
  renderStudyEnvelope(kind, { ...(view || {}), events: studyEvents[kind], kind_hint: kind, event_only: true });
}

async function studyResultText(dir, path) {
  try { return await invoke('study_result', { studyDir: dir, path }); }
  catch { return null; }
}

async function studyResult(dir, path) {
  const text = await studyResultText(dir, path);
  try { return text == null ? null : JSON.parse(text); }
  catch { return null; }
}

const studyResultPaths = envelope => new Set((envelope.results || []).map(file => typeof file === 'string' ? file : file.path).filter(Boolean));

function resultObjectTable(value) {
  const table = document.createElement('table');
  const head = document.createElement('tr');
  ['成员', '可行', '校准', '验证', '说明'].forEach(label => head.appendChild(th(label)));
  table.appendChild(head);
  for (const [member, row] of Object.entries(value || {}).slice(0, 200)) {
    const tr = document.createElement('tr');
    tr.append(td(member), td(row.feasible ? '✓' : '—'), td(metricText(row.calibration)), td(metricText(row.validation)), td(row.reason || ''));
    table.appendChild(tr);
  }
  return table;
}

function importanceTable(rows) {
  const table = document.createElement('table');
  const head = document.createElement('tr');
  ['站点', '变量', '参数', '方法', '影响', 'n'].forEach(label => head.appendChild(th(label)));
  table.appendChild(head);
  for (const row of (rows || []).slice(0, 300)) {
    const tr = document.createElement('tr');
    tr.append(td(row.site), td(row.variable), td(fieldLabel(row.parameter, language())), td(row.method), td(metricText(row.value)), td(row.n));
    table.appendChild(tr);
  }
  return table;
}

function renderEnvelopeChart(host, data) {
  host.textContent = '';
  const chart = node('div', 'chart');
  const minNEff = Math.min(...(data.n_eff || []).filter(Number.isFinite));
  host.append(node('p', 'muted mini', `${data.site} · ${data.variable} · ${data.interpretation || ''} · n_eff(min)=${Number.isFinite(minNEff) ? minNEff : 0}`), chart);
  const colors = chartColors();
  makeChart(chart, {
    title: `${data.site} · ${data.variable}`,
    series: [
      { label: '时间' },
      { label: 'Baseline', stroke: colors.model, width: 1.4 },
      { label: 'P05', stroke: '#7f8c8d', width: 1 },
      { label: 'P50', stroke: colors.obs, width: 1.3 },
      { label: 'P95', stroke: '#7f8c8d', width: 1 },
    ],
    axes: [{}, { label: data.units || '' }],
  }, [data.time || [], data.baseline || [], data.p05 || [], data.p50 || [], data.p95 || []], 280);
}

async function renderStudyResults(kind, envelopes) {
  const host = $(kind === 'tuning' ? 'tune-results' : 'uq-results');
  if (!host) return;
  host.textContent = '';
  const dirs = activeStudyDirs(kind);
  if (!dirs.length) return host.appendChild(node('div', 'result-empty', '还没有 Study。'));
  for (let index = 0; index < dirs.length; index += 1) {
    const dir = dirs[index];
    const envelope = envelopes[index] || {};
    const files = studyResultPaths(envelope);
    const title = envelope.manifest?.id || dir.split(/[\\/]/).pop();
    if (dirs.length > 1) host.appendChild(node('h3', '', title));

    const primary = kind === 'tuning' ? 'objectives.json' : 'importance.json';
    const data = files.has(primary) ? await studyResult(dir, primary) : null;
    const card = node('div', 'study-result-card');
    card.append(node('h4', '', kind === 'tuning' ? '候选目标函数' : '参数影响排序'));
    if (data) card.append(kind === 'tuning' ? resultObjectTable(data) : importanceTable(data));
    else card.append(node('div', 'muted mini', '运行完成后由后端生成。'));
    host.appendChild(card);

    const members = files.has('members.csv') ? await studyResultText(dir, 'members.csv') : null;
    const memberCard = node('details', 'study-result-card');
    memberCard.append(node('summary', '', '成员表（CSV 预览）'));
    memberCard.append(node('pre', 'report-preview', members ? members.split('\n').slice(0, 62).join('\n') : '运行完成后由后端生成。'));
    host.appendChild(memberCard);

    if (kind === 'uq') {
      const paths = [...files].filter(path => /^envelopes\/.+\.json$/.test(path));
      const envelopeCard = node('div', 'study-result-card');
      envelopeCard.append(node('h4', '', '样本分位带'));
      if (paths.length) {
        const select = node('select', 'select');
        for (const path of paths) { const option = document.createElement('option'); option.value = path; option.textContent = path.replace(/^envelopes\//, '').replace(/\.json$/, ''); select.appendChild(option); }
        const button = node('button', 'btn-ghost', '加载图表');
        const chartHost = node('div', 'study-envelope-view');
        button.onclick = async () => {
          const result = await studyResult(dir, select.value);
          if (result) renderEnvelopeChart(chartHost, result);
        };
        envelopeCard.append(select, button, chartHost);
      } else envelopeCard.append(node('div', 'muted mini', '运行完成后可按站点和变量加载，不会一次读取全部成员 history。'));
      host.appendChild(envelopeCard);
    } else {
      const candidates = Object.values(envelope.state?.candidates || {});
      if (candidates.length) host.appendChild(node('div', 'muted mini', `可行候选 ${candidates.filter(candidate => candidate.feasible).length}/${candidates.length}`));
    }
  }
}

async function refreshStudy(kind) {
  const dirs = activeStudyDirs(kind);
  if (!dirs.length) return status('请先创建 Study。');
  const envelopes = [];
  for (const dir of dirs) {
    try {
      envelopes.push(JSON.parse(await invoke('study_status', { studyDir: dir })));
    } catch (error) {
      const reason = error?.message || String(error);
      envelopes.push({ error: reason, study_dir: dir, state: { status: 'failed', tasks: { error: { member: 'study', site: dir.split(/[\\/]/).pop(), status: 'failed', reason } } }, events: [{ study_dir: dir, study_key: dir, kind: 'study_error', reason }] });
    }
  }
  if (envelopes.length === 1 && !envelopes[0].error) renderStudyEnvelope(kind, { ...envelopes[0], kind_hint: kind });
  else {
    const tasks = Object.fromEntries(envelopes.flatMap((envelope, studyIndex) => Object.entries(envelope.state?.tasks || {}).map(([id, task]) => [`${dirs[studyIndex]}/${id}`, { ...task, study_dir: dirs[studyIndex], study_key: dirs[studyIndex] }])));
    const candidates = Object.fromEntries(envelopes.flatMap((envelope, studyIndex) => Object.entries(envelope.state?.candidates || {}).map(([member, candidate]) => [`${dirs[studyIndex]}\u001f${member}`, candidate])));
    const events = envelopes.flatMap((envelope, studyIndex) => (envelope.events || []).map(event => ({ ...event, study_dir: dirs[studyIndex], study_key: dirs[studyIndex] })));
    renderStudyEnvelope(kind, { state: { status: 'multiple', tasks, candidates }, events, kind_hint: kind });
  }
  studyEvents[kind] = envelopes.flatMap((envelope, studyIndex) => (envelope.events || []).map(event => ({ ...event, study_dir: dirs[studyIndex], study_key: dirs[studyIndex] }))).slice(-300);
  await renderStudyResults(kind, envelopes);
}

async function runStudy(kind) {
  const dirs = activeStudyDirs(kind);
  const kernel = currentKernel();
  if (!dirs.length) return status('请先创建 Study。');
  if (!kernel) return status('请先选择内核。');
  if (studyRunning[kind]) return status(`${studyLabel(kind)} Study 正在运行。`);
  const jobs = studyJobCount(kind);
  studyRunning[kind] = true;
  setStudyWizardPage(kind, 5);
  try {
    const perStudyJobs = dirs.length === 1 ? jobs : 1;
    const results = await boundedMap(dirs, Math.min(jobs, dirs.length), async dir => ({
      dir,
      out: await invoke('study_run', {
        studyDir: dir, kernel, stream: true, jobs: perStudyJobs, retryFailed: false,
      }),
    }));
    await refreshStudy(kind);
    setPreview(kind, results.filter(result => result.ok)
      .map(result => `${result.value.dir}\n${result.value.out}`).join('\n\n'));
    const failed = results.filter(result => !result.ok);
    if (failed.length) throw new Error(failed
      .map(result => `${dirs[result.index]}: ${result.error}`).join('\n'));
    status(`${studyLabel(kind)} Study 运行完成。`);
  } finally {
    studyRunning[kind] = false;
  }
}

async function retryStudy(kind) {
  const dirs = activeStudyDirs(kind);
  if (!dirs.length) return status('请先创建 Study。');
  if (studyRunning[kind]) return status(`${studyLabel(kind)} Study 正在运行，不能重试。`);
  const envelopes = [];
  for (const dir of dirs) envelopes.push(JSON.parse(await invoke('study_status', { studyDir: dir })));
  const needsConfirmation = envelopes.some(envelope => Object.values(envelope.state?.tasks || {})
    .some(task => ['needs_review', 'running', 'evaluating'].includes(task.status)));
  if (needsConfirmation && !globalThis.confirm?.(dialogText('存在无法确认原进程状态的任务。仅在确认原模型进程已经退出后重试，是否继续？'))) return;
  for (const dir of dirs) await invoke('study_retry', { studyDir: dir, includeReview: needsConfirmation });
  await runStudy(kind);
}

async function controlStudy(kind, action) {
  const dirs = activeStudyDirs(kind);
  if (!dirs.length) return status('请先创建 Study。');
  const control = action === 'pause' ? dir => invoke('study_pause', { studyDir: dir })
    : action === 'resume' ? dir => invoke('study_resume', { studyDir: dir })
      : action === 'cancel' ? dir => invoke('study_cancel', { studyDir: dir })
        : null;
  if (!control) throw new Error(`未知 Study 操作：${action}`);
  for (const dir of dirs) await control(dir);
  status(action === 'pause' ? '已请求暂停派发。' : action === 'resume' ? '已恢复派发。' : '已请求取消待运行任务。');
  if (action === 'resume' && !studyRunning[kind]) await runStudy(kind);
  else await refreshStudy(kind);
}

async function exportStudy(kind) {
  const dirs = activeStudyDirs(kind);
  if (!dirs.length) return status('请先创建 Study。');
  const out = window.prompt(dialogText('导出目录'), `${parentDir(dirs[0])}/exports`);
  if (!out) return;
  const exported = [];
  for (const dir of dirs) {
    const destination = dirs.length === 1 ? out : `${out}/${dir.split(/[\\/]/).pop()}`;
    await invoke('study_export', { studyDir: dir, out: destination });
    exported.push(destination);
  }
  setPreview(kind, exported.join('\n'));
}

async function applyBestCandidate() {
  const dirs = activeStudyDirs('tuning');
  if (!dirs.length) return status('请先创建调优 Study。');
  const previews = [];
  const members = [];
  for (const dir of dirs) {
    const envelope = JSON.parse(await invoke('study_status', { studyDir: dir }));
    const member = envelope.state?.best_member;
    if (!member) throw new Error(`${envelope.manifest?.id || dir} 还没有可应用的最佳候选。`);
    const baseCase = envelope.manifest?.spec?.base_cases?.[0] || member;
    const site = studySiteId({ dir: baseCase });
    members.push({ dir, member, site });
    const rows = JSON.parse(await invoke('study_apply_preview', { studyDir: dir, member }));
    previews.push(`${dir}\n${rows.map(row => `${row.site}: ${row.field} ${row.old} -> ${row.new}`).join('\n')}`);
  }
  const previewText = previews.join('\n\n');
  setPreview('tuning', previewText);
  if (!globalThis.confirm?.(`${dialogText('即将应用以下参数改动：')}\n\n${previewText.slice(0, 3000)}`)) return;
  const out = window.prompt(dialogText('另存为算例目录'), `${parentDir(studyScope()[0]?.dir)}/tuned`);
  if (!out) return;
  const created = [];
  for (const { dir, member, site } of members) {
    const destination = dirs.length === 1 ? out : `${out}/${site}`;
    created.push(await invoke('study_apply', { studyDir: dir, member, out: destination, name: `${site}-tuned` }));
  }
  setPreview('tuning', created.join('\n'));
}

function syncStudyKernelLabels() {
  if ($('tune-kernel-dir')) $('tune-kernel-dir').value = currentKernel();
}

function wireStudyButton(id, fn) { const el = $(id); if (el) el.onclick = () => fn().catch(e => status(e.message || e)); }
if (listen) listen('study://event', event => {
  const payload = event.payload || {};
  const dir = payload.study_dir || payload.studyDir || payload.dir;
  const kind = kindForStudyDir(dir);
  const active = new Set(activeStudyDirs(kind));
  if (!dir || !active.has(dir)) return;
  studyEvents[kind] = studyEvents[kind].filter(item => !item?.study_dir || active.has(item.study_dir));
  studyEvents[kind].push(payload);
  studyEvents[kind] = studyEvents[kind].slice(-300);
  const host = $(kind === 'tuning' ? 'tune-study-view' : 'uq-study-view');
  if (host && state.step === (kind === 'tuning' ? 'result-tuning' : 'result-uncertainty')) {
    mergeStudyEvent(kind, payload);
  }
});
wireStudyButton('uq-refresh-params', async () => { await loadStudyParams(); await renderStudyOutputs(); });
wireStudyButton('uq-spinup-apply', applyUqSpinup);
wireStudyButton('tune-refresh-params', async () => { await loadStudyParams(); await renderTuningTargets(); });
wireStudyButton('uq-create', () => createStudy('uq'));
wireStudyButton('tune-create', () => createStudy('tuning'));
wireStudyButton('uq-run', () => runStudy('uq'));
wireStudyButton('tune-run', () => runStudy('tuning'));
wireStudyButton('uq-retry', () => retryStudy('uq'));
wireStudyButton('tune-retry', () => retryStudy('tuning'));
wireStudyButton('uq-status', () => refreshStudy('uq'));
wireStudyButton('tune-status', () => refreshStudy('tuning'));
for (const kind of ['uq', 'tuning']) for (const action of ['pause', 'resume', 'cancel']) wireStudyButton(`${kind === 'tuning' ? 'tune' : 'uq'}-${action}`, () => controlStudy(kind, action));
wireStudyButton('uq-export-study', () => exportStudy('uq'));
wireStudyButton('tune-export-study', () => exportStudy('tuning'));
wireStudyButton('tune-apply-best', applyBestCandidate);
for (const id of ['uq-method', 'uq-count', 'uq-seed', 'uq-jobs', 'tune-pop', 'tune-gen', 'tune-jobs']) if ($(id)) $(id).oninput = $(id).onchange = () => {
  if (id === 'uq-method') for (const target of ['uq-count', 'uq-seed']) $(target).disabled = $('uq-method').value === 'oat';
  renderStudyBudget(id.startsWith('tune') ? 'tuning' : 'uq');
};
if ($('tune-site-mode')) $('tune-site-mode').onchange = () => renderTuningTargets().catch(e => status(e.message || e));
if ($('tune-validation')) $('tune-validation').onchange = () => {
  for (const id of ['tune-val-from', 'tune-val-to']) $(id).disabled = !$('tune-validation').checked;
  renderStudyBudget('tuning');
};
for (const id of ['uq-range-confirm', 'tune-range-confirm']) if ($(id)) $(id).onchange = () => {
  renderStudyBudget(id.startsWith('tune') ? 'tuning' : 'uq');
};
for (const id of ['tune-from', 'tune-to', 'tune-val-from', 'tune-val-to', 'tune-min-pairs']) if ($(id)) {
  $(id).oninput = $(id).onchange = () => renderStudyReadiness(id.startsWith('tune') ? 'tuning' : 'uq');
}
for (const kind of ['uq', 'tuning']) {
  const prefix = kind === 'tuning' ? 'tune' : 'uq';
  $(`${prefix}-step-prev`).onclick = () => setStudyWizardPage(kind, studyWizardPages[kind] - 1);
  $(`${prefix}-step-next`).onclick = () => setStudyWizardPage(kind, studyWizardPages[kind] + 1);
  renderStudyWizard(kind);
}
addEventListener('colm:language', () => {
  renderStudyWizard('uq');
  renderStudyWizard('tuning');
});
addEventListener('colm:mode', () => {
  studyParamCasesKey = '';
  if (state.step === 'result-uncertainty' || state.step === 'result-tuning') prepareActivePane();
});
syncStudyKernelLabels();
refreshVars();
