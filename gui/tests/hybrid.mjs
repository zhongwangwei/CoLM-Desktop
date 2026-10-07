// AI 参数化（hybrid.js）的纯函数：插槽、输出校验、权重数与 Study spec 的 `hybrid` 段。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-hybrid-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
// 没有页面：模块加载时找不到卡片就不接线。
globalThis.window = {};
globalThis.document = { getElementById: () => null, documentElement: { lang: 'zh' } };
globalThis.addEventListener = () => {};
const hybrid = await import(pathToFileURL(join(temp, 'app', 'hybrid.js')).href);

assert.equal(hybrid.slotForMode('lct'), 'land_class');
assert.equal(hybrid.slotForMode('pft'), 'pft');
assert.equal(hybrid.slotForMode('pc'), 'pft');
assert.equal(hybrid.outputPrefix('land_class'), 'DEF_LC_');
assert.equal(hybrid.slotDefaults('pft').output.name, 'DEF_PFT_VMAX25');

assert.deepEqual(hybrid.parseFeatures(' pftclass，pftfrac  porsl[1], '), ['pftclass', 'pftfrac', 'porsl[1]']);

assert.deepEqual(hybrid.checkOutput('pft', { name: 'def_pft_vmax25', lo: '10', hi: '80', transform: 'sigmoid' }),
  { name: 'DEF_PFT_VMAX25', lo: 10, hi: 80, transform: 'sigmoid' });
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_LC_VMAX25', lo: 1, hi: 2 }), /DEF_PFT_/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_', lo: 1, hi: 2 }), /DEF_PFT_/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_VMAX25', lo: 5, hi: 5 }), /下限小于上限/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_VMAX25', lo: '', hi: 5 }), /下限小于上限/);
assert.equal(hybrid.outputArg({ name: 'DEF_PFT_VMAX25', lo: 10, hi: 80, transform: 'clamp' }), 'DEF_PFT_VMAX25:10:80:clamp');

// 与 colm-hybrid 的 Mlp::parameter_count、Study 的 weight_count 一致（2→3→1 共 13 个）。
assert.equal(hybrid.weightCount(2, [], 1), 3);
assert.equal(hybrid.weightCount(2, [3], 1), 13);
assert.equal(hybrid.weightCount(3, [4], 2), 26);

const section = hybrid.studySection({
  slot: 'pft', features: 'pftclass, pftfrac', size: 'h4',
  outputs: [{ name: 'DEF_PFT_VMAX25', lo: '20', hi: '80', transform: 'sigmoid' }],
});
assert.deepEqual(section, {
  slot: 'pft', features: ['pftclass', 'pftfrac'], hidden: [4],
  outputs: [{ name: 'DEF_PFT_VMAX25', range: [20, 80], transform: 'sigmoid' }],
});
assert.throws(() => hybrid.studySection({ slot: 'pft', features: '', size: 'linear', outputs: section.outputs }), /输入特征/);
assert.throws(() => hybrid.studySection({ slot: 'pft', features: 'a', size: 'linear', outputs: [] }), /输出参数/);
assert.throws(() => hybrid.studySection({ slot: 'pft', features: 'a', size: 'linear',
  outputs: [{ name: 'DEF_PFT_VMAX25', lo: 1, hi: 2, transform: 'identity' }] }), /S 形映射或截断/);
assert.throws(() => hybrid.studySection({ slot: 'pft', features: 'a', size: 'linear',
  outputs: [{ name: 'DEF_PFT_VMAX25', lo: 1, hi: 2 }, { name: 'def_pft_vmax25', lo: 1, hi: 3 }] }), /不能重复/);

// 页面接线：卡片与表单里用到的元素都在 index.html 里。
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
const source = await readFile(join(root, 'dist', 'app', 'hybrid.js'), 'utf8');
for (const [, id] of source.matchAll(/\$\('([\w-]+)'\)/g)) {
  if (['kernel', 'cases-run', 'model-engine'].includes(id)) continue;
  assert.ok(html.includes(`id="${id}"`), `index.html has no #${id}`);
}
console.log('hybrid: slots, outputs, weight counts, study section and page wiring ok');
