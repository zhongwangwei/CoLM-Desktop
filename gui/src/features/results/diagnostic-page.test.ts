import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import * as seriesHelpers from './series';
import * as chartHelpers from './chart-data';
import * as overviewHelpers from './result-overview';
import * as variableLabels from './variable-labels';
import * as analysisHelpers from './analysis-state';
import * as timeBasisHelpers from './time-basis';
import type { PageContract, UiAction } from '../../lib/guide';

type Element = { type: string | { name: string }; props: Record<string, unknown> };
const normal = {
  time: [1, 2],
  vars: { f_lfevpa: [100, 200] },
  source_n: 2,
  n: 2,
  downsampled: false,
};
const residual = {
  time: [1, 2],
  vars: { energy_residual: [3, 5] },
  source_n: 2,
  n: 2,
  downsampled: false,
};
/** 抽样后的序列：2 与 3 之间那段缺测在抽样里整段消失，只剩 gaps 里的区间。 */
const gapped = {
  time: [1, 2, 3, 4],
  vars: { f_lfevpa: [100, 200, 300, 400] },
  source_n: 100001,
  n: 4,
  downsampled: true,
  gaps: { f_lfevpa: [[1.5, 2.5]] as [number, number][] },
};
/** 所选窗口内一条记录都没有。 */
const emptySeries = {
  time: [] as number[],
  vars: { f_lfevpa: [] as (number | null)[] },
  source_n: 0,
  n: 0,
  downsampled: false,
};
const diagnostic = {
  schema: 1,
  expression: 'Rnet-Qh-Qle-Qg',
  closure_assessed: false,
  full_stats: {
    basis: 'all_records_in_selected_window',
    mean_kind: 'arithmetic_mean_of_common_finite_records',
    status: 'computed',
    source_n: 2,
    n: 2,
    missing: 0,
    mean: 4,
    units: 'W/m²',
  },
  series: residual,
};
function harness(
  stations = false,
  delayedSeries?: Promise<unknown>,
  openError = '',
  initialCase = '/case',
  options: {
    remoteRun?: boolean;
    withBack?: boolean;
    /** 远程运行的本机结果状态（部分取回等）。 */
    localResults?: Record<string, unknown>;
    /** 这些观测路径的 metrics 调用会失败（可被测试实时清空，用来模拟重试成功）。 */
    failingObs?: string[];
    /** 抽样序列带缺测区间（验证图上的断线）。 */
    gaps?: boolean;
    /** 目录里再加一个区域（grid）变量，检查时序页变量下拉。 */
    gridVariable?: boolean;
    /** 读序列会失败；测试里删掉这个字段后重试成功。 */
    seriesFailure?: string;
    /** 读回来的序列在窗口内没有数据。 */
    seriesEmpty?: boolean;
    /** 读运行清单会失败；测试里删掉这个字段后重试成功。 */
    runsFailure?: string;
  } = {},
) {
  let selectedCase = initialCase;
  const opened: string[] = [],
    routes: string[] = [],
    updates: unknown[] = [];
  const requests: { command: string; args: Record<string, unknown> }[] = [];
  const source = readFileSync(new URL('./ResultsPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function')
    .replace('export function ScienceChart', 'function ScienceChart');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  let cursor = 0,
    dirty = false,
    effects: (() => void)[] = [];
  const slots: unknown[] = [],
    exports: string[] = [];
  let contract: PageContract, tree: Element;
  const same = (a: unknown[] | undefined, b: unknown[]) =>
    a?.length === b.length && a.every((x, i) => Object.is(x, b[i]));
  const invokeJson = async (command: string, args: Record<string, unknown>) => {
    requests.push({ command, args });
    if (command === 'remote_fetch') return null;
    // 运行清单读取失败的链路：测试删掉 options.runsFailure 后，同一个重试动作会成功。
    if (command === 'model_case_runs' && options.runsFailure) throw Error(options.runsFailure);
    // 曲线读取失败的链路：测试删掉 options.seriesFailure 后，重试按钮会取到真实数据。
    if (command === 'series' && options.seriesFailure) throw Error(options.seriesFailure);
    // 比较重试用例：先让这些观测失败，测试里清空列表后同样的路径会返回指标。
    if (command === 'metrics' && options.failingObs?.includes(String(args.obs)))
      throw Error('no readable history for this run');
    if (command === 'model_case_runs' && args.dir === '/empty') return [];
    if (command === 'model_case_runs' && args.dir === '/othercase')
      return [
        {
          run_case: '/othercase/runs/b',
          version: { id: 'official', revision: 3 },
          status: 'succeeded',
        },
      ];
    if (command === 'model_case_runs')
      return [
        // The ledger's imported placeholder comes first and has no output of its own.
        { run_case: '/case', version: null, legacy: true, status: 'needs_reconciliation' },
        {
          run_case: '/case/.colm/runs/a',
          version: { id: 'official', revision: 2 },
          status: 'succeeded',
          ...(options.remoteRun
            ? {
                server: 'server',
                outputs_local: false,
                ...(options.localResults ? { local_results: options.localResults } : {}),
              }
            : {}),
        },
      ];
    if (command === 'model_project_runs')
      return [
        {
          run_case: '/case/.colm/runs/a',
          case_dir: '/case',
          case_name: 'case',
          version: { id: 'official', revision: 2 },
          status: 'succeeded',
          finished_ms: 1,
          outputs_local: true,
        },
      ];
    if (command === 'history_catalog')
      return {
        files: 1,
        steps: 2,
        start: 1,
        end: 2,
        variables: [
          { name: 'f_lfevpa', kind: 'series', units: 'W/m²', dimensions: [] },
          ...(options.gridVariable
            ? [
                {
                  name: 'f_tref',
                  kind: 'grid',
                  units: 'K',
                  dimensions: ['time', 'lat', 'lon'],
                },
              ]
            : []),
        ],
      };
    if (command === 'series')
      return delayedSeries ?? (options.seriesEmpty ? emptySeries : options.gaps ? gapped : normal);
    if (command === 'evaluation_catalog') return [{ name: 'Qle', available: true }];
    // 单站评估：只有 /obs/single.nc 这条路径返回一行指标，避免自动评估等用例拿到假数据。
    if (command === 'metrics' && args.obs === '/obs/single.nc')
      return [
        {
          name: 'Qle',
          n: 2,
          rmse: 2,
          mae: 1,
          bias: 1,
          r2: 1,
          correlation: 1,
          nse: 1,
          kge: 1,
          units: 'W/m²',
          source_units: 'kW/m2',
          qc_kind: 'generated_validity',
          qc_rule: 'finite=0, missing=1',
          correction_status: 'not_applied',
          correction_reason: 'Qle_cor missing',
          coverage: 0.6,
          coverage_min: 0.25,
        },
      ];
    // 比较重试用例：这条观测重试成功时返回一行指标（失败时上面的 failingObs 分支先命中）。
    if (command === 'metrics' && args.obs === '/obs/b.nc')
      return [
        {
          name: 'Qle',
          n: 2,
          rmse: 3,
          mae: 2,
          bias: 1,
          r2: 1,
          correlation: 1,
          nse: 1,
          kge: 1,
        },
      ];
    if (command === 'energy_summary') return diagnostic;
    // 多站点比较：每个"运行副本 | 观测"条目调一次 metrics；只在比较测试用的观测路径上返回一行指标，
    // 其他路径照旧抛错，免得把自动评估等用例的结果喂假装数据。
    if (command === 'metrics' && args.obs === '/obs/compare.nc')
      return [
        {
          name: 'f_lfevpa',
          n: 2,
          rmse: 1,
          mae: 1,
          bias: 0,
          r2: 1,
          correlation: 1,
          nse: 1,
          kge: 1,
          units: 'W/m²',
          qc_kind: 'measured_only',
          qc_variable: 'Qle_qc',
          correction_status: 'applied',
          correction_variable: 'Qle_cor',
          coverage: 0.85,
        },
      ];
    throw Error(command);
  };
  const env: Record<string, unknown> = {
    ...seriesHelpers,
    ...chartHelpers,
    ...overviewHelpers,
    ...variableLabels,
    ...analysisHelpers,
    ...timeBasisHelpers,
    GridMap: () => {},
    invokeJson,
    invoke: invokeJson,
    hasBackend: () => true,
    // 页面源码里的报错/失败原因走 bilingual（import 被剥掉）；测试按中文返回。
    bilingual: (zh: string) => zh,
    utcSeconds: (value: string) => Date.parse(value) / 1000,
    defaultObservation: async () => '',
    timeTicks: () => [],
    uPlot: () => {},
    download: async (_name: string, text: string) => {
      exports.push(text);
      return { status: 'saved', path: '/saved.json' };
    },
    React: {
      createElement: (type: Element['type'], props: Element['props'], ...children: unknown[]) => ({
        type,
        props: { ...props, children },
      }),
    },
    useApp: () => ({
      state: {
        language: 'en',
        theme: 'light',
        versionRef: { id: 'official', revision: 1 },
        cases: stations
          ? [
              { dir: '/case', name: 'First' },
              { dir: '/empty', name: 'Empty' },
            ]
          : [{ dir: '/case' }],
        running: {},
      },
      navigate(route: string) {
        routes.push(route);
        return true;
      },
      update(patch: unknown) {
        updates.push(patch);
      },
      async openCase(dir: string) {
        opened.push(dir);
        if (openError === 'blocked') return null;
        if (openError) throw new Error(openError);
        selectedCase = dir;
        return { dir };
      },
    }),
    usePageContract: (value: PageContract) => {
      contract = value;
    },
    useState(initial: unknown) {
      const i = cursor++;
      if (!(i in slots)) slots[i] = typeof initial === 'function' ? initial() : initial;
      return [
        slots[i],
        (value: unknown) => {
          const next = typeof value === 'function' ? value(slots[i]) : value;
          if (!Object.is(next, slots[i])) {
            slots[i] = next;
            dirty = true;
          }
        },
      ];
    },
    useRef(initial: unknown) {
      const i = cursor++;
      return slots[i] ?? (slots[i] = { current: initial });
    },
    useCallback(fn: unknown, deps: unknown[]) {
      const i = cursor++,
        old = slots[i] as { deps: unknown[]; fn: unknown } | undefined;
      if (!same(old?.deps, deps)) slots[i] = { fn, deps };
      return (slots[i] as { fn: unknown }).fn;
    },
    useEffect(fn: () => (() => void) | void, deps: unknown[]) {
      const i = cursor++,
        old = slots[i] as { deps: unknown[]; cleanup?: () => void } | undefined;
      if (!same(old?.deps, deps)) {
        const slot = { deps, cleanup: undefined as (() => void) | undefined };
        slots[i] = slot;
        effects.push(() => {
          old?.cleanup?.();
          slot.cleanup = fn() || undefined;
        });
      }
    },
  };
  for (const name of [
    'ActionButton',
    'Button',
    'Card',
    'Field',
    'Input',
    'Select',
    'Notice',
    'Advanced',
    'Dialog',
    'NavIcon',
    'RemotePathBrowser',
  ])
    env[name] = name;
  const Page = new Function(...Object.keys(env), `${code};return ResultsPage;`)(
    ...Object.values(env),
  );
  async function render() {
    for (let n = 0; n < 15; n++) {
      dirty = false;
      cursor = 0;
      tree = Page({
        casePath: selectedCase,
        onBack: options.withBack ? () => routes.push('back') : undefined,
      });
      const pending = effects;
      effects = [];
      pending.forEach((fn) => fn());
      await new Promise((resolve) => setImmediate(resolve));
      if (!dirty && !effects.length) return;
    }
    throw Error('Render did not settle');
  }
  function charts(node: unknown): unknown[] {
    if (!node || typeof node !== 'object') return [];
    if (Array.isArray(node)) return node.flatMap(charts);
    const element = node as Element;
    return [
      ...(typeof element.type === 'function' && element.type.name === 'ScienceChart'
        ? [{ labels: element.props.labels, data: element.props.data }]
        : []),
      ...charts(element.props?.children),
    ];
  }
  // 动作由按钮登记（第 737 轮）：只收此刻显示着的 ActionButton；有打开的对话框时只收最上层对话框里的。
  // 对话框在 Radix 里关闭时不挂载内容，所以关着的 Dialog 子树整棵跳过。
  function treeActions(): UiAction[] {
    const collected: UiAction[] = [];
    const visit = (node: unknown) => {
      if (Array.isArray(node)) {
        node.forEach(visit);
        return;
      }
      if (!node || typeof node !== 'object') return;
      const element = node as Element;
      const props = element.props as Record<string, unknown> & {
        action?: string;
        actionLabel?: string;
        title?: string;
        kind?: UiAction['kind'];
        disabled?: boolean;
        open?: boolean;
        onOpenChange?: (open: boolean) => void;
        run?: () => unknown;
        label?: string;
        closeAction?: string;
      };
      if (element.type === 'Dialog') {
        if (!props.open) return;
        if (props.closeAction && props.showClose !== false)
          collected.push({
            id: props.closeAction,
            label: 'Close',
            kind: 'click',
            run: () => props.onOpenChange!(false),
          });
        visit(props.children);
        return;
      }
      if (element.type === 'ActionButton')
        collected.push({
          id: String(props.action),
          label: String(props.label),
          kind: props.kind ?? 'click',
          enabled: !props.disabled,
          run: props.run!,
        });
      if (element.type === 'Advanced' && props.action)
        collected.push({
          id: String(props.action),
          label: String(props.actionLabel ?? props.title ?? ''),
          kind: 'click',
          run: () => props.onOpenChange!(!props.open),
        });
      // 合上的折叠区内容在真实界面里不可见（details 未展开），里面的按钮不算显示着。
      if (element.type === 'Advanced' && props.open === false) return;
      if (element.type === 'details' && !props.open) return;
      visit(props.children);
    };
    const dialogs: Element[] = [];
    const collectDialogs = (node: unknown) => {
      if (Array.isArray(node)) return node.forEach(collectDialogs);
      if (!node || typeof node !== 'object') return;
      const element = node as Element;
      if (element.type === 'Dialog' && element.props.open) dialogs.push(element);
      collectDialogs(element.props?.children);
    };
    collectDialogs(tree);
    if (dialogs.length) visit(dialogs[dialogs.length - 1]);
    else visit(tree);
    return collected;
  }
  const guide = (): PageContract => ({
    ...contract,
    actions: [...contract.actions, ...treeActions()],
  });
  return {
    render,
    exports,
    opened,
    routes,
    updates,
    requests,
    field: (id: string) => contract.fields!.find((field) => field.id === id),
    station: () => contract.fields!.find((field) => field.id === 'result-station'),
    async chooseStation(dir: string) {
      contract.fields!.find((field) => field.id === 'result-station')!.set!(dir);
      await new Promise((resolve) => setImmediate(resolve));
      await render();
    },
    charts: () => charts(tree),
    tree: () => tree,
    allowSwitch: () => {
      openError = '';
    },
    async tab(value: string) {
      contract.fields!.find((f) => f.id === 'result-tab')!.set!(value);
      await render();
    },
    guide,
    async action(id: string) {
      await guide()
        .actions.find((a) => a.id === id)!
        .run();
      await render();
    },
  };
}
test('energy diagnostics keep residual charts and exports after visiting the overview', async () => {
  const h = harness();
  await h.render();
  await h.tab('diagnostics');
  await h.action('diagnose-closure');
  const expected = [
    {
      labels: ['energy_residual'],
      data: [
        [1, 2],
        [3, 5],
      ],
    },
  ];
  assert.deepEqual(h.charts(), expected);
  await h.tab('overview');
  await h.tab('diagnostics');
  assert.deepEqual(h.charts(), expected);
  await h.action('export-series-data');
  assert.deepEqual(JSON.parse(h.exports[0]).vars, residual.vars);
  await h.tab('series');
  await h.action('export-series-data');
  assert.deepEqual(JSON.parse(h.exports[1]).vars, normal.vars);
});

test('project station selector switches cases and removes the previous station charts', async () => {
  const h = harness(true);
  await h.render();
  assert.deepEqual(h.station()?.options, [
    { value: '/case', label: 'First' },
    { value: '/empty', label: 'Empty' },
  ]);
  assert.ok(h.charts().length);
  await h.chooseStation('/empty');
  assert.deepEqual(h.opened, ['/empty']);
  assert.equal(h.station()?.value, '/empty');
  assert.deepEqual(h.charts(), []);
  assert.ok(
    h.requests.some(
      (request) => request.command === 'model_case_runs' && request.args.dir === '/empty',
    ),
  );
});

test('delayed series from the previous station cannot populate the new station', async () => {
  let finish!: (value: unknown) => void;
  const h = harness(
    true,
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  await h.render();
  assert.ok(h.requests.some((request) => request.command === 'series'));
  await h.chooseStation('/empty');
  finish(normal);
  await h.render();
  assert.deepEqual(h.charts(), []);
  assert.equal(h.station()?.value, '/empty');
});

test('a failed station switch reloads the original station instead of leaving a blank page', async () => {
  const h = harness(true, undefined, 'Cannot read station');
  await h.render();
  assert.ok(h.charts().length);
  await h.chooseStation('/empty');
  assert.equal(h.station()?.value, '/case');
  assert.ok(h.charts().length, 'original station data reloads after the failure');
  assert.ok(
    h.requests.filter(({ command, args }) => command === 'model_case_runs' && args.dir === '/case')
      .length >= 2,
  );
  assert.ok(JSON.stringify(h.tree()).includes('Cannot read station'));
  h.allowSwitch();
  await h.chooseStation('/empty');
  assert.equal(h.station()?.value, '/empty');
  assert.ok(!JSON.stringify(h.tree()).includes('Cannot read station'));
});

test('blocked navigation restores the original station and an unselected single station has a selectable placeholder', async () => {
  const blocked = harness(true, undefined, 'blocked');
  await blocked.render();
  await blocked.chooseStation('/empty');
  assert.equal(blocked.station()?.value, '/case');
  assert.ok(blocked.charts().length);
  const empty = harness(false, undefined, '', '');
  await empty.render();
  assert.equal(empty.station()?.value, '');
  function placeholders(value: unknown): Element[] {
    if (Array.isArray(value)) return value.flatMap(placeholders);
    if (!value || typeof value !== 'object') return [];
    const node = value as Element;
    return [
      ...(node.type === 'option' && node.props.value === '' && node.props.disabled ? [node] : []),
      ...placeholders(node.props?.children),
    ];
  }
  assert.equal(placeholders(empty.tree()).length, 1);
  await empty.chooseStation('/case');
  assert.deepEqual(empty.opened, ['/case']);
  assert.ok(empty.charts().length);
});

test('the default result is the recorded run, not the legacy placeholder listed before it', async () => {
  const h = harness(true);
  await h.render();
  const catalogs = h.requests.filter((request) => request.command === 'history_catalog');
  assert.ok(catalogs.length);
  assert.ok(catalogs.every((request) => request.args.case === '/case/.colm/runs/a'));
});

// 第 737 轮：助手看到的动作必须就是界面上此刻显示着的按钮（名字、可用状态都取自按钮本身）。
// 原来 PageContract 里另写一份 actions 清单，和 JSX 是两份；这些用例直接从渲染树收集 ActionButton。
const actionIds = (h: ReturnType<typeof harness>) =>
  h
    .guide()
    .actions.map((action) => action.id)
    .sort();
const actionById = (h: ReturnType<typeof harness>, id: string) =>
  h.guide().actions.find((action) => action.id === id);

test('the assistant sees exactly the result buttons on the overview screen', async () => {
  const h = harness();
  await h.render();
  assert.deepEqual(actionIds(h), [
    'export-full-series-csv',
    'result-closure',
    'result-improve',
    'result-runs',
    'result-settings',
  ]);
  assert.equal(actionById(h, 'export-full-series-csv')?.kind, 'commit');
  assert.equal(actionById(h, 'result-runs')?.kind, 'click');
  assert.equal(actionById(h, 'result-runs')?.enabled, true);
});

test('without a recorded run the assistant sees the header and export buttons as disabled', async () => {
  const h = harness(false, undefined, '', '/empty');
  await h.render();
  assert.deepEqual(actionIds(h), [
    'export-full-series-csv',
    'result-closure',
    'result-improve',
    'result-runs',
    'result-settings',
  ]);
  // 没有运行记录时，界面上这些按钮是灰的，助手看到的也必须是不能按；
  // "诊断偏差""模型改进"只切页，没有运行也仍可按。
  for (const id of ['export-full-series-csv', 'result-runs', 'result-settings'])
    assert.equal(actionById(h, id)?.enabled, false, id);
  for (const id of ['result-closure', 'result-improve'])
    assert.equal(actionById(h, id)?.enabled, true, id);
});

test('an open evaluation dialog hides the page actions and exposes only its own', async () => {
  const h = harness();
  await h.render();
  await h.action('result-settings');
  assert.deepEqual(actionIds(h), [
    'evaluate-observations',
    'plot-series',
    'result-advanced-settings',
    'result-pick-observation',
    'result-settings-close',
  ]);
  assert.equal(actionById(h, 'evaluate-observations')?.enabled, false, 'no observation yet');
  assert.equal(actionById(h, 'plot-series')?.enabled, true);
  await h.action('result-settings-close');
  assert.deepEqual(actionIds(h), [
    'export-full-series-csv',
    'result-closure',
    'result-improve',
    'result-runs',
    'result-settings',
  ]);
});

test('an open run chooser lists the recorded runs and keeps the page buttons out', async () => {
  const h = harness();
  await h.render();
  await h.action('result-runs');
  assert.deepEqual(actionIds(h), [
    'result-run-0',
    'result-run-1',
    'result-run-comparison',
    'result-runs-close',
  ]);
  await h.action('result-run-1');
  assert.ok(!actionIds(h).includes('result-runs-close'), 'choosing a run closes the dialog');
  assert.ok(actionIds(h).includes('result-runs'));
});

test('每个结果 tab 的动作跟着该 tab 上真实的按钮走', async () => {
  const h = harness();
  await h.render();
  await h.tab('report');
  // 模型改进只留在结果总览的"下一步"卡片里，不再和其他页签的"返回总览"并列。
  assert.deepEqual(actionIds(h), ['export-report', 'result-overview', 'result-runs']);
  assert.equal(actionById(h, 'export-report')?.kind, 'commit');
  await h.tab('series');
  assert.deepEqual(actionIds(h), [
    'export-full-series-csv',
    'export-series-data',
    'plot-series',
    'result-overview',
    'result-runs',
  ]);
  await h.tab('diagnostics');
  assert.deepEqual(actionIds(h), ['diagnose-closure', 'result-overview', 'result-runs']);
  await h.action('diagnose-closure');
  assert.ok(actionIds(h).includes('export-series-data'));
});

test('远程运行的取回结果与返回模拟按钮只在对应状态下登记', async () => {
  const remote = harness(false, undefined, '', '/case', { remoteRun: true });
  await remote.render();
  assert.equal(actionById(remote, 'result-fetch')?.kind, 'commit');
  assert.equal(actionById(remote, 'result-fetch')?.enabled, true);
  const back = harness(false, undefined, '', '/case', { withBack: true });
  await back.render();
  assert.equal(actionById(back, 'result-back')?.kind, 'click');
  assert.equal(actionById(back, 'result-back')?.enabled, true);
});

test('返回总览与模型改进按钮真的执行各自的导航', async () => {
  const h = harness();
  await h.render();
  await h.tab('diagnostics');
  await h.action('result-overview');
  assert.equal(h.field('result-tab')?.value, 'overview');
  await h.action('result-improve');
  assert.deepEqual(h.updates.at(-1), { versionsView: 'create' });
  assert.equal(h.routes.at(-1), 'versions');
});

test('评估、比较与数据目录页签只登记该页签真实的按钮', async () => {
  const h = harness();
  await h.render();
  await h.tab('evaluation');
  assert.deepEqual(actionIds(h), ['evaluate-observations', 'result-overview', 'result-runs']);
  assert.equal(actionById(h, 'evaluate-observations')?.enabled, false, '没有观测文件');
  await h.tab('comparison');
  assert.deepEqual(actionIds(h), [
    'compare-sites',
    'comparison-add-external',
    'comparison-apply-observation',
    'export-comparison-csv',
    'result-overview',
    'result-runs',
  ]);
  assert.equal(actionById(h, 'compare-sites')?.enabled, false, '还没有比较输入');
  await h.tab('catalog');
  assert.deepEqual(actionIds(h), ['result-overview', 'result-runs']);
});

test('导出按钮的 kind 与高级评估折叠的展开动作', async () => {
  const h = harness();
  await h.render();
  await h.tab('series');
  assert.equal(actionById(h, 'export-series-data')?.kind, 'commit');
  assert.equal(actionById(h, 'plot-series')?.kind, 'click');
  await h.tab('overview');
  await h.action('result-settings');
  assert.equal(actionById(h, 'result-advanced-settings')?.kind, 'click');
  await h.action('result-advanced-settings');
  assert.equal(h.field('result-advanced')?.value, 'true', '展开后字段也反映出来');
});

test('a server run picks its observation on the server, not with the local file dialog', async () => {
  const h = harness(false, undefined, '', '/case', { remoteRun: true });
  await h.render();
  await h.action('result-settings');
  await h.action('result-pick-observation');
  const walk = (node: unknown): { type: unknown; props: Record<string, unknown> }[] => {
    if (Array.isArray(node)) return node.flatMap(walk);
    if (!node || typeof node !== 'object') return [];
    const element = node as { type: unknown; props: Record<string, unknown> };
    return [element, ...walk(element.props?.children)];
  };
  const browser = walk(h.tree()).find((node) => node.type === 'RemotePathBrowser');
  assert.ok(browser, 'the server file browser opens');
  assert.equal(browser.props.host, 'server');
  assert.equal(browser.props.directory, false);
  assert.ok(!h.requests.some((request) => request.command === 'pick_file'));
  (browser.props.onSelect as (path: string) => void)('/srv/PLUMBER2s/Observation/X_Flux.nc');
  await h.render();
  assert.equal(h.field('result-observation')?.value, '/srv/PLUMBER2s/Observation/X_Flux.nc');
  assert.ok(
    !walk(h.tree()).some((node) => node.type === 'RemotePathBrowser'),
    'closed after choosing',
  );
});

test('analysis scope switches between one station and the multi-station comparison', async () => {
  const h = harness(true);
  await h.render();
  assert.equal(h.field('result-scope')?.value, 'single');
  assert.ok(h.station(), 'single-station scope keeps the station selector');
  assert.ok(
    !h.field('result-tab')?.options?.some((option) => option.value === 'report'),
    'the report-export view is no longer offered in the analysis view choices',
  );
  h.field('result-scope')!.set!('multi');
  await h.render();
  assert.equal(h.field('result-scope')?.value, 'multi');
  assert.equal(h.field('result-tab')?.value, 'comparison');
  assert.equal(h.station(), undefined, 'multi-station comparison is not tied to one station');
  assert.ok(actionIds(h).includes('compare-sites'));
  h.field('result-scope')!.set!('single');
  await h.render();
  assert.equal(h.field('result-tab')?.value, 'overview');
  assert.ok(h.station(), 'the station selector comes back');
});

test('single evaluation, comparison and the way back keep their own result snapshots', async () => {
  const h = harness(true);
  await h.render();
  await h.tab('evaluation');
  h.field('result-observation')!.set!('/obs/single.nc');
  await h.render();
  await h.action('evaluate-observations');
  const cells = (node: unknown): string[] => {
    if (Array.isArray(node)) return node.flatMap(cells);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    if (element.type === 'td')
      return [String((element.props.children ?? []).filter((x) => x != null).join(''))];
    return cells(element.props?.children);
  };
  assert.ok(cells(h.tree()).includes('Qle'), 'single evaluation row is shown');
  assert.ok(!cells(h.tree()).includes('f_lfevpa'), 'comparison rows never leak into single scope');
  // 指标表补的四列：单位、QC 来源、订正状态、有效覆盖率（后端新字段缺失时按旧行为显示 —）。
  const rendered = JSON.stringify(h.tree());
  assert.ok(rendered.includes('Generated validity mask'), 'QC provenance is shown');
  assert.ok(rendered.includes('Not applied'), 'unapplied correction is shown, not hidden');
  assert.ok(rendered.includes('60%'), 'coverage is shown as a percentage');

  // 多站比较：得到比较快照（f_lfevpa），单站快照仍保留在各自的 state 里。
  h.field('result-scope')!.set!('multi');
  await h.render();
  h.field('comparison-sources')!.set!('/case/.colm/runs/a | /obs/compare.nc');
  await h.render();
  assert.ok(!cells(h.tree()).includes('Qle'), 'single rows never leak into comparison scope');
  await h.action('compare-sites');
  assert.ok(cells(h.tree()).includes('f_lfevpa'), 'comparison row is shown');

  // 切回单站：页面回到原来的 evaluation 页签，显示的是单站快照，不是比较行。
  h.field('result-scope')!.set!('single');
  await h.render();
  assert.ok(cells(h.tree()).includes('Qle'), 'single snapshot survives the comparison round trip');
  assert.ok(!cells(h.tree()).includes('f_lfevpa'), 'comparison rows do not replace single rows');
});

test('the multi-station comparison asks for run copies and shows paired metrics per site', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  // 比较输入是"运行副本 | 观测文件"的平面列表：这里直接用契约字段填入一行，验证整条调用链。
  h.field('comparison-sources')!.set!('/case/.colm/runs/a | /obs/compare.nc');
  await h.render();
  await h.action('compare-sites');
  const text = (node: unknown): string[] => {
    if (Array.isArray(node)) return node.flatMap(text);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    if (element.type === 'td')
      return [String((element.props.children ?? []).filter((x) => x != null).join(''))];
    return text(element.props?.children);
  };
  const cells = text(h.tree());
  assert.ok(cells.includes('f_lfevpa'), 'the paired metric row is rendered');
  assert.ok(cells.includes('.colm · a'), 'the site column identifies the compared run copy');
  assert.ok(cells.includes('Measured QC'), 'QC provenance column comes from the backend field');
  assert.ok(cells.includes('Applied'), 'correction status column is shown');
  assert.ok(cells.includes('85%'), 'coverage column is shown');
  assert.ok(!cellError(h), 'no error notice after a valid comparison');
});
function cellError(h: { tree: () => unknown }): boolean {
  const walk = (node: unknown): boolean => {
    if (Array.isArray(node)) return node.some(walk);
    if (!node || typeof node !== 'object') return false;
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    if (element.type === 'Notice' && element.props.tone === 'error') return true;
    return walk(element.props?.children);
  };
  return walk(h.tree());
}

test('multi-station comparison loads project runs once and builds rows from ticks', async () => {
  const h = harness(true);
  await h.render();
  const stationScans = () =>
    h.requests.filter((request) => request.command === 'model_case_runs').length;
  const before = stationScans();
  h.field('result-scope')!.set!('multi');
  await h.render();
  // 一次项目级查询，不再为每个站点各调一次 model_case_runs。
  assert.equal(h.requests.filter((request) => request.command === 'model_project_runs').length, 1);
  assert.equal(stationScans(), before, 'the comparison list does not scan every station');
  const inputs = (node: unknown): { props: Record<string, unknown> }[] => {
    if (Array.isArray(node)) return node.flatMap(inputs);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    return [
      ...(element.type === 'input' ? [{ props: element.props }] : []),
      ...inputs(element.props?.children),
    ];
  };
  // 关着的对话框子树也在渲染树里（真实 Radix 不挂载内容），按处理函数认出运行勾选框。
  const checkbox = inputs(h.tree()).find(
    (input) =>
      input.props.type === 'checkbox' &&
      String(input.props.onChange).includes('toggleComparisonRun'),
  );
  assert.ok(checkbox, 'project runs are offered as checkboxes');
  (checkbox.props.onChange as () => void)();
  await h.render();
  assert.match(
    String(h.field('comparison-sources')?.value),
    /\/case\/\.colm\/runs\/a/,
    'ticking a run adds it to the comparison rows',
  );
  // 共用观测：填一次，点「应用到所有行」。
  h.field('result-observation')!.set!('/obs/compare.nc');
  await h.render();
  await h.action('comparison-apply-observation');
  assert.equal(h.field('comparison-sources')!.value, '/case/.colm/runs/a | /obs/compare.nc');
  await h.action('comparison-remove-0');
  assert.equal(h.field('comparison-sources')!.value, '');
});

test('multi-station comparison exports the metrics with their conditions', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  h.field('comparison-sources')!.set!('/case/.colm/runs/a | /obs/compare.nc');
  await h.render();
  await h.action('compare-sites');
  await h.action('export-comparison-csv');
  const csv = h.exports.at(-1)!;
  assert.match(csv, /^site,run_case,status,version,observation,/);
  assert.match(csv, /\/case\/\.colm\/runs\/a/);
  assert.match(csv, /f_lfevpa/);
  assert.match(csv, /\/obs\/compare\.nc/);
  // 条件列来自比较时的快照（空窗口、跳过 0 条、未订正），不是导出时界面上的新输入。
  assert.match(csv, /\/obs\/compare\.nc,,,0,0,f_lfevpa/);
});

test('a run from another project joins the comparison without scanning the project', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  const inputs = (node: unknown): { props: Record<string, unknown> }[] => {
    if (Array.isArray(node)) return node.flatMap(inputs);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    return [
      ...(element.type === 'Input' ? [{ props: element.props }] : []),
      ...inputs(element.props?.children),
    ];
  };
  const external = inputs(h.tree()).find((input) => input.props.placeholder === 'Run folder');
  assert.ok(external, 'an external run folder can be entered');
  (external.props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '/othercase/runs/b' },
  });
  await h.render();
  await h.action('comparison-add-external');
  h.field('result-observation')!.set!('/obs/compare.nc');
  await h.render();
  await h.action('comparison-apply-observation');
  const scans = () => h.requests.filter((request) => request.command === 'model_case_runs');
  const before = scans().length;
  await h.action('compare-sites');
  const added = scans().slice(before);
  // 只按运行目录反推的算例查一次，不扫整个项目。
  assert.deepEqual(
    added.map((request) => request.args?.dir),
    ['/othercase'],
  );
  const cells = (node: unknown): string[] => {
    if (Array.isArray(node)) return node.flatMap(cells);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    if (element.type === 'td')
      return [String((element.props.children ?? []).filter((x) => x != null).join(''))];
    return cells(element.props?.children);
  };
  assert.ok(cells(h.tree()).includes('f_lfevpa'), 'the external run produced metrics');
});

test('multi-station run list can be searched and filtered', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  const nodes = (
    node: unknown,
  ): { type: unknown; props: Record<string, unknown> & { children?: unknown[] } }[] => {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    return [element, ...nodes(element.props?.children)];
  };
  const checkboxes = () =>
    nodes(h.tree()).filter(
      (element) =>
        element.type === 'input' &&
        element.props.type === 'checkbox' &&
        String(element.props.onChange).includes('toggleComparisonRun'),
    );
  assert.equal(checkboxes().length, 1);
  const search = nodes(h.tree()).find(
    (element) => element.type === 'Input' && element.props['aria-label'] === 'Search runs',
  );
  assert.ok(search, 'the run list has a search box');
  (search.props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: 'no-such-station' },
  });
  await h.render();
  assert.equal(checkboxes().length, 0, 'searching hides non-matching runs');
  (search.props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '' },
  });
  await h.render();
  assert.equal(checkboxes().length, 1);
  // 结果已在本机的运行在"只看服务器上的"过滤下也隐藏。
  const serverOnly = nodes(h.tree()).find(
    (element) =>
      element.type === 'input' &&
      element.props.type === 'checkbox' &&
      String(element.props.onChange).includes('setComparisonServerOnly'),
  );
  assert.ok(serverOnly, 'the server-only filter exists');
  (serverOnly.props.onChange as (event: { target: { checked: boolean } }) => void)({
    target: { checked: true },
  });
  await h.render();
  assert.equal(checkboxes().length, 0);
});

test('the comparison shows a per-station RMSE bar chart', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  h.field('comparison-sources')!.set!('/case/.colm/runs/a | /obs/compare.nc');
  await h.render();
  await h.action('compare-sites');
  const nodes = (
    node: unknown,
  ): { type: unknown; props: Record<string, unknown> & { children?: unknown[] } }[] => {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    return [element, ...nodes(element.props?.children)];
  };
  const bars = nodes(h.tree()).filter(
    (element) => element.props?.className === 'comparison-bar-fill',
  );
  assert.equal(bars.length, 1, 'one bar per compared run');
  assert.ok(String((bars[0].props.style as { width?: string }).width).endsWith('%'));
});

test('a downsampled stretch with no samples is drawn as a break, not a smooth line', async () => {
  const h = harness(false, undefined, '', '/case', { gaps: true });
  await h.render();
  const chart = h.charts()[0] as { labels: string[]; data: (number | null)[][] } | undefined;
  assert.ok(chart, 'the sampled series is charted');
  // 缺测区间的起止时刻各插一行 null：5 与 6 之间的 21 个缺测点不再被降采样抹掉。
  assert.deepEqual(chart.data, [
    [1, 1.5, 2, 2.5, 3, 4],
    [100, null, 200, null, 300, 400],
  ]);
  assert.deepEqual(chart.labels, ['Simulation']);
});

test('a partial local fetch says what is local and offers fetching the full history', async () => {
  const h = harness(false, undefined, '', '/case', {
    remoteRun: true,
    localResults: {
      status: 'partial',
      files: 3,
      vars: ['Qle', 'Qh'],
      from: '2008-01-01T00:00:00Z',
      to: '2008-06-01T00:00:00Z',
      scientific_verified: false,
    },
  });
  await h.render();
  const rendered = JSON.stringify(h.tree());
  assert.ok(
    rendered.includes('Local results partial'),
    'the identity badge must not claim a complete local result',
  );
  for (const text of [
    'Only part of the results',
    'Qle',
    'Qh',
    '2008-01',
    '2008-06',
    'not scientifically verified',
  ])
    assert.ok(rendered.includes(text), `the banner explains ${text}`);
  const full = actionById(h, 'result-fetch-all');
  assert.equal(full?.kind, 'commit');
  assert.equal(full?.enabled, true);
  assert.ok(actionById(h, 'result-fetch'), 'the per-run fetch button stays');
  const scans = () => h.requests.filter((request) => request.command === 'model_case_runs').length;
  const before = scans();
  // 部分取回只能用「取回全部历史」补全：all/overwriteNewer 都要为 true。
  await h.action('result-fetch-all');
  assert.deepEqual(
    h.requests.filter((request) => request.command === 'remote_fetch').at(-1)?.args,
    {
      case: '/case/.colm/runs/a',
      host: null,
      all: true,
      overwriteNewer: true,
    },
  );
  assert.ok(scans() > before, 'the run list refreshes so local_results can be re-read');
});

test('a complete local fetch offers no fetch buttons, but an invalid folder keeps the banner', async () => {
  const complete = harness(false, undefined, '', '/case', {
    remoteRun: true,
    localResults: { status: 'complete', files: 12 },
  });
  await complete.render();
  assert.equal(actionById(complete, 'result-fetch'), undefined);
  assert.equal(actionById(complete, 'result-fetch-all'), undefined);
  assert.ok(JSON.stringify(complete.tree()).includes('Local results complete'));

  const invalid = harness(false, undefined, '', '/case', {
    remoteRun: true,
    localResults: { status: 'invalid', files: 0 },
  });
  await invalid.render();
  assert.ok(JSON.stringify(invalid.tree()).includes('local result folder is invalid'));
  assert.equal(
    actionById(invalid, 'result-fetch-all'),
    undefined,
    'an invalid folder has nothing to top up',
  );
});

test('a failed comparison entry is listed without losing the successful rows, and retry recomputes only it', async () => {
  const failing = ['/obs/b.nc'];
  const h = harness(true, undefined, '', '/case', { failingObs: failing });
  await h.render();
  const cells = (node: unknown): string[] => {
    if (Array.isArray(node)) return node.flatMap(cells);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    if (element.type === 'td')
      return [String((element.props.children ?? []).filter((x) => x != null).join(''))];
    return cells(element.props?.children);
  };
  h.field('result-scope')!.set!('multi');
  await h.render();
  h.field('comparison-sources')!.set!(
    '/case/.colm/runs/a | /obs/compare.nc\n/othercase/runs/b | /obs/b.nc',
  );
  await h.render();
  await h.action('compare-sites');
  const metricCalls = () =>
    h.requests.filter(
      (request) => request.command === 'metrics' && request.args.obs !== '/obs/single.nc',
    );
  assert.equal(metricCalls().length, 2, 'both entries are attempted');
  assert.ok(!cellError(h), 'one failed station does not turn the whole comparison into an error');
  assert.ok(cells(h.tree()).includes('f_lfevpa'), 'the successful row stays');
  const rendered = JSON.stringify(h.tree());
  assert.ok(rendered.includes('1 run(s) produced no metrics'));
  assert.ok(rendered.includes('/obs/b.nc'));
  assert.ok(rendered.includes('no readable history for this run'));
  assert.equal(actionById(h, 'retry-comparison-failures')?.enabled, true);
  // 重试只重算失败项；成功后合并进同一张表，失败列表消失。
  failing.length = 0;
  const before = metricCalls().length;
  await h.action('retry-comparison-failures');
  const retried = metricCalls().slice(before);
  assert.equal(retried.length, 1, 'only the failed entry is recomputed');
  assert.equal(retried[0].args.obs, '/obs/b.nc');
  assert.ok(!JSON.stringify(h.tree()).includes('produced no metrics'));
  assert.ok(cells(h.tree()).includes('Qle'), 'the retried row is merged into the snapshot');
  assert.ok(!cellError(h), 'a successful retry leaves no error notice');
});

test('the time-series variable picker offers regional variables with readable labels', async () => {
  const h = harness(false, undefined, '', '/case', { gridVariable: true });
  await h.render();
  await h.tab('series');
  const nodes = (
    node: unknown,
  ): { type: unknown; props: Record<string, unknown> & { children?: unknown[] } }[] => {
    if (Array.isArray(node)) return node.flatMap(nodes);
    if (!node || typeof node !== 'object') return [];
    const element = node as {
      type: unknown;
      props: Record<string, unknown> & { children?: unknown[] };
    };
    return [element, ...nodes(element.props?.children)];
  };
  const options = nodes(h.tree()).filter((element) => element.type === 'option');
  assert.ok(options.some((option) => option.props.value === 'f_lfevpa'));
  // 区域（grid）变量在总览下拉能选，时序页原来只筛 kind==='series' 就看不到；现在统一。
  const grid = options.find((option) => option.props.value === 'f_tref');
  assert.ok(grid, 'regional (grid) variables are selectable on the time-series page');
  const label = String((grid.props.children as unknown[]).join(''));
  assert.match(label, /2m air temperature/, 'the unified readable label replaces the bare name');
  assert.match(label, /f_tref/, 'the raw variable name stays visible for NetCDF cross-checking');
  assert.match(label, /K/, 'units stay visible next to the label');
});

test('a failed curve read shows its error and a retry instead of a spinner', async () => {
  const options: { seriesFailure?: string } = { seriesFailure: 'cannot read history' };
  const h = harness(false, undefined, '', '/case', options);
  await h.render();
  const rendered = JSON.stringify(h.tree());
  assert.ok(rendered.includes('cannot read history'), 'the curve area shows the actual error');
  assert.ok(
    !rendered.includes('Reading simulation results'),
    'a failed read is not reported as a permanent spinner',
  );
  const retry = actionById(h, 'result-curve-retry');
  assert.ok(retry, 'the curve area offers a retry');
  assert.equal(retry.enabled, true);
  delete options.seriesFailure;
  await h.action('result-curve-retry');
  assert.ok(!JSON.stringify(h.tree()).includes('cannot read history'));
  assert.ok(h.charts().length, 'the retried read draws the curve');
});

test('the curve area separates a pending read from a window with no data', async () => {
  let finish!: (value: unknown) => void;
  const pending = new Promise((resolve) => {
    finish = resolve;
  });
  const h = harness(false, pending);
  await h.render();
  assert.ok(
    JSON.stringify(h.tree()).includes('Reading simulation results'),
    'the read is still in flight',
  );
  finish(normal);
  await h.render();
  assert.ok(h.charts().length, 'the finished read draws the curve');

  const empty = harness(false, undefined, '', '/case', { seriesEmpty: true });
  await empty.render();
  const text = JSON.stringify(empty.tree());
  assert.ok(
    text.includes('No curve data in the selected window'),
    'an empty window says so instead of pretending to load',
  );
  assert.equal(empty.charts().length, 0);
});

test('a failed run list shows the error and retries instead of claiming there are no runs', async () => {
  const options: { runsFailure?: string } = { runsFailure: 'ledger is locked' };
  const h = harness(false, undefined, '', '/case', options);
  await h.render();
  const rendered = JSON.stringify(h.tree());
  assert.ok(rendered.includes('ledger is locked'), 'the actual failure is shown');
  assert.ok(!rendered.includes('No recorded run'), 'a read failure is not "there are no runs"');
  const retry = actionById(h, 'result-runs-retry');
  assert.ok(retry, 'the run list offers a retry');
  assert.equal(retry.enabled, true);
  delete options.runsFailure;
  await h.action('result-runs-retry');
  const after = JSON.stringify(h.tree());
  assert.ok(!after.includes('ledger is locked'));
  assert.ok(!after.includes('No recorded run'));
  assert.ok(h.charts().length, 'the retried run list leads to a chart');
});

test('a condition change keeps the previous metrics visible as pending update', async () => {
  const h = harness(true);
  await h.render();
  await h.tab('evaluation');
  h.field('result-observation')!.set!('/obs/single.nc');
  await h.render();
  await h.action('evaluate-observations');
  const before = JSON.stringify(h.tree());
  assert.ok(before.includes('Qle'), 'the evaluation row is rendered');
  assert.ok(!before.includes('pending update'));
  // 改评估窗口：旧表保留并标成「待更新」，而不是变成"还没有结果"的空态。
  h.field('result-from')!.set!('2008-01-01T00:00');
  await h.render();
  const after = JSON.stringify(h.tree());
  assert.ok(after.includes('pending update'), 'the table says its conditions changed');
  assert.ok(after.includes('Qle'), 'the previous rows stay visible with the pending note');
});

test('a comparison condition change marks the previous table as pending update', async () => {
  const h = harness(true);
  await h.render();
  h.field('result-scope')!.set!('multi');
  await h.render();
  h.field('comparison-sources')!.set!('/case/.colm/runs/a | /obs/compare.nc');
  await h.render();
  await h.action('compare-sites');
  assert.ok(!JSON.stringify(h.tree()).includes('pending update'));
  h.field('result-from')!.set!('2008-01-01T00:00');
  await h.render();
  const after = JSON.stringify(h.tree());
  assert.ok(after.includes('pending update'));
  assert.ok(after.includes('f_lfevpa'), 'the previous comparison rows stay visible');
});
