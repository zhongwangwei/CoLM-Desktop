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
  throw new Error('batch parallelism must be configured once, next to the Step 4 run controls');
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
if (!runner.includes("state.wizard?.tracer === 'methane'")) {
  throw new Error('restored methane cases must keep the required runtime directory control visible');
}
if (!/<div id="loadinggate" class="gate loading-gate"[^>]*>/.test(html)
    || !/<div id="launchgate" class="gate launch-gate" hidden>/.test(html)
    || !/<div id="domaingate" class="gate" hidden>/.test(html)) {
  throw new Error('the loading page must be visible before JavaScript initializes');
}

console.log('runview: per-site progress/log formatting and undefined metrics are safe');
