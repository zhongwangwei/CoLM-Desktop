// 改代码页（workspace.js）的纯函数：状态灯文字、版本卡、旧版“启用实验内核”迁移成默认版本，
// 以及 kernel.js 只在正式内核里匹配（修改版由 `colm-cli run --model-version` 自己取内核）。
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
  'ui.js': 'export const $ = id => globalThis.workspaceElements?.[id] ?? null; export const status = () => {}; export const baseName = p => String(p).split("/").pop(); export const appConfirm = async (...args) => globalThis.workspaceConfirm ? globalThis.workspaceConfirm(...args) : true; export const appPrompt = async () => null;',
  'state.js': 'export const state = { kernels: [] };',
  'remote.js': 'export const configuredServers = () => globalThis.workspaceServers ?? [];',
  'shell.js': 'export const go = step => { globalThis.workspaceStep = step; };',
})) await writeFile(join(temp, 'app', name), body);
await writeFile(join(temp, 'app', 'workspace.js'),
  (await readFile(join(temp, 'app', 'workspace.js'), 'utf8')) + '\nexport { createWorkspace, remotePanel, showDetail, render };\n');
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

assert.equal(workspace.commitLine({ short: 'abc1234', subject: 'ws: emg 0.95' }), 'abc1234 · emg 0.95');
// 每项检查都有一句白话说明。
assert.deepEqual(Object.keys(workspace.LIGHT_HELP).sort(), ['compile', 'parity', 'regression', 'tests']);

// 版本卡：从哪来、改了几次、改了哪边。
assert.equal(
  workspace.versionFacts({ origin: '/repo/CoLM-Desktop', commits: 2, touches: { engine: true, kernel: false } }),
  '基于 CoLM-Desktop · 改了 2 次 · 改了 Rust 引擎',
);
assert.equal(
  workspace.versionFacts({ origin: '/x/src', based_on: 'beta', commits: 0, touches: { engine: true, kernel: true }, dirty: true }),
  '基于修改版 beta · 改了 0 次 · 改了 Rust 引擎和 Fortran 内核 · 有未保存的改动',
);
assert.match(workspace.versionFacts({ origin: 'app', commits: 1 }), /没改会影响计算结果的代码/);
assert.deepEqual(workspace.LOCAL_CHECKS.map(([id]) => id), ['build-engine', 'build-kernel', 'test-drift', 'test-cargo', 'parity']);

// 旧版“启用实验内核”：只在还没选过版本时迁移成默认的模型版本。
const legacy = [{ preset: 'default', experimental: { workspace: 'emis', head: 'h1' } }];
assert.equal(workspace.migratedVersion(legacy, null), 'emis');
assert.equal(workspace.migratedVersion(legacy, 'official'), null);
assert.equal(workspace.migratedVersion([{ preset: 'default' }], null), null);
assert.equal(workspace.migratedVersion(undefined, null), null);

// 内核匹配只看正式内核；旧的 adoptedKernels 不再参与。
const official = { preset: 'default', dir: '/app/kernels/default', macros: ['SinglePoint', 'LULC_IGBP'] };
state.kernels = [official, { preset: 'usgs', dir: '/app/kernels/usgs', macros: ['SinglePoint', 'LULC_USGS'] }];
state.wizard = { grid: 'site' };
state.adoptedKernels = [{ preset: 'default', dir: '/ws/emis/kernels/default', macros: ['SinglePoint', 'LULC_IGBP'], experimental: { workspace: 'emis' } }];
assert.equal(kernel.kernelForSubgrid('IGBP', { grid: 'site' }).dir, '/app/kernels/default');
assert.equal(kernel.kernelForSubgrid('USGS', { grid: 'site' }).dir, '/app/kernels/usgs');

// 接线：改代码是一张页面（不再是助手里的对话框）；D 级操作没有出现在助手的工具里。
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
assert.ok(html.includes('data-step="code"'));
for (const id of ['workspace-list', 'workspace-refresh', 'research-workspaces-btn', 'workspace-create-form', 'workspace-name',
  'workspace-note', 'workspace-base', 'workspace-source', 'workspace-rev', 'workspace-create', 'workspace-create-status']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
for (const gone of ['workspace-dialog', 'assistant-workspaces-btn']) assert.ok(!html.includes(`id="${gone}"`), gone);
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
assert.deepEqual(calls[0], ['workspace_create', { name: 'experiment', source: null, rev: null, base: null, note: null }]);
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
assert.deepEqual(calls[1], ['workspace_create', { name: 'experiment', source: '/path with spaces/repo', rev: 'feature/soil', base: null, note: null }]);
assert.deepEqual(calls.slice(2).map(call => call[0]), ['workspace_list']);
assert.match(elements['workspace-create-status'].textContent, /已创建修改版 experiment.*\/workspaces\/experiment/);
// 从另一个修改版开始、写了目的：来源与提交号不再传（以父版本为准）。
elements['workspace-base'] = { value: 'beta', disabled: false };
elements['workspace-note'] = { value: '  lower snow albedo ', disabled: false };
elements['workspace-name'].value = 'snow';
await workspace.createWorkspace({ preventDefault() {} });
assert.deepEqual(calls.at(-2), ['workspace_create', { name: 'snow', source: null, rev: null, base: 'beta', note: 'lower snow albedo' }]);
delete elements['workspace-base'];
delete elements['workspace-note'];
assert.equal(elements['workspace-create'].disabled, false);
console.log('workspace: lights, version cards, legacy migration, kernel matching and wiring ok');

// Remote workflows retain failures/stale evidence and never adopt a remote binary.
assert.deepEqual(workspace.remoteRequest('verify', ' default ', '/data/case one', 'colm-core'), {
  action: 'verify', preset: 'default', kind: 'refactor', package: 'colm-core', case: '/data/case one',
});
assert.throws(() => workspace.remoteRequest('verify', 'default', 'relative', 'colm-core'));
assert.throws(() => workspace.remoteRequest('test', 'default', '', '--bad package'));
assert.match(workspace.remoteJobText({ id: 'j1', host: 'server', state: 'failed', commit: 'abcdef012', stale: true }), /失败.*abcdef01.*需要重测/);
class Element {
  constructor(tag) { this.tag = tag; this.children = []; this.listeners = {}; this.attributes = {}; this.value = ''; this.disabled = false; this.isConnected = false; this.hidden = false; }
  append(...items) { this.children.push(...items); for (const item of items) if (item) item.parentElement = this; if (this.tag === 'select' && !this.value) this.value = items[0]?.value ?? ''; }
  replaceChildren(...items) { this.children = items; }
  setAttribute(name, value) { this.attributes[name] = value; }
  querySelector(selector) {
    const match = item => selector.startsWith('.') ? item.className?.split(' ').includes(selector.slice(1))
      : selector.startsWith('#') ? item.id === selector.slice(1) : item.tag === selector;
    return this.children.find(match) ?? this.children.map(item => item.querySelector?.(selector)).find(Boolean) ?? null;
  }
  querySelectorAll(selector) { return this.children.flatMap(item => [...(item.tag === selector ? [item] : []), ...(item.querySelectorAll?.(selector) ?? [])]); }
  after(item) { const at = this.parentElement.children.indexOf(this); this.parentElement.children.splice(at+1,0,item); item.parentElement = this.parentElement; }
  remove() { this.parentElement.children = this.parentElement.children.filter(item=>item!==this); }
  focus() { document.activeElement = this; }
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

// Four-tab detail uses visible UI actions and confirms per-file mutations.
const page = new Element('section');
const list = new Element('div'); list.id = 'workspace-list';
const createForm = new Element('form'); createForm.id = 'workspace-create-form';
const officialCard = new Element('div'); officialCard.className = 'ws-official';
page.append(officialCard,list,createForm);
globalThis.workspaceElements = new Proxy({}, {get:(_,id)=>page.querySelector(`#${String(id)}`)});
globalThis.document.createTextNode = textContent=>({textContent});
globalThis.workspaceServers = [{host:'7920ts',root:'/data/colm'}];
const detailCalls = [];
let confirm = false;
globalThis.workspaceConfirm = async () => confirm;
const detail = {
  workspace:{name:'experiment',note:'snow test',based_on:'parent'}, dir:'/ws/experiment',
  lights:{compile:'pass',tests:'stale',parity:'not_needed'},
  commits:[{short:'abc123',subject:'snow'}],
  changed_files:[{path:'vendor/CoLM202X/Snow.F90',added:6,removed:2,touches:{kernel:true}},
    {path:'crates/colm-core/src/snow.rs',added:3,removed:1,touches:{engine:true}},
    {path:'docs/note.md',added:1,removed:0,touches:{}}],
  studies:[{dir:'/study/one',id:'study-1',head:'123456789',location:'7920ts',pinned_unix:1700000000,current:false}],
};
globalThis.workspaceInvoke = async (command,args) => {
  detailCalls.push([command,args]);
  if (command === 'workspace_status') return args.name === 'parent' ? {studies:[]} : detail;
  if (command === 'workspace_diff') return {diff:'<script>literal diff</script>',truncated:true};
  if (command === 'workspace_list') return {workspaces:[{name:'experiment',commits:1}]};
  if (command === 'workspace_revert_file') return {commit:'new'};
  throw new Error(`unexpected ${command}`);
};
workspace.render([{name:'experiment',commits:1}]);
const findButton = (root, label)=>root.querySelectorAll('button').find(item=>item.textContent===label);
await findButton(list,'experiment').onclick();
assert.equal(list.hidden,true); assert.equal(createForm.hidden,true);
let detailBox = page.querySelector('#workspace-detail');
const allText = node => [node.textContent ?? '',...node.children?.map(allText) ?? []].join(' ');
assert.match(allText(detailBox),/snow test.*parent/);
const tabs = detailBox.querySelector('.ws-detail-tabs').children;
assert.deepEqual(tabs.map(tab=>tab.textContent),['修改','检查','对比','用到它的研究']);
assert.equal(tabs[0].attributes['aria-selected'],'true');
assert.equal(detailBox.querySelector('#workspace-panel-modify').hidden,false);
assert.equal(detailBox.querySelector('#workspace-panel-checks').hidden,true);
tabs[0].onkeydown({key:'ArrowRight',preventDefault(){}});
assert.equal(tabs[1].attributes['aria-selected'],'true');
assert.equal(document.activeElement,tabs[1]);
assert.equal(detailBox.querySelector('#workspace-panel-checks').hidden,false);
assert.ok(detailCalls.every(([command])=>!['remote_config','remote_probe','workspace_remote'].includes(command)),'detail rendering never starts SSH');
assert.match(allText(detailBox),/相对官方版的改动.*Fortran 内核.*Rust 引擎.*其他改动/);
assert.match(allText(detailBox),/study-1.*7920ts.*之后又改过/);
await Promise.resolve(); await Promise.resolve();
assert.match(allText(detailBox),/父版本的研究.*还没有研究用过这个版本/);
await findButton(detailBox,'看差异').onclick();
assert.deepEqual(detailCalls.at(-1),['workspace_diff',{name:'experiment',path:'vendor/CoLM202X/Snow.F90'}]);
assert.match(allText(detailBox),/<script>literal diff<\/script>.*已截断/s);
await findButton(detailBox,'撤销这处').onclick();
assert.ok(!detailCalls.some(([command])=>command==='workspace_revert_file'),'cancel confirmation must not mutate');
confirm = true;
await findButton(detailBox,'撤销这处').onclick();
assert.equal(detailCalls.filter(([command])=>command==='workspace_revert_file').length,1);
assert.deepEqual(detailCalls.find(([command])=>command==='workspace_revert_file')[1],{name:'experiment',path:'vendor/CoLM202X/Snow.F90',message:'Restore file to official baseline'});
detailBox = page.querySelector('#workspace-detail');
await findButton(detailBox,'用我的编辑器打开').onclick();
assert.ok(detailBox.querySelector('#workspace-panel-modify').children.some(item=>item.textContent==='/ws/experiment/src'&&!item.hidden));
await findButton(detailBox,'去运行方式').onclick(); assert.equal(globalThis.workspaceStep,'run-mode');
await findButton(detailBox,'比较两个版本').onclick(); assert.equal(globalThis.workspaceStep,'result-tuning');
assert.ok(detailBox.querySelector('details').querySelectorAll('button').some(item=>item.textContent==='删除这个修改版…'));
await findButton(detailBox,'← 改代码').onclick();
assert.equal(list.hidden,false); assert.equal(createForm.hidden,false); assert.equal(page.querySelector('#workspace-detail'),null);
console.log('workspace: four tabs, cached check locations, file diff, confirmed undo and study snapshots passed');

// Choosing a configured check location uses that exact host and keeps other hosts' jobs out.
const selectedRemoteCalls = [];
globalThis.workspaceInvoke = async (command,args) => {
  selectedRemoteCalls.push([command,args]);
  if (command !== 'workspace_remote') throw new Error(`unexpected ${command}`);
  if (args.operation === 'list') return {jobs:[{id:'other',host:'other-server',state:'finished'}]};
  if (args.operation === 'submit') return {job:{id:'new',host:args.host,state:'queued'}};
  throw new Error(`unexpected ${args.operation}`);
};
const selectedPanel = workspace.remotePanel('experiment','7920ts');
selectedPanel.open = true;
await selectedPanel.listeners.toggle();
const selectedForm = selectedPanel.children[1];
assert.equal(selectedForm.children[0].children[0].value,'7920ts');
assert.equal(selectedForm.children[0].children[0].disabled,true);
assert.ok(!allText(selectedPanel).includes('other-server'));
selectedForm.children[2].children[0].value = '/data/case';
await selectedForm.onsubmit({preventDefault(){}});
assert.equal(selectedRemoteCalls.find(([,args])=>args.operation==='submit')[1].host,'7920ts');
assert.ok(selectedRemoteCalls.every(([command])=>command==='workspace_remote'),'configured location avoids remote_config and probes');

// Detail-read errors keep a working way back to the list.
globalThis.workspaceInvoke = async () => {throw new Error('workspace removed');};
await workspace.showDetail('missing');
assert.match(allText(page.querySelector('#workspace-detail')),/workspace removed/);
await findButton(page.querySelector('#workspace-detail'),'← 改代码').onclick();
assert.equal(list.hidden,false);
console.log('workspace: selected remote host, scoped jobs and detail failure recovery passed');
