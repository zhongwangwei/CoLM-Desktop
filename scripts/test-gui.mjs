import {readdir} from 'node:fs/promises';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {join} from 'node:path';
const root=fileURLToPath(new URL('../',import.meta.url));
const tests=(await readdir(join(root,'gui/tests'))).filter(name=>name.endsWith('.mjs')).sort();
let failures=0;
for(const name of tests){const test=spawnSync(process.execPath,[join(root,'gui/tests',name)],{cwd:root,encoding:'utf8'});if(test.status!==0){failures++;process.stderr.write(test.stdout+test.stderr);}else process.stdout.write(`PASS ${name}\n`);}
console.log(`${tests.length-failures}/${tests.length} legacy scripts passed`);process.exitCode=failures?1:0;
