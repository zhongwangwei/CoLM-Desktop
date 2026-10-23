import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  activeNavigation,
  currentProjectView,
  navigation,
  navigationTarget,
  projectName,
  projectToSave,
  readSavedProject,
  writeSavedProject,
} from './navigation';

test('sidebar groups model/data work under Project and opens real case tabs', () => {
  assert.deepEqual(
    navigation.map((item) => item.zh),
    ['项目', '模拟', '结果', '研究', '比较', '历史记录', '计算资源', '设置'],
  );
  // Research keeps its own sidebar entry; it is also reached from 新建修改版.
  for (const route of ['project', 'preparation', 'versions'] as const)
    assert.equal(activeNavigation({ route, caseTab: 'results' }), 'project');
  assert.equal(activeNavigation({ route: 'research', caseTab: 'results' }), 'research');
  assert.equal(activeNavigation({ route: 'compare', caseTab: 'results' }), 'compare');
  // The sidebar entry always lands on research home, also when research is already open.
  const research = navigation.find((item) => item.id === 'research')!;
  assert.deepEqual(navigationTarget(research, { root: '/p' }), {
    route: 'research',
    researchHome: true,
  });
  assert.equal(activeNavigation({ route: 'new-case', caseTab: 'settings' }), 'simulation');
  assert.equal(activeNavigation({ route: 'case', caseTab: 'settings' }), 'simulation');
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

test('the current project and its page survive navigation and restarts', () => {
  const store = new Map<string, string>();
  const storage = {
    getItem: (key: string) => store.get(key) ?? null,
    setItem: (key: string, value: string) => void store.set(key, value),
    removeItem: (key: string) => void store.delete(key),
  };
  assert.equal(readSavedProject(storage), null);
  const batch = {
    root: '/p',
    projectView: { root: '/p', page: 'batch' as const, fromCreation: true },
  };
  writeSavedProject(storage, projectToSave(batch, null));
  assert.deepEqual(readSavedProject(storage), { root: '/p', page: 'batch', fromCreation: true });

  // Going home or elsewhere keeps the saved page; returning opens it, not the home page.
  const away = { root: '/p', projectView: undefined };
  assert.deepEqual(projectToSave(away, readSavedProject(storage)), readSavedProject(storage));
  assert.deepEqual(currentProjectView(away, readSavedProject(storage)), {
    root: '/p',
    page: 'batch',
    fromCreation: true,
    home: false,
  });
  // Another project starts on its case list; closing the project forgets it.
  assert.deepEqual(
    currentProjectView({ root: '/q', projectView: undefined }, readSavedProject(storage)),
    {
      root: '/q',
      page: 'cases',
      home: false,
    },
  );
  writeSavedProject(storage, projectToSave({ root: '', projectView: undefined }, null));
  assert.equal(readSavedProject(storage), null);
  store.set('colm-ui-project', '{not json');
  assert.equal(readSavedProject(storage), null);
  store.set('colm-ui-project', JSON.stringify({ root: '/p', page: 'elsewhere' }));
  assert.equal(readSavedProject(storage), null);
});

test('projects are named by their server folder or their own folder', () => {
  assert.equal(
    projectName('/cache/remote-projects/abc123', [
      { run_host: 'tms17l', remote_directory: '/tera11/zhwei/colm-desktop/Plumber/AT-Neu' },
    ]),
    'tms17l: Plumber',
  );
  assert.equal(projectName('/Users/me/Desktop/my-project/', []), 'my-project');
  assert.equal(projectName('D:\\runs\\basin', []), 'basin');
});

test("a case's run records light up History in the sidebar", () => {
  assert.equal(activeNavigation({ route: 'case', caseTab: 'runs' }), 'history');
  assert.equal(activeNavigation({ route: 'case', caseTab: 'settings' }), 'simulation');
  assert.equal(activeNavigation({ route: 'case', caseTab: 'results' }), 'results');
});
