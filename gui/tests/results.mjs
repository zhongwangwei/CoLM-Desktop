import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { runInNewContext } from 'node:vm';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-results-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;

const {
  LruCache, boundedMap, envelopeDiagnostics, metricKey, resultCases, rowsToCsv, sortedImportanceRows,
} = await import(moduleUrl('result-model.js'));
const { aggregateStudy, aggregateStudyStatuses, bestTuningSummary, studyActionState, studyWarnings } = await import(moduleUrl('study-model.js'));
const { WORKFLOW, nextOf } = await import(moduleUrl('shell.js'));
const { state } = await import(moduleUrl('state.js'));

const groupFor = id => WORKFLOW.find(group => group.steps.some(step => step.id === id));
const analysisGroup = groupFor('result-overview');
for (const id of [
  'params-water', 'params-eco', 'params-river', 'params-da', 'params-tracer', 'params-urban',
]) {
  const step = groupFor(id)?.steps.find(item => item.id === id);
  if (!step?.show?.() || !step.need()?.includes('建一个算例')) {
    throw new Error(`${id} must stay visible but disabled before a site/case is selected`);
  }
}
for (const id of ['result-uncertainty', 'result-tuning', 'result-export']) {
  const group = groupFor(id);
  if (!group || group === analysisGroup || group.steps.length !== 1) {
    throw new Error(`${id} must be a top-level workflow peer of Results analysis`);
  }
  if (group.steps[0].show) throw new Error(`${id} must stay visible in the top-level workflow`);
}
state.selected = { dir: '/cases/pending', name: 'pending' };
state.cases = [state.selected];
state.createdCases.add(state.selected.dir);
for (const id of ['result-uncertainty', 'result-tuning']) {
  if (groupFor(id).steps[0].need() !== null || groupFor(id).steps[0].optional !== true) {
    throw new Error(`${id} must unlock as soon as Basic setup has created a case`);
  }
}
if (!groupFor('result-export').steps[0].need()?.includes('至少一个算例')) {
  throw new Error('result-export must still require a completed result');
}
state.selected.has_history = true;
if (nextOf('result-diagnostics')?.id !== 'result-export') {
  throw new Error('optional Study branches must not become required Next steps');
}
state.selected.has_history = false;

const cases = [
  { dir: '/cases/old', name: 'old', has_history: true },
  { dir: '/cases/a', name: 'A', has_history: true },
  { dir: '/cases/b', name: 'B', has_history: false },
  { dir: '/cases/c', name: 'C', has_history: true },
];
const scope = resultCases(cases, new Set(['/cases/a', '/cases/b', '/cases/c']));
if (scope.map(c => c.name).join('|') !== 'A|B|C') {
  throw new Error('result scope leaked old root cases or lost current cases');
}
if (resultCases(cases, new Set(['/cases/a', '/cases/c']), true).map(c => c.name).join('|') !== 'A|C') {
  throw new Error('completed result scope is not derived from history availability');
}

let active = 0;
let peak = 0;
const mapped = await boundedMap([1, 2, 3, 4, 5], 2, async value => {
  active += 1;
  peak = Math.max(peak, active);
  await new Promise(resolve => setTimeout(resolve, 5));
  active -= 1;
  if (value === 3) throw new Error('site failed');
  return value * 2;
});
if (peak !== 2 || mapped.filter(x => x.ok).length !== 4 || mapped[2].ok) {
  throw new Error('bounded result pool lost its concurrency or partial-failure contract');
}

const cache = new LruCache(2);
cache.set('a', 1); cache.set('b', 2); cache.get('a'); cache.set('c', 3);
if (cache.has('b') || !cache.has('a') || !cache.has('c')) {
  throw new Error('result LRU does not evict the least recently used item');
}
if (metricKey({ caseDir: '/a', obs: '/o', summaryOnly: false })
    === metricKey({ caseDir: '/a', obs: '/o', summaryOnly: true })) {
  throw new Error('summary-only metrics must not collide with chart-pair metrics in cache');
}
if (metricKey({ caseDir: '/a', obs: '/o', pairVars: ['Qle', 'Rnet'] })
    !== metricKey({ caseDir: '/a', obs: '/o', pairVars: ['Rnet', 'Qle', 'Rnet'] })) {
  throw new Error('selected evaluation variables do not produce a stable cache key');
}

const csv = rowsToCsv([{ site: 'A,1', note: 'line\n"two"' }], ['site', 'note']);
if (!csv.includes('"A,1"') || !csv.includes('"line\n""two"""')) {
  throw new Error('result CSV export does not quote delimiters and newlines safely');
}

const influenceRows = sortedImportanceRows([
  { parameter: 'low', value: -0.220, method: 'spearman', n: 12 },
  { parameter: 'missing', value: null, method: 'spearman', n: 0 },
  { parameter: 'top', value: 1.00, method: 'spearman', n: 30 },
  { parameter: 'mid', value: -0.5, method: 'oat_finite_difference_slope', n: 2 },
]);
if (influenceRows.map(row => row.parameter).join('|') !== 'top|mid|low|missing') {
  throw new Error('uncertainty influence rows must sort by absolute finite value and keep null last');
}
const envelopeStats = envelopeDiagnostics({
  n_eff: [3, 5, null, 4],
  stable: [true, false, true, false],
  p05: [1, 2, null, 4],
  p50: [2, 4, 6, null],
  p95: [5, 8, 9, 10],
  baseline: [1, 1, 10, 0],
});
if (envelopeStats.minNEff !== 3 || envelopeStats.maxNEff !== 5 || envelopeStats.unsupported !== 2
    || envelopeStats.meanWidth !== 5.333333333333333 || envelopeStats.maxWidth !== 6
    || envelopeStats.meanMedianBaselineDiff !== 2.6666666666666665) {
  throw new Error(`uncertainty envelope diagnostics are wrong: ${JSON.stringify(envelopeStats)}`);
}
const hugeEnvelopeStats = envelopeDiagnostics({ n_eff: Array.from({ length: 120000 }, (_, i) => i % 7), stable: [] });
if (hugeEnvelopeStats.minNEff !== 0 || hugeEnvelopeStats.maxNEff !== 6) {
  throw new Error('uncertainty envelope diagnostics must handle large arrays without spread-based min/max');
}
for (const [high, low] of [
  [[1e308, 1e308, null], [0, 0, 1]],
  [[1e308, 0], [-1e308, 0]],
]) {
  const diagnostics = envelopeDiagnostics({ p95: high, p05: low, p50: high, baseline: low });
  for (const key of ['meanWidth', 'meanMedianBaselineDiff']) {
    if (!Number.isFinite(diagnostics[key]) || Math.abs(diagnostics[key] / 1e308 - 1) > 1e-15) {
      throw new Error(`${key} overflowed although the mean is finite: ${diagnostics[key]}`);
    }
  }
}
const unrepresentable = envelopeDiagnostics({ p95: [1e308], p05: [-1e308], p50: [1e308], baseline: [-1e308] });
if (unrepresentable.meanWidth !== null || unrepresentable.maxWidth !== null
    || unrepresentable.meanMedianBaselineDiff !== null) {
  throw new Error('unrepresentable envelope diagnostics must be unavailable, not infinite');
}

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const pane of [
  'result-overview', 'result-data', 'result-series', 'result-evaluation',
  'result-comparison', 'result-diagnostics', 'result-uncertainty',
  'result-tuning', 'result-export',
]) {
  if (!html.includes(`data-flow-pane="${pane}"`)) {
    throw new Error(`results workbench is missing ${pane}`);
  }
}
if (!html.includes('id="series-csv"')) {
  throw new Error('time-series workbench is missing the full-resolution CSV export');
}
if (!html.includes('<option value="pdf">PDF</option>')) {
  throw new Error('report format selector must offer PDF directly');
}
for (const control of [
  'evaluation-variable-selector', 'batch-evaluation-variable-selector',
  'uq-readiness', 'uq-create', 'uq-run', 'uq-status', 'uq-retry',
  'uq-spinup-years', 'uq-spinup-repeat', 'uq-spinup-apply', 'uq-spinup-note', 'uq-range-confirm', 'uq-step-progress', 'uq-step-do', 'uq-step-why', 'uq-step-prev', 'uq-step-next',
  'uq-purpose', 'uq-method-help', 'uq-spinup-help', 'uq-count-help',
  'uq-seed-help', 'uq-jobs-help',
  'tune-readiness', 'tune-create', 'tune-run', 'tune-status', 'tune-retry', 'tune-val-from', 'tune-val-to',
  'tune-min-pairs', 'tune-range-confirm', 'tune-step-progress', 'tune-step-do', 'tune-step-why', 'tune-step-prev', 'tune-step-next',
  'evaluation-chart-refresh', 'export-pdf',
]) {
  if (!html.includes(`id="${control}"`)) throw new Error(`results workbench is missing ${control}`);
}
for (const kind of ['uq', 'tuning']) {
  const steps = [...html.matchAll(new RegExp(`data-study-wizard="${kind}" data-study-step="(\\d+)"`, 'g'))]
    .map(match => Number(match[1])).sort((a, b) => a - b);
  if (steps.join(',') !== '0,1,2,3,4,5,6') throw new Error(`${kind} must remain a seven-page guided analysis workflow`);
}
for (const text of ['尚未运行过基准算例也可以配置', '尚未运行过原算例也可以准备', '先设计，再准备任务，最后开始计算', '先设计，再准备任务，最后开始搜索']) {
  if (!html.includes(text)) throw new Error(`Study guidance is missing: ${text}`);
}
for (const text of [
  '不确定性分析要回答什么？', '有限样本分位带不是统计置信区间',
  '这个阶段不运行模型', 'OAT 候选数 = 2 × 已选参数数',
  '预热后实际写出的全部 history', '预热期不写 history', '少于 20 仅适合流程试跑',
  '并发只影响墙钟时间', '无需再次选择运行目录', '应用到当前算例',
]) {
  if (!html.includes(text)) throw new Error(`uncertainty design guidance is missing: ${text}`);
}
for (const text of ['5 · 生成分析任务', '6 · 开始计算与监控', '生成分析任务', '开始计算', '手动刷新', '导出分析记录']) {
  if (!html.includes(text)) throw new Error(`analysis task flow is missing: ${text}`);
}
for (const [kind, jobs, runJobs] of [['uq', 'uq-jobs', 'uq-run-jobs'], ['tuning', 'tune-jobs', 'tune-run-jobs']]) {
  const page1 = html.indexOf(`data-study-wizard="${kind}" data-study-step="0"`);
  const nextPage = html.indexOf(`data-study-wizard="${kind}"`, page1 + 1);
  const page6 = html.indexOf(`data-study-wizard="${kind}" data-study-step="5"`);
  const page7 = html.indexOf(`data-study-wizard="${kind}" data-study-step="6"`);
  const designControl = html.indexOf(`id="${jobs}"`);
  const runControl = html.indexOf(`id="${runJobs}"`);
  if (designControl < page1 || designControl > nextPage || runControl < page6 || runControl > page7
      || (html.match(new RegExp(`id="${jobs}"`, 'g')) || []).length !== 1
      || (html.match(new RegExp(`id="${runJobs}"`, 'g')) || []).length !== 1) {
    throw new Error(`${kind} parallel controls must appear once on both design and run pages`);
  }
}
for (const text of ['同时运行数（并行）', '本次同时运行数（并行）', '实时运行日志', '尚无运行日志']) {
  if (!html.includes(text)) throw new Error(`run-and-monitor guidance is missing: ${text}`);
}
for (const text of ['线性（等差）', '对数（等比）', '下界 ≤ 0 必须选线性', '上界/下界约 ≥ 10']) {
  if (!html.includes(text)) throw new Error(`sampling-scale guidance is missing: ${text}`);
}
for (const text of ['创建 Study', '运行 Study', '导出 Study']) {
  if (html.includes(text)) throw new Error(`visible analysis workflow must not expose internal term: ${text}`);
}

for (const hidden of ['uq-site-mode', 'uq-from', 'uq-to', 'uq-kernel-dir', 'uq-site-mode-help', 'uq-window-help', 'uq-kernel-help']) {
  if (html.includes(`id="${hidden}"`)) throw new Error(`uncertainty design should not expose ${hidden}`);
}
if (html.includes('id="tune-kernel-dir"')) {
  throw new Error('tuning must inherit the current kernel instead of exposing a fake runtime-directory choice');
}
for (const id of [
  'tune-purpose', 'tune-metric-help', 'tune-min-pairs-help', 'tune-site-mode-help',
  'tune-spinup-years', 'tune-spinup-repeat', 'tune-spinup-apply', 'tune-spinup-note',
  'tune-window-help', 'tune-pop-help', 'tune-gen-help', 'tune-seed-help', 'tune-jobs-help',
]) {
  if (!html.includes(`id="${id}"`)) throw new Error(`tuning guidance is missing ${id}`);
}
for (const text of ['参数调优要回答什么？', '先设计，再准备任务，最后开始搜索', '目标权重：', '权重必须大于 0', '如果校准明显变好但验证变差']) {
  if (!html.includes(text)) throw new Error(`tuning scientific guidance is missing: ${text}`);
}
for (const metric of ['abs_bias', 'nse', 'r']) {
  if (!html.includes(`value="${metric}"`)) throw new Error(`tuning metric selector is missing ${metric}`);
}
const resultUi = await readFile(join(root, 'dist', 'app', 'results.js'), 'utf8');
const resultCss = await readFile(join(root, 'dist', 'app', 'style.css'), 'utf8');

// Listed results must expose read/parse failures without leaking stale-scope errors.
for (const [path, failure] of [
  ['importance.json', 'ipc'], ['importance.json', 'json'], ['importance.json', 'null'],
  ['members.csv', 'ipc'], ['envelopes/site/a.json', 'ipc'], ['envelopes/site/a.json', 'json'],
]) {
  for (const stale of [false, true]) {
    let scope = 'original';
    const elements = [];
    const node = (tag, className = '', textContent = '') => {
      const element = { tag, className, textContent, children: [], value: '',
        append(...children) { this.children.push(...children); },
        appendChild(child) { this.children.push(child); },
      };
      elements.push(element);
      return element;
    };
    const host = node('host');
    const source = resultUi.slice(resultUi.indexOf('async function studyResultText('), resultUi.indexOf('\nconst studyResultPaths'))
      + resultUi.slice(resultUi.indexOf('async function renderStudyResults('), resultUi.indexOf('\nasync function runStudy('));
    const refresh = runInNewContext(source + '\nrefreshStudy;', {
      $: () => host, node, document: { createElement: node, createDocumentFragment: () => node('fragment') },
      activeStudyDirs: () => ['/cases/.colm/studies/one'], studyScopeKey: () => scope,
      studyRefreshRequests: { uq: 0 }, studyEvents: { uq: [] },
      studyResultsReady: () => true, studyResultPaths: () => new Set([path]),
      destroyChartsInside() {}, renderUncertaintyDiagnostics: () => node('diagnostics'),
      renderStudyEnvelope() {}, envelopeExplanation: () => node('explanation'),
      renderEnvelopeChart() { throw new Error('invalid chart result was rendered'); },
      status() {},
      invoke: async command => {
        if (command === 'study_status') return JSON.stringify({ manifest: { id: 'one' }, state: { status: 'completed' }, events: [] });
        if (stale) scope = 'changed';
        if (failure === 'ipc') throw new Error('permission denied');
        return failure === 'null' ? 'null' : '{invalid';
      },
    });
    await refresh('uq');
    if (path.startsWith('envelopes/')) {
      elements.find(element => element.tag === 'select').value = path;
      await elements.find(element => element.tag === 'button').onclick();
    }
    const errors = elements.filter(element => element.className === 'warn mini');
    if (errors.length !== Number(!stale) || (!stale && !errors[0].textContent.includes(path))) {
      throw new Error(`listed ${path} ${failure} failure was hidden or crossed scope (stale=${stale})`);
    }
  }
}

// The NeedsReview recovery path must not undo the spatial mutation guard.
for (const kind of ['uq', 'tuning']) {
  for (const current of ['Ready', 'NeedsReview', 'Running', 'Paused', 'Completed']) {
    for (const spatial of [false, true]) {
      const elements = new Map();
      const element = id => {
        if (!elements.has(id)) elements.set(id, {});
        return elements.get(id);
      };
      const render = runInNewContext(resultUi.slice(
        resultUi.indexOf('function renderStudyActions('),
        resultUi.indexOf('\nasync function renderStudySpinup('),
      ) + '\nrenderStudyActions;', {
        $: element, document: { querySelectorAll: () => [] },
        spatialStudyReason: () => spatial ? 'early state' : '',
        activeStudyDirs: () => ['/cases/.colm/studies/one'],
        studyViews: { [kind]: { state: { status: current } } },
        studyRunning: { [kind]: false }, studyCreating: { [kind]: false },
        aggregateStudy, bestTuningSummary, studyActionState, studyJobInputs: () => [], dialogText: text => text,
      });
      render(kind);
      const prefix = kind === 'uq' ? 'uq' : 'tune';
      const actions = studyActionState(current, true);
      for (const action of ['run', 'retry', 'resume']) {
        if (element(`${prefix}-${action}`).disabled !== (spatial || !actions[action])) {
          throw new Error(`${kind} ${current}: spatial=${spatial} incorrectly enables ${action}`);
        }
      }
      for (const action of ['pause', 'cancel']) {
        if (element(`${prefix}-${action}`).disabled !== !actions[action]) {
          throw new Error(`${kind} ${current}: spatial must preserve ${action}`);
        }
      }
    }
  }
}
{
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, {});
    return elements.get(id);
  };
  const render = runInNewContext(resultUi.slice(
    resultUi.indexOf('function renderStudyActions('),
    resultUi.indexOf('\nasync function renderStudySpinup('),
  ) + '\nrenderStudyActions;', {
    $: element, document: { querySelectorAll: () => [] },
    spatialStudyReason: () => '', activeStudyDirs: () => ['/cases/.colm/studies/tuning'],
    studyViews: { tuning: { state: { status: 'Completed', best_member: 'm999999', candidates: { m999999: { feasible: false, calibration: 0.1 } } } } },
    studyRunning: { tuning: false }, studyCreating: { tuning: false },
    aggregateStudy, bestTuningSummary, studyActionState, studyJobInputs: () => [], dialogText: text => text,
  });
  render('tuning');
  if (element('tune-apply-best').disabled !== true) throw new Error('tuning apply button must stay disabled without a feasible best candidate');
}

// A metadata refresh must not erase the selected output while its own guard is awaiting IPC.
{
  let selected = true;
  let appended = 0;
  const host = {
    querySelectorAll: () => selected ? [{ dataset: { uqOutput: 'f_rnet' } }] : [],
    querySelector: () => selected ? {} : null,
    set textContent(_) { selected = false; },
    appendChild() { appended++; selected = true; },
  };
  const render = runInNewContext(resultUi.slice(
    resultUi.indexOf('async function renderStudyOutputs('),
    resultUi.indexOf('\nasync function renderTuningTargets('),
  ) + '\nrenderStudyOutputs;', {
    $: () => host, studyAsyncRequests: { outputs: 0 }, studyScopeKey: () => 'same',
    studyScope: () => [{ dir: '/cases/site', has_history: false }],
    plannedHistoryCatalog: async () => ({ variables: [{ name: 'f_rnet', kind: 'series' }] }),
    studySiteId: () => 'site', COMMON_VARIABLES: { f_rnet: [] }, variableMeta: () => ({}),
    node: () => ({ append() {} }), document: { createElement: () => ({ dataset: {} }) },
  });
  await render(() => selected);
  if (!selected || appended !== 1) throw new Error('output refresh invalidated its own creation/design guard');
}

for (const delayedStage of ['metadata', 'metadata-design', 'outputs', 'outputs-design', 'preflight', 'design', 'create', 'unchanged']) {
  let scope = 'original';
  let design = 'original';
  let resolve;
  let creates = 0;
  let registrations = 0;
  const creating = { uq: false, tuning: false };
  const plans = () => [{ caseRoot: '/cases', specJson: JSON.stringify({
    budget: { candidate_count: 2 }, parameters: [{ name: design }],
  }) }];
  const delay = () => new Promise(done => { resolve = done; });
  const create = runInNewContext(resultUi.slice(
    resultUi.indexOf('async function createStudy('),
    resultUi.indexOf('\nfunction renderStudyEnvelope('),
  ) + '\ncreateStudy;', {
    studyScope: () => [{ dir: '/cases/site' }], studyScopeKey: () => scope,
    parentDir: () => '/cases', $: () => ({}), dialogText: value => value,
    studyCreating: creating,
    loadStudyParams: () => delayedStage.startsWith('metadata') ? delay() : Promise.resolve(),
    renderStudyOutputs: () => delayedStage.startsWith('outputs') ? delay() : Promise.resolve(), studyPlans: plans,
    studyDesignKeys: () => plans().map(plan => plan.specJson), stableStudySpecKey: value => value,
    MAX_STUDY_CANDIDATES: 1000,
    invoke: async command => {
      if (command === 'study_preflight_json') return ['preflight', 'design'].includes(delayedStage) ? delay() : '';
      creates++;
      return ['create', 'unchanged'].includes(delayedStage) ? delay() : '/cases/.colm/studies/old';
    },
    setActiveStudyDirs: () => { registrations++; },
    spatialStudyReason: () => '', blockSpatialStudy: () => false,
    studyDirScopes: { uq: {} }, studyDirDesignKeys: { uq: {} },
    saveStudyDirs() {}, setPreview() {}, refreshStudy: async () => {},
    setStudyWizardPage() {}, renderStudyReadiness() {}, status() {},
  });
  const running = create('uq');
  for (let i = 0; !resolve && i < 20; i++) await Promise.resolve();
  if (!resolve) throw new Error(`create never reached ${delayedStage}`);
  if (!creating.uq) throw new Error('Study creation did not remain busy during IPC');
  await create('uq');
  if (['design', 'metadata-design', 'outputs-design'].includes(delayedStage)) design = 'changed';
  else if (delayedStage !== 'unchanged') scope = 'changed';
  resolve('/cases/.colm/studies/old');
  await running.catch(() => {});
  if (creating.uq) throw new Error('Study creation stayed busy after rejection');
  if (registrations !== Number(delayedStage === 'unchanged')
      || (['create', 'unchanged'].includes(delayedStage) ? creates !== 1 : creates !== 0)) {
    throw new Error(`Study creation crossed the project boundary after ${delayedStage}`);
  }
}

// Exercise the real async refresh orchestration with delayed IPC, not a source-string guard.
{
  let dirs = ['/cases/a/.colm/studies/one'];
  let scope = 'a';
  const pending = [];
  const rendered = [];
  const context = {
    activeStudyDirs: () => dirs,
    studyScopeKey: () => scope,
    studyRefreshRequests: { uq: 0, tuning: 0 },
    studyEvents: { uq: [], tuning: [] },
    invoke: () => new Promise(resolve => pending.push(resolve)),
    renderStudyEnvelope: (_, envelope) => rendered.push(envelope.manifest.id),
    renderStudyResults: async () => {},
    status: () => {},
  };
  const refresh = runInNewContext(resultUi.slice(
    resultUi.indexOf('async function refreshStudy('),
    resultUi.indexOf('\nasync function runStudy('),
  ) + '\nrefreshStudy;', context);
  const reply = id => JSON.stringify({ manifest: { id }, state: { status: 'ready' }, events: [] });
  const old = refresh('uq');
  const latest = refresh('uq');
  pending[1](reply('latest'));
  await latest;
  pending[0](reply('old'));
  await old;
  if (rendered.join('|') !== 'latest') throw new Error('old Study status overwrote the latest refresh');

  const changed = refresh('uq');
  scope = 'b';
  dirs = ['/cases/b/.colm/studies/two'];
  pending[2](reply('wrong-project'));
  await changed;
  if (rendered.join('|') !== 'latest') throw new Error('Study refresh crossed the current project boundary');

  dirs = ['/cases/b/.colm/studies/one', '/cases/b/.colm/studies/two'];
  context.aggregateStudyStatuses = () => 'completed';
  context.renderStudyEnvelope = (_, envelope) => rendered.push(envelope.state.warnings);
  context.invoke = async (_, { studyDir }) => JSON.stringify({ manifest: { id: studyDir.split('/').pop() }, state: { warnings: ['insufficient support'] } });
  await refresh('uq');
  if (rendered.at(-1).join('|') !== 'one: insufficient support|two: insufficient support') {
    throw new Error('multi-Study refresh lost warning provenance');
  }
  context.aggregateStudyStatuses = aggregateStudyStatuses;
  context.invoke = async (_, { studyDir }) => {
    if (studyDir.endsWith('two')) throw new Error('status unavailable');
    return JSON.stringify({ manifest: { id: 'one' }, state: { status: 'completed', tasks: { 'm000001/site': { member: 'm000001', site: 'site', status: 'succeeded' } } } });
  };
  context.renderStudyEnvelope = (_, envelope) => {
    if (aggregateStudy(envelope).status !== 'CompletedWithFailures'
        || !envelope.events.some(event => event.kind === 'study_error' && event.study_dir.endsWith('two'))) {
      throw new Error('failed Study status retrieval was hidden by the successful Study');
    }
  };
  context.renderStudyResults = async (_, envelopes) => {
    if (envelopes[0].manifest.id !== 'one' || !envelopes[1].error) throw new Error('mixed Study result envelopes lost their identities');
  };
  await refresh('uq');
}

// Backend diagnostics must be visible without loading a chart, with candidate/member denominators.
{
  const elements = [];
  const node = (tag, className, textContent = '') => {
    const element = { tag, className, textContent, append() {}, appendChild() {} };
    elements.push(element);
    return element;
  };
  const helpers = resultUi.includes('function renderUncertaintyDiagnostics(')
    ? resultUi.slice(resultUi.indexOf('function renderUncertaintyDiagnostics('), resultUi.indexOf('\nfunction renderStudyEnvelope(')) : '';
  const render = runInNewContext(helpers + resultUi.slice(
    resultUi.indexOf('async function renderStudyResults('),
    resultUi.indexOf('\nasync function refreshStudy('),
  ) + '\nrenderStudyResults;', {
    $: () => node('host'), node, aggregateStudy, studyWarnings,
    document: { createDocumentFragment: () => node('fragment') },
    resultKpi: (value, label) => node('kpi', '', `${label}: ${value}`),
    activeStudyDirs: () => ['/cases/.colm/studies/one'],
    studyResultsReady: () => true, studyResultPaths: () => new Set(),
    destroyChartsInside() {}, envelopeExplanation() {},
  });
  const tasks = Object.fromEntries([
    ['m000000', 'a', 'failed'], ['m000000', 'b', 'failed'],
    ['m000001', 'a', 'succeeded'], ['m000001', 'b', 'interrupted'],
    ['m000002', 'a', 'succeeded'], ['m000002', 'b', 'succeeded'],
    ['m000003', 'a', 'cancelled'], ['m000003', 'b', 'cancelled'],
    ['m000004', 'a', 'needs_review'], ['m000004', 'b', 'needs_review'],
    ['study', 'status-fetch-error', 'failed'],
  ].map(([member, site, status]) => [`${member}/${site}`, { member, site, status }]));
  await render('uq', [{ state: { status: 'completed_with_failures', tasks, warnings: ['site/f_lfevpa: insufficient support'] } }]);
  const text = elements.map(element => element.textContent).join('\n');
  if (!text.includes('site/f_lfevpa: insufficient support')) throw new Error('UQ backend warnings are hidden before loading a chart');
  if (!text.includes('成功候选: 1/4') || !text.includes('失败候选比例: 25.0% (1/4)')) {
    throw new Error(`UQ failure counts must exclude baseline, count members not sites, and not classify review/cancel as execution failure: ${text}`);
  }
  if (!elements.some(element => element.className === 'warn mini' && element.textContent.includes('20%'))) {
    throw new Error('UQ high-failure warning is missing');
  }
}

for (const delayedFile of ['importance.json', 'members.csv']) {
  let current = true;
  let resolve;
  const host = { children: [], appendChild(child) { this.children.push(child); } };
  const delayed = () => new Promise(done => { resolve = done; });
  const context = {
    $: () => host,
    activeStudyDirs: () => ['/cases/.colm/studies/old'],
    studyResultsReady: () => true,
    studyResultPaths: () => new Set([delayedFile]),
    studyResult: delayed,
    studyResultText: delayed,
    destroyChartsInside: () => {}, renderUncertaintyDiagnostics: () => ({}),
    node: () => ({ append() {}, appendChild() {} }),
    document: { createDocumentFragment: () => ({ appendChild() {} }) },
    envelopeExplanation: () => ({}),
  };
  const render = runInNewContext(resultUi.slice(
    resultUi.indexOf('async function renderStudyResults('),
    resultUi.indexOf('\nasync function refreshStudy('),
  ) + '\nrenderStudyResults;', context);
  const old = render('uq', [{}], () => current);
  for (let i = 0; !resolve && i < 10; i++) await Promise.resolve();
  if (!resolve) throw new Error(`result renderer never requested ${delayedFile}`);
  current = false;
  resolve(delayedFile.endsWith('.csv') ? 'member,status\nm1,succeeded' : null);
  await old;
  if (host.children.length) throw new Error(`stale ${delayedFile} appended to the current Study results`);
}

{
  let current = true;
  const elements = [];
  const pending = [];
  const rendered = [];
  const node = tag => {
    const element = { tag, value: '', append() {}, appendChild() {} };
    elements.push(element);
    return element;
  };
  const render = runInNewContext(resultUi.slice(
    resultUi.indexOf('async function renderStudyResults('),
    resultUi.indexOf('\nasync function refreshStudy('),
  ) + '\nrenderStudyResults;', {
    $: () => node('host'), node, document: { createElement: node, createDocumentFragment: () => node('fragment') },
    activeStudyDirs: () => ['/cases/.colm/studies/one'],
    studyResultsReady: () => true,
    studyResultPaths: () => new Set(['envelopes/site/a.json', 'envelopes/site/b.json']),
    studyResult: (_, path) => new Promise(resolve => pending.push({ path, resolve })),
    renderEnvelopeChart: (_, result) => rendered.push(result.variable),
    destroyChartsInside() {}, envelopeExplanation() {}, renderUncertaintyDiagnostics: () => ({}),
  });
  await render('uq', [{}], () => current);
  const select = elements.find(element => element.tag === 'select');
  const button = elements.find(element => element.tag === 'button');
  select.value = 'envelopes/site/a.json';
  const a = button.onclick();
  select.value = 'envelopes/site/b.json';
  const b = button.onclick();
  pending[1].resolve({ variable: 'b' });
  await b;
  pending[0].resolve({ variable: 'a' });
  await a;
  if (rendered.join('|') !== 'b') throw new Error('old envelope chart replaced the selected variable');
  const stale = button.onclick();
  current = false;
  pending[2].resolve({ variable: 'stale' });
  await stale;
  if (rendered.join('|') !== 'b') throw new Error('detached Study results still rendered a chart');
}

if (!resultUi.includes("site_mode: 'shared'")
    || resultUi.includes("analysis_from: design.from")
    || resultUi.includes("analysis_to: design.to")
    || resultUi.includes("$('uq-site-mode')?.value === 'independent'")) {
  throw new Error('uncertainty Study specs must default to shared mode and analyze each site full output');
}
if (!resultUi.includes("invoke('read_timing'")
    || !resultUi.includes("invoke('set_spinup'")
    || !resultUi.includes("studySpinupTarget")
    || !resultUi.includes("renderStudySpinup")
    || !resultUi.includes("wireStudyButton('tune-spinup-apply', () => applyStudySpinup('tuning'))")
    || !resultUi.includes('state.text = r.text;')
    || !resultUi.includes('预热年数和重复轮数必须是非负整数。')
    || !resultUi.includes('创建算例后显示预热设置。')
    || !resultUi.includes("const independent = kind === 'tuning' && $('tune-site-mode')?.value === 'independent'")) {
  throw new Error('uncertainty design must expose and apply model spin-up settings from the base cases');
}
if (!resultUi.includes("const seedText = $('tune-seed')?.value.trim() || ''")
    || !resultUi.includes('seed: design.seed')) {
  throw new Error('tuning random seeds must be validated and frozen like uncertainty seeds');
}
if (resultUi.includes("node('label', 'evaluation-variable study-param-option')")
    || !resultUi.includes("node('div', 'evaluation-variable study-param-option')")
    || !resultUi.includes('input.ariaLabel = label;')
    || !resultUi.includes("min.ariaLabel = `${label} 采样下界`;")
    || !resultUi.includes("max.ariaLabel = `${label} 采样上界`;")) {
  throw new Error('Study parameter range rows must not wrap numeric inputs in one label and must label lower/upper fields');
}
if (!resultCss.includes('#uq-params, #tune-params { grid-template-columns: minmax(0, 1fr); }')) {
  throw new Error('Study parameter ranges must use full-width rows so lower/upper inputs cannot overlap neighboring parameters');
}
if (!html.includes('<div class="study-control-row study-window-controls">\n                <label for="tune-from">')
    || !resultCss.includes('.study-window-controls label.check { grid-column: 1 / -1; }')) {
  throw new Error('the independent-validation toggle must span both columns so validation start/end stay on one row');
}
if (!html.includes('DE/rand/1/bin') || !html.includes('10.1023/A:1008202821328')) {
  throw new Error('the tuning design must identify its differential-evolution variant and foundational reference');
}
if (!resultUi.includes('function studyWizardIssue(kind, page)')
    || !resultUi.includes('function renderStudyWizard(kind)')
    || !resultUi.includes('function setStudyWizardPage(kind, page)')
    || !resultUi.includes('const studyWizardHelp =')
    || !resultUi.includes("$(`${prefix}-step-do`).textContent = dialogText(help[0])")
    || !resultUi.includes("$(`${prefix}-step-why`).textContent = dialogText(help[1])")
    || !resultUi.includes('setStudyWizardPage(kind, 5)')
    || !resultUi.includes('function renderStudyActions(kind)')
    || !resultUi.includes('studyActionState(summary.status, hasTask, studyRunning[kind])')
    || !resultUi.includes("$(`${prefix}-step-prev`).onclick")
    || !resultUi.includes("$(`${prefix}-step-next`).onclick")) {
  throw new Error('Study workflows must provide guarded previous/next pages and move running work to status');
}
if (!html.includes('id="uq-cancel" disabled>终止运行</button>')
    || !html.includes('id="tune-cancel" disabled>终止运行</button>')
    || !resultUi.includes("if (['retry', 'pause', 'resume'].includes(name)) button.hidden = !enabled;")) {
  throw new Error('Study termination must remain visible and become enabled when its state permits');
}
if (!resultUi.includes('const studyResultsReady = view =>')
    || !resultUi.includes('if (!studyResultsReady(studyViews[kind]))')
    || !resultUi.includes('envelopes.some(envelope => !studyResultsReady(envelope))')
    || !resultUi.includes('renderStudyWizard(flowKind);')
    || !resultUi.includes('分析任务尚未完成；请到“开始计算与监控”页启动计算，完成后再查看结果。')) {
  throw new Error('Study results must stay gated until the run reaches a result-bearing terminal state');
}

if (!resultUi.includes('参数影响诊断')
    || !resultUi.includes('sortedImportanceRows(rows).slice(0, 300)')
    || !resultUi.includes('统计值（ρ / 斜率）')
    || !resultUi.includes('Spearman ρ 的范围是 -1 到 1：1.00 表示完全正单调关系，-0.220 表示弱负单调关系')
    || !resultUi.includes('它不是百分比，也不证明因果')
    || !resultUi.includes('OAT 有限差分斜率表示“输出均值变化 / 参数变化”')
    || !resultUi.includes('不能直接横向比较')
    || !resultUi.includes('结果摘要与下一步')
    || !resultUi.includes('Top 3 按 |Spearman ρ|')
    || !resultUi.includes('样本较少、排序不稳定')
    || !resultUi.includes('只有在有独立证据支持时')
    || !resultUi.includes('按 |OAT 斜率| 浏览的 3 条（不可作为跨量纲重要性排名）')
    || !resultUi.includes('输出均值随参数增加而降低')
    || resultUi.includes('这么多输出单位')
    || !resultUi.includes('不可判定：有效成员不足、参数取值恒定或输出均值恒定。')) {
  throw new Error('uncertainty influence results must explain Spearman/OAT values, sorting, n, and non-causal/non-percent meaning');
}
if (!resultUi.includes('分位带说明：Baseline 是未扰动基准成员；P05/P50/P95')
    || !resultUi.includes('n_eff 是该时刻参与分位数计算的有限成员数')
    || !resultUi.includes('有限样本分位带不是统计置信区间')
    || !resultUi.includes("envelopeCard.append(node('h4', '', '样本分位带'), envelopeExplanation())")
    || !resultUi.includes('const diagnostics = envelopeDiagnostics(data)')
    || resultUi.includes('Math.min(...(data.n_eff')
    || !resultUi.includes('支持不足时刻数')
    || !resultUi.includes('平均/最大 P95-P05 带宽')
    || !resultUi.includes('P50 相对 baseline 平均绝对偏离')
    || !resultUi.includes('这些诊断只基于当前加载的一个站点和变量')) {
  throw new Error('uncertainty envelope chart must explain n_eff before loading and show per-chart diagnostics after loading');
}
if (!resultUi.includes('function invalidateActiveStudy(kind, reason)')
    || !resultUi.includes('const studyDirScopes = { uq: {}, tuning: {} }')
    || !resultUi.includes('const studyDirDesignKeys = { uq: {}, tuning: {} }')
    || !resultUi.includes("localStorage.setItem('colm.studyDirScopes'")
    || !resultUi.includes("localStorage.setItem('colm.studyDirDesignKeys'")
    || !resultUi.includes('studyDirScopes[kind][dir] = studyScopeKey()')
    || !resultUi.includes('studyDirDesignKeys[kind][dir] = designKeys[index]')
    || !resultUi.includes('function stableStudySpecKey(specJson)')
    || !resultUi.includes('if (spec.budget) delete spec.budget.jobs')
    || !resultUi.includes('const currentKeys = new Set(studyDesignKeys(kind))')
    || !resultUi.includes('currentKeys.has(studyDirDesignKeys[kind]?.[dir])')
    || !resultUi.includes('const studyScopeKey = () => `${currentKernel()}')
    || !resultUi.includes('invalidateActiveStudy(kind,')
    || !resultUi.includes("invalidateActiveStudy('tuning', '调优设计已修改，请重新生成调优任务。')")
    || !resultUi.includes("invalidateActiveStudy(id.startsWith('tune') ? 'tuning' : 'uq'")
    || !resultUi.includes("step.inert = current === 'Running'")
    || !resultUi.includes('setActiveStudyDirs(kind, [])')) {
  throw new Error('Study design edits must invalidate the previously generated task registration');
}
const tuningTimingLoader = resultUi.slice(resultUi.indexOf('async function initializeTuningDatesFromCases'), resultUi.indexOf('function renderStudyParams'));
if (!tuningTimingLoader.includes("invoke('read_timing', { dirs: [c.dir] })")
    || tuningTimingLoader.indexOf('tuningCasePeriods = new Map();') > tuningTimingLoader.indexOf("invoke('read_timing'")
    || !tuningTimingLoader.includes('catch { renderTuningWindowPreview(cases); }')
    || !resultUi.includes('tuningCasePeriods = new Map(periods.map')
    || !resultUi.includes('tuningWindowForCase(c, design.fromPct, design.toPct)')
    || resultUi.includes('if (tuningDatesInitialized === studyScopeKey())')
    || !resultUi.includes('运行时先用基准成员复核真实模型—观测配对数')) {
  throw new Error('Multi-site tuning must map percentage windows per site and explain the real-pair baseline gate');
}
if (!resultUi.includes('bestTuningSummary(envelope)')
    || !resultUi.includes('最优方案摘要')
    || !resultUi.includes('候选目标函数排名')
    || !resultUi.includes("invoke('study_apply_preview'")
    || !resultUi.includes('按站点、目标与时段分解')
    || !resultUi.includes('观测标准差')
    || !resultUi.includes('较 baseline 改进')) {
  throw new Error('Tuning results must show the actual best member, parameters, and ranked objectives');
}
if (!resultUi.includes('if (!matched && statusByEvent[eventKind])')
    || !resultUi.includes('候选成员按代生成；成员表、目标函数和临时最佳会继续更新')) {
  throw new Error('New DE generation members and their changing provisional results must appear during the run');
}
if (!resultUi.includes('const studyAsyncRequests =')
    || !resultUi.includes('async function loadStudyParams(stillCurrent = () => true)')
    || !resultUi.includes('async function renderStudyOutputs(stillCurrent = () => true)')
    || !resultUi.includes('async function renderTuningTargets(stillCurrent = () => true)')
    || !resultUi.includes('function studyDesign(kind)')
    || !resultUi.includes("const seedText = $('uq-seed')?.value.trim() || ''")
    || !resultUi.includes("method === 'lhs' && (!seedText")
    || !resultUi.includes('Number.isSafeInteger(seed)')
    || !resultUi.includes('seed: design.seed')
    || !resultUi.includes("for (const target of ['uq-count', 'uq-seed'])")
    || !resultUi.includes('使用对数采样时上下界必须大于 0')
    || !resultUi.includes('采样范围超出代码硬边界')) {
  throw new Error('Study page transitions must reject stale responses and invalid page-level scientific inputs');
}
if (!resultUi.includes('let activePaneRequest = 0')
    || !resultUi.includes('let activeDataBrowserRequest = 0')
    || !resultUi.includes('let activeBatchEvaluationCatalogRequest = 0')
    || !resultUi.includes('const isCurrent = () => token === activePaneRequest')
    || !resultUi.includes("state.step !== 'result-data' || activeCase()?.dir !== c.dir")
    || !resultUi.includes("state.step !== 'result-comparison' || resultScopeKey() !== scopeKey")) {
  throw new Error('result async pane refreshes must ignore stale case, step, and scope responses');
}
if (!resultUi.includes('const cached = maxPoints === null ? undefined : seriesCache.get(key)')
    || !resultUi.includes('return maxPoints === null ? data : seriesCache.set(key, data)')) {
  throw new Error('full-resolution series exports must bypass the bounded plotting LRU');
}
const paramsUi = await readFile(join(root, 'dist', 'app', 'params.js'), 'utf8');
const timingUi = await readFile(join(root, 'dist', 'app', 'timing.js'), 'utf8');
if (!timingUi.includes('Number.isSafeInteger(years)')
    || timingUi.includes("+$('tm-years').value | 0")) {
  throw new Error('basic spinup inputs must not use 32-bit truncation');
}
if (!resultUi.includes('summaryOnly') || !resultUi.includes('pairVars')
    || !resultUi.includes('false, [summaryRow.name], 2400')
    || !resultUi.includes("$('evaluation-chart-refresh').onclick")) {
  throw new Error('multi-site summaries and selected-variable chart pairs are not loaded independently');
}
const histvarsUi = await readFile(join(root, 'dist', 'app', 'histvars.js'), 'utf8');
if (!resultUi.includes('export async function markResultsStale(dirs)')
    || !resultUi.includes("state.runState[c.dir] = '需重跑'")
    || !resultUi.includes('c.has_history = false')
    || !resultUi.includes("invoke('mark_results_stale', { dirs: [...target] })")
    || !resultUi.includes("value === 'stale' ? badge('需重跑', 'warn')")
    || !paramsUi.includes("import { markResultsStale } from './results.js';")
    || !paramsUi.includes('await markResultsStale(dirs);')
    || !timingUi.includes("import { markResultsStale } from './results.js';")
    || !timingUi.includes('await markResultsStale(dirs);')
    || !histvarsUi.includes("import { markResultsStale } from './results.js';")
    || !histvarsUi.includes('await markResultsStale(dirs);')) {
  throw new Error('parameter saves must mark old history as stale and invalidate result caches');
}
if (!resultUi.includes("['待运行', '运行中'].includes(state.runState[c.dir])")) {
  throw new Error('results being regenerated must not remain readable during colm execution');
}
if (!resultUi.includes("$('result-refresh').onclick = async () =>")
    || !resultUi.includes('allCurrent().forEach(c => invalidateResultCase(c.dir))')
    || resultUi.includes("$('result-refresh').onclick = () => { catalogCache.clear();")) {
  throw new Error('manual result refresh must invalidate every current case cache, not only the catalog cache');
}
if (!resultUi.includes('const historyHealth = new Map()')
    || !resultUi.includes('hasValidatedHistory')
    || !resultUi.includes("invoke('history_catalog'")
    || !resultUi.includes('assertUsableCatalog(catalog)')
    || !resultUi.includes("['waiting', 'running'].includes(caseState(c))")
    || !resultUi.includes("await prepareActivePane();")
    || !resultUi.includes('history 文件损坏或不完整')
    || !resultUi.includes('batchEvaluationCatalogFailures')
    || !resultUi.includes('const total = resultScope().length')) {
  throw new Error('result analysis must validate history files and keep failed site catalogs in the denominator');
}
if (!resultUi.includes('allCurrent().forEach(c => invalidateResultCase(c.dir))')
    || !resultUi.includes('export async function refreshVars()')) {
  throw new Error('manual result refresh must invalidate every per-case cache before reloading');
}
for (const urban of ['f_fach', 'f_fhac', 'f_fsenroof', 'f_fvehc', 'f_lfevproof', 'f_t_roof', 'f_t_room', 'f_t_wall']) {
  if (!resultUi.includes(urban)) throw new Error(`urban history variable ${urban} lacks a readable result mapping`);
}
for (const crop of [
  'f_grainc', 'f_cropprod1c', 'f_cropprodc_rainfed_temp_corn',
  'f_plantdate_rainfed_temp_corn', 'f_gddplant', 'f_gddmaturity', 'f_hui',
]) {
  if (!resultUi.includes(crop)) throw new Error(`crop history variable ${crop} lacks a readable result mapping`);
}
if (!resultUi.includes("/crop|grain|fert|plantdate|gdd|hui/i.test(name)")) {
  throw new Error('crop history variables are not grouped as crop outputs');
}
for (const methane of [
  'f_methane_surf_flux_soil', 'f_methane_surf_flux_wetland', 'f_methane_surf_flux_lake',
  'f_methane_prod_tot', 'f_methane_oxid_tot', 'f_totcol_methane', 'f_o2_cap_gain',
  'f_CONC_O2_UNSAT', 'f_O2_DECOMP_DEPTH_UNSAT',
]) {
  if (!resultUi.includes(methane)) throw new Error(`methane history variable ${methane} lacks a readable result mapping`);
}
for (const variable of [
  'f_methane_surf_flux_global_total_with_lake',
  'f_methane_surf_flux_global_phys_with_lake',
  'f_methane_balance_residual_global_with_lake',
  'f_methane_ch4_clip_credit_global_with_lake',
]) {
  if (!resultUi.includes(`${variable}:`) || !resultUi.match(new RegExp(`${variable}:[^\\n]+mol/m²/s`))) {
    throw new Error(`${variable} must keep the model's land-area-mean flux unit`);
  }
}
if (!resultUi.includes('/methane|ch4|(^|_)o2(_|$)/i.test(name)')) {
  throw new Error('methane history variables are not grouped as methane outputs');
}
if (!resultUi.includes("invalid: 'Invalid result'") || !resultUi.includes("invalid: '结果异常'")) {
  throw new Error('printable reports do not localize invalid history results');
}
if (!resultUi.includes("label: `${meta.label} · ${variable}`")
    || !resultUi.includes("const dialogText = text =>")
    || !resultUi.includes("dialogText('导出目录')")
    || !resultUi.includes("dialogText('另存为算例目录')")
    || !resultUi.includes("dialogText('存在无法确认原进程状态的任务。仅在确认原模型进程已经退出后重试，是否继续？')")
    || !resultUi.includes("'确认并继续'")
    || !resultUi.includes('按输入指纹跳过')
    || !resultUi.includes("dialogText('即将应用以下参数改动：')")
    || !resultUi.includes("if (hasBackend) await invoke('print_report')")
    || !resultUi.includes('else if (typeof window.print')
    || !resultUi.includes('printableReportHtml') || !resultUi.includes('requestAnimationFrame')) {
  throw new Error('chart legend or printable PDF report support regressed');
}
if (!resultUi.includes("invoke('study_parameter_contexts'")
    || !resultUi.includes('parameter_id: meta?.id')
    || !resultUi.includes('scope_instance: scope ?')
    || !resultUi.includes("['pft-type', 'pc-pft-component']")
    || !resultUi.includes("invoke('study_result'")
    || !resultUi.includes('validation_from')
    || !resultUi.includes('data-tune-weight')
    || !resultUi.includes('data-study-scale')
    || !resultUi.includes('需要填写上下界')
    || !resultUi.includes('预计时间未知')
    || !resultUi.includes('磁盘需求未知')
    || !resultUi.includes("version: $('about-version')?.textContent?.trim() || 'unknown'")
    || resultUi.includes("version: '0.1.0'")
    || !resultUi.includes('覆盖 ${n}/${cases.length}')
    || !resultUi.includes('row.n === cases.length')
    || !resultUi.includes('dataset.outputSites')
    || !resultUi.includes('MAX_STUDY_CANDIDATES')
    || resultUi.includes('const studyScope = () => resultScope()')
    || !resultUi.includes('const cases = allCurrent();')
    || !resultUi.includes("invoke('hist_vars'")
    || !resultUi.includes("invoke('evaluation_plan'")
    || !resultUi.includes('renderStudyReadiness(kind)')
    || !resultUi.includes('const totalCandidates = candidateCounts.reduce')
    || !resultUi.includes('aggregateStudyStatuses(envelopes.map')
    || resultUi.includes("status: 'multiple'")
    || !resultUi.includes('studyEventText(item)')
    || !resultUi.includes('logPanel.open = true')
    || !resultUi.includes("bindStudyJobInputs('uq')")
    || !resultUi.includes("bindStudyJobInputs('tuning')")
    || !resultUi.includes("['linear', '线性（等差）']")
    || !resultUi.includes("['log', '对数（等比）']")
    || !resultUi.includes("invoke('study_apply_preview'")
    || !resultUi.includes('分析任务正在运行，不能重试')
    || !resultUi.includes('已生成但未登记的分析任务')
    || !resultUi.includes('study_key')
    || !resultUi.includes('min_pairs')
    || !resultUi.includes("invoke('study_retry'")
    || !resultUi.includes("invoke('study_preflight_json'")
    || !resultUi.includes('studyCpuCapacity')
    || !resultUi.includes('const perStudyJobs = dirs.length === 1 ? jobs : 1')
    || !resultUi.includes('boundedMap(dirs, Math.min(jobs, dirs.length)')
    || !resultUi.includes("listen('study://event'")
    || !resultUi.includes('if (!dir || !active.has(dir)) return;')) {
  throw new Error('study workflows must gate parameters, stream events, and load backend results on demand');
}
if (!resultUi.includes('tuningDatesInitialized === scopeKey')
    || !resultUi.includes('tuningDatesInitialized = scopeKey')
    || !resultUi.includes('const site = studySiteId({ dir: baseCase });')
    || resultUi.includes('const site = envelope.manifest?.spec?.base_cases?.[0] || member;')) {
  throw new Error('tuning dates and applied case names must follow the current result scope safely');
}
if (!resultUi.includes("const PLANNED_PROFILE_VARIABLES = new Set(['f_t_soisno', 'f_wliq_soisno', 'f_wice_soisno'])")
    || !resultUi.includes('!PLANNED_PROFILE_VARIABLES.has(name)')) {
  throw new Error('pre-run UQ output preview must not mislabel known vertical profiles as scalar series');
}
if (!resultUi.includes('!c.has_history || isStaleResult(c) || isActiveResult(c)')
    || resultUi.includes('try { catalog = await loadCatalog(c); }\n      catch { catalog = await plannedHistoryCatalog(c); }')
    || !resultUi.includes('不会退回计划值')
    || !resultUi.includes('以下已有结果未通过评估目录检查，不会退回计划值')
    || resultUi.includes('if (!counts.size) {\n    const failures = rows.filter')) {
  throw new Error('broken completed histories must remain visible instead of falling back to planned outputs');
}
if (!resultUi.includes('const uncovered = independent ? cases.filter')
    || !resultUi.includes("sites.split('\\u001f').includes(studySiteId(c))")
    || !resultUi.includes('uncovered.length === 0')) {
  throw new Error('independent Study readiness must require an applicable selection for every site');
}
const syntaxFile = join(temp, 'results-syntax.mjs');
await writeFile(syntaxFile, resultUi);
const syntax = spawnSync(process.execPath, ['--check', syntaxFile], { encoding: 'utf8' });
if (syntax.status !== 0) throw new Error(`results.js is not valid ESM: ${syntax.stderr}`);
const capability = await readFile(join(root, 'src-tauri', 'capabilities', 'default.json'), 'utf8');
if (!capability.includes('core:webview:allow-print')) {
  throw new Error('PDF printing lacks the Tauri webview print permission');
}

console.log('results: scope, Study controls, bounded loading, PDF, and nine panes are present');

// Delayed Study mutations must re-check the current non-spatial scope before each mutating IPC.
{
  const helper = resultUi.slice(
    resultUi.indexOf('const studyMutationGuard ='),
    resultUi.indexOf('const studyScopeKey ='),
  );
  let spatial = false;
  let runs = 0;
  const runStudy = runInNewContext(helper + resultUi.slice(
    resultUi.indexOf('async function runStudy('),
    resultUi.indexOf('\nasync function retryStudy('),
  ) + '\nrunStudy;', {
    spatialStudyReason: () => spatial ? 'spatial disabled' : '',
    studyScopeKey: () => 'scope', activeStudyDirs: () => ['/studies/a', '/studies/b'], currentKernel: () => '/kernel',
    aggregateStudy: () => ({ status: 'Ready' }), studyViews: { uq: {} }, status() {}, renderStudyReadiness() {},
    studyRunning: { uq: false }, studyJobCount: () => 1, renderStudyActions() {}, setStudyWizardPage() {},
    boundedMap: async (items, _limit, fn) => {
      const out = [];
      for (const item of items) {
        try { out.push({ ok: true, value: await fn(item) }); }
        catch (error) { out.push({ ok: false, error }); }
        spatial = true;
      }
      return out;
    },
    invoke: async command => { if (command === 'study_run') runs++; return 'ok'; },
    refreshStudy: async () => {}, setPreview() {}, renderStudyWizard() {},
  });
  await runStudy('uq').catch(() => {});
  if (runs !== 1) throw new Error('spatial switch must stop queued Study runs without killing the already-started one');
}

{
  const helper = resultUi.slice(
    resultUi.indexOf('const studyMutationGuard ='),
    resultUi.indexOf('const studyScopeKey ='),
  );
  let spatial = false;
  let retries = 0;
  let statusReads = 0;
  let failure;
  const retryStudy = runInNewContext(helper + resultUi.slice(
    resultUi.indexOf('async function retryStudy('),
    resultUi.indexOf('\nasync function controlStudy('),
  ) + '\nretryStudy;', {
    spatialStudyReason: () => spatial ? 'spatial disabled' : '',
    studyScopeKey: () => 'scope', activeStudyDirs: () => ['/studies/a'], currentKernel: () => '/kernel',
    status() {}, renderStudyReadiness() {}, studyRunning: { uq: false }, dialogText: x => x,
    globalThis: { confirm: () => true }, runStudy: async () => {},
    invoke: async command => {
      if (command === 'study_status') { statusReads++; spatial = true; return JSON.stringify({ state: { tasks: {} } }); }
      if (command === 'study_retry') retries++;
      return '';
    },
  });
  await retryStudy('uq').catch(error => { failure = error; });
  if (statusReads !== 1 || !failure?.message.includes('分析设计已修改')) throw new Error('retry must reach delayed status and fail at the scope guard');
  if (retries !== 0) throw new Error('spatial switch after Study status must stop retry mutation');
}

{
  const helper = resultUi.slice(
    resultUi.indexOf('const studyMutationGuard ='),
    resultUi.indexOf('const studyScopeKey ='),
  );
  let spatial = false;
  let applies = 0;
  let previews = 0;
  let failure;
  const applyBestCandidate = runInNewContext(helper + resultUi.slice(
    resultUi.indexOf('async function applyBestCandidate('),
    resultUi.indexOf('\nfunction wireStudyButton('),
  ) + '\napplyBestCandidate;', {
    spatialStudyReason: () => spatial ? 'spatial disabled' : '',
    studyScopeKey: () => 'scope', activeStudyDirs: () => ['/studies/a'], currentKernel: () => '/kernel',
    status() {}, renderStudyReadiness() {}, dialogText: x => x, studySiteId: ({ dir }) => dir.split('/').pop(),
    parentDir: () => '/cases', studyScope: () => [{ dir: '/cases/site' }], setPreview() {},
    globalThis: { confirm: () => true }, window: { prompt: () => '/cases/tuned' },
    bestTuningSummary,
    invoke: async command => {
      if (command === 'study_status') return JSON.stringify({ manifest: { spec: { base_cases: ['/cases/site'] } }, state: { best_member: 'm000001', candidates: { m000001: { feasible: true, calibration: 1 } } } });
      if (command === 'study_apply_preview') { previews++; spatial = true; return JSON.stringify([{ site: 'site', field: 'p', old: 1, new: 2 }]); }
      if (command === 'study_apply') applies++;
      return '';
    },
  });
  await applyBestCandidate().catch(error => { failure = error; });
  if (previews !== 1 || !failure?.message.includes('调优设计已修改')) throw new Error('apply must reach delayed preview and fail at the scope guard');
  if (applies !== 0) throw new Error('spatial switch before apply prompt/output must stop tuning apply mutation');
}

{
  const helper = resultUi.slice(
    resultUi.indexOf('const studyMutationGuard ='),
    resultUi.indexOf('const studyScopeKey ='),
  );
  let previewMember = '';
  const applyBestCandidate = runInNewContext(helper + resultUi.slice(
    resultUi.indexOf('async function applyBestCandidate('),
    resultUi.indexOf('\nfunction wireStudyButton('),
  ) + '\napplyBestCandidate;', {
    spatialStudyReason: () => '', studyScopeKey: () => 'scope', activeStudyDirs: () => ['/studies/a'], currentKernel: () => '/kernel',
    status() {}, renderStudyReadiness() {}, dialogText: x => x, studySiteId: ({ dir }) => dir.split('/').pop(),
    parentDir: () => '/cases', studyScope: () => [{ dir: '/cases/site' }], setPreview() {},
    globalThis: { confirm: () => false }, window: { prompt: () => { throw new Error('prompt should not run'); } },
    bestTuningSummary,
    invoke: async (command, args) => {
      if (command === 'study_status') return JSON.stringify({
        manifest: { spec: { base_cases: ['/cases/site'] } },
        state: {
          best_member: 'm999999',
          candidates: {
            m000001: { feasible: true, calibration: 0.8 },
            m999999: { feasible: false, calibration: 0.1, reason: 'infeasible target' },
          },
        },
      });
      if (command === 'study_apply_preview') {
        previewMember = args.member;
        return JSON.stringify([{ site: 'site', field: 'p', old: 1, new: 2 }]);
      }
      throw new Error(`unexpected IPC ${command}`);
    },
  });
  await applyBestCandidate();
  if (previewMember !== 'm000001') throw new Error('apply preview used stale/infeasible state.best_member instead of the feasible tuning winner');
}

// A delayed final retry/resume must not start a new scope's Study as its tail action.
for (const action of ['retry', 'resume']) {
  for (const switchScope of [false, true]) {
    let scope = 'old';
    let mutations = 0;
    let runs = 0;
    let caught;
    const helper = resultUi.slice(resultUi.indexOf('const studyMutationGuard ='), resultUi.indexOf('const studyScopeKey ='));
    const name = action === 'retry' ? 'retryStudy' : 'controlStudy';
    const end = action === 'retry' ? '\nasync function controlStudy(' : '\nasync function exportStudy(';
    const fn = runInNewContext(helper + resultUi.slice(resultUi.indexOf(`async function ${name}(`), resultUi.indexOf(end)) + `\n${name};`, {
      spatialStudyReason: () => '', studyScopeKey: () => scope,
      activeStudyDirs: () => [scope === 'old' ? '/old-study' : '/new-study'], currentKernel: () => '/kernel',
      status() {}, renderStudyReadiness() {}, studyRunning: { uq: false }, dialogText: x => x,
      globalThis: { confirm: () => true }, runStudy: async () => { runs++; }, refreshStudy: async () => {},
      invoke: async command => {
        if (command === 'study_status') return JSON.stringify({ state: { tasks: {} } });
        if (command === `study_${action}`) { mutations++; if (switchScope) scope = 'new'; return 'ok'; }
        throw new Error(`unexpected IPC ${command}`);
      },
    });
    try { await fn('uq', action); } catch (error) { caught = error; }
    if (mutations !== 1 || runs !== (switchScope ? 0 : 1)) throw new Error(`delayed ${action} must only launch the original unchanged site scope`);
    if (switchScope ? !caught?.message.includes('分析设计已修改') : caught) throw new Error(`unexpected ${action} result: ${caught}`);
  }
}
