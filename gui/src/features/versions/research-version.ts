import type {FrozenRevision,ModelDraft} from '../../lib/types';
type Operations={save:(draft:ModelDraft)=>Promise<ModelDraft>;freeze:(id:string,assetRoot:string|null)=>Promise<FrozenRevision>};
export async function researchVersion(draft:ModelDraft|null,dirty:boolean,selected:FrozenRevision|null,assetRoot:string,operations:Operations):Promise<FrozenRevision>{
 if(!draft){if(!selected)throw new Error('请选择冻结版本 / Select a frozen revision.');return selected;}
 const saved=dirty?await operations.save(draft):draft;
 if(saved.id!==draft.id)throw new Error('保存的草稿不匹配 / Saved draft identity does not match.');
 const frozen=await operations.freeze(saved.id,assetRoot||null);
 if(frozen.version.id!==saved.id)throw new Error('冻结版本不匹配 / Frozen revision identity does not match.');
 return frozen;
}
export async function refreshDraftCode(draft:ModelDraft,hasUnsaved:()=>boolean,refresh:(id:string)=>Promise<ModelDraft>):Promise<ModelDraft>{
 const check=()=>{if(hasUnsaved())throw new Error('草稿有未保存修改，未覆盖当前草稿 / The draft has unsaved edits; it was not overwritten.');};
 check();const saved=await refresh(draft.id);check();
 if(saved.id!==draft.id)throw new Error('源码刷新草稿不匹配 / Refreshed draft identity does not match.');
 return saved;
}
