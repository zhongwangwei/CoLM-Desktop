import type {Site} from '../../lib/types';
export type {Site} from '../../lib/types';
export interface SchemaField {name:string;kind:string;default:string;doc?:string;group?:string;derived:boolean;values:string[];section:string}
// Only data/time/output belong to a case. Model parameters and physics stay in revisions.
export function isCaseField(field:SchemaField):boolean {
  return !field.derived && !!field.group && !/^DEF_USE_(USGS|IGBP)$/.test(field.name) && ((field.kind.startsWith('Character')&&/file|dir|path|namelist/i.test(field.name)) || ['时间与预热','初始场','输出与重启','输出变量','调试与诊断','强迫场','地表数据','网格与并行'].includes(field.section) || /^(SITE_|USE_SITE_|DEF_(forcing|dir_(rawdata|runtime|existing_srfdata)|timestep|domain|GRIDBASED|CatchmentMesh|file_mesh))/.test(field.name));
}
export function requiredNumber(value:string,label:string):number { if (!value.trim() || !Number.isFinite(Number(value))) throw new Error(`${label}: 请输入有效数字 / enter a number`); return Number(value); }
export function caseDirectory(root:string,name:string):string { if (!root.trim() || !name.trim() || /[\\/]|^\.{1,2}$/.test(name)) throw new Error('请填写根目录和有效算例名 / root and valid case name required'); return `${root.replace(/[\\/]$/, '')}/${name.trim()}`; }
export function physicsFields(physics:Record<string,string> = {}) {return Object.entries(physics).map(([path,value])=>({path,value}));}
export function versionMode(physics:Record<string,string> = {},classification?:'IGBP'|'USGS'):string {
  const enabled=(key:string)=>/^(true|\.true\.|1)$/i.test(physics[key]??'');
  const sub=classification==='USGS'?'usgs':enabled('DEF_USE_PFT')?'pft':enabled('DEF_USE_PC')?'pc':classification===undefined&&enabled('DEF_USE_USGS')?'usgs':'igbp';
  return enabled('DEF_URBAN_RUN')?`urban-${sub}`:sub;
}
export function displayLiteral(value:string):string {return /^(['"]).*\1$/.test(value)?value.slice(1,-1):value;}
export function siteVersionMismatch(site:Pick<Site,'crop'|'urban'>,physics:Record<string,string>={}):string|null {
 const enabled=(key:string)=>/^(true|\.true\.|1)$/i.test(physics[key]??'');
 if(site.crop!==cropEnabled(physics))return '站点作物类型与当前版本 CROP 配置不符 / Crop configuration mismatch';
 if(site.urban!==enabled('DEF_URBAN_RUN'))return '站点城市类型与当前版本配置不符 / Urban configuration mismatch';
 return null;
}
export function preparedDestination(directory:string,source:string):string {
 const stem=(source.split(/[\\/]/).pop()??'forcing').replace(/\.[^.]+$/,'');
 return `${directory.replace(/[\\/]$/,'')}/${stem}_colm.nc`;
}

export function effectiveModelFields(overlay?:{physics:Record<string,string>;parameters:{raw_key:string;value:string;scope_instance:{kind:string}}[]}):Record<string,string> {
 if(!overlay)return {};
 return {...overlay.physics,...Object.fromEntries(overlay.parameters.filter(p=>p.scope_instance.kind==='case-scalar').map(p=>[p.raw_key,p.value]))};
}
export function cropEnabled(fields:Record<string,string>):boolean {
 return Object.keys(fields).some(key=>key.toUpperCase()==='DEF_TUNING_CROP_PLANTING_DAY');
}
