import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { runInNewContext } from 'node:vm';

const source = await readFile(new URL('../dist/app/results.js', import.meta.url), 'utf8');
const fn = name => {
  const start = source.indexOf(`async function ${name}(`);
  assert.ok(start >= 0, name);
  return source.slice(start, source.indexOf('\n}\n', start) + 3);
};
const button = { disabled: false, closest: () => null };
const controls = { plot: button, var: { value: 'f_lfevpa' }, 'series-from': { value: '' }, 'series-to': { value: '' } };
const problem = new Error('history reader failed');
const plot = runInNewContext(`${fn('plotSeries')}; plotSeries`, {
  activeCase: () => ({ dir: '/case', name: 'site' }), $: id => controls[id],
  activeSeriesRequest: 0, parseLocalClock: () => null, status: () => {},
  getSeries: async () => { throw problem; }, loadCatalog: async () => ({}),
});
await assert.rejects(plot(true), error => error === problem);
assert.equal(button.disabled, false, 'failed operations must release buttons');
await plot(); // Manual button use still reports errors through the status bar.

let release;
let completed = false;
const state = { step: 'result-series', resultMetrics: [], resultFailures: [] };
const field = { id: '', type: 'number', dataset: { studyMin: 'g1(1)' } };
const pane = { contains: element => element === button, querySelectorAll: () => [field], innerText: 'visible result' };
const context = {
  state, document: { querySelector: () => pane }, $: id => id === 'plot' ? button : null,
  activeCase: () => ({ dir: '/case' }), stripMetricPairs: value => value,
  plotSeries: async strict => { assert.equal(strict, true); await new Promise(resolve => { release = resolve; }); completed = true; },
  prepareActivePane: async () => {},
};
const guide = runInNewContext(source.slice(source.indexOf('export const resultsGuide =')).replace('export const', 'const') + '\nresultsGuide;', context);
const action = guide.actions().plot;
const pending = action.run();
assert.equal(completed, false, 'bridge must await the actual computation');
release();
const result = await pending;
assert.equal(completed, true);
assert.equal(guide.state().page_text, 'visible result');
assert.equal(guide.state().case, '/case');
assert.equal(guide.fields()[0].id, 'result-series-studyMin-g1(1)');
assert.equal(guide.fields()[0].id, field.id, 'dynamic field identity survives repeated reads');
state.step = 'result-tuning';
await assert.rejects(action.run(), /当前页面/);
state.step = 'result-series';
button.disabled = true;
await assert.rejects(action.run(), /当前页面/);
button.disabled = false;
context.plotSeries = async () => { throw problem; };
await assert.rejects(guide.actions().plot.run(), error => error === problem);

const noStudy = runInNewContext(`${fn('runStudy')}; runStudy`, {
  spatialStudyReason: () => '', aggregateStudy: () => ({ status: 'NotStarted' }),
  studyViews: {}, activeStudyDirs: () => [], currentKernel: () => '/kernel', status: () => {},
});
await assert.rejects(noStudy('tuning', true), /请先生成率定任务/);
const exportStudy = runInNewContext(`${fn('exportStudy')}; exportStudy`, {
  activeStudyDirs: () => ['/project/study'], $: () => ({ value: '' }),
  appPrompt: () => { throw new Error('must not prompt inside assistant action'); },
});
await assert.rejects(exportStudy('uq', true), /请先填写导出目录/);
const exportStart = source.indexOf('function exportChart(');
const chartExport = runInNewContext(source.slice(exportStart, source.indexOf('\n}\n', exportStart) + 3) + '\nexportChart;');
assert.throws(() => chartExport({ querySelector: () => null }, 'chart.png', true), /没有可导出的图表/);
let strictRefresh = false;
context.refreshCurrentEvaluationCatalog = async strict => { strictRefresh = strict; throw problem; };
await assert.rejects(guide.changed({ id: 'obs', dataset: {} }), error => error === problem);
assert.equal(strictRefresh, true);
state.step = 'result-export';
context.$ = id => id === 'export-pdf' ? button : null;
context.exportPdfReport = async () => {};
const pdf = await guide.actions()['export-pdf'].run();
assert.equal(pdf.outcome, 'print_dialog_opened');
assert.equal(pdf.file_saved, false, 'opening a print dialog is not proof of a saved file');

// Opening a result pane must join the history validation already started by rendering.
let finishHistory;
let historyReads = 0;
let populated = false;
const historyHealth = new Map();
const catalogCache = new Map();
const example = { dir: '/pending', name: 'pending', has_history: true };
const raceState = { step: 'result-series' };
const raceControls = { 'series-from': { value: '' }, 'series-to': { value: '' } };
const raceContext = {
  globalThis: { dispatchEvent: () => {} }, Event: class {},
  state: raceState, activePaneRequest: 0, historyHealth, catalogCache, hasBackend: true,
  allCurrent: () => [example], navigator: { hardwareConcurrency: 2 },
  invoke: async () => { historyReads++; return new Promise(resolve => { finishHistory = resolve; }); },
  assertUsableCatalog: () => {},
  boundedMap: async (items, width, work) => Promise.all(items.map(async item => {
    try { return { ok: true, value: await work(item) }; } catch (error) { return { ok: false, error: String(error) }; }
  })),
  activeCase: () => historyHealth.get(example.dir)?.ok ? example : null,
  syncResultCaseSelects: () => {}, syncObservation: () => {}, updateButtons: () => {}, status: () => {},
  renderOverview: () => {}, loadCatalog: async () => catalogCache.get(example.dir),
  fillVariableSelect: () => { populated = true; }, inputClock: value => String(value),
  $: id => raceControls[id],
};
const race = runInNewContext(`${fn('validateHistoryCases')}\n${fn('prepareActivePane')}\n({ validateHistoryCases, prepareActivePane });`, raceContext);
const manual = race.validateHistoryCases();
let ready = false;
const opening = race.prepareActivePane(true).then(() => { ready = true; });
await Promise.resolve();
assert.equal(ready, false, 'strict prepare must wait for a pending history read');
assert.equal(historyReads, 1, 'strict prepare must reuse the pending read');
finishHistory(JSON.stringify({ start: 1, end: 2, variables: [] }));
await Promise.all([manual, opening]);
assert.equal(populated, true, 'variables must be populated before prepare resolves');
assert.equal(raceControls['series-from'].value, '1');
// Refresh invalidates health first; it must await a fresh validation as well.
historyHealth.clear(); populated = false; ready = false;
const refreshing = race.prepareActivePane(true).then(() => { ready = true; });
await Promise.resolve();
assert.equal(ready, false);
assert.equal(historyReads, 2);
finishHistory(JSON.stringify({ start: 3, end: 4, variables: [] }));
await refreshing;
assert.equal(populated, true);
// A failed history read must not report an empty successful page.
historyHealth.clear();
raceContext.invoke = async () => { throw new Error('corrupt history'); };
await assert.rejects(race.prepareActivePane(true), /结果校验失败.*corrupt history/);
console.log('results guide: awaited actions, strict failures, pending-history reuse and refresh readiness passed');

// The overview assistant bridge waits for the real read, propagates failure, and rejects departed contexts.
state.step = 'result-overview';
context.$ = id => id === 'hub-plot-button' ? button : null;
let overviewFinished = false;
context.plotOverview = async strict => { assert.equal(strict, true); await new Promise(resolve => { release = resolve; }); overviewFinished = true; };
const overviewAction = guide.actions()['hub-plot-button'];
const overviewPending = overviewAction.run();
assert.equal(overviewFinished, false);
release(); await overviewPending;
assert.equal(overviewFinished, true);
overviewFinished = false;
const variablePending = guide.changed({id:'hub-variable',dataset:{}});
assert.equal(overviewFinished, false);
release(); await variablePending;
assert.equal(overviewFinished, true);
context.plotOverview = async strict => { assert.equal(strict, true); throw problem; };
await assert.rejects(overviewAction.run(), error => error === problem);
await assert.rejects(guide.changed({id:'hub-variable',dataset:{}}), error => error === problem);
context.plotOverview = async () => new Promise(resolve => { release = resolve; });
const departed = overviewAction.run();
state.step = 'result-evaluation'; release();
await assert.rejects(departed, /分析上下文已变化/);
