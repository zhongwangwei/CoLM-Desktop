import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import ts from 'typescript';
import type { AppState, RunState } from '../../lib/types';
const require = createRequire(import.meta.url);
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function render(
  record?: RunState,
  history: unknown[] = [],
  remoteStage?: string,
  otherRuns: Record<string, RunState> = {},
  batchCases?: string[],
  savedHost?: string,
  saveError?: string,
  remoteDirectory?: string,
  onBack?: () => void,
) {
  const effects: (() => void)[] = [];
  const contracts: {
    details?: { runs?: { run: { casePath: string } | null }[]; run?: unknown };
  }[] = [];
  const state = {
    selectedCase: {
      dir: '/case',
      entry: { name: '/case' },
      profile: { run_host: savedHost ?? null, remote_directory: remoteDirectory ?? null },
    },
    language: 'zh',
    root: '/project',
    projectRunSettings: undefined as AppState['projectRunSettings'],
    expert: false,
    running: { ...otherRuns, ...(record ? { '/case': record } : {}) } as Record<string, RunState>,
    kernels: [],
    cases: [],
    version: null,
    versionRef: { id: 'official', revision: 1 },
  };
  const routes: unknown[][] = [],
    calls: string[] = [],
    errors: string[] = [],
    hostChanges: unknown[] = [];
  let stateIndex = 0;
  const app = {
    state,
    navigate: (...args: unknown[]) => routes.push(args),
    openCase: async (...args: unknown[]) => routes.push(['openCase', ...args]),
    notify: (error: string) => errors.push(error),
    update: (patch: Partial<typeof state> | ((old: typeof state) => Partial<typeof state>)) =>
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch),
  };
  const react = {
    useEffect: (effect: () => void) => effects.push(effect),
    // The stored completion has already been reconciled; other boolean defaults are false.
    useState: (initial: unknown) => {
      const index = stateIndex++;
      return [
        initial === true ? false : initial === null ? (record?.progress?.remote ?? null) : initial,
        (value: unknown) => {
          if (index === 0) hostChanges.push(value);
        },
      ];
    },
    useRef: (current: unknown) => ({ current }),
    createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => ({
      type,
      props: props ?? {},
      children,
    }),
  };
  const native = {
    invoke: async (command: string) => {
      if (command === 'set_case_run_host') {
        calls.push(command);
        if (saveError) throw new Error(saveError);
      }
      return { servers: [] };
    },
    invokeJson: async (command: string) => {
      calls.push(command);
      if (command === 'remote_job_record') return null;
      if (command === 'model_case_runs') return history;
      return command === 'remote_fetch'
        ? { run_case: '/run-copy' }
        : {
            record: { host: 'server', job: 'job', provenance: { stage: remoteStage ?? null } },
            status: { state: 'finished', exit_code: 0 },
          };
    },
  };
  const text = readFileSync(new URL('./RunPanel.tsx', import.meta.url), 'utf8');
  const js = ts.transpileModule(text, {
    compilerOptions: {
      module: ts.ModuleKind.CommonJS,
      jsx: ts.JsxEmit.React,
      target: ts.ScriptTarget.ES2022,
    },
  }).outputText;
  const exports: { default?: (props: unknown) => Element } = {};
  const dependencies = (name: string) =>
    name === 'react'
      ? react
      : name === '../../lib/app-state'
        ? { useApp: () => app }
        : name === '../../lib/ipc'
          ? native
          : name === '../../lib/guide'
            ? {
                usePageContract: (contract: (typeof contracts)[number]) => contracts.push(contract),
              }
            : name === '../../components/ui'
              ? Object.fromEntries(
                  ['Button', 'Card', 'Field', 'Input', 'Select', 'Advanced', 'Notice'].map((n) => [
                    n,
                    n,
                  ]),
                )
              : name === '../../lib/bilingual'
                ? { bilingual: (zh: string) => zh }
                : require(name);
  new Function('require', 'exports', 'React', js)(dependencies, exports, react);
  const tree = exports.default!({ casePath: '/case', batchCases, effectiveProfile: null, onBack });
  const elements: Element[] = [];
  function visit(value: unknown) {
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (value && typeof value === 'object' && 'children' in value) {
      const el = value as Element;
      elements.push(el);
      el.children.forEach(visit);
    }
  }
  visit(tree);
  return {
    elements,
    contracts,
    back: elements.find((el) => el.type === 'Button' && el.children.includes('上一步')),
    next: elements.find(
      (el) =>
        el.type === 'Button' &&
        (el.children.includes('下一步：查看结果') || el.children.includes('查看 /case 的结果')),
    ),
    stop: elements.find((el) => el.type === 'Button' && el.children.includes('停止运行')),
    remoteStatus: elements.find(
      (el) => el.type === 'Button' && el.children.includes('核对服务器状态'),
    ),
    remoteFetch: elements.find((el) => el.type === 'Button' && el.children.includes('取回结果')),
    location: elements.find((el) => el.type === 'Select'),
    effects,
    errors,
    hostChanges,
    routes,
    calls,
    state,
    recover: async () => {
      effects[1]();
      await new Promise<void>((resolve) => setImmediate(resolve));
    },
  };
}
const success: RunState = {
  case: '/case',
  run_id: 'one',
  status: 'succeeded',
  running: false,
  logs: [],
  stages: {},
  location: 'local',
  run_case: '/run-copy',
};

test('input validation is explained as a startup action rather than an incomplete checklist', () => {
  const page = render();
  const note = page.elements.find(
    (el) =>
      el.type === 'p' &&
      el.children.some(
        (text) => typeof text === 'string' && text.includes('点击“开始运行”时检查输入文件'),
      ),
  );
  assert.equal(note?.props.className, 'muted');
  assert.ok(!page.elements.some((el) => el.children.includes('启动时检查实际输入')));
});

test('batch Back preserves run resources and cannot leave while another local member is active', () => {
  let returned = 0;
  const page = render(
    undefined,
    [],
    undefined,
    {},
    ['/case', '/other'],
    undefined,
    undefined,
    undefined,
    () => {
      returned++;
    },
  );
  assert.ok(page.back);
  assert.equal(page.back.props.disabled, false);
  (page.back.props.onClick as () => void)();
  assert.equal(returned, 1);
  assert.deepEqual(page.state.projectRunSettings, {
    root: '/project',
    cases: ['/case', '/other'],
    batch: true,
    host: '',
    ranks: '1',
    threads: '1',
    workers: '1',
    scheduler: 'auto',
    cpus: '',
  });
  const active = render(
    { ...success, status: 'failed' },
    [],
    undefined,
    { '/other': { ...success, case: '/other', running: true, status: 'running' } },
    ['/case', '/other'],
    undefined,
    undefined,
    undefined,
    () => {
      returned++;
    },
  );
  assert.equal(active.back?.props.disabled, true);
  (active.back!.props.onClick as () => void)();
  assert.equal(returned, 1);
  assert.equal(active.state.projectRunSettings, undefined);
});
test('completed simulation exposes next button and opens its case results', async () => {
  const page = render(success);
  assert.ok(page.next);
  assert.equal(page.next.props.disabled, false);
  await (page.next.props.onClick as () => Promise<void>)();
  assert.deepEqual(page.routes, [['case', 'results']]);
});
test('running, failed, cancelled and preprocessing-only runs have no next-result button', () => {
  for (const record of [
    { ...success, status: 'running', running: true },
    { ...success, status: 'failed' },
    { ...success, status: 'cancelled' },
    { ...success, requested_stage: 'mksrfdata' },
    { ...success, requested_stage: 'mkinidata' },
  ])
    assert.equal(render(record).next, undefined);
});
test('remote next retrieves finished output before navigating', async () => {
  const page = render({
    ...success,
    location: 'remote',
    run_case: undefined,
    progress: {
      remote: {
        record: { host: 'server', job: 'job' },
        status: { state: 'finished', exit_code: 0 },
      },
    },
  });
  assert.ok(page.next);
  await (page.next.props.onClick as () => Promise<void>)();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.deepEqual(page.calls, ['remote_fetch', 'remote_status']);
  assert.equal(page.state.running['/case'].run_case, '/run-copy');
  assert.deepEqual(page.routes, [['case', 'results']]);
});

const completedHistory = {
  status: 'succeeded',
  operation_id: 'saved',
  run_case: '/saved-run',
  provenance: { ok: true, stage: null },
};
test('reopening a completed case restores next and its exact result directory', async () => {
  const page = render(undefined, [completedHistory]);
  await page.recover();
  const restored = page.state.running['/case'];
  assert.equal(restored.run_case, '/saved-run');
  assert.equal(restored.run_id, 'saved');
  assert.ok(render(restored).next);
});
test('history recovery never substitutes an older success for an unconfirmed latest run', async () => {
  for (const latest of [
    { ...completedHistory, status: 'failed' },
    { ...completedHistory, needs_reconciliation: true },
    { ...completedHistory, provenance: { ok: false } },
  ]) {
    const page = render(undefined, [latest, completedHistory]);
    await page.recover();
    assert.equal(page.state.running['/case'], undefined);
  }
});
test('recovered preprocessing completion does not offer simulation results', async () => {
  const page = render(undefined, [
    { ...completedHistory, provenance: { ok: true, stage: 'mksrfdata' } },
  ]);
  await page.recover();
  assert.equal(render(page.state.running['/case']).next, undefined);
});
test('remote reconciliation restores preprocessing stage before showing results', async () => {
  const page = render(
    {
      ...success,
      location: 'remote',
      run_case: undefined,
      progress: {
        remote: {
          record: { host: 'server', job: 'job' },
          status: { state: 'finished', exit_code: 0 },
        },
      },
    },
    [],
    'mkinidata',
  );
  await (page.next!.props.onClick as () => void)();
  await new Promise<void>((resolve) => setImmediate(resolve));
  assert.equal(page.state.running['/case'].requested_stage, 'mkinidata');
  assert.deepEqual(page.routes, []);
  assert.equal(render(page.state.running['/case']).next, undefined);
});

test('Stop remains visible when the primary batch case failed but another member is running', () => {
  const primary = { ...success, status: 'failed', running: false };
  const member = { ...success, case: '/other', status: 'running', running: true };
  assert.ok(render(primary, [], undefined, { '/other': member }).stop);
  assert.equal(render(primary).stop, undefined);
  assert.equal(
    render(primary, [], undefined, { '/other': { ...member, run_id: 'unrelated' } }).stop,
    undefined,
  );
});

test('old remote status and fetch cannot replace a newer local result', async () => {
  const local: RunState = {
    ...success,
    operation_id: 'local-operation',
    logs: ['latest local output'],
    run_case: '/local-new',
    progress: {
      remote: {
        record: { host: 'server', job: 'job' },
        status: { state: 'finished', exit_code: 0 },
      },
    },
  };
  const page = render(local);
  for (const button of [page.remoteStatus, page.remoteFetch]) {
    assert.ok(button);
    assert.equal(button.props.disabled, false);
    (button.props.onClick as () => void)();
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.strictEqual(page.state.running['/case'], local);
  }
  assert.deepEqual(page.calls, ['remote_status', 'remote_fetch', 'remote_status']);
});

test('project batch results open the completed target instead of a previously selected case', async () => {
  const page = render(success, [], undefined, {}, ['/case', '/other']);
  assert.ok(page.next);
  await (page.next.props.onClick as () => Promise<void>)();
  assert.deepEqual(page.routes, [['openCase', '/case', 'results']]);
});

test('run location restores the saved case host, with active job host taking precedence', () => {
  const missing = render(undefined, [], undefined, {}, undefined, 'removed-server').location;
  assert.ok(
    missing?.children
      .flat(Infinity)
      .some(
        (child) =>
          child &&
          typeof child === 'object' &&
          (child as Element).type === 'option' &&
          (child as Element).props.value === 'removed-server',
      ),
  );
  assert.equal(
    render(undefined, [], undefined, {}, undefined, 'saved-server').location?.props.value,
    'saved-server',
  );
  assert.equal(
    render(
      { ...success, running: true, location: 'remote', progress: { host: 'running-server' } },
      [],
      undefined,
      {},
      undefined,
      'saved-server',
    ).location?.props.value,
    'running-server',
  );
  assert.equal(
    render(
      { ...success, location: 'remote', progress: { host: 'old-server' } },
      [],
      undefined,
      {},
      undefined,
      'saved-server',
    ).location?.props.value,
    'saved-server',
  );
  const batch = render(undefined, [], undefined, {}, ['/case'], 'saved-server');
  assert.equal(batch.location?.props.value, 'saved-server');
  assert.equal(batch.location?.props.disabled, true);
  batch.effects[3]();
  assert.deepEqual(batch.hostChanges, ['saved-server']);
});

test('changing run location persists the case preference before applying it', async () => {
  const page = render(undefined, [], undefined, {}, undefined, 'saved-server');
  await (page.location!.props.onChange as (e: unknown) => Promise<void>)({
    target: { value: 'new-server' },
  });
  assert.deepEqual(page.calls, ['set_case_run_host']);
  assert.equal(page.state.selectedCase.profile.run_host, 'new-server');
  assert.deepEqual(page.hostChanges, ['new-server']);
});

test('failed run location save retains the previous host and reports the error', async () => {
  const page = render(undefined, [], undefined, {}, undefined, 'saved-server', 'disk unavailable');
  await (page.location!.props.onChange as (e: unknown) => Promise<void>)({ target: { value: '' } });
  assert.equal(page.state.selectedCase.profile.run_host, 'saved-server');
  assert.deepEqual(page.hostChanges, []);
  assert.match(page.errors[0], /disk unavailable/);
});

test('active local jobs override a saved remote run location', () => {
  const page = render(
    { ...success, running: true, status: 'running', location: 'local' },
    [],
    undefined,
    {},
    undefined,
    'saved-server',
  );
  assert.equal(page.location?.props.value, '');
  page.effects[3]();
  page.effects[4]();
  assert.deepEqual(page.hostChanges, ['']);
});

test('server-native case keeps its owning host locked in the run panel', async () => {
  const page = render(
    undefined,
    [],
    undefined,
    {},
    undefined,
    'compute',
    undefined,
    '/srv/project/site',
  );
  assert.equal(page.location?.props.disabled, true);
  await (page.location!.props.onChange as (e: unknown) => Promise<void>)({ target: { value: '' } });
  assert.deepEqual(page.calls, []);
  assert.equal(page.state.selectedCase.profile.run_host, 'compute');
});

test('a fresh batch tolerates unrelated active runs without exposing their Stop control', () => {
  const page = render(
    undefined,
    [],
    undefined,
    { '/other': { ...success, case: '/other', run_id: 'other-run', running: true } },
    ['/case'],
  );
  assert.equal(page.stop, undefined);
});

test('an unknown remote batch job without an identity still offers explicit reconciliation', () => {
  const page = render(
    {
      ...success,
      status: 'unknown',
      location: 'remote',
      running: true,
      progress: { host: 'server' },
    },
    [],
    undefined,
    {},
    ['/case'],
    'server',
  );
  assert.ok(page.remoteStatus);
  assert.equal(page.remoteStatus.props.disabled, false);
  assert.equal(page.remoteFetch?.props.disabled, true);
});

test('batch monitor belongs to the all-stations page while single-case progress remains visible', () => {
  const running: RunState = {
    case: '/case',
    running: true,
    status: 'running',
    logs: ['step'],
    stages: {},
    progress: { stage: 'colm', step: 1, total_steps: 10 },
  };
  const single = render(running);
  assert.equal(
    single.elements.filter(
      (el) => el.type === 'section' && el.props.className === 'simulation-monitor',
    ).length,
    1,
  );
  const batch = render(running, [], undefined, { '/other': { ...running, case: '/other' } }, [
    '/case',
    '/other',
  ]);
  assert.deepEqual(
    batch.contracts[0].details?.runs?.map(({ run }) => run?.casePath),
    ['/case', '/other'],
  );
  assert.equal(
    batch.elements.filter(
      (el) => el.type === 'section' && el.props.className === 'simulation-monitor',
    ).length,
    0,
  );
});
