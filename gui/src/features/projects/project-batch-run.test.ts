import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { remoteActivity, restoreRunView } from '../runs/run-model';

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
      {
        running: boolean;
        status?: string;
        error?: string;
        logs?: string[];
        progress?: Record<string, unknown>;
      }
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
  const elements: { type: unknown; props: Record<string, unknown>; children: unknown[] }[] = [];
  const RunPanel = () => {};
  const context = {
    React: {
      createElement: (type: unknown, props: Record<string, unknown>, ...children: unknown[]) => {
        text.push(...children.filter((child): child is string => typeof child === 'string'));
        if (type === RunPanel) panels.push(props);
        const element = { type, props, children };
        elements.push(element);
        return element;
      },
    },
    Button: () => {},
    Advanced: () => {},
    Notice: () => {},
    RunPanel,
    restoreRunView,
    remoteActivity,
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
    // The page reads every station in one model_case_profiles call; each station still
    // appears here as its own request so tests can answer stations one by one.
    invoke: (command: string, args: { dirs?: string[]; modelRevision?: unknown }) => {
      if (command !== 'model_case_profiles')
        return new Promise((resolve) => {
          requests.push({ args, resolve });
        });
      const dirs = args.dirs ?? [];
      return Promise.all(
        dirs.map(
          (dir) =>
            new Promise<{ profile: unknown }>((resolve) => {
              requests.push({
                args: { dir, modelRevision: args.modelRevision },
                resolve: (value) => resolve(value as { profile: unknown }),
              });
            }),
        ),
      ).then((answers) =>
        Object.fromEntries(dirs.map((dir, index) => [dir, answers[index].profile])),
      );
    },
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
    elements.length = 0;
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
    elements,
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

test('batch monitor shows each active station progress before waiting and completed stations', () => {
  const page = screen();
  page.state.cases = ['Done', 'First', 'Second', 'Waiting', 'Preparing', 'Unknown'].map((name) => ({
    dir: `/${name}`,
    name,
  }));
  page.state.running = {
    '/Done': { running: false, status: 'succeeded', logs: ['Done log'] },
    '/First': {
      running: true,
      status: 'running',
      logs: ['First log'],
      progress: { stage: 'colm', step: 12, total_steps: 50 },
    },
    '/Second': {
      running: true,
      status: 'running',
      logs: ['Second log'],
      progress: { stage: 'mksrfdata', step: 3, total_steps: 10 },
    },
    '/Waiting': { running: true, status: 'queued', logs: [] },
    '/Preparing': { running: true, status: 'preparing', logs: [] },
    '/Unknown': { running: true, status: 'unknown', error: 'Connection lost', logs: [] },
  };
  page.render();
  assert.ok(
    page.text.includes(
      'Running 2 · Waiting 2 · Completed 1 · Failed 0 · Stopped 0 · Unconfirmed 1',
    ),
  );
  assert.deepEqual(
    page.elements.filter((el) => el.type === 'article').map((el) => el.props['aria-label']),
    ['First', 'Second', 'Unknown', 'Waiting', 'Preparing', 'Done'],
  );
  assert.deepEqual(
    page.elements.filter((el) => el.type === 'progress').map((el) => el.props),
    [
      { 'aria-label': 'First', value: 12, max: 50 },
      { 'aria-label': 'Second', value: 3, max: 10 },
    ],
  );
  assert.ok(page.text.includes(' · Simulation'));
  assert.ok(page.text.includes(' · Surface data'));
  for (const message of ['First log', 'Second log', 'Done log', 'Connection lost'])
    assert.ok(page.text.includes(message));
  const groups = page.elements.filter(
    (el) => el.type === 'details' && el.children.some((child) => Array.isArray(child)),
  );
  assert.equal(groups.length, 2, 'waiting and finished stations are collapsed separately');
  assert.ok(groups.every((group) => !group.props?.open));
});

test('remote scheduler queue is not counted as a running station', () => {
  const page = screen();
  page.state.running = {
    '/a': {
      running: true,
      status: 'submitted',
      logs: [],
      progress: { remote: { status: { state: 'queued' } } },
    },
    '/b': {
      running: true,
      status: 'submitted',
      logs: [],
      progress: { remote: { status: { state: 'running' } } },
    },
  };
  page.render();
  assert.ok(
    page.text.includes(
      'Running 1 · Waiting 1 · Completed 0 · Failed 0 · Stopped 0 · Unconfirmed 0',
    ),
  );
});

test('remote batch cards show actual remote-status stage, numerical progress and logs', () => {
  const page = screen();
  page.state.running = {
    '/a': {
      running: true,
      status: 'submitted',
      logs: [],
      progress: {
        remote: {
          status: { state: 'running', log_tail: 'A server model log' },
          stages: [
            ['mksrfdata', 'done'],
            ['colm', 'begin'],
          ],
          progress: { step: 14, date: '2002-01-01', total_steps: 50 },
        },
      },
    },
    '/b': {
      running: true,
      status: 'submitted',
      logs: [],
      progress: {
        remote: {
          status: { state: 'running', log_tail: 'B server initialization log' },
          stages: [['mkinidata', 'begin']],
          progress: null,
        },
      },
    },
  };
  page.render();
  assert.ok(page.text.includes(' · Simulation'));
  assert.ok(page.text.includes(' · Initial conditions'));
  assert.ok(page.text.includes('A server model log'));
  assert.ok(page.text.includes('B server initialization log'));
  assert.deepEqual(
    page.elements.filter((el) => el.type === 'progress').map((el) => el.props),
    [{ 'aria-label': 'A', value: 14, max: 50 }],
  );
});

test('a station whose remote job still runs cannot be deselected', async () => {
  const page = screen();
  page.render();
  for (const request of page.requests) request.resolve({ profile: { grid: 'site' } });
  await new Promise<void>((resolve) => setImmediate(resolve));
  page.state.running['/a'] = {
    running: true,
    status: 'submitted',
    location: 'remote',
  } as (typeof page.state.running)[string];
  page.render();
  page.select('/a', false);
  page.render();
  const field = page.fields().find((entry) => entry.id === 'project-batch-case:/a')!;
  assert.equal(field.disabled, true);
  page.select('/b', false);
  page.render();
  // The running station keeps its place; the idle one could still be removed.
  assert.ok(!page.fields().find((entry) => entry.id === 'project-batch-case:/b')!.disabled);
  const value = (dir: string) =>
    page.fields().find((entry) => entry.id === `project-batch-case:${dir}`)! as unknown as {
      value: string;
    };
  assert.equal(value('/a').value, 'true');
  assert.equal(value('/b').value, 'false');
});

test('a server station says why it waits, or that the engine is being compiled', () => {
  const page = screen();
  page.state.cases = ['Q', 'B', 'S'].map((name) => ({ dir: `/${name}`, name }));
  const remote = (status: Record<string, unknown>, extra: Record<string, unknown> = {}) => ({
    running: true,
    status: 'submitted',
    location: 'remote',
    logs: [],
    progress: { remote: { record: { host: 'land02', job: 'run-1' }, status, ...extra } },
  });
  page.state.running = {
    '/Q': remote({ state: 'queued', detail: 'PENDING|Resources' }),
    '/B': remote({ state: 'running', phase: 'building the Rust engine' }),
    '/S': {
      ...remote({ state: 'lost', detail: 'CANCELLED by 1026|0:0' }, { ended: 'cancelled' }),
      running: false,
      status: 'cancelled',
    },
  };
  page.render();
  assert.ok(page.text.includes('Queued: waiting for free cores and memory'));
  assert.ok(page.text.includes('Compiling the engine on the server'));
  assert.ok(page.text.includes('Stopped (the job was cancelled)'));
});

test('station picker filters by status and quick-selects only what is listed or unfinished', () => {
  const page = screen();
  page.state.cases = ['A', 'B', 'C'].map((name) => ({ dir: `/${name}`, name }));
  page.state.running = {
    '/A': { running: false, status: 'succeeded', logs: [] },
    '/B': { running: false, status: 'failed', logs: [] },
  };
  page.render();
  page
    .actions()
    .find((action) => action.id === 'project-batch-selection')!
    .run();
  page.render();
  const click = (label: string) => {
    const element = page.elements.find(
      (el) => el.children.includes(label) && typeof el.props?.onClick === 'function',
    )!;
    (element.props.onClick as () => void)();
    page.render();
  };
  const chosen = () =>
    page
      .fields()
      .filter((field) => (field as unknown as { value: string }).value === 'true')
      .map((field) => field.id.split(':')[1]);
  const listed = () =>
    page.elements
      .filter((el) => el.type === 'label' && String(el.props.className).startsWith('batch-station'))
      .map((el) => el.props.className);
  assert.deepEqual(chosen(), ['/A', '/B', '/C']);
  click('Only stations without results (2)');
  assert.deepEqual(chosen(), ['/B', '/C']);
  click('Failed or stopped');
  assert.deepEqual(listed(), ['batch-station is-failed is-selected']);
  click('Deselect listed');
  assert.deepEqual(chosen(), ['/C']);
  click('All');
  assert.equal(listed().length, 3);
  click('Select all stations');
  assert.deepEqual(chosen(), ['/A', '/B', '/C']);
});

test('monitor tiles show one status at a time and pick failed stations for a rerun', () => {
  const page = screen();
  page.state.cases = ['Done', 'Broke', 'Halted', 'Busy'].map((name) => ({
    dir: `/${name}`,
    name,
  }));
  page.state.running = {
    '/Done': { running: false, status: 'succeeded', logs: [] },
    '/Broke': { running: false, status: 'failed', logs: [] },
    '/Halted': { running: false, status: 'cancelled', logs: [] },
    // 本地运行会锁住整个选择；服务器上的运行只钉住它自己。
    '/Busy': {
      running: true,
      status: 'running',
      location: 'remote',
      logs: [],
    } as (typeof page.state.running)[string],
  };
  page.render();
  assert.ok(page.text.includes('3 of 4 ended'));
  const articles = () =>
    page.elements.filter((el) => el.type === 'article').map((el) => el.props['aria-label']);
  const tile = (tone: string) =>
    page.elements.find(
      (el) => el.type === 'button' && el.props.className === `batch-tile is-${tone}`,
    )!;
  (tile('failed').props.onClick as () => void)();
  page.render();
  assert.deepEqual(articles(), ['Broke']);
  assert.equal(tile('failed').props['aria-pressed'], true);
  assert.ok(
    !page.elements.some(
      (el) => el.type === 'details' && String(el.props?.className) === 'batch-group',
    ),
  );
  (tile('failed').props.onClick as () => void)();
  page.render();
  assert.deepEqual(articles(), ['Busy', 'Done', 'Broke', 'Halted']);
  const rerun = page.elements.find((el) =>
    el.children.includes('Select the 2 failed or stopped stations to rerun'),
  )!;
  (rerun.props.onClick as () => void)();
  page.render();
  assert.deepEqual(articles(), ['Busy', 'Broke', 'Halted'], 'the running station stays selected');
});
