import type { AgentEvent } from './model';

export async function respondToUiRequest(
  event: AgentEvent,
  drive: (action: string, args: Record<string, unknown>) => unknown | Promise<unknown>,
  respond: (id: string, ok: boolean, result: unknown) => Promise<unknown>,
) {
  if (!event.id) throw new Error('UI request is missing its response ID');
  let ok = true;
  let result: unknown;
  try {
    if (!event.action) throw new Error('UI request is missing its action');
    result = await drive(event.action, event.args || {});
  } catch (error) {
    ok = false;
    result = error instanceof Error ? error.message : String(error);
  }
  // Transport errors must not trigger a second response for an already completed action.
  await respond(event.id, ok, result);
}

export interface UiGrant {toolId: string; action: string; args: Record<string, unknown>; approved: boolean}
export function needsUiGrant(event:AgentEvent){return event.action==='commit'||event.action==='set'||(event.action==='workspace_remote'&&['submit','cancel','fetch'].includes(String(event.args?.operation)));}
function canonical(value:unknown):string {if(Array.isArray(value))return `[${value.map(canonical).join(',')}]`;if(value&&typeof value==='object')return `{${Object.entries(value).sort(([a],[b])=>a.localeCompare(b)).map(([key,v])=>`${JSON.stringify(key)}:${canonical(v)}`).join(',')}}`;return JSON.stringify(value)??'undefined';}
export function uiGrant(event: AgentEvent): UiGrant | null {
  if (!event.id) return null;
  try {
    const args:unknown=JSON.parse(event.arguments || '{}');
    if(!args||typeof args!=='object'||Array.isArray(args))return null;
    const source=args as Record<string,unknown>;
    if(['ui_commit','ui_set'].includes(event.name||''))return {toolId:event.id,action:event.name==='ui_commit'?'commit':'set',args:source,approved:event.preapproved===true};
    const operation=({remote_workspace_submit:'submit',remote_workspace_cancel:'cancel',remote_workspace_fetch:'fetch'} as Record<string,string>)[event.name||''];
    if(!operation||typeof source.name!=='string'||!source.name)return null;
    const optionalString=(key:string)=>typeof source[key]==='string'?source[key]:null;
    let request:Record<string,unknown>|null=null;
    if(operation==='submit'){
      if(!optionalString('host')||!['build-engine','build-kernel','test','run','parity','regress','verify'].includes(String(source.action)))return null;
      request={action:source.action};
      for(const key of ['case','preset','kind','package','engine','rtol','atol','first_records'])if(source[key]!=null)request[key]=source[key];
    }else if(!optionalString('job'))return null;
    return {toolId:event.id,action:'workspace_remote',args:{name:source.name,operation,host:optionalString('host'),job:optionalString('job'),request},approved:event.preapproved===true};
  } catch {return null;}
}
export function matchesGrant(grant: UiGrant | null, event: AgentEvent) {
  if (!grant?.approved || grant.action !== event.action) return false;
  if(event.action==='workspace_remote')return canonical(grant.args)===canonical(event.args);
  return event.action === 'commit' ? grant.args.target === event.args?.target : canonical(grant.args.fields) === canonical(event.args?.fields);
}
export function approvedUiRequest(grant:UiGrant|null,event:AgentEvent,approvedSnapshot:string,currentSnapshot:string){return approvedSnapshot===currentSnapshot&&matchesGrant(grant,event);}

/** Bind approval to the displayed inputs; read-only progress must not revoke consent. */
export function approvalSnapshot(identity: Record<string, unknown>, page: unknown): string {
  const snapshot = page && typeof page === 'object' ? page as Record<string, unknown> : {};
  const fields = Array.isArray(snapshot.fields) ? snapshot.fields : [];
  const inputs = fields.filter((field): field is Record<string, unknown> => Boolean(field) && typeof field === 'object' && field.readOnly !== true && field.visible !== false)
    .map(field => ({id: String(field.id ?? field.name ?? ''), value: field.value}))
    .sort((a, b) => a.id.localeCompare(b.id));
  return JSON.stringify({identity, step: snapshot.step, inputs});
}
