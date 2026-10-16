import { test } from 'node:test';
import assert from 'node:assert/strict';
import { applyRunEvent } from './run-events';

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
