import {it} from 'node:test';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
import assert from 'node:assert/strict';
import {methodFromStudy,completedCandidates} from './research-home';
it('opens existing studies in the matching built-in method',()=>{
  assert.equal(methodFromStudy({kind:'uncertainty'}),'uncertainty');
  assert.equal(methodFromStudy({kind:'tuning',hybrid:{slot:'soil_stress'}}),'process');
  assert.equal(methodFromStudy({kind:'tuning',hybrid:{slot:'pft'}}),'parameters');
  assert.equal(methodFromStudy({kind:'tuning'}),'tuning');
  assert.equal(methodFromStudy(null),'tuning');
});
it('never invents completed progress from absent or invalid counts',()=>{
  for(const value of [undefined,null,'4',-1,1.5,NaN,Infinity])assert.equal(completedCandidates(value),null);
  assert.equal(completedCandidates(0),0);
  assert.equal(completedCandidates(7),7);
});

// Exercise actual page callbacks so the directory switch and task tokens cannot drift from this regression.
function pageCallback(name:string,context:Record<string,unknown>){
 const path=new URL('./ResearchPage.tsx',import.meta.url);
 const source=ts.createSourceFile(path.pathname,readFileSync(path,'utf8'),ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX);
 let body:string|undefined;
 const visit=(node:ts.Node)=>{if(name==='localStudyPoll'&&ts.isCallExpression(node)&&node.expression.getText(source)==='useEffect'&&node.getText(source).includes('3000'))body=node.arguments[0].getText(source);if(ts.isFunctionDeclaration(node)&&node.name?.text===name)body=node.getText(source);if(ts.isVariableDeclaration(node)&&node.name.getText(source)===name&&node.initializer)body=node.initializer.getText(source);ts.forEachChild(node,visit);};
 visit(source);assert.ok(body);
 const js=ts.transpileModule(`const run=(${body});`,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
 return new Function(...Object.keys(context),`${js};return run;`)(...Object.values(context)) as ()=>Promise<unknown>;
}
it('opening a broken study shows its error after advancing the directory token',async()=>{
 const epoch={current:1},lock={current:false},errors:string[]=[],opened:boolean[]=[];
 const noop=()=>{};
 const task=pageCallback('task',{epoch,lock,setBusy:noop,setError:(error:string)=>errors.push(error)});
 const browse=pageCallback('browseStudy',{task,epoch,invoke:async()=>'/broken',setStudyDir:noop,setEnvelope:noop,setRemoteReply:noop,setResult:noop,setConfirmed:noop,setError:(error:string)=>errors.push(error),readStatus:async()=>{throw new Error('invalid study manifest');},methodFromStudy,setTab:noop,chooseSlot:noop,setWorkbench:(value:boolean)=>opened.push(value)});
 await assert.rejects(browse(),/invalid study manifest/);
 assert.equal(errors.at(-1),'Error: invalid study manifest');assert.equal(lock.current,false);assert.deepEqual(opened,[]);
});
it('late study reads neither display stale errors nor open a stale workbench',async()=>{
 for(const fail of [false,true]){
  const epoch={current:1},errors:string[]=[],opened:boolean[]=[];const noop=()=>{};
  const task=pageCallback('task',{epoch,lock:{current:false},setBusy:noop,setError:(error:string)=>errors.push(error)});
  const browse=pageCallback('browseStudy',{task,epoch,invoke:async()=>'/old',setStudyDir:noop,setEnvelope:noop,setRemoteReply:noop,setResult:noop,setConfirmed:noop,setError:(error:string)=>errors.push(error),readStatus:async()=>{epoch.current++;if(fail)throw new Error('old read failed');return {manifest:{spec:{kind:'uncertainty'}}};},methodFromStudy,setTab:()=>opened.push(true),chooseSlot:noop,setWorkbench:(value:boolean)=>opened.push(value)});
  if(fail)await assert.rejects(browse(),/old read failed/);else await browse();
  assert.deepEqual(errors,['']);assert.deepEqual(opened,[]);
 }
});

it('a picked directory becomes the current study only after a successful current status read',async()=>{
 for(const outcome of ['valid','broken','stale']){
  const epoch={current:1},paths:string[]=[],shown:unknown[]=[];const noop=()=>{};
  const read=pageCallback('readStatus',{useCallback:(fn:unknown)=>fn,studyDir:'/previous',epoch,invokeJson:async()=>{if(outcome==='broken')throw new Error('missing manifest');if(outcome==='stale')epoch.current++;return {manifest:{id:'s-actual'},state:{},results:[]};},remoteStudyReply:(value:unknown)=>value,setStudyDir:(path:string)=>paths.push(path),setEnvelope:(value:unknown)=>shown.push(value),setHost:noop,setMember:noop,setResultPath:noop}) as (path:string,adopt:boolean)=>Promise<unknown>;
  if(outcome==='broken')await assert.rejects(read('/picked',true),/missing manifest/);else await read('/picked',true);
  assert.deepEqual(paths,outcome==='valid'?['/picked']:[],`${outcome} cannot replace the directory prematurely`);
  assert.equal(shown.length,outcome==='valid'?1:0);
 }
});

it('local polling stops during study selection and a scheduled callback rechecks the operation lock',async()=>{
 const scheduled:(()=>Promise<void>)[]=[],lock={current:false};let reads=0;
 const context={studyDir:'/old',busy:false,launching:false,remotePermissions:{exists:false},envelope:{state:{status:'running'}},lock,readStatus:async()=>{reads++;},setError:()=>{},setTimeout:(fn:()=>Promise<void>)=>{scheduled.push(fn);return 1;},clearTimeout:()=>{}};
 pageCallback('localStudyPoll',{...context,busy:true})();assert.equal(scheduled.length,0);
 pageCallback('localStudyPoll',context)();assert.equal(scheduled.length,1);
 lock.current=true;await scheduled[0]();assert.equal(reads,0,'an old timer cannot start a read after browseStudy acquires its lock');
});

it('legacy research provenance does not inherit the currently selected frozen revision',()=>{
 const current={versionRef:{id:'official',revision:3}};
 assert.equal(pageCallback('modelUsed',{envelope:{model_version:{name:'t8-ui-karman'}},state:current}),undefined);
 const pinned={id:'soil',revision:2};
 assert.equal(pageCallback('modelUsed',{envelope:{model_version:{model_revision:{version:pinned}}},state:current}),pinned);
});
