import type {ReactNode} from 'react';
import {Advanced} from '../../components/ui';
import type {SchemaField} from './case-model';
export type ProcessDataGroup={id:string;title:string;fields:SchemaField[];open:boolean};
export default function ProcessDataGroups({groups,onToggle,render,disabled=false}:{groups:ProcessDataGroup[];onToggle:(id:string,open:boolean)=>void;render:(field:SchemaField)=>ReactNode;disabled?:boolean}){
 return <div className="page-stack">{groups.map(group=><Advanced key={group.id} title={`${group.title} · ${group.fields.length}`} open={group.open} onOpenChange={open=>{if(!disabled)onToggle(group.id,open);}}>{group.open&&<div className="form-grid">{group.fields.map(render)}</div>}</Advanced>)}</div>;
}
