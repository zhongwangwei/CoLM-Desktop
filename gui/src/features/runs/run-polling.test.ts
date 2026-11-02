import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { endedRun, openRuns, restoredRuns } from './run-model';
import type { RunState } from '../../lib/types';

function execute<T>(
  file: string,
  select: (node: ts.Node, source: ts.SourceFile) => string | undefined,
  context: Record<string, unknown>,
): T {
  const source = ts.createSourceFile(
    file,
    readFileSync(new URL(file, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let expression = '';
  function visit(node: ts.Node) {
    expression ||= select(node, source) ?? '';
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(expression);
  const js = ts.transpileModule(`const selected = (${expression});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return selected;`)(
    ...Object.values(context),
  );
}
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

test('engine progress waits for a slow request and ignores responses after its host changes', async () => {
  const requests: ((value: unknown) => void)[] = [];
  const changes: unknown[] = [];
  let tick!: () => void;
  let cleared = false;
  const effect = execute<() => () => void>(
    './RunPanel.tsx',
    (node, source) =>
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.arguments[0].getText(source).includes("'remote_engine_progress'")
        ? node.arguments[0].getText(source)
        : undefined,
    {
      submittingOn: 'server',
      setServerBuild: (value: unknown) => changes.push(value),
      invoke: () => new Promise((resolve) => requests.push(resolve)),
      setInterval: (callback: () => void) => {
        tick = callback;
        return 1;
      },
      clearInterval: () => {
        cleared = true;
      },
    },
  );
  const cleanup = effect();
  tick();
  tick();
  assert.equal(requests.length, 1, 'slow SSH requests do not overlap');
  requests[0]({ engine: 'one', done: 1 });
  await flush();
  assert.equal(changes.length, 2, 'the current response updates progress');
  tick();
  assert.equal(requests.length, 2);
  cleanup();
  assert.equal(cleared, true);
  requests[1]({ engine: 'one', done: 0 });
  await flush();
  assert.equal(changes.length, 2, 'a retired effect cannot replace progress');
});

test('server load A-B-A ignores the old A response and a superseded refresh', async () => {
  const requests: { host: string; resolve: (value: unknown) => void }[] = [];
  const readings: { host: string; load: unknown }[] = [];
  const busy: boolean[] = [];
  const current = { current: 'A' };
  const request = { current: 0 };
  const refresh = (host: string) =>
    execute<() => Promise<void>>(
      './LiveServerLoad.tsx',
      (node, source) =>
        ts.isFunctionDeclaration(node) && node.name?.text === 'refresh'
          ? node.getText(source)
          : undefined,
      {
        host,
        current,
        request,
        invoke: (_: string, args: { host: string }) =>
          new Promise((resolve) => requests.push({ host: args.host, resolve })),
        setReading: (value: (typeof readings)[number]) => readings.push(value),
        setBusy: (value: boolean) => busy.push(value),
      },
    );
  const oldA = refresh('A')();
  current.current = 'B';
  const b = refresh('B')();
  current.current = 'A';
  const newA = refresh('A')();
  requests[2].resolve('latest A');
  await newA;
  requests[0].resolve('old A');
  requests[1].resolve('old B');
  await Promise.all([oldA, b]);
  assert.deepEqual(
    readings.map(({ host, load }) => [host, load]),
    [['A', 'latest A']],
  );
  assert.equal(busy.filter((value) => !value).length, 1);
  const oldRefresh = refresh('A')();
  const newerRefresh = refresh('A')();
  requests[3].resolve('superseded');
  await oldRefresh;
  assert.equal(busy.at(-1), true, 'an old refresh cannot hide the active spinner');
  requests[4].resolve('newest');
  await newerRefresh;
  assert.equal(readings.at(-1)?.load, 'newest');
});

test('RunPanel load ignores A-B-A stale readings and keeps overlapping refresh busy', async () => {
  const requests: { resolve: (value: unknown) => void; reject: (reason: unknown) => void }[] = [];
  const readings: { load?: unknown; error?: string }[] = [];
  const busy: boolean[] = [];
  const loadTarget = { current: 'A' };
  const loadRequest = { current: 0 };
  const refresh = (host: string) =>
    execute<() => Promise<unknown>>(
      './RunPanel.tsx',
      (node, source) =>
        ts.isFunctionDeclaration(node) && node.name?.text === 'refreshLoad'
          ? node.getText(source)
          : undefined,
      {
        host,
        loadTarget,
        loadRequest,
        alive: { current: true },
        invoke: () => new Promise((resolve, reject) => requests.push({ resolve, reject })),
        setServerLoad: (value: (typeof readings)[number]) => readings.push(value),
        setLoadBusy: (value: boolean) => busy.push(value),
      },
    );
  const oldA = refresh('A')();
  loadTarget.current = 'B';
  const b = refresh('B')();
  loadTarget.current = 'A';
  const newA = refresh('A')();
  requests[2].resolve('latest A');
  await newA;
  requests[0].resolve('old A');
  requests[1].reject('old B error');
  await Promise.all([oldA, b]);
  assert.deepEqual(
    readings.map((value) => value.load),
    ['latest A'],
  );
  assert.equal(busy.filter((value) => !value).length, 1);
  const old = refresh('A')();
  const latest = refresh('A')();
  requests[3].reject('superseded error');
  await old;
  assert.equal(busy.at(-1), true);
  assert.equal(readings.length, 1);
  requests[4].resolve('newest A');
  await latest;
  assert.equal(readings.at(-1)?.load, 'newest A');
  assert.equal(busy.at(-1), false);
});

test('RunPanel retiring a load host invalidates its request even without another refresh', async () => {
  const loadTarget = { current: 'A' };
  const loadRequest = { current: 0 };
  const readings: unknown[] = [];
  const busy: boolean[] = [];
  let resolve!: (value: unknown) => void;
  const context = {
    host: 'A',
    loadTarget,
    loadRequest,
    alive: { current: true },
    invoke: () =>
      new Promise((done) => {
        resolve = done;
      }),
    setServerLoad: (value: unknown) => readings.push(value),
    setLoadBusy: (value: boolean) => busy.push(value),
  };
  const effect = execute<() => () => void>(
    './RunPanel.tsx',
    (node, source) =>
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.arguments[0].getText(source).includes('const request = loadRequest')
        ? node.arguments[0].getText(source)
        : undefined,
    context,
  );
  const refresh = execute<() => Promise<unknown>>(
    './RunPanel.tsx',
    (node, source) =>
      ts.isFunctionDeclaration(node) && node.name?.text === 'refreshLoad'
        ? node.getText(source)
        : undefined,
    context,
  );
  const cleanupA = effect();
  const pending = refresh();
  cleanupA();
  loadTarget.current = 'B';
  const cleanupB = effect();
  cleanupB();
  loadTarget.current = 'A';
  effect();
  resolve('retired A');
  await pending;
  assert.deepEqual(readings, []);
  assert.equal(busy.at(-1), false);
});

test('remote batch result access still waits for submission, cancellation, recovery and monitoring', () => {
  for (const guard of ['busy', 'cancelling', 'recovering', 'monitor']) {
    const reason = execute<string | null>(
      './RunPanel.tsx',
      (node, source) =>
        ts.isVariableDeclaration(node) && node.name.getText(source) === 'resultsBlockedReason'
          ? node.initializer?.getText(source)
          : undefined,
      {
        openingResults: false,
        disabled: false,
        monitor: false,
        recovering: false,
        cancelling: false,
        busy: false,
        host: 'server',
        batchWaiting: false,
        batchCases: ['/A', '/B'],
        pending: true,
        [guard]: true,
        t: (zh: string) => zh,
      },
    );
    assert.ok(reason, `${guard} must explain why the result action is blocked`);
  }
});

test('manual reconciliation locks configuration until the check completes', () => {
  for (const monitor of [false, true]) {
    const pending = execute<boolean>(
      './RunPanel.tsx',
      (node, source) =>
        ts.isVariableDeclaration(node) && node.name.getText(source) === 'pending'
          ? node.initializer?.getText(source)
          : undefined,
      {
        busy: false,
        cancelling: false,
        recovering: false,
        monitor,
        persisted: null,
        batchCases: null,
      },
    );
    assert.equal(pending, monitor);
  }
});

test('a waiting remote batch allows completed station results while another station runs', () => {
  const reason = execute<string | null>(
    './RunPanel.tsx',
    (node, source) =>
      ts.isVariableDeclaration(node) && node.name.getText(source) === 'resultsBlockedReason'
        ? node.initializer?.getText(source)
        : undefined,
    {
      openingResults: false,
      disabled: false,
      monitor: false,
      recovering: false,
      cancelling: false,
      busy: true,
      batchWaiting: true,
      host: 'server',
      batchCases: ['/done', '/running'],
      pending: true,
      t: (zh: string) => zh,
    },
  );
  assert.equal(reason, null);
});

test('manual check and cancellation keep the parent batch selection locked', () => {
  for (const flag of ['monitor', 'cancelling']) {
    const values: boolean[] = [];
    const effect = execute<() => () => void>(
      './RunPanel.tsx',
      (node, source) =>
        ts.isCallExpression(node) &&
        node.expression.getText(source) === 'useEffect' &&
        node.arguments[0].getText(source).includes('onBusyChange?.')
          ? node.arguments[0].getText(source)
          : undefined,
      {
        pending: true,
        batchCases: ['/a'],
        busy: false,
        recovering: false,
        monitor: false,
        cancelling: false,
        [flag]: true,
        onBusyChange: (value: boolean) => values.push(value),
      },
    );
    effect();
    assert.deepEqual(values, [true]);
  }
});

function restorePanel(legacy = false) {
  let imported = false;
  const currentHost = { current: 'A' },
    restoreRequest = { current: 0 };
  const latest = { current: {} as Record<string, RunState> };
  const requests: ((value: unknown) => void)[] = [],
    errors: string[] = [];
  let remote: unknown = null,
    recovering = false;
  const select = (host: string) => {
    currentHost.current = host;
    return execute<() => () => void>(
      './RunPanel.tsx',
      (node, source) => {
        if (
          !ts.isCallExpression(node) ||
          node.expression.getText(source) !== 'useEffect' ||
          !node.arguments[0].getText(source).includes("'remote_job_record'")
        )
          return;
        assert.match(
          node.arguments[1].getText(source),
          /\bhost\b/,
          'changing host must schedule restoration again',
        );
        return node.arguments[0].getText(source);
      },
      {
        host,
        casePath: '/case',
        batchCases: undefined,
        currentHost,
        restoreRequest,
        latest,
        currentCase: { current: '/case' },
        alive: { current: true },
        setRemote: (value: unknown) => {
          remote = value;
        },
        setRecovering: (value: boolean) => {
          recovering = value;
        },
        notify: (value: string) => errors.push(value),
        endedRun: legacy ? endedRun : () => true,
        restoredRuns: legacy ? restoredRuns : () => ({}),
        openRuns: legacy ? openRuns : () => ({}),
        update: (
          fn: (old: { running: typeof latest.current }) => { running?: typeof latest.current },
        ) => {
          const patch = fn({ running: latest.current });
          if (patch.running) latest.current = patch.running;
        },
        invokeJson: async (command: string) => {
          if (command === 'project_last_runs')
            return legacy
              ? imported
                ? {
                    '/case': {
                      status: 'unknown',
                      location: 'remote',
                      host: 'A',
                      operation_id: 'legacy-job',
                    },
                  }
                : {}
              : { '/case': { status: 'succeeded' } };
          if (command === 'model_case_runs') imported = true;
          if (command === 'remote_job_record')
            return new Promise((resolve) => requests.push(resolve));
          return [];
        },
      },
    )();
  };
  return {
    select,
    requests,
    latest,
    errors,
    get remote() {
      return remote;
    },
    get recovering() {
      return recovering;
    },
  };
}
const finishedJob = (host: string, job: string) => ({
  record: { host, job },
  status: { state: 'finished', exit_code: 0 },
  pending: false,
  needs_reconciliation: false,
});

test('remote job restores after server/local/server selection without remounting', async () => {
  const page = restorePanel();
  let cleanup = page.select('A');
  await flush();
  page.requests[0]({ host: 'A', job: 'done' });
  await flush();
  assert.deepEqual(page.remote, finishedJob('A', 'done'));
  cleanup();
  cleanup = page.select('');
  await flush();
  assert.equal(page.remote, null);
  assert.equal(page.requests.length, 1, 'local selection does not read a remote record');
  cleanup();
  cleanup = page.select('A');
  await flush();
  page.requests[1]({ host: 'A', job: 'done' });
  await flush();
  assert.deepEqual(page.remote, finishedJob('A', 'done'));
  cleanup();
  page.select('B');
  await flush();
  page.requests[2]({ host: 'A', job: 'done' });
  await flush();
  assert.equal(page.remote, null, 'another server’s record stays hidden');
  assert.deepEqual(page.errors, []);
});

test('late A restore cannot replace second A or clear its busy indicator', async () => {
  const page = restorePanel();
  const first = page.select('A');
  await flush();
  first();
  const second = page.select('B');
  await flush();
  second();
  page.select('A');
  await flush();
  page.requests[0]({ host: 'A', job: 'old' });
  page.requests[1]({ host: 'B', job: 'other' });
  await flush();
  assert.equal(page.remote, null);
  assert.equal(page.recovering, true);
  page.requests[2]({ host: 'A', job: 'new' });
  await flush();
  assert.deepEqual(page.remote, finishedJob('A', 'new'));
  assert.equal(page.recovering, false);
});

test('pending restoration cannot replace a newer operation', async () => {
  const page = restorePanel();
  page.select('A');
  await flush();
  page.latest.current['/case'] = {
    run_id: 'new',
    case: '/case',
    location: 'remote',
    status: 'running',
    running: true,
    logs: [],
    stages: {},
  };
  page.requests[0]({ host: 'A', job: 'old' });
  await flush();
  assert.equal(page.remote, null);
});

test('first opening a legacy case discovers its active remote job after history import', async () => {
  const page = restorePanel(true);
  page.select('');
  await flush();
  assert.equal(page.latest.current['/case']?.status, 'unknown');
  assert.equal(page.latest.current['/case']?.running, true);
  assert.equal(page.latest.current['/case']?.progress?.host, 'A');
  assert.equal(page.remote, null, 'remote details wait until the restored host is selected');
});

test('explicit local selection keeps completed remote records hidden', async () => {
  const page = restorePanel(true);
  page.latest.current['/case'] = {
    case: '/case',
    run_id: 'done',
    location: 'remote',
    status: 'succeeded',
    running: false,
    progress: { host: 'A', remote: { host: 'A', job: 'done' } },
    logs: [],
    stages: {},
  };
  page.select('');
  await flush();
  assert.equal(page.remote, null);
  assert.equal(page.latest.current['/case'].status, 'succeeded');
  assert.equal(page.latest.current['/case'].running, false);
  assert.equal(page.requests.length, 0);
});
