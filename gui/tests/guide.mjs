import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const temp = await mkdtemp(join(tmpdir(), 'colm-guide-'));
await cp(new URL('../dist/app/', import.meta.url), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}');
for (const [name, code] of Object.entries({
  'ipc.js': 'export const invoke = (...args) => globalThis.testInvoke(...args);',
  'params.js': 'export const renderFields = () => globalThis.testRenderFields();',
  'results.js': 'export const refreshVars = () => {}; export const markResultsStale = () => {}; export const resultsGuide={fields:()=>globalThis.taskFields??[], actions:()=>globalThis.taskActions??{},state:()=>({source:"built-in",phase:"ready"})};',
  'hybrid.js': 'export const hybridGuide={fields:()=>globalThis.taskFields??[],actions:()=>globalThis.taskActions??{},state:()=>({source:"built-in",mode:"learn"})};',
  'homepage.js': 'export const homeGuide={fields:()=>[],actions:()=>globalThis.homeActions??{},state:()=>({busy:false}),openCase:dir=>globalThis.testOpen(dir,"basic-files",{strict:true})}; export const showHome=()=>{globalThis.document.getElementById("launchgate").hidden=false;};',
  'shell.js': `import { state } from './state.js'; import { $ } from './ui.js';
    export const renderSteps = () => {}; export const setStatus = s => { $('status').textContent = String(s); };
    export const go = step => { state.step = step; };
    export const STEPS = [{ id:'basic-files', t:'files', need:()=>null }, {id:'basic-site',t:'site',need:()=>globalThis.nextBlocked??null}, { id:'run', t:'run', need:()=>null }];
    export const nextOf = id => STEPS[STEPS.findIndex(s=>s.id===id)+1]??null;`,
  'kernel.js': 'export const urbanEnabled = () => false;',
  'domain.js': 'export const wizardFields = () => []; export const wizardFieldNames = () => new Set();',
  'spatial.js': 'export const createSpatialCase = (...args) => globalThis.testSpatial(...args);',
  'opencase.js': 'export const openExistingCase = (...args) => globalThis.testOpen(...args);',
  'runner.js': 'export const runRequested = (...args) => globalThis.testRun(...args); export const cancelRequested = () => globalThis.testCancel();',
})) await writeFile(join(temp, 'app', name), code);

class El {
  constructor(id = '') { Object.assign(this, { id, tagName:'INPUT', value:'', _text:'', disabled:false, hidden:false, children:[], style:{}, dataset:{}, type:'text', classList:{add(){}} }); }
  get textContent() { return this._text; }
  set textContent(value) { this._text = String(value); this.children = []; }
  addEventListener() {}
  dispatchEvent() {}
  getClientRects() { return [1]; }
  closest() { return this.hidden ? this : null; }
  getAttribute(name) { return this[name] ?? null; }
  setAttribute() {}
  replaceChildren(...children) { this.children = children; }
  appendChild(child) { this.children.push(child); return child; }
  append(...children) { this.children.push(...children); }
  querySelector(selector) {
    const tags = selector.toUpperCase().split(',').map(s => s.trim());
    return this.children.find(c => tags.includes(c.tagName))
      ?? this.children.map(c => c.querySelector?.(selector)).find(Boolean) ?? null;
  }
  querySelectorAll() { return []; }
  click() { if (!this.disabled) this.onclick?.(); }
}
const elements = new Map();
const el = id => { if (!elements.has(id)) elements.set(id, new El(id)); return elements.get(id); };
globalThis.window = globalThis;
globalThis.addEventListener = () => {};
globalThis.dispatchEvent = () => {};
globalThis.CustomEvent = class { constructor(type, options) { Object.assign(this, {type}, options); } };
let parameterRows = [], runModeCards = [];
globalThis.document = {
  getElementById:el, createElement:tag => { const e = new El(); e.tagName = tag.toUpperCase(); return e; },
  createTextNode:text => ({textContent:text}), querySelectorAll:selector => selector.includes('data-parameter-key') ? parameterRows : selector === '#run-mode-choices .run-mode-card' ? runModeCards : [],
  querySelector: selector => selector === '#makecase .btn-next' ? el('makecase').querySelector('button') : null,
};
el('loadinggate').hidden = true; el('launchgate').hidden = true; el('domaingate').hidden = true;
let listed = [], mode = 'failure', calls = [], rendered = false, scanFailureAfterCreate = false, renderFailure = false, siteScanFailure = false;
globalThis.testRenderFields = async () => { if (renderFailure) throw new Error('configuration load failed'); await Promise.resolve(); rendered = true; };
globalThis.testInvoke = async (command, args) => {
  calls.push([command, args]);
  if (command === 'list_cases') {
    if (scanFailureAfterCreate && calls.some(([cmd]) => cmd === 'new_case')) throw new Error('case result scan failed');
    return [...listed];
  }
  if (command === 'new_case') {
    if (mode === 'failure') throw new Error('rawdata directory does not exist: /missing/rawdata');
    listed.push({name:args.name,dir:args.out}); return 'created';
  }
  if (command === 'read_text') return '&nl_colm /';
  if (command === 'unknown_fields') return [];
  if (command === 'install_example') return { sitedir:'/examples/Sitedata', forcingdir:'/examples/Forcing', root:'/default', runtimedir:'/examples/Runtime', already:true };
  if (command === 'scan_sites') { if (siteScanFailure) throw new Error('site scan failed'); return { sites:[site] }; }
  throw new Error(`unexpected command ${command}`);
};
const url = name => pathToFileURL(join(temp, 'app', name)).href;
const { state } = await import(url('state.js'));
const sites = await import(url('sites.js'));
const site = {name:'CN-Cng',site_file:'/examples/Sitedata/CN-Cng_site.nc',met_file:'/examples/Forcing/CN-Cng_Met.nc',urban:false,crop:false};
state.wizard = {physics:{}}; state.subgrid = 'IGBP'; state.step = 'basic-files'; state.sites = [site]; state.pickedSite = site;
el('root').value = '/project/example-runs/nospin';

try {
  assert.deepEqual(await sites.ensureCases([site]), []);
  assert.match(el('status').textContent, /rawdata directory does not exist: \/missing\/rawdata/, 'the original creation error must survive batch summary');
  await assert.rejects(sites.confirmSelection(), /rawdata directory does not exist/);
  el('rawdata').value = '/missing/rawdata'; el('runtime').value = '/missing/runtime';
  await sites.loadExample();
  assert.equal(el('rawdata').value, '', 'bundled inputs must not inherit stale rawdata');
  assert.equal(el('runtime').value, '', 'natural example must not inherit stale runtime');
  assert.equal(el('root').value, '/project/example-runs/nospin', 'loading examples must preserve an explicit output root');
  assert.equal(state.pickedSite.name, 'CN-Cng', 'example loading waits for scanning');
  state.wizard.physics.bgc = true;
  await sites.loadExample();
  assert.equal(el('runtime').value, '/examples/Runtime');
  state.wizard.physics.bgc = false;
  mode = 'success'; calls = [];
  sites.renderMakeCase();
  const guide = await import(url('guide.js'));
  const request = async () => {
    // The request listener is exercised directly, without model/provider traffic.
    return await new Promise((resolve, reject) => globalThis.guideHandler({detail:{action:'commit',args:{target:'create-case'},respond:(ok,result)=>ok?resolve(result):reject(new Error(result))}}));
  };
  // Import again to capture the registered production request handler.
  globalThis.addEventListener = (name, handler) => { if (name === 'colm:ui-request') globalThis.guideHandler = handler; };
  await import(url('guide.js')+'?listener');
  const drive = (action, args) => new Promise((resolve,reject) => globalThis.guideHandler({detail:{action,args,respond:(ok,r)=>ok?resolve(r):reject(new Error(r))}}));
  const result = await request();
  assert.equal(result.created, '/project/example-runs/nospin/CN-Cng');
  assert.equal(result.step, 'basic-files', 'creation must preserve sequential page review');
  assert.ok(guide.uiState().buttons.some(b=>b.name==='next-step'));
  await drive('click',{target:'next-step'});
  assert.equal(state.step, 'basic-site');
  assert.ok(rendered, 'the case configuration must finish loading before success');
  assert.equal(calls.filter(([cmd]) => cmd === 'new_case').length, 1);
  state.step = 'basic-files';
  sites.renderMakeCase();
  const reused = await request();
  assert.equal(reused.created, result.created, 'already-selected cases must not wait for a directory change');
  assert.equal(calls.filter(([cmd]) => cmd === 'new_case').length, 1, 'reusing a case must not rewrite it');
  assert.equal(guide.uiState().step, 'basic-files');
  globalThis.nextBlocked = 'next page blocked';
  await assert.rejects(drive('click',{target:'next-step'}), /next page blocked/);
  globalThis.nextBlocked = null;
  state.step = 'basic-files';
  el('root').value = '/project/different-root';
  sites.renderMakeCase();
  const moved = await request();
  assert.ok(moved.created.startsWith('/project/different-root/'), 'changing output root must not reuse a case in the old root');
  assert.notEqual(moved.created, result.created);
  assert.equal(calls.filter(([cmd]) => cmd === 'new_case').length, 2);
  state.step = 'basic-files';
  el('root').value = '/project/failed-scan';
  sites.renderMakeCase(); calls = []; scanFailureAfterCreate = true;
  await assert.rejects(request(), /case result scan failed/, 'partial results must not be reported as complete');
  scanFailureAfterCreate = false;
  state.step = 'basic-files';
  el('root').value = '/project/failed-load';
  sites.renderMakeCase(); renderFailure = true;
  await assert.rejects(request(), /configuration load failed/, 'creation success must wait for loading its configuration');
  siteScanFailure = true;
  await assert.rejects(sites.loadExample(), /site scan failed/);
  assert.equal(state.pickedSite, null, 'failed example scans cannot leave a stale selected site');
  assert.deepEqual(state.sites, []);
  assert.equal(el('use-example').disabled, false);
  // A late example installation or site scan cannot overwrite a newer workflow.
  const originalInvoke = globalThis.testInvoke;
  let releaseInstall;
  globalThis.testInvoke = (command,args) => command === 'install_example'
    ? new Promise(resolve => { releaseInstall = resolve; }) : originalInvoke(command,args);
  const previousDir = el('sitedir').value;
  const pendingInstall = sites.loadExample();
  state.wizard = {physics:{}};
  releaseInstall({sitedir:'/late/sites',forcingdir:'/late/forcing',root:'/late/root'});
  await assert.rejects(pendingInstall, /工作流已变化/);
  assert.equal(el('sitedir').value, previousDir);
  let releaseScan;
  globalThis.testInvoke = (command,args) => command === 'scan_sites'
    ? new Promise(resolve => { releaseScan = resolve; }) : originalInvoke(command,args);
  const currentSites = state.sites = [site];
  const scanning = sites.scanPreparedSites(null, {strict:true});
  state.wizard = {physics:{urban:true}};
  releaseScan({sites:[{...site,name:'late'}]});
  await assert.rejects(scanning, /工作流已变化/);
  assert.equal(state.sites, currentSites);
  globalThis.testInvoke = originalInvoke;
  // Exercise the real parameter loader's catches, rather than only its caller's stub.
  await cp(new URL('../dist/app/params.js', import.meta.url), join(temp, 'app', 'params-real.js'));
  await writeFile(join(temp, 'app', 'histvars.js'), 'export const renderHistVars = () => {};');
  await writeFile(join(temp, 'app', 'timing.js'), 'export const renderTiming = () => {};');
  const { renderFields } = await import(url('params-real.js'));
  state.text = '&nl_colm /'; state.fields = []; state.batch = [state.selected.dir];
  el('kernel').value = '/kernel';
  for (const failure of ['read_text', 'read_case', 'field_states_batch']) {
    globalThis.testInvoke = async command => {
      if (command === failure) throw new Error(`${failure} failed`);
      if (command === 'read_text') return '&nl_colm /';
      if (['read_case', 'land_cover_contexts', 'varying_fields'].includes(command)) return [];
      throw new Error(`unexpected command ${command}`);
    };
    await assert.rejects(renderFields(() => true, {strict:true}), new RegExp(`${failure} failed`));
    await renderFields(); // Manual page rendering retains its visible-error behavior.
  }
  state.expert = true;
  for (const failure of ['land_cover_contexts', 'varying_fields', 'process_parameter_files', 'site_pfts']) {
    globalThis.testInvoke = async command => {
      if (command === failure) throw new Error(`${failure} failed`);
      if (command === 'read_text') return '&nl_colm /';
      if (['read_case', 'field_states_batch', 'land_cover_contexts', 'varying_fields', 'process_parameter_files', 'site_pfts'].includes(command)) return [];
      throw new Error(`unexpected command ${command}`);
    };
    await assert.rejects(renderFields(() => true, {strict:true}), new RegExp(`${failure} failed`));
  }
  state.expert = false;
  let finishRead;
  globalThis.testInvoke = () => new Promise(resolve => {finishRead=resolve;});
  const superseded = renderFields(() => true, {strict:true});
  await Promise.resolve();
  state.text = ''; state.selected = null;
  await renderFields();
  finishRead('&nl_colm /');
  await assert.rejects(superseded, /superseded/);
  state.selected = {dir:'/case'}; state.batch = ['/case']; state.text = '&nl_colm /';
  const entry = {path:'DEF_TEST', value:'1', known:false, group:'nl_colm'};
  state.fields = [{name:entry.path, kind:'Real', section:'输出与重启', group:'nl_colm', default:'1'}];
  globalThis.testInvoke = async command => {
    if (command === 'read_text') return '&nl_colm /';
    if (command === 'read_case') return [entry];
    if (command === 'field_states_batch') return [{name:entry.path, mode:'editable'}];
    if (['land_cover_contexts', 'varying_fields'].includes(command)) return [];
    if (command === 'set_field_batch') throw new Error('write denied');
    throw new Error(`unexpected command ${command}`);
  };
  await renderFields();
  const table = el('output-fields').children[0];
  parameterRows = table.children.filter(child => child.dataset?.parameterKey);
  assert.equal(parameterRows.length, 1);
  await assert.rejects(globalThis.guideHandler && new Promise((resolve, reject) => {
    globalThis.guideHandler({detail:{action:'set', args:{fields:[{field:'DEF_TEST',value:'2'}]}, respond:(ok,r)=>ok?resolve(r):reject(new Error(r))}});
  }), /write denied/, 'real save callback failures must reach ui_set');
  let finishSave;
  const input = parameterRows[0].querySelector('input');
  input.onchange = async () => { el('status').textContent = 'unrelated status update'; await new Promise(resolve => {finishSave=resolve;}); };
  let saved = false;
  const pending = new Promise((resolve, reject) => globalThis.guideHandler({detail:{action:'set',args:{fields:[{field:'DEF_TEST',value:'3'}]},respond:(ok,r)=>ok?resolve(r):reject(new Error(r))}})).then(() => {saved=true;});
  await Promise.resolve();
  assert.equal(saved, false, 'status changes must not signal save completion');
  finishSave(); await pending;
  input.readOnly = true;
  await assert.rejects(new Promise((resolve,reject) => globalThis.guideHandler({detail:{action:'set',args:{fields:[{field:'DEF_TEST',value:'4'}]},respond:(ok,r)=>ok?resolve(r):reject(new Error(r))}})), /cannot be changed/);
  parameterRows = [];
  await cp(new URL('../dist/app/histvars.js', import.meta.url), join(temp, 'app', 'histvars-real.js'));
  const { renderHistVars } = await import(url('histvars-real.js'));
  globalThis.testInvoke = async () => { throw new Error('history read failed'); };
  await assert.rejects(renderHistVars(el('hist-fields'), () => true, {strict:true}), /history read failed/);
  await renderHistVars(el('hist-fields'));
  await cp(new URL('../dist/app/timing.js', import.meta.url), join(temp, 'app', 'timing-real.js'));
  const { renderTiming } = await import(url('timing-real.js'));
  globalThis.testInvoke = async () => { throw new Error('read_timing failed'); };
  await assert.rejects(renderTiming(() => true, {strict:true}), /read_timing failed/);
  await renderTiming();
  globalThis.testInvoke = async command => {
    if (command === 'read_timing') return {spinup_years:0,spinup_repeat:0};
    if (command === 'set_spinup') throw new Error('spin-up write denied');
    throw new Error(`unexpected command ${command}`);
  };
  await renderTiming();
  state.step = 'basic-timing'; el('tm-years').value = '1'; el('tm-repeat').value = '10';
  const applySpinup = () => new Promise((resolve,reject) => globalThis.guideHandler({detail:{action:'commit',args:{target:'apply-spinup'},respond:(ok,r)=>ok?resolve(r):reject(new Error(r))}}));
  await assert.rejects(applySpinup(), /spin-up write denied/);
  el('tm-years').value = '-1';
  await assert.rejects(applySpinup(), /非负整数/);
  let completeSpinup;
  globalThis.testInvoke = command => command === 'set_spinup'
    ? new Promise(resolve => {completeSpinup=resolve;})
    : Promise.resolve({spinup_years:1,spinup_repeat:10});
  el('tm-years').value = '1';
  const spinup = applySpinup(); await Promise.resolve();
  completeSpinup({written:1,text:'updated'});
  assert.equal((await spinup).written, 1);
  state.step = 'basic-files';
  globalThis.testSpatial = async () => { throw new Error('spatial creation denied'); };
  await assert.rejects(drive('commit',{target:'make-spatial-case'}), /spatial creation denied/);
  globalThis.testOpen = async () => { throw new Error('open denied'); };
  await assert.rejects(drive('click',{target:'open-case:/selected'}), /open denied/);
  state.step = 'run';
  for (const target of ['run-all','run-colm','run-mksrfdata','run-mkinidata','cancel-run']) {
    globalThis.testRun = globalThis.testCancel = async () => { throw new Error('operation rejected'); };
    await assert.rejects(drive('commit',{target}), /operation rejected/);
  }
  let completeRun;
  globalThis.testRun = async stage => {
    assert.equal(stage,'colm');
    await new Promise(resolve => {completeRun=resolve;});
    return {phase:'started',started:['/case'],queued:[]};
  };
  const run = drive('commit',{target:'run-colm'}); await Promise.resolve();
  completeRun(); assert.equal((await run).phase,'started');
  for (const step of ['result-series','result-tuning','result-uncertainty','hybrid-learn','hybrid-process']) {
    await drive('go',{step});
    globalThis.taskFields = [el('task-draft')];
    globalThis.taskActions = {
      preview: {kind:'click',element:el('task-preview'),run:async()=>{}},
      execute: {kind:'commit',element:el('task-execute'),run:async()=>{throw new Error('built-in rejected');}},
    };
    const page = guide.uiState();
    assert.equal(page.step,step);
    assert.equal(page.task.source,'built-in');
    assert.ok(page.fields.some(f=>f.name==='task-draft' && !f.saves));
    await drive('fill',{fields:[{field:'task-draft',value:'42'}]});
    assert.equal(el('task-draft').value,'42');
    await assert.rejects(drive('click',{target:'execute'}),/use ui_commit/);
    await assert.rejects(drive('commit',{target:'preview'}),/use ui_click/);
    await assert.rejects(drive('commit',{target:'execute'}),/built-in rejected/);
    el('task-execute').disabled = true;
    await assert.rejects(drive('commit',{target:'execute'}),/disabled/);
    el('task-execute').disabled = false;
    let finishAction, completed = false;
    globalThis.taskActions.execute.run = async () => {await new Promise(resolve=>{finishAction=resolve;});return {phase:'completed'};};
    const executing = drive('commit',{target:'execute'}).then(result=>{completed=true;return result;});
    await Promise.resolve(); assert.equal(completed,false);
    finishAction(); assert.equal((await executing).phase,'completed');
    el('task-draft').hidden = true;
    await assert.rejects(drive('fill',{fields:[{field:'task-draft',value:'43'}]}),/not a draft field/);
    el('task-draft').hidden = false;
  }
  let finishDraft;
  el('task-draft').onchange = async () => { await new Promise(resolve=>{finishDraft=resolve;}); };
  let filled = false;
  const drafting = drive('fill',{fields:[{field:'task-draft',value:'44'}]}).then(()=>{filled=true;});
  await Promise.resolve(); await Promise.resolve();
  assert.equal(filled,false,'draft changes must await dependent option loading');
  finishDraft(); await drafting;
  el('task-draft').onchange = async () => {throw new Error('draft option loading failed');};
  await assert.rejects(drive('fill',{fields:[{field:'task-draft',value:'45'}]}),/draft option loading failed/);
  state.step = 'run-mode';
  const contextFields = ['context-select-version', 'context-select-target', 'context-select-engine'];
  const canonicalFields = ['model-version', 'run-target', 'model-engine'];
  const parallelFields = ['cpu-workers', 'mpi-ranks', 'case-threads'];
  runModeCards = canonicalFields.map((field, index) => el(`run-mode-choices-choice-${['version','target','engine'][index]}-0`));
  for (const [index, card] of runModeCards.entries()) {
    const source = el(canonicalFields[index]); source.hidden = true;
    card.tagName = 'BUTTON'; card.textContent = `Choice ${index}`;
    card['aria-pressed'] = 'true';
    let calls = 0, finishChange;
    card.onclick = async () => { calls++; source.value = 'chosen'; await new Promise(resolve => { finishChange = resolve; }); };
    assert.ok(guide.uiState().buttons.some(button => button.name === card.id && button.kind === 'click'));
    assert.equal(guide.uiState().buttons.find(button => button.name === card.id).selected, true);
    let complete = false;
    const changing = drive('click', {target:card.id}).then(() => { complete = true; });
    await Promise.resolve(); assert.equal(complete, false, 'card action waits for its manual handler');
    finishChange(); await changing;
    assert.equal(calls, 1); assert.equal(source.value, 'chosen');
    card.disabled = true;
    await assert.rejects(drive('click', {target:card.id}), /disabled/); card.disabled = false;
    card.hidden = true;
    assert.ok(!guide.uiState().buttons.some(button => button.name === card.id));
    await assert.rejects(drive('click', {target:card.id}), /not a button/); card.hidden = false;
    await assert.rejects(drive('commit', {target:card.id}), /does not save/);
    assert.equal(calls, 1, 'rejected requests never select a card');
    source.hidden = false;
    await assert.rejects(drive('fill', {fields:[{field:source.id,value:'other'}]}), /not a draft field/);
    source.hidden = true;
    await assert.rejects(drive('fill', {fields:[{field:contextFields[index],value:'other'}]}), /not a draft field/);
  }
  const modeFields = guide.uiState().fields.map(f=>f.name);
  for (const id of parallelFields) assert.ok(modeFields.includes(id));
  for (const id of contextFields) assert.ok(!modeFields.includes(id));
  assert.ok(!modeFields.includes('force'));
  await drive('fill',{fields:parallelFields.map(field=>({field,value:'2'}))});
  await assert.rejects(drive('commit',{target:'run-all'}),/not.*page|not available/);
  state.step = 'run';
  const runFields = guide.uiState().fields.map(f=>f.name);
  assert.ok(runFields.includes('force'));
  assert.ok(!guide.uiState().buttons.some(button => runModeCards.some(card => card.id === button.name)));
  await assert.rejects(drive('click', {target:runModeCards[0].id}), /not a button/);
  for (const id of parallelFields) {
    assert.ok(!runFields.includes(id));
    await assert.rejects(drive('fill',{fields:[{field:id,value:'3'}]}),/not a draft field/);
  }
  let runCalls = 0;
  globalThis.testRun = async () => {runCalls++;};
  await assert.rejects(drive('click',{target:'run-all'}),/ui_commit/);
  await assert.rejects(drive('fill',{fields:[{field:'run-all',value:'true'}]}),/not a draft field/);
  assert.equal(runCalls,0,'draft and click operations must not bypass run approval');
  for (const gate of ['loadinggate','launchgate','domaingate']) {
    el(gate).hidden = false;
    assert.notEqual(guide.uiState().screen,'workbench');
    for (const field of contextFields) {
      await assert.rejects(drive('fill',{fields:[{field,value:'first'}]}),/not a draft field/);
    }
    el(gate).hidden = true;
  }
  // Home actions expose current buttons, respect approval, and cannot operate the covered workbench.
  el('launchgate').hidden = false; state.step = 'run';
  let homeClicks = 0, finishExample;
  globalThis.homeActions = {
    'new-case': {element:el('home-new-case'),kind:'click',run:async()=>{homeClicks++;}},
    'example:CN-Cng': {element:el('home-example-CN-Cng'),kind:'commit',run:()=>new Promise(resolve=>{finishExample=resolve;})},
  };
  const homepage = guide.uiState();
  assert.equal(homepage.screen, 'launch');
  assert.deepEqual(homepage.buttons.map(button=>button.name), ['new-case','example:CN-Cng']);
  const prompt = await readFile(new URL('../../crates/colm-agent/src/prompt.md', import.meta.url), 'utf8');
  assert.match(prompt, /For a new case: new-case →/);
  assert.match(prompt, /ui_commit\("example:<site name>"\)/);
  assert.ok(!prompt.includes('local-run →'), 'assistant prompt must use the current homepage actions');
  await assert.rejects(drive('click',{target:'example:CN-Cng'}), /ui_commit/);
  await assert.rejects(drive('commit',{target:'run-all'}), /not a commit button/);
  await assert.rejects(drive('click',{target:'next-step'}), /not a button/);
  await assert.rejects(drive('click',{target:'scan'}), /not a button/);
  state.step = 'basic-files';
  await assert.rejects(drive('click',{target:'site:CN-Cng'}), /not a button/);
  await drive('click',{target:'new-case'}); assert.equal(homeClicks,1);
  el('home-new-case').disabled = true;
  await assert.rejects(drive('click',{target:'new-case'}), /disabled/);
  let homeCompleted = false;
  const example = drive('commit',{target:'example:CN-Cng'}).then(result=>{homeCompleted=true;return result;});
  await Promise.resolve(); assert.equal(homeCompleted,false);
  finishExample({started:false}); assert.equal((await example).started,false);
  el('launchgate').hidden = true;
  console.log('guide: detailed failure, example isolation, awaited creation, reuse and incomplete-result checks passed');
} finally { await rm(temp, {recursive:true, force:true}); }
