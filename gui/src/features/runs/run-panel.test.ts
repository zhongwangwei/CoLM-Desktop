import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { settleRuns } from './run-model';
import type { RunState } from '../../lib/types';

function panel(
  options: {
    migrate?: (dir: string) => Promise<unknown>;
    submit?: () => Promise<unknown>;
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
      ['action', 'patchRun', 'run', 'cancel'].includes(node.name.text)
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
    if (command === 'model_migrate_case') {
      await options.migrate?.(String(args.dir));
      return { version: state.versionRef };
    }
    if (command === 'cancel_runs') return 0;
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
    batch: ['/bad'],
    host: '',
    recovering: false,
    disabled: false,
    kernelMatches: true,
    kernel: '/kernel',
    engine: 'rust',
    preprocessors: 'rust',
    ranks: '1',
    threads: '1',
    workers: '1',
    stage: '',
    force: false,
    invoke,
    invokeJson: invoke,
    notify: () => {},
    setBusy: () => {},
    setRemote: () => {},
    bilingual: (zh: string) => zh,
    t: (zh: string) => zh,
    settleRuns,
    update: (patch: Partial<typeof state> | ((old: typeof state) => Partial<typeof state>)) => {
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch);
      latest.current = state.running;
    },
  };
  const code = ts.transpileModule(functions.join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const methods = new Function(...Object.keys(context), `${code};return {run,cancel};`)(
    ...Object.values(context),
  ) as { run: () => Promise<unknown>; cancel: () => Promise<unknown> };
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
