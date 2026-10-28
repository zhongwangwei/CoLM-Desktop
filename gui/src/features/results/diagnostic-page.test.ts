import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import * as seriesHelpers from './series';
import * as overviewHelpers from './result-overview';
import * as variableLabels from './variable-labels';
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
  options: { remoteRun?: boolean; withBack?: boolean } = {},
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
    if (command === 'model_case_runs' && args.dir === '/empty') return [];
    if (command === 'model_case_runs')
      return [
        // The ledger's imported placeholder comes first and has no output of its own.
        { run_case: '/case', version: null, legacy: true, status: 'needs_reconciliation' },
        {
          run_case: '/case/.colm/runs/a',
          version: { id: 'official', revision: 2 },
          status: 'succeeded',
          ...(options.remoteRun ? { server: 'server', outputs_local: false } : {}),
        },
      ];
    if (command === 'history_catalog')
      return {
        files: 1,
        steps: 2,
        start: 1,
        end: 2,
        variables: [{ name: 'f_lfevpa', kind: 'series', units: 'W/m²', dimensions: [] }],
      };
    if (command === 'series') return delayedSeries ?? normal;
    if (command === 'energy_summary') return diagnostic;
    throw Error(command);
  };
  const env: Record<string, unknown> = {
    ...seriesHelpers,
    ...overviewHelpers,
    ...variableLabels,
    GridMap: () => {},
    invokeJson,
    invoke: invokeJson,
    hasBackend: () => true,
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
  assert.deepEqual(actionIds(h), ['compare-sites', 'result-overview', 'result-runs']);
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
