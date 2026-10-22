import { it } from 'node:test';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import assert from 'node:assert/strict';
import {
  methodFromStudy,
  completedCandidates,
  resultFileLabel,
  shownResultFiles,
  researchMethods,
} from './research-home';
import {
  applyResearchSeed,
  calibrationWindows,
  candidateBudget,
  fitValidation,
  gridTargetDefaults,
  gridTargetSpec,
  networkDefaults,
  networkSpec,
  parameterSpec,
  studyPermissions,
  suggestedRange,
} from './study';
import { remoteStudyPermissions, remoteStudyReply, studyModelRevision } from './remote-study';
import { hubMethods } from '../versions/hub-methods';
import { IGBP, PFT, USGS } from '../versions/scope-labels';
import { REMOTE_CHECK_MS, compatibleKernel } from '../runs/run-model';
import { withCatalogLabels } from '../../lib/param-presentation';
it('opens existing studies in the matching built-in method', () => {
  assert.equal(methodFromStudy({ kind: 'uncertainty' }), 'uncertainty');
  assert.equal(methodFromStudy({ kind: 'tuning', hybrid: { slot: 'soil_stress' } }), 'process');
  assert.equal(methodFromStudy({ kind: 'tuning', hybrid: { slot: 'pft' } }), 'parameters');
  assert.equal(methodFromStudy({ kind: 'tuning' }), 'tuning');
  assert.equal(methodFromStudy(null), 'tuning');
});
it('never invents completed progress from absent or invalid counts', () => {
  for (const value of [undefined, null, '4', -1, 1.5, NaN, Infinity])
    assert.equal(completedCandidates(value), null);
  assert.equal(completedCandidates(0), 0);
  assert.equal(completedCandidates(7), 7);
});

// Exercise actual page callbacks so the directory switch and task tokens cannot drift from this regression.
function pageCallback(name: string, context: Record<string, unknown>) {
  const path = new URL('./ResearchPage.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body: string | undefined;
  const visit = (node: ts.Node) => {
    if (
      name === 'localStudyPoll' &&
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.getText(source).includes('3000')
    )
      body = node.arguments[0].getText(source);
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name && node.initializer)
      body = node.initializer.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const js = ts.transpileModule(`const run=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js};return run;`)(
    ...Object.values(context),
  ) as () => Promise<unknown>;
}
it('opening a broken study shows its error after advancing the directory token', async () => {
  const epoch = { current: 1 },
    lock = { current: false },
    errors: string[] = [],
    opened: boolean[] = [];
  const noop = () => {};
  const task = pageCallback('task', {
    epoch,
    lock,
    setBusy: noop,
    setError: (error: string) => errors.push(error),
  });
  const browse = pageCallback('browseStudy', {
    task,
    epoch,
    invoke: async () => '/broken',
    setStudyDir: noop,
    setEnvelope: noop,
    setRemoteReply: noop,
    setResult: noop,
    setConfirmed: noop,
    setError: (error: string) => errors.push(error),
    readStatus: async () => {
      throw new Error('invalid study manifest');
    },
    methodFromStudy,
    setTab: noop,
    chooseSlot: noop,
    setWorkbench: (value: boolean) => opened.push(value),
  });
  await assert.rejects(browse(), /invalid study manifest/);
  assert.equal(errors.at(-1), 'Error: invalid study manifest');
  assert.equal(lock.current, false);
  assert.deepEqual(opened, []);
});
it('late study reads neither display stale errors nor open a stale workbench', async () => {
  for (const fail of [false, true]) {
    const epoch = { current: 1 },
      errors: string[] = [],
      opened: boolean[] = [];
    const noop = () => {};
    const task = pageCallback('task', {
      epoch,
      lock: { current: false },
      setBusy: noop,
      setError: (error: string) => errors.push(error),
    });
    const browse = pageCallback('browseStudy', {
      task,
      epoch,
      invoke: async () => '/old',
      setStudyDir: noop,
      setEnvelope: noop,
      setRemoteReply: noop,
      setResult: noop,
      setConfirmed: noop,
      setError: (error: string) => errors.push(error),
      readStatus: async () => {
        epoch.current++;
        if (fail) throw new Error('old read failed');
        return { manifest: { spec: { kind: 'uncertainty' } } };
      },
      methodFromStudy,
      setTab: () => opened.push(true),
      chooseSlot: noop,
      setWorkbench: (value: boolean) => opened.push(value),
    });
    if (fail) await assert.rejects(browse(), /old read failed/);
    else await browse();
    assert.deepEqual(errors, ['']);
    assert.deepEqual(opened, []);
  }
});

it('a picked directory becomes the current study only after a successful current status read', async () => {
  for (const outcome of ['valid', 'broken', 'stale']) {
    const epoch = { current: 1 },
      paths: string[] = [],
      shown: unknown[] = [];
    const noop = () => {};
    const read = pageCallback('readStatus', {
      useCallback: (fn: unknown) => fn,
      studyDir: '/previous',
      epoch,
      invokeJson: async () => {
        if (outcome === 'broken') throw new Error('missing manifest');
        if (outcome === 'stale') epoch.current++;
        return { manifest: { id: 's-actual' }, state: {}, results: [] };
      },
      remoteStudyReply: (value: unknown) => value,
      setStudyDir: (path: string) => paths.push(path),
      setEnvelope: (value: unknown) => shown.push(value),
      setHost: noop,
      setMember: noop,
      setResultPath: noop,
      shownResultFiles,
    }) as (path: string, adopt: boolean) => Promise<unknown>;
    if (outcome === 'broken') await assert.rejects(read('/picked', true), /missing manifest/);
    else await read('/picked', true);
    assert.deepEqual(
      paths,
      outcome === 'valid' ? ['/picked'] : [],
      `${outcome} cannot replace the directory prematurely`,
    );
    assert.equal(shown.length, outcome === 'valid' ? 1 : 0);
  }
});

it('local polling stops during study selection and a scheduled callback rechecks the operation lock', async () => {
  const scheduled: (() => Promise<void>)[] = [],
    lock = { current: false };
  let reads = 0;
  const context = {
    active: true,
    studyDir: '/old',
    busy: false,
    launching: false,
    remotePermissions: { exists: false },
    envelope: { state: { status: 'running' } },
    lock,
    readStatus: async () => {
      reads++;
    },
    setError: () => {},
    setTimeout: (fn: () => Promise<void>) => {
      scheduled.push(fn);
      return 1;
    },
    clearTimeout: () => {},
  };
  pageCallback('localStudyPoll', { ...context, busy: true })();
  assert.equal(scheduled.length, 0);
  // 研究页常驻但隐藏时不轮询，回到研究页再接着刷新。
  pageCallback('localStudyPoll', { ...context, active: false })();
  assert.equal(scheduled.length, 0);
  pageCallback('localStudyPoll', context)();
  assert.equal(scheduled.length, 1);
  lock.current = true;
  await scheduled[0]();
  assert.equal(reads, 0, 'an old timer cannot start a read after browseStudy acquires its lock');
});

it('legacy research provenance does not inherit the currently selected frozen revision', () => {
  const current = { versionRef: { id: 'official', revision: 3 } };
  assert.equal(
    pageCallback('modelUsed', {
      envelope: { model_version: { name: 't8-ui-karman' } },
      state: current,
    }),
    undefined,
  );
  const pinned = { id: 'soil', revision: 2 };
  assert.equal(
    pageCallback('modelUsed', {
      envelope: { model_version: { model_revision: { version: pinned } } },
      state: current,
    }),
    pinned,
  );
});

// 研究结果进版本要走草稿：没绑定草稿时以研究记下的版本为父版本新建，绑定了就写回那个草稿，
// 研究没记版本时不拿当前选的版本去猜。
function writeBackContext(overrides: Record<string, unknown>) {
  const calls: [string, Record<string, unknown>][] = [];
  const written: unknown[] = [];
  const context = {
    task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    studyDir: '/p/.colm/studies/s-1',
    state: { researchTarget: null as null | { draftId: string; name: string } },
    modelUsed: { id: 'official', revision: 6 } as unknown,
    update: () => {},
    tr: (zh: string) => zh,
    en: false,
    methodTitle: { zh: '参数率定', en: 'Calibration' },
    envelope: { manifest: { id: 's-1' } },
    member: '',
    defaultMember: () => '',
    setWriting: (value: unknown) => written.push(value),
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'model_create' || command === 'model_draft')
        return { id: (args.id as string) ?? 'model-new', generation: 4, overlay: {} };
      if (command === 'model_import_study') return { member: 'm000001', objective: 1, records: [] };
      return null;
    },
    ...overrides,
  };
  return { context, calls, written };
}
it('writes study results into a new draft based on the version the study recorded', async () => {
  const { context, calls, written } = writeBackContext({});
  await pageCallback('writeBack', context)();
  assert.deepEqual(
    calls.map(([command]) => command),
    ['model_create', 'model_import_study'],
  );
  assert.deepEqual(calls[0]![1].parent, { id: 'official', revision: 6 });
  assert.equal(calls[1]![1].id, 'model-new');
  assert.equal(calls[1]![1].preview, true);
  assert.equal(calls[1]![1].member, null, 'an empty member means the best one');
  assert.equal((written[0] as { created: boolean }).created, true);
});
it('writes back into the draft the study was started from without creating another', async () => {
  const { context, calls, written } = writeBackContext({
    state: { researchTarget: { draftId: 'model-7', name: 'PFT' } },
    modelUsed: undefined,
    member: 'm000003',
  });
  await pageCallback('writeBack', context)();
  assert.deepEqual(
    calls.map(([command]) => command),
    ['model_draft', 'model_import_study'],
  );
  assert.equal(calls[1]![1].id, 'model-7');
  assert.equal(calls[1]![1].member, 'm000003');
  assert.equal((written[0] as { created: boolean }).created, false);
});
it('refuses a new version for a study without a recorded version and cleans up failed previews', async () => {
  const none = writeBackContext({ modelUsed: undefined });
  await assert.rejects(pageCallback('writeBack', none.context)(), /没有记下所用的模型版本/);
  assert.equal(none.calls.length, 0);
  const failing = writeBackContext({});
  const invoke = failing.context.invokeJson;
  failing.context.invokeJson = async (command: string, args: Record<string, unknown>) => {
    if (command === 'model_import_study') {
      failing.calls.push([command, args]);
      throw new Error('study has no completed member');
    }
    return invoke(command, args);
  };
  await assert.rejects(pageCallback('writeBack', failing.context)(), /no completed member/);
  assert.deepEqual(
    failing.calls.map(([command]) => command),
    ['model_create', 'model_import_study', 'model_delete_draft'],
  );
  assert.equal(failing.written.length, 0);
});

// 不确定性分析没有最优成员：下拉框显示的和写入的都是第一个样本成员，不是空值（后端当 best 拒绝）。
it('the member written by default is the one the member list shows', () => {
  const pick = (state: unknown) => pageCallback('defaultMember', { envelope: { state } })();
  assert.equal(pick({ best_member: 'm000004', candidates: { m000004: {} } }), 'm000004');
  assert.equal(pick({ candidates: { m000002: {}, m000000: {}, m000001: {} } }), 'm000001');
  assert.equal(pick({ candidates: { m000000: {} } }), 'm000000');
  assert.equal(pick({}), '');
});

it('result files list the results people read, not every member record', () => {
  const files = shownResultFiles([
    'de/g000001.json',
    'metrics.csv',
    { path: 'objectives.csv' },
    'provenance/m000000/cng.json',
    'tasks/m000001/cng.json',
  ]);
  assert.deepEqual(files, ['objectives.csv', 'de/g000001.json', 'metrics.csv']);
  assert.equal(
    resultFileLabel('objectives.csv', false),
    '各成员的目标函数（表格） · objectives.csv',
  );
  assert.equal(resultFileLabel('de/g000012.json', false), '差分进化第 12 代 · de/g000012.json');
  assert.equal(resultFileLabel('outputs/qle.csv', true), 'outputs/qle.csv');
});

// "待复核"的成员：问过用户再连同它们一起重跑；用户不确认就什么都不发（GUI 审查 R3）。
it('retrying a study that needs review reruns those members only after confirmation', async () => {
  for (const confirmed of [true, false]) {
    const calls: [string, Record<string, unknown>][] = [];
    const control = pageCallback('control', {
      remotePermissions: { exists: false },
      tr: (zh: string) => zh,
      status: 'needs_review',
      askConfirm: async () => confirmed,
      epoch: { current: 1 },
      task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => false),
      invoke: async (command: string, args: Record<string, unknown>) => {
        calls.push([command, args]);
        return null;
      },
      studyDir: '/s',
      readStatus: async () => null,
      runLock: { current: false },
      start: async () => null,
    });
    await (control as unknown as (command: string) => Promise<unknown>)('study_retry');
    assert.deepEqual(
      calls,
      confirmed ? [['study_retry', { studyDir: '/s', includeReview: true }]] : [],
    );
  }
});

// 服务器提交失败而研究里没有任何服务器记录：收回"提交中"占位，可以改了再交（GUI 审查 R2）。
it('a server submission that recorded nothing can be submitted again', async () => {
  for (const recorded of [false, true]) {
    const replies: unknown[] = [];
    let reply: unknown = null;
    const start = pageCallback('start', {
      runLock: { current: false },
      lock: { current: false },
      runKernel: '/kernel',
      studyDir: '/s',
      envelope: { state: { status: 'Ready' } },
      confirmed: true,
      remotePermissions: { exists: false },
      tr: (zh: string) => zh,
      jobs: '1',
      studyModelRevision: () => ({ id: 'official', revision: 1 }),
      state: { versionRef: { id: 'official', revision: 1 } },
      remoteRequest: { current: 0 },
      setLaunching: () => {},
      setError: () => {},
      epoch: { current: 1 },
      host: 'server',
      setRemoteReply: (next: unknown) => {
        reply = typeof next === 'function' ? (next as (old: unknown) => unknown)(reply) : next;
        replies.push(reply);
      },
      remoteStudyReply: (value: unknown) => value,
      invokeJson: async () => {
        throw new Error('ssh: connection refused');
      },
      readStatus: async () => (recorded ? { remote_pending: { host: 'server' } } : {}),
      ai: false,
      engine: 'rust',
      preprocessors: 'rust',
    });
    await assert.rejects(start(), /connection refused/);
    if (recorded) assert.equal((reply as { status: { state: string } }).status.state, 'unknown');
    else assert.equal(reply, null);
  }
});

// 第 737 轮：动作由界面按钮自己登记（ActionButton）。这里执行整个页面组件（去掉 import，
// 用保留元素树的假 React），从渲染结果里收集助手此刻能按的动作，规则与 lib/guide.ts 的
// renderedButtons() 一致：只算显示着的按钮；合上的折叠区里的按钮不算；有打开的对话框时只算
// 最上层对话框里的按钮。旧的 PageContract 手写清单已清空，所以下面断言的就是助手看到的全部。
type TreeProps = Record<string, unknown> & {
  action?: string;
  label?: string;
  kind?: 'click' | 'commit';
  disabled?: boolean;
  open?: boolean;
  title?: unknown;
  run?: () => unknown;
};
type Tree = { type: unknown; props: TreeProps; children: unknown[] };
type Action = {
  id: string;
  label: string;
  kind: 'click' | 'commit';
  enabled: boolean;
  run: () => unknown;
};
type Contract = { actions: Action[] } & Record<string, unknown>;
type Request = {
  command: string;
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
};
function researchPage(overrides: Record<string, unknown> = {}) {
  const source = readFileSync(new URL('./ResearchPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const state: Record<string, unknown> = {
    language: 'zh',
    selectedCase: { dir: '/p/site1', name: 'site1' },
    versionRef: { id: 'official', revision: 1 },
    versions: [{ version: { id: 'official', revision: 1 }, name: 'Official CoLM' }],
    kernels: [
      { dir: '/kernels/default', preset: 'default', platform: 'macos', macros: ['SinglePoint'] },
    ],
    kernelDir: '/kernels/default',
    cases: [{ dir: '/p/site1', name: 'site1', spatial: false, has_history: false }],
    root: '/p',
    theme: 'light',
    busy: false,
    ...overrides,
  };
  const slots: unknown[] = [];
  const callbacks: { deps: unknown[]; fn: unknown }[] = [];
  const effects: { deps: unknown[]; cleanup?: () => void }[] = [];
  const requested: Request[] = [];
  const updates: unknown[] = [];
  const routes: unknown[] = [];
  const notices: unknown[] = [];
  let cursor = 0,
    callbackCursor = 0,
    effectCursor = 0;
  const pending: (() => void)[] = [];
  let tree: Tree | null = null;
  let contract: Contract = { actions: [] };
  // remote_config 与 model_case_profiles 是页面自己读的环境信息，不是动作；直接给空结果，
  // 免得每个用例先处理它们。
  const invoke = (command: string, args: Record<string, unknown>) =>
    new Promise((resolve, reject) => {
      if (command === 'remote_config') return resolve({ servers: [] });
      if (command === 'model_case_profiles') return resolve({});
      requested.push({ command, args, resolve, reject });
    });
  const app = {
    state,
    notify: (message: string, error?: boolean) => notices.push([message, error]),
    refreshVersions: async () => {},
    navigate: (route: string, tab?: string) => {
      routes.push([route, tab]);
      return true;
    },
    update: (patch: Record<string, unknown>) => Object.assign(state, patch),
  };
  const env: Record<string, unknown> = {
    React: {
      createElement: (type: unknown, props: TreeProps | null, ...children: unknown[]) => ({
        type,
        props: props ?? {},
        children,
      }),
      Fragment: 'Fragment',
    },
    ActionButton: 'ActionButton',
    Advanced: 'Advanced',
    Button: 'Button',
    Card: 'Card',
    Field: 'Field',
    // 参数表用 <Fragment key> 分组，JSX 会编译成 React.createElement(Fragment, ...)。
    Fragment: 'Fragment',
    Input: 'Input',
    LiveServerLoad: 'LiveServerLoad',
    Notice: 'Notice',
    ScienceChart: 'ScienceChart',
    Select: 'Select',
    StudyImportDialog: 'StudyImportDialog',
    askConfirm: async () => true,
    applyResearchSeed,
    calibrationWindows,
    candidateBudget,
    compatibleKernel,
    completedCandidates,
    fitValidation,
    gridTargetDefaults,
    gridTargetSpec,
    hubMethods,
    IGBP,
    invoke,
    invokeJson: invoke,
    methodFromStudy,
    networkDefaults,
    networkSpec,
    parameterSpec,
    PFT,
    remoteStudyPermissions,
    remoteStudyReply,
    REMOTE_CHECK_MS,
    researchMethods,
    resultFileLabel,
    shownResultFiles,
    studyModelRevision,
    studyPermissions,
    USGS,
    withCatalogLabels,
    suggestedRange,
    useApp: () => app,
    useNavigationGuard: () => {},
    usePageContract: (value: Contract) => {
      contract = value;
    },
    useCallback: (fn: unknown, deps: unknown[]) => {
      const i = callbackCursor++;
      if (!callbacks[i] || deps.some((value, j) => value !== callbacks[i]!.deps[j]))
        callbacks[i] = { deps, fn };
      return callbacks[i]!.fn;
    },
    useEffect: (effect: () => () => void, deps: unknown[]) => {
      const i = effectCursor++;
      if (!effects[i] || deps.some((value, j) => value !== effects[i]!.deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          effects[i]!.cleanup = effect();
        });
      }
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (value: unknown) => {
          slots[i] =
            typeof value === 'function' ? (value as (old: unknown) => unknown)(slots[i]) : value;
        },
      ];
    },
    // 轮询用的计时器：用例里不真的跑定时器。
    setTimeout: () => 0,
    clearTimeout: () => {},
  };
  const Page = new Function(...Object.keys(env), `${code}; return ResearchPage;`)(
    ...Object.values(env),
  ) as (props: { active?: boolean }) => Tree;
  const render = () => {
    cursor = callbackCursor = effectCursor = 0;
    tree = Page({ active: true });
    while (pending.length) pending.shift()!();
    return tree;
  };
  const nodes = (node: unknown): Tree[] =>
    Array.isArray(node)
      ? node.flatMap(nodes)
      : node && typeof node === 'object'
        ? [node as Tree, ...(node as Tree).children.flatMap(nodes)]
        : [];
  function collect(node: unknown): Action[] {
    if (Array.isArray(node)) return node.flatMap(collect);
    if (!node || typeof node !== 'object') return [];
    const element = node as Tree;
    const own: Action[] = [];
    if (element.type === 'ActionButton' && typeof element.props.action === 'string')
      own.push({
        id: element.props.action,
        label: String(element.props.label ?? ''),
        kind: element.props.kind ?? 'click',
        enabled: element.props.disabled !== true,
        run: element.props.run as () => unknown,
      });
    // 折叠区标题（Advanced 的 action）本身也是按钮；合上时里面的按钮在界面上不显示。
    if (element.type === 'Advanced' && typeof element.props.action === 'string')
      own.push({
        id: element.props.action,
        label: typeof element.props.title === 'string' ? element.props.title : 'Advanced',
        kind: 'click',
        enabled: true,
        run: () => {},
      });
    if (element.type === 'Advanced' && element.props.open !== true) return own;
    return [...own, ...collect(element.children)];
  }
  function actions(): Action[] {
    const all = nodes(tree);
    // 打开的对话框挡住房面上的按钮：只算最上层对话框里面的动作。
    const dialogs = all.filter(
      (node) => node.type === 'Dialog' || node.type === 'StudyImportDialog',
    );
    if (dialogs.length) return collect(dialogs.at(-1)!.children);
    return collect(tree);
  }
  // 界面点击不等待异步结果（和真实按钮一致）；用例随后 resolve 页面发出的请求再 settle。
  const press = (id: string) => {
    const action = actions().find((item) => item.id === id);
    assert.ok(action, `${id} is registered`);
    assert.notEqual(action.enabled, false, `${id} is enabled`);
    void Promise.resolve(action.run()).catch(() => {});
    return render();
  };
  const settle = async () => {
    for (let i = 0; i < 6; i++) {
      await new Promise((resolve) => setImmediate(resolve));
      render();
    }
  };
  render();
  render();
  return {
    state,
    requests: requested,
    updates,
    routes,
    notices,
    render,
    press,
    settle,
    actions,
    ids: () =>
      actions()
        .map((action) => action.id)
        .sort(),
    contract: () => contract,
    nodes: () => nodes(tree),
  };
}
const parameterRow = () => ({
  id: 'test',
  name: 'DEF_TEST',
  label_en: 'Test parameter',
  label_zh: '测试参数',
  scope: 'pft',
  scope_instance: { kind: 'case-scalar', scheme: null, index: null },
  default: 1,
  min: 0,
  max: 10,
  min_inclusive: true,
  max_inclusive: true,
  scale: 'linear',
});
const completedEnvelope = () => ({
  manifest: { id: 's-1', spec: { kind: 'tuning', kernel_dir: '/kernels/default' } },
  state: {
    status: 'Completed',
    best_member: 'm1',
    completed_candidates: 2,
    candidates: { m1: { calibration: 0.1 }, m000000: { calibration: 0.2 } },
  },
  results: ['objectives.csv', 'provenance/m1/cng.json'],
  model_version: { model_revision: { version: { id: 'official', revision: 1 } } },
  remote: null,
});
// 打开一个本机已完成的研究，停在运行与结果那一步。
async function openCompletedStudy() {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/study');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  page.requests.at(-1)!.resolve(completedEnvelope());
  await page.settle();
  return page;
}
it('the research home offers exactly the method cards and the open-study button', () => {
  const page = researchPage();
  page.render();
  assert.deepEqual(page.ids(), [
    'research-method-manual',
    'research-method-parameters',
    'research-method-process',
    'research-method-source',
    'research-method-tuning',
    'research-method-uncertainty',
    'study-browse',
  ]);
  // PageContract 不再手写动作清单：助手看到的全部来自渲染出的按钮。
  assert.deepEqual(page.contract().actions, []);
});
it('a method card opens the workbench and each step lists only the buttons on screen', () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  assert.deepEqual(page.ids(), [
    'research-home',
    'research-step-1',
    'research-step-2',
    'research-step-3',
    'research-step-4',
    'research-step-next',
    'study-browse',
  ]);
  const enabled = (id: string) => page.actions().find((action) => action.id === id)!.enabled;
  assert.equal(enabled('research-step-1'), true);
  assert.equal(enabled('research-step-2'), true);
  // 第 3 步要勾参数、第 4 步要建好研究：界面灰着，助手看到的也是不可用。
  assert.equal(enabled('research-step-3'), false);
  assert.equal(enabled('research-step-4'), false);
  // 本机运行没有"计算资源"链接，旧清单却一直列着它（幽灵动作）。
  assert.equal(
    page.actions().some((action) => action.id === 'research-resources'),
    false,
  );
});
it('step 2 prefills a ticked range and step 3 gates only create on the review checkbox', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  // 页面用 execute 把读参数推到下一个微任务，读请求在 settle 之后才发出来。
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'model_case_context');
  page.requests.at(-1)!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_parameter_contexts');
  page.requests.at(-1)!.resolve([parameterRow()]);
  await page.settle();
  assert.deepEqual(page.ids(), [
    'research-home',
    'research-step-1',
    'research-step-2',
    'research-step-3',
    'research-step-4',
    'research-step-back',
    'research-step-next',
    'study-browse',
  ]);
  const enabled = (id: string) => page.actions().find((action) => action.id === id)!.enabled;
  assert.equal(enabled('research-step-next'), false, 'nothing ticked yet');
  const parameter = page
    .nodes()
    .find((node) => node.type === 'input' && node.props['aria-label'] === 'DEF_TEST');
  assert.ok(parameter);
  (parameter.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  assert.equal(enabled('research-step-next'), true);
  const bounds = page
    .nodes()
    .filter((node) => node.type === 'Input' && node.props.className === 'param-input')
    .map((node) => node.props.value);
  assert.deepEqual(bounds, ['0.5', '1.5'], 'ticking fills 0.5–1.5 × default');
  page.press('research-step-3');
  assert.deepEqual(page.ids(), [
    'research-home',
    'research-step-1',
    'research-step-2',
    'research-step-3',
    'research-step-4',
    'research-step-back',
    'study-browse',
    'study-create',
    // 每个已选站点的观测文件"选择…"按钮都在界面上，按站点路径区分。
    'study-observation-pick:/p/site1',
    'study-preflight',
  ]);
  // 预检不落盘，不必先勾核对项；创建才要。
  assert.equal(enabled('study-preflight'), true);
  assert.equal(enabled('study-create'), false);
  const confirm = page
    .nodes()
    .find((node) => node.type === 'input' && node.props.type === 'checkbox');
  assert.ok(confirm);
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  assert.equal(enabled('study-preflight'), true);
  const create = page.actions().find((action) => action.id === 'study-create')!;
  assert.equal(create.enabled, true);
  assert.equal(create.kind, 'commit');
});
it('step 3 offers the targets every site can compare once an observation is chosen', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  page.requests.at(-1)!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  page.requests.at(-1)!.resolve([parameterRow()]);
  await page.settle();
  const parameter = page
    .nodes()
    .find((node) => node.type === 'input' && node.props['aria-label'] === 'DEF_TEST');
  (parameter!.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  page.press('research-step-3');
  await page.settle();
  assert.ok(!page.requests.some((r) => r.command === 'evaluation_plan'), 'no observation yet');
  const obs = page
    .nodes()
    .find(
      (node) => node.type === 'Input' && String(node.props.placeholder ?? '').includes('*_Flux.nc'),
    );
  (obs!.props.onChange as (event: unknown) => void)({ target: { value: '/obs/site1_Flux.nc' } });
  await page.settle();
  const plan = page.requests.at(-1)!;
  assert.equal(plan.command, 'evaluation_plan');
  assert.deepEqual(plan.args, {
    case: '/p/site1',
    obs: '/obs/site1_Flux.nc',
    kernelDir: '/kernels/default',
  });
  const row = (name: string, available: boolean) => ({
    name,
    label_zh: name,
    label_en: name,
    units: 'W/m²',
    available,
  });
  plan.resolve([row('Qle', true), row('Qh', true), row('NEE', false)]);
  await page.settle();
  const boxes = () =>
    page
      .nodes()
      .filter((node) => node.type === 'label' && node.props.className === 'param-check')
      .flatMap((node) => (node.children as Tree[]).filter((c) => c?.type === 'input'));
  assert.deepEqual(
    boxes().map((box) => box.props.checked),
    [true, false],
    'only comparable targets are offered; the default Qle is ticked',
  );
  (boxes()[1]!.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  const field = page
    .nodes()
    .find((node) => node.type === 'Input' && node.props.value === 'Qle, Qh');
  assert.ok(field, 'ticking Qh adds it to the target list');
});
it('defaults concurrency to the machine and population to the ticked parameters until edited', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  page.requests.at(-1)!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  page.requests
    .at(-1)!
    .resolve([0, 1, 2].map((i) => ({ ...parameterRow(), id: `t${i}`, name: `DEF_T${i}` })));
  await page.settle();
  const field = (id: string) =>
    (page.contract().fields as { id: string; value: string; set: (v: string) => void }[]).find(
      (f) => f.id === id,
    )!;
  const cores = globalThis.navigator?.hardwareConcurrency || 4;
  assert.equal(
    field('research-jobs').value,
    String(Math.min(16, Math.max(1, Math.floor(cores * 0.75)))),
  );
  assert.equal(field('population').value, '8');
  const tick = (name: string) => {
    const box = page.nodes().find((n) => n.type === 'input' && n.props['aria-label'] === name);
    (box!.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  };
  ['DEF_T0', 'DEF_T1', 'DEF_T2'].forEach(tick);
  await page.settle();
  assert.equal(field('population').value, '12', 'four sets per parameter');
  field('population').set('20');
  field('research-jobs').set('2');
  await page.settle();
  const untick = page.nodes().find((n) => n.type === 'input' && n.props['aria-label'] === 'DEF_T0');
  (untick!.props.onChange as (event: unknown) => void)({ target: { checked: false } });
  await page.settle();
  assert.equal(field('population').value, '20', 'an edited population stays');
  assert.equal(field('research-jobs').value, '2', 'an edited concurrency stays');
});
it('step 4 lists the run-and-results buttons on screen and a write-back dialog blocks them', async () => {
  const page = await openCompletedStudy();
  assert.deepEqual(page.ids(), [
    'research-export-panel',
    'research-home',
    'research-step-1',
    'research-step-2',
    'research-step-3',
    'research-step-4',
    'study-browse',
    'study-results',
    'study-status',
    'study-write-version',
  ]);
  // 导出按钮在合着的"导出研究"折叠区里：先登记折叠区标题，导出按钮此刻不列。
  assert.equal(
    page.actions().some((action) => action.id === 'study-export'),
    false,
  );
  // 已完成的本机研究没有可用的运行/暂停按钮，界面不渲染它们，旧清单却一直列着。
  assert.equal(
    page.actions().some((action) => action.id === 'study-run'),
    false,
  );
  page.press('study-write-version');
  assert.equal(page.requests.at(-1)!.command, 'model_create');
  page.requests.at(-1)!.resolve({ id: 'd1', name: 'Calibration s-1', generation: 3, overlay: {} });
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'model_import_study');
  page.requests.at(-1)!.resolve({ member: 'm1', parameters: 3 });
  await page.settle();
  assert.ok(page.nodes().some((node) => node.type === 'StudyImportDialog'));
  // 写回对话框打开时，研究页自己的按钮全被挡住（截图里的 StudyImportDialog 是版本页那批的文件，
  // 它里面的取消/写进草稿由它自己登记，不在这棵树里）。
  assert.deepEqual(page.actions(), []);
});
it('the home page lists one return-to-simulation action even though it shows two buttons', async () => {
  const page = await openCompletedStudy();
  page.press('research-home');
  page.state.simulationReturn = { route: 'case' };
  page.render();
  assert.equal(
    page.actions().filter((action) => action.id === 'research-return-simulation').length,
    1,
  );
  assert.ok(page.actions().some((action) => action.id === 'research-current'));
  page.press('research-return-simulation');
  assert.deepEqual(page.routes.at(-1), ['case', 'settings']);
});
