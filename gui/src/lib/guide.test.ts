import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { bilingual } from './bilingual';
import {
  assistantPanelWidth,
  pageExecutionContext,
  publicUiResult,
  waitForContract,
} from './utils';

test('page navigation waits for the actual lazy page contract', async () => {
  let frames = 0;
  await waitForContract(
    () => frames === 4,
    async () => {
      frames++;
    },
  );
  assert.equal(frames, 4);
  await assert.rejects(
    waitForContract(
      () => false,
      async () => {},
      0,
    ),
    /did not become ready/,
  );
});

test('actual page kernel is explicit and disappears when leaving its page', () => {
  assert.deepEqual(
    pageExecutionContext([{}, { kernelDir: '/kernels/usgs', preprocessors: 'fortran' }]),
    { kernelDir: '/kernels/usgs', preprocessors: 'fortran' },
  );
  assert.equal(pageExecutionContext([{ kernelDir: null }, {}]).kernelDir, null);
  assert.equal(pageExecutionContext([{}]).kernelDir, null);
});

test('results, run history and model pages cannot guess a boot kernel', () => {
  for (const step of ['results', 'runs', 'versions']) {
    const execution = pageExecutionContext([{ step } as { kernelDir?: string }]);
    assert.deepEqual(execution, { kernelDir: null, preprocessors: null });
  }
});

test('assistant outcomes preserve failure evidence while omitting credentials and bounding logs', () => {
  const result = {
    record: { host: 'server', job: 'j1', ssh_auth: { password: 'secret' } },
    status: { state: 'finished', exit_code: 1 },
    api_key: 'secret',
    logs: 'password=secret\nfailed',
  };
  assert.deepEqual(publicUiResult(result), {
    record: { host: 'server', job: 'j1' },
    status: { state: 'finished', exit_code: 1 },
    logs: 'password=[redacted]\nfailed',
  });
  assert.equal(result.record.ssh_auth.password, 'secret');
  assert.equal(publicUiResult('Authorization: Bearer secret'), 'Authorization=[redacted]');
  assert.deepEqual(
    publicUiResult(JSON.stringify({ exit_code: 0, authorization: 'secret' })),
    JSON.stringify({ exit_code: 0 }),
  );
  assert.match(String(publicUiResult('x'.repeat(16002))), /truncated 2 characters/);
  const sample = publicUiResult(Array.from({ length: 1002 }, (_, i) => i)) as {
    items: number[];
    omitted_items: number;
  };
  assert.equal(sample.items.length, 1000);
  assert.equal(sample.omitted_items, 2);
});

test('assistant width keeps both panes readable across drag limits and small windows', () => {
  assert.equal(assistantPanelWidth(320, 1440), 320);
  assert.equal(assistantPanelWidth(1000, 1440), 600);
  assert.equal(assistantPanelWidth(600, 960), 432);
  assert.equal(assistantPanelWidth(100, 960), 280);
  assert.equal(assistantPanelWidth(NaN, 960), 320);
});

test('the actual guide isolates modal controls from parent pages and restores navigation after closing', async () => {
  const source = readFileSync(new URL('./guide.ts', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace(/^export /gm, '');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const guide = new Function(
    'pageExecutionContext',
    'publicUiResult',
    'waitForContract',
    'requestAnimationFrame',
    'bilingual',
    `${js};return {pages,guideState,drive,openPage,bind:value=>{app=value;}};`,
  )(pageExecutionContext, publicUiResult, waitForContract, (fn: () => void) => fn(), bilingual);
  const shell = Symbol(),
    parent = Symbol(),
    modal = Symbol();
  let navigations = 0,
    backgroundClicks = 0;
  guide.pages.set(shell, {
    title: 'Shell',
    step: 'shell',
    scope: 'shell',
    status: 'shell-ready',
    fields: [],
    actions: [
      {
        id: 'nav-project',
        label: 'Project',
        kind: 'click',
        run: () => {
          backgroundClicks++;
        },
      },
    ],
  });
  guide.pages.set(parent, {
    title: 'Case',
    step: 'case',
    status: 'case-ready',
    kernelDir: '/case-kernel',
    fields: [
      {
        id: 'case-tab',
        label: 'Tab',
        value: 'results',
        type: 'choice',
        set: () => {
          backgroundClicks++;
        },
      },
    ],
    actions: [
      {
        id: 'case-tab-settings',
        label: 'Settings',
        kind: 'click',
        run: () => {
          backgroundClicks++;
        },
      },
    ],
  });
  guide.pages.set(modal, {
    title: 'Evaluation settings',
    step: 'results',
    modal: true,
    status: 'modal-ready',
    details: { dialog: true },
    fields: [
      { id: 'result-observation', label: 'Observations', value: '', type: 'text', set: () => {} },
    ],
    actions: [
      {
        id: 'result-settings-close',
        label: 'Close',
        kind: 'click',
        run: () => guide.pages.delete(modal),
      },
    ],
  });
  const state = { route: 'case', busy: false };
  guide.bind({
    state,
    navigate: (route: string) => {
      navigations++;
      state.route = route;
      guide.pages.set(parent, { title: 'Project', step: route, fields: [], actions: [] });
      return true;
    },
  });
  const snapshot = guide.guideState();
  assert.equal(snapshot.title, 'Evaluation settings');
  assert.equal(snapshot.kernelDir, null);
  assert.deepEqual(snapshot.status, ['modal-ready']);
  assert.deepEqual(snapshot.details, [{ dialog: true }]);
  assert.deepEqual(
    snapshot.fields.map((field: { id: string }) => field.id),
    ['result-observation'],
  );
  assert.deepEqual(
    snapshot.buttons.map((button: { name: string }) => button.name),
    ['result-settings-close'],
  );
  assert.deepEqual(snapshot.steps, []);
  await assert.rejects(guide.drive('click', { target: 'nav-project' }), /unique visible control/);
  await assert.rejects(
    guide.drive('fill', { fields: [{ field: 'case-tab', value: 'settings' }] }),
    /unique visible control/,
  );
  await assert.rejects(guide.drive('go', { step: 'project' }), /Close the current dialog/);
  assert.equal(navigations, 0);
  assert.equal(backgroundClicks, 0);
  await guide.drive('click', { target: 'result-settings-close' });
  assert.ok(
    guide.guideState().buttons.some((button: { name: string }) => button.name === 'nav-project'),
  );
  assert.ok(guide.guideState().steps.length);
  await guide.drive('go', { step: 'project' });
  assert.equal(navigations, 1);
});

// 批量运行页也在 project 路由下：助手 `ui_go('project')` 要回到项目首页，而不是等 10 秒超时（GUI 审查 S05）。
test('going to the project page from the batch run page returns to the project home', async () => {
  const source = readFileSync(new URL('./guide.ts', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace(/^export /gm, '');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const guide = new Function(
    'pageExecutionContext',
    'publicUiResult',
    'waitForContract',
    'requestAnimationFrame',
    'bilingual',
    `${js};return {pages,drive,bind:value=>{app=value;}};`,
  )(pageExecutionContext, publicUiResult, waitForContract, (fn: () => void) => fn(), bilingual);
  const page = Symbol();
  guide.pages.set(page, { title: 'Batch', step: 'project-batch-run', fields: [], actions: [] });
  const state = {
    route: 'project',
    root: '/p',
    busy: false,
    projectView: { root: '/p', page: 'batch', home: false },
  };
  guide.bind({
    state,
    navigate: () => true,
    update: (patch: { projectView: typeof state.projectView }) => {
      state.projectView = patch.projectView;
      guide.pages.set(page, { title: 'Project', step: 'project', fields: [], actions: [] });
    },
  });
  await guide.drive('go', { step: 'project' });
  assert.equal(state.projectView.home, true);
});

test('assistant buttons are exactly the visible ones, and only those inside an open dialog', async () => {
  // guide.ts 经 app-state 在加载时读 localStorage；node 下没有它。
  Object.assign(globalThis, {
    localStorage: globalThis.localStorage ?? { getItem: () => null, setItem() {} },
  });
  const { registerRenderedAction, renderedButtons } = await import('./guide');
  type Fake = {
    isConnected: boolean;
    disabled?: boolean;
    visible: boolean;
    inside?: Fake;
    getClientRects: () => unknown[];
    closest: () => null;
    contains: (other: Fake) => boolean;
  };
  const fake = (visible: boolean, fields: Partial<Fake> = {}): Fake => {
    const element: Fake = {
      isConnected: true,
      visible,
      getClientRects: () => (element.visible ? [{}] : []),
      closest: () => null,
      contains: (other) => other.inside === element,
      ...fields,
    };
    return element;
  };
  const saved = globalThis.document;
  let dialogs: Fake[] = [];
  Object.assign(globalThis, { document: { querySelectorAll: () => dialogs } });
  try {
    const run = () => 'ran';
    const shown = fake(true);
    const hidden = fake(false);
    const off = fake(true, { disabled: true });
    const dialog = fake(true);
    const inDialog = fake(true, { inside: dialog });
    const unregister = [
      registerRenderedAction(
        { id: 'shown', label: 'Shown', kind: 'click', run },
        () => shown as never,
      ),
      registerRenderedAction(
        { id: 'hidden', label: 'Hidden', kind: 'click', run },
        () => hidden as never,
      ),
      registerRenderedAction({ id: 'off', label: 'Off', kind: 'commit', run }, () => off as never),
      registerRenderedAction({ id: 'gone', label: 'Gone', kind: 'click', run }, () => null),
      registerRenderedAction(
        { id: 'dlg', label: 'In dialog', kind: 'click', run },
        () => inDialog as never,
      ),
    ];
    assert.deepEqual(
      renderedButtons().map((a) => [a.id, a.enabled]),
      [
        ['shown', true],
        ['off', false],
        ['dlg', true],
      ],
    );
    dialogs = [dialog];
    assert.deepEqual(
      renderedButtons().map((a) => a.id),
      ['dlg'],
      'an open dialog hides the page behind it',
    );
    unregister.forEach((done) => done());
    assert.deepEqual(renderedButtons(), []);
  } finally {
    Object.assign(globalThis, { document: saved });
  }
});

test('page details fit the assistant budget by dropping the largest entries first', () => {
  const source = readFileSync(new URL('./guide.ts', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace(/^export /gm, '');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const { budgetUiDetails } = new Function(
    'pageExecutionContext',
    'publicUiResult',
    'waitForContract',
    'requestAnimationFrame',
    'bilingual',
    `${js};return {budgetUiDetails};`,
  )(pageExecutionContext, publicUiResult, waitForContract, (fn: () => void) => fn(), bilingual) as {
    budgetUiDetails: (details: unknown, limit?: number) => unknown;
  };
  const small = [{ studyDir: '/s', status: 'ok' }];
  assert.deepEqual(budgetUiDetails(small, 1000), small);
  const big = [{ studyDir: '/s', huge: 'x'.repeat(5000), medium: 'y'.repeat(800) }];
  const fitted = budgetUiDetails(big, 1200) as Record<string, unknown>[];
  assert.equal(fitted[0]!.studyDir, '/s');
  assert.equal(fitted[0]!.medium, 'y'.repeat(800), 'only as much is dropped as needed');
  assert.match(
    JSON.stringify(fitted[0]!.huge),
    /5002 characters over the assistant context budget/,
  );
  assert.ok(JSON.stringify(fitted).length <= 1200);
});
