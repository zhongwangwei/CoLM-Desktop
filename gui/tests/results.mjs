import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { spawnSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-results-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;

const {
  LruCache, boundedMap, metricKey, resultCases, rowsToCsv,
} = await import(moduleUrl('result-model.js'));
const { WORKFLOW } = await import(moduleUrl('shell.js'));
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
for (const id of ['result-uncertainty', 'result-tuning', 'result-export']) {
  if (!groupFor(id).steps[0].need()?.includes('至少一个算例')) {
    throw new Error(`${id} must be disabled, not hidden, before results exist`);
  }
}

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
  'uq-create', 'uq-run', 'uq-status', 'uq-retry', 'uq-from', 'uq-to',
  'uq-range-confirm',
  'tune-create', 'tune-run', 'tune-status', 'tune-retry', 'tune-val-from', 'tune-val-to',
  'tune-min-pairs', 'tune-range-confirm', 'evaluation-chart-refresh', 'export-pdf',
]) {
  if (!html.includes(`id="${control}"`)) throw new Error(`results workbench is missing ${control}`);
}
for (const metric of ['abs_bias', 'nse', 'r']) {
  if (!html.includes(`value="${metric}"`)) throw new Error(`tuning metric selector is missing ${metric}`);
}
const resultUi = await readFile(join(root, 'dist', 'app', 'results.js'), 'utf8');
if (!resultUi.includes('summaryOnly') || !resultUi.includes('pairVars')
    || !resultUi.includes('false, [summaryRow.name], 2400')
    || !resultUi.includes("$('evaluation-chart-refresh').onclick")) {
  throw new Error('multi-site summaries and selected-variable chart pairs are not loaded independently');
}
if (!resultUi.includes("label: `${meta.label} · ${variable}`")
    || !resultUi.includes("const dialogText = text =>")
    || !resultUi.includes("dialogText('导出目录')")
    || !resultUi.includes("dialogText('另存为算例目录')")
    || !resultUi.includes("dialogText('存在无法确认原进程状态的任务。仅在确认原模型进程已经退出后重试，是否继续？')")
    || !resultUi.includes("dialogText('即将应用以下参数改动：')")
    || !resultUi.includes("if (hasBackend) await invoke('print_report')")
    || !resultUi.includes('else if (typeof window.print')
    || !resultUi.includes('printableReportHtml') || !resultUi.includes('requestAnimationFrame')) {
  throw new Error('chart legend or printable PDF report support regressed');
}
if (!resultUi.includes("invoke('field_states_batch'")
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
    || !resultUi.includes('const studyScope = () => resultScope()')
    || !resultUi.includes('const totalCandidates = candidateCounts.reduce')
    || !resultUi.includes('state: { status: \'multiple\', tasks, candidates }')
    || !resultUi.includes("invoke('study_apply_preview'")
    || !resultUi.includes('Study 正在运行，不能重试')
    || !resultUi.includes('已创建但未登记的 Study')
    || !resultUi.includes('study_key')
    || !resultUi.includes('min_pairs')
    || !resultUi.includes('analysis_from: from')
    || !resultUi.includes("invoke('study_retry'")
    || !resultUi.includes("invoke('study_preflight_json'")
    || !resultUi.includes('studyCpuCapacity')
    || !resultUi.includes('const perStudyJobs = dirs.length === 1 ? jobs : 1')
    || !resultUi.includes('boundedMap(dirs, Math.min(jobs, dirs.length)')
    || !resultUi.includes("listen('study://event'")
    || !resultUi.includes('if (!dir || !active.has(dir)) return;')) {
  throw new Error('study workflows must gate parameters, include windows, stream events, and load backend results on demand');
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
