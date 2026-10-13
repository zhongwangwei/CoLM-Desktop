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
    setMetrics: noop,
    setDiagnostic: noop,
    setFrom: noop,
    setTo: noop,
    setSpinup: noop,
    setCorrected: noop,
    setComparison: noop,
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
    observationEdited: { current: false },
    automaticObservation: { current: '/default.nc' },
    automaticEvaluation: { current: '' },
    automaticEvaluator: {
      current: execute(evaluator, { run: (work: () => void) => work(), evaluate }),
    },
  };
  execute(body, { ...context, catalogKey: '' })();
  execute(body, { ...context, busy: true })();
  assert.equal(evaluations, 0);
  execute(body, context)();
  execute(body, context)();
  assert.equal(evaluations, 1);
  context.observationEdited.current = true;
  execute(body, { ...context, selectionKey: 'manual' })();
  assert.equal(evaluations, 1);
});
