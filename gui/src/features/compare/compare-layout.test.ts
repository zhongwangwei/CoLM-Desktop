import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as compare from './compare';
import * as compareConfig from './config';
import type { CompareSnapshot } from './config';
import {
  benchErrorText,
  buildOpenbenchConfig,
  comparisonName,
  designFingerprint,
  evaluationSummary,
  type ComparisonDesign,
  type HistoryInfo,
} from './compare';

/**
 * U-04/U-05 的页面证据。两类用例都跑真实页面代码：
 * 1. 用 TypeScript 把 ComparePage.tsx 的 JSX 编译后整页渲染（受控 hooks/组件/后端），检查三块层级、
 *    折叠的安装区、"编辑模拟信息"入口、SaveBadge 三态和运行前条件摘要；
 * 2. 用 AST 抽出页面真实函数（comparisonDesign / conditionSummary / generate / run / task），在受控
 *    上下文里证明"条件摘要 / 配置 / 站点清单 / 实际运行提交"来自同一份设计。
 * 不修改上一轮 GUI-04/05/07/08 的保护：失效指纹、过期禁运行、请求代次、作业身份都继续生效。
 */
const pagePath = new URL('./ComparePage.tsx', import.meta.url);
const pageSource = readFileSync(pagePath, 'utf8');
const source = ts.createSourceFile(
  pagePath.pathname,
  pageSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);

function declaration(name: string): string {
  let found = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableStatement(node))
      for (const declarationNode of node.declarationList.declarations)
        if (ts.isIdentifier(declarationNode.name) && declarationNode.name.text === name)
          found = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(found, `ComparePage.tsx defines ${name}`);
  return found;
}

function execute<T>(name: string, context: Record<string, unknown>): T {
  const defaults = {
    hostEpoch: { current: 0 },
    stillCurrent: () => true,
    setNotice: () => {},
    setConfigSaving: () => {},
    setWrittenFingerprint: () => {},
    benchTasks: { begin: () => '', update: () => {}, finish: () => {}, clear: () => {} },
    benchTaskId: { current: '' },
    LEFT: 'compare page left',
    store: () => {},
    compareDraftKey: (host: string) => `colm.compareDraft@${host}`,
    showRunNotice: () => {},
  };
  const withDefaults = { ...defaults, ...context };
  const code = ts.transpileModule(`${declaration(name)}; return ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(withDefaults), code)(...Object.values(withDefaults)) as T;
}

// ---- 整页渲染：受控 hooks / 组件树 ----

type Node = { type: unknown; props: Record<string, unknown>; children: unknown[] };

function walk(node: unknown): Node[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(walk);
  const element = node as Node;
  return [element, ...walk(element.children)];
}

function text(node: unknown): string {
  if (Array.isArray(node)) return node.map(text).join('');
  if (node == null || node === false) return '';
  if (typeof node === 'object') {
    const element = node as Node;
    // ActionButton 没写 children 时把 label 显示出来（真实组件 children ?? label）。
    const children = text(element.children);
    return children || (typeof element.props.label === 'string' ? element.props.label : '');
  }
  return String(node);
}

const probedGrid: HistoryInfo = {
  history_dir: '/p/hb/out/hb/history',
  prefix: 'hb_hist_',
  tim_res: 'Day',
  syear: 2002,
  eyear: 2005,
  spatial: true,
  lon: null,
  lat: null,
  files: 48,
  extent: [111, 115.5, 21.5, 24.5],
  resolution: 0.008333,
  data_groupby: 'Month',
};

const readyStatus = {
  root: '/ob',
  uv: '/ob/tools/uv',
  uv_own: true,
  uv_version: '0.8.0',
  uv_pinned: '0.8.0',
  openbench_pinned: '3.0.8',
  python: '/ob/python/bin/python',
  openbench: '3.0.8',
  projects: [] as { name: string; report: string | null }[],
};
const referenceList = [
  { name: 'ERA5LAND_LowRes', data_type: 'grid', tim_res: 'Day', variables: ['Latent_Heat'] },
];
const metricOptions = {
  metrics: [
    { name: 'bias', label: 'Bias' },
    { name: 'RMSE', label: 'RMSE' },
    { name: 'correlation', label: 'Correlation' },
    { name: 'KGE', label: 'KGE' },
  ],
  scores: [{ name: 'Overall_Score', label: 'Overall Score' }],
};

type Contract = {
  fields: { id: string; label: string; value: string; set: (value: string) => void }[];
  details?: Record<string, unknown>;
};

function pageHarness(
  options: {
    seed?: Record<string, string>;
    remoteStatus?: Record<string, unknown>;
    localStatus?: Record<string, unknown>;
  } = {},
) {
  const seed: Record<string, string> = {
    'colm.openbenchRefRoot': '/refs',
    ...options.seed,
  };
  // 页面会往 localStorage 写草稿/机器选择：这里保留一份内存副本，测试才能断言"写了什么"而不是只看内存状态。
  const disk = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => disk.get(key) ?? seed[key] ?? null,
      setItem: (key: string, value: string) => {
        disk.set(key, value);
      },
      removeItem: (key: string) => {
        disk.delete(key);
      },
    },
  });
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  let pendingRun: { promise: Promise<unknown>; resolve: (value: unknown) => void } | null = null;
  let confirmAnswer = false;
  let firstReferenceFailure = false;
  const slots: unknown[] = [];
  let cursor = 0;
  let effectCursor = 0;
  let effects: (() => void)[] = [];
  const registered = new Map<number, { deps?: unknown[]; cleanup?: () => void }>();
  let tree: unknown;
  let contract: Contract;

  const invokeJson = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    switch (command) {
      case 'remote_config':
        return { servers: [] };
      case 'bench_status':
        return options.localStatus ?? readyStatus;
      case 'bench_remote_status':
        return options.remoteStatus ?? readyStatus;
      case 'bench_remote_references':
        return referenceList;
      case 'bench_remote_metric_options':
        return metricOptions;
      case 'bench_remote_inspect':
        return probedGrid;
      case 'bench_references':
        if (firstReferenceFailure) {
          firstReferenceFailure = false;
          throw new Error('参考数据列表读不懂：boom');
        }
        return referenceList;
      case 'bench_metric_options':
        return metricOptions;
      case 'pick_folder':
        return probedGrid.history_dir;
      case 'bench_inspect':
        return probedGrid;
      case 'bench_run':
        return pendingRun ? pendingRun.promise : { report: null };
      default:
        return null;
    }
  };
  const invoke = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'pick_folder') return probedGrid.history_dir;
    if (command === 'open_path') return null;
    return null;
  };
  const listen = async () => () => {};
  const askConfirm = async () => confirmAnswer;

  const useState = (initial: unknown) => {
    const key = cursor++;
    if (!(key in slots))
      slots[key] = typeof initial === 'function' ? (initial as () => unknown)() : initial;
    const set = (value: unknown) => {
      slots[key] =
        typeof value === 'function' ? (value as (old: unknown) => unknown)(slots[key]) : value;
    };
    return [slots[key], set] as const;
  };
  const useRef = (initial: unknown) => {
    const key = cursor++;
    if (!(key in slots)) slots[key] = { current: initial };
    return slots[key] as { current: unknown };
  };
  // 页面用 useMemo 缓存"从配置文本解析出的有效配置"；受控渲染每次重算即可。
  const useMemo = (fn: () => unknown) => fn();
  const useEffect = (fn: () => void | (() => void), deps?: unknown[]) => {
    const key = effectCursor++;
    const previous = registered.get(key);
    if (
      previous &&
      deps &&
      previous.deps &&
      deps.length === previous.deps.length &&
      deps.every((value, index) => Object.is(value, previous.deps?.[index]))
    )
      return;
    const entry = { deps, cleanup: undefined as (() => void) | undefined };
    registered.set(key, entry);
    effects.push(() => {
      previous?.cleanup?.();
      const cleanup = fn();
      if (typeof cleanup === 'function') entry.cleanup = cleanup;
    });
  };
  const React = {
    createElement: (
      type: unknown,
      props: Record<string, unknown> | null,
      ...children: unknown[]
    ) => ({
      type,
      props: props ?? {},
      children,
    }),
    Fragment: 'Fragment',
  };
  const useApp = () => ({ state: { language: 'en', cases: [] } });
  // 跨路由任务条在整页 harness 里记录调用即可；reducer/provider 的测试在 bench-task.test.ts。
  const benchCalls: string[] = [];
  const useBenchTasks = () => ({
    tasks: [],
    begin: (input: { project: string }) => {
      benchCalls.push(`begin:${input.project}`);
      return 'task-1';
    },
    update: (_id: string, patch: { phase?: [string, string] }) => {
      benchCalls.push(`update:${patch.phase?.[0] ?? ''}`);
    },
    finish: (_id: string, outcome: { state: string }) => {
      benchCalls.push(`finish:${outcome.state}`);
    },
    clear: () => {},
  });
  const usePageContract = (value: Contract) => {
    contract = value;
  };
  const environment: Record<string, unknown> = {
    ...compare,
    ...compareConfig,
    React,
    useApp,
    useBenchTasks,
    usePageContract,
    useState,
    useRef,
    useEffect,
    useMemo,
    Advanced: 'Advanced',
    Button: 'Button',
    Card: 'Card',
    Dialog: 'Dialog',
    Field: 'Field',
    Input: 'Input',
    Notice: 'Notice',
    Select: 'Select',
    ActionButton: 'ActionButton',
    SaveBadge: 'SaveBadge',
    RemotePathBrowser: 'RemotePathBrowser',
    invoke,
    invokeJson,
    listen,
    askConfirm,
  };
  const stripped = pageSource
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(stripped, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(environment), `${code}; return ComparePage;`)(
    ...Object.values(environment),
  ) as () => unknown;

  function render() {
    cursor = 0;
    effectCursor = 0;
    tree = Component();
    return tree;
  }
  async function settle() {
    for (let round = 0; round < 40; round++) {
      const pending = effects;
      effects = [];
      pending.forEach((effect) => effect());
      for (let turn = 0; turn < 25; turn++) await Promise.resolve();
      render();
      if (!effects.length) return;
    }
    throw new Error('page effects did not stabilize');
  }
  async function mount() {
    render();
    await settle();
  }
  const nodes = () => walk(tree);
  const actions = () =>
    nodes()
      .filter((node) => node.type === 'ActionButton')
      .map((node) => ({
        id: String(node.props.action),
        label: String(node.props.label),
        enabled: !node.props.disabled,
        run: node.props.run as () => unknown,
      }));
  const section = (name: string) =>
    nodes().find((node) => node.props['data-compare-section'] === name);
  const cardTitle = (sectionName: string) => {
    const found = section(sectionName);
    assert.ok(found, `section ${sectionName}`);
    const card = walk(found).find((node) => node.type === 'Card');
    assert.ok(card, `Card in ${sectionName}`);
    return text(card.props.title);
  };
  async function press(actionId: string) {
    const action = actions().find((item) => item.id === actionId);
    assert.ok(action, `action ${actionId}`);
    assert.notEqual(action.enabled, false, `${actionId} is enabled`);
    const result = action.run();
    if (result instanceof Promise) await result;
    await settle();
    return result;
  }
  async function pressLabel(label: string) {
    const button = nodes().find((node) => node.type === 'Button' && text(node) === label);
    assert.ok(button, `button ${label}`);
    (button.props.onClick as () => void)();
    await settle();
  }
  async function setField(id: string, value: string) {
    const field = contract.fields.find((item) => item.id === id);
    assert.ok(field, `field ${id}`);
    field.set(value);
    await settle();
  }
  const configTextarea = () =>
    nodes().find(
      (node) => node.type === 'textarea' && node.props['data-field'] === 'openbench-config',
    );
  const badgeState = () => {
    const badge = nodes().find((node) => node.type === 'SaveBadge');
    return badge ? String(badge.props.state) : null;
  };
  const summaryText = () => {
    const list = nodes().find((node) => node.props['data-compare-summary'] === true);
    return list ? text(list) : '';
  };
  const addVariable = async () => {
    const picker = nodes().find(
      (node) =>
        node.type === 'Select' &&
        walk(node.children).some((option) => option.props.value === 'Latent_Heat'),
    );
    assert.ok(picker, 'variable picker');
    (picker.props.onChange as (event: unknown) => void)({ target: { value: 'Latent_Heat' } });
    await settle();
  };
  const deferRun = () => {
    let resolve!: (value: unknown) => void;
    const promise = new Promise<unknown>((done) => {
      resolve = done;
    });
    pendingRun = { promise, resolve };
    return (value: unknown) => {
      pendingRun = null;
      resolve(value);
    };
  };
  return {
    mount,
    settle,
    render,
    nodes,
    actions,
    section,
    cardTitle,
    press,
    pressLabel,
    setField,
    configTextarea,
    badgeState,
    localValue: (key: string) => disk.get(key) ?? '',
    benchCalls: () => [...benchCalls],
    summaryText,
    addVariable,
    deferRun,
    setConfirm: (value: boolean) => {
      confirmAnswer = value;
    },
    calls,
    contract: () => contract,
    setFirstReferenceFailure: () => {
      firstReferenceFailure = true;
    },
  };
}

test('U-04: the comparison page opens as environment → this evaluation → jobs and reports', async () => {
  const page = pageHarness();
  await page.mount();
  const sections = page
    .nodes()
    .filter((node) => typeof node.props['data-compare-section'] === 'string')
    .map((node) => String(node.props['data-compare-section']));
  assert.deepEqual(sections, ['environment', 'evaluation', 'jobs']);
  assert.equal(page.cardTitle('environment'), 'Environment status');
  assert.equal(page.cardTitle('evaluation'), 'This evaluation');
  assert.equal(page.cardTitle('jobs'), 'Jobs and reports');

  // 环境块只显示机器与是否就绪；安装/重装/维护在折叠区（Advanced）里。
  const environment = page.section('environment')!;
  assert.match(text(environment), /OpenBench is ready \(3\.0\.8\)/);
  const folded = walk(environment).find((node) => node.type === 'Advanced');
  assert.ok(folded, 'install/maintenance fold');
  assert.equal(text(folded.props.title), 'Install, reinstall and maintenance');
  const foldedActions = walk(folded)
    .filter((node) => node.type === 'ActionButton')
    .map((node) => String(node.props.action));
  for (const action of ['compare-install-all', 'compare-reinstall-openbench'])
    assert.ok(foldedActions.includes(action), `${action} is folded`);
  // 折叠区之外没有安装动作（否则就不算"折叠"）。
  const environmentActions = walk(environment)
    .filter((node) => !walk(folded).includes(node) && node.type === 'ActionButton')
    .map((node) => String(node.props.action));
  assert.deepEqual(environmentActions, []);

  // 评估块包含模拟、参考数据、变量、指标和运行前条件摘要。
  const evaluation = page.section('evaluation')!;
  for (const heading of [
    'Simulations',
    'Reference data',
    'Variables and references',
    'Pre-run condition summary',
    'Evaluation configuration',
  ])
    assert.ok(
      page.nodes().some((node) => text(node) === heading),
      `heading ${heading}`,
    );
  assert.ok(
    walk(evaluation).some(
      (node) => node.type === 'ActionButton' && node.props.action === 'compare-generate',
    ),
    'the evaluation block owns the save-configuration action',
  );
  // 任务与报告块包含报告列表和载入入口（功能仍在，不删）。
  const jobs = page.section('jobs')!;
  assert.match(text(jobs), /Reports/);
  assert.match(text(jobs), /Load a report…/);
});

test('U-04: the per-simulation entry is named "Edit simulation information" and the dialog agrees', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  const pageText = () => text(page.nodes());
  assert.match(pageText(), /Edit simulation information/);
  assert.doesNotMatch(pageText(), /Variables…/);
  // 打开编辑弹窗：标题与入口同名，说明入口确实能改类型/时间/前缀（不只是变量）。
  await page.pressLabel('Edit simulation information');
  const dialog = page.nodes().find((node) => node.type === 'Dialog' && node.props.open === true);
  assert.ok(dialog, 'simulation dialog opens');
  assert.equal(text(dialog.props.title), 'Edit simulation information');
  const entry = page
    .nodes()
    .find(
      (node) =>
        node.type === 'Button' &&
        text(node) === 'Edit simulation information' &&
        typeof node.props.title === 'string',
    );
  assert.ok(entry, 'the entry explains it edits folder/kind/time/prefix/grouping');
  assert.match(String(entry.props.title), /folder, kind, time, prefix and grouping/);
});

test('U-04/U-05: summary, saved configuration and the running task all come from one design', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  // 还没生成配置：徽标说的是"有未生成的修改"，不是"未保存"（配置还没写盘这件事要分开说）。
  assert.equal(page.badgeState(), 'dirty');
  const badgeLabels = () =>
    page.nodes().find((node) => node.type === 'SaveBadge')?.props.labels as
      Record<string, string> | undefined;
  assert.match(badgeLabels()?.dirty ?? '', /Changes not generated/);
  assert.equal(
    page.actions().find((action) => action.id === 'compare-generate')?.label,
    'Generate configuration',
  );

  await page.press('compare-generate');
  assert.equal(page.badgeState(), 'saved');
  // 生成只写页面文本；运行前不能说"已写盘"。
  assert.match(badgeLabels()?.saved ?? '', /Generated \(not on disk\)/);
  const textarea = page.configTextarea();
  assert.ok(textarea, 'configuration text after saving');
  const config = String(textarea.props.value);
  const summary = page.summaryText();
  // 摘要里的机器/路径/模拟/变量/指标都能在配置文本里找到对应内容 —— 同一份 design。
  for (const part of [
    '/p/hb/out/hb/history',
    'hb_hist_',
    '2002–2005',
    'target 0.25°',
    'Latent_Heat: ERA5LAND_LowRes',
  ])
    assert.ok(summary.includes(part), `${part} in summary ${summary}`);
  for (const part of [
    '/p/hb/out/hb/history',
    'hb_hist_',
    'grid_res: 0.25',
    'Latent_Heat: "ERA5LAND_LowRes"',
    'tim_res: Day',
  ])
    assert.ok(config.includes(part), `${part} in config`);
  assert.match(summary, /Comparison project folder.*\/ob\/projects\/comparison/);
  assert.match(config, /name: "comparison"/);

  // 运行：提交期间 SaveBadge 显示"正在保存"，结束后回到"已保存"，成功提示只属于这一版设计。
  const release = page.deferRun();
  const runPromise = page
    .actions()
    .find((action) => action.id === 'compare-run')!
    .run();
  await Promise.resolve();
  page.render();
  assert.equal(page.badgeState(), 'saving');
  release({ report: null });
  await runPromise;
  await page.settle();
  assert.equal(page.badgeState(), 'saved');
  assert.match(text(page.nodes()), /Evaluation finished/);
  // 页面把这次 run 登记进跨路由任务条，并在成功时收尾（UX ③ 的页面侧接线）。
  assert.ok(
    page.benchCalls().some((call) => call.startsWith('begin:comparison')),
    'run is mirrored',
  );
  assert.ok(page.benchCalls().includes('finish:done'), 'successful run finishes the mirror');
  // 运行提交成功后这一版配置已经写进比较项目目录，徽标才显示"已写盘"。
  assert.match(badgeLabels()?.saved ?? '', /Written to the project folder/);
  const runCall = page.calls.find((call) => call.command === 'bench_run');
  assert.ok(runCall);
  assert.equal(runCall.args?.config, config, 'running submits exactly the saved configuration');
  // 改设置：配置作废，过时的成功提示清掉（U-05）。
  await page.setField('compare-name', 'another comparison');
  assert.equal(page.badgeState(), 'dirty');
  assert.equal(page.configTextarea(), undefined);
  assert.doesNotMatch(text(page.nodes()), /Evaluation finished/);
});

test('U-05: a run that finishes after the settings changed does not claim the new settings are done', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  await page.press('compare-generate');
  const release = page.deferRun();
  const runPromise = page
    .actions()
    .find((action) => action.id === 'compare-run')!
    .run();
  await Promise.resolve();
  page.render();
  assert.equal(page.badgeState(), 'saving');
  // 运行期间改设置：这次运行完成的是旧设计，旧成功提示不能挂到新设置上。
  await page.setField('compare-name', 'changed while running');
  release({ report: null });
  await runPromise;
  await page.settle();
  assert.doesNotMatch(text(page.nodes()), /Evaluation finished/);
  assert.equal(page.badgeState(), 'dirty', 'the new settings need their own configuration');
});

test('U-05/R01: hand-edited configuration survives but expires once the page settings change', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  await page.press('compare-generate');
  const textarea = page.configTextarea()!;
  const generated = String(textarea.props.value);
  (textarea.props.onChange as (event: unknown) => void)({
    target: { value: `${generated}# mine` },
  });
  await page.settle();
  await page.setField('compare-name', 'another comparison');
  // 手改过的配置不被背景清掉；但表单改动后冻结的站点清单/模拟输入与这份配置不再来自同一次生成，
  // 快照过期：页面明确提示并禁止运行，不能"摘要读表单、提交读旧站点"（R01；原 U-05 的"仍可运行"被收紧）。
  const kept = page.configTextarea();
  assert.ok(kept, 'the edited configuration is kept');
  assert.match(String(kept.props.value), /# mine/);
  assert.match(text(page.nodes()), /You changed the evaluation settings after generating/);
  assert.equal(
    page.actions().find((action) => action.id === 'compare-run')?.enabled,
    false,
    'a configuration generated before the settings changed cannot run',
  );
  assert.equal(
    (page.contract().details?.config as { stale?: boolean } | undefined)?.stale,
    true,
    'the page reports the snapshot as expired, not merely edited',
  );
  assert.match(page.summaryText(), /Expired/, 'the summary starts with the expiry note');
  // 不确认就不覆盖手改内容；确认后才重新生成。
  page.setConfirm(false);
  await page.pressLabel('Regenerate evaluation configuration');
  assert.match(String(page.configTextarea()!.props.value), /# mine/);
  page.setConfirm(true);
  await page.pressLabel('Regenerate evaluation configuration');
  const regenerated = String(page.configTextarea()!.props.value);
  assert.doesNotMatch(regenerated, /# mine/);
  assert.match(regenerated, /name: "another_comparison"/);
  assert.equal(page.badgeState(), 'saved');
});

/**
 * 打开"编辑模拟信息"弹窗，按 `data-sim-field` 改字段后保存：走页面真实的"编辑站点目录/坐标"路径，
 * 不是直接改内部 picks（R01 的复现链必须经过用户实际操作）。
 */
async function editSimulation(
  page: ReturnType<typeof pageHarness>,
  values: Record<string, string>,
) {
  await page.pressLabel('Edit simulation information');
  for (const [field, value] of Object.entries(values)) {
    const control = page
      .nodes()
      .find(
        (node) =>
          (node.type === 'Input' || node.type === 'Select') &&
          node.props['data-sim-field'] === field,
      );
    assert.ok(control, `dialog field ${field}`);
    (control.props.onChange as (event: unknown) => void)({ target: { value } });
    await page.settle();
  }
  await page.pressLabel('Save simulation');
}

/** 把一个格点模拟改成站点并填上页面确认的坐标：这样生成配置时会带上要传给后端的站点清单。 */
async function makeStationSimulation(
  page: ReturnType<typeof pageHarness>,
  lon: string,
  lat: string,
) {
  await editSimulation(page, { dataType: 'stn', lon, lat });
}

test('R01: hand-edited text alone still runs with the current parsed config and the frozen station list', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  await makeStationSimulation(page, '11.3', '47.1');
  await page.press('compare-generate');
  const original = String(page.configTextarea()!.props.value);
  assert.match(original, /data_type: stn/);
  const fingerprintText = () =>
    text(page.nodes().find((node) => node.props['data-compare-effective'] === true) ?? '');
  const before = fingerprintText();
  assert.match(before, /Effective input fingerprint [0-9a-f]{8}/);

  // 只改文本（表单不动）：快照仍有效，运行按"当前文本解析出的配置 + 冻结站点清单"提交（U-05 意图保留）。
  const edited = original
    .replace('years: [2002, 2005]', 'years: [2003, 2004]')
    .replace('tim_res: Day', 'tim_res: Month');
  assert.notEqual(edited, original);
  (page.configTextarea()!.props.onChange as (event: unknown) => void)({
    target: { value: edited },
  });
  await page.settle();
  assert.equal(
    page.actions().find((action) => action.id === 'compare-run')?.enabled,
    true,
    'a text-only edit is still runnable',
  );
  assert.equal(
    (page.contract().details?.config as { stale?: boolean } | undefined)?.stale,
    false,
    'a text-only edit does not expire the snapshot',
  );
  // 摘要 = 当前配置（年份/时间尺度）+ 冻结站点清单（history 目录/坐标），与实际提交一致。
  const summary = page.summaryText();
  assert.match(summary, /2003–2004/);
  assert.match(summary, /Month/);
  assert.match(summary, /\/p\/hb\/out\/hb\/history/);
  assert.match(summary, /47\.1°, 11\.3°/);
  assert.notEqual(fingerprintText(), before, 'the fingerprint follows the edited configuration');

  // 显示的指纹必须就是"当前文本解析出的配置 + 冻结站点清单"一起算出的那个，不能另算一套。
  const stationSite = {
    id: 'hb',
    station: 'hb',
    history_dir: probedGrid.history_dir,
    prefix: probedGrid.prefix,
    lon: 11.3,
    lat: 47.1,
  };
  const expectedFingerprint = compareConfig.configFingerprint(
    {
      machine: { host: '', openbenchRoot: '/ob', refRoot: '/refs' },
      name: 'comparison',
      years: [2002, 2005],
      timRes: 'Day',
      variables: {},
      simulations: [],
      projectDir: '/ob/projects/comparison',
    },
    compareConfig.parseOpenbenchConfig(edited).config!,
    [stationSite],
  );
  assert.match(
    fingerprintText(),
    new RegExp(`Effective input fingerprint ${expectedFingerprint}`),
    'the shown fingerprint covers exactly the submitted config and frozen sites',
  );

  await page.press('compare-run');
  const sent = page.calls.find((call) => call.command === 'bench_run');
  assert.ok(sent, 'the run reaches the backend');
  assert.equal(sent.args?.config, edited, 'the edited text is what gets submitted');
  assert.deepEqual(sent.args?.sites, [stationSite]);
});

test('R01: f0059ee chain — editing the station coordinates after generating expires the snapshot and blocks the run', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  await makeStationSimulation(page, '11.3', '47.1');
  await page.press('compare-generate');
  const generatedText = String(page.configTextarea()!.props.value);

  // 1) 手改 YAML 年份/时间尺度（表单没变）：仍可运行。
  const edited = generatedText
    .replace('years: [2002, 2005]', 'years: [2004, 2004]')
    .replace('tim_res: Day', 'tim_res: Month');
  (page.configTextarea()!.props.onChange as (event: unknown) => void)({
    target: { value: edited },
  });
  await page.settle();
  assert.equal(page.actions().find((action) => action.id === 'compare-run')?.enabled, true);

  // 2) 再编辑站点坐标：冻结的站点清单与这份配置不再是同一次生成 → 快照过期。
  await editSimulation(page, { lon: '12.5', lat: '48.5' });
  assert.equal(
    page.actions().find((action) => action.id === 'compare-run')?.enabled,
    false,
    'the stale snapshot blocks the run button',
  );
  assert.equal((page.contract().details?.config as { stale?: boolean } | undefined)?.stale, true);
  const pageText = text(page.nodes());
  assert.match(pageText, /You changed the evaluation settings after generating/);
  assert.match(pageText, /Regenerate the evaluation configuration/);
  assert.match(page.summaryText(), /Expired/, 'the summary says the snapshot expired');

  // 3) 绕过界面直接调用 run：内部也必须拒绝，且一个后端调用都不发。
  const run = page.actions().find((action) => action.id === 'compare-run')!
    .run as () => Promise<unknown>;
  await run();
  await page.settle();
  assert.deepEqual(
    page.calls.filter((call) => call.command === 'bench_run'),
    [],
    'a stale snapshot never reaches the backend',
  );
  assert.match(
    text(page.nodes()),
    /settings changed after the configuration was generated/,
    'the page explains why the run was refused',
  );
});

test('U-04: summary, frozen station list and the run submission all come from one snapshot', async () => {
  const t = (_zh: string, english: string) => english;
  const probed: HistoryInfo = {
    ...probedGrid,
    spatial: false,
    extent: null,
    lon: 11.3,
    lat: 47.1,
    prefix: 'AT-Neu_hist_',
    history_dir: '/p/AT-Neu/history',
    files: 48,
  };
  const picks = [{ dir: '/p/AT-Neu/history', label: 'AT-Neu', info: probed }];
  const comparisonDesign = execute<() => ComparisonDesign>('comparisonDesign', {
    name: 'versions vs flux',
    from: '2003',
    to: '2005',
    timRes: 'Day',
    chosen: { Latent_Heat: ['ERA5LAND_LowRes'] },
    ok: picks,
    status: { root: '/ob' },
    gridRes: '0.25',
    weight: '',
    minYear: '1',
    timeAlignment: '',
    numCores: '',
    metrics: ['bias'],
    scores: ['Overall_Score'],
    comparisonItems: [],
    statisticsItems: [],
    comparisonDir: compare.comparisonDir,
  });
  const machine = { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' };
  const design = comparisonDesign();
  const machineDesign = { machine, ...design };
  const byLabelOf = (summary: ReturnType<typeof evaluationSummary>, label: string) =>
    summary.find((item) => item.label[1] === label)?.value;
  // 还没生成配置：摘要退回表单设计（旧路径）。
  const formSummary = execute<() => ReturnType<typeof evaluationSummary>>('conditionSummary', {
    generatedConfig: null,
    staleSnapshot: false,
    effectiveConfig: null,
    formDesign: machineDesign,
    effectiveSummary: compareConfig.effectiveSummary,
    evaluationSummary,
    t,
  });
  assert.deepEqual(formSummary(), evaluationSummary(machineDesign));
  assert.equal(byLabelOf(formSummary(), 'Machine'), 'server serverA');
  assert.equal(byLabelOf(formSummary(), 'Machine'), 'server serverA');
  assert.match(byLabelOf(formSummary(), 'Simulation 1') ?? '', /AT-Neu_hist_/);
  assert.match(byLabelOf(formSummary(), 'Simulation 1') ?? '', /47\.1°, 11\.3°/);

  // 用同一份 design 生成快照；摘要、指纹和 run 提交都读它。
  let generatedValue: CompareSnapshot | null = null;
  const generated = {
    get current() {
      return generatedValue;
    },
    set current(value: CompareSnapshot | null) {
      generatedValue = value;
    },
  };
  const yamlRef: { current: string } = { current: '' };
  const errors: string[] = [];
  const generate = execute<() => void>('generate', {
    comparisonDesign,
    buildOpenbenchConfig,
    machine,
    freezeCompareSnapshot: compareConfig.freezeCompareSnapshot,
    generated,
    setYaml: (value: string) => {
      yamlRef.current = value;
    },
    setMissingExtent: () => {},
    setError: (value: string) => errors.push(value),
    shown: (error: unknown) => String(error),
  });
  generate();
  assert.ok(
    errors.every((message) => !message),
    'generating a valid design reports no error',
  );
  const built = buildOpenbenchConfig(design);
  assert.equal(yamlRef.current, built.config);
  assert.equal(generatedValue!.generatedText, built.config);
  assert.deepEqual(generatedValue!.sites, built.sites);
  assert.equal(generatedValue!.designFingerprint, designFingerprint(machineDesign));
  assert.equal(
    generatedValue!.fingerprint,
    compareConfig.configFingerprint(
      machineDesign,
      compareConfig.parseOpenbenchConfig(built.config).config!,
      built.sites,
    ),
  );

  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const notices: string[] = [];
  const parsed = compareConfig.parseOpenbenchConfig(yamlRef.current);
  assert.deepEqual(parsed.problems, []);
  const run = execute<() => Promise<unknown>>('run', {
    task: async (fn: () => Promise<unknown>) => fn(),
    generated,
    formDesign: machineDesign,
    designFingerprint,
    snapshotStale: compareConfig.snapshotStale,
    name: 'versions vs flux',
    comparisonName,
    remote: false,
    parseOpenbenchConfig: compareConfig.parseOpenbenchConfig,
    structuralProblems: compareConfig.structuralProblems,
    configFingerprint: compareConfig.configFingerprint,
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return { report: null };
    },
    yaml: yamlRef.current,
    sites: [],
    scanDir: '/refs',
    host: '',
    view: async () => {},
    watch: async () => ({}),
    fetchReport: async () => {},
    setConfigSaving: () => {},
    setLastRun: () => {},
    showRunNotice: (value: string) => notices.push(value),
    t,
  });
  // 摘要 = 冻结站点清单（站点部分）+ 当前配置文本解析（其余条件）。
  const effectiveSummary = execute<() => ReturnType<typeof evaluationSummary>>('conditionSummary', {
    generatedConfig: generatedValue,
    staleSnapshot: false,
    effectiveConfig: parsed.config,
    formDesign: machineDesign,
    effectiveSummary: compareConfig.effectiveSummary,
    evaluationSummary,
    t,
  });
  const byLabel = (label: string) => byLabelOf(effectiveSummary(), label);
  assert.equal(byLabel('Evaluation years'), '2003–2005');
  assert.equal(byLabel('Time scale'), 'Day');
  await run();
  const submitted = calls.find((call) => call.command === 'bench_run');
  assert.ok(submitted);
  assert.equal(submitted.args?.config, built.config);
  assert.deepEqual(submitted.args?.sites, generatedValue!.sites);
  assert.equal(submitted.args?.refRoot, '/refs');
  assert.deepEqual(notices, ['Evaluation finished']);
  // 摘要里的模拟 history、坐标、站点清单和提交的站点清单是同一份冻结输入。
  assert.ok(String(byLabel('Simulation 1')).includes(probed.history_dir));
  assert.ok(String(byLabel('Simulation 1')).includes('47.1°, 11.3°'));
  assert.deepEqual(
    generatedValue!.sites.map((site) => site.history_dir),
    [probed.history_dir],
  );
  assert.deepEqual(submitted.args?.sites, built.sites);
  assert.ok(built.config.includes('Latent_Heat: "ERA5LAND_LowRes"'));
});

test('U-05: a run success notice is cleared as soon as the evaluation settings change', () => {
  const cleared: string[] = [];
  const clearWhenChanged = execute<() => void>('clearStaleEvaluationNotice', {
    runNotice: 'Evaluation finished',
    runNoticeFor: { current: 'old-design' },
    effectiveFingerprint: 'new-design',
    setRunNotice: (value: string) => cleared.push(value),
  });
  clearWhenChanged();
  assert.deepEqual(cleared, ['']);
  const kept: string[] = [];
  const sameDesign = execute<() => void>('clearStaleEvaluationNotice', {
    runNotice: 'Evaluation finished',
    runNoticeFor: { current: 'same-design' },
    effectiveFingerprint: 'same-design',
    setRunNotice: (value: string) => kept.push(value),
  });
  sameDesign();
  assert.deepEqual(kept, []);
  const empty: string[] = [];
  const noNotice = execute<() => void>('clearStaleEvaluationNotice', {
    runNotice: '',
    runNoticeFor: { current: '' },
    effectiveFingerprint: 'x',
    setRunNotice: (value: string) => empty.push(value),
  });
  noNotice();
  assert.deepEqual(empty, []);
});

test('GUI-07: submission errors carry the sidecar unfinished-job message into the page', async () => {
  const ipcCalls: string[] = [];
  const errors: string[] = [];
  const notices: string[] = [];
  const shown = (error: unknown) => {
    const [, english] = benchErrorText(String(error).replace(/^Error: /, ''));
    return english;
  };
  const task = execute<(fn: () => Promise<unknown>, done?: string) => Promise<void>>('task', {
    lock: { current: false },
    setBusy: () => {},
    setError: (value: string) => errors.push(value),
    setNotice: (value: string) => notices.push(value),
    benchErrorText,
    shown,
    t: (_zh: string, english: string) => english,
    LEFT: 'compare page left',
    host: 'serverA',
    hostEpoch: { current: 1 },
    refresh: async () => null,
  });
  const gridInfo: HistoryInfo = {
    ...probedGrid,
    history_dir: '/p/hb/out/hb/history',
  };
  const yaml = buildOpenbenchConfig({
    name: 'qfo era5',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: 'hb', info: gridInfo }],
    projectDir: '/ob/projects/qfo_era5',
  }).config;
  const qfoDesign = {
    machine: { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' },
    name: 'qfo era5',
    years: [2002, 2005] as [number, number],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: 'hb', info: gridInfo }],
    projectDir: '/ob/projects/qfo_era5',
  };
  // 运行必须绑定生成时冻结的快照（R01）；这里模拟"刚生成、未改动"的状态。
  const generated = {
    current: compareConfig.freezeCompareSnapshot(qfoDesign, { config: yaml, sites: [] }),
  };
  const run = execute<() => Promise<unknown>>('run', {
    task,
    generated,
    formDesign: qfoDesign,
    designFingerprint,
    snapshotStale: compareConfig.snapshotStale,
    parseOpenbenchConfig: compareConfig.parseOpenbenchConfig,
    structuralProblems: compareConfig.structuralProblems,
    configFingerprint: compareConfig.configFingerprint,
    name: 'qfo era5',
    comparisonName,
    remote: true,
    host: 'serverA',
    assertNoUnresolvedJob: () => {},
    invokeJson: async (command: string) => {
      ipcCalls.push(command);
      if (command === 'bench_remote_run')
        throw new Error('这个比较还有未完成的评估作业（bench-run-77）：先核对或停止它。');
      return { job: 'new-job' };
    },
    yaml,
    sites: [],
    scanDir: '/refs',
    watch: async () => ({ state: 'succeeded' }),
    afterWatch: async () => ({ state: 'succeeded' }),
    fetchReport: async () => {},
    setConfigSaving: () => {},
    setLastRun: () => {},
    showRunNotice: () => {},
    t: (_zh: string, english: string) => english,
  });
  await run();
  assert.deepEqual(
    ipcCalls,
    ['bench_remote_run'],
    'the rejected submission makes one backend call',
  );
  assert.equal(
    errors.at(-1),
    'This comparison already has an unfinished evaluation job (bench-run-77); confirm its final state or stop it before resubmitting.',
    'the submission path shows the mapped English message',
  );
  assert.deepEqual(notices.filter(Boolean), [], 'a rejected submission is not a success');
});

test('GUI-07: fetching a report also surfaces the sidecar unfinished-job message', async () => {
  const errors: string[] = [];
  const shown = (error: unknown) => {
    const [, english] = benchErrorText(String(error).replace(/^Error: /, ''));
    return english;
  };
  const task = execute<(fn: () => Promise<unknown>, done?: string) => Promise<void>>('task', {
    lock: { current: false },
    setBusy: () => {},
    setError: (value: string) => errors.push(value),
    setNotice: () => {},
    benchErrorText,
    shown,
    t: (_zh: string, english: string) => english,
    LEFT: 'compare page left',
    host: 'serverA',
    hostEpoch: { current: 1 },
    refresh: async () => null,
  });
  const fetchReport = execute<(machine: string, project: string) => Promise<void>>('fetchReport', {
    fetching: { current: new Set<string>() },
    hostEpoch: { current: 1 },
    stillCurrent: () => true,
    t: (_zh: string, english: string) => english,
    invokeJson: async () => {
      throw new Error('这个比较还有未完成的评估作业（bench-run-88）：先核对或停止它，再取回报告。');
    },
    view: async () => {},
  });
  await task(() => fetchReport('serverA', 'qfo_era5'));
  assert.equal(
    errors.at(-1),
    'This comparison already has an unfinished evaluation job (bench-run-88); confirm its final state or stop it before resubmitting.',
    'the fetch path shows the mapped English message',
  );
});

test('U-05: editing a variable row in the simulation dialog keeps the row identity (focus is not lost)', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.pressLabel('Edit simulation information');
  await page.pressLabel('Add variable');
  const rows = () =>
    page.nodes().filter((node) => node.props['data-sim-variable-row'] !== undefined);
  const keysBefore = rows().map((row) => String(row.props.key));
  assert.equal(
    keysBefore.length,
    1,
    'the empty form starts without variable rows, Add creates one',
  );
  assert.deepEqual(keysBefore, ['0'], 'row keys are the row positions');
  // 逐字符输入不能改变行的 key：key 变了 React 会重建行、输入框丢焦点（审查报告 C12）。
  const input = page
    .nodes()
    .find((node) => node.type === 'Input' && node.props['data-sim-field'] === 'variables');
  assert.ok(input, 'the first variable row input');
  (input.props.onChange as (event: unknown) => void)({ target: { value: 'Latent_Heat' } });
  await page.settle();
  const keysAfter = rows().map((row) => String(row.props.key));
  assert.deepEqual(keysAfter, keysBefore, 'row keys follow position, not the edited text');
  assert.equal(rows()[0]!.props['data-sim-variable-row'], 0);
  const edited = page
    .nodes()
    .find((node) => node.type === 'Input' && node.props['data-sim-field'] === 'variables');
  assert.equal(edited?.props.value, 'Latent_Heat');
});

test('R12: a remote project row shows the last run failure instead of "no run recorded"', async () => {
  const page = pageHarness({
    seed: { 'colm.benchHost': 'serverA' },
    remoteStatus: {
      ...readyStatus,
      projects: [
        {
          name: 'qfo_era5',
          report: null,
          latest_run: {
            run_id: 'bench-run-42',
            state: 'failed',
            report: null,
            created_at: '2026-10-30T01:00:00Z',
            finished_at: '2026-10-30T01:02:00Z',
            config_sha: 'a'.repeat(64),
            error: 'check failed: boom',
          },
          latest_success: null,
        },
      ],
    },
  });
  await page.mount();
  const shown = text(page.nodes());
  assert.match(shown, /last run: failed/);
  assert.match(shown, /bench-run-42/);
  assert.match(shown, /check failed: boom/);
});

test('U-04: older successful runs stay reachable from the run history', async () => {
  const page = pageHarness({
    localStatus: {
      ...readyStatus,
      projects: [
        {
          name: 'cmp',
          report: '/ob/projects/cmp/runs/r2/output/run/reports/evaluation_report.html',
          latest_run: {
            run_id: 'r2',
            state: 'succeeded',
            report: '/ob/projects/cmp/runs/r2/output/run/reports/evaluation_report.html',
            created_at: '2026-10-30T01:00:00Z',
            finished_at: '2026-10-30T01:02:00Z',
            config_sha: 'a'.repeat(64),
            error: null,
          },
          latest_success: {
            run_id: 'r2',
            config_sha: 'a'.repeat(64),
            report: '/ob/projects/cmp/runs/r2/output/run/reports/evaluation_report.html',
            finished_at: '2026-10-30T01:02:00Z',
          },
          runs: [
            {
              run_id: 'r2',
              state: 'succeeded',
              report: '/ob/projects/cmp/runs/r2/output/run/reports/evaluation_report.html',
              created_at: '2026-10-30T01:00:00Z',
              finished_at: '2026-10-30T01:02:00Z',
              config_sha: 'a'.repeat(64),
              error: null,
            },
            {
              run_id: 'r1',
              state: 'failed',
              report: null,
              created_at: '2026-10-30T00:00:00Z',
              finished_at: '2026-10-30T00:05:00Z',
              config_sha: 'b'.repeat(64),
              error: 'check failed: boom',
            },
          ],
        },
      ],
    },
  });
  await page.mount();
  const shown = text(page.nodes());
  assert.match(shown, /Run history/);
  assert.match(shown, /r2/);
  assert.match(shown, /check failed: boom/);
  const view = page
    .nodes()
    .find((node) => node.type === 'Button' && text(node) === 'View this report');
  assert.ok(view, 'the succeeded older run offers its own report');
  (view.props.onClick as () => void)();
  await page.settle();
  assert.ok(
    page.calls.some(
      (call) =>
        call.command === 'bench_report' &&
        call.args?.path === '/ob/projects/cmp/runs/r2/output/run/reports/evaluation_report.html',
    ),
    "viewing from the history opens that run's report, not the latest file on disk",
  );
});

test('U-06: leaving the page keeps a draft and returning offers an explicit restore', async () => {
  const draft = {
    version: 1,
    savedAt: '2026-10-30T01:00:00Z',
    host: '',
    name: 'restored compare',
    from: '2003',
    to: '2004',
    timRes: 'Month',
    gridRes: '0.5',
    chosen: { Latent_Heat: ['ERA5LAND_LowRes'] },
    weight: '',
    minYear: '1',
    timeAlignment: '',
    numCores: '',
    metrics: ['bias'],
    scores: ['Overall_Score'],
    comparisonItems: ['Taylor_Diagram'],
    statisticsItems: ['Mean'],
    yaml: '',
    picks: [
      {
        dir: probedGrid.history_dir,
        label: 'hb',
        info: probedGrid,
        dataGroupby: 'Month',
        gridRes: 0.5,
      },
    ],
  };
  const page = pageHarness({
    seed: { 'colm.compareDraft@': JSON.stringify(draft) },
  });
  await page.mount();
  assert.match(text(page.nodes()), /Restore draft/);
  await page.pressLabel('Restore draft');
  const summary = page.summaryText();
  assert.match(summary, /\/ob\/projects\/restored_compare/);
  assert.match(summary, /Lagrange|Month/);
  assert.match(summary, /Latent_Heat/);
  assert.equal(text(page.nodes()).includes('Restore draft'), false);
});

test('U-06: discarding the draft removes it and leaves the page empty', async () => {
  const draft = JSON.stringify({
    version: 1,
    savedAt: '',
    host: '',
    name: 'stale draft',
    from: '',
    to: '',
    timRes: 'Day',
    gridRes: '',
    chosen: {},
    weight: '',
    minYear: '1',
    timeAlignment: '',
    numCores: '',
    metrics: [],
    scores: [],
    comparisonItems: [],
    statisticsItems: [],
    yaml: '',
    picks: [{ dir: '/p/x/history', label: 'x', info: null }],
  });
  const page = pageHarness({ seed: { 'colm.compareDraft@': draft } });
  await page.mount();
  await page.pressLabel('Discard draft');
  assert.equal(page.localValue('colm.compareDraft@'), '');
  assert.equal(text(page.nodes()).includes('Discard draft'), false);
  assert.equal(page.summaryText().includes('stale draft'), false);
});

test('U-06: page inputs are written to a per-machine draft as they change', async () => {
  const page = pageHarness();
  await page.mount();
  await page.press('compare-add-folder');
  await page.addVariable();
  const raw = page.localValue('colm.compareDraft@');
  assert.ok(raw, 'the page writes a draft after the first input');
  const draft = JSON.parse(raw) as {
    picks: { dir: string }[];
    chosen: Record<string, string[]>;
    name: string;
  };
  assert.equal(draft.picks[0]?.dir, probedGrid.history_dir);
  assert.deepEqual(draft.chosen.Latent_Heat, ['ERA5LAND_LowRes']);
  assert.equal(draft.name, 'comparison');
});
