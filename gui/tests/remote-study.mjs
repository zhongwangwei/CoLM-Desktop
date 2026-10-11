// 服务器上的研究（remote-study.js）：状态文字、进度文字、提交参数、等到取回。
import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-remote-study-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
for (const [name, body] of Object.entries({
  'ipc.js': 'export const invoke = async (...a) => globalThis.fakeInvoke(...a); export const listen = async () => {}; export const hasBackend = true;',
  'ui.js': 'export const $ = id => globalThis.fakeNodes?.[id] ?? null; export const status = () => {};',
})) await writeFile(join(temp, 'app', name), body);
const remote = await import(pathToFileURL(join(temp, 'app', 'remote-study.js')).href);

assert.equal(remote.serverStudyText(null), '');
assert.match(remote.serverStudyText({ host: 'c1', job: 'j1' }), /在服务器 c1 上运行（作业 j1）/);
assert.match(remote.serverStudyText({ host: 'c1', job: 'j1', fetched_at: 5 }), /跑完，结果已取回/);
assert.match(remote.serverProgressText({ status: { state: 'running' }, study: { completed_candidates: 3 } }), /已完成 3 个候选/);
assert.match(remote.serverProgressText({ status: { state: 'running' } }), /正在准备/);
assert.match(remote.serverProgressText({ status: { state: 'finished', exit_code: 2 } }), /失败（退出码 2）/);
assert.match(remote.serverProgressText({ status: { state: 'lost' } }), /不在了/);

globalThis.fakeNodes = { 'run-target': { value: 'c1' } };
assert.equal(remote.studyTarget(), 'c1');
globalThis.fakeNodes = {};
assert.equal(remote.studyTarget(), 'local');

const calls = [];
globalThis.fakeInvoke = async (command, args) => { calls.push([command, args]); return {}; };
await remote.submitStudies(['/s/a', '/s/b'], { host: 'c1', kernel: '/k', engine: 'rust', jobs: 2, versionFor: async dir => (dir === '/s/a' ? 'snow' : null) });
assert.deepEqual(calls, [
  ['remote_study_run', { studyDir: '/s/a', host: 'c1', kernel: '/k', engine: 'rust', jobs: 2, modelVersion: 'snow' }],
  ['remote_study_run', { studyDir: '/s/b', host: 'c1', kernel: '/k', engine: 'rust', jobs: 2, modelVersion: null }],
]);

// 等到跑完：先运行中，再结束（退出码 0）→ 取回。
let polls = 0;
const seen = [];
globalThis.fakeInvoke = async command => {
  seen.push(command);
  if (command === 'remote_study_status') return ++polls < 2 ? { status: { state: 'running' } } : { status: { state: 'finished', exit_code: 0 } };
  return {};
};
await remote.awaitServerStudy('/s/a', { pollMs: 1 });
assert.deepEqual(seen, ['remote_study_status', 'remote_study_status', 'remote_study_fetch']);
globalThis.fakeInvoke = async command => (command === 'remote_study_status' ? { status: { state: 'finished', exit_code: 3 } } : {});
await assert.rejects(remote.awaitServerStudy('/s/a', { pollMs: 1 }), /退出码 3/);
console.log('remote-study: texts, target, submission and waiting for fetch ok');
