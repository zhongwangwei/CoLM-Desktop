// AI 模型（hybrid.js）的纯函数：插槽、输出校验、权重数与 Study spec 的 `hybrid` 段。
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
  { name: 'DEF_PFT_VMAX25', lo: 10, hi: 80, transform: 'sigmoid', relative: false });
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_LC_VMAX25', lo: 1, hi: 2 }), /DEF_PFT_/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_', lo: 1, hi: 2 }), /DEF_PFT_/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_VMAX25', lo: 5, hi: 5 }), /下限小于上限/);
assert.throws(() => hybrid.checkOutput('pft', { name: 'DEF_PFT_VMAX25', lo: '', hi: 5 }), /下限小于上限/);
assert.equal(hybrid.outputArg({ name: 'DEF_PFT_VMAX25', lo: 10, hi: 80, transform: 'clamp' }), 'DEF_PFT_VMAX25:10:80:clamp');
assert.equal(hybrid.outputArg({ name: 'DEF_PFT_VMAX25', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true }), 'DEF_PFT_VMAX25:0.5:2:sigmoid:relative');
assert.deepEqual(hybrid.studySection({ slot: 'pft', features: 'clim_tair', size: 'linear', outputs: [{ name: 'DEF_PFT_VMAX25', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true }] }).outputs, [{ name: 'DEF_PFT_VMAX25', range: [0.5, 2], transform: 'sigmoid', relative: true }]);

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
// 预设：填出来的设置本身要能通过 Study 段的校验；β 在 H2 之前不可选。
for (const preset of hybrid.PRESETS.filter(p => p.outputs)) {
  const section = hybrid.studySection({ slot: preset.slot, features: preset.features, outputs: preset.outputs, size: preset.size });
  assert.ok(section.outputs.every(o => o.relative), preset.id);
}
// β 预设（过程插槽 soil_stress）：PC 模式或开着植物水力时不能用。
const beta = hybrid.PRESETS.find(p => p.id === 'beta');
assert.equal(beta.slot, 'soil_stress');
assert.ok(!beta.disabled);
assert.equal(hybrid.presetBlocked(beta, { land_mode: 'lct', plant_hydraulics: false }), '');
assert.match(hybrid.presetBlocked(beta, { land_mode: 'pc', plant_hydraulics: true }), /PC/);
assert.match(hybrid.presetBlocked(beta, { land_mode: 'pft', plant_hydraulics: true }), /PLANTHYDRAULICS/);
assert.equal(hybrid.presetBlocked(beta, undefined), '');
assert.equal(hybrid.presetBlocked(hybrid.PRESETS[0], { land_mode: 'pc', plant_hydraulics: true }), '');
assert.deepEqual(hybrid.checkOutput('soil_stress', { name: 'BETA', lo: '0.5', hi: '2', transform: 'sigmoid', relative: true }),
  { name: 'beta', lo: 0.5, hi: 2, transform: 'sigmoid', relative: true });
assert.throws(() => hybrid.checkOutput('soil_stress', { name: 'DEF_PFT_VMAX25', lo: 0, hi: 1 }), /beta/);
const stressSection = hybrid.studySection({ slot: 'soil_stress', features: hybrid.STRESS_FEATURES, outputs: beta.outputs, size: 'linear' });
assert.deepEqual(stressSection.features, ['beta_physics', 'root_saturation', 'root_temperature', 'frozen_root_fraction']);
assert.deepEqual(stressSection.outputs, [{ name: 'beta', range: [0.5, 2], transform: 'sigmoid', relative: true }]);
assert.equal(hybrid.slotDefaults('soil_stress').output.name, 'beta');
assert.equal(hybrid.presetForMode('lct'), 'vcmax-lc');
assert.equal(hybrid.presetForMode('pc'), 'vcmax-pft');

// 两步法的第一步：哪些率定任务能用。
const study = { status: 'completed', trains_network: false, parameters: ['DEF_PFT_VMAX25'], best_member: 'm000029', sites: ['CA-SF3'] };
assert.deepEqual(hybrid.studyUsable(study, ['DEF_PFT_VMAX25']), { usable: true, reason: '' });
assert.equal(hybrid.studyUsable({ ...study, status: 'running' }, ['DEF_PFT_VMAX25']).usable, false);
assert.equal(hybrid.studyUsable({ ...study, trains_network: true }, ['DEF_PFT_VMAX25']).usable, false);
assert.match(hybrid.studyUsable(study, ['DEF_PFT_VMAX25', 'DEF_PFT_SLA']).reason, /DEF_PFT_SLA/);
assert.equal(hybrid.studyUsable({ ...study, status: 'completed_with_failures' }, ['DEF_PFT_VMAX25']).usable, true);

// 拟合报告的摘要（第 617 轮的真实数值）：网络 0.483 不低于基准 0.443，不通过。
const failed = hybrid.fitSummary({
  rows: 4300,
  held_out_rmse: [{ study: '/s/1', rmse: [0.40], mean_predictor_rmse: [0.45] }, { study: '/s/2', rmse: [0.6], mean_predictor_rmse: [0.5] }],
  validation: { network_rmse: [0.483], mean_predictor_rmse: [0.443], passed: false },
});
assert.equal(failed.passed, false);
assert.deepEqual(failed.rows.map(r => r.better), [true, false]);
assert.equal(failed.network, 0.483);
assert.equal(failed.samples, 4300);
const single = hybrid.fitSummary({ rows: 10, held_out_rmse: [], validation: null });
assert.equal(single.passed, null);
assert.equal(hybrid.fitSummary({ held_out_rmse: [], validation: { network_rmse: [0.3, 0.2], mean_predictor_rmse: [0.4, 0.5], passed: true } }).network, 0.25);

console.log('hybrid: slots, outputs, weight counts, study section, presets, two-step fitting and page wiring ok');
