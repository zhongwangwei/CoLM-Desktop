import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import ts from 'typescript';
import type { AppState, RunState } from '../../lib/types';
import type { UiField } from '../../lib/guide';

const require = createRequire(import.meta.url);
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
const source = readFileSync(new URL('./RunPanel.tsx', import.meta.url), 'utf8');
const code = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    jsx: ts.JsxEmit.React,
    target: ts.ScriptTarget.ES2022,
  },
}).outputText;
const servers = [
  { host: 'A', nodes: 2, cores_per_node: 48, threads: 8, scheduler: 'slurm', memory_gb: 64 },
  { host: 'B', nodes: 3, cores_per_node: 64, threads: 16, scheduler: 'slurm', memory_gb: 128 },
];

async function page(
  options: {
    spatial?: boolean;
    batch?: boolean;
    host?: string;
    language?: string;
    settings?: AppState['projectRunSettings'];
  } = {},
) {
  const spatial = options.spatial ?? true;
  const batchCases = options.batch ? ['/case', '/other'] : undefined;
  const state = {
    selectedCase: {
      dir: '/case',
      entry: { name: 'case' },
      profile: { spatial, run_host: options.host ?? 'A' },
    },
    language: options.language ?? 'zh',
    root: '/project',
    projectRunSettings: options.settings,
    running: {} as Record<string, RunState>,
    kernels: [
      {
        dir: '/kernel',
        preset: 'default',
        macros: [spatial ? 'GRIDBASED' : 'SinglePoint', 'LULC_IGBP'],
      },
    ],
    cases: (batchCases ?? ['/case']).map((dir) => ({ dir, spatial })),
    version: null,
    versionRef: { id: 'official', revision: 1 },
  };
  const values: unknown[] = [],
    refs: { current: unknown }[] = [];
  let stateIndex = 0,
    refIndex = 0;
  let effects: (() => void)[] = [];
  let contract: { fields: UiField[] };
  let tree: Element;
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const app = {
    state,
    notify: () => {},
    navigate: () => true,
    openCase: async () => true,
    update: (patch: Partial<typeof state> | ((old: typeof state) => Partial<typeof state>)) =>
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch),
  };
  const react = {
    useState: (initial: unknown) => {
      const index = stateIndex++;
      if (!(index in values)) values[index] = initial === true ? false : initial;
      return [
        values[index],
        (next: unknown) => {
          values[index] = typeof next === 'function' ? next(values[index]) : next;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const index = refIndex++;
      return refs[index] ?? (refs[index] = { current: initial });
    },
    useEffect: (effect: () => void) => effects.push(effect),
    createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => ({
      type,
      props: props ?? {},
      children,
    }),
  };
  const ipc = async (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'remote_config') return { servers };
    if (command === 'model_migrate_case') return { version: state.versionRef };
    if (command === 'remote_run') return { host: state.selectedCase.profile.run_host, job: 'new' };
    return null;
  };
  const exports: { default?: (props: unknown) => Element } = {};
  const dependencies = (name: string): unknown => {
    if (name === 'react') return react;
    if (name === '../../lib/app-state') return { useApp: () => app };
    if (name === '../../lib/ipc') return { invoke: ipc, invokeJson: ipc };
    if (name === '../../lib/guide')
      return {
        usePageContract: (next: typeof contract) => {
          contract = next;
        },
      };
    if (name === '../../components/ui')
      return Object.fromEntries(
        ['Button', 'Card', 'Field', 'Input', 'Select', 'Advanced', 'Notice'].map((n) => [n, n]),
      );
    if (name === '../../components/ActionButton') return { ActionButton: 'ActionButton' };
    if (name === './NamelistPreview') return { default: 'NamelistPreview' };
    return require(name);
  };
  new Function('require', 'exports', 'React', code)(dependencies, exports, react);
  const render = () => {
    stateIndex = refIndex = 0;
    effects = [];
    tree = exports.default!({
      casePath: '/case',
      batchCases,
      effectiveProfile: { grid: spatial ? 'latlon' : undefined, classification: 'IGBP' },
    });
  };
  const elements = (value: unknown): Element[] =>
    Array.isArray(value)
      ? value.flatMap(elements)
      : value && typeof value === 'object' && 'children' in value
        ? [value as Element, ...(value as Element).children.flatMap(elements)]
        : [];
  const field = (id: string) => contract.fields.find((entry) => entry.id === id)!;
  const set = async (id: string, value: string) => {
    await field(id).set(value);
    render();
  };
  render();
  effects.find((effect) => effect.toString().includes("'remote_config'"))!();
  await new Promise<void>((resolve) => setImmediate(resolve));
  render();
  if (!options.batch) await set('run-kernel', '/kernel');
  return {
    state,
    calls,
    render,
    field,
    set,
    fields: () => contract.fields,
    text: () => JSON.stringify(tree),
    submit: () => elements(tree).find((el) => el.props.action === 'run-submit')!,
    persist: () =>
      effects.find((effect) =>
        effect.toString().includes('JSON.stringify(state.projectRunSettings)'),
      )!(),
  };
}

test('automatic spatial defaults follow the selected server while explicit values remain per host', async () => {
  const p = await page();
  assert.equal(p.field('run-parallel').value, 'auto');
  assert.equal(p.field('run-nodes').value, '2');
  assert.equal(p.field('run-cores-per-node').value, '48');
  await p.set('run-host', 'B');
  assert.equal(p.field('run-nodes').value, '3');
  assert.equal(p.field('run-cores-per-node').value, '64');
  await p.set('run-nodes', '4');
  await p.set('run-host', 'A');
  assert.equal(p.field('run-nodes').value, '2');
  await p.set('run-cores-per-node', '32');
  await p.set('run-host', 'B');
  assert.equal(p.field('run-nodes').value, '4');
  assert.equal(p.field('run-cores-per-node').value, '64');
  await p.set('run-host', 'A');
  assert.equal(p.field('run-cores-per-node').value, '32');
});

test('automatic settings survive page recreation and legacy settings stay manual', async () => {
  const p = await page();
  await p.set('run-nodes', '4');
  await p.set('run-cores-per-node', '32');
  p.persist();
  const restored = await page({ settings: p.state.projectRunSettings });
  assert.equal(restored.field('run-parallel').value, 'auto');
  assert.equal(restored.field('run-nodes').value, '4');
  assert.equal(restored.field('run-cores-per-node').value, '32');
  const otherHost = await page({ host: 'B', settings: p.state.projectRunSettings });
  assert.equal(otherHost.field('run-nodes').value, '3');
  assert.equal(otherHost.field('run-cores-per-node').value, '64');
  await otherHost.set('run-host', 'A');
  assert.equal(otherHost.field('run-nodes').value, '4');
  assert.equal(otherHost.field('run-cores-per-node').value, '32');
  const legacy = await page({
    settings: {
      root: '/project',
      cases: ['/case'],
      host: 'A',
      ranks: '6',
      threads: '4',
      workers: '1',
      scheduler: 'slurm',
      cpus: '4',
    },
  });
  assert.equal(legacy.field('run-parallel').value, 'manual');
  assert.equal(legacy.field('run-ranks').value, '6');
  assert.equal(legacy.field('run-threads').value, '4');
  await legacy.set('run-parallel', 'auto');
  await legacy.set('run-parallel', 'manual');
  assert.equal(legacy.field('run-ranks').value, '6');
  assert.equal(legacy.field('run-threads').value, '4');
});

test('automatic plans show the resolved resources and submit the raw budget', async () => {
  const p = await page({ language: 'en' });
  assert.match(
    p.text(),
    /2 nodes × 48 cores per node; 2 processes with 48 threads each; total CPU budget: 96/,
  );
  assert.match(p.text(), /Configured memory budget: 64 GB per node/);
  await p.set('run-engine', 'fortran');
  assert.match(p.text(), /96 processes with 1 thread each/);
  await (p.submit().props.run as () => Promise<unknown>)();
  const call = p.calls.find((entry) => entry.command === 'remote_run')!;
  assert.equal(call.args.parallel, 'auto');
  assert.equal(call.args.nodes, 2);
  assert.equal(call.args.coresPerNode, 48);
  assert.equal(call.args.ranks, null);
  assert.equal(call.args.threads, null);
  assert.equal(call.args.cpus, null);
});

test('invalid budgets and non-Slurm multi-node plans are visible and cannot start', async () => {
  const p = await page({ language: 'en' });
  await p.set('run-nodes', '0');
  assert.equal(p.submit().props.disabled, true);
  assert.match(p.text(), /Nodes and cores per node must be integers/);
  await assert.rejects((p.submit().props.run as () => Promise<unknown>)());
  assert.ok(!p.calls.some((call) => call.command === 'model_migrate_case'));
  await p.set('run-nodes', '2');
  for (const scheduler of ['bare', 'pbs', 'lsf']) {
    await p.set('run-scheduler', scheduler);
    assert.equal(p.submit().props.disabled, true);
    assert.match(p.text(), /Automatic multi-node plans currently require Slurm/);
  }
  await p.set('run-nodes', '1');
  assert.equal(p.submit().props.disabled, false);
});

test('automatic budget fields share the active-run configuration lock', async () => {
  const p = await page();
  p.state.running['/case'] = {
    case: '/case',
    run_id: 'active',
    running: true,
    status: 'running',
    location: 'remote',
    logs: [],
    stages: {},
  };
  p.render();
  for (const id of ['run-parallel', 'run-nodes', 'run-cores-per-node'])
    assert.equal(p.field(id).disabled, true, id);
  await p.set('run-nodes', '9');
  await p.set('run-parallel', 'manual');
  assert.equal(p.field('run-nodes').value, '2');
  assert.equal(p.field('run-parallel').value, 'auto');
});

test('single stations and batches do not expose the spatial automatic budget', async () => {
  for (const options of [
    { spatial: false },
    { spatial: false, batch: true },
    { spatial: true, batch: true },
  ]) {
    const p = await page(options);
    assert.ok(
      !p
        .fields()
        .some((field) => ['run-parallel', 'run-nodes', 'run-cores-per-node'].includes(field.id)),
    );
  }
  const spatial = await page();
  await spatial.set('run-host', '');
  assert.ok(
    spatial.field('run-threads'),
    'a spatial case remains threaded after switching to local',
  );
  assert.ok(!spatial.field('run-parallel'));
});
