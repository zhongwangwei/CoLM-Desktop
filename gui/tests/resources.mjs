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
  'ipc.js': 'export const invoke = (...args) => globalThis.resourceInvoke(...args); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = () => null; export const status = () => {}; export const baseName = p => p; export const appConfirm = async () => true; export const appPrompt = async () => null;',
  'remote.js': 'export const openServerDialog = async () => null; export const probeLines = () => []; export const kernelLines = () => []; export const runTarget = () => "local";',
})) await writeFile(join(temp, 'app', name), body);
await writeFile(join(temp, 'app', 'resources.js'), (await readFile(join(temp, 'app', 'resources.js'), 'utf8')) + '\nexport { serverCard };\n');
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

// Visiting/rendering a card reads no network; the explicit probe updates shared cache.
const storage = new Map();
globalThis.localStorage = { getItem: key => storage.get(key), setItem: (key, value) => storage.set(key, value) };
const { cachedProbe } = await import(pathToFileURL(join(temp, 'app', 'remote-probe-cache.js')).href);
globalThis.document = { createElement: tag => ({ tag, children: [], options: [], append(...children) { this.children.push(...children); if (tag === 'select') this.options.push(...children); }, replaceChildren(...children) { this.children = children; } }) };
globalThis.Option = function(text, value) { return { text, value }; };
let probes = 0;
globalThis.resourceInvoke = async name => { assert.equal(name, 'remote_probe'); probes++; return { cpus: 8 }; };
const server = { host: 'test', root: '/jobs' };
const card = resources.serverCard(server);
assert.equal(probes, 0);
const probeButton = card.children[1].children.find(button => button.textContent === '检查连接');
await probeButton.onclick();
assert.equal(probes, 1);
assert.equal(cachedProbe(server).probe.cpus, 8);
globalThis.resourceInvoke = async () => { throw new Error('unreachable'); };
await probeButton.onclick();
assert.equal(cachedProbe(server).probe, null);
assert.equal(cachedProbe(server).error, 'probe_failed');
console.log('resources: explicit probe success and failure update cache; rendering never probes');
