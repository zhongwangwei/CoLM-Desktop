import type {RunState} from '../../lib/types';
import {remoteStudyReply,remoteStudyPermissions} from '../studies/remote-study';
export type Server={host:string;username:string;port:number;auth:string;identity_file:string;root:string;maps:{local:string;remote:string}[];threads:number;scheduler:string;partition:string;account:string;walltime:string;cpus:number;memory_gb:number;nodes:number;fetch_vars:string;env_script:string;directives:string[]};
export const newServer=():Server=>({host:'',username:'',port:0,auth:'config',identity_file:'',root:'',maps:[],threads:8,scheduler:'auto',partition:'',account:'',walltime:'',cpus:0,memory_gb:0,nodes:0,fetch_vars:'',env_script:'',directives:[]});
export function replaceServer(servers:Server[],original:string|null,next:Server){if(!next.host.trim()||next.host.startsWith('-')||/\s/.test(next.host))throw new Error('Invalid SSH host');if(servers.some(s=>s.host===next.host&&s.host!==original))throw new Error('Duplicate SSH host');if(original&&!servers.some(s=>s.host===original))throw new Error('Server was removed; reload before saving');return original?servers.map(s=>s.host===original?next:s):[...servers,next];}

export function remoteCheckRequest(action:string,preset:string,casePath:string,packageName:string,kind:string){const request:Record<string,string>={action,preset:preset.trim()||'default'};if(action==='verify'||action==='test'){if(!/^[A-Za-z0-9_-]+$/.test(packageName))throw new Error('Invalid test package');request.kind=action==='verify'?kind:'cargo';request.package=packageName;}if(action==='verify'){if(!casePath.startsWith('/')||/[\0\r\n]/.test(casePath))throw new Error('Absolute remote case path required');request.case=casePath;}return request;}

export function localRunEntries(running:Record<string,RunState>){return Object.entries(running).filter(([,run])=>run.location==='local');}

export function studyResourcePermissions(value:unknown,result?:unknown){
 const data=value&&typeof value==='object'?value as Record<string,unknown>:{};
 const initial=remoteStudyReply({record:data.remote});
 const reply=result==null?null:remoteStudyReply(result);
 const candidate=reply?.record??initial.record;
 const record=candidate?.job?candidate:null;
 const pending=!record&&Boolean(data.remote_pending||data.remote);
 return {...remoteStudyPermissions(record,pending||null,reply),pending};
}

export type ResourceCheck={configuration:string;checked_at:string;probe?:{hostname?:string;problems?:string[];root_exists?:boolean;root_writable?:boolean;maps?:{local:string;remote:string;local_exists:boolean;remote_exists:boolean}[]};engines?:{engines?:{id?:string;ready?:boolean}[]};probe_error?:string;engine_error?:string};
type CheckState={state:'pending'|'success'|'warning'|'error';label:string};
export function resourceCheckSummary(server:Server,check?:ResourceCheck,en=false){
 const t=(zh:string,english:string)=>en?english:zh;
 const pending:CheckState={state:'pending',label:t('尚未检查','Not checked')};
 if(!check||check.configuration!==JSON.stringify(server))return {ssh:pending,engine:pending,mapping:pending,checkedAt:null};
 const ssh:CheckState=check.probe_error?{state:'error',label:t('连接检查失败','Connection check failed')}:check.probe?.hostname?{state:'success',label:t('此次连接成功','Connected at last check')}:pending;
 const inventory=check.engines?.engines;
 const ready=Array.isArray(inventory)?inventory.filter(engine=>engine?.ready===true).length:0;
 const engine:CheckState=check.engine_error?{state:'error',label:t('引擎检查失败','Engine check failed')}:Array.isArray(inventory)?{state:ready?'success':'warning',label:ready?t(`${ready} 个可用快照`,`${ready} available snapshots`):t('未找到可用快照','No available snapshot')}:pending;
 const maps=Array.isArray(check.probe?.maps)?check.probe.maps:[];
 const matched=server.maps.filter(map=>maps.some(checked=>checked&&checked.local===map.local&&checked.remote===map.remote&&checked.local_exists===true&&checked.remote_exists===true)).length;
 const mapping:CheckState=check.probe_error?{state:'error',label:t('目录检查失败','Path check failed')}:!check.probe?pending:!server.maps.length?{state:'warning',label:t('尚未配置对应目录','No directory mapping')}:matched===server.maps.length?{state:'success',label:t('对应路径存在','Mapped paths exist')}:{state:'warning',label:t(`${server.maps.length-matched} 项待确认`,`${server.maps.length-matched} mappings need review`)};
 return {ssh,engine,mapping,checkedAt:Number.isFinite(Date.parse(check.checked_at))?check.checked_at:null};
}

export function resourceTaskInfo(value:unknown,result?:unknown){
 const object=(value:unknown):Record<string,unknown>=>value&&typeof value==='object'?value as Record<string,unknown>:{};
 const base=object(value),raw=object(result),reply=object(raw.reconciled_status??raw);
 const initial=object(base.remote_pending??base.remote??base.record??(base.pending&&typeof base.pending==='object'?base.pending:base));
 const identity=object((typeof reply.pending==='object'?reply.pending:null)??reply.record??initial);
 const status=object(reply.status??base.status);
 const pending=Boolean(reply.pending||reply.needs_reconciliation||(!reply.record&&(base.remote_pending||base.pending||base.needs_reconciliation)));
 const confirmed=typeof identity.host==='string'&&!!identity.host&&typeof identity.job==='string'&&!!identity.job;
 let state='unknown';
 if(confirmed&&!pending&&['queued','running','lost','unknown'].includes(String(status.state)))state=String(status.state);
 if(confirmed&&!pending&&status.state==='finished'&&Number.isInteger(status.exit_code))state=status.exit_code===0?'succeeded':'failed';
 return {host:typeof identity.host==='string'?identity.host:null,job:typeof identity.job==='string'?identity.job:null,status:state,pending};
}
