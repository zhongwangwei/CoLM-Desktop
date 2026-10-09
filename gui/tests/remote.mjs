// 服务器运行面板（remote.js）的纯函数：路径对应的解析与回写、探测摘要、作业状态文字。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-remote-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
// 只测纯函数：把依赖 DOM 与后端的模块换成空壳。
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = (...args) => globalThis.remoteInvoke(...args); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = id => globalThis.remoteNodes?.[id] ?? null; export const status = () => {}; export const baseName = p => String(p).split("/").pop();',
  'sites.js': 'export const renderCases = () => {};',
  'results.js': 'export const invalidateResultCase = () => {};',
  'shell.js': 'export const setRunning = () => {};',
  'state.js': 'export const state = {};',
  'engine.js': 'export const modelEngine = () => "rust";',
})) await writeFile(join(temp, 'app', name), body);
// Expose internal actions only in the temporary test copy.
await writeFile(join(temp, 'app', 'remote.js'), (await readFile(join(temp, 'app', 'remote.js'), 'utf8')) + '\nexport { resumeJobs, refreshJob, fetchJob, renderJobs, jobs };\n');
const remote = await import(pathToFileURL(join(temp, 'app', 'remote.js')).href);

assert.deepEqual(
  remote.parseMaps(' /Volumes/Data/Data/PLUMBER2s = /media/zhwei/data02/zhwei/training2026/PLUMBER2s \n\n/Users/me/raw=/data/raw\n'),
  [
    { local: '/Volumes/Data/Data/PLUMBER2s', remote: '/media/zhwei/data02/zhwei/training2026/PLUMBER2s' },
    { local: '/Users/me/raw', remote: '/data/raw' },
  ],
);
assert.throws(() => remote.parseMaps('/a /b'), /第 1/);
assert.throws(() => remote.parseMaps('\n/a = '), /第 2/);
assert.equal(remote.formatMaps([{ local: '/a', remote: '/b' }, { local: '/c', remote: '/d' }]), '/a = /b\n/c = /d');
assert.deepEqual(remote.parseMaps(remote.formatMaps([{ local: '/a', remote: '/b' }])), [{ local: '/a', remote: '/b' }]);

const lines = remote.probeLines({ hostname: 'T7920', os: 'Ubuntu 26.04.1 LTS', arch: 'x86_64', cpus: 96, memory_gb: 750, root_free_gb: 26397, cargo: 'cargo 1.95.0', gfortran: 'GNU Fortran 15.2', mpi: null, schedulers: [] });
assert.equal(lines[0], 'T7920 · Ubuntu 26.04.1 LTS · x86_64');
assert.match(lines[1], /96 核 · 750 GB 内存/);
assert.match(lines[3], /无（直接在后台运行）/);

assert.match(remote.jobSummary({ state: 'running', phase: 'building the Rust engine' }), /编译 Rust 引擎/);
assert.equal(remote.jobSummary({ state: 'running', phase: 'running', progress: { step: 8808, total_steps: 17616, date: '2004-07-01-0' } }), '运行中 50% · 2004-07-01-0');
assert.equal(remote.jobSummary({ state: 'finished', exit_code: 0, fetched: true }), '完成，结果已取回');
assert.equal(remote.jobSummary({ state: 'finished', exit_code: 101 }), '失败（退出码 101）');
assert.match(remote.jobSummary({ state: 'lost' }), /进程不在了/);
assert.match(remote.jobSummary({ state: 'running', error: 'ssh broke' }), /出错：ssh broke/);

// 调度系统（R2）：排队、被调度器杀掉的作业、队列列表、其他指令的解析。
assert.equal(remote.jobSummary({ state: 'queued', detail: 'PENDING' }), '在调度系统里排队（PENDING）');
assert.equal(remote.jobSummary({ state: 'queued' }), '在调度系统里排队');
assert.equal(remote.jobSummary({ state: 'lost', detail: 'TIMEOUT|0:0' }), '作业没有留下退出码，调度系统说：TIMEOUT|0:0');
assert.deepEqual(remote.parseDirectives(' --constraint=ib \n\n--exclusive\n'), ['--constraint=ib', '--exclusive']);
assert.deepEqual(remote.parseDirectives(''), []);
const withQueues = remote.probeLines({ hostname: 'c1', os: 'Linux', arch: 'x86_64', schedulers: ['slurm'], queues: ['cpu', 'gpu'] });
assert.equal(withQueues.at(-1), '分区或队列: cpu, gpu');
assert.equal(remote.probeLines({ hostname: 'c', os: 'L', arch: 'x86_64', engine: 'prebuilt' }).at(-1), '引擎: 传预编的程序（这台服务器不用编译）');
assert.equal(remote.probeLines({ hostname: 'c', os: 'L', arch: 'x86_64', engine: 'source' }).at(-1), '引擎: 在服务器上从源码编译');
assert.equal(withQueues.at(-2), '调度系统: slurm');

// R5：只取了所选变量的作业。
assert.equal(remote.jobSummary({ state: 'finished', exit_code: 0, fetched: true, partial: true }), '完成，已取回所选变量（可以再取回全部）');
assert.equal(remote.jobSummary({ state: 'finished', exit_code: 0, fetched: true, partial: false }), '完成，结果已取回');

// Fortran 引擎（R4）：只有它用 MPI 进程数；服务器上的内核摘要。
assert.deepEqual(remote.engineChoice('rust', '8'), { engine: 'rust', ranks: null });
assert.deepEqual(remote.engineChoice('fortran', '8'), { engine: 'fortran', ranks: 8 });
assert.deepEqual(remote.engineChoice('fortran', 'abc'), { engine: 'fortran', ranks: 1 });
assert.deepEqual(remote.engineChoice('fortran', '0'), { engine: 'fortran', ranks: 1 });
assert.deepEqual(remote.kernelLines([]), ['服务器上还没有内核']);
assert.deepEqual(
  remote.kernelLines([{ name: 'latlon-ab12', preset: 'latlon', full: true }, { name: 'default-cd34', preset: 'default', full: false }]),
  ['latlon-ab12 · latlon · 完整', 'default-cd34 · default · 只有清单（Rust 引擎用）'],
);

// 运行页与首页都接上了：运行按钮在选了服务器时交给 remote.js；首页的服务器卡片可点。
const runner = await readFile(join(root, 'dist', 'app', 'runner.js'), 'utf8');
assert.match(runner, /if \(runTarget\(\) !== 'local'\) \{\s*await remoteRun\(/);
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['run-target', 'manage-servers', 'remote-runs', 'remote-dialog', 'remote-host', 'remote-root', 'remote-maps', 'remote-test', 'remote-save', 'remote-scheduler', 'remote-partition', 'remote-account', 'remote-walltime', 'remote-cpus', 'remote-memory', 'remote-env', 'remote-directives', 'preview-job', 'remote-preview-dialog', 'remote-preview-text', 'remote-nodes', 'remote-kernel-preset', 'remote-list-kernels', 'remote-build-kernel', 'remote-kernels-result', 'remote-fetch-vars']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
console.log('remote: maps, probe summary, job states and wiring ok');

const { state } = await import(pathToFileURL(join(temp, 'app', 'state.js')).href);
for (const history of [true, false]) {
  remote.jobs.clear();
  state.selected = { dir: '/case' };
  state.cases = [{ dir: '/case', has_history: history }];
  const calls = [];
  globalThis.remoteInvoke = async name => {
    calls.push(name);
    return { record: { host: 'server', job: 'old-job' }, status: { state: 'finished', exit_code: 0 } };
  };
  await remote.resumeJobs();
  assert.equal(calls.includes('remote_fetch'), !history);
  if (history) {
    assert.equal(remote.jobs.get('/case').fetched, false);
    await remote.refreshJob('/case');
    assert.equal(calls.includes('remote_fetch'), false);
    await remote.fetchJob('/case', true);
    assert.equal(calls.at(-1), 'remote_fetch');
  }
}
remote.jobs.clear();
state.cases = [{ dir: '/case', has_history: false }];
let statusCalls = 0;
const identityCalls = [];
globalThis.remoteInvoke = async name => {
  identityCalls.push(name);
  return { record: { host: 'server', job: ++statusCalls === 1 ? 'old-job' : 'new-job' }, status: { state: 'finished', exit_code: 0 } };
};
await remote.resumeJobs();
assert.equal(identityCalls.includes('remote_fetch'), false);
console.log('remote: resume preserves local history, explicit fetch, and changed job identity');

remote.jobs.set('/case', { host: 'server', job: 'new-job', fetched: true });
globalThis.remoteInvoke = async name => {
  identityCalls.push(name);
  return { record: { host: 'other-server', job: 'new-job' }, status: { state: 'finished', exit_code: 0 } };
};
await remote.refreshJob('/case');
assert.equal(remote.jobs.get('/case').fetched, false);
assert.equal(identityCalls.includes('remote_fetch'), false);
assert.equal(remote.jobSummary(remote.jobs.get('/case')), '完成，等待手动取回结果');

const nodes = [];
globalThis.document = { createElement: tag => {
  const node = { tag, children: [], append(...items) { this.children.push(...items); }, appendChild(item) { this.children.push(item); }, replaceChildren() { this.children = []; } };
  nodes.push(node);
  return node;
} };
globalThis.remoteNodes = { 'remote-runs': document.createElement('div'), 'remote-run-list': document.createElement('div') };
remote.renderJobs();
const fetchButton = nodes.find(node => node.tag === 'button' && node.textContent === '取回全部变量');
assert.ok(fetchButton, 'resumed job with preserved local history must remain explicitly fetchable');
fetchButton.onclick();
await new Promise(resolve => setImmediate(resolve));
assert.equal(identityCalls.at(-1), 'remote_fetch');
assert.equal(remote.jobs.get('/case').fetched, true);
