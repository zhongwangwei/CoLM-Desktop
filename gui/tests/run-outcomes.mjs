import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const temp = await mkdtemp(join(tmpdir(), 'colm-run-outcomes-'));
await cp(new URL('../dist/app/', import.meta.url), join(temp, 'app'), {recursive:true});
await writeFile(join(temp,'package.json'),' {"type":"module"}');
for (const [name,code] of Object.entries({
  'ipc.js': 'export const invoke=(...a)=>globalThis.invokeFake(...a); export const listen=async(n,f)=>{globalThis.listeners.set(n,f);return ()=>globalThis.listeners.delete(n)};',
  'ui.js': 'export const $=id=>globalThis.nodeFor(id); export const status=()=>{}; export const baseName=p=>p;',
  'sites.js':'export const renderCases=()=>{}; export const renderSites=()=>{}; export const ensureCases=async()=>{throw new Error("creation failed")};',
  'batch.js':'export const batchTarget=()=>globalThis.targets; export const updateCaseBatchButtons=()=>{};',
  'results.js':'export const invalidateResultCase=()=>{}; export const refreshVars=()=>{};',
  'shell.js':'export const setRunning=()=>{}; export const renderSteps=()=>{}; export const setStatus=()=>{};',
  'params.js':'export const renderFields=()=>{};',
  'kernel.js':'export const kernelForSubgrid=()=>null; export const urbanEnabled=()=>false; export const hasTracer=()=>false;',
  'engine.js':'export const modelEngine=()=>"rust";',
  'hybrid.js':'export const fortranBlockedCases=async()=>globalThis.blocked;',
  'i18n.js':'export const language=()=>"zh"; export const translateZh=s=>s;',
  'remote.js':'export const runTarget=()=>"local"; export const remoteRun=()=>{};',
})) await writeFile(join(temp,'app',name),code);
class Node {
  constructor(){Object.assign(this,{value:'1',textContent:'',children:[],dataset:{},style:{},checked:false});}
  addEventListener(){} appendChild(n){this.children.push(n)} querySelector(){return new Node()}
}
const nodes=new Map();
globalThis.nodeFor=id=>{if(!nodes.has(id))nodes.set(id,new Node());return nodes.get(id)};
globalThis.document={createElement:()=>new Node()};
globalThis.addEventListener=()=>{};
globalThis.listeners=new Map();globalThis.targets=[];globalThis.blocked=[];
const {state}=await import(pathToFileURL(join(temp,'app/state.js')));
const {runRequested,cancelRequested}=await import(pathToFileURL(join(temp,'app/runner.js')));
await assert.rejects(runRequested(),/没有可运行/);
globalThis.targets=[{dir:'/a'}]; globalThis.blocked=['/a'];
await assert.rejects(runRequested(),/Fortran/);globalThis.blocked=[];
globalThis.invokeFake=async()=>{throw new Error('missing engine')};
await assert.rejects(runRequested(),/missing engine/);
assert.equal(state.runningCases.size,0);assert.equal(listeners.size,0);
let complete, launchArgs;
globalThis.invokeFake=(cmd,args)=>{launchArgs=args;return new Promise(resolve=>{complete=resolve})};
let settled=false;const pending=runRequested().then(r=>{settled=true;return r});
await new Promise(resolve=>setTimeout(resolve,20));
assert.equal(settled,false,'slow launch must not report success');
listeners.get('run://started')({payload:{run_id:'stale',case:'/a'}});
await Promise.resolve();assert.equal(settled,false);
listeners.get('run://started')({payload:{run_id:launchArgs.runId,case:'/a'}});
const result=await pending;assert.equal(result.phase,'started');assert.deepEqual(result.started,['/a']);assert.equal(listeners.size,0);
await assert.rejects(runRequested(),/正在运行/);
globalThis.invokeFake=async(cmd,args)=>{assert.equal(cmd,'cancel_runs');assert.deepEqual(args.cases,['/a']);return 1};
assert.deepEqual(await cancelRequested(),{requested:1,cases:['/a']});
globalThis.invokeFake=async()=>{throw new Error('cancel rejected')};
await assert.rejects(cancelRequested(),/cancel rejected/);
complete(0);await Promise.resolve();state.runningCases.clear();
await assert.rejects(cancelRequested(),/没有正在运行/);
// A fast command can finish before its started event is delivered.
globalThis.invokeFake=async()=>0;
assert.equal((await runRequested()).phase,'completed');state.runningCases.clear();
globalThis.invokeFake=async()=>2;
await assert.rejects(runRequested(),/运行失败/);
assert.equal(state.runningCases.size,0);
// A batch acknowledgement must distinguish the first spawned process from queued targets.
globalThis.targets=[{dir:'/a'},{dir:'/b'}];
globalThis.invokeFake=(command,args)=>{
  assert.equal(command,'run_batch');
  queueMicrotask(()=>listeners.get('run://started')({payload:{run_id:args.runId,case:'/a'}}));
  return new Promise(()=>{});
};
const batch=await runRequested();
assert.deepEqual(batch.started,['/a']);assert.deepEqual(batch.queued,['/b']);
state.runningCases.clear();globalThis.targets=[{dir:'/a'}];
// Bound a lost start acknowledgement without claiming the still-pending process failed.
const nativeTimeout = globalThis.setTimeout;
let deadline;
globalThis.setTimeout = (callback, ms) => { assert.equal(ms,90000); deadline=callback; return 0; };
globalThis.invokeFake=()=>new Promise(()=>{});
const timeoutRun=runRequested();
await Promise.resolve();await Promise.resolve();await Promise.resolve();
deadline();
await assert.rejects(timeoutRun,/启动尚未确认/);
assert.equal(listeners.size,0);
assert.equal(state.runningCases.has('/a'),true);
globalThis.setTimeout=nativeTimeout;
await assert.rejects(runRequested(),/正在运行/);
await rm(temp,{recursive:true,force:true});
console.log('run outcome checks passed');
