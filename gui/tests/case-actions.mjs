import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const temp = await mkdtemp(join(tmpdir(), 'colm-case-actions-'));
try {
  await cp(new URL('../dist/app/', import.meta.url), join(temp, 'app'), { recursive:true });
  await writeFile(join(temp, 'package.json'), '{"type":"module"}');
  const stubs = {
    'ipc.js': 'export const invoke = (...args) => globalThis.testInvoke(...args);',
    'ui.js': 'export const $ = id => globalThis.el(id); export const joinPath = (a,b) => `${a}/${b}`;',
    'shell.js': `import { state } from './state.js'; export const renderSteps = () => {};
      export const setStatus = value => { globalThis.status = String(value); };
      export const go = step => { if (!globalThis.blockNavigation) state.step = step; };`,
    'kernel.js': 'export const hasTracer = () => false;',
    'domain.js': `import { state } from './state.js'; export const wizardFields = () => [];
      export const startSession = () => { state.wizard = {}; };`,
    'sites.js': `export const renderCases = () => {}; export const selectCase = (...args) => globalThis.testSelect(...args);`,
    'batch.js': `import { state } from './state.js'; export const adoptCaseList = cases => { state.cases = cases; };`,
  };
  for (const [name, code] of Object.entries(stubs)) await writeFile(join(temp,'app',name),code);
  const elements = new Map();
  globalThis.el = id => {
    if (!elements.has(id)) elements.set(id,{value:'',hidden:false,disabled:false,textContent:'',addEventListener(){},dispatchEvent(){},appendChild(){}});
    return elements.get(id);
  };
  globalThis.document = { createElement: () => ({}) };
  globalThis.addEventListener = () => {};
  const { state } = await import(pathToFileURL(join(temp,'app/state.js')));
  const { createSpatialCase, fillSpatialForm } = await import(pathToFileURL(join(temp,'app/spatial.js')));
  const { openExistingCase } = await import(pathToFileURL(join(temp,'app/opencase.js')));
  const entry = {name:'case',dir:'/cases/case'};
  const opened = {root:'/cases',entry,profile:{grid:'site',spatial:false,inputs:{}}};
  let invokeFailure = '', selectionFailure = false, pauseCreate, pauseSelect, list = [entry];
  globalThis.testInvoke = async command => {
    if (command === invokeFailure) throw new Error(`failed ${command}`);
    if (command === 'new_spatial_case') await pauseCreate;
    if (command === 'list_cases') return list;
    if (command === 'open_case') return opened;
    if (command === 'forcing_datasets') return ['JRA3Q'];
  };
  globalThis.testSelect = async (e, options) => {
    assert.equal(options.strict,true);
    state.selected = e;
    await pauseSelect;
    if (selectionFailure) throw new Error('configuration failed');
  };
  Object.assign(state,{wizard:{},domain:'region',grid:'latlon',spatial:{domain:{kind:'region'},grid:{kind:'latlon'}},kernels:[],step:'basic-files'});
  for (const [id,value] of Object.entries({'spatial-root':'/cases','spatial-name':'case','spatial-rawdata':'/raw','spatial-runtime':'/runtime','spatial-forcing-dataset':'JRA3Q','spatial-forcing-dir':'/forcing','spatial-start':'2008-01-01','spatial-end':'2009-01-01','spatial-timestep':'1800'})) el(id).value=value;
  selectionFailure = true;
  await assert.rejects(createSpatialCase({strict:true}),/configuration failed/);
  assert.equal(el('make-spatial-case').disabled,false);
  assert(!globalThis.status.includes('已通过预检'));
  await assert.rejects(openExistingCase(entry.dir,'run',{strict:true}),/configuration failed/);
  assert(!globalThis.status.includes('已打开算例'));
  selectionFailure = false;
  list = [{name:'case',dir:'/other/case'}];
  await assert.rejects(createSpatialCase({strict:true}),/没有找到它/);
  list = [entry];
  let release;
  pauseSelect = new Promise(resolve => { release=resolve; });
  let complete = false;
  const pending = createSpatialCase({strict:true}).then(value => { complete=true; return value; });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(complete,false,'early selected state must not complete the operation');
  await assert.rejects(createSpatialCase({strict:true}),/正在创建/);
  release();
  assert.equal((await pending).dir,entry.dir);
  pauseSelect = undefined;
  pauseCreate = new Promise(resolve => { release=resolve; });
  const stale = createSpatialCase({strict:true});
  state.wizard = {};
  release();
  await assert.rejects(stale,/切换工作流/);
  pauseCreate = undefined;
  el('rawdata').value='/stale'; el('runtime').value='/stale';
  el('launchgate').hidden=false; el('domaingate').hidden=false;
  assert.equal((await openExistingCase(entry.dir,'run',{strict:true})).dir,entry.dir);
  assert.equal(state.step,'run');
  assert.equal(el('launchgate').hidden,true); assert.equal(el('domaingate').hidden,true);
  assert.equal(el('rawdata').value,''); assert.equal(el('runtime').value,'');
  el('spatial-rawdata').value='/stale'; el('spatial-runtime').value='/stale';
  await fillSpatialForm({name:'case'},'/cases');
  assert.equal(el('spatial-rawdata').value,''); assert.equal(el('spatial-runtime').value,'');
  globalThis.blockNavigation = true; state.step='basic-files';
  await assert.rejects(openExistingCase(entry.dir,'run',{strict:true}),/无法进入/);
  globalThis.blockNavigation = false;
  pauseSelect = new Promise(resolve => { release=resolve; });
  const opening = openExistingCase(entry.dir,'run',{strict:true});
  await new Promise(resolve => setImmediate(resolve));
  await assert.rejects(openExistingCase(entry.dir,'run',{strict:true}),/正在打开/);
  state.wizard = {}; release();
  await assert.rejects(opening,/工作流已变化/);
  pauseSelect = undefined;
  invokeFailure = 'open_case';
  await assert.rejects(openExistingCase(entry.dir,'run',{strict:true}),/failed open_case/);
  el('launchgate').hidden=false;
  await openExistingCase(entry.dir);
  assert.match(globalThis.status,/failed open_case/);
  assert.match(el('home-status').textContent,/failed open_case/);
  console.log('case-actions: strict spatial/open completion, errors, stale context and input clearing passed');
} finally {
  await rm(temp,{recursive:true,force:true});
}
