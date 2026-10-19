import test, { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import type { RunState } from '../../lib/types';
import {
  remoteSubmissionBlocked,
  remoteRunPermissions,
  reconcileRemoteRun,
  matchesRemoteRun,
  compatibleKernel,
  restoreRunView,
  settleRuns,
  submittingHost,
  remoteEndStatus,
} from './run-model';
const run: RunState = {
  run_id: 'new',
  case: '/case/a',
  status: 'running',
  running: true,
  logs: ['step 1'],
  stages: { colm: 'begin' },
  progress: { stage: 'colm', step: 1, total_steps: 10 },
};
describe('persistent run state', () => {
  it('restores logs, identity and progress after page remount', () => {
    assert.deepEqual(restoreRunView(run, '/case/a'), {
      runId: 'new',
      casePath: '/case/a',
      status: 'running',
      lines: ['step 1'],
      stage: 'colm',
      step: 1,
      total: 10,
    });
  });
  it('does not let an old completion settle a newer run', () => {
    const records = { '/case/a': run };
    assert.equal(settleRuns(records, ['/case/a'], 'old', true)['/case/a'], run);
  });
  it('does not report cancellation as completion', () => {
    const cancelled = { ...run, running: false, status: 'cancelled' };
    assert.equal(
      settleRuns({ '/case/a': cancelled }, ['/case/a'], 'new', true)['/case/a'].status,
      'cancelled',
    );
  });
  it('settles only targets belonging to the same batch', () => {
    const other = { ...run, run_id: 'other' };
    const result = settleRuns(
      { '/case/a': run, '/case/b': other },
      ['/case/a', '/case/b'],
      'new',
      true,
    );
    assert.equal(result['/case/a'].running, false);
    assert.equal(result['/case/b'].running, true);
  });
  it('does not resubmit unknown remote jobs', () => {
    assert.equal(remoteSubmissionBlocked({ job: '123' }), true);
    assert.equal(remoteSubmissionBlocked({ status: { state: 'lost' } }), true);
    assert.equal(
      remoteSubmissionBlocked({
        host: 'server',
        job: '123',
        status: { state: 'finished', exit_code: 1 },
      }),
      false,
    );
    assert.equal(remoteSubmissionBlocked(null), false);
  });
  it('selects a compatible crop kernel instead of the first kernel', () => {
    const kernels = [
      { dir: '/default', preset: 'default', macros: ['SinglePoint', 'LULC_IGBP'] },
      { dir: '/crop', preset: 'crop', macros: ['SinglePoint', 'LULC_IGBP', 'CROP'] },
    ];
    assert.equal(
      compatibleKernel(kernels, {}, { DEF_TUNING_CROP_PLANTING_DAY: '120' })?.dir,
      '/crop',
    );
  });
});

it('pending run identity requires reconciliation before mutations or resubmission', () => {
  const pending = {
    host: 'server',
    job: 'new-job',
    pending: true,
    needs_reconciliation: true,
    status: { state: 'finished', exit_code: 0 },
  };
  assert.deepEqual(remoteRunPermissions(pending), {
    pending: true,
    terminal: false,
    cancel: false,
    fetch: false,
  });
  assert.equal(remoteSubmissionBlocked(pending), true);
  const unresolved = reconcileRemoteRun(pending, {
    record: null,
    pending: { job: 'new-job' },
    status: { state: 'unknown' },
  });
  assert.equal(remoteRunPermissions(unresolved).cancel, false);
  const confirmed = reconcileRemoteRun(unresolved, {
    record: { host: 'server', job: 'new-job' },
    pending: null,
    status: { state: 'running' },
  });
  assert.equal(remoteRunPermissions(confirmed).pending, false);
  assert.equal(remoteRunPermissions(confirmed).cancel, true);
  assert.equal(remoteRunPermissions({ ...confirmed, status: { state: 'finished' } }).fetch, false);
  assert.equal(
    remoteRunPermissions({ ...confirmed, status: { state: 'finished', exit_code: 0 } }).fetch,
    true,
  );
});

it('uses materialized model crop and land cover instead of stale original physics', () => {
  const kernels = [
    { dir: '/default', preset: 'default', macros: ['SinglePoint', 'LULC_IGBP'] },
    { dir: '/crop', preset: 'crop', macros: ['SinglePoint', 'LULC_IGBP', 'CROP'] },
    { dir: '/usgs', preset: 'usgs', macros: ['SinglePoint', 'LULC_USGS'] },
  ];
  assert.equal(
    compatibleKernel(
      kernels,
      { subgrid: 'USGS', crop: false },
      { DEF_TUNING_CROP_PLANTING_DAY: '120' },
    )?.dir,
    '/usgs',
  );
  assert.equal(compatibleKernel(kernels, { subgrid: 'IGBP', crop: true }, {})?.dir, '/crop');
  assert.equal(
    compatibleKernel(
      kernels,
      { subgrid: 'IGBP', crop: false },
      { DEF_TUNING_CROP_PLANTING_DAY: '120' },
    )?.dir,
    '/default',
  );
});

it('keeps USGS classification independent of PFT and PC structure', () => {
  const kernels = [
    { dir: '/igbp', preset: 'default', macros: ['SinglePoint', 'LULC_IGBP'] },
    { dir: '/usgs', preset: 'usgs', macros: ['SinglePoint', 'LULC_USGS'] },
  ];
  for (const subgrid of ['PFT', 'PC'])
    assert.equal(
      compatibleKernel(kernels, { classification: 'USGS', subgrid, crop: false })?.dir,
      '/usgs',
    );
  assert.equal(
    compatibleKernel(kernels.slice(0, 1), { classification: 'USGS', subgrid: 'PFT', crop: false }),
    undefined,
    'no compatible USGS kernel must not silently become IGBP',
  );
});

describe('remote monitor ownership', () => {
  const remote = { host: 'server', job: 'old', provenance: { operation_id: 'remote-op' } };
  const status = { record: remote, status: { state: 'finished', exit_code: 1 } };
  it('keeps newer local status, logs and output separate from an old remote job', () => {
    assert.equal(
      matchesRemoteRun({ ...run, location: 'local', operation_id: 'local-op' }, status),
      false,
    );
  });
  it('rejects another remote job, host or operation even with the same UI run ID', () => {
    const current: RunState = {
      ...run,
      location: 'remote',
      operation_id: 'remote-op',
      progress: { remote },
    };
    assert.equal(matchesRemoteRun(current, status), true);
    for (const record of [
      { ...remote, job: 'new' },
      { ...remote, host: 'other' },
      { ...remote, provenance: { operation_id: 'other-op' } },
    ])
      assert.equal(matchesRemoteRun(current, { ...status, record }), false);
  });
  it('matches recovered wrapped records and flat submission records without operation metadata', () => {
    assert.equal(
      matchesRemoteRun({ ...run, location: 'remote', progress: { remote: status } }, status),
      true,
    );
    assert.equal(
      matchesRemoteRun({ ...run, location: 'remote', progress: { remote } }, status),
      true,
    );
  });
  it('checks the recovered operation identity when the run has no operation field', () => {
    assert.equal(
      matchesRemoteRun(
        {
          ...run,
          location: 'remote',
          progress: { remote: { ...remote, operation_id: 'different' } },
        },
        status,
      ),
      false,
    );
  });
  it('does not apply unidentified or missing records', () => {
    assert.equal(matchesRemoteRun(undefined, status), false);
    assert.equal(matchesRemoteRun({ ...run, location: 'remote' }, status), false);
    assert.equal(
      matchesRemoteRun({ ...run, location: 'remote', progress: { remote } }, { record: null }),
      false,
    );
  });
});

test('identityless remote recovery requires the exact started operation and host', () => {
  const current: RunState = {
    ...run,
    location: 'remote',
    status: 'unknown',
    operation_id: 'started',
    progress: { host: 'server' },
  };
  const record = { host: 'server', job: 'job', provenance: { operation_id: 'started' } };
  assert.equal(matchesRemoteRun(current, { record }), true);
  assert.equal(matchesRemoteRun({ ...current, operation_id: undefined }, { record }), false);
  assert.equal(
    matchesRemoteRun(current, { record: { ...record, provenance: { operation_id: 'old' } } }),
    false,
  );
  assert.equal(matchesRemoteRun(current, { record: { ...record, host: 'other' } }), false);
  assert.equal(matchesRemoteRun({ ...current, location: 'local' }, { record }), false);
});

test('remote view reads parsed status progress, last valid stage and log tail without guessing a stage', () => {
  const remoteRun: RunState = {
    ...run,
    location: 'remote',
    progress: {
      remote: {
        status: { state: 'running', log_tail: 'server step 12\nserver step 13' },
        stages: [['mksrfdata', 'done'], ['mkinidata', 'done'], ['colm', 'begin'], ['malformed']],
        progress: { step: 13, date: '2002-01-01', total_steps: 100 },
      },
    },
  };
  assert.deepEqual(restoreRunView(remoteRun, '/case/a'), {
    runId: 'new',
    casePath: '/case/a',
    status: 'running',
    lines: ['server step 12', 'server step 13'],
    stage: 'colm',
    step: 13,
    total: 100,
  });
  remoteRun.progress = { remote: { status: { state: 'queued' }, stages: [], progress: null } };
  const queued = restoreRunView(remoteRun, '/case/a');
  assert.equal(queued?.stage, undefined);
  assert.equal(queued?.step, undefined);
  assert.equal(queued?.total, undefined);
  assert.deepEqual(queued?.lines, ['step 1'], 'local submission diagnostics remain available');
  for (const phase of [
    'preparing the engine',
    'installing Rust/Cargo',
    'installing CMake',
    'building the Rust engine',
  ]) {
    remoteRun.progress = {
      remote: { status: { state: 'running', phase }, stages: [], progress: null },
    };
    assert.equal(restoreRunView(remoteRun, '/case/a')?.stage, phase);
  }
  remoteRun.progress = {
    remote: { status: { phase: 'installing CMake' }, stages: [['colm', 'begin']] },
  };
  assert.equal(restoreRunView(remoteRun, '/case/a')?.stage, 'colm');
  remoteRun.progress = { remote: { status: { phase: 'unknown text' }, stages: [] } };
  assert.equal(restoreRunView(remoteRun, '/case/a')?.stage, undefined);
});

test('the engine build is watched only while a server run is being submitted', () => {
  const running = {
    '/a': { status: 'submitting' },
    '/b': { status: 'submitted' },
  };
  assert.equal(submittingHost('tianhe', ['/a', '/b'], running), 'tianhe');
  assert.equal(submittingHost('tianhe', ['/b'], running), '', 'already submitted');
  assert.equal(submittingHost('', ['/a'], running), '', 'a local run builds no server engine');
});

test('a server job the scheduler cancelled before it ran is over, with nothing to fetch', () => {
  const record = { host: 'land02', job: 'run-1' };
  const cancelled = {
    record,
    status: { state: 'lost', detail: 'CANCELLED by 1026|0:0' },
    ended: 'cancelled',
  };
  assert.deepEqual(remoteRunPermissions(cancelled), {
    pending: false,
    terminal: true,
    cancel: false,
    fetch: false,
  });
  assert.equal(remoteEndStatus(cancelled), 'cancelled');
  assert.equal(remoteEndStatus({ ...cancelled, ended: 'failed' }), 'failed');
  assert.equal(
    remoteRunPermissions({ record, status: { state: 'lost' }, ended: null }).terminal,
    false,
  );
  assert.equal(
    remoteEndStatus({ record, status: { state: 'finished', exit_code: 0 } }),
    'succeeded',
  );
});
