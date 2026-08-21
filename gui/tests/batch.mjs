import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const elements = new Map();
globalThis.document = {
  getElementById(id) {
    if (!elements.has(id)) elements.set(id, { textContent: '', disabled: false });
    return elements.get(id);
  },
};

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-batch-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');

const { state } = await import(join(temp, 'app', 'state.js'));
const { currentCases, freshCaseName, batchTarget } = await import(join(temp, 'app', 'batch.js'));

state.cases = [
  { name: 'old', dir: '/cases/old' },
  { name: 'site', dir: '/cases/site' },
  { name: 'site-2', dir: '/cases/site-2' },
];
state.createdCases.add('/cases/site-2');
state.batch = ['/cases/site-2', '/cases/old'];

if (currentCases().map(c => c.name).join('|') !== 'site-2') {
  throw new Error('old root cases leaked into the current-task list');
}
if (batchTarget().map(c => c.name).join('|') !== 'site-2') {
  throw new Error('old root cases leaked into batch execution');
}
if (freshCaseName('site') !== 'site-3' || freshCaseName('new') !== 'new') {
  throw new Error('new case names do not avoid old root directories');
}

console.log('batch: only current-created cases are visible and old names are not overwritten');
