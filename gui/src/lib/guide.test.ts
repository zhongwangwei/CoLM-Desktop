import assert from 'node:assert/strict';
import test from 'node:test';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
import {assistantPanelWidth,pageExecutionContext,publicUiResult,waitForContract} from './utils';

test('page navigation waits for the actual lazy page contract',async()=>{
 let frames=0;
 await waitForContract(()=>frames===4,async()=>{frames++;});
 assert.equal(frames,4);
 await assert.rejects(waitForContract(()=>false,async()=>{},0),/did not become ready/);
});

test('actual page kernel is explicit and disappears when leaving its page',()=>{
 assert.deepEqual(pageExecutionContext([{}, {kernelDir:'/kernels/usgs',preprocessors:'fortran'}]), {kernelDir:'/kernels/usgs',preprocessors:'fortran'});
 assert.equal(pageExecutionContext([{kernelDir:null},{}]).kernelDir,null);
 assert.equal(pageExecutionContext([{}]).kernelDir,null);
});

test('results, run history and model pages cannot guess a boot kernel',()=>{
 for(const step of ['results','runs','versions']){const execution=pageExecutionContext([{step} as {kernelDir?:string}]);assert.deepEqual(execution,{kernelDir:null,preprocessors:null});}
});

test('assistant outcomes preserve failure evidence while omitting credentials and bounding logs',()=>{
 const result={record:{host:'server',job:'j1',ssh_auth:{password:'secret'}},status:{state:'finished',exit_code:1},api_key:'secret',logs:'password=secret\nfailed'};
 assert.deepEqual(publicUiResult(result),{record:{host:'server',job:'j1'},status:{state:'finished',exit_code:1},logs:'password=[redacted]\nfailed'});
 assert.equal(result.record.ssh_auth.password,'secret');
 assert.equal(publicUiResult('Authorization: Bearer secret'),'Authorization=[redacted]');
 assert.deepEqual(publicUiResult(JSON.stringify({exit_code:0,authorization:'secret'})),JSON.stringify({exit_code:0}));
 assert.match(String(publicUiResult('x'.repeat(16002))),/truncated 2 characters/);
 const sample=publicUiResult(Array.from({length:1002},(_,i)=>i)) as {items:number[];omitted_items:number};
 assert.equal(sample.items.length,1000);assert.equal(sample.omitted_items,2);
});

test('assistant width keeps both panes readable across drag limits and small windows',()=>{assert.equal(assistantPanelWidth(320,1440),320);assert.equal(assistantPanelWidth(1000,1440),600);assert.equal(assistantPanelWidth(600,960),432);assert.equal(assistantPanelWidth(100,960),280);assert.equal(assistantPanelWidth(NaN,960),320);});

test('the actual guide isolates modal controls from parent pages and restores navigation after closing',async()=>{
 const source=readFileSync(new URL('./guide.ts',import.meta.url),'utf8').replace(/^import .*$/gm,'').replace(/^export /gm,'');
 const js=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
 const guide=new Function('pageExecutionContext','publicUiResult','waitForContract','requestAnimationFrame',`${js};return {pages,guideState,drive,openPage,bind:value=>{app=value;}};`)(pageExecutionContext,publicUiResult,waitForContract,(fn:()=>void)=>fn());
 const shell=Symbol(),parent=Symbol(),modal=Symbol();let navigations=0,backgroundClicks=0;
 guide.pages.set(shell,{title:'Shell',step:'shell',scope:'shell',status:'shell-ready',fields:[],actions:[{id:'nav-project',label:'Project',kind:'click',run:()=>{backgroundClicks++;}}]});
 guide.pages.set(parent,{title:'Case',step:'case',status:'case-ready',kernelDir:'/case-kernel',fields:[{id:'case-tab',label:'Tab',value:'results',type:'choice',set:()=>{backgroundClicks++;}}],actions:[{id:'case-tab-settings',label:'Settings',kind:'click',run:()=>{backgroundClicks++;}}]});
 guide.pages.set(modal,{title:'Evaluation settings',step:'results',modal:true,status:'modal-ready',details:{dialog:true},fields:[{id:'result-observation',label:'Observations',value:'',type:'text',set:()=>{}}],actions:[{id:'result-settings-close',label:'Close',kind:'click',run:()=>guide.pages.delete(modal)}]});
 const state={route:'case',busy:false};guide.bind({state,navigate:(route:string)=>{navigations++;state.route=route;guide.pages.set(parent,{title:'Project',step:route,fields:[],actions:[]});return true;}});
 const snapshot=guide.guideState();assert.equal(snapshot.title,'Evaluation settings');assert.equal(snapshot.kernelDir,null);assert.deepEqual(snapshot.status,['modal-ready']);assert.deepEqual(snapshot.details,[{dialog:true}]);assert.deepEqual(snapshot.fields.map((field:{id:string})=>field.id),['result-observation']);assert.deepEqual(snapshot.buttons.map((button:{name:string})=>button.name),['result-settings-close']);assert.deepEqual(snapshot.steps,[]);
 await assert.rejects(guide.drive('click',{target:'nav-project'}),/unique visible control/);
 await assert.rejects(guide.drive('fill',{fields:[{field:'case-tab',value:'settings'}]}),/unique visible control/);
 await assert.rejects(guide.drive('go',{step:'project'}),/Close the current dialog/);assert.equal(navigations,0);assert.equal(backgroundClicks,0);
 await guide.drive('click',{target:'result-settings-close'});assert.ok(guide.guideState().buttons.some((button:{name:string})=>button.name==='nav-project'));assert.ok(guide.guideState().steps.length);
 await guide.drive('go',{step:'project'});assert.equal(navigations,1);
});
