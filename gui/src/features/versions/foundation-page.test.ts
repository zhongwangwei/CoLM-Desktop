import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import ts from 'typescript';
import * as model from './model';
import * as setup from './scientific-setup';
import {researchVersion} from './research-version';
import type {ParameterOverride} from '../../lib/types';
import {compatibleKernel} from '../runs/run-model';
import {effectiveModelFields} from '../cases/case-model';
import type {UiField,UiAction} from '../../lib/guide';
type Element={type:string;props:Record<string,unknown>;children:unknown[]};
function page(subgrid?:'LCT'|'PFT'|'PC',physics:Record<string,string>={},parameters:ParameterOverride[]=[]){
 const version={name:'Official',note:'',version:{id:'official',revision:1},code:{kind:'official'},overlay:{physics,parameters,process_files:[],hybrid:{}},applicability:{sites:[],land_classes:[]}};
 let failedCommand='';let draft={...version,id:'custom',generation:1};const calls:{command:string;args:Record<string,unknown>}[]=[],scrolls:unknown[]=[],routes:string[]=[];const state={language:'zh',version,versionRef:version.version,versions:[version],simulationReturn:(subgrid?{route:'new-case',step:2,subgrid}:null) as {route:'new-case'|'case';step:2;subgrid?:'LCT'|'PFT'|'PC';setup?:setup.SetupRequest}|null,expert:false,kernels:[]};
 let cursor=0,effectCursor=0,contract:{fields:UiField[];actions:UiAction[];status:string};const slots:unknown[]=[],registered=new Set<number>(),effects:(()=>unknown)[]=[],elements:Element[]=[];
 const context={document:{querySelector:(selector:string)=>{assert.equal(selector,'.main-pane');return {scrollTo:(position:unknown)=>scrolls.push(position)};}},...model,...setup,ScientificSetupCards:'ScientificSetupCards',researchVersion,compatibleKernel,effectiveModelFields,ConfigurationOptions:'ConfigurationOptions',React:{createElement:(type:string,props:Record<string,unknown>,...children:unknown[])=>{const result={type,props:props??{},children};elements.push(result);return result;}},...Object.fromEntries(['Button','Card','Field','Input','Select','Notice','Advanced','CodeWorkspace'].map(k=>[k,k])),IGBP:[],USGS:[],PFT:[],useApp:()=>({state,update:(patch:object)=>Object.assign(state,patch),navigate:(route:string)=>{routes.push(route);return true;},refreshVersions:async()=>{}}),useNavigationGuard:()=>{},usePageContract:(c:typeof contract)=>{contract=c;},useState:(initial:unknown)=>{const key=cursor++;if(!(key in slots))slots[key]=initial;return [slots[key],(value:unknown)=>{slots[key]=value;}];},useRef:(initial:unknown)=>{const key=cursor++;if(!(key in slots))slots[key]={current:initial};return slots[key];},useEffect:(fn:()=>unknown)=>{const key=effectCursor++;if(!registered.has(key)){registered.add(key);effects.push(fn);}},invokeJson:async(command:string,args:Record<string,unknown>={})=>{calls.push({command,args});if(command===failedCommand)throw new Error(command+' failed');switch(command){case 'model_create':return draft;case 'model_save_draft':draft=args.draft as typeof draft;return {...draft,generation:draft.generation+1};case 'model_freeze':return {...draft,version:{id:draft.id,revision:2}};case 'model_configuration':return {fields:[],process_files:[],pft_parameters:[],warnings:[],context_complete:false};case 'parameter_catalog':return [];case 'model_fields':return setup.setupHandledKeys.map(name=>({name,kind:name==='DEF_TUNING_CROP_PLANTING_DAY'?'integer':'logical',values:[],section:'',derived:false,default:'.false.'}));case 'model_drafts':return [draft];case 'history_list':return {items:[]};case 'model_draft':return draft;default:throw Error(command);}}};
 const source=readFileSync(new URL('./VersionsPage.tsx',import.meta.url),'utf8').replace(/^import .*$/gm,'').replace('export default function','function');
 const code=ts.transpileModule(source,{compilerOptions:{target:ts.ScriptTarget.ES2022,jsx:ts.JsxEmit.React}}).outputText;
 const Component=new Function(...Object.keys(context),`${code};return VersionsPage;`)(...Object.values(context));
 function render(){cursor=0;effectCursor=0;elements.length=0;Component();return contract;}
 async function mount(){render();effects.splice(0).forEach(f=>f());for(let i=0;i<12;i++)await Promise.resolve();return render();}
 function card(){return elements.find(e=>e.type==='ScientificSetupCards')!;}
 function choose(id:string,value:string){const p=card().props;if(p.disabled||!p.onChange)throw new Error('当前版本不可修改');(p.onChange as (overlay:unknown)=>void)(setup.applySetupRequest(p.overlay as Parameters<typeof setup.applySetupRequest>[0],setup.setupHandledKeys.map(name=>({name,kind:name==='DEF_TUNING_CROP_PLANTING_DAY'?'integer':'logical',derived:false,default:'.false.',values:[],section:''})),{id,value},p.environment as setup.SetupEnvironment));return render();}
 return {render,mount,elements,state,calls,routes,version,card,choose,scrolls,fail:(command:string)=>{failedCommand=command;},settle:async()=>{for(let i=0;i<24;i++)await Promise.resolve();return render();}};
}
test('parent exposes the actual shared cards and removes hidden foundation selectors',async()=>{
 const p=page();let c=await p.mount();assert.equal(p.card().props.onChange,undefined);assert.throws(()=>p.choose('subgrid','PC'),/不可修改/);
 assert.ok(!c.fields.some(f=>f.id.startsWith('version-foundation-')),'the shared visible card contract replaces parent hidden fields');
 await c.actions.find(a=>a.id==='version-draft-custom')!.run();p.render();assert.equal(typeof p.card().props.onChange,'function');
 c=p.choose('subgrid','PC');assert.equal(model.foundationValue((p.card().props.overlay as {physics:Record<string,string>}).physics,'subgrid'),'PC');assert.equal(c.status,'未保存');
 assert.ok(!c.fields.find(f=>f.id==='version-physics')?.options?.some(o=>o.value==='DEF_SUBGRID_SCHEME'||o.value==='DEF_USE_LCT'));
 const code=p.elements.find(e=>e.type==='CodeWorkspace')!;(code.props.onPendingChange as (v:boolean)=>void)(true);p.render();assert.equal(p.card().props.disabled,true);assert.throws(()=>p.choose('subgrid','LCT'),/不可修改/);
});

for(const mode of ['PFT','PC'] as const)test(`${mode} simulation entry applies the complete draft foundation before freezing`,async()=>{
 const p=page(mode);let c=await p.mount();assert.deepEqual(p.version.overlay.physics,{});
 for(const [id,value] of [['version-name',`${mode} test`],['version-note','choose vegetation structure']]){c.fields.find(f=>f.id===id)!.set(value);c=p.render();}
 if(mode==='PFT')await c.actions.find(a=>a.id==='version-create')!.run();
 else {const button=p.elements.find(e=>e.type==='Button'&&e.children.includes('派生新版本'))!;assert.equal(button.props.disabled,false);(button.props.onClick as ()=>void)();await p.settle();}
 c=p.render();
 assert.equal(model.foundationValue((p.card().props.overlay as {physics:Record<string,string>}).physics,'subgrid'),mode);assert.equal(c.status,'未保存');
 assert.equal(c.actions.find(a=>a.id==='version-return-simulation')?.enabled,false);assert.deepEqual(p.version.overlay.physics,{});
 await c.actions.find(a=>a.id==='version-freeze')!.run();c=p.render();
 assert.equal(p.state.versionRef.id,'custom');assert.equal(p.state.versionRef.revision,2);
 assert.deepEqual(p.state.version.overlay.physics,{DEF_USE_LCT:'.false.',DEF_USE_PFT:mode==='PFT'?'.true.':'.false.',DEF_USE_PC:mode==='PC'?'.true.':'.false.'});
 assert.deepEqual(p.version.overlay.physics,{});assert.ok(p.calls.some(call=>call.command==='model_save_draft'));assert.ok(p.calls.some(call=>call.command==='model_freeze'));
 await c.actions.find(a=>a.id==='version-return-simulation')!.run();assert.deepEqual(p.routes,['new-case']);
});
test('requested LCT retains an editable BGC draft so the user can explicitly resolve the conflict',async()=>{
 const physics={DEF_USE_LCT:'.false.',DEF_USE_PFT:'.true.',DEF_USE_PC:'.false.',DEF_USE_BGC:'.true.'},p=page('LCT',physics);let c=await p.mount();
 for(const [id,value] of [['version-name','LCT'],['version-note','test conflict']]){c.fields.find(f=>f.id===id)!.set(value);c=p.render();}
 await c.actions.find(a=>a.id==='version-create')!.run();c=p.render();
 assert.ok(p.calls.some(call=>call.command==='model_create'));assert.match(c.status,/BGC/);
 assert.equal(typeof p.card().props.onChange,'function');
 assert.equal(model.foundationValue((p.card().props.overlay as {physics:Record<string,string>}).physics,'DEF_USE_BGC'),'.true.');assert.deepEqual(p.version.overlay.physics,physics);
 p.choose('DEF_USE_BGC','.false.');c=p.choose('subgrid','LCT');
 await c.actions.find(a=>a.id==='version-freeze')!.run();assert.equal(model.foundationValue(p.state.version.overlay.physics,'subgrid'),'LCT');
 assert.equal(p.state.version.overlay.physics.DEF_USE_BGC,'.false.');assert.equal(p.version.overlay.physics.DEF_USE_BGC,'.true.');
});

test('requested structure replaces only case-scalar structure records inside the new draft',async()=>{
 const records=['LCT','PFT','PC','TRACER'].map(name=>({parameter_id:`case:DEF_USE_${name}`,raw_key:`DEF_USE_${name}`,value:name==='LCT'?'.true.':'.false.',unit:null,scope_instance:{kind:'case-scalar'}} as ParameterOverride));
 const p=page('PC',{},records);let c=await p.mount();
 for(const [id,value] of [['version-name','PC from scalar'],['version-note','preserve unrelated parameters']]){c.fields.find(f=>f.id===id)!.set(value);c=p.render();}
 await c.actions.find(a=>a.id==='version-create')!.run();c=p.render();assert.equal(model.foundationValue((p.card().props.overlay as {physics:Record<string,string>}).physics,'subgrid'),'PC');
 await c.actions.find(a=>a.id==='version-freeze')!.run();
 assert.deepEqual(p.state.version.overlay.parameters,[records[3]]);assert.deepEqual(p.version.overlay.parameters,records);assert.deepEqual(p.version.overlay.physics,{});
});

test('soil card intent is applied atomically to a derived version and can be frozen for return',async()=>{
 const p=page();p.state.simulationReturn={route:'new-case',step:2,setup:{id:'soil',value:'vG'}};let c=await p.mount();
 for(const [id,value] of [['version-name','soil model'],['version-note','explicit hydraulic scheme']]){c.fields.find(f=>f.id===id)!.set(value);c=p.render();}
 await c.actions.find(a=>a.id==='version-create')!.run();c=p.render();
 const overlay=p.card().props.overlay as {physics:Record<string,string>};assert.equal(model.foundationValue(overlay.physics,'soil'),'vG');assert.equal(overlay.physics.DEF_USE_VariablySaturatedFlow,'.true.');const query=p.calls.find(call=>call.command==='model_configuration');assert.ok(query);assert.deepEqual(query.args.source,{kind:'draft',id:'custom',generation:1,overlay:p.version.overlay});assert.deepEqual(p.version.overlay.physics,{});
 await c.actions.find(a=>a.id==='version-freeze')!.run();assert.equal(p.state.version.overlay.physics.DEF_USE_Campbell_SOIL_MODEL,'.false.');
});

for(const mode of ['PFT','PC'] as const)test(`${mode} has a ready one-action apply path that saves, freezes and returns`,async()=>{
 const p=page(mode);let c=await p.mount();const original=JSON.stringify(p.version);
 const action=c.actions.find(a=>a.id==='version-apply-simulation-structure');assert.ok(action);assert.equal(action.enabled,true,'the requested setup must not wait for manual version metadata');
 if(mode==='PFT')await action.run();else{const b=p.elements.find(e=>e.type==='Button'&&e.children.includes('应用 PC 并继续'))!;assert.equal(b.props.disabled,false);(b.props.onClick as ()=>void)();await p.settle();}
 c=p.render();assert.equal(model.foundationValue(p.state.version.overlay.physics,'subgrid'),mode);assert.deepEqual(p.routes,['new-case']);assert.equal(JSON.stringify(p.version),original);assert.equal(c.status,'');
 assert.deepEqual(p.calls.filter(x=>['model_create','model_save_draft','model_freeze'].includes(x.command)).map(x=>x.command),['model_create','model_save_draft','model_freeze']);
 const created=p.calls.find(x=>x.command==='model_create')!;assert.ok(String(created.args.name).includes(mode));assert.ok(String(created.args.note).includes(mode));
});
for(const failure of ['model_save_draft','model_freeze'])test(`structure apply retains the original version and recoverable draft when ${failure} fails`,async()=>{
 const p=page('PC');let c=await p.mount();p.fail(failure);const action=c.actions.find(a=>a.id==='version-apply-simulation-structure');assert.ok(action);await assert.rejects(async()=>{await action.run();},/failed/);c=p.render();
 assert.equal(p.state.versionRef.id,'official');assert.deepEqual(p.routes,[]);assert.equal(typeof p.card().props.onChange,'function');assert.match(c.status,/failed/);
 p.fail('');await c.actions.find(a=>a.id==='version-apply-simulation-structure')!.run();assert.equal(p.state.versionRef.id,'custom');assert.deepEqual(p.routes,['new-case']);assert.equal(p.calls.filter(x=>x.command==='model_create').length,1,'retry reuses the draft');
});

test('quick structure apply preserves the draft instead of freezing an incompatible LCT request',async()=>{
 const p=page('LCT',{DEF_USE_LCT:'.false.',DEF_USE_PFT:'.true.',DEF_USE_PC:'.false.',DEF_USE_BGC:'.true.'});let c=await p.mount();await c.actions.find(a=>a.id==='version-apply-simulation-structure')!.run();c=p.render();
 assert.match(c.status,/BGC/);assert.deepEqual(p.routes,[]);assert.equal(p.state.versionRef.id,'official');assert.ok(!p.calls.some(x=>x.command==='model_freeze'));
 p.choose('DEF_USE_BGC','.false.');c=p.render();await c.actions.find(a=>a.id==='version-apply-simulation-structure')!.run();assert.deepEqual(p.routes,['new-case']);assert.equal(model.foundationValue(p.state.version.overlay.physics,'subgrid'),'LCT');assert.equal(p.calls.filter(x=>x.command==='model_create').length,1);
});

test('existing case PFT request freezes its model change before returning to case settings',async()=>{
 const p=page();p.state.simulationReturn={route:'case',step:2,setup:{id:'subgrid',value:'PFT'}};const c=await p.mount();await c.actions.find(a=>a.id==='version-apply-simulation-structure')!.run();p.render();assert.equal(model.foundationValue(p.state.version.overlay.physics,'subgrid'),'PFT');assert.deepEqual(p.routes,['case']);assert.ok(!p.calls.some(x=>x.command==='case_data_save'));
});

test('structure application starts at its primary action and returns at the top after successful freeze',async()=>{
 const p=page('PC'),c=await p.mount();assert.deepEqual(p.scrolls,[{top:0}]);await c.actions.find(a=>a.id==='version-apply-simulation-structure')!.run();assert.deepEqual(p.scrolls,[{top:0},{top:0}]);
});
