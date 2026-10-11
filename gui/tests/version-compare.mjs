// 比较两个版本（version-compare.js）的纯函数：比较表、结论、研究固定的版本。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-version-compare-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = async () => ({}); export const listen = async () => {}; export const hasBackend = false;',
  'ui.js': 'export const $ = () => null; export const status = () => {};',
  'results.js': 'export const activeTuningStudies = () => []; export const refreshTuningStudy = async () => {}; export const studyVersionArg = async () => null;',
  'remote-study.js': 'export const awaitServerStudy = async () => {}; export const studyTarget = () => "local"; export const submitStudies = async () => {};',
})) await writeFile(join(temp, 'app', name), body);
globalThis.addEventListener = () => {};
const compare = await import(pathToFileURL(join(temp, 'app', 'version-compare.js')).href);

const result = {
  a: { version: 'official', status: 'completed', best_member: 'm3', calibration: 0.31, validation: 0.5, baseline_validation: 0.7 },
  b: { version: 'snow', status: 'completed', best_member: 'm7', calibration: 0.35, validation: 0.42, baseline_calibration: 0.6 },
  same_design: true, period: 'validation', better: 'b', notes: [],
};
assert.deepEqual(compare.compareRows(result).map(r => [r.key, r.version, r.best, r.calibration, r.validation, r.baseline]), [
  ['a', '官方版', 'm3', '0.3100', '0.5000', '0.7000'],
  ['b', '修改版：snow', 'm7', '0.3500', '0.4200', '0.6000'],
]);
assert.equal(compare.compareVerdict(result), '在验证期上，修改版：snow更好（目标值更低）。');
assert.equal(compare.compareVerdict({ ...result, better: 'a', period: 'calibration' }), '在率定期上，官方版更好（目标值更低）。');
assert.equal(compare.compareVerdict({ ...result, better: 'tie' }), '在验证期上，两个版本一样好。');
assert.match(compare.compareVerdict({ ...result, same_design: false }), /不是公平比较/);
assert.match(compare.compareVerdict({ ...result, better: null }), /比不出来/);
assert.equal(compare.compareRows({ a: {}, b: {} })[0].validation, '—');

// 研究固定的版本：固定过的用固定的；用官方版跑过的就是官方版；还没跑过的可以选。
assert.equal(compare.fixedVersion({ model_version: { name: 'snow' } }), 'snow');
assert.equal(compare.fixedVersion({ state: { tasks: { x: { status: 'succeeded' } } } }), 'official');
assert.equal(compare.fixedVersion({ state: { tasks: { x: { status: 'materialized' } } } }), null);
assert.equal(compare.fixedVersion({}), null);

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
for (const id of ['version-compare-card', 'compare-version-a', 'compare-version-b', 'compare-versions-run', 'compare-versions-result']) {
  assert.ok(html.includes(`id="${id}"`), id);
}
assert.match(await readFile(join(root, 'dist', 'app', 'main.js'), 'utf8'), /import '\.\/version-compare\.js';/);
console.log('version-compare: rows, verdicts, fixed versions and wiring ok');
