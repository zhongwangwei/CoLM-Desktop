import type {KernelEntry,Overlay} from '../../lib/types';
import {foundationControls,foundationValue,setFoundation,type ModelField} from './model';
import {modelSettingValue} from '../cases/model-settings';
import {cropEnabled,effectiveModelFields} from '../cases/case-model';
import {resetModelOption,setModelOption,unquote,type ConfigurationField,type ConfigurationResult} from './configuration-options';
export type SetupContext=Pick<ConfigurationResult,'fields'|'process_files'>;
export type SetupRequest={id:string;value:string};
export type SetupEnvironment={domain?:string|null;grid?:string|null;classification?:'IGBP'|'USGS';kernel?:KernelEntry;cropKernel?:KernelEntry};
const planting='DEF_TUNING_CROP_PLANTING_DAY';
const isSite=(environment:SetupEnvironment)=>['single','site'].includes(environment.domain??'');
const extra=[{id:'DEF_USE_VariablySaturatedFlow',zh:'变饱和流',en:'Variably saturated flow'},{id:'DEF_UnitCatchment_regional',zh:'区域单元汇流',en:'Regional unit-catchment routing'},{id:'DEF_USE_LULCC',zh:'土地覆盖变化',en:'Land-cover change'},{id:'crop',zh:'作物过程',en:'Crop processes'}].map(c=>({...c,keys:[c.id==='crop'?planting:c.id],choices:[['.true.','开启','On'],['.false.','关闭','Off']]}));
const order=['subgrid','soil','DEF_USE_VariablySaturatedFlow','DEF_USE_GridRiverLakeFlow','DEF_UnitCatchment_regional','DEF_USE_LULCC','DEF_USE_BGC','DEF_URBAN_RUN','crop','DEF_USE_TRACER'];
const controls=order.map(id=>[...foundationControls,...extra].find(c=>c.id===id)!);
export const setupHandledKeys=controls.flatMap(c=>c.keys);
export function setupChoices(overlay:Overlay,fields:ModelField[],environment:SetupEnvironment,queryFields:ConfigurationField[]=[],context?:SetupContext){
 queryFields=context?.fields??queryFields;
 const value=(key:string)=>(modelSettingValue(overlay,key)??queryFields.find(f=>f.name.toLowerCase()===key.toLowerCase())?.configured_value)?.trim().toLowerCase();
 const on=(key:string)=>value(key)==='.true.';
 const subgrid=foundationValue(overlay.physics,'subgrid',overlay.parameters),soil=foundationValue(overlay.physics,'soil',overlay.parameters);
 const list=(key:string)=>unquote(value(key)??'').split(',').map(v=>unquote(v.trim()).toLowerCase());
 const names=list('DEF_TRACER_NAMES'),methane=names.some(n=>['ch4','methane'].includes(n)),sediment=names.some(n=>['sediment','sed'].includes(n)),isotope=list('DEF_TRACER_TYPES').includes('isotope');
 const activeFiles=context?.process_files.filter(f=>f.verified&&f.active_methane_asset===true)??[];
 const methaneModes=activeFiles.map(f=>f.entries.find(e=>e.path.toLowerCase()==='def_methane%inundation_mode'&&!e.unset)?.value).map(v=>v?unquote(v).toLowerCase():null);
 const riverModes=['routing','hybrid','dh_all_thr05','dyn_routing_hybrid'],nonRiverModes=['wetwat','satellite','giems','dynamic_wtd','dynamic-wtd'];
 const methaneRiver=methaneModes.some(m=>m!==null&&riverModes.includes(m));
 const methaneKnown=methaneModes.length>0&&methaneModes.every(m=>m!==null&&[...riverModes,...nonRiverModes].includes(m));
 const crop=cropEnabled(effectiveModelFields(overlay)),compiledCrop=environment.kernel?.macros.includes('CROP')===true;
 function reason(id:string,v:string):string {
  if(!controls.find(c=>c.id===id)!.keys.every(k=>fields.some(f=>f.name===k&&!f.derived)))return '当前版本没有可写字段 / No writable field in this version';
  if(on('DEF_USE_TRACER')&&id!=='DEF_USE_TRACER'){
   if(methane&&((id==='DEF_USE_BGC'&&v==='.false.')||(id==='subgrid'&&v==='LCT')))return '请先关闭甲烷示踪或修改 CH4 定义 / Disable methane tracers or change the CH4 definition first';
   if(isotope&&id==='DEF_USE_LULCC'&&v==='.true.')return '请先关闭同位素示踪或修改 isotope 类型 / Disable isotope tracers or change the isotope type first';
   if(id==='DEF_USE_GridRiverLakeFlow'&&v==='.false.'){
    if(sediment)return '请先关闭河道泥沙示踪或修改 SEDIMENT 定义 / Disable sediment tracers or change SEDIMENT first';
    if(methane&&methaneRiver)return '请先修改甲烷 routing/hybrid 淹没方案或关闭示踪 / Change methane routing/hybrid inundation or disable tracers first';
    if(methane&&!methaneKnown)return '待核对：当前活动甲烷文件及淹没方案尚未确认；可先关闭示踪 / Pending: active methane asset and inundation mode are unverified; disable tracers first';
   }
  }
  if(on('DEF_GridRiverLake_FloodFeedback')&&((id==='subgrid'&&v!=='LCT')||(id==='DEF_URBAN_RUN'&&v==='.true.')||(id==='DEF_USE_LULCC'&&v==='.true.')||(['DEF_USE_GridRiverLakeFlow','DEF_USE_VariablySaturatedFlow'].includes(id)&&v==='.false.')))return '请先关闭漫滩回馈 / Disable floodplain feedback first';
  if(v==='.false.'&&id==='DEF_USE_VariablySaturatedFlow'&&value('DEF_USE_Campbell_SOIL_MODEL')==='.false.')return 'van Genuchten 必须开启变饱和流；请先改用 Campbell / van Genuchten requires variably saturated flow; choose Campbell first';
  if(v==='.false.'&&id==='DEF_USE_BGC'&&(crop||compiledCrop))return compiledCrop?'当前 CROP 内核要求 BGC；请先切换非作物内核 / Current CROP kernel requires BGC; switch to a non-crop kernel first':'请先关闭作物过程 / Disable crop first';
  if(v==='.false.'&&id==='DEF_USE_VariablySaturatedFlow'&&on('DEF_USE_TRACER'))return '请先关闭示踪过程 / Disable tracers first';
  if(v==='.false.')return '';
  if(id!=='crop'){const blocked=(id==='soil'?['DEF_USE_Campbell_SOIL_MODEL']:controls.find(c=>c.id===id)!.keys).map(k=>queryFields.find(f=>f.name===k)).find(f=>f&&['disabled','readonly','read-only'].includes(f.mode));if(blocked)return blocked.reason??'当前源码或内核不支持修改 / Current source or kernel disallows this option';}
  if(id==='subgrid'&&v==='LCT'&&(on('DEF_USE_BGC')||crop||compiledCrop))return '请先关闭 BGC 和作物过程 / Disable BGC and crop first';
  if(id==='soil'&&v==='Campbell'&&on('DEF_USE_TRACER'))return '请先关闭示踪过程 / Disable tracers first';
  if(isSite(environment)&&((id==='DEF_USE_BGC'&&on('DEF_URBAN_RUN'))||(id==='DEF_URBAN_RUN'&&on('DEF_USE_BGC'))))return '单点城市过程与 BGC 不能同开；请先关闭另一项 / Single-point urban and BGC cannot coexist; disable the other first';
  if(id==='DEF_USE_BGC'||id==='crop'){
   if(!['PFT','PC'].includes(subgrid))return '请先明确选择 PFT 或 PC / Select PFT or PC first';
   if(id==='DEF_USE_BGC'&&on('DEF_USE_LULCC'))return '请先关闭土地覆盖变化 / Disable land-cover change first';
   if(id==='crop'){
    if(!on('DEF_USE_BGC'))return '请先开启 BGC / Enable BGC first';
    if(!environment.cropKernel)return '待核对：缺少作物内核 / Pending: crop kernel unavailable';
    if(!environment.cropKernel.macros.includes('CROP'))return '内核未编入 CROP / Kernel lacks CROP';
    if(!environment.domain)return '待核对：请先确定模拟范围 / Pending: choose simulation domain';
   }
  }
  if(id==='DEF_URBAN_RUN'){
   if(environment.grid==='catchment'||environment.kernel?.macros.includes('CATCHMENT'))return '流域网格不支持城市过程 / Catchment grid does not support urban processes';
   if(!isSite(environment)&&!environment.grid&&!environment.kernel?.macros.some(m=>['GRIDBASED','UNSTRUCTURED','SinglePoint'].includes(m)))return '待核对：请先确定网格或候选内核 / Pending: choose grid or candidate kernel';
  }
  if(id==='DEF_URBAN_RUN'&&on('DEF_USE_TRACER'))return '请先关闭示踪过程 / Disable tracers first';
  if(id==='DEF_USE_TRACER'){
   if(methane&&(!on('DEF_USE_BGC')||!['PFT','PC'].includes(subgrid)))return '甲烷示踪需要先开启 BGC 并选择 PFT/PC / Methane requires BGC and PFT/PC first';
   if(isotope&&on('DEF_USE_LULCC'))return '请先关闭 LULCC 或修改 isotope 类型 / Disable LULCC or change isotope type first';
   if(sediment&&(isSite(environment)||!on('DEF_USE_GridRiverLakeFlow')||!environment.kernel?.macros.includes('GridRiverLakeFlow')))return 'SEDIMENT 需要空间模式和已开启的河湖内核 / SEDIMENT requires a spatial river-capable kernel with routing enabled';
   if(methane&&methaneRiver&&!on('DEF_USE_GridRiverLakeFlow'))return '甲烷 routing/hybrid 需要先开启河湖汇流 / Methane routing/hybrid requires river-lake routing first';
   if(soil!=='vG')return '请先选择 van Genuchten 和变饱和流 / Select van Genuchten and variably saturated flow';
   if(value('DEF_URBAN_RUN')!=='.false.')return '请先明确关闭城市过程 / Explicitly disable urban processes first';
   if(!environment.grid&&!isSite(environment))return '待核对：请先确定网格 / Pending: choose grid';
   if(environment.grid==='catchment')return '流域网格不支持示踪 / Catchment grid does not support tracers';
  }
  if(['DEF_USE_LULCC','DEF_USE_GridRiverLakeFlow','DEF_UnitCatchment_regional'].includes(id)){
   if(isSite(environment))return '需要空间模拟 / Requires spatial simulation';
   if(!environment.domain||!environment.grid)return '待核对：请先确定模拟范围和网格 / Pending: choose domain and grid';
   if(id==='DEF_USE_GridRiverLakeFlow'||id==='DEF_UnitCatchment_regional'){
    if(id==='DEF_UnitCatchment_regional'){
     if(!on('DEF_USE_GridRiverLakeFlow'))return '请先开启河湖汇流 / Enable river-lake routing first';
     if(value('DEF_USE_LULCC')!=='.false.')return '请先明确关闭土地覆盖变化 / Explicitly disable land-cover change first';
    }
    if(environment.grid==='catchment')return '流域网格不能启用网格河湖汇流 / Grid routing is unavailable for catchments';
    if(!environment.kernel)return '待核对：缺少匹配内核 / Pending: matching kernel unavailable';
    if(!environment.kernel.macros.includes('GridRiverLakeFlow'))return '内核未编入河湖汇流 / Kernel lacks river-lake routing';
   }else{
    if(!environment.classification)return '待核对：请先确定地表分类 / Pending: choose classification';
    if(environment.grid==='catchment')return '流域网格不支持土地覆盖变化 / Catchment grid does not support land-cover change';
    if(environment.classification==='USGS')return 'USGS 不支持土地覆盖变化 / USGS does not support land-cover change';
    if(value('DEF_USE_BGC')!=='.false.')return '请先明确关闭 BGC / Explicitly disable BGC first';
    if(on('DEF_UnitCatchment_regional'))return '请先关闭区域单元汇流 / Disable regional unit-catchment routing first';
    if(value('DEF_UnitCatchment_regional')!=='.false.')return '待核对：请先确认区域单元汇流已关闭 / Pending: confirm regional unit-catchment routing is off';
   }
  }
  return '';
 }
 return controls.map(c=>({...c,value:c.id==='crop'?(crop?'.true.':''):foundationValue(overlay.physics,c.id,overlay.parameters),pending:c.keys.some(k=>!queryFields.find(f=>f.name===k)||queryFields.find(f=>f.name===k)?.pending),choices:c.choices.map(([value,zh,en])=>({value,zh,en,reason:reason(c.id,value)}))}));
}
export function applySetupRequest(overlay:Overlay,fields:ModelField[],request:SetupRequest,environment:SetupEnvironment,context?:SetupContext):Overlay{
 const choice=setupChoices(overlay,fields,environment,context?.fields,context).find(c=>c.id===request.id);
 if(!choice)throw new Error('Unknown setup control');
 const value=request.id==='crop'&&!['.true.','.false.'].includes(request.value)?'.true.':request.value;
 const candidate=choice.choices.find(c=>c.value===value);if(!candidate)throw new Error('Invalid setup choice');if(candidate.reason)throw new Error(candidate.reason);
 if(request.id==='crop'){
  if(value==='.false.')return resetModelOption(overlay,planting);
  if(request.value==='.true.')return overlay;
  if(!/^\d+$/.test(request.value)||Number(request.value)>366||(!isSite(environment)&&Number(request.value)!==0))throw new Error('单点种植日为 0–366；空间模拟只能为 0 / Planting day: site 0–366, spatial 0');
  return setModelOption(overlay,fields,planting,request.value);
 }
 const foundation=foundationControls.find(c=>c.id===request.id);
 if(!foundation)return setModelOption(overlay,fields,request.id,request.value);
 // Migrate only the keys changed by this atomic control; unrelated scoped values stay intact.
 const keys=(request.id==='soil'&&request.value==='Campbell'?['DEF_USE_Campbell_SOIL_MODEL']:foundation.keys).map(k=>k.toLowerCase());
 const parameters=overlay.parameters.filter(p=>p.scope_instance.kind!=='case-scalar'||!keys.includes(p.raw_key.toLowerCase()));
 const physics={...overlay.physics};for(const p of overlay.parameters)if(p.scope_instance.kind==='case-scalar'&&keys.includes(p.raw_key.toLowerCase()))physics[p.raw_key]=p.value;
 return {...overlay,parameters,physics:setFoundation(physics,parameters,fields,request.id,request.value)};
}
