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
const { appendLogText, progressText } = await import(moduleUrl('run-format.js'));
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
const long = appendLogText('x'.repeat(59999), ['site-only']);
if (long.length > 40020 || !long.endsWith('site-only\n')) {
  throw new Error('per-site log ring did not retain the newest lines');
}
const runner = await readFile(join(root, 'dist', 'app', 'runner.js'), 'utf8');
if (!runner.includes('function failPendingRuns(reason)')
    || !/catch \(e\) \{\s*failPendingRuns\(e\);/.test(runner)) {
  throw new Error('a rejected batch launch must clear every pending per-site run');
}
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
const outputVariables = html.indexOf('输出变量（按需展开）');
const startRun = html.indexOf('<h3>开始运行</h3>');
if (outputVariables < 0 || startRun < 0 || outputVariables > startRun) {
  throw new Error('start-run card must be below output variables');
}
if (!/<div id="domaingate" class="gate">/.test(html)) {
  throw new Error('the startup wizard must be visible before JavaScript initializes');
}

console.log('runview: per-site progress/log formatting and undefined metrics are safe');
