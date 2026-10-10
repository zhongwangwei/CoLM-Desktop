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
  'ipc.js': 'export const invoke = (...args) => globalThis.remoteInvoke(...args); export const listen = async () => {}; export const hasBackend = true;',
  'ui.js': 'export const $ = id => globalThis.remoteNodes?.[id] ?? null; export const status = () => {}; export const baseName = p => String(p).split("/").pop();',
  'sites.js': 'export const renderCases = () => {};',
  'results.js': 'export const invalidateResultCase = () => {};',
  'shell.js': 'export const setRunning = () => {};',
  'state.js': 'export const state = {};',
  'engine.js': 'export const modelEngine = () => "rust";',
})) await writeFile(join(temp, 'app', name), body);
// Expose internal actions only in the temporary test copy.
await writeFile(join(temp, 'app', 'remote.js'), (await readFile(join(temp, 'app', 'remote.js'), 'utf8')) + '\nexport { resumeJobs, refreshJob, fetchJob, renderJobs, jobs, formServer, fillDialog, closeDialog, testConnection, saveServer, syncAuth, clearPassword, wire };\n');
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
    if (name === 'remote_job_record') return { host: 'server', job: 'old-job' };
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
const identityCalls = [];
globalThis.remoteInvoke = async name => {
  identityCalls.push(name);
  if (name === 'remote_job_record') return { host: 'server', job: 'old-job' };
  return { record: { host: 'server', job: 'new-job' }, status: { state: 'finished', exit_code: 0 } };
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

remote.jobs.clear();

// Authentication fields keep secrets out of serialized server profiles.
function field(value = '') {
  return { value, hidden: false, options: [], children: [], handlers: {},
    addEventListener(name, fn) { this.handlers[name] = fn; },
    replaceChildren(...items) { this.children = items; this.options = items; },
    appendChild(item) { this.children.push(item); this.options = this.children; },
    prepend(item) { this.children.unshift(item); }, close() {}, showModal() {},
  };
}
globalThis.Option = function(text, value) { return { text, value }; };
globalThis.addEventListener = () => {};
const authIds = ['run-target', 'manage-servers', 'serverRunCard', 'preview-job', 'remote-preview-close',
  'remote-which', 'remote-host', 'remote-username', 'remote-port', 'remote-auth', 'remote-identity',
  'remote-password', 'remote-password-note', 'remote-key-row', 'remote-password-row', 'remote-pick-key',
  'remote-root', 'remote-maps', 'remote-threads', 'remote-scheduler', 'remote-partition', 'remote-account',
  'remote-walltime', 'remote-cpus', 'remote-memory', 'remote-nodes', 'remote-fetch-vars', 'remote-env',
  'remote-directives', 'remote-delete', 'remote-probe-result', 'remote-dialog', 'remote-test',
  'remote-list-kernels', 'remote-build-kernel', 'remote-save', 'remote-close', 'remote-queue-list'];
globalThis.remoteNodes = Object.fromEntries(authIds.map(id => [id, field()]));
const fields = globalThis.remoteNodes;
let configured = { servers: [{ host: 'legacy', root: '/data/jobs' }] };
const authCalls = [];
globalThis.remoteInvoke = async (name, args) => {
  authCalls.push({ name, args });
  if (name === 'remote_config') return configured;
  if (name === 'remote_save_config') configured = args.config;
  if (name === 'remote_probe') return { hostname: 'server', os: 'Linux', arch: 'x86_64' };
};
remote.wire();
await new Promise(resolve => setImmediate(resolve));
remote.fillDialog('legacy');
assert.equal(fields['remote-auth'].value, 'config');
assert.equal(fields['remote-username'].value, '');
assert.equal(fields['remote-port'].value, '');
assert.equal(fields['remote-key-row'].hidden, true);
assert.equal(fields['remote-password-row'].hidden, true);
fields['remote-auth'].value = 'key';
fields['remote-auth'].handlers.change();
assert.equal(fields['remote-key-row'].hidden, false);
fields['remote-identity'].value = '/private/key';
assert.equal(remote.formServer().identity_file, '/private/key');
fields['remote-auth'].value = 'password';
fields['remote-auth'].handlers.change();
fields['remote-username'].value = 'alice';
fields['remote-port'].value = '2222';
fields['remote-password'].value = 'example-test-secret';
assert.equal(fields['remote-password-row'].hidden, false);
assert.equal(fields['remote-key-row'].hidden, true);
assert.equal(remote.formServer().identity_file, '');
assert.equal(JSON.stringify(remote.formServer()).includes('example-test-secret'), false);
authCalls.length = 0;
await remote.testConnection();
assert.deepEqual(authCalls.map(c => c.name), ['remote_set_password', 'remote_probe']);
assert.deepEqual(authCalls[0].args, { host: 'legacy', username: 'alice', port: 2222, password: 'example-test-secret' });
assert.equal(authCalls[1].args.server.auth, 'password');
assert.equal(JSON.stringify(authCalls[1]).includes('example-test-secret'), false);
assert.equal(fields['remote-password'].value, '');
authCalls.length = 0;
await remote.testConnection();
assert.deepEqual(authCalls.map(c => c.name), ['remote_probe'], 'empty password preserves the backend session credential');
for (const id of ['remote-host', 'remote-username', 'remote-port']) {
  fields['remote-password'].value = 'draft';
  fields[id].handlers.input();
  assert.equal(fields['remote-password'].value, '', `${id} changes must clear the secret draft`);
}
fields['remote-password'].value = 'draft';
fields['remote-auth'].handlers.change();
assert.equal(fields['remote-password'].value, '');
fields['remote-password'].value = 'draft';
remote.closeDialog(null);
assert.equal(fields['remote-password'].value, '');
fields['remote-port'].value = '65536';
assert.throws(() => remote.formServer(), /端口/);
fields['remote-port'].value = '22.5';
assert.throws(() => remote.formServer(), /端口/);
fields['remote-port'].value = '2222';
fields['remote-password'].value = 'save-secret';
authCalls.length = 0;
await remote.saveServer();
assert.deepEqual(authCalls.map(c => c.name), ['remote_set_password', 'remote_save_config', 'remote_config', 'remote_job_record']);
assert.equal(JSON.stringify(configured).includes('save-secret'), false);
assert.equal(configured.servers[0].username, 'alice');
assert.equal(fields['remote-password'].value, '');

// Switching profiles while a probe is in flight must not show the old server's result.
let finishProbe;
let probeCount = 0;
globalThis.remoteInvoke = async name => {
  if (name === 'remote_probe') { probeCount++; return new Promise(resolve => { finishProbe = resolve; }); }
};
const pendingProbe = remote.testConnection();
await new Promise(resolve => setImmediate(resolve));
await remote.testConnection();
await remote.saveServer();
assert.equal(probeCount, 1, 'repeated actions cannot race the pending credential/probe operation');
remote.fillDialog(null);
fields['remote-probe-result'].textContent = 'new profile';
finishProbe({ hostname: 'stale-server', os: 'Linux', arch: 'x86_64' });
await pendingProbe;
assert.equal(fields['remote-probe-result'].textContent, 'new profile');
console.log('remote: authentication modes, scoped session secrets, legacy profiles and stale probes');

let finishPicker;
globalThis.remoteInvoke = async (name, args) => {
  assert.equal(name, 'pick_file');
  assert.deepEqual(args, { key: 'remote-identity', filter: '' });
  return new Promise(resolve => { finishPicker = resolve; });
};
const pendingPicker = fields['remote-pick-key'].onclick();
remote.closeDialog(null);
finishPicker('/private/old-key');
await pendingPicker;
assert.equal(fields['remote-identity'].value, '');
console.log('remote: duplicate actions blocked and stale key picker ignored');

// A local submission record remains visible when a restart has lost the session password.
remote.jobs.clear();
fields['remote-runs'] = field();
fields['remote-run-list'] = field();
state.selected = { dir: '/case' };
state.cases = [{ dir: '/case', has_history: true }];
let sessionPassword = false;
const restartCalls = [];
globalThis.remoteInvoke = async (name, args) => {
  restartCalls.push(name);
  if (name === 'remote_job_record') return { host: 'legacy', job: 'existing-job' };
  if (name === 'remote_status') {
    if (!sessionPassword) throw new Error('Re-enter your server password');
    return { record: { host: 'legacy', job: 'existing-job' }, status: { state: 'finished', exit_code: 0 } };
  }
  if (name === 'remote_set_password') sessionPassword = true;
  if (name === 'remote_config') return configured;
  if (name === 'remote_save_config') configured = args.config;
};
await remote.resumeJobs();
assert.equal(remote.jobs.get('/case').host, 'legacy');
assert.match(remote.jobs.get('/case').error, /Re-enter/);
assert.equal(fields['remote-runs'].hidden, false);
const reconnectButton = nodes.find(node => node.tag === 'button' && node.textContent === '重新连接服务器…');
assert.ok(reconnectButton, 'a failed resume must offer a visible reconnect action');
remote.fillDialog('legacy');
fields['remote-password'].value = 'reentered-test-secret';
await remote.saveServer();
assert.equal(remote.jobs.get('/case').error, null);
assert.equal(remote.jobs.get('/case').state, 'finished');
assert.equal(restartCalls.filter(name => name === 'remote_status').length, 2);
assert.equal(restartCalls.includes('remote_fetch'), false, 'reconnection preserves newer local history');
remote.jobs.clear();
const absentCalls = [];
globalThis.remoteInvoke = async name => { absentCalls.push(name); return null; };
await remote.resumeJobs();
assert.equal(remote.jobs.size, 0);
assert.deepEqual(absentCalls, ['remote_job_record']);
assert.equal(fields['remote-runs'].hidden, true);
console.log('remote: restarted password jobs remain visible, reconnect retries, absent records are quiet');
