#!/usr/bin/env node
import {readFileSync, readdirSync} from 'node:fs';
import {join, dirname, relative} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
const root=dirname(dirname(fileURLToPath(import.meta.url)));
const ts=(await import(pathToFileURL(join(root,'gui/node_modules/typescript/lib/typescript.js')).href)).default;
function files(dir,exts){return readdirSync(dir,{withFileTypes:true}).flatMap(entry=>entry.isDirectory()?files(join(dir,entry.name),exts):exts.some(ext=>entry.name.endsWith(ext))&&!entry.name.includes('.test.')?[join(dir,entry.name)]:[]);}
function invocations(source){
 const ast=ts.createSourceFile('source.tsx',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX),out=[];
 function visit(node){
  if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&['invoke','invokeJson'].includes(node.expression.text)&&node.arguments[0]&&ts.isStringLiteralLike(node.arguments[0])){
   const arg=node.arguments[1],properties=arg&&ts.isObjectLiteralExpression(arg)?arg.properties:null;
   const keys=properties?.flatMap(property=>property.name&&(ts.isIdentifier(property.name)||ts.isStringLiteralLike(property.name))?[property.name.text]:[])??[];
   out.push({name:node.arguments[0].text,keys,complete:!arg||Boolean(properties&&!properties.some(ts.isSpreadAssignment))});
  }
  ts.forEachChild(node,visit);
 }
 visit(ast);return out;
}
function issues(calls,contract,registered){const errors=[];for(const {name,keys,complete} of calls){
 if(!registered.has(name)){errors.push(`${name}: not registered by Tauri`);continue;}
 const signature=contract[name];if(!signature){errors.push(`${name}: missing generated signature`);continue;}
 for(const key of keys)if(!Object.hasOwn(signature,key))errors.push(`${name}: unexpected argument ${key}`);
 if(complete)for(const [key,value] of Object.entries(signature))if(value.required&&!keys.includes(key))errors.push(`${name}: missing required argument ${key}`);
 }return errors;}
if(process.argv.includes('--self-test')){
 const calls=invocations("invoke<Foo>('check', {file: value, nested:{wrong: 1}}); invokeJson('missing',{}); invoke('check', {...args, typo: 1});");
 assert.deepEqual(calls[0].keys,['file','nested']);
 const result=issues(calls,{check:{file:{required:true},nested:{required:false}}},new Set(['check']));
 assert(result.some(s=>s.includes('missing: not registered')));assert(result.some(s=>s.includes('unexpected argument typo')));
 assert(issues(invocations("invoke('check',{})"),{check:{file:{required:true}}},new Set(['check'])).some(s=>s.includes('missing required argument file')));
 console.log('GUI contract parser regressions passed.');process.exit(0);
}
// A relative path from the repository root: MSYS2's python (Windows release job) reads a
// Windows absolute path as relative and cannot find the script.
const freshness=spawnSync(process.platform==='win32'?'python':'python3',['scripts/generate-gui-contract.py','--check'],{cwd:root,stdio:'inherit'});
if(freshness.error)throw freshness.error;if(freshness.status!==0)process.exit(freshness.status??1);
const generated=readFileSync(join(root,'gui/src/lib/command-contract.ts'),'utf8');
const contract=JSON.parse(generated.slice(generated.indexOf('=')+1,generated.indexOf(' as const;')));
const lib=readFileSync(join(root,'gui/src-tauri/src/lib.rs'),'utf8');
const handler=lib.match(/generate_handler!\[([\s\S]*?)\]/)?.[1];if(!handler)throw new Error('No Tauri handler found.');
const registered=new Set(handler.split(',').map(name=>name.trim().split('::').at(-1)).filter(Boolean));
const errors=[],called=new Set();
for(const file of files(join(root,'gui/src'),['.ts','.tsx'])){
 const calls=invocations(readFileSync(file,'utf8'));for(const call of calls)called.add(call.name);
 for(const error of issues(calls,contract,registered))errors.push(`${relative(root,file)}: ${error}`);
}
if(!called.size)errors.push('No frontend calls found; the contract check is ineffective.');
const backend=files(join(root,'gui/src-tauri/src'),['.rs']).map(file=>readFileSync(file,'utf8')).join('\n');
const emitted=new Set([...backend.matchAll(/emit\(\s*"([^"]+)"/g)].map(match=>match[1]));
const ipc=readFileSync(join(root,'gui/src/lib/ipc.ts'),'utf8').match(/export type AppEvent\s*=([^;]+);/)?.[1]??'';
for(const match of ipc.matchAll(/'([^']+)'/g))if(!emitted.has(match[1]))errors.push(`Frontend event ${match[1]} has no backend emitter.`);
if(errors.length){console.error(errors.join('\n'));process.exit(1);}
console.log(`GUI contract: ${registered.size} registered, ${called.size} statically called, all declared events resolve.`);
