import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { activeNavigation, navigation, navigationTarget } from './navigation';

test('sidebar groups model/data work under Project and opens real case tabs', () => {
  assert.deepEqual(
    navigation.map((item) => item.zh),
    ['项目', '模拟', '结果', '研究', '历史记录', '计算资源', '设置'],
  );
  for (const route of ['project', 'preparation', 'versions'] as const)
    assert.equal(activeNavigation({ route, caseTab: 'results' }), 'project');
  assert.equal(activeNavigation({ route: 'new-case', caseTab: 'settings' }), 'simulation');
  for (const tab of ['settings', 'runs'] as const)
    assert.equal(activeNavigation({ route: 'case', caseTab: tab }), 'simulation');
  assert.equal(activeNavigation({ route: 'case', caseTab: 'results' }), 'results');
  assert.deepEqual(
    navigation.find((item) => item.id === 'results'),
    { id: 'results', zh: '结果', en: 'Results', route: 'case', tab: 'results' },
  );
  assert.equal(activeNavigation({ route: 'resources', caseTab: 'results' }), 'resources');
});

test('Simulation returns to the current batch after visiting results or another sidebar page', () => {
  const simulation = navigation.find((item) => item.id === 'simulation')!;
  const results = navigation.find((item) => item.id === 'results')!;
  const state = {
    root: '/multi',
    projectView: { root: '/multi', page: 'batch' as const },
  };
  assert.deepEqual(navigationTarget(simulation, state), {
    route: 'project',
    projectView: { ...state.projectView, home: false },
  });
  assert.deepEqual(navigationTarget(results, state), { route: 'case', tab: 'results' });
  assert.equal(activeNavigation({ ...state, route: 'project', caseTab: 'results' }), 'simulation');
  assert.deepEqual(navigationTarget(simulation, { ...state, root: '/other' }), {
    route: 'case',
    tab: 'settings',
  });
  assert.deepEqual(navigationTarget(simulation, { root: '/multi' }), {
    route: 'case',
    tab: 'settings',
  });
});

test('Project opens home while Simulation preserves and restores the stopped batch bookmark', () => {
  const project = navigation.find((item) => item.id === 'project')!;
  const simulation = navigation.find((item) => item.id === 'simulation')!;
  for (const running of [true, false]) {
    const state = {
      root: '/multi',
      projectView: { root: '/multi', page: 'batch' as const, fromCreation: true },
      running,
    };
    const home = navigationTarget(project, state);
    assert.deepEqual(home, { route: 'project', projectView: { ...state.projectView, home: true } });
    const atHome = { ...state, ...home, caseTab: 'settings' as const };
    assert.equal(activeNavigation(atHome), 'project');
    const restored = navigationTarget(simulation, atHome);
    assert.deepEqual(restored, {
      route: 'project',
      projectView: { ...state.projectView, home: false },
    });
    assert.equal(activeNavigation({ ...atHome, ...restored }), 'simulation');
  }
  assert.deepEqual(
    navigationTarget(project, { root: '/other', projectView: { root: '/multi', page: 'batch' } }),
    { route: 'project' },
  );
});

test('sidebar applies the Project view only after navigation is allowed', () => {
  const source = ts.createSourceFile(
    'App.tsx',
    readFileSync(new URL('../App.tsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'openNavigation')
      body = node.initializer!.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const code = ts.transpileModule(`const open = ${body};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const bookmark = { root: '/multi', page: 'batch' as const, fromCreation: true };
  const state = { root: '/multi', projectView: bookmark };
  const updates: unknown[] = [];
  let allowed = false;
  const open = new Function(
    'state',
    'navigationTarget',
    'navigate',
    'update',
    `${code}; return open;`,
  )(
    state,
    navigationTarget,
    () => allowed,
    (patch: unknown) => updates.push(patch),
  ) as (item: (typeof navigation)[number]) => boolean;
  const project = navigation.find((item) => item.id === 'project')!;
  assert.equal(open(project), false);
  assert.deepEqual(updates, []);
  allowed = true;
  assert.equal(open(project), true);
  assert.deepEqual(updates, [
    { projectView: { ...bookmark, home: true }, projectCasesOpen: false },
  ]);
});
