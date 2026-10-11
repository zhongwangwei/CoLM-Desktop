import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import assert from 'node:assert/strict';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-run-mode-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;

const { state } = await import(moduleUrl('state.js'));
const {
  OFFICIAL, modelVersion, versionLabel, runModeBlocker, readinessRows, touchesText,
  contextChips, runModeSnapshot, provenanceChips, provenanceMismatch,
} = await import(moduleUrl('run-mode.js'));

// 没选过就是官方版。
state.modelVersion = undefined;
assert.equal(modelVersion(), OFFICIAL);
state.modelVersion = 'snow';
assert.equal(modelVersion(), 'snow');
assert.equal(versionLabel(OFFICIAL), '官方版');
assert.equal(versionLabel('snow'), '修改版：snow');

// 官方版不需要检查；修改版要检查通过、且只能在本机跑。
const ready = { name: 'snow', ready: true, items: [{ code: 'clean', ok: true, blocking: false, message: '源码干净' }] };
const notBuilt = {
  name: 'snow',
  ready: false,
  items: [
    { code: 'clean', ok: true, blocking: false, message: '源码干净' },
    { code: 'engine_built', ok: false, blocking: true, message: 'Rust 引擎还没编好' },
    { code: 'engine_mismatch', ok: false, blocking: false, message: '只改了 Rust 引擎' },
  ],
};
assert.equal(runModeBlocker({ version: OFFICIAL, target: 'srv', readiness: null }), null);
assert.equal(runModeBlocker({ version: 'snow', target: 'local', readiness: ready }), null);
assert.match(runModeBlocker({ version: 'snow', target: 'srv', readiness: ready }), /只能在本机运行/);
assert.match(runModeBlocker({ version: 'snow', target: 'local', readiness: null }), /还在检查/);
// 上一个版本的检查结果不能拿来放行这一个。
assert.match(runModeBlocker({ version: 'snow', target: 'local', readiness: { ...ready, name: 'other' } }), /还在检查/);
assert.match(runModeBlocker({ version: 'snow', target: 'local', readiness: notBuilt }), /还不能运行：Rust 引擎还没编好$/);

assert.deepEqual(readinessRows(notBuilt).map(r => r.kind), ['ok', 'block', 'warn']);
assert.equal(touchesText({ engine: true, kernel: true }), '改了 Rust 引擎和 Fortran 内核');
assert.equal(touchesText({ engine: false, kernel: true }), '改了 Fortran 内核');
assert.equal(touchesText({ engine: false, kernel: false }), '没改会影响计算结果的代码');

// 上下文条：算例 · 版本 · 位置 · 引擎。
assert.deepEqual(
  contextChips({ caseName: 'CN-Cng', cases: 3, version: 'snow', target: 'srv', engine: 'fortran' }).map(c => c.value),
  ['CN-Cng 等 3 个', '修改版：snow', '服务器：srv', 'Fortran 内核'],
);
assert.deepEqual(
  contextChips({ version: OFFICIAL, engine: 'rust' }).map(c => c.value),
  ['还没有算例', '官方版', '本机', 'Rust 引擎'],
);

// 给助手的快照：检查清单只在它属于当前版本时附上。
const snap = runModeSnapshot({ version: 'snow', target: 'local', engine: 'rust', readiness: notBuilt });
assert.equal(snap.ready, false);
assert.equal(snap.readiness.length, 3);
assert.deepEqual(runModeSnapshot({ version: OFFICIAL, readiness: notBuilt }).readiness, []);
assert.equal(runModeSnapshot({}).ready, true);

// 出处标签。
assert.deepEqual(provenanceChips(null), []);
const official = {
  ok: true, engine: 'rust', preprocessors: 'rust', location: { kind: 'local' },
  version: { kind: 'official', colm_cli: '0.3.0' }, kernel: { identity: 'default@f59ffdce', dir: '/k' }, finished: 1,
};
assert.deepEqual(provenanceChips(official).map(c => c.key), ['version', 'engine', 'target', 'kernel', 'time']);
assert.equal(provenanceChips(official)[0].text, '官方版');
const modified = {
  ...official, ok: false, error: 'stage colm failed', engine: 'fortran',
  version: { kind: 'workspace', name: 'snow', head: '0123456789abcdef', base_commit: 'fedcba9876543210' },
};
const chips = provenanceChips(modified);
assert.equal(chips[0].text, '修改版：snow');
assert.equal(chips[0].title, '提交 01234567，基于 fedcba98');
assert.ok(chips[0].accent);
assert.equal(chips[1].text, 'Fortran 内核');
assert.ok(chips.find(c => c.key === 'failed')?.warn);
assert.equal(provenanceChips({ ...official, location: { kind: 'server' } }).find(c => c.key === 'target').text, '服务器');
assert.equal(provenanceChips(official).find(c => c.key === 'target').text, '本机');

assert.equal(provenanceMismatch(official, { version: OFFICIAL, engine: 'rust' }), null);
assert.equal(provenanceMismatch(official, { version: 'snow', engine: 'rust' }), '这批结果是用官方版跑的，当前选的是修改版：snow');
assert.equal(provenanceMismatch(modified, { version: 'snow', engine: 'rust' }), '这批结果是用Fortran 内核跑的，当前选的是Rust 引擎');
assert.equal(provenanceMismatch({}, { version: 'snow' }), null);

console.log('run-mode: blockers, context chips, assistant snapshot and provenance chips');
