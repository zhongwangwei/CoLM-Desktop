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
for (const text of ['尚未运行过基准算例也可以配置', '尚未运行过原算例也可以准备', '先设计，再准备任务，最后开始计算', '先确认观测与目标，再开始搜索']) {
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
for (const metric of ['abs_bias', 'nse', 'r']) {
  if (!html.includes(`value="${metric}"`)) throw new Error(`tuning metric selector is missing ${metric}`);
}
const resultUi = await readFile(join(root, 'dist', 'app', 'results.js'), 'utf8');
const resultCss = await readFile(join(root, 'dist', 'app', 'style.css'), 'utf8');

if (!resultUi.includes("site_mode: 'shared'")
    || resultUi.includes("analysis_from: design.from")
    || resultUi.includes("analysis_to: design.to")
    || resultUi.includes("$('uq-site-mode')?.value === 'independent'")) {
  throw new Error('uncertainty Study specs must default to shared mode and analyze each site full output');
}
if (!resultUi.includes("invoke('read_timing'")
    || !resultUi.includes("invoke('set_spinup'")
    || !resultUi.includes("uqSpinupTarget")
    || !resultUi.includes("renderUqSpinup")
    || !resultUi.includes('state.text = r.text;')
    || !resultUi.includes('预热年数和重复轮数必须是非负整数。')
    || !resultUi.includes('创建算例后显示预热设置。')
    || !resultUi.includes("const independent = kind === 'tuning' && $('tune-site-mode')?.value === 'independent'")) {
  throw new Error('uncertainty design must expose and apply model spin-up settings from the base cases');
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
