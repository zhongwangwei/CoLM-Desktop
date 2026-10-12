import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';
const source = await readFile(new URL('../dist/app/results.js', import.meta.url), 'utf8');
const fn = name => {
  const start = source.indexOf(`async function ${name}(`) >= 0 ? source.indexOf(`async function ${name}(`) : source.indexOf(`function ${name}(`);
  assert.ok(start >= 0, name);
  return source.slice(start, source.indexOf('\n}\n', start) + 3);
};
function setup() {
  const element = () => ({ value: '', checked: false, disabled: false, children: [], textContent: '', appendChild(item) { this.children.push(item); } });
  const controls = Object.fromEntries(['hub-chart','hub-variable','hub-period','hub-plot-button','hub-chart-status','hub-metrics','spinup','corrected'].map(id => [id, element()]));
  const state = { step: 'result-overview', resultMetrics: [] };
  let current = { dir: '/a', name: 'A' }, obs = '/obs';
  const charts = [], calls = [], events = [];
  const catalog = { start: 1, end: 2, variables: [{ name: 'f_lfevpa', kind: 'series', units: 'W/m²' }] };
  const row = { name: 'Qle', n: 20, bias: null, rmse: 2, units: 'W/m²', time: [1,2], model: [2,3], obs: [1,2], pair_source_n: 20 };
  const context = {
    state, activeOverviewRequest: 0, activeCase: () => current, observationFor: () => obs,
    COMMON_VARIABLES: { f_lfevpa: [] }, document: { createElement: element },
    $: id => controls[id], dialogText: s => s, utcText: String,
    destroyChart: host => { host.textContent = ''; }, overviewMetrics: null,
    node: (tag, cls, text) => ({ textContent: text, children: [], append(...items) { this.children.push(...items); }, appendChild(item) { this.children.push(item); } }), resultKpi: (value, label) => ({ value, label }),
    metricText: value => Number.isFinite(value) ? String(value) : '—',
    variableMeta: (name, units) => ({ label: name, units }),
    loadCatalog: async () => catalog,
    loadEvaluationCatalog: async () => [{ name: 'Qle', model_var: 'f_lfevpa', available: true }],
    getMetrics: async (...args) => { calls.push(args); return [row]; },
    getSeries: async () => ({ time: [1,2], vars: { f_lfevpa: [4,5] } }),
    chartColors: () => ({ model: '#138F96', obs: '#66758B' }),
    makeChart: (host, opts, data) => { charts.push({opts,data}); },
    stripMetricPairs: ({time,model,obs,...rest}) => rest,
    globalThis: { dispatchEvent: event => events.push(event.type) }, Event: class { constructor(type) { this.type = type; } },
  };
  const plot = runInNewContext(`${fn('overviewMetrics')}\n${fn('plotOverview')}\nplotOverview`, context);
  return { context, controls, state, charts, calls, events, catalog, row, plot, setCase: value => { current = value; }, setObs: value => { obs = value; } };
}
const paired = setup();
await paired.plot(true);
assert.equal(paired.charts.length, 1);
assert.deepEqual(JSON.parse(JSON.stringify(paired.charts[0].data)), [[1,2],[2,3],[1,2]]);
assert.equal(paired.calls[0][5][0], 'Qle');
assert.equal(paired.calls[0][6], 2400);
assert.equal(paired.controls['hub-metrics'].children[0].children[1].textContent, '—');
assert.equal(paired.state.resultMetrics[0].case_dir, '/a');
assert.equal(paired.state.resultOverviewMetric.case_dir, '/a');
assert.equal(paired.state.resultMetrics[0].time, undefined);
assert.ok(paired.events.includes('colm:metrics'));
assert.equal(paired.controls['hub-period'].textContent, '1 — 2 UTC');
const noObs = setup(); noObs.setObs('');
await noObs.plot(true);
assert.equal(noObs.calls.length, 0);
assert.equal(noObs.charts[0].data.length, 2, 'no invented observation line');
assert.equal(noObs.state.resultMetrics.length, 0);
assert.equal(noObs.state.resultOverviewMetric, null);
assert.match(noObs.controls['hub-chart-status'].textContent, /尚未配置观测/);
const noVar = setup(); noVar.catalog.variables = [{name:'time',kind:'series'}];
await noVar.plot();
await assert.rejects(noVar.plot(true), /没有可绘制/);
assert.equal(noVar.charts.length, 0);
assert.equal(noVar.controls['hub-plot-button'].disabled, true);
assert.match(noVar.controls['hub-chart-status'].textContent, /没有可绘制/);
const empty = setup(); empty.row.n = 0; empty.context.getSeries = async () => ({time: [],vars:{f_lfevpa:[]}});
await empty.plot();
await assert.rejects(empty.plot(true), /没有有效模拟数据/);
assert.equal(empty.charts.length, 0);
assert.equal(empty.state.resultMetrics.length, 0);
const nullValues = setup(); nullValues.setObs('');
nullValues.context.getSeries = async () => ({time:[1,2],vars:{f_lfevpa:[null,null]}});
await nullValues.plot();
assert.equal(nullValues.charts.length, 0);
await assert.rejects(nullValues.plot(true), /没有有效模拟数据/);
const noCase = setup(); noCase.setCase(null);
await noCase.plot();
assert.equal(noCase.controls['hub-plot-button'].disabled, true);
await assert.rejects(noCase.plot(true), /请选择已有可用结果/);
const wrongPane = setup(); wrongPane.state.step = 'result-series';
await assert.rejects(wrongPane.plot(true), /当前页面不能执行/);
for (const change of [
  x => x.setCase({dir:'/b',name:'B'}), x => { x.state.step = 'result-series'; },
  x => x.setObs('/other'), x => { x.controls.corrected.checked = true; },
  x => { x.controls.spinup.value = '2'; }, x => { x.controls['hub-variable'].value = 'other'; },
]) {
  const stale = setup(); let finish;
  stale.context.getMetrics = () => new Promise(resolve => { finish = resolve; });
  const pending = stale.plot(true);
  while (!finish) await Promise.resolve();
  change(stale); finish([stale.row]); await assert.rejects(pending, /结果上下文已变化/);
  assert.equal(stale.charts.length, 0, 'stale response must not repopulate a chart');
  assert.equal(stale.state.resultMetrics.length, 0);
  assert.equal(stale.state.resultOverviewMetric, null);
}
const failing = setup(); failing.context.loadEvaluationCatalog = async () => { throw new Error('bad obs'); };
await failing.plot(true);
assert.equal(failing.charts[0].data.length, 2);
assert.match(failing.controls['hub-chart-status'].textContent, /bad obs/);
const late = setup(); let history;
late.context.loadCatalog = () => new Promise(resolve => { history = resolve; });
const loading = late.plot(true);
assert.equal(late.charts.length, 0);
history(late.catalog); await loading;
assert.equal(late.charts.length, 1);
assert.match(fn('validateHistoryCases'), /\['result-overview', 'result-data'/);
// The initial asynchronous history validation must re-enter the overview pane.
let finishHistory, overviewPrepared = 0;
const health = new Map();
const historyContext = {
  state: {step:'result-overview'}, historyHealth: health, catalogCache: new Map(), hasBackend: true,
  navigator:{hardwareConcurrency:2}, allCurrent: () => [{dir:'/late',has_history:true}],
  invoke: () => new Promise(resolve => { finishHistory = resolve; }), assertUsableCatalog: () => {},
  boundedMap: async (items, width, work) => Promise.all(items.map(async item => ({ok:true,value:await work(item)}))),
  prepareActivePane: async () => { overviewPrepared++; },
};
const validate = runInNewContext(`${fn('validateHistoryCases')}\nvalidateHistoryCases`, historyContext);
const validation = validate();
assert.equal(overviewPrepared, 0);
finishHistory(JSON.stringify({steps:2,variables:[{name:'f_lfevpa'}]}));
await validation;
assert.equal(overviewPrepared, 1);
assert.equal(health.get('/late').ok, true);
console.log('results overview: paired/model-only charts, truthful empty states, stale-response rejection and late history passed');

// The concise menu groups raw variables without hiding scientific outputs.
const grouped = setup();
grouped.catalog.variables.push({name:'custom_flux',kind:'series',units:'W/m²'});
await grouped.plot(true);
assert.equal(grouped.controls['hub-variable'].children.length, 2);
assert.equal(grouped.controls['hub-variable'].children[1].children[0].value, 'custom_flux');

// Theme defaults keep chart axes readable and retain explicit axis options.
let rendered;
const themeContext = {
 document: { documentElement: { dataset: { theme: 'light' } } },
 destroyChart: () => {}, charts: new Map(), chartResize: null,
 uPlot: class { constructor(options) { rendered=options; } },
};
const make = runInNewContext(`${fn('makeChart')}; makeChart`, themeContext);
make({clientWidth:500}, {axes:[{label:'W/m²',grid:{show:false}}]}, []);
const lightStroke=rendered.axes[0].stroke;
assert.equal(rendered.axes[0].label, 'W/m²');
assert.equal(rendered.axes[0].grid.show, false);
themeContext.document.documentElement.dataset.theme='dark';
make({clientWidth:500}, {axes:[{label:'W/m²'}]}, []);
assert.notEqual(rendered.axes[0].stroke, lightStroke);
