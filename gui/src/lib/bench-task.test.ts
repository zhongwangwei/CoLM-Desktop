import assert from 'node:assert/strict';
import test from 'node:test';
import {
  beginBenchTask,
  clearBenchTask,
  finishBenchTask,
  markInterruptedBenchTasks,
  parseBenchTasks,
  serializeBenchTasks,
  updateBenchTask,
  visibleBenchTasks,
  type BenchTask,
} from './bench-task';

const input = {
  host: '',
  project: 'comparison',
  label: 'comparison',
  kind: 'run' as const,
  phase: ['提交运行', 'Submitting'] as [string, string],
};

test('begin adds a running task and supersedes an unfinished one for the same project', () => {
  const first = beginBenchTask([], { ...input, id: 'a' }, 1000);
  assert.equal(first.id, 'a');
  assert.equal(first.tasks[0]?.state, 'running');
  const second = beginBenchTask(first.tasks, { ...input, id: 'b' }, 2000);
  assert.equal(second.tasks.length, 2);
  assert.equal(second.tasks[0]?.id, 'b');
  assert.equal(second.tasks[1]?.state, 'interrupted');
  assert.equal(second.tasks[1]?.finishedAt, 2000);
  // 别的机器/项目的任务不受影响。
  const other = beginBenchTask(second.tasks, { ...input, host: 'serverA', id: 'c' }, 3000);
  assert.equal(other.tasks.filter((task) => task.state === 'running').length, 2);
});

test('update/finish/clear only touch the requested task', () => {
  const started = beginBenchTask([], { ...input, id: 'a' }, 1000).tasks;
  const updated = updateBenchTask(started, 'a', {
    phase: ['服务器运行中', 'Running on the server'],
    job: 'bench-run-7',
  });
  assert.deepEqual(updated[0]?.phase, ['服务器运行中', 'Running on the server']);
  assert.equal(updated[0]?.job, 'bench-run-7');
  const done = finishBenchTask(updated, 'a', { state: 'done', report: '/ob/report.html' }, 5000);
  assert.equal(done[0]?.state, 'done');
  assert.equal(done[0]?.finishedAt, 5000);
  assert.equal(done[0]?.report, '/ob/report.html');
  const interrupted = finishBenchTask(
    updated,
    'a',
    { state: 'interrupted', error: 'compare page left' },
    5500,
  );
  assert.equal(interrupted[0]?.state, 'interrupted');
  const failed = finishBenchTask(updated, 'a', { state: 'failed', error: 'boom' }, 6000);
  assert.equal(failed[0]?.state, 'failed');
  assert.equal(failed[0]?.error, 'boom');
  assert.deepEqual(clearBenchTask(done, 'a'), []);
  assert.deepEqual(clearBenchTask(done, 'missing'), done);
});

test('visible tasks keep running ones and done ones only for the linger window', () => {
  const started = beginBenchTask([], { ...input, id: 'a' }, 1000).tasks;
  const done = finishBenchTask(started, 'a', { state: 'done' }, 2000);
  assert.equal(visibleBenchTasks(done, 2000 + 60_000).length, 1);
  assert.equal(visibleBenchTasks(done, 2000 + 6 * 60_000).length, 0);
  assert.equal(visibleBenchTasks(started, 2000 + 6 * 60_000).length, 1);
});

test('a restart marks unfinished tasks interrupted instead of pretending they still run', () => {
  const tasks = beginBenchTask([], { ...input, id: 'a' }, 1000).tasks;
  const restarted = markInterruptedBenchTasks(tasks, 9000);
  assert.equal(restarted[0]?.state, 'interrupted');
  assert.equal(restarted[0]?.finishedAt, 9000);
});

test('persisted tasks round-trip and unreadable rows are dropped one by one', () => {
  const task: BenchTask = {
    id: 'a',
    host: 'serverA',
    project: 'cmp',
    label: 'cmp',
    kind: 'run',
    startedAt: 1000,
    phase: ['排队中', 'Queued'],
    job: 'j1',
    state: 'running',
    report: null,
    error: null,
  };
  const parsed = parseBenchTasks(serializeBenchTasks([task]));
  assert.equal(parsed.length, 1);
  assert.equal(parsed[0]?.host, 'serverA');
  assert.deepEqual(parsed[0]?.phase, ['排队中', 'Queued']);
  assert.equal(parseBenchTasks('not json').length, 0);
  assert.equal(parseBenchTasks('{"a":1}').length, 0);
  const mixed = JSON.stringify([task, { id: 1 }, { ...task, kind: 'other' }]);
  assert.equal(parseBenchTasks(mixed).length, 1);
});
