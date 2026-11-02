import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { selectionAfterPrune, withoutRevisions } from './model-tidy';
import { closeReasons, navigationBlock } from './navigation-guard';

function source(path: string) {
  return ts.createSourceFile(
    path,
    readFileSync(new URL(path, import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
}
function expression(ast: ts.SourceFile, match: (node: ts.Node) => boolean) {
  let found: ts.Node | undefined;
  const visit = (node: ts.Node) => {
    if (match(node)) found = node;
    ts.forEachChild(node, visit);
  };
  visit(ast);
  assert.ok(found);
  return found.getText(ast);
}
function evaluate<T>(code: string, context: Record<string, unknown>): T {
  const js = ts.transpileModule(`const result = (${code});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return result;`)(
    ...Object.values(context),
  ) as T;
}

test('manual cache pruning replaces a removed explicit choice and preserves no explicit choice', async () => {
  const ast = source('../features/settings/SettingsPage.tsx');
  const runCode = expression(
    ast,
    (node) =>
      ts.isArrowFunction(node) &&
      ts.isVariableDeclaration(node.parent) &&
      node.parent.name.getText(ast) === 'run',
  );
  for (const chosenVersion of [{ id: 'official', revision: 1 }, null]) {
    let state = {
      versions: [1, 2].map((revision) => ({ version: { id: 'official', revision } })),
      versionRef: { id: 'official', revision: 1 },
      chosenVersion,
    };
    const run = evaluate<(dry: boolean) => Promise<void>>(runCode, {
      setBusy: () => {},
      setMessage: () => {},
      setReport: () => {},
      invokeJson: async () => ({ removed: [{ id: 'official', revision: 1 }], bytes: 0 }),
      update: (patch: (old: typeof state) => Partial<typeof state>) => {
        state = { ...state, ...patch(state) };
      },
      selectionAfterPrune,
      withoutRevisions,
      formatBytes: () => '0',
      t: (zh: string) => zh,
    });
    await run(false);
    assert.deepEqual(state.versionRef, { id: 'official', revision: 2 });
    assert.deepEqual(state.chosenVersion, chosenVersion ? { id: 'official', revision: 2 } : null);
  }
});

test('a completion seen on Compare does not notify later when leaving the page', () => {
  const ast = source('../App.tsx');
  const effect = expression(
    ast,
    (node) =>
      ts.isArrowFunction(node) &&
      ts.isCallExpression(node.parent) &&
      node.parent.expression.getText(ast) === 'useEffect' &&
      node.getText(ast).includes('notifiedBenchTasks.current'),
  );
  const state = { route: 'compare', language: 'zh' };
  const seen = new Set<string>();
  let updates = 0;
  const run = evaluate<() => void>(effect, {
    state,
    notifiedBenchTasks: { current: seen },
    benchTasks: { tasks: [{ id: 'finished', state: 'done' }] },
    update: () => {
      updates++;
    },
    benchTaskNotice: () => 'done',
  });
  run();
  assert.equal(seen.has('finished'), true);
  state.route = 'project';
  run();
  assert.equal(updates, 0);
});

test('operation close guards remain registered until release without blocking navigation', () => {
  const ast = source('./app-state.tsx');
  const registerCode = expression(
    ast,
    (node) =>
      ts.isCallExpression(node) &&
      ts.isVariableDeclaration(node.parent) &&
      node.parent.name.getText(ast) === 'registerCloseGuard',
  );
  const closeGuards = { current: new Set<() => string | null>() };
  const register = evaluate<(guard: () => string | null) => () => void>(registerCode, {
    closeGuards,
    useCallback: (fn: unknown) => fn,
  });
  const release = register(() => 'Background operation is still running');
  assert.equal(navigationBlock({ route: 'history' }, { route: 'project' }, []), null);
  assert.deepEqual(closeReasons(closeGuards.current, {}, String), [
    'Background operation is still running',
  ]);
  release();
  assert.deepEqual(closeReasons(closeGuards.current, {}, String), []);
});

test('local OpenBench task and operation guards produce one close warning in either language', () => {
  for (const language of ['zh', 'en']) {
    const app = source('../App.tsx');
    const compare = source('../features/compare/ComparePage.tsx');
    const appGuard = evaluate<() => string | null>(
      expression(
        app,
        (node) =>
          ts.isCallExpression(node) &&
          node.expression.getText(app) === 'useCloseGuard' &&
          node.getText(app).includes('benchTasks.tasks'),
      ),
      {
        useCloseGuard: (guard: unknown) => guard,
        benchTasks: { tasks: [{ host: '', state: 'running' }] },
        state: { language },
      },
    );
    const operationGuard = evaluate<() => string | null>(
      expression(
        compare,
        (node) =>
          ts.isCallExpression(node) && node.expression.getText(compare) === 'registerCloseGuard',
      ),
      {
        registerCloseGuard: (guard: unknown) => guard,
        host: '',
        t: (zh: string, en: string) => (language === 'en' ? en : zh),
      },
    );
    assert.ok(appGuard());
    assert.deepEqual(closeReasons([appGuard, operationGuard], {}, String), [appGuard()]);
  }
});
