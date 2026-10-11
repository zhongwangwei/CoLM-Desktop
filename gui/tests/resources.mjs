// 计算资源页（resources.js）的纯函数：服务器与本机的摘要、编内核用哪一版的源码，以及页面接线。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-resources-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = (...args) => globalThis.resourceInvoke(...args); export const listen = async () => {}; export const hasBackend = true;',
  'ui.js': 'export const $ = id => globalThis.resourceNodes?.[id] ?? null; export const status = () => {}; export const baseName = p => p; export const appConfirm = text => globalThis.resourceConfirm(text); export const appPrompt = async () => null;',
  'kernel.js': 'export const kernelForSubgrid = () => ({dir: "/kernel"});',
  'remote.js': 'export const openServerDialog = async () => null; export const probeLines = () => []; export const kernelLines = () => []; export const runTarget = () => "local"; export const sessionJobs = () => structuredClone(globalThis.resourceJobs ?? []);',
})) await writeFile(join(temp, 'app', name), body);
await writeFile(join(temp, 'app', 'resources.js'), (await readFile(join(temp, 'app', 'resources.js'), 'utf8')) + '\nexport { serverCard, readResourceRecords };\n');
globalThis.addEventListener = () => {};
const resources = await import(pathToFileURL(join(temp, 'app', 'resources.js')).href);

assert.equal(
  resources.serverFacts({ host: 'c1', root: '/data/colm', scheduler: 'slurm', threads: 16, maps: [{}, {}] }),
  '工作目录 /data/colm · slurm · 16 线程 · 2 条目录对应',
);
assert.equal(resources.serverFacts({ root: '/x', scheduler: 'auto' }), '工作目录 /x · 自动识别调度 · 还没有目录对应');
assert.equal(resources.serverFacts({ root: '/x', scheduler: 'bare', maps: [] }), '工作目录 /x · 不用调度系统 · 还没有目录对应');
assert.equal(resources.localFacts(16, [{ preset: 'default' }, { preset: 'crop' }]), '16 个逻辑 CPU · Rust 引擎随应用安装 · Fortran 内核：default, crop');
assert.match(resources.localFacts(4, []), /没有找到 Fortran 内核/);
// 编内核：选着修改版就用它的源码，官方版不带版本参数。
assert.deepEqual(resources.kernelBuildRequest('c1', 'default', 'snow'), { host: 'c1', preset: 'default', modelVersion: 'snow' });
assert.deepEqual(resources.kernelBuildRequest('c1', 'default', 'official'), { host: 'c1', preset: 'default', modelVersion: null });

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['resource-local', 'resource-servers', 'resource-add-server', 'resource-refresh', 'open-resources']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
assert.ok(html.includes('data-step="resources"'));
const main = await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8');
assert.match(main, /import '\.\/resources\.js';/);
console.log('resources: server and local summaries, kernel build version and wiring ok');

// Rendering and local refresh never probe SSH. Cleanup remains explicit and per snapshot.
const storage = new Map();
globalThis.localStorage = { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) };
const { cachedProbe, recordProbe } = await import(pathToFileURL(join(temp, 'app', 'remote-probe-cache.js')).href);
const { state } = await import(pathToFileURL(join(temp, 'app', 'state.js')).href);
function element(tag) {
  return { tag, children: [], options: [], textContent: '',
    append(...children) { this.children.push(...children); if (tag === 'select') this.options.push(...children); },
    replaceChildren(...children) { this.children = children; }, addEventListener() {},
  };
}
globalThis.document = { createElement: element };
globalThis.Option = function(text, value) { return { text, value }; };
const nodes = item => [item, ...(item?.children ?? []).flatMap(child => nodes(child))];
const text = item => nodes(item).map(child => child?.textContent || '').join('\n');
const findButton = (root, label) => nodes(root).find(child => child.tag === 'button' && child.textContent === label);
const server = { host: 'test', root: '/jobs', auth: 'key', maps: [{local: '/input', remote: '/missing'}] };
const inventory = { engines: [
  {id: '0123456789abcdef', ready: true, versions: ['snow'], can_clean: true, protected_reasons: []},
  {id: 'fedcba9876543210', ready: true, versions: ['used'], can_clean: false, protected_reasons: ['active job']},
], cleanup_blockers: [] };
const calls = [];
globalThis.resourceInvoke = async (name, args) => {
  calls.push({name, args});
  if (name === 'remote_probe') return { cpus: 8, os: 'Linux', schedulers: ['slurm'], maps: [{local: '/input', remote: '/missing', local_exists: true, remote_exists: false}] };
  if (name === 'remote_engines') return inventory;
  if (name === 'remote_engine_clean') return {removed: args.snapshot};
  throw new Error(name);
};
let confirmations = [];
globalThis.resourceConfirm = async prompt => { confirmations.push(prompt); return false; };
const card = resources.serverCard(server);
assert.equal(calls.length, 0);
assert.match(text(card), /尚未测试/);
const probeButton = findButton(card, '检查连接');
await probeButton.onclick();
assert.deepEqual(calls.map(call => call.name), ['remote_probe', 'remote_engines']);
assert.equal(cachedProbe(server).probe.cpus, 8);
assert.equal(cachedProbe(server).probe.maps[0].remote_exists, false);
assert.match(text(card), /密钥登录.*Linux.*8.*slurm/);
assert.match(text(card), /✕ 服务器：\/missing/);
assert.ok(findButton(card, '修改目录对应…'));
assert.match(text(card), /上次测试/);
assert.match(text(card), /上次已连接；缺失目录对应：1/);
assert.match(text(card), /当前使用情况未知/);
let cleanButtons = nodes(card).filter(child => child.tag === 'button' && child.textContent === '清理这个快照…');
assert.equal(cleanButtons[1].disabled, true);
await cleanButtons[1].onclick();
assert.equal(confirmations.length, 0);
await cleanButtons[0].onclick();
assert.equal(calls.some(call => call.name === 'remote_engine_clean'), false, 'cancel never deletes');
assert.match(confirmations[0], /test: \/jobs\/engine\/0123456789abcdef/);
globalThis.resourceConfirm = async prompt => { confirmations.push(prompt); return true; };
await cleanButtons[0].onclick();
assert.deepEqual(calls.find(call => call.name === 'remote_engine_clean').args, {server, snapshot: '0123456789abcdef'});
let deleted = 0;
globalThis.resourceInvoke = async name => {
  if (name === 'remote_engine_clean') { deleted++; return {removed: '0123456789abcdef', warning: 'cleanup lock removal failed'}; }
  throw new Error('inventory after deletion unavailable');
};
await findButton(card, '清理这个快照…').onclick();
assert.equal(deleted, 1);
assert.match(text(card), /引擎快照已清理：0123456789abcdef/);
assert.match(text(card), /cleanup lock removal failed/);
assert.match(text(card), /引擎清单读取失败/);
assert.equal(nodes(card).some(child => child.textContent === '清理这个快照…'), false);

// Interrupted submission records only block cleanup; an hour-old record can be cleared after confirmation.
{
  const stuck = { engines: [], cleanup_blockers: ['Submissions in progress or interrupted: abc (120 min)'], leases: [{ id: 'abc', age_s: 7200 }, { id: 'new', age_s: 30 }] };
  const recoverCalls = [];
  globalThis.resourceInvoke = async (name, args) => {
    recoverCalls.push(name);
    if (name === 'remote_probe') return { cpus: 8, os: 'Linux', schedulers: [], maps: [] };
    if (name === 'remote_engines') return recoverCalls.includes('remote_engine_recover') ? { engines: [], cleanup_blockers: [], leases: [] } : stuck;
    if (name === 'remote_engine_recover') return { removed_leases: ['abc'], kept_leases: ['new'], removed_lock: false };
    throw new Error(name);
  };
  const stuckCard = resources.serverCard({ ...server, host: 'stuck' });
  await findButton(stuckCard, '检查连接').onclick();
  assert.match(text(stuckCard), /中断的提交记录：1/);
  globalThis.resourceConfirm = async () => false;
  await findButton(stuckCard, '解除中断的提交记录…').onclick();
  assert.equal(recoverCalls.includes('remote_engine_recover'), false, 'cancel removes nothing');
  globalThis.resourceConfirm = async prompt => { assert.match(prompt, /一小时以前/); return true; };
  await findButton(stuckCard, '解除中断的提交记录…').onclick();
  assert.ok(recoverCalls.includes('remote_engine_recover'));
  assert.match(text(stuckCard), /已解除中断的提交记录：1/);
  assert.equal(findButton(stuckCard, '解除中断的提交记录…'), undefined);
}

// An inventory read failure must not turn a successful connection into a failed probe.
globalThis.resourceInvoke = async name => { if (name === 'remote_probe') return {cpus: 16}; throw new Error('inventory unavailable'); };
await probeButton.onclick();
assert.equal(cachedProbe(server).error, null);
assert.match(text(card), /引擎清单读取失败/);
globalThis.resourceInvoke = async () => { throw new Error('unreachable'); };
await probeButton.onclick();
assert.equal(cachedProbe(server).probe, null);
assert.equal(cachedProbe(server).error, 'probe_failed');
assert.match(text(card), /连接失败/);
assert.match(text(card), /\? 服务器：\/missing/);
assert.equal(nodes(card).some(child => child.textContent === '清理这个快照…'), false);
// Existing jobs and studies are read from local records; unknown/stale stays explicit.
state.cases = [{dir: '/case'}];
state.selected = {dir: '/case'};
state.kernels = [];
globalThis.resourceJobs = [{dir: '/case', host: 'test', job: 'run-1', state: 'running'}];
const localCalls = [];
globalThis.resourceInvoke = async (name, args) => {
  localCalls.push(name);
  if (name === 'remote_config') return {servers: [server]};
  if (name === 'workspace_list') return {workspaces: [{name: 'snow'}]};
  if (name === 'version_check') return {ready: args.engine === 'rust'};
  if (name === 'remote_job_record') return {host: 'test', job: 'run-1'};
  if (name === 'case_runs') return {studies: [{dir: '/study'}]};
  if (name === 'study_status') return JSON.stringify({state: {status: 'running'}, remote: {host: 'test'}});
  if (name === 'workspace_remote') { assert.equal(args.operation, 'list'); return {jobs: [{host: 'test', id: 'check-1', state: 'queued'}]}; }
  throw new Error(`unexpected ${name}`);
};
globalThis.resourceNodes = {'resource-local': element('div'), 'resource-servers': element('div')};
await resources.refresh();
assert.equal(localCalls.some(name => ['remote_probe', 'remote_engines', 'remote_status', 'remote_study_status'].includes(name)), false);
assert.match(text(resourceNodes['resource-local']), /官方版.*snow（Rust）/);
assert.match(text(resourceNodes['resource-servers']), /研究 \/study/);
assert.match(text(resourceNodes['resource-servers']), /版本检查 snow · check-1/);
assert.match(text(resourceNodes['resource-servers']), /本地记录，可能已过期/);
assert.equal(text(resourceNodes['resource-servers']).match(/算例 \/case/g)?.length, 1, 'session job and local record deduplicated');
recordProbe({...server, root: '/changed'}, {cpus: 32});
assert.equal(cachedProbe(server).error, 'probe_failed', 'edited configuration does not reuse different-root health');
console.log('resources: cached probe/maps, local-only usage and readiness, explicit inventories, confirmation and protected cleanup ok');
