import test from 'node:test';
import assert from 'node:assert/strict';
import {
  newServer,
  replaceServer,
  remoteCheckRequest,
  localRunEntries,
  studyResourcePermissions,
  resourceCheckSummary,
  resourceTaskInfo,
  type ResourceCheck,
} from './resource';
test('server edits preserve others and reject duplicate host', () => {
  const a = { ...newServer(), host: 'a' },
    b = { ...newServer(), host: 'b' };
  assert.equal(replaceServer([a, b], 'a', { ...a, root: '/data' })[1], b);
  assert.throws(() => replaceServer([a, b], 'a', { ...a, host: 'b' }));
  assert.equal('password' in newServer(), false);
});

test('remote verification requires actual case and uses explicit test kind', () => {
  assert.throws(() => remoteCheckRequest('verify', 'default', '', 'colm-core', 'refactor'));
  assert.deepEqual(remoteCheckRequest('test', 'default', '', 'colm-core', 'refactor'), {
    action: 'test',
    preset: 'default',
    kind: 'cargo',
    package: 'colm-core',
  });
});

test('saving a removed server does not report a silent success', () => {
  assert.throws(() => replaceServer([], 'a', { ...newServer(), host: 'a' }));
});

test('local cancellation excludes server and unverified task locations', () => {
  const run = { case: '/case', status: 'submitted', running: true, logs: [], stages: {} };
  assert.deepEqual(
    localRunEntries({
      local: { ...run, location: 'local' },
      remote: { ...run, location: 'remote' },
      unknown: run,
    }).map(([dir]) => dir),
    ['local'],
  );
});

test('pending server studies remain visible with only read-only reconciliation', () => {
  const pending = { remote: null, remote_pending: { host: 'server', job: 'j1' } };
  const unresolved = studyResourcePermissions(pending);
  assert.equal(unresolved.exists, true);
  assert.equal(unresolved.pending, true);
  assert.equal(unresolved.cancel, false);
  assert.equal(unresolved.fetch, false);
  const recovered = studyResourcePermissions(pending, {
    record: { host: 'server', job: 'j1' },
    status: { state: 'running' },
  });
  assert.equal(recovered.pending, false);
  assert.equal(recovered.cancel, true);
  assert.equal(recovered.fetch, false);
  assert.equal(
    studyResourcePermissions(pending, {
      record: { host: 'server', job: 'j1' },
      status: { state: 'finished', exit_code: 0 },
    }).fetch,
    true,
  );
  assert.equal(studyResourcePermissions({ remote: null, remote_pending: null }).exists, false);
});

test('resource checks require current configuration and separate SSH, engine snapshots and path evidence', () => {
  const server = { ...newServer(), host: 'a', maps: [{ local: '/local', remote: '/remote' }] };
  assert.deepEqual(
    Object.values(resourceCheckSummary(server))
      .slice(0, 3)
      .map((item) => (item as { state: string }).state),
    ['pending', 'pending', 'pending'],
  );
  const check: ResourceCheck = {
    configuration: JSON.stringify(server),
    checked_at: '2026-10-12T05:00:00Z',
    probe: {
      hostname: 'a',
      maps: [{ local: '/local', remote: '/remote', local_exists: true, remote_exists: true }],
    },
    engines: {
      engines: [
        { id: 'one', ready: true },
        { id: 'two', ready: false },
      ],
    },
  };
  const summary = resourceCheckSummary(server, check);
  assert.equal(summary.ssh.state, 'success');
  assert.equal(summary.engine.label, '1 个可用快照');
  assert.equal(summary.mapping.label, '对应路径存在');
  assert.equal(summary.checkedAt, check.checked_at);
  assert.equal(
    resourceCheckSummary({ ...server, username: 'another' }, check).checkedAt,
    null,
    'edited credentials invalidate earlier evidence',
  );
  assert.equal(
    resourceCheckSummary({ ...server, maps: [] }, check).ssh.state,
    'pending',
    'edited mappings invalidate earlier evidence',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: undefined }).engine.state,
    'pending',
    'probe supply strategy is never a ready engine',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: {} }).engine.state,
    'pending',
    'a missing inventory is not proof of no available engine',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: { engines: [] } }).engine.state,
    'warning',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe: { hostname: 'a', maps: [] } }).mapping.state,
    'warning',
  );
  assert.equal(
    resourceCheckSummary(server, {
      ...check,
      probe: {
        hostname: 'a',
        maps: [{ local: '/different', remote: '/remote', local_exists: true, remote_exists: true }],
      },
    }).mapping.state,
    'warning',
    'counts cannot substitute for matching path identities',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe_error: 'ssh failed' }).ssh.state,
    'error',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engine_error: 'inventory failed' }).engine.state,
    'error',
  );
  assert.equal(resourceCheckSummary(server, { ...check, checked_at: 'invalid' }).checkedAt, null);
});

test('resource task summaries retain pending host identity and require actual remote completion evidence', () => {
  const pending = {
    remote: null,
    remote_pending: { host: 'a', job: 'unconfirmed' },
    state: 'completed',
  };
  assert.deepEqual(resourceTaskInfo(pending), {
    host: 'a',
    job: 'unconfirmed',
    status: 'unknown',
    pending: true,
  });
  assert.deepEqual(resourceTaskInfo({ host: 'a', job: 'one' }), {
    host: 'a',
    job: 'one',
    status: 'unknown',
    pending: false,
  });
  assert.equal(
    resourceTaskInfo({ remote: { host: 'a', job: 'one' }, state: 'completed' }).status,
    'unknown',
    'local study state is not remote success',
  );
  const running = { record: { host: 'a', job: 'one' }, status: { state: 'running' } };
  assert.deepEqual(resourceTaskInfo(pending, running), {
    host: 'a',
    job: 'one',
    status: 'running',
    pending: false,
  });
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished' } }).status,
    'unknown',
  );
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished', exit_code: 0 } }).status,
    'succeeded',
  );
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished', exit_code: 2 } }).status,
    'failed',
  );
  assert.deepEqual(
    resourceTaskInfo(pending, {
      record: null,
      pending: { host: 'b', job: 'new' },
      status: { state: 'finished', exit_code: 0 },
    }),
    { host: 'b', job: 'new', status: 'unknown', pending: true },
  );
  assert.equal(
    resourceTaskInfo(pending, {
      cancellation: { request_accepted: true },
      reconciled_status: running,
    }).status,
    'running',
    'cancellation acceptance is not a stopped job',
  );
  assert.equal(resourceTaskInfo({ error: 'record read failed' }).host, null);
  assert.equal(
    resourceTaskInfo({ record: null, status: { state: 'finished', exit_code: 0 } }).status,
    'unknown',
    'completion also requires a recorded host and job',
  );
});
