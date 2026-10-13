import test from 'node:test';import assert from 'node:assert/strict';import {makeRecord,upsertParameter,importParameters,type Descriptor} from './model';
const d={id:'lct:IGBP:x',raw_key:'x',storage:'case-nml',scope:'land-cover-class',unit:'m'} as Descriptor;
test('typed scope distinguishes land classes and validates indices',()=>{const a=makeRecord(d,'1',1,''),b=makeRecord(d,'2',2,'');assert.equal(upsertParameter([a],b).length,2);assert.equal(upsertParameter([a],{...a,value:'3'})[0].value,'3');assert.throws(()=>makeRecord(d,'1',18,''));});
test('imports reject dataset fields and duplicate overrides',()=>{const r=makeRecord(d,'1',1,'');assert.throws(()=>importParameters(JSON.stringify([r]),[d],[],[]));assert.throws(()=>importParameters(JSON.stringify([r,r]),[d],[{name:'x'} as never],[]));});

import {foundationControls,foundationValue,setFoundation,type ModelField} from './model';
const foundationFields=foundationControls.flatMap(c=>c.keys).map(name=>({name,derived:false}) as ModelField);
test('foundation selection updates exactly one vegetation structure atomically',()=>{
 const before={DEF_USE_LCT:'.true.',keep:'1'};const next=setFoundation(before,[],foundationFields,'subgrid','PC');
 assert.deepEqual(next,{DEF_USE_LCT:'.false.',DEF_USE_PFT:'.false.',DEF_USE_PC:'.true.',keep:'1'});
 assert.deepEqual(before,{DEF_USE_LCT:'.true.',keep:'1'});assert.equal(foundationValue(next,'subgrid'),'PC');
 assert.equal(foundationValue({},'subgrid'),'');assert.equal(foundationValue({DEF_USE_PFT:'.true.'},'subgrid'),'');
 assert.throws(()=>setFoundation(before,[],foundationFields,'DEF_SUBGRID_SCHEME','PFT'));
});
test('vG enables variably saturated flow and preserves unrelated physics',()=>{
 const next=setFoundation({keep:'1'},[],foundationFields,'soil','vG');assert.equal(next.DEF_USE_Campbell_SOIL_MODEL,'.false.');assert.equal(next.DEF_USE_VariablySaturatedFlow,'.true.');assert.equal(next.keep,'1');assert.equal(foundationValue(next,'soil'),'vG');
 assert.equal(foundationValue({DEF_USE_Campbell_SOIL_MODEL:'.false.'},'soil'),'');
 assert.equal(setFoundation(next,[],foundationFields,'soil','Campbell').DEF_USE_VariablySaturatedFlow,'.true.');
});
test('foundation controls read explicit case-scalar parameter overrides',()=>{
 const parameters=['LCT','PFT','PC'].map(name=>({raw_key:`def_use_${name.toLowerCase()}`,value:name==='PFT'?'.true.':'.false.',scope_instance:{kind:'case-scalar'}})) as Parameters<typeof foundationValue>[2];
 assert.equal(foundationValue({},'subgrid',parameters),'PFT');
 assert.equal(setFoundation({},parameters!,foundationFields,'DEF_USE_BGC','.true.').DEF_USE_BGC,'.true.');
 const soil=[{raw_key:'DEF_USE_Campbell_SOIL_MODEL',value:'.false.',scope_instance:{kind:'case-scalar'}},{raw_key:'DEF_USE_VariablySaturatedFlow',value:'.true.',scope_instance:{kind:'case-scalar'}}] as Parameters<typeof foundationValue>[2];
 assert.equal(foundationValue({},'soil',soil),'vG');
});
test('BGC dependencies and parameter conflicts are rejected without mutations',()=>{
 const initial=setFoundation({},[],foundationFields,'subgrid','PFT');const bgc=setFoundation(initial,[],foundationFields,'DEF_USE_BGC','.true.');
 assert.throws(()=>setFoundation(bgc,[],foundationFields,'subgrid','LCT'),/BGC/);assert.equal(bgc.DEF_USE_PFT,'.true.');
 assert.throws(()=>setFoundation({},[],foundationFields,'DEF_USE_BGC','.true.'),/PFT/);
 const record={raw_key:'def_use_lct',value:'.false.'} as never;
 assert.throws(()=>setFoundation({},[record],foundationFields,'subgrid','PC'),/conflicting/);
 assert.throws(()=>setFoundation({},[{raw_key:'DEF_USE_BGC',value:'.true.'} as never],foundationFields,'subgrid','LCT'),/BGC/);
 assert.throws(()=>setFoundation({},[],foundationFields.filter(f=>f.name!=='DEF_USE_PC'),'subgrid','PC'),/writable/);
 assert.throws(()=>setFoundation({},[],foundationFields.map(f=>({...f,derived:true})),'soil','vG'),/writable/);
});
