import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
import * as resource from './resource';
import {remoteRunPermissions,reconcileRemoteRun} from '../runs/run-model';
import type {UiAction,UiField} from '../../lib/guide';

type Contract={fields:UiField[];actions:UiAction[];details:unknown};
function harness(records:Record<string,unknown>={},hosts=['7920land','7920mp']){
 const source=readFileSync(new URL('./ResourcesPage.tsx',import.meta.url),'utf8').replace(/^import .*;\n/gm,'').replace('export default function','function');
 const code=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.React}}).outputText;
 const slots:unknown[]=[];let cursor=0,effected=false,contract:Contract,guard:()=>string|null;
 const calls:{command:string;args:Record<string,unknown>|undefined}[]=[];
 const servers=hosts.map(host=>({...resource.newServer(),host}));
 let failProbe=false;
 const ipc=async(command:string,args?:Record<string,unknown>)=>{calls.push({command,args});switch(command){case 'remote_config':return {servers};case 'remote_ssh_hosts':return ['7920land','7920mp'];case 'remote_job_record':return records[String(args?.case)]??null;case 'case_runs':return {studies:[]};case 'remote_probe':if(failProbe)throw new Error('SSH unavailable');return {hostname:'remote',maps:[]};case 'remote_engines':return {engines:[{id:'snapshot',ready:true}]};case 'pick_file':return '/private/id_ed25519';case 'remote_status':return {record:{host:'7920land',job:'job1'},status:{state:'running'}};default:return null;}};
 const environment={...resource,remoteRunPermissions,reconcileRemoteRun,React:{createElement:()=>null},Advanced:()=>{},Button:()=>{},Card:()=>{},Field:()=>{},Input:()=>{},Notice:()=>{},Select:()=>{},NavIcon:()=>{},
  useApp:()=>({state:{language:'zh',cases:Object.keys(records).map(dir=>({dir})),running:{},kernels:[],version:null},update:()=>{}}),
  useState:(initial:unknown)=>{const key=cursor++;if(!(key in slots))slots[key]=initial;return [slots[key],(next:unknown)=>{slots[key]=typeof next==='function'?next(slots[key]):next;}];},
  useRef:(initial:unknown)=>{const key=cursor++;if(!(key in slots))slots[key]={current:initial};return slots[key];},
  useCallback:(fn:unknown)=>fn,useEffect:(fn:()=>void)=>{if(!effected){effected=true;fn();}},useNavigationGuard:(fn:()=>string|null)=>{guard=fn;},usePageContract:(value:Contract)=>{contract=value;},invoke:ipc,invokeJson:ipc};
 const Page=new Function(...Object.keys(environment),`${code};return ResourcesPage;`)(...Object.values(environment));
 function render(){cursor=0;Page();return contract;}
 async function ready(){render();await new Promise(resolve=>setImmediate(resolve));return render();}
 async function action(id:string){const item=contract.actions.find(action=>action.id===id);assert.ok(item,`visible action ${id}`);assert.notEqual(item.enabled,false,`${id} is enabled`);await item.run();return render();}
 function fill(id:string,value:string){const item=contract.fields.find(field=>field.id===id);assert.ok(item,`visible field ${id}`);item.set(value);return render();}
 return {ready,action,fill,render,calls,servers,guard:()=>guard(),contract:()=>contract,failProbe:()=>{failProbe=true;}};
}

test('resource selection is page-local and dirty drafts block switching and navigation until discarded',async()=>{
 const page=harness();let contract=await page.ready();
 assert.equal((contract.details as {selected:string}).selected,'server:7920land');assert.equal(page.guard(),null);
 assert.ok(!contract.actions.some(action=>action.id==='resource-save'||action.id==='resource-discard'),'unchanged settings do not expose hidden save controls');
 assert.deepEqual(contract.actions.filter(action=>action.id.startsWith('resource-select-')).map(action=>action.id),['resource-select-local','resource-select-server-7920land','resource-select-server-7920mp']);
 page.fill('resource-username','changed');assert.match(page.guard()!,/保存/);
 assert.ok(page.contract().actions.some(action=>action.id==='resource-save'));assert.ok(page.contract().actions.some(action=>action.id==='resource-discard'));
 contract=page.contract();assert.equal(contract.actions.find(action=>action.id==='resource-select-server-7920mp')?.enabled,false);assert.equal(contract.actions.find(action=>action.id==='resource-new')?.enabled,false);assert.equal((contract.details as {selected:string}).selected,'server:7920land');
 assert.equal(contract.fields.find(field=>field.id==='resource-username')?.value,'changed');
 await page.action('resource-discard');assert.equal(page.guard(),null);
 contract=await page.action('resource-select-server-7920mp');assert.equal((contract.details as {selected:string}).selected,'server:7920mp');
 assert.equal(page.calls.filter(call=>call.command==='remote_save_config').length,0);
});

test('only visible configuration fields and selected maintenance actions enter the contract',async()=>{
 const page=harness();let contract=await page.ready();
 assert.deepEqual(contract.fields.map(field=>field.id),['resource-host','resource-username','resource-auth']);
 assert.ok(!contract.actions.some(action=>action.id==='resource-build'));
 contract=await page.action('resource-directories-toggle');assert.ok(contract.fields.some(field=>field.id==='resource-root'));
 await page.action('resource-add-map');page.fill('resource-map-0-local','/data');
 contract=await page.action('resource-directories-toggle');assert.ok(!contract.fields.some(field=>field.id.includes('map-')||field.id==='resource-root'));assert.ok(!contract.actions.some(action=>action.id==='resource-add-map'));
 await page.action('resource-discard');await page.action('resource-select-server-7920mp');contract=await page.action('resource-maintenance-toggle');assert.ok(contract.actions.some(action=>action.id==='resource-build'));
 await page.action('resource-kernels');assert.deepEqual(page.calls.at(-1),{command:'remote_kernels',args:{host:'7920mp'}});
 assert.ok(!contract.actions.some(action=>/clean|delete|recover/.test(action.id)));
});

test('checks aggregate both readonly replies, edits revoke evidence, and native key selection edits only the draft',async()=>{
 const page=harness();await page.ready();let contract=await page.action('resource-check');
 let check=(contract.details as {check:ReturnType<typeof resource.resourceCheckSummary>}).check;
 assert.equal(check.ssh.state,'success');assert.equal(check.engine.label,'1 个可用快照');assert.ok(check.checkedAt);
 assert.deepEqual(page.calls.filter(call=>['remote_probe','remote_engines'].includes(call.command)).map(call=>call.command),['remote_probe','remote_engines']);
 page.fill('resource-auth','key');await page.action('resource-pick-identity');contract=page.contract();assert.equal(contract.fields.find(field=>field.id==='resource-identity')?.value,'/private/id_ed25519');
 check=(contract.details as {check:ReturnType<typeof resource.resourceCheckSummary>}).check;assert.equal(check.ssh.state,'pending');
 await page.action('resource-discard');contract=page.contract();assert.equal((contract.details as {check:{ssh:{state:string}}}).check.ssh.state,'pending','discard does not resurrect revoked evidence');
 page.failProbe();contract=await page.action('resource-check');check=(contract.details as {check:ReturnType<typeof resource.resourceCheckSummary>}).check;assert.equal(check.ssh.state,'error');assert.equal(check.engine.state,'success');
 assert.ok(!page.calls.some(call=>call.command==='remote_save_config'));
});

test('remote tasks filter by actual host, preserve orphan records, and expose operations only after expansion',async()=>{
 const page=harness({'/case/land':{host:'7920land',job:'job1'},'/case/mp':{host:'7920mp',job:'job2'},'/case/orphan':{host:'removed',job:'job3'},'/case/missing':{error:'Unreadable record'}});
 let contract=await page.ready();assert.ok(contract.actions.some(action=>action.id==='resource-task-/case/land'));assert.ok(!contract.actions.some(action=>action.id==='resource-task-/case/mp'));assert.ok(contract.actions.some(action=>action.id==='resource-task-/case/orphan'));assert.ok(contract.actions.some(action=>action.id==='resource-task-/case/missing'));
 assert.ok(!contract.actions.some(action=>action.id==='resource-case-status-/case/land'));
 contract=await page.action('resource-task-/case/land');assert.equal(contract.actions.find(action=>action.id==='resource-case-fetch-/case/land')?.kind,'commit');assert.equal(contract.actions.find(action=>action.id==='resource-case-status-/case/land')?.kind,'click');
 await page.action('resource-case-status-/case/land');assert.deepEqual(page.calls.at(-1),{command:'remote_status',args:{case:'/case/land'}});
 contract=await page.action('resource-task-/case/land');assert.ok(!contract.actions.some(action=>action.id==='resource-case-status-/case/land'));
});

function callback(name:string,context:Record<string,unknown>){
 const path=new URL('./ResourcesPage.tsx',import.meta.url),source=ts.createSourceFile(path.pathname,readFileSync(path,'utf8'),ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX);
 let body='';function visit(node:ts.Node){if(ts.isFunctionDeclaration(node)&&node.name?.text===name)body=node.getText(source);ts.forEachChild(node,visit);}visit(source);assert.ok(body);
 const code=ts.transpileModule(`const fn=(${body});`,{compilerOptions:{target:ts.ScriptTarget.ES2022}}).outputText;
 return new Function(...Object.keys(context),`${code};return fn;`)(...Object.values(context));
}
test('password registration stays native-session-only and revokes connection evidence',async()=>{
 const editor={...resource.newServer(),host:'7920land',username:'researcher',port:22,auth:'password'},calls:unknown[]=[];
 let password='session-secret',output:Record<string,unknown>={'check:server:7920land':{old:true},'check:server:7920mp':{other:true}};
 const register=callback('registerPassword',{editor,password,selected:'server:7920land',invoke:async(command:string,args:unknown)=>calls.push({command,args}),setPassword:(next:string)=>{password=next;},setOutput:(update:(old:Record<string,unknown>)=>Record<string,unknown>)=>{output=update(output);},t:(zh:string)=>zh});
 await register();assert.equal(password,'');assert.deepEqual(output,{'check:server:7920mp':{other:true}});assert.deepEqual(calls,[{command:'remote_set_password',args:{host:'7920land',username:'researcher',port:22,password:'session-secret'}}]);assert.ok(!JSON.stringify(editor).includes('session-secret'));
 const page=harness();await page.ready();const contract=page.fill('resource-auth','password');assert.ok(!contract.fields.some(field=>field.id==='resource-password'));assert.ok(!contract.actions.some(action=>action.id==='resource-register-password'));
});
test('busy operations block navigation even before React rerenders the busy state',()=>{
 const lock={current:true};const reason=callback('navigationReason',{busy:false,lock,dirty:false,t:(zh:string)=>zh});assert.match(reason(),/操作进行中/);lock.current=false;assert.equal(reason(),null);
});
test('SSH aliases local and new remain server selections, and new servers expose their required root',async()=>{
 const page=harness({},['local','new']);let contract=await page.ready();
 assert.equal((contract.details as {selected:string}).selected,'server:local');assert.ok(!contract.actions.some(action=>action.id==='resource-local'));assert.equal(new Set(contract.actions.map(action=>action.id)).size,contract.actions.length);await page.action('resource-select-local');contract=await page.action('resource-select-server-local');assert.equal((contract.details as {selected:string}).selected,'server:local');
 contract=await page.action('resource-select-server-new');assert.equal((contract.details as {selected:string}).selected,'server:new');assert.ok(contract.fields.some(field=>field.id==='resource-host'&&field.value==='new'));
 contract=await page.action('resource-new');assert.equal((contract.details as {selected:string}).selected,'new');assert.ok(contract.fields.some(field=>field.id==='resource-root'));
});
test('leaving password authentication clears the hidden password draft',()=>{
 const editor={...resource.newServer(),auth:'password'};let password='secret',next:unknown;
 callback('editField',{editor,setPassword:(value:string)=>{password=value;},editServer:(server:unknown)=>{next=server;}})('auth','config');
 assert.equal(password,'');assert.deepEqual(next,{...editor,auth:'config'});
});
test('refresh replaces job identities without retaining a previous job status for the same directory',async()=>{
 const records:Record<string,unknown>={'/case/land':{host:'7920land',job:'job1'}};const page=harness(records);await page.ready();await page.action('resource-task-/case/land');await page.action('resource-case-status-/case/land');
 records['/case/land']={host:'7920land',job:'new-job'};const contract=await page.action('resource-refresh');
 const rows=(contract.details as {records:{job:string;status:string}[]}).records;assert.equal(rows[0].job,'new-job');assert.equal(rows[0].status,'unknown');
});
test('failed checks retain readable error evidence only when their details are visible',async()=>{
 const page=harness();await page.ready();page.failProbe();let contract=await page.action('resource-check');assert.ok(!('checkEvidence' in (contract.details as object)));
 contract=await page.action('resource-check-details');assert.match((contract.details as {checkEvidence:{probe_error:string}}).checkEvidence.probe_error,/SSH unavailable/);
 contract=await page.action('resource-check-details');assert.ok(!('checkEvidence' in (contract.details as object)));
});
