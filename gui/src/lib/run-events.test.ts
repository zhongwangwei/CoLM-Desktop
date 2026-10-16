import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyRunEvent } from './run-events';
import type { RunState } from './types';

test('only the station that starts leaves the local waiting queue', () => {
  const queued = (dir: string): RunState => ({
    case: dir,
    run_id: 'batch',
    status: 'queued',
    location: 'local',
    running: true,
    logs: [],
    stages: {},
  });
  const waiting = queued('/b');
  const active = applyRunEvent(queued('/a'), 'run://started', { case: '/a', run_id: 'batch' })!;
  assert.equal(active.status, 'running');
  const stage = applyRunEvent(active, 'run://stage', {
    case: '/a',
    run_id: 'batch',
    stage: 'mkinidata',
    state: 'begin',
  })!;
  assert.equal(stage.status, 'running');
  assert.equal(
    stage.progress?.stage,
    'mkinidata',
    'stages are visible before numeric progress starts',
  );
  const progress = applyRunEvent(stage, 'run://progress', {
    case: '/a',
    run_id: 'batch',
    stage: 'colm',
    step: 17,
    total_steps: 100,
  })!;
  assert.equal(progress.status, 'running');
  assert.equal(progress.progress?.step, 17);
  assert.equal(progress.progress?.stage, 'colm');
  assert.equal(waiting.status, 'queued');
  assert.equal(waiting.progress, undefined);
});

test('late activity never revives a terminal run or confirms an uncertain remote job', () => {
  const finished: RunState = {
    case: '/a',
    run_id: 'run',
    location: 'local',
    status: 'succeeded',
    running: false,
    logs: [],
    stages: {},
  };
  for (const event of ['run://stage', 'run://progress']) {
    const next = applyRunEvent(finished, event, {
      case: '/a',
      run_id: 'run',
      stage: 'colm',
      step: 10,
    })!;
    assert.equal(next.running, false);
    assert.equal(next.status, 'succeeded');
  }
  const unknown = { ...finished, location: 'remote' as const, status: 'unknown', running: true };
  for (const location of ['remote', undefined]) {
    const next = applyRunEvent(unknown, 'run://progress', {
      case: '/a',
      run_id: 'run',
      location,
      stage: 'colm',
      step: 10,
    })!;
    assert.equal(next.status, 'unknown');
    assert.equal(next.location, 'remote');
  }
});

test('completion retains its requested stage', () => {
  const run = {
    case: '/case',
    run_id: 'run',
    status: 'running',
    running: true,
    logs: [],
    stages: {},
  };
  const done = applyRunEvent(run, 'run://done', {
    case: '/case',
    run_id: 'run',
    requested_stage: 'mksrfdata',
    code: 0,
  })!;
  assert.equal(done.requested_stage, 'mksrfdata');
  assert.equal(done.status, 'succeeded');
});

test('source case retains its run while pages change, and old events cannot finish its replacement', () => {
  const first = applyRunEvent(undefined, 'run://started', {
    case: '/source',
    run_case: '/copy',
    run_id: 'one',
  })!;
  const logs = applyRunEvent(first, 'run://lines', {
    case: '/source',
    run_id: 'one',
    lines: ['step 1'],
  })!;
  assert.deepEqual(logs.logs, ['step 1']);
  const replacement = { ...logs, run_id: 'two', running: true };
  assert.equal(
    applyRunEvent(replacement, 'run://done', { case: '/source', run_id: 'one', code: 0 }),
    null,
  );
  const finished = applyRunEvent(replacement, 'run://done', {
    case: '/source',
    run_id: 'two',
    code: 0,
  })!;
  assert.equal(finished.status, 'succeeded');
  assert.equal(applyRunEvent(finished, 'run://started', { case: '/source', run_id: 'two' }), null);
  const third = applyRunEvent(finished, 'run://started', { case: '/source', run_id: 'three' })!;
  assert.equal(third.running, true);
  assert.deepEqual(third.logs, []);
  assert.equal(
    applyRunEvent(third, 'run://lines', { case: '/source', run_id: 'two', lines: ['old'] }),
    null,
  );
});

test('remote start records its operation without switching location to local', () => {
  const run = applyRunEvent(undefined, 'run://started', {
    source_case: '/case',
    run_id: 'remote',
    operation_id: 'operation',
    location: 'remote',
  });
  assert.equal(run?.location, 'remote');
  assert.equal(run?.operation_id, 'operation');
});
