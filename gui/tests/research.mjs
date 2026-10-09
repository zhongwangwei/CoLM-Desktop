// 研究（research.js）的纯函数：按评估指标推荐、准备情况；以及左栏状态（study-model.js）与 hybrid 的模式过滤。
import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-research-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
globalThis.window = {};
globalThis.document = { getElementById: () => null, querySelectorAll: () => [], documentElement: { lang: 'zh' } };
globalThis.addEventListener = () => {};
const url = name => pathToFileURL(join(temp, 'app', name)).href;
const research = await import(url('research.js'));
const { studyBadge } = await import(url('study-model.js'));
const hybrid = await import(url('hybrid.js'));

// 推荐：最差 NSE 低于 0.5 推荐率定，否则推荐不确定性分析；没有 NSE 时不推荐。
assert.equal(research.recommend([]), null);
assert.equal(research.recommend([{ name: 'Qle', nse: null }, { name: 'Qh', nse: Number.NaN }]), null);
{
  const pick = research.recommend([{ name: 'Qh', nse: 0.63 }, { name: 'Qle', nse: 0.41 }, { name: 'GPP', nse: 0.55 }]);
  assert.equal(pick.study, 'tuning');
  assert.equal(pick.row.name, 'Qle');
}
assert.equal(research.recommend([{ name: 'Qh', nse: 0.8 }, { name: 'Qle', nse: 0.7 }]).study, 'uq');
assert.equal(research.recommend([{ name: 'Qle', nse: 0.5 }]).study, 'uq', '门槛本身不算偏低');

// 四条研究、两组，入口步骤与左栏一致。
assert.deepEqual(research.STUDIES.map(s => [s.id, s.group, s.step]), [
  ['tuning', 'classic', 'result-tuning'], ['uq', 'classic', 'result-uncertainty'],
  ['learn', 'ai', 'hybrid-learn'], ['process', 'ai', 'hybrid-process'],
]);

const base = { caseName: 'SP1', spatial: false, missingObs: 0, engine: 'rust', processBlock: '' };
const levels = items => items.map(item => item.level);

// 参数率定：没有算例、空间算例、缺观测都挡路；不确定性分析缺观测只是提醒。
assert.ok(research.blocked(research.readiness('tuning', { ...base, caseName: '' })));
assert.equal(research.readiness('tuning', { ...base, caseName: '' })[0].step, 'basic-files');
assert.ok(research.blocked(research.readiness('tuning', { ...base, spatial: true })));
assert.ok(research.blocked(research.readiness('tuning', { ...base, missingObs: 2 })));
assert.ok(!research.blocked(research.readiness('uq', { ...base, missingObs: 2 })));
assert.ok(levels(research.readiness('uq', { ...base, missingObs: 2 })).includes('warn'));
assert.ok(!research.blocked(research.readiness('tuning', base)));

// AI：学习参数没有算例只是提醒（两步法只要项目目录）；替换过程要算例且 β 插槽可用；Fortran 引擎是提醒。
assert.ok(!research.blocked(research.readiness('learn', { ...base, caseName: '' })));
assert.ok(research.blocked(research.readiness('process', { ...base, caseName: '' })));
assert.ok(research.blocked(research.readiness('process', { ...base, processBlock: '要先关闭植物水力（DEF_USE_PLANTHYDRAULICS）' })));
assert.ok(!research.blocked(research.readiness('process', base)));
{
  const items = research.readiness('learn', { ...base, engine: 'fortran' });
  assert.ok(!research.blocked(items));
  assert.equal(items.find(item => item.level === 'warn').step, 'run');
}

// 左栏状态。
assert.equal(studyBadge(null), null);
assert.equal(studyBadge({ status: 'Draft', total: 0 }), null);
assert.equal(studyBadge({ status: 'Draft', total: 41 }), '任务已生成');
assert.equal(studyBadge({ status: 'Running', done: 12, total: 41 }), '运行中 12/41');
assert.equal(studyBadge({ status: 'Completed', done: 41, total: 41 }), '完成');
assert.equal(studyBadge({ status: 'CompletedWithFailures', done: 41, total: 41 }), '完成（有失败）');

// hybrid 的两种模式：插槽、预设、训练方法各自过滤。
assert.equal(hybrid.modeForStep('hybrid-learn'), 'params');
assert.equal(hybrid.modeForStep('hybrid-process'), 'process');
assert.deepEqual(hybrid.MODE_SLOTS.params, ['pft', 'land_class']);
assert.deepEqual(hybrid.MODE_SLOTS.process, [hybrid.SOIL_STRESS]);
assert.deepEqual(hybrid.presetsForMode('params').map(p => p.id), ['vcmax-pft', 'vcmax-lc', 'custom']);
assert.deepEqual(hybrid.presetsForMode('process').map(p => p.id), ['beta', 'custom']);
assert.deepEqual(hybrid.methodsForMode('params'), ['de', 'two-step']);
assert.deepEqual(hybrid.methodsForMode('process'), ['de', 'gradient']);

console.log('research: recommendation, readiness, study badges and hybrid modes');
