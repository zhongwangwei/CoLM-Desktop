import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as compare from './compare';
import * as compareConfig from './config';
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

function pageHarness() {
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => {
        const seed: Record<string, string> = { 'colm.openbenchRefRoot': '/refs' };
        return seed[key] ?? null;
      },
      setItem: () => {},
      removeItem: () => {},
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
        return readyStatus;
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
  const usePageContract = (value: Contract) => {
    contract = value;
  };
  const environment: Record<string, unknown> = {
    ...compare,
    ...compareConfig,
    React,
    useApp,
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
  // 未保存配置：SaveBadge 是"有未保存修改"。
  assert.equal(page.badgeState(), 'dirty');
  assert.equal(
    page.actions().find((action) => action.id === 'compare-generate')?.label,
    'Save evaluation configuration',
  );

  await page.press('compare-generate');
  assert.equal(page.badgeState(), 'saved');
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

test('U-05: hand-edited configuration survives until the user explicitly regenerates it', async () => {
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
  // 手改过的配置不被背景清掉；表单改动没有进入这份文本，页面要明确说出来。运行按手改后的文本
  // （有效配置即文本本身），所以不再用"表单指纹变了"把它一刀切判成不可运行。
  const kept = page.configTextarea();
  assert.ok(kept, 'the edited configuration is kept');
  assert.match(String(kept.props.value), /# mine/);
  assert.match(text(page.nodes()), /You changed the evaluation settings after generating/);
  assert.equal(
    page.actions().find((action) => action.id === 'compare-run')?.enabled,
    true,
    'a hand-edited configuration is the effective one and can run',
  );
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

test('U-04: the condition summary is computed from the very comparisonDesign the run submits', async () => {
  // 直接执行页面真实函数：comparisonDesign / conditionSummary / generate / run 共享同一份上下文。
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
    comparisonDir: compare.comparisonDir,
  });
  const machine = { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' };
  const design = comparisonDesign();
  const machineDesign = { machine, ...design };
  // 摘要现在的来源是"有效配置"（解析后的提交文本）；没有文本时才退回表单设计。
  const formSummary = execute<() => ReturnType<typeof evaluationSummary>>('conditionSummary', {
    effectiveConfig: null,
    formDesign: machineDesign,
    effectiveSummary: compareConfig.effectiveSummary,
    evaluationSummary,
  });
  assert.deepEqual(formSummary(), evaluationSummary(machineDesign));
  const byLabelOf = (summary: ReturnType<typeof evaluationSummary>, label: string) =>
    summary.find((item) => item.label[1] === label)?.value;
  assert.equal(byLabelOf(formSummary(), 'Machine'), 'server serverA');
  assert.equal(byLabelOf(formSummary(), 'Comparison project folder'), design.projectDir);
  assert.equal(byLabelOf(formSummary(), 'Reference data root'), '/refs');
  assert.match(byLabelOf(formSummary(), 'Simulation 1') ?? '', /AT-Neu_hist_/);
  assert.match(byLabelOf(formSummary(), 'Simulation 1') ?? '', /47\.1°, 11\.3°/);

  // 用同一份 design 生成配置；再用 run 提交。比对 summary ↔ config ↔ sites ↔ 实际 IPC 参数。
  const yamlRef: { current: string } = { current: '' };
  const sitesRef: { current: unknown[] } = { current: [] };
  const generated: { current: { fingerprint: string; config: string; sites: unknown[] } | null } = {
    current: null,
  };
  const errors: string[] = [];
  const generate = execute<() => void>('generate', {
    comparisonDesign,
    buildOpenbenchConfig,
    designFingerprint,
    machine,
    generated,
    setYaml: (value: string) => {
      yamlRef.current = value;
    },
    setSites: (value: unknown[]) => {
      sitesRef.current = value;
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
  assert.deepEqual(sitesRef.current, built.sites);
  assert.equal(generated.current?.fingerprint, designFingerprint({ machine, ...design }));

  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const notices: string[] = [];
  const parsed = compareConfig.parseOpenbenchConfig(yamlRef.current);
  assert.deepEqual(parsed.problems, []);
  const parseOpenbenchConfig = compareConfig.parseOpenbenchConfig;
  const structuralProblems = compareConfig.structuralProblems;
  const configFingerprint = compareConfig.configFingerprint;
  const run = execute<() => Promise<unknown>>('run', {
    task: async (fn: () => Promise<unknown>) => fn(),
    generated,
    formDesign: machineDesign,
    name: 'versions vs flux',
    comparisonName,
    remote: false,
    parseOpenbenchConfig,
    structuralProblems,
    configFingerprint,
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return { report: null };
    },
    yaml: yamlRef.current,
    sites: sitesRef.current,
    scanDir: '/refs',
    host: '',
    view: async () => {},
    watch: async () => ({}),
    fetchReport: async () => {},
    setConfigSaving: () => {},
    setLastRun: () => {},
    showRunNotice: (value: string) => notices.push(value),
    t: (_zh: string, english: string) => english,
  });
  // 摘要从有效配置解析：与表单设计同一份条件，数值一致。
  const effectiveSummary = execute<() => ReturnType<typeof evaluationSummary>>('conditionSummary', {
    effectiveConfig: parsed.config,
    formDesign: machineDesign,
    effectiveSummary: compareConfig.effectiveSummary,
    evaluationSummary,
  });
  const byLabel = (label: string) => byLabelOf(effectiveSummary(), label);
  assert.equal(byLabel('Evaluation years'), '2003–2005');
  assert.equal(byLabel('Time scale'), 'Day');
  await run();
  const submitted = calls.find((call) => call.command === 'bench_run');
  assert.ok(submitted);
  assert.equal(submitted.args?.config, built.config);
  assert.deepEqual(submitted.args?.sites, built.sites);
  assert.equal(submitted.args?.refRoot, '/refs');
  assert.deepEqual(notices, ['Evaluation finished']);
  // 摘要里的模拟 history、站点清单、变量和指标都来自这份被提交的设计。
  assert.ok(String(byLabel('Simulation 1')).includes(probed.history_dir));
  assert.deepEqual(
    built.sites.map((site) => site.history_dir),
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
  const generated = { current: { fingerprint: 'same', config: yaml, sites: [] } };
  const run = execute<() => Promise<unknown>>('run', {
    task,
    generated,
    formDesign: {
      machine: { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' },
      name: 'qfo era5',
      years: [2002, 2005],
      timRes: 'Day',
      variables: { Latent_Heat: 'ERA5LAND_LowRes' },
      simulations: [{ label: 'hb', info: gridInfo }],
      projectDir: '/ob/projects/qfo_era5',
    },
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
