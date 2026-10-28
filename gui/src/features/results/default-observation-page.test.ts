import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
const path = new URL('./ResultsPage.tsx', import.meta.url);
const source = ts.createSourceFile(
  path.pathname,
  readFileSync(path, 'utf8'),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const effects: string[] = [];
let choose = '',
  evaluator = '';
function visit(node: ts.Node) {
  if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect')
    effects.push(node.arguments[0].getText(source));
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'chooseObservation')
    choose = node.initializer!.getText(source);
  if (ts.isBinaryExpression(node) && node.left.getText(source) === 'automaticEvaluator.current')
    evaluator = node.right.getText(source);
  ts.forEachChild(node, visit);
}
visit(source);
function execute(expression: string, context: Record<string, unknown>) {
  const js = ts.transpileModule(`const callback = (${expression});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return callback;`)(
    ...Object.values(context),
  );
}
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function initialization() {
  let resolve!: (path: string) => void;
  const pending = new Promise<string>((done) => {
    resolve = done;
  });
  const written: string[] = [];
  const noop = () => {};
  const context = {
    epoch: { current: 0 },
    requestSeq: { current: { single: 0, multi: 0 } },
    stationTarget: { current: '/case' },
    stationRetry: { attempt: 0, error: '' },
    cache: { current: new Map() },
    caseDir: '/case',
    versionId: 'official',
    revision: 1,
    completedRunCase: undefined,
    observationEdited: { current: false },
    automaticObservation: { current: '' },
    automaticEvaluation: { current: '' },
    defaultObservation: () => pending,
    invokeJson: async () => [],
    setObs: (value: string) => written.push(value),
    setRuns: noop,
    setSource: noop,
    setCatalog: noop,
    setSeries: noop,
    setSingleResult: noop,
    setComparisonResult: noop,
    setDiagnostic: noop,
    setFrom: noop,
    setTo: noop,
    setSpinup: noop,
    setCorrected: noop,
    setComparisonEntries: noop,
    setComparisonProgress: noop,
    setError: noop,
  };
  const body = effects.find((effect) => effect.includes('defaultObservation(caseDir)'));
  assert.ok(body);
  const cleanup = execute(body, context)();
  return { context, written, resolve, cleanup };
}
test('results load the default dataset while preserving manual selection and clearing', async () => {
  const automatic = initialization();
  automatic.resolve('/default.nc');
  await tick();
  assert.equal(automatic.written.at(-1), '/default.nc');
  assert.equal(automatic.context.automaticObservation.current, '/default.nc');
  for (const selection of ['/manual.nc', '']) {
    const state = initialization();
    execute(choose, state.context)(selection);
    state.resolve('/late-default.nc');
    await tick();
    assert.deepEqual(state.written, ['', selection]);
    assert.equal(state.context.automaticObservation.current, '');
  }
});
test('a departed case cannot populate the next case with its delayed observation', async () => {
  const old = initialization();
  old.cleanup();
  old.resolve('/old-case.nc');
  await tick();
  assert.deepEqual(old.written, ['']);
});
test('default observation uses the existing evaluator once its run catalog is ready', () => {
  let evaluations = 0;
  const evaluate = () => {
    evaluations++;
  };
  const body = effects.find((effect) => effect.includes('automaticEvaluator.current()'));
  assert.ok(body);
  const context = {
    busy: false,
    source: '/run',
    catalogKey: '/run',
    obs: '/default.nc',
    selectionKey: 'run/default',
    // 自动评估只属于单站范围；多站比较里改 spinup 不能触发单站评估顶掉比较结果。
    multiStation: false,
    observationEdited: { current: false },
    automaticObservation: { current: '/default.nc' },
    automaticEvaluation: { current: '' },
    automaticEvaluator: {
      current: execute(evaluator, { run: (work: () => void) => work(), evaluate }),
    },
  };
  execute(body, { ...context, catalogKey: '' })();
  execute(body, { ...context, busy: true })();
  execute(body, { ...context, multiStation: true })();
  assert.equal(evaluations, 0);
  execute(body, context)();
  execute(body, context)();
  assert.equal(evaluations, 1);
  context.observationEdited.current = true;
  execute(body, { ...context, selectionKey: 'manual' })();
  assert.equal(evaluations, 1);
});

test('finishing or starting a run keeps the chosen observations, window and metrics', async () => {
  const calls: { body: string; deps: string }[] = [];
  const walk = (node: ts.Node) => {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect')
      calls.push({
        body: node.arguments[0].getText(source),
        deps: node.arguments[1]?.getText(source) ?? '',
      });
    ts.forEachChild(node, walk);
  };
  walk(source);
  const reset = calls.find((call) => call.body.includes('defaultObservation(caseDir)'));
  assert.ok(reset);
  assert.ok(!reset.deps.includes('completedRunCase'), reset.deps);
  const refresh = calls.find(
    (call) => call.deps.includes('completedRunCase') && call.body.includes('model_case_runs'),
  );
  assert.ok(refresh, 'a separate effect follows finished runs');
  const chosen: string[] = [];
  let runs: unknown[] = [];
  // Only these names exist: touching observations, the window or metrics would throw.
  execute(refresh.body, {
    caseDir: '/case',
    completedRunCase: '/case/runs/new',
    stationTarget: { current: '/case' },
    invokeJson: async () => [{ run_case: '/case/runs/old' }, { run_case: '/case/runs/new' }],
    setRuns: (value: unknown[]) => (runs = value),
    setSource: (value: string) => chosen.push(value),
  })();
  await tick();
  assert.equal(runs.length, 2);
  assert.deepEqual(chosen, ['/case/runs/new']);
  // A run that just started (no completed output yet) changes nothing.
  const idle = execute(refresh.body, { caseDir: '/case', completedRunCase: undefined })();
  assert.equal(idle, undefined);
});

test('a server run is read in place, and fetching takes only that run', async () => {
  const bodies: string[] = [];
  let fetchBody = '';
  const walk = (node: ts.Node) => {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect')
      bodies.push(node.arguments[0].getText(source));
    if (ts.isFunctionDeclaration(node) && node.name?.getText(source) === 'fetchSelectedRun')
      fetchBody = node
        .getText(source)
        .replace(/^async function fetchSelectedRun/, 'async function');
    ts.forEachChild(node, walk);
  };
  walk(source);
  const catalog = bodies.find((body) => body.includes("'history_catalog'"));
  assert.ok(catalog && fetchBody);
  const calls: { command: string; args: unknown }[] = [];
  const noop = () => {};
  const fetching: boolean[] = [];
  const context = (runs: unknown[]) => ({
    setFetching: (value: boolean) => fetching.push(value),
    epoch: { current: 0 },
    stationTarget: { current: '/case' },
    caseDir: '/case',
    source: '/case/runs/r1',
    runs,
    invokeJson: async (command: string, args: unknown) => {
      calls.push({ command, args });
      return command === 'model_case_runs' ? [] : { variables: [] };
    },
    overviewVariable: () => '',
    setError: noop,
    setCatalog: noop,
    setSeries: noop,
    setSingleResult: noop,
    setComparisonResult: noop,
    setDiagnostic: noop,
    setCatalogKey: noop,
    setVariable: noop,
  });
  const remote = { run_case: '/case/runs/r1', server: 'tianhe', outputs_local: false };
  // The backend reads it on the server (remote-read); the page asks for it like any other run.
  execute(catalog, context([remote]))();
  await tick();
  assert.deepEqual(
    calls.map((call) => call.command),
    ['history_catalog'],
  );
  calls.length = 0;
  let runs: unknown;
  const fetch = execute(fetchBody, {
    ...context([remote]),
    task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    setRuns: (value: unknown) => {
      runs = value;
    },
  });
  await fetch();
  assert.deepEqual(calls, [
    {
      command: 'remote_fetch',
      args: { case: '/case/runs/r1', host: null, all: false, overwriteNewer: false },
    },
    { command: 'model_case_runs', args: { dir: '/case' } },
  ]);
  assert.deepEqual(runs, []);
  // 按钮只在真取回时写"正在取回…"，结束后复原。
  assert.deepEqual(fetching, [true, false]);
});
