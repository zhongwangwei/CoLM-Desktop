//! 结果分析工作台。
//!
//! 参数编辑用 `state.selected`，结果浏览用 `state.resultCaseDir`。两者故意分开：
//! 切一张图不该把 20 个算例的参数编辑目标悄悄换成其中一个。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { sourceSite } from './batch.js';
import { go, renderSteps } from './shell.js';
import { metricText } from './metric-format.js';
import { language } from './i18n.js';
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
  f_xy_snow: ['降雪', 'mm/s', '积雪'],
  f_snowdp: ['雪深', 'm', '积雪'],
  f_scv: ['雪水当量', 'kg/m²', '积雪'],
};

const catalogCache = new LruCache(16);
const seriesCache = new LruCache(12);
const metricsCache = new LruCache(180);
const charts = new Map();
let currentMetricRows = [];
let currentComparisonSummary = null;
let comparisonController = null;
let activeSeriesRequest = 0;
let activeMetricRequest = 0;
let activeMetricChartRequest = 0;
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
const completed = () => resultCases(state.cases, state.createdCases, true);
const allCurrent = () => resultCases(state.cases, state.createdCases, false);
const resultScope = () => {
  const done = completed();
  if (!state.resultSelectionTouched) return done;
  return done.filter(c => state.resultSelection.has(c.dir));
};

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
  if (c.has_history || running === '已完成') return 'done';
  return 'waiting';
}

function stateBadge(c) {
  const value = caseState(c);
  return value === 'done' ? badge('已完成', 'pass')
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
  if (/snow|sno|ice/i.test(name)) group = '积雪';
  else if (/soil|soi|zwt|runoff|rnof|qinfl|qover|water/i.test(name)) group = '水文/土壤';
  else if (/urban|roof|wall|imper|perv/i.test(name)) group = '城市';
  else if (/lai|sai|veg|assim|resp|gpp|npp|leaf/i.test(name)) group = '植被/碳氮';
  else if (/rad|rnet|solar|long|short|albedo|fsena|lfevpa|fgrnd/i.test(name)) group = '能量/辐射';
  return { label: bare, units: units || '—', group };
}

function resultKpi(value, label, kind = '') {
  const card = node('div', `result-kpi ${kind}`.trim());
  card.append(node('div', 'value', value), node('div', 'label', label));
  return card;
}

function renderOverview() {
  const cases = allCurrent();
  const done = cases.filter(c => c.has_history);
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
    resultKpi(observed.length, '可与观测评估'),
    resultKpi(Math.max(0, done.length - observed.length), '缺少观测', done.length > observed.length ? 'warn' : ''),
  );

  const query = state.resultCaseSearch.toLowerCase();
  const filter = state.resultStatusFilter;
  const shown = cases.filter(c => !query || c.name.toLowerCase().includes(query))
    .filter(c => filter === 'all'
      || (filter === 'done' && caseState(c) === 'done')
      || (filter === 'failed' && caseState(c) === 'failed')
      || (filter === 'waiting' && !['done', 'failed'].includes(caseState(c)))
      || (filter === 'no-observation' && c.has_history && !observationFor(c)));
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
    check.disabled = !c.has_history;
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
    const hist = td(''); hist.appendChild(c.has_history ? badge('可用', 'pass') : badge('无')); row.appendChild(hist);
    const obs = td(''); obs.appendChild(observationFor(c) ? badge('已匹配', 'pass') : badge('缺少', c.has_history ? 'warn' : '')); row.appendChild(obs);
    row.onclick = () => {
      if (!c.has_history) { status(`${c.name} 还没有 history 结果`); return; }
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

async function loadCatalog(c) {
  const cached = catalogCache.get(c.dir);
  if (cached) return cached;
  const catalog = JSON.parse(await invoke('history_catalog', { case: c.dir }));
  catalogCache.set(c.dir, catalog);
  return catalog;
}

function kindLabel(kind) {
  return { series: '时间序列', profile: '垂直剖面', category: '分类维度', scalar: '标量' }[kind] ?? kind;
}

async function renderDataBrowser() {
  const c = activeCase();
  const table = $('result-variable-table');
  table.textContent = '';
  if (!c) return;
  try {
    const catalog = await loadCatalog(c);
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
    table.appendChild(node('caption', 'warn', String(error)));
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
  const cached = seriesCache.get(key);
  if (cached) return cached;
  const json = await invoke('series', {
    case: c.dir, vars: variable,
    from: options.from ?? null, to: options.to ?? null,
    maxPoints,
  });
  return seriesCache.set(key, JSON.parse(json));
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
    const data = await getSeries(c, variable, {
      from: parseLocalClock($('series-from').value),
      to: parseLocalClock($('series-to').value),
      maxPoints: 2400,
    });
    if (token !== activeSeriesRequest || activeCase()?.dir !== c.dir) return;
    const values = data.vars[variable];
    const meta = variableMeta(variable);
    const colors = chartColors();
    makeChart($('charts'), {
      title: `${c.name} · ${meta.label} · ${data.n}/${data.source_n ?? data.n} 点`,
      series: [{ label: '时间' }, { label: meta.units, stroke: colors.model, width: 1.3 }],
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

async function getMetrics(c, obs, spinup, corrected, summaryOnly = false, pairVar = '', maxPoints = null) {
  const key = metricKey({ caseDir: c.dir, obs, spinup, corrected, summaryOnly, pairVar, maxPoints: maxPoints ?? '' });
  const cached = metricsCache.get(key);
  if (cached) return cached;
  const rows = JSON.parse(await invoke('metrics', {
    case: c.dir, obs, spinup, corrected, summaryOnly,
    pairVar: pairVar || null, maxPoints,
  }));
  return metricsCache.set(key, rows);
}

async function evaluateCurrent() {
  const c = activeCase();
  const obs = $('obs').value.trim();
  if (!c || !obs) { status('要先给当前站点选择观测文件'); return; }
  state.resultObsOverrides.set(c.dir, obs);
  const token = ++activeMetricRequest;
  $('evaluate').disabled = true;
  try {
    const rows = await getMetrics(c, obs, Number($('spinup').value) || 0, $('corrected').checked, true);
    if (token !== activeMetricRequest || activeCase()?.dir !== c.dir) return;
    currentMetricRows = rows;
    state.resultMetrics = state.resultMetrics.filter(row => row.case_dir !== c.dir);
    rows.forEach(row => state.resultMetrics.push({ site: c.name, case_dir: c.dir, ...row }));
    renderMetrics(rows);
    if (rows.length) drawComparison(rows[0]);
    status(`${c.name} 评估完成：${rows.length} 个变量`);
  } catch (error) { status(error); }
  finally { if (token === activeMetricRequest) $('evaluate').disabled = false; }
}

function renderMetrics(rows) {
  const box = $('metrics');
  box.textContent = '';
  destroyChartsInside($('evaluation-charts'));
  $('evaluation-charts').textContent = '';
  $('evaluation-png').disabled = true;
  if (!rows.length) { box.appendChild(node('div', 'result-empty', '没有可配对的变量。')); return; }
  const table = document.createElement('table');
  const head = document.createElement('tr');
  for (const label of ['变量', 'n', 'RMSE', 'MAE', 'Bias', 'R²', 'r', 'NSE', 'KGE', 'α', 'β']) head.appendChild(th(label, label === '变量' ? '' : 'n'));
  table.appendChild(head);
  for (const row of rows) {
    const tr = document.createElement('tr');
    tr.dataset.variable = row.model_var;
    const cells = [row.obs_var ?? row.name, row.n, metricText(row.rmse, 2), metricText(row.mae, 2),
      metricText(row.bias, 2, true), metricText(row.r2), metricText(row.correlation), metricText(row.nse, 3, true),
      metricText(row.kge, 3, true), metricText(row.alpha), metricText(row.beta)];
    cells.forEach((value, index) => tr.appendChild(td(value, index ? 'n' : '')));
    if (row.beta_warning) { tr.title = row.beta_warning; tr.lastChild.className = 'n warn'; }
    tr.onclick = () => drawComparison(row);
    table.appendChild(tr);
  }
  box.appendChild(table);
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
      false, summaryRow.name, 2400);
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
    `${row.name} · n=${row.n} · RMSE=${metricText(row.rmse, 2)} · Bias=${metricText(row.bias, 2, true)} · R²=${metricText(row.r2)} · NSE=${metricText(row.nse, 3, true)} · KGE=${metricText(row.kge, 3, true)} · 图形点 ${row.pair_n ?? row.time.length}/${row.pair_source_n ?? row.time.length}`);
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
    series: [{ label: '时间' }, { label: '模型', stroke: colors.model, width: 1.2 }, { label: '观测', stroke: colors.obs, width: 1.2 }],
    axes: [{}, {}],
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
  $('eval-all').disabled = !scope.length || !!comparisonController;
  $('eval-all').textContent = `评估分析范围内的 ${scope.length} 个站点`;
}

async function evaluateAll() {
  const todo = resultScope();
  if (!todo.length) return;
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
    const rows = await getMetrics(c, obs, spinup, corrected, true);
    return { case: c, rows };
  }, {
    signal: comparisonController.signal,
    onProgress: progress => {
      $('eval-progress').value = progress.completed;
      $('eval-progress-text').textContent = `${progress.completed}/${progress.total}`;
    },
  });
  state.resultMetrics = [];
  state.resultFailures = [];
  results.forEach((result, index) => {
    const c = todo[index];
    if (result.ok) result.value.rows.forEach(row => state.resultMetrics.push({ site: c.name, case_dir: c.dir, ...row }));
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
  if (!rows.length && !state.resultFailures.length) {
    summary.appendChild(node('div', 'result-empty', '尚未运行多站点评估。'));
    return;
  }
  const variables = [...new Set(rows.map(row => row.name))];
  state.summaryVar = variables.includes(state.summaryVar) ? state.summaryVar
    : variables.includes('Rnet') ? 'Rnet' : variables[0];
  const variableSelect = $('summary-var');
  variableSelect.textContent = '';
  for (const variable of variables) {
    const option = document.createElement('option'); option.value = variable; option.textContent = variable; variableSelect.appendChild(option);
  }
  variableSelect.value = state.summaryVar ?? '';
  const metric = $('summary-metric').value || state.summarySort || 'r2';
  state.summarySort = metric;
  const search = $('summary-search').value.trim().toLowerCase();
  const shown = rows.filter(row => row.name === state.summaryVar)
    .filter(row => !search || row.site.toLowerCase().includes(search));
  renderRanking(shown, metric);
  const table = document.createElement('table');
  const columns = [['站点', 'site'], ['n', 'n'], ['RMSE', 'rmse'], ['MAE', 'mae'], ['Bias', 'bias'], ['R²', 'r2'], ['r', 'correlation'], ['NSE', 'nse'], ['KGE', 'kge']];
  const head = document.createElement('tr'); columns.forEach(([label], index) => head.appendChild(th(label, index ? 'n' : ''))); table.appendChild(head);
  const meta = METRIC_META[metric] ?? { better: 'high' };
  const badness = row => meta.better === 'low' ? Number(row[metric])
    : meta.better === 'zero' ? Math.abs(Number(row[metric])) : -Number(row[metric]);
  shown.sort((a, b) => badness(b) - badness(a));
  for (const row of shown) {
    const tr = document.createElement('tr');
    columns.forEach(([, key], index) => tr.appendChild(td(index ? metricText(row[key], key === 'n' ? 0 : 3, key === 'bias') : row[key], index ? 'n' : '')));
    if (row.beta_warning) tr.title = row.beta_warning;
    tr.onclick = () => { setResultCase(row.case_dir); go('result-evaluation'); evaluateCurrent(); };
    table.appendChild(tr);
  }
  const wrap = node('div', 'result-table-wrap'); wrap.appendChild(table); summary.appendChild(wrap);
  if (state.resultFailures.length) {
    const failure = node('div', 'warn mini');
    failure.textContent = `${state.resultFailures.length} 个站点未完成评估：` + state.resultFailures.map(item => `${item.site}（${item.reason}）`).join('、');
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
  return {
    product: 'CoLM Desktop', version: '0.1.0', generated_at: new Date().toISOString(),
    copyright: 'CoLM LSM Development Team, School of Atmospheric Sciences, SYSU',
    settings: {
      domain: state.domain,
      subgrid: state.subgrid,
      wizard: state.wizard,
      kernel: $('kernel')?.value || null,
      discarded_records: Number($('spinup').value) || 0,
      energy_closure_corrected: $('corrected').checked,
      analysis_sites: scope.length,
    },
    cases: allCurrent().map(c => ({
      name: c.name, dir: c.dir, status: caseState(c), in_analysis_scope: scopeDirs.has(c.dir),
      has_history: !!c.has_history, observation: observationFor(c) || null,
    })),
    metrics: $('export-metrics').checked
      ? state.resultMetrics.filter(row => scopeDirs.has(row.case_dir)).map(stripMetricPairs) : [],
    failures: $('export-failures').checked ? [...runFailures, ...evaluationFailures] : [],
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
      '', '## Cases', '', '| Site | Status | Analysis scope | History | Observation |', '|---|---|---:|---:|---|'];
    data.cases.forEach(c => lines.push(`| ${c.name} | ${c.status} | ${c.in_analysis_scope ? 'Yes' : 'No'} | ${c.has_history ? 'Yes' : 'No'} | ${c.observation ?? '—'} |`));
    if (data.metrics.length) {
      lines.push('', '## Evaluation metrics', '', '| Site | Variable | n | RMSE | MAE | Bias | R² | r | NSE | KGE |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|');
      data.metrics.forEach(row => lines.push(`| ${row.site} | ${row.name} | ${row.n} | ${metricText(row.rmse)} | ${metricText(row.mae)} | ${metricText(row.bias)} | ${metricText(row.r2)} | ${metricText(row.correlation)} | ${metricText(row.nse)} | ${metricText(row.kge)} |`));
    }
    if (data.failures.length) {
      lines.push('', '## Incomplete items', '');
      data.failures.forEach(item => lines.push(`- ${item.site} [${item.phase}]: ${item.reason}`));
    }
    lines.push('', '---', '', 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU');
    return lines.join('\n') + '\n';
  }
  const lines = ['# CoLM Desktop 结果分析报告', '', `软件版本：${data.version}`, `生成时间：${data.generated_at}`,
    `分析范围：${data.settings.analysis_sites} 个站点`, `次网格方案：${data.settings.subgrid ?? '—'}`,
    `丢弃输出记录：${data.settings.discarded_records}`, `能量闭合订正：${data.settings.energy_closure_corrected ? '是' : '否'}`,
    '', '## 算例', '', '| 站点 | 状态 | 分析范围 | History | 观测 |', '|---|---|---:|---:|---|'];
  const statusLabels = { done: '已完成', failed: '失败', running: '运行中', waiting: '未完成' };
  data.cases.forEach(c => lines.push(`| ${c.name} | ${statusLabels[c.status] ?? c.status} | ${c.in_analysis_scope ? '是' : '否'} | ${c.has_history ? '是' : '否'} | ${c.observation ?? '—'} |`));
  if (data.metrics.length) {
    lines.push('', '## 评估指标', '', '| 站点 | 变量 | n | RMSE | MAE | Bias | R² | r | NSE | KGE |', '|---|---|---:|---:|---:|---:|---:|---:|---:|---:|');
    data.metrics.forEach(row => lines.push(`| ${row.site} | ${row.name} | ${row.n} | ${metricText(row.rmse)} | ${metricText(row.mae)} | ${metricText(row.bias)} | ${metricText(row.r2)} | ${metricText(row.correlation)} | ${metricText(row.nse)} | ${metricText(row.kge)} |`));
  }
  if (data.failures.length) {
    lines.push('', '## 未完成项', '');
    const phases = { run: '运行', evaluation: '评估' };
    data.failures.forEach(item => lines.push(`- ${item.site} [${phases[item.phase] ?? item.phase}]：${item.reason}`));
  }
  lines.push('', '---', '', 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU');
  return lines.join('\n') + '\n';
}

function escapeHtml(value) {
  return String(value).replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;');
}

function generateReport() {
  const data = reportData();
  const format = $('export-format').value;
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

export function invalidateResultCase(dir) {
  catalogCache.delete(dir);
  seriesCache.deleteWhere(key => key.startsWith(`${dir}\u001f`));
  metricsCache.deleteWhere(key => key.startsWith(`${dir}\u001f`));
  state.resultMetrics = state.resultMetrics.filter(row => row.case_dir !== dir);
  state.resultFailures = state.resultFailures.filter(row => row.case_dir !== dir);
}

function updateButtons() {
  const c = activeCase();
  $('plot').disabled = !c || !$('var').value;
  $('series-csv').disabled = !c || !$('var').value;
  $('evaluate').disabled = !c || !$('obs').value.trim();
  $('diagnose').disabled = !c;
  updateComparisonButton();
}

async function prepareActivePane() {
  if (!state.step.startsWith('result-')) return;
  renderOverview();
  syncResultCaseSelects();
  syncObservation();
  const c = activeCase();
  if (!c) { updateButtons(); return; }
  if (['result-data', 'result-series'].includes(state.step)) {
    try {
      const catalog = await loadCatalog(c);
      fillVariableSelect(catalog);
      if (!$('series-from').value) $('series-from').value = inputClock(catalog.start);
      if (!$('series-to').value) $('series-to').value = inputClock(catalog.end);
      if (state.step === 'result-data') await renderDataBrowser();
    } catch (error) { status(error); }
  }
  if (state.step === 'result-comparison') renderComparison();
  updateButtons();
}

/** 外部仍调用旧名字 `refreshVars`；现在它刷新的是整个结果索引。 */
export function refreshVars() {
  renderOverview();
  syncResultCaseSelects();
  syncObservation();
  renderSteps();
  prepareActivePane();
}

$('result-case-search').oninput = () => { state.resultCaseSearch = $('result-case-search').value; renderOverview(); };
$('result-status-filter').onchange = () => { state.resultStatusFilter = $('result-status-filter').value; renderOverview(); };
$('result-refresh').onclick = () => { catalogCache.clear(); refreshVars(); status('结果索引已刷新'); };
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
$('obs').onchange = () => { const c = activeCase(); if (c) state.resultObsOverrides.set(c.dir, $('obs').value.trim()); updateButtons(); };
$('evaluate').onclick = evaluateCurrent;
$('corrected').onchange = () => { if (currentMetricRows.length) evaluateCurrent(); };
$('evaluation-png').onclick = () => exportChart($('evaluation-charts'), `${activeCase()?.name ?? 'colm'}-evaluation.png`);
$('eval-all').onclick = evaluateAll;
$('eval-cancel').onclick = () => comparisonController?.abort();
$('summary-var').onchange = () => { state.summaryVar = $('summary-var').value; renderComparison(); };
$('summary-metric').onchange = () => { state.summarySort = $('summary-metric').value; renderComparison(); };
$('summary-search').oninput = renderComparison;
$('summary-copy').onclick = async () => {
  const rows = state.resultMetrics.filter(row => row.name === state.summaryVar).map(stripMetricPairs);
  await copyText(rowsToCsv(rows)); status('多站点指标 CSV 已复制');
};
$('diagnose').onclick = diagnoseCurrent;
$('export-report').onclick = generateReport;
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

refreshVars();
