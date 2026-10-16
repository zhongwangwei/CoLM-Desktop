import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import ts from 'typescript';
import type { RunState } from '../../lib/types';
const require = createRequire(import.meta.url);
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function render(
  record?: RunState,
  history: unknown[] = [],
  remoteStage?: string,
  otherRuns: Record<string, RunState> = {},
) {
  const effects: (() => void)[] = [];
  const state = {
    language: 'zh',
    expert: false,
    running: { ...otherRuns, ...(record ? { '/case': record } : {}) } as Record<string, RunState>,
    kernels: [],
    cases: [],
    version: null,
    versionRef: { id: 'official', revision: 1 },
  };
  const routes: unknown[][] = [],
    calls: string[] = [];
  const app = {
    state,
    navigate: (...args: unknown[]) => routes.push(args),
    notify: () => {},
    update: (patch: (old: typeof state) => Partial<typeof state>) =>
      Object.assign(state, patch(state)),
  };
  const react = {
    useEffect: (effect: () => void) => effects.push(effect),
    // The stored completion has already been reconciled; other boolean defaults are false.
    useState: (initial: unknown) => [
      initial === true ? false : initial === null ? (record?.progress?.remote ?? null) : initial,
      () => {},
    ],
    useRef: (current: unknown) => ({ current }),
    createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => ({
      type,
      props: props ?? {},
      children,
    }),
  };
  const native = {
    invoke: async () => ({ servers: [] }),
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
            ? { usePageContract: () => {} }
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
  const tree = exports.default!({ casePath: '/case', effectiveProfile: null });
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
    next: elements.find((el) => el.type === 'Button' && el.children.includes('下一步：查看结果')),
    stop: elements.find((el) => el.type === 'Button' && el.children.includes('停止运行')),
    remoteStatus: elements.find(
      (el) => el.type === 'Button' && el.children.includes('核对服务器状态'),
    ),
    remoteFetch: elements.find((el) => el.type === 'Button' && el.children.includes('取回结果')),
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
