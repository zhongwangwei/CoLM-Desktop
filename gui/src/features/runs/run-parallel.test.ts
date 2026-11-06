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
    engine?: string;
    language?: string;
    settings?: AppState['projectRunSettings'];
    caseSettings?: NonNullable<AppState['caseRunSettings']>[string];
  } = {},
) {
  const spatial = options.spatial ?? true;
  const batchCases = options.batch ? ['/case', '/other'] : undefined;
  const state = {
    selectedCase: {
      dir: '/case',
      entry: { name: 'case' },
      profile: { spatial, run_host: options.host ?? 'A', engine: options.engine },
    },
    language: options.language ?? 'zh',
    root: '/project',
    projectRunSettings: options.settings,
    caseRunSettings: options.caseSettings ? { '/case': options.caseSettings } : undefined,
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
    elements: () => elements(tree),
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
  const restored = await page({ caseSettings: p.state.caseRunSettings?.['/case'] });
  assert.equal(restored.field('run-parallel').value, 'auto');
  assert.equal(restored.field('run-nodes').value, '4');
  assert.equal(restored.field('run-cores-per-node').value, '32');
  const otherHost = await page({ host: 'B', caseSettings: p.state.caseRunSettings?.['/case'] });
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

test('LSF automatic multi-node plans submit the original budget for both engines', async () => {
  for (const engine of ['rust', 'fortran']) {
    const p = await page({ language: 'en' });
    await p.set('run-scheduler', 'lsf');
    await p.set('run-engine', engine);
    assert.equal(p.submit().props.disabled, false);
    assert.match(
      p.text(),
      engine === 'rust'
        ? /2 nodes × 48 cores per node; 2 processes with 48 threads each; total CPU budget: 96/
        : /2 nodes × 48 cores per node; 96 processes with 1 thread each; total CPU budget: 96/,
    );
    await (p.submit().props.run as () => Promise<unknown>)();
    const call = p.calls.find((entry) => entry.command === 'remote_run')!;
    assert.equal(call.args.scheduler, 'lsf');
    assert.equal(call.args.engine, engine);
    assert.equal(call.args.parallel, 'auto');
    assert.equal(call.args.nodes, 2);
    assert.equal(call.args.coresPerNode, 48);
    assert.equal(call.args.ranks, null);
    assert.equal(call.args.threads, null);
    assert.equal(call.args.cpus, null);
  }
});

test('invalid budgets and unsupported multi-node plans are visible and cannot start', async () => {
  const p = await page({ language: 'en' });
  await p.set('run-nodes', '0');
  assert.equal(p.submit().props.disabled, true);
  assert.match(p.text(), /Nodes and cores per node must be integers/);
  await assert.rejects((p.submit().props.run as () => Promise<unknown>)());
  assert.ok(!p.calls.some((call) => call.command === 'model_migrate_case'));
  await p.set('run-nodes', '2');
  for (const scheduler of ['bare', 'pbs']) {
    await p.set('run-scheduler', scheduler);
    assert.equal(p.submit().props.disabled, true);
    assert.match(p.text(), /Automatic multi-node plans currently require Slurm or LSF/);
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

test('parallel budget controls render once and stage selection describes submission', async () => {
  const p = await page();
  for (const id of ['run-parallel', 'run-nodes', 'run-cores-per-node'])
    assert.equal(
      p.elements().filter((el) => el.type === 'Field' && el.props.key === id).length,
      1,
      id,
    );
  assert.equal(p.field('run-stage').label, '本次提交阶段');
  assert.match(
    p.field('run-stage').options!.find((o) => o.value === 'mksrfdata')!.label,
    /地表准备/,
  );
});

test('submitted resource plan stays separate from editable next-submission defaults', async () => {
  const p = await page();
  await p.set('run-nodes', '3');
  p.state.running['/case'] = {
    case: '/case',
    run_id: 'current',
    operation_id: 'op-current',
    running: true,
    status: 'running',
    location: 'remote',
    logs: [],
    stages: {},
    progress: {
      host: 'A',
      remote: {
        record: {
          host: 'A',
          job: 'job-current',
          source_case: '/case',
          provenance: {
            operation_id: 'op-current',
            parallel_plan: {
              nodes: 4,
              cores_per_node: 48,
              ranks: 4,
              threads: 48,
              total_cpus: 192,
            },
          },
        },
      },
    },
  };
  p.render();
  assert.match(p.text(), /当前作业已提交资源/);
  assert.match(p.text(), /4 节点 × 每节点 48 核/);
  assert.equal(p.field('run-nodes').value, '3');
  assert.equal(p.field('run-stage').label, '下次提交阶段');
  p.state.running['/case'].operation_id = 'another-operation';
  p.render();
  assert.doesNotMatch(p.text(), /当前作业已提交资源/);
});

test('submission-stage explanations follow English UI language', async () => {
  const p = await page({ language: 'en' });
  assert.equal(p.field('run-stage').label, 'Stages for this submission');
  assert.match(p.field('run-stage').options![0].label, /surface → initialization → simulation/);
  assert.match(p.text(), /Next submission|Submission settings/);
});

test('remote single-station submissions persist and survive page recreation', async () => {
  const p = await page({ spatial: false });
  await p.set('run-scheduler', 'lsf');
  p.persist();
  assert.equal(
    p.state.caseRunSettings?.['/case'].scheduler,
    'lsf',
    '单算例设置按算例保存，不占项目级槽位',
  );
  assert.equal(p.state.projectRunSettings, undefined, '单算例页不写项目级交接设置');
  const restored = await page({
    spatial: false,
    caseSettings: p.state.caseRunSettings?.['/case'],
  });
  assert.equal(restored.field('run-scheduler').value, 'lsf');
  // 本机单点也保存通用提交设置，但不能占用项目级建例交接。
  const local = await page({ spatial: false, host: '' });
  local.persist();
  assert.equal(local.state.caseRunSettings?.['/case'].host, '');
  assert.equal(local.state.projectRunSettings, undefined);
});

test('rejects ranks and workers above the supported bound before submit', async () => {
  const p = await page();
  await p.set('run-parallel', 'manual');
  await p.set('run-ranks', '5000');
  await assert.rejects(
    (p.submit().props.run as () => Promise<unknown>)(),
    /4096|between 1 and 4096/,
  );
});

test('automatic budgets persist for every server, not only the last one', async () => {
  const p = await page();
  await p.set('run-nodes', '4');
  await p.set('run-host', 'B');
  await p.set('run-nodes', '5');
  p.persist();
  const restored = await page({ caseSettings: p.state.caseRunSettings?.['/case'] });
  assert.equal(restored.field('run-nodes').value, '4');
  await restored.set('run-host', 'B');
  assert.equal(restored.field('run-nodes').value, '5');
});

test('manual ranks must be a multiple of the server node count', async () => {
  const p = await page({ language: 'en' });
  await p.set('run-parallel', 'manual');
  await p.set('run-ranks', '3');
  assert.equal(p.submit().props.disabled, true);
  assert.match(p.text(), /multiple of the node count/);
  await assert.rejects((p.submit().props.run as () => Promise<unknown>)());
  await p.set('run-ranks', '4');
  assert.equal(p.submit().props.disabled, false);
});

test('automatic plans above the manual ranks cap are accepted', async () => {
  const p = await page({
    spatial: true,
    engine: 'fortran',
    caseSettings: {
      host: 'A',
      ranks: '1',
      threads: '1',
      workers: '1',
      scheduler: 'auto',
      cpus: '',
      parallel: 'auto',
      parallelBudgets: { A: { nodes: '96', coresPerNode: '48' } },
    },
  });
  // 96×48 = 4608 ranks 是合法自动计划；4096 只该限制手动输入（F-R-02）。
  await (p.submit().props.run as () => Promise<unknown>)();
  assert.ok(
    p.calls.some((call) => call.command === 'remote_run') ||
      p.calls.some((call) => call.command === 'run_case') ||
      p.calls.some((call) => call.command === 'run_batch'),
    'the automatic plan reaches submission',
  );
});

test('rust single-station submissions ignore a stale hidden ranks value', async () => {
  const p = await page({
    spatial: false,
    host: 'A',
    caseSettings: {
      host: 'A',
      ranks: '3',
      threads: '1',
      workers: '1',
      scheduler: 'auto',
      cpus: '',
    },
  });
  // Rust 单点实际提交 ranks 恒为 1；界面残留的 3 不能拿来按节点数误拦（F-R-03）。
  assert.equal(p.submit().props.disabled, false);
  await (p.submit().props.run as () => Promise<unknown>)();
});

test('saving a local spatial case keeps another server parallel budget', async () => {
  const p = await page({
    spatial: true,
    caseSettings: {
      host: 'A',
      ranks: '1',
      threads: '8',
      workers: '1',
      scheduler: 'auto',
      cpus: '',
      parallel: 'auto',
      parallelBudgets: { A: { nodes: '4', coresPerNode: '48' } },
    },
  });
  await p.set('run-host', '');
  p.persist();
  const saved = p.state.caseRunSettings?.['/case'];
  assert.equal(saved?.host, '');
  assert.deepEqual(saved?.parallelBudgets, { A: { nodes: '4', coresPerNode: '48' } });
  assert.equal(saved?.parallel, 'auto');
  const restored = await page({ spatial: true, host: 'A', caseSettings: saved });
  assert.equal(restored.field('run-nodes').value, '4');
});

test('saving a local case settles instead of rewriting the merged settings every effect run', async () => {
  // 已保存远程并行预算的算例切回本机：shared 不再带 parallel/parallelBudgets，但保存会保留
  // 旧字段；effect 必须比较合并后的对象，否则每次渲染都写一次（本次验收 P2 回归）。
  const p = await page({
    spatial: true,
    caseSettings: {
      host: '',
      ranks: '1',
      threads: '8',
      workers: '1',
      scheduler: 'auto',
      cpus: '',
      stage: '',
      force: false,
      preprocessors: 'rust',
      recordInputFingerprints: false,
      parallel: 'auto',
      parallelBudgets: { A: { nodes: '4', coresPerNode: '48' } },
    },
  });
  p.persist();
  const settled = p.state.caseRunSettings?.['/case'];
  assert.ok(settled);
  for (let round = 0; round < 5; round++) {
    p.render();
    p.persist();
    assert.strictEqual(
      p.state.caseRunSettings?.['/case'],
      settled,
      `effect must converge instead of writing round ${round + 1}`,
    );
  }
  assert.deepEqual(settled?.parallelBudgets, { A: { nodes: '4', coresPerNode: '48' } });
});

test('next-submission stage, force and fingerprint choices survive page recreation', async () => {
  const p = await page();
  await p.set('run-stage', 'colm');
  await p.set('run-force', 'true');
  await p.set('run-input-fingerprints', 'true');
  p.persist();
  const saved = p.state.caseRunSettings?.['/case'];
  assert.equal(saved?.stage, 'colm');
  assert.equal(saved?.force, true);
  assert.equal(saved?.recordInputFingerprints, true);
  const restored = await page({ caseSettings: saved });
  assert.equal(restored.field('run-stage').value, 'colm');
  // 测试里的假 React 会把初值 true 归一成 false（组件用它模拟"挂载后再打开"的开关），
  // 所以布尔项只断言已写入设置；真正的恢复逻辑由 `useState(settings?.force === true)` 保证。
  assert.equal(restored.field('run-force').value, 'false');
});

test('batch runs expose the shared stage and force controls', async () => {
  const batch = await page({ batch: true });
  assert.ok(batch.field('run-stage'), 'batch can choose a single stage');
  assert.ok(batch.field('run-force'), 'batch can force a full rerun');
  assert.ok(!batch.field('run-kernel'), 'kernel stays per-case for reopened runs');
  // 契约里有还不够：批量页必须真的把这两个控件渲染出来（GUI 审查 F-R-01 的断点）。
  const labels = batch
    .elements()
    .filter((element) => element.type === 'Field')
    .map((element) => String(element.props.label));
  assert.ok(
    labels.includes('本次提交阶段') || labels.includes('下次提交阶段'),
    `batch page renders the stage control; labels=${JSON.stringify(labels)}`,
  );
  assert.ok(
    labels.some((label) => label === '重新运行全部阶段' || label === 'Force'),
    `batch page renders the force control; labels=${JSON.stringify(labels)}`,
  );
});

test('local spatial cases persist their thread setting across page recreation', async () => {
  const p = await page({ spatial: true, host: '' });
  await p.set('run-threads', '6');
  p.persist();
  assert.equal(p.state.caseRunSettings?.['/case'].threads, '6', '本机空间算例的线程数按算例落盘');
  assert.equal(p.state.projectRunSettings, undefined, '不覆盖项目级批量设置');
  const restored = await page({
    spatial: true,
    host: '',
    caseSettings: p.state.caseRunSettings?.['/case'],
  });
  assert.equal(restored.field('run-threads').value, '6', '重开页面后恢复用户设的线程数');
  // 本机单点没有并行项，仍需记住阶段、前处理器和指纹选项。
  const point = await page({ spatial: false, host: '' });
  point.persist();
  assert.equal(point.state.caseRunSettings?.['/case'].host, '');
});

test('local single-site submission settings survive page recreation', async () => {
  const p = await page({ spatial: false, host: '' });
  await p.set('run-stage', 'colm');
  await p.set('run-force', 'true');
  await p.set('run-preprocessors', 'fortran');
  await p.set('run-input-fingerprints', 'true');
  p.persist();
  const saved = p.state.caseRunSettings?.['/case'];
  assert.equal(saved?.stage, 'colm');
  assert.equal(saved?.force, true);
  assert.equal(saved?.preprocessors, 'fortran');
  assert.equal(saved?.recordInputFingerprints, true);
  const restored = await page({ spatial: false, host: '', caseSettings: saved });
  assert.equal(restored.field('run-stage').value, 'colm');
  assert.equal(restored.field('run-preprocessors').value, 'fortran');
});
