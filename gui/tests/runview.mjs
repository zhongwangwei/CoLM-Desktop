import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-runview-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');

const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;
const { metricText } = await import(moduleUrl('metric-format.js'));
const { acceptsRunEvent, appendLogText, progressText } = await import(moduleUrl('run-format.js'));
if (metricText(null) !== '—' || metricText(Number.NaN) !== '—') {
  throw new Error('undefined metrics must render without calling toFixed');
}
if (metricText(0.125, 2, true) !== '+0.13' || metricText(-0.125, 2, true) !== '-0.13') {
  throw new Error('finite metric formatting changed');
}
if (metricText(0.123456789) !== '0.123457' || metricText(0.0000001234, 4) !== '0.000000123') {
  throw new Error('result metrics must retain enough precision to distinguish small non-zero values from zero');
}
if (progressText({ step: 12, total_steps: 48, date: '2008-01-01-21600' })
    !== '第 12/48 步 · 2008-01-01-21600') {
  throw new Error('per-site progress text lost its exact step count');
}
if (!progressText({ step: 2, total_steps: 8, date: 'x', spinup: [2, 3] }).startsWith('预热 2/3 轮')) {
  throw new Error('per-site progress must distinguish spin-up cycles');
}
if (progressText({}, '已取消') !== '已取消') {
  throw new Error('cancelled runs must not be rendered as failures');
}
if (acceptsRunEvent(['/new'], '/old', false, 'run-2', 'run-2')
    || acceptsRunEvent(['/new'], '/new', false, 'run-2', 'run-1')
    || !acceptsRunEvent(['/new'], '/new', false, 'run-2', 'run-2')
    || !acceptsRunEvent([], '/restored', true, null, 'run-restored')) {
  throw new Error('late events from a previous run must not re-enter the current run view');
}
const long = appendLogText('x'.repeat(59999), ['site-only']);
if (long.length > 40020 || !long.endsWith('site-only\n')) {
  throw new Error('per-site log ring did not retain the newest lines');
}
const runner = await readFile(join(root, 'dist', 'app', 'runner.js'), 'utf8');
if (!runner.includes('function failPendingRuns(reason)')
    || !/catch \(e\) \{\s*failPendingRuns\(e\);/.test(runner)) {
  throw new Error('a rejected batch launch must clear every pending per-site run');
}
if (!runner.includes("dirs.length === 1\n    ? progressText")) {
  throw new Error('single-case overall progress must retain detailed spin-up text');
}
const css = await readFile(join(root, 'dist', 'app', 'style.css'), 'utf8');
if (!/--live-w/.test(css) || !/--live-h/.test(css) || !/col-resize/.test(css)
    || !/row-resize/.test(css) || !/#log\s*\{[^}]*resize:\s*vertical/s.test(css)) {
  throw new Error('live log panel must be resizable');
}
const mainJs = await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8');
if (!mainJs.includes("stacked ? '--live-h' : '--live-w'")
    || !mainJs.includes("$('live-resizer').addEventListener('pointerdown', beginLiveResize)")
    || !mainJs.includes("window.addEventListener('pointermove', apply)")) {
  throw new Error('right live panel border drag must update --live-w after leaving the border');
}
if (!/catch \(e\) \{\s*setStatus\('后端出错：' \+ e\);\s*throw e;/s.test(mainJs)) {
  throw new Error('a failed backend boot must keep the loading gate visible');
}

const spatialJs = await readFile(join(root, 'dist', 'app', 'spatial.js'), 'utf8');
if (!spatialJs.includes("root: $('spatial-root')?.value.trim()")
    || !spatialJs.includes('wizard: state.wizard')
    || !spatialJs.includes('const context = spatialContext()')
    || !spatialJs.includes('当前已切换工作流，未加入本次算例列表')
    || spatialJs.indexOf('state.createdCases.add(out)') < spatialJs.indexOf("const cases = await invoke('list_cases'")) {
  throw new Error('late spatial case creation must capture full context and avoid state writes before the current check');
}
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
const outputVariables = html.indexOf('输出变量（按需展开）');
const startRun = html.indexOf('<h3>开始运行</h3>');
if (outputVariables < 0 || startRun < 0 || outputVariables > startRun) {
  throw new Error('start-run card must be below output variables');
}
const runSection = html.slice(startRun, html.indexOf('</section>', startRun));
if ((html.match(/id="cpu-workers"/g) || []).length !== 1
    || !runSection.includes('id="cpu-workers"')
    || !runSection.includes('id="cpu-capacity"')
    || !runSection.includes('批量并行算例数')) {
  throw new Error('batch parallelism must be configured once, next to the Step 3 run controls');
}
const runnerJs = await readFile(join(root, 'dist', 'app', 'runner.js'), 'utf8');
if (!runnerJs.includes('const spatialDefaultRanks = Math.min(8, cpuCapacity);')
    || !runnerJs.includes("else if (!mpiRanksCustomized) $('mpi-ranks').value = String(spatialDefaultRanks);")) {
  throw new Error('spatial cases must default to a bounded MPI rank count without overriding a user choice');
}
const expectedRunButtons = [
  ['run-mksrfdata', '运行 mksrfdata'],
  ['run-mkinidata', '运行 mkinidata'],
  ['run-colm', '运行 colm'],
  ['runall', '运行全部'],
  ['cancel-run', '终止运行'],
];
for (const [id, label] of expectedRunButtons) {
  if (!new RegExp(`<button[^>]+id="${id}"[^>]*>[^<]*${label}`).test(runSection)) {
    throw new Error(`start-run card is missing ${label}`);
  }
}
const resultsJs = await readFile(join(root, 'dist', 'app', 'results.js'), 'utf8');
if (resultsJs.includes('/DEF_domain%|DEF_file_mesh')
    || !resultsJs.includes('const spatialCaseEntry = c => c?.spatial === true')) {
  throw new Error('Study blocking must rely on authoritative case.spatial metadata, not raw case.nml text or aliases');
}
const controlStudy = resultsJs.slice(resultsJs.indexOf('async function controlStudy('), resultsJs.indexOf('\nasync function exportStudy('));
if (!controlStudy.includes("action === 'resume' ? spatialStudyReason() : ''")
    || /if \(spatialStudyReason\(\)\)/.test(controlStudy)) {
  throw new Error('pause/cancel controls must stay usable; only resume is spatial-blocked');
}

if (runner.includes("early state") || runner.includes("不建议使用")) {
  throw new Error('kernel presets must not carry an early-state label');
}
const domainJs = await readFile(join(root, 'dist', 'app', 'domain.js'), 'utf8');
if (!domainJs.includes("已有非结构 mesh NetCDF（必需）")
    || !domainJs.includes("picked.grid === 'unstructured') return s.meshFile")
    || !domainJs.includes("if (picked.grid === 'unstructured') {")
    || !domainJs.includes("经纬度网格设置")
    || !domainJs.includes('meshFile: picked.spatial.meshFile')) {
  throw new Error('the spatial wizard must require an existing unstructured mesh and retain lat-lon settings');
}
if (!spatialJs.includes('meshFile: grid.meshFile ?? null')
    || !spatialJs.includes('读取网格、预检并建算例')) {
  throw new Error('the spatial wizard must forward and read an existing mesh through the native backend');
}
if (!runner.includes("const RUN_STAGES = ['mksrfdata', 'mkinidata', 'colm', null]")) {
  throw new Error('the four run buttons must map to three individual stages and the full workflow');
}
if (!runner.includes("['begin', 'failed'].includes(colmState)")) {
  throw new Error('a failed colm stage must leave its partially replaced history unavailable');
}
if (!runner.includes("invoke('cancel_runs', { cases })")
    || !runner.includes("d.cancelled ? '已取消'")
    || !runner.includes('maxConcurrent: requestedWorkers()')) {
  throw new Error('run cancellation must reach the backend and keep its own terminal state');
}
if (runner.includes('status(state.runCancelled.has')
    || !runner.includes('const terminal = d.cancelled')) {
  throw new Error('terminal cancellation status must come from run://done, not event ordering');
}
if (!runner.includes("hasTracer(state.wizard, 'methane')")) {
  throw new Error('restored methane cases must keep the required runtime directory control visible');
}
if (!/<div id="loadinggate" class="gate loading-gate"[^>]*>/.test(html)
    || !/<div id="launchgate" class="gate launch-gate" hidden>/.test(html)
    || !/<div id="domaingate" class="gate" hidden>/.test(html)) {
  throw new Error('the loading page must be visible before JavaScript initializes');
}

console.log('runview: per-site progress/log formatting and undefined metrics are safe');

// 空间算例的日期是文本框（WKWebView 里 `type="date"` 不能用键盘输入）：几种常见写法都能规范成 YYYY-MM-DD。
{
  const body = spatialJs.match(/export function normalizeDate\(text\) \{[\s\S]*?\n\}/)?.[0];
  if (!body) throw new Error('spatial.js must define normalizeDate');
  if (/type="date"/.test(await readFile(join(root, 'dist', 'index.html'), 'utf8'))) {
    throw new Error('date inputs must stay keyboard-editable text fields');
  }
  const normalizeDate = new Function(`${body.replace('export ', '')}; return normalizeDate;`)();
  const cases = [
    ['2010-01-01', '2010-01-01'], ['2010-1-1', '2010-01-01'], ['2010/1/31', '2010-01-31'],
    ['2010.12.3', '2010-12-03'], ['20100215', '2010-02-15'], [' 2012-02-29 ', '2012-02-29'],
    ['2010-02-29', null], ['2010-13-01', null], ['10-01-2010', null], ['', null],
  ];
  for (const [text, want] of cases) {
    const got = normalizeDate(text);
    if (got !== want) throw new Error(`normalizeDate(${JSON.stringify(text)}) = ${got}, want ${want}`);
  }
}

// Rust 引擎的空间算例用每算例线程数代替 MPI 进程数，两条运行路径都把它交给后端。
{
  const runner = await readFile(new URL('../dist/app/runner.js', import.meta.url), 'utf8');
  const html = await readFile(new URL('../dist/index.html', import.meta.url), 'utf8');
  if (!/id="case-threads"/.test(html)) throw new Error('the run page must offer threads per case');
  if ((runner.match(/engine: modelEngine\(\), threads,/g) ?? []).length !== 2) {
    throw new Error('run_case and run_batch must both receive the requested threads');
  }
  const css = await readFile(new URL('../dist/app/style.css', import.meta.url), 'utf8');
  if (!/\.run-parallel-setting\[hidden\] \{ display: none; \}/.test(css)) {
    throw new Error('hidden parallel settings must not stay visible under display:grid');
  }
  if (!/state\.selected\?\.spatial === true/.test(runner)) {
    throw new Error('an opened spatial case counts as spatial without the wizard');
  }
  if (!/\$\('workers-setting'\)\.hidden = spatial;/.test(runner)
    || !/\$\('mpi-setting'\)\.hidden = !mpi;/.test(runner)
    || !/\$\('threads-setting'\)\.hidden = !\(spatial && !mpi\)/.test(runner)) {
    throw new Error('threads per case replaces MPI ranks only for Rust spatial runs');
  }
}
