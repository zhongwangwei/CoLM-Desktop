import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

function screen(
  handoff?: {
    root: string;
    cases: string[];
    batch?: boolean;
    host: string;
    threads: string;
    ranks: string;
    workers: string;
    scheduler: string;
    cpus: string;
  },
  fromCreation = !!handoff,
) {
  const state = {
    language: 'en',
    root: '/project',
    projectRunSettings: handoff,
    busy: false,
    cases: [
      { dir: '/a', name: 'A' },
      { dir: '/b', name: 'B' },
    ],
    running: {} as Record<
      string,
      { running: boolean; status?: string; error?: string; logs?: string[] }
    >,
    versionRef: { id: 'official', revision: 2 },
    version: { name: 'Official' },
  };
  const slots: unknown[] = [];
  let cursor = 0;
  const effects = new Map<number, { deps: unknown[]; cleanup: () => void }>();
  let contract: {
    fields: { id: string; disabled: boolean; set: (v: string) => void }[];
    actions: { id: string; enabled: boolean; run: () => void }[];
  };
  const requests: { args: unknown; resolve: (value: unknown) => void }[] = [];
  const panels: Record<string, unknown>[] = [];
  const returns: string[] = [];
  const text: string[] = [];
  const RunPanel = () => {};
  const context = {
    React: {
      createElement: (type: unknown, props: Record<string, unknown>, ...children: unknown[]) => {
        text.push(...children.filter((child): child is string => typeof child === 'string'));
        if (type === RunPanel) panels.push(props);
        return null;
      },
    },
    Button: () => {},
    Advanced: () => {},
    Notice: () => {},
    RunPanel,
    useApp: () => ({ state }),
    usePageContract: (value: typeof contract) => {
      contract = value;
    },
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [
        slots[index],
        (value: unknown) => {
          slots[index] = typeof value === 'function' ? value(slots[index]) : value;
        },
      ];
    },
    useEffect: (effect: () => () => void, deps: unknown[]) => {
      const index = cursor++;
      const previous = effects.get(index);
      if (previous && deps.every((value, i) => Object.is(value, previous.deps[i]))) return;
      previous?.cleanup();
      effects.set(index, { deps, cleanup: effect() });
    },
    invoke: (_command: string, args: unknown) =>
      new Promise((resolve) => {
        requests.push({ args, resolve });
      }),
  };
  const source = readFileSync(new URL('./ProjectBatchRun.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Page = new Function(...Object.keys(context), `${code}; return ProjectBatchRun;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    panels.length = 0;
    text.length = 0;
    Page({
      onBack: () => {
        returns.push('back');
      },
      fromCreation,
    });
  }
  function select(dir: string, checked: boolean) {
    if (!contract.fields.length) adjust();
    contract.fields.find((field) => field.id === `project-batch-case:${dir}`)!.set(String(checked));
    render();
  }
  function selectAll() {
    if (!contract.fields.length) adjust();
    contract.actions.find((action) => action.id === 'project-batch-select-all')!.run();
    render();
  }
  function adjust() {
    contract.actions.find((action) => action.id === 'project-batch-selection')!.run();
    render();
  }
  return {
    state,
    requests,
    panels,
    returns,
    text,
    render,
    select,
    selectAll,
    fields: () => contract.fields,
    actions: () => contract.actions,
  };
}

test('select all loads each station once and respects an active local run', async () => {
  const page = screen();
  page.render();
  assert.equal(page.requests.length, 2);
  assert.equal(page.fields().length, 0, 'station editing is optional and initially collapsed');
  page.select('/b', false);
  page.state.running['/a'] = { running: true };
  page.render();
  page.selectAll();
  assert.equal(page.requests.length, 3);
  page.state.running = {};
  page.render();
  page.selectAll();
  assert.equal(page.requests.length, 5);
  for (const request of page.requests) request.resolve({ profile: { grid: 'site' } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  assert.deepEqual(page.panels[0].batchCases, ['/a', '/b']);
});

test('existing projects default to all stations and optional edits reject stale context', async () => {
  const page = screen();
  page.render();
  assert.equal(page.requests.length, 2);
  assert.equal(page.panels.length, 0);
  assert.deepEqual(page.requests[0].args, {
    dir: '/a',
    modelRevision: { id: 'official', revision: 2 },
  });
  page.select('/a', false);
  page.requests[0].resolve({ profile: { grid: 'stale', crop: true } });
  page.requests[1].resolve({ profile: { grid: 'stale', crop: true } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  assert.equal(page.panels.length, 0, 'late context from a deselected case cannot run');
  page.requests[2].resolve({ profile: { grid: 'site', crop: false } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  assert.equal(page.panels[0].casePath, '/b');
  assert.deepEqual(page.panels[0].batchCases, ['/b']);
  assert.equal((page.panels[0].effectiveProfile as { crop: boolean }).crop, false);
  (page.panels[0].onBusyChange as (busy: boolean) => void)(true);
  page.render();
  page.select('/b', false);
  assert.deepEqual(page.panels[0].batchCases, ['/b'], 'submission locks selections');
  (page.panels[0].onBusyChange as (busy: boolean) => void)(false);
  page.state.running['/b'] = { running: true };
  page.render();
  page.select('/b', false);
  assert.deepEqual(page.panels[0].batchCases, ['/b'], 'running targets remain selected');
  page.state.running = {};
  page.state.versionRef = { id: 'official', revision: 3 };
  page.render();
  assert.equal(page.panels.length, 0, 'a new model requires a fresh effective profile');
  assert.deepEqual(page.requests[3].args, {
    dir: '/b',
    modelRevision: { id: 'official', revision: 3 },
  });
});

test('batch results expose each case failure even when the first case succeeds', async () => {
  const page = screen();
  page.render();
  for (const request of page.requests) request.resolve({ profile: { grid: 'site' } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.state.running = {
    '/a': { running: false, status: 'succeeded' },
    '/b': {
      running: false,
      status: 'failed',
      error: 'Missing forcing data',
      logs: ['B failed at preprocessing'],
    },
  };
  page.render();
  assert.deepEqual(page.panels[0].summary, { name: 'A、B', period: 'Each case’s saved period' });
  assert.ok(page.text.includes('Completed'));
  assert.ok(page.text.includes('Failed'));
  assert.ok(page.text.includes('Missing forcing data'));
  assert.ok(page.text.includes('B failed at preprocessing'));
});

test('new project handoff selects only created cases and preserves resource choices', async () => {
  const settings = {
    root: '/project',
    cases: ['/b'],
    host: 'server',
    threads: '4',
    ranks: '1',
    workers: '2',
    scheduler: 'slurm',
    cpus: '4',
  };
  const page = screen(settings);
  page.render();
  assert.equal(page.fields().length, 0, 'creation selections are not requested again');
  assert.ok(
    !page
      .actions()
      .some(
        (action) =>
          action.id === 'project-batch-selection' || action.id === 'project-batch-select-all',
      ),
  );
  assert.equal(page.requests.length, 1);
  page.requests[0].resolve({ profile: { grid: 'site', run_host: 'server', engine: 'rust' } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  assert.deepEqual(page.panels[0].batchCases, ['/b']);
  assert.equal(page.panels[0].initialHost, 'server');
  assert.deepEqual(page.panels[0].initialSettings, settings);
  const other = screen({ ...settings, root: '/another-project' });
  other.render();
  assert.equal(other.requests.length, 2, 'another project uses its own cases');
});

test('returning to an existing project retains the last station selection and optional edits', () => {
  const page = screen(
    {
      root: '/project',
      cases: ['/b'],
      batch: true,
      host: '',
      threads: '2',
      ranks: '1',
      workers: '1',
      scheduler: 'auto',
      cpus: '1',
    },
    false,
  );
  page.render();
  assert.equal(page.requests.length, 1, 'the previous run selection is preserved');
  assert.equal(page.fields().length, 0);
  assert.ok(page.actions().some((action) => action.id === 'project-batch-selection'));
  page.select('/a', true);
  assert.equal(page.fields().length, 2);
});

test('single-case run preferences do not narrow a manually opened project batch', () => {
  const page = screen(
    {
      root: '/project',
      cases: ['/b'],
      host: '',
      threads: '2',
      ranks: '1',
      workers: '1',
      scheduler: 'auto',
      cpus: '1',
    },
    false,
  );
  page.render();
  assert.equal(page.requests.length, 2);
});

test('batch run panel receives the guarded Back action', async () => {
  const page = screen();
  page.render();
  for (const request of page.requests) request.resolve({ profile: { grid: 'site' } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  (page.panels[0].onBusyChange as (value: boolean) => void)(true);
  page.render();
  (page.panels[0].onBack as () => void)();
  assert.deepEqual(page.returns, []);
  (page.panels[0].onBusyChange as (value: boolean) => void)(false);
  page.render();
  (page.panels[0].onBack as () => void)();
  assert.deepEqual(page.returns, ['back']);
});

test('ninety selected stations load the run panel without repeating idle status rows', async () => {
  const cases = Array.from({ length: 90 }, (_, index) => ({
    dir: `/site-${index}`,
    name: `Site ${index}`,
  }));
  const page = screen({
    root: '/project',
    cases: cases.map((entry) => entry.dir),
    host: '',
    threads: '1',
    ranks: '1',
    workers: '2',
    scheduler: 'auto',
    cpus: '1',
  });
  page.state.cases = cases;
  page.render();
  assert.equal(page.requests.length, 90);
  for (const request of page.requests) request.resolve({ profile: { grid: 'site', crop: false } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.render();
  assert.equal(page.panels.length, 1);
  assert.equal((page.panels[0].batchCases as string[]).length, 90);
  assert.deepEqual(page.panels[0].summary, {
    name: '90 stations',
    period: 'Each case’s saved period',
  });
  assert.ok(page.text.includes('Stations to run: 90'));
  assert.equal(page.fields().length, 0);
  assert.ok(!page.text.includes('Not started'));
});

test('batch waits for every selected profile and rejects different hosts or engines', async () => {
  for (const mismatch of [{ run_host: 'another-server' }, { engine: 'fortran' }]) {
    const page = screen();
    page.render();
    page.requests[0].resolve({ profile: { grid: 'site', run_host: 'server', engine: 'rust' } });
    await new Promise<void>((resolve) => setImmediate(resolve));
    page.render();
    assert.equal(page.panels.length, 0);
    page.requests[1].resolve({
      profile: { grid: 'site', run_host: 'server', engine: 'rust', ...mismatch },
    });
    await new Promise<void>((resolve) => setImmediate(resolve));
    page.render();
    assert.equal(page.panels.length, 0);
    assert.ok(page.text.includes('Select cases with the same run location and engine.'));
  }
});
