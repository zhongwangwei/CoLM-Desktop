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
  'ipc.js': 'export const invoke = async () => ({}); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = () => null; export const status = () => {}; export const baseName = p => String(p).split("/").pop();',
  'sites.js': 'export const renderCases = () => {};',
  'results.js': 'export const invalidateResultCase = () => {};',
  'shell.js': 'export const setRunning = () => {};',
  'state.js': 'export const state = {};',
})) await writeFile(join(temp, 'app', name), body);
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

// 运行页与首页都接上了：运行按钮在选了服务器时交给 remote.js；首页的服务器卡片可点。
const runner = await readFile(join(root, 'dist', 'app', 'runner.js'), 'utf8');
assert.match(runner, /if \(runTarget\(\) !== 'local'\) \{\s*await remoteRun\(/);
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['run-target', 'manage-servers', 'remote-runs', 'remote-dialog', 'remote-host', 'remote-root', 'remote-maps', 'remote-test', 'remote-save']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
console.log('remote: maps, probe summary, job states and wiring ok');
