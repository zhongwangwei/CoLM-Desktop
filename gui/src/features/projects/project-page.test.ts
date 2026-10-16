import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { choosableVersions, latestOfficial } from '../versions/version-tree';

function choose(context: Record<string, unknown>) {
  const path = new URL('./ProjectPage.tsx', import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'chooseVersion')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const code = ts.transpileModule(`const fn=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${code};return fn;`)(...Object.values(context)) as (
    key: string,
  ) => Promise<unknown>;
}
test('project selection reads the precise frozen revision and rejects unknown or late selections', async () => {
  const model = { version: { id: 'soil', revision: 4 }, name: 'Soil experiment' },
    updates: unknown[] = [],
    calls: unknown[] = [];
  const active = { current: true };
  const context = {
    work: async (fn: () => Promise<unknown>) => fn(),
    state: { versions: [model] },
    versionKey: (value: typeof model) => JSON.stringify(value.version),
    invokeJson: async (command: string, args: unknown) => {
      calls.push({ command, args });
      return model;
    },
    active,
    setVersionChooser: () => {},
    update: (value: unknown) => updates.push(value),
    t: (zh: string) => zh,
  };
  const select = choose(context),
    key = JSON.stringify(model.version);
  assert.deepEqual(await select(key), { version: model.version, name: model.name });
  assert.deepEqual(calls, [{ command: 'model_get', args: model.version }]);
  assert.deepEqual(updates, [
    { version: model, versionRef: model.version, chosenVersion: model.version },
  ]);
  await assert.rejects(select('unknown'));
  assert.equal(calls.length, 1);
  active.current = false;
  await assert.rejects(select(key));
  assert.equal(updates.length, 1, 'a late selection cannot replace the model on a different page');
});

function expression(name: string, context: Record<string, unknown>, page = './ProjectPage.tsx') {
  const path = new URL(page, import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name && node.initializer)
      body = node.initializer.getText(source);
    if (
      name === 'contract' &&
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'usePageContract'
    )
      body = node.arguments[0].getText(source);
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const code = ts.transpileModule(`const value=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${code};return value;`)(...Object.values(context));
}

test('opening a case does not navigate again after a late completion', async () => {
  let release!: () => void;
  const completion = new Promise<void>((resolve) => {
    release = resolve;
  });
  let route = 'project';
  const opened: unknown[] = [];
  const chooseCase = expression('chooseCase', {
    work: (fn: () => unknown) => fn(),
    openCase: async (dir: string, tab: string) => {
      opened.push([dir, tab]);
      route = 'case';
      await completion;
      return { dir };
    },
    setChooser: () => {},
    navigate: (next: string) => {
      route = next;
    },
  });
  const pending = chooseCase('/project/site', 'results');
  assert.equal(route, 'case');
  route = 'research';
  release();
  await pending;
  assert.equal(route, 'research');
  assert.deepEqual(opened, [['/project/site', 'results']]);
});
test('version modal closes only after a successful frozen revision read', async () => {
  const model = { version: { id: 'soil', revision: 4 }, name: 'Soil experiment' },
    events: string[] = [];
  let fail = false;
  const select = choose({
    work: async (fn: () => Promise<unknown>) => fn(),
    state: { versions: [model] },
    versionKey: (value: typeof model) => JSON.stringify(value.version),
    invokeJson: async () => {
      events.push('read');
      if (fail) throw new Error('unavailable');
      return model;
    },
    active: { current: true },
    update: () => events.push('update'),
    setVersionChooser: (open: boolean) => events.push(open ? 'open' : 'close'),
    t: (zh: string) => zh,
  });
  await select(JSON.stringify(model.version));
  assert.deepEqual(events, ['read', 'update', 'close']);
  events.length = 0;
  fail = true;
  await assert.rejects(select(JSON.stringify(model.version)), /unavailable/);
  assert.deepEqual(events, ['read']);
  const lock = { current: false };
  const close = expression('changeVersionChooser', {
    lock,
    state: { busy: false },
    setError: () => {},
    setVersionChooser: (open: boolean) => events.push(open ? 'open' : 'close'),
  });
  close(false);
  assert.deepEqual(events, ['read', 'close'], 'cancel does not read or update the model');
  lock.current = true;
  close(false);
  assert.equal(events.length, 2, 'the modal stays open during a model read');
});
test('project contract exposes only version dialog actions while the dialog is open', async () => {
  const model = { version: { id: 'soil', revision: 4 }, name: 'Soil experiment' },
    chosen: string[] = [];
  const versionActions = expression('versionActions', {
    state: { versions: [model] },
    versionRows: [{ version: model, depth: 0 }],
    versionKey: (value: typeof model) => JSON.stringify(value.version),
    versionEnabled: true,
    chooseVersion: async (key: string) => chosen.push(key),
    changeVersionChooser: () => {},
    t: (zh: string) => zh,
  });
  const context = {
    versionChooser: true,
    chooser: null,
    busy: false,
    error: '',
    modelChosen: false,
    t: (zh: string) => zh,
    versionActions,
    chooserActions: [{ id: 'case' }],
    actions: [{ id: 'project-existing' }],
  };
  const modal = expression('contract', context);
  assert.equal(modal.modal, true);
  assert.deepEqual(modal.fields, []);
  assert.deepEqual(
    modal.actions.map((action: { id: string }) => action.id),
    [`project-version:${JSON.stringify(model.version)}`, 'project-version-close'],
  );
  await modal.actions[0].run();
  assert.deepEqual(chosen, [JSON.stringify(model.version)]);
  const home = expression('contract', { ...context, versionChooser: false });
  assert.equal(home.modal, false);
  assert.deepEqual(home.fields, []);
  assert.deepEqual(home.actions, [{ id: 'project-existing' }]);
  const cases = expression('contract', {
    ...context,
    versionChooser: false,
    chooser: 'simulation',
  });
  assert.deepEqual(cases.actions, [{ id: 'case' }]);
});

test('version cards distinguish latest official from older official and modified revisions', () => {
  const official = { version: { id: 'official', revision: 3 } };
  for (const [versionRef, expectedOfficial] of [
    [{ id: 'official', revision: 3 }, true],
    [{ id: 'official', revision: 2 }, false],
    [{ id: 'soil', revision: 4 }, false],
  ] as const) {
    const officialSelected = expression('officialSelected', {
      modelChosen: true,
      state: { versionRef },
      official,
    });
    assert.equal(officialSelected, expectedOfficial);
    assert.equal(
      expression('existingSelected', { modelChosen: true, officialSelected }),
      !expectedOfficial,
    );
  }
  assert.equal(
    expression('officialSelected', {
      modelChosen: false,
      state: { versionRef: official.version },
      official,
    }),
    false,
  );
  assert.equal(
    expression('existingSelected', { modelChosen: false, officialSelected: false }),
    false,
  );
});

function home() {
  const model = { version: { id: 'official', revision: 4 }, name: 'Official CoLM' };
  const state = {
    language: 'zh',
    root: '/project',
    busy: false,
    selectedCase: null,
    cases: [],
    versions: [model],
    version: model,
    versionRef: model.version,
    chosenVersion: null as null | { id: string; revision: number },
  };
  const text = readFileSync(new URL('./ProjectPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(text, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  let slots: unknown[] = [],
    cursor = 0,
    contract: { actions: { id: string; enabled?: boolean; run: () => unknown }[]; status: string },
    fail = false;
  const calls: unknown[] = [],
    routes: string[] = [];
  const context = {
    React: { createElement: () => null },
    Button: () => {},
    Dialog: () => {},
    Notice: () => {},
    NavIcon: () => {},
    useApp: () => ({
      state,
      update: (patch: Partial<typeof state>) => Object.assign(state, patch),
      navigate: (route: string) => {
        routes.push(route);
        return true;
      },
      openCase: () => {
        throw new Error('not requested');
      },
    }),
    hasBackend: () => false,
    useEffect: () => {},
    useRef: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = { current: initial };
      return slots[key];
    },
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (value: unknown) => {
          slots[key] = typeof value === 'function' ? value(slots[key]) : value;
        },
      ];
    },
    usePageContract: (value: typeof contract) => {
      contract = value;
    },
    recentProjectRoots: () => [],
    rememberProject: (_roots: unknown, root: string) => [root],
    choosableVersions,
    latestOfficial,
    projectName: (root: string) => root,
    invokeJson: async (command: string, args: unknown) => {
      calls.push({ command, args });
      if (fail) throw new Error('model read failed');
      return model;
    },
  };
  const Page = new Function(...Object.keys(context), `${code};return ProjectPage;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    Page();
    return contract;
  }
  async function click(id: string) {
    const action = contract.actions.find((item) => item.id === id);
    assert.ok(action);
    assert.notEqual(action.enabled, false, `${id} is enabled`);
    await action.run();
    return render();
  }
  function remount() {
    slots = [];
    return render();
  }
  return {
    state,
    model,
    calls,
    routes,
    render,
    click,
    remount,
    fail: () => {
      fail = true;
    },
  };
}
test('the loaded official model is usable at once; choosing another version is optional', async () => {
  const page = home(),
    tasks = [
      'project-new-simulation',
      'project-open',
      'project-view-results',
      'project-example',
      'project-prepare',
      'project-recent:/project',
    ];
  let contract = page.render();
  for (const id of tasks)
    assert.equal(contract.actions.find((action) => action.id === id)?.enabled, true, id);
  await page.click('project-existing');
  await page.click('project-version-close');
  contract = page.render();
  for (const id of tasks)
    assert.equal(
      contract.actions.find((action) => action.id === id)?.enabled,
      true,
      'closing the chooser keeps the current model',
    );
  await page.click('project-new-simulation');
  assert.deepEqual(page.routes, ['new-case']);
  page.state.versionRef = { id: 'official', revision: 5 };
  contract = page.render();
  for (const id of tasks)
    assert.equal(
      contract.actions.find((action) => action.id === id)?.enabled,
      false,
      'tasks wait while the selected revision is still being read',
    );
});
test('a failed version read keeps the current model and leaves the version modal open', async () => {
  const page = home();
  page.render();
  const before = page.state.versionRef;
  await page.click('project-existing');
  page.fail();
  await assert.rejects(
    page.click(`project-version:${JSON.stringify(page.model.version)}`),
    /model read failed/,
  );
  let contract = page.render();
  assert.deepEqual(page.state.versionRef, before);
  assert.ok(contract.actions.some((action) => action.id === 'project-version-close'));
  await page.click('project-version-close');
  contract = page.render();
  assert.equal(
    contract.actions.find((action) => action.id === 'project-new-simulation')?.enabled,
    true,
  );
});
test('model manager selection and successful freezing record the chosen frozen revision', async () => {
  const model = { version: { id: 'modified', revision: 2 } },
    updates: unknown[] = [];
  await expression(
    'select',
    {
      dirty: false,
      t: (zh: string) => zh,
      update: (patch: unknown) => updates.push(patch),
      setDraft: () => {},
    },
    '../versions/VersionsPage.tsx',
  )(model);
  assert.deepEqual(updates.pop(), {
    version: model,
    versionRef: model.version,
    chosenVersion: model.version,
  });
  await expression(
    'freeze',
    {
      researchVersion: async () => model,
      draft: {},
      dirtyNow: { current: false },
      selected: null,
      assetRoot: '',
      save: () => {},
      invokeJson: () => {},
      update: (patch: unknown) => updates.push(patch),
      setDraft: () => {},
      setDirty: () => {},
      refreshVersions: async () => {},
    },
    '../versions/VersionsPage.tsx',
  )();
  assert.deepEqual(updates.pop(), {
    version: model,
    versionRef: model.version,
    chosenVersion: model.version,
  });
});

test('bundled example is not rejected by case switches retained in a legacy frozen revision', async () => {
  const site = {
    name: 'CN-Cng',
    urban: false,
    crop: false,
    site_file: '/site.nc',
    met_file: '/met.nc',
    start: '2000-01-01',
    end: '2001-01-01',
  };
  const state = {
    root: '/project',
    version: {
      overlay: { physics: { DEF_URBAN_RUN: '.true.', DEF_TUNING_CROP_PLANTING_DAY: '0' } },
    },
  };
  const updates: unknown[] = [],
    routes: string[] = [];
  const example = expression('example', {
    state,
    work: async (fn: () => Promise<unknown>) => fn(),
    invoke: async (command: string) =>
      command === 'install_example'
        ? { sitedir: '/sites', forcingdir: '/forcing', root: '/examples', runtimedir: '/runtime' }
        : { sites: [site] },
    active: { current: true },
    lock: { current: true },
    update: (value: unknown) => updates.push(value),
    navigate: (route: string) => {
      routes.push(route);
      return true;
    },
    t: (zh: string) => zh,
  });
  await example();
  assert.deepEqual(routes, ['new-case']);
  assert.equal(updates.length, 1);
  assert.equal(state.version.overlay.physics.DEF_URBAN_RUN, '.true.');
});

test('project location displays shared server project without a selected case', () => {
  const state = {
    root: '/local/cache/opaque-hash',
    selectedCase: null,
    cases: [
      { run_host: 'compute', remote_directory: '/server/project/site-a' },
      { run_host: 'compute', remote_directory: '/server/project/site-b/' },
    ],
  };
  assert.equal(expression('projectLocation', { state }), 'compute: /server/project');
  assert.equal(state.root, '/local/cache/opaque-hash');
  for (const entry of [
    {},
    { run_host: 'other', remote_directory: '/server/project/site-c' },
    { run_host: 'compute', remote_directory: '/other/site-c' },
  ]) {
    assert.equal(
      expression('projectLocation', { state: { ...state, cases: [...state.cases, entry] } }),
      state.root,
    );
  }
  assert.equal(expression('projectLocation', { state: { ...state, cases: [] } }), state.root);
});
