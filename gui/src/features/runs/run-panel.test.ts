import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  settleRuns,
  remoteRunPermissions,
  remoteSubmissionBlocked,
  reconcileRemoteRun,
  matchesRemoteRun,
} from './run-model';
import { runRemoteBatch } from './remote-batch';
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
    ts.forEachChild(node, visit);
  };
  visit(source);
  const state = {
    running: {} as Record<string, RunState>,
    versionRef: { id: 'chosen', revision: 2 },
    selectedCase: { dir: '/good', binding: { version: { id: 'old', revision: 1 } } },
  };
  const calls: { command: string; args: Record<string, unknown> }[] = [];
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
    casePath: '/good',
    currentCase: { current: '/good' },
    batchCases:
      options.single || (options.remote && !options.remoteBatch)
        ? undefined
        : (options.targets ?? ['/good', '/bad']),
    host: options.remote ? 'server' : '',
    monitor: false,
    remotePermissions: { cancel: false, fetch: true },
    setMonitor: () => {},
    remoteRunPermissions,
    remoteSubmissionBlocked,
    reconcileRemoteRun,
    matchesRemoteRun,
    recovering: false,
    disabled: false,
    kernelMatches: true,
    kernel: '/kernel',
    engine: 'rust',
    preprocessors: 'rust',
    ranks: options.ranks ?? '1',
    threads: '1',
    cpus: '',
    scheduler: 'auto',
    workers: '1',
    stage: '',
    force: false,
    recordInputFingerprints: options.recordInputFingerprints ?? false,
    invoke,
    invokeJson: invoke,
    notify: () => {},
    setBusy: () => {},
    bilingual: (zh: string) => zh,
    t: (zh: string) => zh,
    settleRuns,
    runRemoteBatch: (args: Parameters<typeof runRemoteBatch>[0]) =>
      runRemoteBatch({ ...args, wait: async () => {} }),
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
    `let remote = null; const setRemote = value => {remote = value}; ${code};return {run,cancel,remoteAction};`,
  )(...Object.values(context)) as {
    run: () => Promise<unknown>;
    cancel: () => Promise<unknown>;
    remoteAction: (command: string) => Promise<unknown>;
  };
  return { ...methods, state, calls };
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

test('remote batch passes resource overrides to each case and tracks separate finished jobs', async () => {
  const p = panel({
    remote: true,
    remoteBatch: true,
    ranks: '8',
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
    assert.equal(args.cpus, null);
  }
  assert.equal(p.state.running['/good'].status, 'succeeded');
  assert.equal(p.state.running['/bad'].status, 'failed');
  assert.equal(p.state.running['/bad'].running, false);
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
    assert.equal(p.state.running['/bad'].status, 'cancelled');
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
