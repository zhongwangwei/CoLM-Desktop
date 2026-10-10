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
  'ipc.js': 'export const invoke = async (...args) => globalThis.workspaceInvoke ? globalThis.workspaceInvoke(...args) : ({}); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = id => globalThis.workspaceElements?.[id] ?? null; export const status = () => {}; export const baseName = p => String(p).split("/").pop(); export const appConfirm = async () => true; export const appPrompt = async () => null;',
  'state.js': 'export const state = { kernels: [] };',
})) await writeFile(join(temp, 'app', name), body);
await writeFile(join(temp, 'app', 'workspace.js'),
  (await readFile(join(temp, 'app', 'workspace.js'), 'utf8')) + '\nexport { createWorkspace, remotePanel, showDetail };\n');
const workspace = await import(pathToFileURL(join(temp, 'app', 'workspace.js')).href);
const kernel = await import(pathToFileURL(join(temp, 'app', 'kernel.js')).href);
const { state } = await import(pathToFileURL(join(temp, 'app', 'state.js')).href);

// 状态灯
assert.equal(workspace.lightText('pass'), '通过');
assert.equal(workspace.lightText('stale'), '需要重测（之后又改过代码）');
assert.equal(workspace.lightText('whatever'), '还没测');
assert.equal(workspace.lightText('not_needed'), '不需要（没改会影响计算结果的代码）');
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
for (const id of ['workspace-dialog', 'workspace-list', 'workspace-refresh', 'workspace-close', 'assistant-workspaces-btn',
  'research-workspaces-btn', 'workspace-create-form', 'workspace-name', 'workspace-source', 'workspace-rev', 'workspace-create', 'workspace-create-status']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
assert.match(html, /id="workspace-name"[^>]*required[^>]*maxlength="40"/);
assert.match(html, /从仓库创建时只复制已提交的代码，未提交的修改不会带入/);
assert.match(html, /id="workspace-create-status"[^>]*aria-live="polite"/);
const main = await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8');
assert.match(main, /import '\.\/workspace\.js';/);
const assistant = await readFile(join(root, 'dist', 'app', 'assistant.js'), 'utf8');
for (const forbidden of ['workspace_adopt', 'workspace_delete', 'workspace_export', 'workspace_revert']) {
  assert.ok(!assistant.includes(forbidden), `${forbidden} must not be reachable from the assistant panel code`);
}

// Direct creation: invalid input does not call IPC; one pending submit, then retry after failure.
const elements = Object.fromEntries(['workspace-create-form', 'workspace-name', 'workspace-source', 'workspace-rev',
  'workspace-source-browse', 'workspace-create', 'workspace-create-status', 'workspace-list']
  .map(id => [id, { value: '', disabled: false, textContent: '', className: '', replaceChildren() {}, append() {} }]));
globalThis.workspaceElements = elements;
globalThis.document = { documentElement: { lang: 'zh' }, createElement: () => ({}) };
elements['workspace-name'].value = 'experiment';
elements['workspace-create-form'].reportValidity = () => false;
const calls = [];
globalThis.workspaceInvoke = async (...args) => { calls.push(args); return {}; };
await workspace.createWorkspace({ preventDefault() {} });
assert.equal(calls.length, 0);
elements['workspace-create-form'].reportValidity = () => true;
let rejectCreation;
globalThis.workspaceInvoke = (...args) => {
  calls.push(args);
  return new Promise((_, reject) => { rejectCreation = reject; });
};
const pending = workspace.createWorkspace({ preventDefault() {} });
assert.deepEqual(calls[0], ['workspace_create', { name: 'experiment', source: null, rev: null }]);
assert.equal(elements['workspace-create'].disabled, true);
await workspace.createWorkspace({ preventDefault() {} });
assert.equal(calls.length, 1);
rejectCreation(new Error('source unavailable'));
await pending;
assert.equal(elements['workspace-create'].disabled, false);
assert.equal(elements['workspace-name'].value, 'experiment');
assert.equal(elements['workspace-create-status'].textContent, 'source unavailable');
elements['workspace-source'].value = '/path with spaces/repo';
elements['workspace-rev'].value = ' feature/soil ';
globalThis.workspaceInvoke = async (...args) => {
  calls.push(args);
  return args[0] === 'workspace_create' ? { dir: '/workspaces/experiment' } : {};
};
await workspace.createWorkspace({ preventDefault() {} });
assert.deepEqual(calls[1], ['workspace_create', { name: 'experiment', source: '/path with spaces/repo', rev: 'feature/soil' }]);
assert.deepEqual(calls.slice(2).map(call => call[0]).sort(), ['workspace_kernels', 'workspace_list']);
assert.match(elements['workspace-create-status'].textContent, /已创建工作区 experiment.*\/workspaces\/experiment/);
assert.equal(elements['workspace-create'].disabled, false);
console.log('workspace: lights, adoption, kernel matching and wiring ok');

// Remote workflows retain failures/stale evidence and never adopt a remote binary.
assert.deepEqual(workspace.remoteRequest('verify', ' default ', '/data/case one', 'colm-core'), {
  action: 'verify', preset: 'default', kind: 'refactor', package: 'colm-core', case: '/data/case one',
});
assert.throws(() => workspace.remoteRequest('verify', 'default', 'relative', 'colm-core'));
assert.throws(() => workspace.remoteRequest('test', 'default', '', '--bad package'));
assert.match(workspace.remoteJobText({ id: 'j1', host: 'server', state: 'failed', commit: 'abcdef012', stale: true }), /失败.*abcdef01.*需要重测/);
class Element {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.value = ''; this.disabled = false; this.isConnected = false; }
  append(...items) { this.children.push(...items); if (this.tag === 'select' && !this.value) this.value = items[0]?.value ?? ''; }
  replaceChildren(...items) { this.children = items; }
  setAttribute() {}
  addEventListener(event, fn) { this.listeners[event] = fn; }
  reportValidity() { return true; }
}
globalThis.document.createElement = tag => new Element(tag);
const remoteCalls = [];
let remoteJobs = [{ id: 'job-1', host: 'srv', state: 'running', commit: '123456789', source_sha256: 'hash', stale: true }];
globalThis.workspaceInvoke = async (command, request) => {
  remoteCalls.push([command, request]);
  if (command === 'remote_config') return { servers: [{ host: 'srv' }] };
  if (request.operation === 'list') return { jobs: remoteJobs };
  if (request.operation === 'status') return { job: { ...remoteJobs[0], state: 'failed' }, status: { state: 'finished', exit_code: 1, phase: 'test', log_tail: 'test failed' } };
  if (request.operation === 'fetch') return { job: { ...remoteJobs[0], state: 'failed', report_dir: '/reports/job-1' }, status: { state: 'finished', exit_code: 1 } };
  if (request.operation === 'submit') throw new Error('SSH password required');
};
const panel = workspace.remotePanel('experiment');
panel.open = true;
await panel.listeners.toggle();
const [summary, form, message, refreshButton, jobs] = panel.children;
assert.match(summary.textContent, /服务器/);
assert.match(jobs.children[0].children[0].textContent, /失败.*需要重测/);
assert.equal(jobs.children[0].children.find(c => c.tag === 'pre').textContent, 'test failed');
assert.equal(remoteCalls[1][1].operation, 'list', 'reopening retrieves persisted jobs');
await jobs.children[0].children.find(c => c.tag === 'button').onclick();
assert.match(message.textContent, /已取回.*失败/, 'fetching failed evidence is never marked passing');
form.children[2].children[0].value = '/data/case';
await form.onsubmit({ preventDefault() {} });
assert.equal(message.textContent, 'SSH password required');
assert.equal(form.children.at(-1).disabled, false, 'retry enabled after failed submit');
assert.ok(remoteCalls.every(([command]) => command !== 'workspace_adopt'));
console.log('workspace: remote lifecycle, stale evidence, failure recovery and separate adoption ok');

let pendingSubmission;
globalThis.workspaceInvoke = async (command, request) => {
  remoteCalls.push([command, request]);
  if (request.operation === 'submit') return new Promise(resolve => { pendingSubmission = resolve; });
  if (request.operation === 'list') return { jobs: [] };
};
const submitsBefore = remoteCalls.filter(([, args]) => args?.operation === 'submit').length;
const submitting = form.onsubmit({ preventDefault() {} });
await form.onsubmit({ preventDefault() {} });
assert.equal(remoteCalls.filter(([, args]) => args?.operation === 'submit').length, submitsBefore + 1, 'no duplicate submissions');
pendingSubmission({ job: { id: 'uncertain', host: 'srv', state: 'unknown' }, status: { state: 'unknown' }, error: 'connection interrupted after launch' });
await submitting;
assert.equal(message.textContent, 'connection interrupted after launch', 'ambiguous submission retains actionable failure');
assert.equal(form.children.at(-1).disabled, false);

const card = new Element('div');
card.append(panel);
card.querySelector = selector => {
  assert.equal(selector, '.ws-local-detail', 'local history must not remove remote controls');
  return undefined;
};
globalThis.workspaceInvoke = async () => ({ workspace: { base_commit: 'base' }, commits: [], changed_files: [] });
await workspace.showDetail('experiment', card);
assert.equal(card.children[0], panel, 'remote controls survive local detail opening');
assert.match(card.children[1].className, /ws-local-detail/);

const evidence = { commit: 'candidate', base_commit: 'baseline', stale: true, execution_succeeded: false,
  gates: { candidate: { engine: { ok: true, commit: 'candidate' }, kernels: { default: { ok: true, commit: 'candidate' } },
    tests: { cargo: { ok: false, commit: 'candidate' } } } },
  outcomes: [{ verdict: 'closure degraded', first_difference: { variable: 'f_lfev', record: 4 }, compare: { differs: 1, files: 2 } }],
};
assert.match(workspace.remoteEvidenceText(evidence), /编译 需要重测/);
assert.match(workspace.remoteEvidenceText(evidence), /首个差异.*f_lfev/);
assert.match(workspace.remoteEvidenceText(evidence), /closure degraded/);
assert.match(workspace.remoteEvidenceText(evidence), /对比统计.*differs=1/);
const freshPanel = workspace.remotePanel('experiment');
globalThis.workspaceInvoke = async (command, request) => {
  if (command === 'remote_config') return { servers: [{ host: 'srv' }] };
  if (request.operation === 'fetch') return { job: { ...remoteJobs[0], state: 'failed' }, status: { state: 'finished', exit_code: 1 }, evidence };
  if (request.operation === 'status') return { job: { ...remoteJobs[0], state: 'failed' }, status: { state: 'finished', exit_code: 1 } };
  return { jobs: remoteJobs };
};
freshPanel.open = true;
await freshPanel.listeners.toggle();
await freshPanel.children[4].children[0].children.find(c => c.tag === 'button').onclick();
assert.equal(freshPanel.children[5].hidden, false);
assert.match(freshPanel.children[5].textContent, /远程验证证据.*job-1/);
assert.match(freshPanel.children[5].textContent, /closure degraded/);
await freshPanel.children[3].onclick();
assert.match(freshPanel.children[5].textContent, /closure degraded/, 'refresh keeps fetched evidence separate from job rows');
