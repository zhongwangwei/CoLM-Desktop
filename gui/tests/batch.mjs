import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

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

const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;
const { state } = await import(moduleUrl('state.js'));
const { currentCases, freshCaseName, batchTarget, sourceSite } = await import(moduleUrl('batch.js'));

state.cases = [
  { name: 'old', dir: '/cases/old' },
  { name: 'site', dir: '/cases/site' },
  { name: 'site-2', dir: '/cases/site-2' },
];
state.createdCases.add('/cases/site-2');
state.batch = ['/cases/site-2', '/cases/old'];
state.sites = [{ name: 'site', site_file: '/data/site.nc', obs_file: '/data/site-obs.nc' }];
state.createdBySite.set('/data/site.nc', '/cases/site-2');

if (currentCases().map(c => c.name).join('|') !== 'site-2') {
  throw new Error('old root cases leaked into the current-task list');
}
if (batchTarget().map(c => c.name).join('|') !== 'site-2') {
  throw new Error('old root cases leaked into batch execution');
}
if (freshCaseName('site') !== 'site-3' || freshCaseName('new') !== 'new') {
  throw new Error('new case names do not avoid old root directories');
}
if (sourceSite(state.cases[2])?.obs_file !== '/data/site-obs.nc') {
  throw new Error('renamed case lost the observation file of its source site');
}

console.log('batch: only current-created cases are visible and old names are not overwritten');

// 运行目标：勾了就是勾中的；一个没勾就是本次全部算例，没勾的不能从列表里消失。
{
  state.cases = [{ name: 'a', dir: '/c/a' }, { name: 'b', dir: '/c/b' }, { name: 'old', dir: '/c/old' }];
  state.createdCases.clear(); state.createdCases.add('/c/a'); state.createdCases.add('/c/b');
  state.pickedCases.clear(); state.batch = [];
  if (batchTarget().map(c => c.name).join('|') !== 'a|b') throw new Error('nothing ticked runs every case of the session');
  state.pickedCases.add('/c/a');
  if (batchTarget().map(c => c.name).join('|') !== 'a') throw new Error('an unticked case must not run');
  if (currentCases().map(c => c.name).join('|') !== 'a|b') throw new Error('the unticked case stays listed');
  const runner = await import('node:fs').then(fs => fs.readFileSync(new URL('../dist/app/runner.js', import.meta.url), 'utf8'));
  if (!/!state\.createdCases\.has\(state\.createdBySite\.get\(s\.site_file\)\)/.test(runner)) {
    throw new Error('running only builds sites that have no case yet');
  }
}
