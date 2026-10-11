// 结果页汇总点（results-hub.js）的纯函数：运行列表、研究列表、白话评估、推荐与理由。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-results-hub-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = async () => ({}); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = () => null; export const status = () => {};',
  'shell.js': 'export const go = () => {};',
})) await writeFile(join(temp, 'app', name), body);
globalThis.addEventListener = () => {};
const hub = await import(pathToFileURL(join(temp, 'app', 'results-hub.js')).href);

const summary = {
  derived_from: { study: 's-2', kind: 'tuning', member: 'm7' },
  runs: [
    { version: { kind: 'workspace', name: 'snow', head: 'a' }, engine: 'rust', location: { kind: 'server', host: '7920ts' }, ok: true, source_changed: true, finished: 1791734900 },
    { version: { kind: 'official' }, engine: 'fortran', location: { kind: 'local' }, ok: false, ranks: 4, finished: 1791734000 },
  ],
  studies: [{ study: 's-2', dir: '/p/.colm/studies/s-2', kind: 'tuning', status: 'completed', best_member: 'm7', best_objective: 0.31234, version: null, location: '7920ts' }],
};
const rows = hub.runRows(summary);
assert.deepEqual(rows[0].chips, ['修改版：snow', '率定参数 · 研究 s-2 成员 m7', '7920ts · Rust 引擎']);
assert.deepEqual(rows[0].flags, ['源码之后又改过']);
assert.ok(rows[0].current && rows[0].modified);
assert.match(rows[0].source, /^服务器 · \d+月\d+日$/);
assert.deepEqual(rows[1].chips, ['官方版', '率定参数 · 研究 s-2 成员 m7', '本机 · Fortran 内核']);
assert.deepEqual(rows[1].flags, ['这次运行失败', '4 个 MPI 进程']);
assert.equal(hub.paramsLabel(null), '本算例的参数');
const studies = hub.studyRows(summary);
assert.deepEqual(studies[0].chips, ['官方版', '参数率定 s-2', '服务器 7920ts']);
assert.equal(studies[0].status, '已完成');
assert.equal(studies[0].best, '最优 m7 · 目标 0.3123');

// 白话评估：偏差超过观测标准差两成算系统偏高/偏低。
assert.equal(hub.metricSentence({ label_zh: '感热', units: 'W/m2', nse: 0.31, bias: 12.5, obs_sd: 40 }).text, '感热：系统偏高（平均偏差 +12.50 W/m2，NSE 0.31）');
assert.equal(hub.metricSentence({ label_zh: '潜热', nse: 0.72, bias: 1, obs_sd: 40 }).text, '潜热：与观测基本吻合（NSE 0.72）');
assert.match(hub.metricSentence({ name: 'GPP', nse: 0.2, bias: -9, obs_sd: 10 }).text, /^GPP：系统偏低/);
assert.match(hub.metricSentence({ name: 'GPP', nse: 0.2, bias: 0.1, obs_sd: 10 }).text, /变化过程对不上/);

// 推荐：修改版的结果先公平比较；率定过还差查过程；没率定过先调参数；指标尚可看稳不稳；没指标不推荐。
const metrics = [{ label_zh: '感热', nse: 0.31 }, { label_zh: '潜热', nse: 0.7 }];
assert.equal(hub.recommendRoute(metrics, summary).route, 'compare');
const official = { runs: [{ version: { kind: 'official' } }], studies: [] };
assert.equal(hub.recommendRoute(metrics, official).route, 'params');
assert.match(hub.recommendRoute(metrics, official).reason, /感热 的 NSE 只有 0.31，还没率定过/);
const calibrated = { ...official, studies: [{ kind: 'tuning', status: 'completed' }] };
assert.equal(hub.recommendRoute(metrics, calibrated).route, 'process');
assert.match(hub.recommendRoute(metrics, calibrated).reason, /率定之后仍只有 0.31/);
assert.equal(hub.recommendRoute(metrics, { ...official, derived_from: { kind: 'tuning' } }).route, 'process');
assert.equal(hub.recommendRoute([{ label_zh: '潜热', nse: 0.8 }], official).route, 'robust');
assert.equal(hub.recommendRoute([], official), null);
assert.deepEqual(hub.ROUTES.map(r => r.id), ['params', 'process', 'robust', 'compare']);
assert.deepEqual(hub.ROUTES.flatMap(r => r.actions.map(a => a[1])), ['result-tuning', 'hybrid-learn', 'code', 'hybrid-process', 'result-uncertainty', 'result-tuning']);

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['hub-runs', 'hub-summary', 'hub-routes', 'result-next-list']) assert.ok(html.includes(`id="${id}"`), id);
assert.match(await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8'), /import '\.\/results-hub\.js';/);
console.log('results-hub: runs, studies, plain-language summary, recommendations and wiring ok');
