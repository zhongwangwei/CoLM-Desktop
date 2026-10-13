import {readdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {spawn} from 'node:child_process';
import {join} from 'node:path';
const root=fileURLToPath(new URL('../gui/',import.meta.url));
async function walk(dir){const entries=await readdir(dir,{withFileTypes:true});return (await Promise.all(entries.map(entry=>entry.isDirectory()?walk(join(dir,entry.name)):entry.name.endsWith('.test.ts')?[join(dir,entry.name)]:[]))).flat();}
const tests=(await walk(join(root,'src'))).sort();
if(!tests.length)throw new Error('No React regression tests discovered');
const child=spawn(process.execPath,['--import','tsx','--import',fileURLToPath(new URL('./react-test-language.mjs',import.meta.url)),'--test',...tests],{cwd:root,stdio:'inherit'});
child.on('exit',code=>{process.exitCode=code??1;});
