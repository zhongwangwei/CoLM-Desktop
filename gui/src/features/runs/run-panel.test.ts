import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  settleRuns,
  endedRun,
  openRuns,
  remoteJobFacts,
  runCountsText,
  remoteStatusSummary,
  restoredRuns,
  remoteRunPermissions,
  neverSubmitted,
  remoteSubmissionBlocked,
  reconcileRemoteRun,
  matchesRemoteRun,
  remoteEndStatus,
  restoreRunView,
} from './run-model';
import { eachLimited, runRemoteBatch } from './remote-batch';
import { compatibleKernel } from './run-model';
import { applyRunEvent } from '../../lib/run-events';
import type { RunState } from '../../lib/types';

function panel(
  options: {
    migrate?: (dir: string) => Promise<unknown>;
    submit?: () => Promise<unknown>;
    remote?: boolean;
    remoteBatch?: boolean;
    recordInputFingerprints?: boolean;
    ranks?: string;
    ipc?: (command: string, args: Record<string, unknown>) => Promise<unknown>;
    single?: boolean;
    targets?: string[];
    notify?: (message: string) => void;
    setMonitor?: (value: boolean) => void;
    stationConcurrency?: number;
    engine?: string;
    threads?: string;
    cpus?: string;
  } = {},
) {
  const source = ts.createSourceFile(
    'RunPanel.tsx',
    readFileSync(new URL('./RunPanel.tsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const functions: string[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isFunctionDeclaration(node) &&
      node.name &&
      ['action', 'patchRun', 'run', 'cancel', 'remoteAction'].includes(node.name.text)
    )
      functions.push(node.getText(source));
    // `openBatchRuns` 是箭头函数常量：批量自动核对能看到哪些待核对站点由它决定（GUI-12）。
    if (
      ts.isVariableDeclaration(node) &&
      node.name.getText(source) === 'openBatchRuns' &&
      node.initializer
    )
      functions.push(`const openBatchRuns = ${node.initializer.getText(source)};`);
    ts.forEachChild(node, visit);
  };
  visit(source);
  const state = {
    running: {} as Record<string, RunState>,
    versionRef: { id: 'chosen', revision: 2 },
    selectedCase: { dir: '/good', binding: { version: { id: 'old', revision: 1 } } },
  };
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const batchWorkers: number[] = [];
  const latest = { current: state.running };
  const invoke = async (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (options.ipc) return options.ipc(command, args);
    if (command === 'model_migrate_case') {
      await options.migrate?.(String(args.dir));
      return { version: state.versionRef };
    }
    if (command === 'cancel_runs') return 0;
    if (command === 'remote_run') {
      state.running['/good'] = applyRunEvent(state.running['/good'], 'run://started', {
        run_id: args.runId,
        source_case: '/good',
        operation_id: 'submitted-operation',
        location: 'remote',
      })!;
      throw new Error('connection lost after submission');
    }
    if (command === 'remote_status')
      return {
        record: {
          host: 'server',
          job: 'job-new',
          provenance: { operation_id: 'submitted-operation' },
        },
        status: { state: 'finished', exit_code: 0 },
      };
    if (command === 'remote_fetch') return { run_case: '/fetched-run' };
    await options.submit?.();
    return command === 'run_batch' ? { succeeded: (args.cases as string[]).length, failed: 0 } : 0;
  };
  const context = {
    state,
    latest,
    lock: { current: false },
    alive: { current: true },
    cancelledRun: { current: null as string | null },
    stoppedBatches: new Set<string>(),
    drivenBatches: new Set<string>(),
    casePath: '/good',
    currentCase: { current: '/good' },
    batchCases:
      options.single || (options.remote && !options.remoteBatch)
        ? undefined
        : (options.targets ?? ['/good', '/bad']),
    host: options.remote ? 'server' : '',
    siteBatch: !!options.remote && !!options.remoteBatch && !options.single,
    stationConcurrency: options.stationConcurrency ?? 1,
    localSiteNow: false,
    monitor: false,
    remotePermissions: { cancel: false, fetch: true },
    setMonitor: options.setMonitor ?? (() => {}),
    remoteRunPermissions,
    neverSubmitted,
    remoteSubmissionBlocked,
    reconcileRemoteRun,
    matchesRemoteRun,
    remoteEndStatus,
    recovering: false,
    disabled: false,
    kernelMatches: true,
    kernel: '/kernel',
    engine: options.engine ?? 'rust',
    preprocessors: 'rust',
    ranks: options.ranks ?? '1',
    threads: options.threads ?? '1',
    cpus: options.cpus ?? '',
    scheduler: 'auto',
    workers: '1',
    stage: '',
    force: false,
    recordInputFingerprints: options.recordInputFingerprints ?? false,
    invoke,
    invokeJson: invoke,
    notify: options.notify ?? (() => {}),
    setBusy: () => {},
    bilingual: (zh: string) => zh,
    t: (zh: string) => zh,
    settleRuns,
    eachLimited,
    runRemoteBatch: (args: Parameters<typeof runRemoteBatch>[0]) => {
      batchWorkers.push(args.workers);
      return runRemoteBatch({ ...args, wait: async () => {} });
    },
    update: (patch: Partial<typeof state> | ((old: typeof state) => Partial<typeof state>)) => {
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch);
      latest.current = state.running;
    },
  };
  const code = ts.transpileModule(functions.join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const methods = new Function(
    ...Object.keys(context),
    `let remote = null; const setRemote = value => {remote = value}; ${code};return {run,cancel,remoteAction,openBatchRuns};`,
  )(...Object.values(context)) as {
    run: () => Promise<unknown>;
    cancel: () => Promise<unknown>;
    remoteAction: (
      command: string,
      target?: string,
      quiet?: boolean,
      held?: boolean,
    ) => Promise<unknown>;
    openBatchRuns: () => string[];
  };
  return { ...methods, state, calls, batchWorkers, latest, alive: context.alive };
}

test('one failed migration does not prevent healthy batch cases from running', async () => {
  const p = panel({
    migrate: async (dir) => {
      if (dir === '/bad') throw new Error('history import failed');
    },
  });
  await p.run();
  const submitted = p.calls.find((call) => call.command === 'run_batch');
  assert.ok(submitted);
  assert.deepEqual(submitted.args.cases, ['/good']);
  assert.equal(p.state.running['/bad'].status, 'failed');
  assert.equal(p.state.running['/good'].status, 'succeeded');
});

test('stop during migration prevents later migration and run submission', async () => {
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const p = panel({ migrate: () => gate });
  const running = p.run();
  await p.cancel();
  release();
  await running;
  assert.equal(p.calls.filter((call) => call.command === 'model_migrate_case').length, 1);
  assert.ok(
    !p.calls.some((call) => ['run_batch', 'run_case', 'remote_run'].includes(call.command)),
  );
  assert.equal(p.state.running['/good'].status, 'cancelled');
  assert.equal(p.state.running['/bad'].status, 'cancelled');
});

test('an entirely failed migration batch submits no run', async () => {
  const p = panel({
    migrate: async () => {
      throw new Error('migration failed');
    },
  });
  await p.run();
  assert.ok(!p.calls.some((call) => ['run_batch', 'run_case'].includes(call.command)));
  assert.equal(p.state.running['/good'].status, 'failed');
  assert.equal(p.state.running['/bad'].status, 'failed');
});

test('a failed primary case does not prevent another selected case from running', async () => {
  const p = panel({
    migrate: async (dir) => {
      if (dir === '/good') throw new Error('primary migration failed');
    },
  });
  await p.run();
  assert.deepEqual(p.calls.find((call) => call.command === 'run_batch')?.args.cases, ['/bad']);
  assert.equal(p.state.running['/good'].status, 'failed');
  assert.equal(p.state.running['/bad'].status, 'succeeded');
});

test('stop after the last migration wait still prevents submission and permits a later run', async () => {
  let release!: () => void;
  let reachedLast!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  const lastMigration = new Promise<void>((resolve) => {
    reachedLast = resolve;
  });
  const p = panel({
    migrate: async (dir) => {
      if (dir === '/bad') {
        reachedLast();
        await gate;
      }
    },
  });
  const running = p.run();
  await lastMigration;
  await p.cancel();
  release();
  await running;
  assert.ok(!p.calls.some((call) => call.command === 'run_batch'));
  assert.ok(Object.values(p.state.running).every((run) => run.status === 'cancelled'));
  await p.run();
  assert.equal(p.calls.filter((call) => call.command === 'run_batch').length, 1);
  assert.ok(Object.values(p.state.running).every((run) => run.status === 'succeeded'));
});

test('an uncertain remote submission recovers by its started operation and can fetch results', async () => {
  const p = panel({ remote: true });
  await assert.rejects(p.run(), /connection lost after submission/);
  assert.equal(p.state.running['/good'].status, 'unknown');
  assert.equal(p.state.running['/good'].location, 'remote');
  assert.equal(p.state.running['/good'].operation_id, 'submitted-operation');
  await p.remoteAction('remote_status');
  assert.equal(p.state.running['/good'].status, 'succeeded');
  assert.equal(p.state.running['/good'].running, false);
  await p.remoteAction('remote_fetch');
  assert.equal(p.state.running['/good'].run_case, '/fetched-run');
});

test('a single-case page submits only its current case', async () => {
  const p = panel({ single: true });
  await p.run();
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'model_migrate_case').map((call) => call.args.dir),
    ['/good'],
  );
  assert.equal(p.calls.find((call) => call.command === 'run_case')?.args.case, '/good');
  assert.ok(!p.calls.some((call) => call.command === 'run_batch'));
});

test('project batch submits only the explicitly selected cases', async () => {
  const p = panel({ targets: ['/bad', '/bad'] });
  await p.run();
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'model_migrate_case').map((call) => call.args.dir),
    ['/bad'],
  );
  assert.equal(p.calls.find((call) => call.command === 'run_case')?.args.case, '/bad');
});

test('an empty project selection cannot start a run', async () => {
  const p = panel({ targets: [] });
  await assert.rejects(p.run(), /请选择算例/);
  assert.equal(p.calls.length, 0);
});

test('remote batch runs as many stations as CPUs are requested, one CPU each, and tracks separate jobs', async () => {
  const p = panel({
    remote: true,
    remoteBatch: true,
    ranks: '8',
    stationConcurrency: 6,
    ipc: async (command, args) => {
      if (command === 'model_migrate_case') return { version: { id: 'chosen', revision: 2 } };
      if (command === 'remote_job_record') return null;
      if (command === 'remote_run') return { host: 'server', job: `job-${args.case}` };
      if (command === 'remote_status')
        return {
          record: { host: 'server', job: `job-${args.case}` },
          status: { state: 'finished', exit_code: args.case === '/good' ? 0 : 1 },
        };
      throw new Error(command);
    },
  });
  await p.run();
  const submissions = p.calls.filter((call) => call.command === 'remote_run');
  assert.deepEqual(
    submissions.map((call) => call.args.case),
    ['/good', '/bad'],
  );
  for (const { args } of submissions) {
    assert.equal(args.threads, 1);
    assert.equal(args.ranks, 1);
    assert.equal(args.scheduler, 'auto');
    assert.equal(args.cpus, 1);
    // One batch shares one node queue; the server packs it only on whole-node partitions.
    assert.equal(args.pack, true);
    assert.equal(args.runId, submissions[0].args.runId);
  }
  // Packed per node on the server: every station joins the queue at once.
  assert.deepEqual(p.batchWorkers, [2]);
  assert.equal(p.state.running['/good'].status, 'succeeded');
  assert.equal(p.state.running['/bad'].status, 'failed');
  assert.equal(p.state.running['/bad'].running, false);
});

test('leaving the page does not stop the stations still to be submitted', async () => {
  const targets = ['/a', '/b', '/c', '/d'];
  const p: ReturnType<typeof panel> = panel({
    remote: true,
    remoteBatch: true,
    stationConcurrency: 1,
    targets,
    ipc: async (command, args) => {
      if (command === 'model_migrate_case') return { version: { id: 'chosen', revision: 2 } };
      if (command === 'remote_job_record') return null;
      if (command === 'remote_run') {
        // The page unmounts right after the first station is submitted.
        if (args.case === '/a') p.alive.current = false;
        return { host: 'server', job: `job-${args.case}` };
      }
      if (command === 'remote_status')
        return {
          record: { host: 'server', job: `job-${args.case}` },
          status: { state: 'finished', exit_code: 0 },
        };
      throw new Error(command);
    },
  });
  await p.run();
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'remote_run').map((call) => call.args.case),
    targets,
  );
  for (const dir of targets) assert.equal(p.state.running[dir].status, 'succeeded');
});

test('batch kernel check validates every selected profile', () => {
  const source = ts.createSourceFile(
    'RunPanel.tsx',
    readFileSync(new URL('./RunPanel.tsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let expression = '';
  function visit(node: ts.Node) {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'kernelMatches')
      expression = node.initializer!.getText(source);
    ts.forEachChild(node, visit);
  }
  visit(source);
  const javascript = ts.transpileModule(`const result = ${expression};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const evaluate = new Function(
    'effectiveProfile',
    'batchProfiles',
    'state',
    'kernel',
    'compatibleKernel',
    'effectiveModelFields',
    `${javascript}; return result;`,
  );
  const profile = { grid: 'site', classification: 'IGBP', crop: false };
  const state = {
    kernels: [{ dir: '/kernel', preset: 'default', macros: ['SinglePoint', 'LULC_IGBP'] }],
  };
  assert.equal(
    evaluate(
      profile,
      { a: profile, b: { ...profile, crop: true } },
      state,
      '/kernel',
      compatibleKernel,
      () => ({}),
    ),
    false,
  );
  assert.equal(
    evaluate(profile, { a: profile, b: profile }, state, '/kernel', compatibleKernel, () => ({})),
    true,
  );
});

test('Rust execution ignores ranks left over from a previous Fortran selection', async () => {
  const p = panel({ single: true, ranks: '8' });
  await p.run();
  assert.equal(p.calls.find((call) => call.command === 'run_case')?.args.mpiRanks, 1);
});

test('remote batch failures retain a reconciliation placeholder without inventing job identity', async () => {
  for (const failure of ['remote_run', 'remote_job_record']) {
    const p = panel({
      remote: true,
      remoteBatch: true,
      ipc: async (command) => {
        if (command === failure) throw new Error('Connection lost');
        if (command === 'model_migrate_case') return { version: { id: 'chosen', revision: 2 } };
        if (command === 'remote_job_record') return null;
        throw new Error(command);
      },
    });
    await assert.rejects(p.run(), /Connection lost/);
    const current = p.state.running['/good'];
    assert.equal(current.status, 'unknown');
    assert.equal(current.running, true);
    assert.deepEqual(current.progress?.remote, {
      status: { state: 'unknown' },
      error: 'Error: Connection lost',
    });
    // A packed batch sends every station at once, so the other station was sent too and is
    // equally unconfirmed rather than never sent.
    assert.equal(p.state.running['/bad'].status, 'unknown');
  }
});

test('explicit reconciliation adopts a confirmed job for an unknown submission with no captured identity', async () => {
  const p = panel({
    remote: true,
    remoteBatch: true,
    ipc: async (command) => {
      if (command === 'model_migrate_case') return { version: { id: 'chosen', revision: 2 } };
      if (command === 'remote_job_record') return null;
      if (command === 'remote_run') throw new Error('Connection lost before identity event');
      if (command === 'remote_status')
        return {
          record: { host: 'server', job: 'discovered-job' },
          status: { state: 'finished', exit_code: 0 },
        };
      throw new Error(command);
    },
  });
  await assert.rejects(p.run(), /Connection lost/);
  assert.equal(p.state.running['/good'].running, true);
  await p.remoteAction('remote_status');
  assert.equal(p.state.running['/good'].running, false);
  assert.equal(p.state.running['/good'].status, 'succeeded');
});

test('batch members wait in queue until their own worker starts', async () => {
  const p = panel({
    submit: async () => {
      assert.equal(p.state.running['/good'].status, 'queued');
      assert.equal(p.state.running['/bad'].status, 'queued');
      assert.equal(
        p.state.running['/good'].running,
        true,
        'queued work still locks duplicate submission',
      );
      const runId = p.state.running['/good'].run_id;
      p.state.running['/good'] = applyRunEvent(p.state.running['/good'], 'run://started', {
        run_id: runId,
        source_case: '/good',
        location: 'local',
      })!;
      assert.equal(p.state.running['/good'].status, 'running');
      assert.equal(p.state.running['/bad'].status, 'queued');
    },
  });
  await p.run();
});

test('input fingerprint recording is off by default and opt-in reaches single, batch and remote runs', async () => {
  for (const shape of [
    { single: true },
    {},
    { remote: true },
    { remote: true, remoteBatch: true },
  ]) {
    for (const recordInputFingerprints of [false, true]) {
      const p = panel({
        ...shape,
        recordInputFingerprints,
        ipc: async (command) => {
          if (command === 'model_migrate_case') return { version: { id: 'chosen', revision: 2 } };
          if (command === 'remote_job_record') return null;
          if (command === 'remote_run')
            return {
              record: { host: 'server', job: 'test-job' },
              status: { state: 'finished', exit_code: 0 },
            };
          if (command === 'remote_status')
            return {
              record: { host: 'server', job: 'test-job' },
              status: { state: 'finished', exit_code: 0 },
            };
          return command === 'run_batch' ? { succeeded: 2, failed: 0 } : 0;
        },
      });
      await p.run();
      const calls = p.calls.filter((call) =>
        ['run_case', 'run_batch', 'remote_run'].includes(call.command),
      );
      assert.ok(calls.length);
      for (const call of calls)
        assert.equal(call.args.recordInputFingerprints, recordInputFingerprints);
    }
  }
  const defaults = panel({ single: true });
  await defaults.run();
  assert.equal(
    defaults.calls.find((call) => call.command === 'run_case')?.args.recordInputFingerprints,
    false,
  );
});

test('automatic server checks stay quiet and announce only the finish', async () => {
  const notices: string[] = [],
    monitors: boolean[] = [];
  let fail = true;
  const p = panel({
    remote: true,
    single: true,
    notify: (message) => notices.push(message),
    setMonitor: (value) => monitors.push(value),
    ipc: async (command) => {
      if (command !== 'remote_status') return 0;
      if (fail) throw new Error('ssh timeout');
      return {
        record: { host: 'server', job: 'job-new' },
        status: { state: 'finished', exit_code: 0 },
      };
    },
  });
  await assert.rejects(p.remoteAction('remote_status', '/good', true), /ssh timeout/);
  assert.deepEqual(notices, [], 'a failed background check raises no notice');
  fail = false;
  await p.remoteAction('remote_status', '/good', true);
  assert.deepEqual(monitors, [], 'background checks never lock the panel');
  assert.equal(notices.length, 1);
  assert.match(notices[0], /服务器作业已结束/);
  await p.remoteAction('remote_status', '/good', true);
  assert.equal(notices.length, 1, 'the finish is announced once');
});

test('a batch that finished before is restored as finished, not as stations to re-check', () => {
  const finished = {
    status: 'succeeded',
    operation_id: 'op-1',
    needs_reconciliation: false,
    finished_ms: 1,
    location: 'remote',
    host: 'tms17l',
  };
  const restored = restoredRuns(
    {
      '/a': finished,
      '/b': { ...finished, operation_id: 'op-2', status: 'failed' },
      '/c': { ...finished, needs_reconciliation: true },
      '/d': { ...finished, finished_ms: null, status: 'running' },
      '/e': finished,
    },
    { '/e': { case: '/e', status: 'running', running: true, logs: [], stages: {} } },
  );
  assert.deepEqual(Object.keys(restored).sort(), ['/a', '/b']);
  assert.equal(restored['/a'].status, 'succeeded');
  assert.equal(restored['/a'].running, false);
  assert.equal(restored['/a'].location, 'remote');
  assert.equal(restored['/b'].status, 'failed');
  assert.ok(endedRun(finished));
  assert.ok(!endedRun({ ...finished, needs_reconciliation: true }));
  assert.ok(!endedRun(undefined));
});

test('a station that ended before the window opened shows its recorded last stage and step', () => {
  const ended = {
    status: 'succeeded',
    operation_id: 'op-1',
    needs_reconciliation: false,
    finished_ms: 1,
    location: 'remote',
    host: 'land02',
  };
  const restored = restoredRuns(
    {
      '/HU-Bug': {
        ...ended,
        progress: {
          stages: [['colm', 'ok']] as [string, string][],
          step: 87646,
          date: '2006-12-31-81000',
          total_steps: 87646,
        },
      },
      '/old': ended,
      '/stopped': { ...ended, operation_id: 'op-3', status: 'cancelled' },
    },
    {},
  );
  const view = restoreRunView(restored['/HU-Bug'], '/HU-Bug')!;
  assert.deepEqual(
    [view.status, view.stage, view.step, view.total],
    ['succeeded', 'colm', 87646, 87646],
  );
  assert.equal((restored['/HU-Bug'].progress as { host: string }).host, 'land02');
  assert.equal(restored['/HU-Bug'].progress?.remote, undefined, 'nothing is left to re-check');
  const old = restoreRunView(restored['/old'], '/old')!;
  assert.deepEqual([old.stage, old.step, old.total], [undefined, undefined, undefined]);
  assert.equal(restored['/stopped'].status, 'cancelled');
});

test('open ledger runs are watched, and checking server jobs reports a summary', () => {
  const last = (status: string, needs = false, location: string | null = 'remote') => ({
    status,
    operation_id: `op-${status}`,
    needs_reconciliation: needs,
    finished_ms: status === 'running' ? null : 1,
    location,
    host: 'bd',
  });
  const watched = openRuns(
    {
      '/done': last('succeeded'),
      '/open': last('running'),
      '/unsure': last('succeeded', true),
      '/local': last('running', false, 'local'),
      '/known': last('running'),
    },
    { '/known': { case: '/known', status: 'running', running: true, logs: [], stages: {} } },
  );
  assert.deepEqual(Object.keys(watched).sort(), ['/open', '/unsure']);
  assert.equal(watched['/open'].status, 'unknown');
  assert.equal(watched['/open'].running, true);
  assert.equal((watched['/open'].progress as { host: string }).host, 'bd');
  const record = { host: 'bd', job: 'j' };
  const summary = remoteStatusSummary([
    { record, status: { state: 'finished', exit_code: 0 } },
    { record, status: { state: 'finished', exit_code: 0 } },
    { record, status: { state: 'finished', exit_code: 2 } },
    { record, status: { state: 'running' } },
    { record, status: { state: 'queued' } },
    { record, status: { state: 'lost' } },
  ]);
  assert.equal(
    summary,
    '已核对 6 个服务器作业：已完成 2 · 失败 1 · 运行中 1 · 排队中 1 · 未能确认 1',
  );
});

test('a server job is described in words, with the raw record left for troubleshooting', () => {
  const record = {
    host: 'bd',
    job: 'run-abc',
    scheduler: 'lsf',
    scheduler_id: '232723',
    submitted_at: 1792375398,
    remote_case: '/share/home/u/colm-desktop/qqq/AT-Neu/runs/abc',
    provenance: {
      engine: 'rust',
      ranks: 1,
      model_revision: { id: 'official', revision: 74 },
      remote_options: [
        ['--partition', 'normal'],
        ['--cpus', '1'],
        ['--threads', '1'],
      ],
    },
  };
  const facts = (value: unknown, now?: number) =>
    Object.fromEntries(remoteJobFacts(value, false, now).map((f) => [f.label, f.value]));
  const running = facts(
    {
      record,
      status: {
        state: 'running',
        detail: 'RUN',
        phase: 'running',
        log_tail: 'starting\nTIMESTEP = 17471 | DATE = 2002-12-30-82800\n',
      },
    },
    1792375398000 + 5 * 60000,
  );
  assert.equal(running['状态'], '运行中，已算到 2002-12-30');
  assert.equal(running['调度器状态'], 'RUN');
  assert.equal(running['服务器'], 'bd');
  assert.equal(running['调度系统'], 'LSF · 作业号 232723');
  assert.equal(running['申请资源'], '队列 normal · 1 CPU · 1 线程');
  assert.match(running['提交时间'], /5 分钟前/);
  assert.equal(running['模型版本'], '官方版 r74');
  assert.equal(running['计算引擎'], 'Rust');
  assert.equal(running['模型计算进度'], '第 17471 步（2002-12-30）');
  assert.equal(running['最近输出'], 'starting');
  assert.equal(
    facts({ record, status: { state: 'finished', exit_code: 0 } })['状态'],
    '已结束，成功（退出码 0）',
  );
  assert.equal(
    facts({ record, status: { state: 'finished', exit_code: 3 } })['状态'],
    '已结束，失败（退出码 3）',
  );
  assert.match(facts({ record, status: { state: 'queued' } })['状态'], /排队中/);
  assert.match(facts({ status: { state: 'unknown' } })['状态'], /未能确认/);
  // Nothing is invented for a record that says little.
  assert.deepEqual(Object.keys(facts(null)), ['状态']);
});

test('the standing of a batch is said in numbers, so a skipped check still answers', () => {
  const run = (status: string, running = false) => ({
    case: '/x',
    status,
    running,
    logs: [],
    stages: {},
  });
  assert.equal(
    runCountsText([
      run('submitted', true),
      run('running', true),
      run('preparing', true),
      run('succeeded'),
      run('succeeded'),
      run('failed'),
      run('cancelled'),
      run('unknown', true),
      undefined,
    ]),
    '运行中 2 · 等待 1 · 完成 2 · 失败 1 · 已停止 1 · 待核对 1',
  );
  assert.equal(runCountsText([]), '');
});

test('every station of a reopened batch claims its server job on the first status check', async () => {
  const p = panel({ remote: true, remoteBatch: true });
  // What reopening a project restores: no job record, no operation id, only the host.
  for (const dir of ['/good', '/bad'])
    p.state.running[dir] = {
      case: dir,
      location: 'remote',
      run_id: `legacy-${dir}`,
      status: 'unknown',
      running: true,
      logs: [],
      stages: {},
      progress: { host: 'server' },
    };
  for (const dir of ['/good', '/bad']) await p.remoteAction('remote_status', dir, true);
  for (const dir of ['/good', '/bad']) {
    assert.equal(p.state.running[dir].status, 'succeeded', dir);
    assert.equal(p.state.running[dir].running, false, dir);
  }
});

// GUI-12 复核证据：自动核对与手动核对都从 `openBatchRuns` 取目标，原来固定 `.slice(0, 8)`，
// 前 8 个一直运行时其余站点永远得不到核对。
test('a reopened batch watches every unfinished station, not just the first eight', () => {
  const dirs = Array.from({ length: 20 }, (_, index) => `/s${index}`);
  const p = panel({ remote: true, remoteBatch: true, targets: dirs });
  for (const dir of dirs)
    p.state.running[dir] = {
      case: dir,
      location: 'remote',
      run_id: `batch-${dir}`,
      status: 'running',
      running: true,
      logs: [],
      stages: {},
    };
  assert.deepEqual(p.openBatchRuns(), dirs, 'all 20 open stations are targets');
  // 已经结束的站点不再出现在待核对目标里。
  p.state.running['/s3'].running = false;
  assert.equal(p.openBatchRuns().includes('/s3'), false);
  assert.equal(p.openBatchRuns().length, 19);
});

// GUI-12 验收："已结束任务不因迟到响应回到运行中"——两个核对响应乱序返回时，晚到的
// "运行中"不能盖掉已经写入的终态。
test('a late status response cannot put a finished server job back to running', async () => {
  const job = {
    host: 'server',
    job: 'job-new',
    provenance: { operation_id: 'submitted-operation' },
  };
  const terminal = { record: job, status: { state: 'finished', exit_code: 0 } };
  const stillRunning = { record: job, status: { state: 'running' } };
  let releaseSlow!: (value: unknown) => void;
  const slow = new Promise<unknown>((resolve) => {
    releaseSlow = resolve;
  });
  const responses: Promise<unknown>[] = [slow, Promise.resolve(terminal)];
  const p = panel({
    remote: true,
    single: true,
    ipc: async (command) => {
      if (command !== 'remote_status') return 0;
      return responses.shift();
    },
  });
  p.state.running['/good'] = {
    case: '/good',
    location: 'remote',
    run_id: 'run-1',
    operation_id: 'submitted-operation',
    status: 'running',
    running: true,
    logs: [],
    stages: {},
    progress: { host: 'server', remote: stillRunning },
  };
  // 自动轮询与手动核对可以同时发出状态请求：先发的那次最后才返回。
  const slowCheck = p.remoteAction('remote_status', '/good', true, true);
  const fastCheck = p.remoteAction('remote_status', '/good', true, true);
  await fastCheck;
  assert.equal(p.state.running['/good'].status, 'succeeded');
  assert.equal(p.state.running['/good'].running, false);
  releaseSlow(stillRunning);
  await slowCheck;
  assert.equal(p.state.running['/good'].status, 'succeeded', 'the late running answer is ignored');
  assert.equal(p.state.running['/good'].running, false);
});

test('a Rust server run requests its thread count as CPUs; Fortran keeps the CPUs per rank', async () => {
  for (const [engine, ranks, expected] of [
    ['rust', '1', 6],
    ['fortran', '2', 4],
  ] as const) {
    const p = panel({ remote: true, single: true, engine, ranks, threads: '6', cpus: '4' });
    await p.run().catch(() => {});
    const submitted = p.calls.find((call) => call.command === 'remote_run');
    assert.equal(submitted?.args.cpus, expected, engine);
  }
});

test('stop during a single remote submission interrupts the local submitting process', async () => {
  const notices: string[] = [];
  const p = panel({
    remote: true,
    notify: (message: string) => notices.push(message),
    ipc: async (command) => (command === 'cancel_remote_submission' ? 1 : 0),
  });
  await p.cancel();
  const call = p.calls.find((entry) => entry.command === 'cancel_remote_submission');
  assert.deepEqual(call?.args, { case: '/good' });
  assert.ok(!p.calls.some((entry) => entry.command === 'cancel_runs'));
  assert.match(notices.join('\n'), /已中断提交/);
});
