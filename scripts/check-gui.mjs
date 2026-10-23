#!/usr/bin/env node
import {readFileSync, readdirSync} from 'node:fs';
import {join, dirname, relative} from 'node:path';
import {fileURLToPath, pathToFileURL} from 'node:url';
import {spawnSync} from 'node:child_process';
import assert from 'node:assert/strict';
const root=dirname(dirname(fileURLToPath(import.meta.url)));
const ts=(await import(pathToFileURL(join(root,'gui/node_modules/typescript/lib/typescript.js')).href)).default;
function files(dir,exts){return readdirSync(dir,{withFileTypes:true}).flatMap(entry=>entry.isDirectory()?files(join(dir,entry.name),exts):exts.some(ext=>entry.name.endsWith(ext))&&!entry.name.includes('.test.')?[join(dir,entry.name)]:[]);}
// 命令名不是字面量时（`remote ? 'bench_remote_x' : 'bench_x'`、参数标成字面量联合的 `command`）
// 也要逐个候选核对；原来直接跳过，9 处动态调用从没被查过（GUI 审查 A-13）。解析不出候选就报错。
function literalTypes(type){
 if(!type)return null;
 if(ts.isParenthesizedTypeNode(type))return literalTypes(type.type);
 if(ts.isLiteralTypeNode(type)&&ts.isStringLiteralLike(type.literal))return [type.literal.text];
 if(ts.isUnionTypeNode(type)){const all=type.types.map(literalTypes);return all.every(Boolean)?all.flat():null;}
 return null;
}
function constInitializer(identifier){
 for(let scope=identifier.parent;scope;scope=scope.parent)
  if(ts.isBlock(scope)||ts.isSourceFile(scope))for(const statement of scope.statements)
   if(ts.isVariableStatement(statement)&&statement.declarationList.flags&ts.NodeFlags.Const)
    for(const declaration of statement.declarationList.declarations)
     if(ts.isIdentifier(declaration.name)&&declaration.name.text===identifier.text)return declaration.initializer;
 return undefined;
}
function elementNames(expression){
 if(ts.isParenthesizedExpression(expression)||ts.isAsExpression(expression))return elementNames(expression.expression);
 if(ts.isArrayLiteralExpression(expression)){const all=expression.elements.map(names);return all.every(Boolean)?all.flat():null;}
 if(ts.isIdentifier(expression)){const initializer=constInitializer(expression);return initializer?elementNames(initializer):null;}
 return null;
}
function resolveName(identifier){
 for(let scope=identifier.parent;scope;scope=scope.parent){
  // `for (const event of ['a', 'b'] as const)`：循环变量取数组里的每一个。
  if(ts.isForOfStatement(scope)&&ts.isVariableDeclarationList(scope.initializer)&&scope.initializer.declarations.some(d=>ts.isIdentifier(d.name)&&d.name.text===identifier.text))return elementNames(scope.expression);
  if(ts.isFunctionLike(scope))for(const parameter of scope.parameters)
   if(ts.isIdentifier(parameter.name)&&parameter.name.text===identifier.text)return literalTypes(parameter.type);
  if(ts.isBlock(scope)||ts.isSourceFile(scope))for(const statement of scope.statements)
   if(ts.isVariableStatement(statement)&&statement.declarationList.flags&ts.NodeFlags.Const)
    for(const declaration of statement.declarationList.declarations)
     if(ts.isIdentifier(declaration.name)&&declaration.name.text===identifier.text)
      return literalTypes(declaration.type)??(declaration.initializer?names(declaration.initializer):null);
 }
 return null;
}
function names(expression){
 if(ts.isParenthesizedExpression(expression)||ts.isAsExpression(expression))return names(expression.expression);
 if(ts.isStringLiteralLike(expression))return [expression.text];
 if(ts.isConditionalExpression(expression)){const a=names(expression.whenTrue),b=names(expression.whenFalse);return a&&b?[...a,...b]:null;}
 if(ts.isIdentifier(expression))return resolveName(expression);
 return null;
}
function invocations(source){
 const ast=ts.createSourceFile('source.tsx',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX),out=[];
 function visit(node){
  // 第一个参数是对象的不是 IPC（组件自己的 `invoke` 回调属性）。
  if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&['invoke','invokeJson'].includes(node.expression.text)&&node.arguments[0]&&!ts.isObjectLiteralExpression(node.arguments[0])){
   const arg=node.arguments[1],properties=arg&&ts.isObjectLiteralExpression(arg)?arg.properties:null;
   const keys=properties?.flatMap(property=>property.name&&(ts.isIdentifier(property.name)||ts.isStringLiteralLike(property.name))?[property.name.text]:[])??[];
   const complete=!arg||Boolean(properties&&!properties.some(ts.isSpreadAssignment));
   const first=node.arguments[0],dynamic=!ts.isStringLiteralLike(first),resolved=names(first);
   if(!resolved)out.push({name:null,text:first.getText(ast),keys,complete,dynamic});
   else for(const name of resolved)out.push({name,keys,complete,dynamic});
  }
  ts.forEachChild(node,visit);
 }
 visit(ast);return out;
}
function listened(source){
 const ast=ts.createSourceFile('source.tsx',source,ts.ScriptTarget.Latest,true,ts.ScriptKind.TSX),out=[];
 (function visit(node){
  if(ts.isCallExpression(node)&&ts.isIdentifier(node.expression)&&node.expression.text==='listen'&&node.arguments[0])out.push(...(names(node.arguments[0])??[]));
  ts.forEachChild(node,visit);
 })(ast);
 return out;
}
function issues(calls,contract,registered){const errors=[];for(const {name,text,keys,complete} of calls){
 if(name===null){errors.push(`${text}: command name cannot be resolved to literals; type it as a union of command names`);continue;}
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
 const dynamic=invocations("function go(command: 'a' | 'b'){ invoke(command, {x: 1}); } invoke(flag ? 'c' : ('d'), {}); const k = 'e'; invoke(k); invoke(anything); remote({ id: 1 }); invoke({ id: 1 });");
 assert.deepEqual(dynamic.map(call=>call.name),['a','b','c','d','e',null]);
 assert(issues(dynamic,{a:{x:{required:false}},b:{},c:{},d:{},e:{}},new Set(['a','b','c','d','e'])).some(s=>s.includes('unexpected argument x')&&s.startsWith('b:')));
 assert(issues(dynamic,{},new Set()).some(s=>s.includes('anything: command name cannot be resolved')));
 assert.deepEqual(listened("listen('run://done', f); listen(flag ? 'a' : 'b', g); const all = ['c', 'd'] as const; for (const e of all) listen(e, h);"),['run://done','a','b','c','d']);
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
const errors=[],called=new Set(),dynamic=new Set(),listeners=new Set();
for(const file of files(join(root,'gui/src'),['.ts','.tsx'])){
 const source=readFileSync(file,'utf8');for(const name of listened(source))listeners.add(name);
 // ipc.ts 是 invoke 本身，命令名是类型参数，不是调用点。
 if(relative(join(root,'gui/src'),file).split(/[\\/]/).join('/')==='lib/ipc.ts')continue;
 const calls=invocations(source);for(const call of calls)if(call.name){called.add(call.name);if(call.dynamic)dynamic.add(call.name);}
 for(const error of issues(calls,contract,registered))errors.push(`${relative(root,file)}: ${error}`);
}
if(!called.size)errors.push('No frontend calls found; the contract check is ineffective.');
const backend=files(join(root,'gui/src-tauri/src'),['.rs']).map(file=>readFileSync(file,'utf8')).join('\n');
const emitted=new Set([...backend.matchAll(/emit\(\s*"([^"]+)"/g)].map(match=>match[1]));
const ipc=readFileSync(join(root,'gui/src/lib/ipc.ts'),'utf8').match(/export type AppEvent\s*=([^;]+);/)?.[1]??'';
for(const match of ipc.matchAll(/'([^']+)'/g)){
 if(!emitted.has(match[1]))errors.push(`Frontend event ${match[1]} has no backend emitter.`);
 // 反向：声明了却没人听，后端发出去的就悄悄丢了。
 if(!listeners.has(match[1]))errors.push(`Frontend event ${match[1]} is declared but nothing listens to it.`);
}
if(errors.length){console.error(errors.join('\n'));process.exit(1);}
console.log(`GUI contract: ${registered.size} registered, ${called.size} called (${dynamic.size} through resolved dynamic names), every declared event has an emitter and a listener.`);
