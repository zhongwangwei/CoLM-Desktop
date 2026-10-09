// 开发工作区面板（workspace.js）的纯函数：状态灯文字、可采纳的内核、采纳过的内核在匹配表里的样子，
// 以及 kernel.js 只在采纳之后才让实验内核参与匹配。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-workspace-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = async () => ({}); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = () => null; export const status = () => {}; export const baseName = p => String(p).split("/").pop(); export const appConfirm = async () => true; export const appPrompt = async () => null;',
  'state.js': 'export const state = { kernels: [] };',
})) await writeFile(join(temp, 'app', name), body);
const workspace = await import(pathToFileURL(join(temp, 'app', 'workspace.js')).href);
const kernel = await import(pathToFileURL(join(temp, 'app', 'kernel.js')).href);
const { state } = await import(pathToFileURL(join(temp, 'app', 'state.js')).href);

// 状态灯
assert.equal(workspace.lightText('pass'), '通过');
assert.equal(workspace.lightText('stale'), '需要重测（之后又改过代码）');
assert.equal(workspace.lightText('whatever'), '还没测');
assert.equal(
  workspace.lightsSummary({ compile: 'pass', tests: 'stale', regression: 'unknown', parity: 'fail' }),
  '编译 通过 · 测试 需要重测（之后又改过代码） · 两版一致 不通过 · 与原版对比 还没测',
);

// 只显示这个工作区的实验内核。
const registered = [
  { workspace: 'emis', preset: 'default', head: 'h1', dir: '/ws/emis/kernels/default', macros: ['SinglePoint', 'LULC_IGBP'], label: '实验内核：emis（未审阅）· default' },
  { workspace: 'other', preset: 'default', head: 'h2', dir: '/ws/other/kernels/default', macros: ['SinglePoint', 'LULC_IGBP'], label: 'x' },
];
assert.deepEqual(workspace.adoptableKernels(registered, 'emis').map(k => k.dir), ['/ws/emis/kernels/default']);
assert.deepEqual(workspace.adoptableKernels(registered, 'nobody'), []);
assert.deepEqual(workspace.adoptableKernels(undefined, 'emis'), []);

// 匹配表里的条目：和 list_kernels 同形，另带 experimental 标记，且标明未审阅。
const entry = workspace.kernelEntry(registered[0]);
assert.equal(entry.preset, 'default');
assert.deepEqual(entry.macros, ['SinglePoint', 'LULC_IGBP']);
assert.deepEqual(entry.experimental, { workspace: 'emis', label: registered[0].label, head: 'h1', reviewed: false });

// 工作区又有了新提交（head 变了）：采纳过的内核不再算数。
assert.equal(workspace.stillRegistered([entry], registered).length, 1);
assert.equal(workspace.stillRegistered([entry], [{ ...registered[0], head: 'h9' }]).length, 0);
assert.equal(workspace.stillRegistered([entry], []).length, 0);
assert.deepEqual(workspace.stillRegistered(undefined, registered), []);
assert.equal(workspace.commitLine({ short: 'abc1234', subject: 'ws: emg 0.95' }), 'abc1234 · emg 0.95');
// 每项检查都有一句白话说明。
assert.deepEqual(Object.keys(workspace.LIGHT_HELP).sort(), ['compile', 'parity', 'regression', 'tests']);
// 正在使用的内核：只看这个工作区的。
assert.deepEqual(workspace.adoptedFrom([entry], 'emis').map(k => k.preset), ['default']);
assert.deepEqual(workspace.adoptedFrom([entry], 'other'), []);
assert.deepEqual(workspace.adoptedFrom(undefined, 'emis'), []);

// 内核匹配：没采纳过时只看正式内核（行为不变）；采纳后同预设的实验内核排在前面。
const official = { preset: 'default', dir: '/app/kernels/default', macros: ['SinglePoint', 'LULC_IGBP'] };
state.kernels = [official, { preset: 'usgs', dir: '/app/kernels/usgs', macros: ['SinglePoint', 'LULC_USGS'] }];
state.wizard = { grid: 'site' };
state.adoptedKernels = [];
assert.equal(kernel.kernelForSubgrid('IGBP', { grid: 'site' }).dir, '/app/kernels/default');
state.adoptedKernels = [entry];
assert.equal(kernel.kernelForSubgrid('IGBP', { grid: 'site' }).dir, '/ws/emis/kernels/default', 'the adopted experimental kernel wins');
assert.equal(kernel.kernelForSubgrid('USGS', { grid: 'site' }).dir, '/app/kernels/usgs', 'other presets are untouched');
state.adoptedKernels = undefined;
assert.equal(kernel.kernelForSubgrid('IGBP', { grid: 'site' }).dir, '/app/kernels/default');

// 接线：按钮和对话框都在页面里，助手面板头有入口；D 级操作没有出现在助手的工具里。
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['workspace-dialog', 'workspace-list', 'workspace-refresh', 'workspace-close', 'assistant-workspaces-btn']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
const main = await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8');
assert.match(main, /import '\.\/workspace\.js';/);
const assistant = await readFile(join(root, 'dist', 'app', 'assistant.js'), 'utf8');
for (const forbidden of ['workspace_adopt', 'workspace_delete', 'workspace_export', 'workspace_revert']) {
  assert.ok(!assistant.includes(forbidden), `${forbidden} must not be reachable from the assistant panel code`);
}
console.log('workspace: lights, adoption, kernel matching and wiring ok');
