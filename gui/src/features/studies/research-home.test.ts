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
  budgetAdvice,
  calibrationWindows,
  candidateBudget,
  fitValidation,
  gridTargetDefaults,
  gridTargetSpec,
  memberVerdict,
  networkDefaults,
  networkSpec,
  parameterSpec,
  studyPermissions,
  calibrationSeed,
  deriveStudyDesign,
  frozenDesign,
  plannedCandidates,
  networkWeightCount,
  studyProgress,
  convergenceRows,
  nearOptimalRows,
  NEAR_OPTIMAL_MIN_MEMBERS,
  crossValidationRows,
  sensitivityRanking,
  sensitivityResponses,
  studyDiagnostics,
  studyRunOutcome,
  studyErrorText,
  suggestedRange,
  suggestedWindows,
  researchBaselineOf,
} from './study';
import { remoteStudyPermissions, remoteStudyReply, studyModelRevision } from './remote-study';
import {
  clearParameterContextMarker,
  parameterContextKey,
  sameParameterContextIdentity,
  shouldStartParameterContextLoad,
} from './parameter-context-cache';
import { homeResearchEntry, hubMethods } from '../versions/hub-methods';
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
  // 预览的产物类型由后端按成员识别；测试可以直接换掉这次预览的返回。
  const importReply = overrides.importReply as Record<string, unknown> | undefined;
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
      if (command === 'model_import_study')
        return {
          member: 'm000001',
          objective: 1,
          records: [],
          kind: 'parameters',
          asset_root: null,
          network: null,
          ...importReply,
        };
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

// 审计 P1-5：DE 训练出的网络以前没有写回通道（后端拒绝含网络权重的成员）。现在按产物类型
// 分流，确认后把网络资产目录交给版本页，冻结时发布进新版本。
const soilNetwork = {
  slot: 'soil_stress',
  features: ['beta_physics', 'root_saturation'],
  outputs: ['beta'],
  hidden: [4],
  normalize: true,
  weights: 13,
};
function confirmContext(reply: Record<string, unknown>, writing: Record<string, unknown>) {
  const calls: [string, Record<string, unknown>][] = [];
  const patches: Record<string, unknown>[] = [];
  const notices: string[] = [];
  const run = pageCallback('confirmWriteBack', {
    writing,
    task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      return reply;
    },
    setWriting: () => {},
    notify: (message: string) => notices.push(message),
    update: (patch: Record<string, unknown>) => patches.push(patch),
    navigate: () => true,
    tr: (zh: string) => zh,
    en: false,
  }) as unknown as (chosen: [string, string | null, number | null][]) => Promise<unknown>;
  return { run, calls, patches, notices };
}
it('a network trained by a study previews as a network artifact', async () => {
  const { context, written } = writeBackContext({
    importReply: { kind: 'network', member: 'm000003', records: [], network: soilNetwork },
  });
  await pageCallback('writeBack', context)();
  const preview = written[0] as {
    kind?: string;
    network?: { slot: string; weights: number } | null;
    records: unknown[];
  };
  assert.equal(preview.kind, 'network');
  assert.equal(preview.network?.slot, 'soil_stress');
  assert.equal(preview.network?.weights, 13);
  assert.deepEqual(preview.records, []);
});
it('confirming a network import passes no parameter filter and hands its asset root to the versions page', async () => {
  const { run, calls, patches, notices } = confirmContext(
    {
      draft: { id: 'model-9', name: 'PFT', generation: 3 },
      member: 'm000003',
      kind: 'network',
      parameters: 0,
      asset_root: '/assets/model-9/7',
      network: soilNetwork,
    },
    {
      study: '/p/.colm/studies/s-1',
      member: 'm000003',
      objective: 0.4,
      records: [],
      kind: 'network',
      network: soilNetwork,
      draft: { id: 'model-9', name: 'PFT', generation: 2, overlay: {} },
      created: false,
    },
  );
  await run([]);
  assert.equal(calls.length, 1);
  assert.equal(calls[0]![0], 'model_import_study');
  // 网络没有可勾选的参数：传空数组会被后端当成"勾选结果为空"而拒绝。
  assert.equal(calls[0]![1].only, null);
  assert.equal(patches.at(-1)!.openDraft, 'model-9');
  assert.equal(patches.at(-1)!.openDraftAssets, '/assets/model-9/7');
  assert.match(notices.at(-1)!, /soil_stress|网络/);
});
it('confirming a joint import filters only the ticked parameters and still carries the network', async () => {
  const { run, calls, patches } = confirmContext(
    {
      draft: { id: 'model-9', name: 'PFT', generation: 3 },
      member: 'm000003',
      kind: 'joint',
      parameters: 2,
      asset_root: '/assets/model-9/7',
      network: soilNetwork,
    },
    {
      study: '/p/.colm/studies/s-1',
      member: 'm000003',
      objective: 0.4,
      records: [],
      kind: 'joint',
      network: soilNetwork,
      draft: { id: 'model-9', name: 'PFT', generation: 2, overlay: {} },
      created: false,
    },
  );
  const chosen: [string, string | null, number | null][] = [['case:DEF_TUNING_CNFAC', null, null]];
  await run(chosen);
  assert.deepEqual(calls[0]![1].only, chosen);
  assert.equal(patches.at(-1)!.openDraftAssets, '/assets/model-9/7');
});
it('confirming a parameter import keeps the old request and carries no asset root', async () => {
  const { run, calls, patches } = confirmContext(
    {
      draft: { id: 'model-9', name: 'PFT', generation: 3 },
      member: 'm000001',
      kind: 'parameters',
      parameters: 1,
      asset_root: null,
      network: null,
    },
    {
      study: '/p/.colm/studies/s-1',
      member: 'm000001',
      objective: 0.5,
      records: [],
      kind: 'parameters',
      network: null,
      draft: { id: 'model-9', name: 'PFT', generation: 2, overlay: {} },
      created: false,
    },
  );
  const chosen: [string, string | null, number | null][] = [['case:DEF_TUNING_CNFAC', null, null]];
  await run(chosen);
  assert.deepEqual(calls[0]![1].only, chosen);
  assert.equal(patches.at(-1)!.openDraftAssets, null);
});
// 审计 P2-11：没有交叉验证的拟合以前也能冻结（validation:null 直接放行），存出来的版本
// 和验证过的看起来一样。现在允许保存（探索是合法用法），但名字/说明持续标"未验证实验"。
function saveFitContext(report: unknown) {
  const calls: [string, Record<string, unknown>][] = [];
  const notices: string[] = [];
  const run = pageCallback('saveFitVersion', {
    fit: {
      model: '/out/study.mlp.json',
      normalize: '/out/study.norm.json',
      network: 'study.mlp.json',
      networkSpec: {
        slot: 'pft',
        features: ['clim_tair'],
        outputs: [
          { name: 'DEF_PFT_VMAX25', range: [0.5, 2], transform: 'sigmoid', relative: true },
        ],
        hidden: [],
      },
      studyDirs: ['/p/.colm/studies/s-1'],
      sourceCase: '/p/site1',
      parent: { id: 'official', revision: 1 },
      report: { validation: report },
      report_file: '/out/report.json',
    },
    name: 'net',
    en: false,
    tr: (zh: string) => zh,
    fitValidation,
    task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'model_create') return { id: 'model-9' };
      if (command === 'model_attach_network')
        return { draft: { id: 'model-9' }, assetRoot: '/assets/9' };
      return { version: { id: 'model-9', revision: 1 } };
    },
    refreshVersions: async () => {},
    notify: (message: string) => notices.push(message),
    setFit: () => {},
  }) as unknown as () => Promise<unknown>;
  return { run, calls, notices };
}
it('a fit without cross-validation is saved as an explicitly unvalidated experiment', async () => {
  const { run, calls, notices } = saveFitContext(null);
  await run();
  const create = calls.find(([command]) => command === 'model_create')!;
  assert.match(String(create[1].name), /未验证/);
  const note = JSON.parse(String(create[1].note)) as { validation: unknown; unvalidated: boolean };
  assert.equal(note.validation, null);
  assert.equal(note.unvalidated, true);
  assert.match(notices.at(-1)!, /未验证|不能当作已验证结论/);
});
it('a failed cross-validation still blocks saving a fit', async () => {
  const { run, calls } = saveFitContext({ passed: false });
  await assert.rejects(run(), /交叉验证没通过/);
  assert.equal(
    calls.some(([command]) => command === 'model_create'),
    false,
  );
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
      resumeRequested: { current: false },
      runKernel: '/kernel',
      studyDir: '/s',
      envelope: { state: { status: 'Ready' } },
      confirmed: true,
      remotePermissions: { exists: false },
      canConfirmSubmission: true,
      tr: (zh: string) => zh,
      jobs: '1',
      // U-01：运行用的修订由“研究基线 + 研究自己记录的修订”决定，不再直接读全局运行版本。
      studyBaseline: { id: 'official', revision: 1 },
      studyModelRevision: () => ({ id: 'official', revision: 1 }),
      state: { versionRef: { id: 'official', revision: 1 } },
      remoteRequest: { current: 0 },
      setLaunching: () => {},
      setError: () => {},
      notify: () => {},
      studyRunOutcome,
      studyErrorText,
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
function researchPage(
  overrides: Record<string, unknown> = {},
  harness: { profiles?: Record<string, unknown>; timing?: Record<string, unknown> } = {},
) {
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
  // remote_config 与 model_case_profiles 是页面自己读的环境信息，不是动作。第 3 步会拿算例档案的
  // 模拟期去推荐时间窗：一直回空对象会让预填分支永远不执行，全链路测试就看不到"推荐窗口覆盖派生
  // 设计窗口"这一类回归。默认给一个真实形状的算例档案（2000–2010、已设预热），需要别的时段时用
  // harness.profiles / harness.timing 覆盖。read_timing 也在这里直接答，免得每个用例手动处理。
  const defaultProfile = {
    spatial: false,
    grid: null,
    subgrid: 'default',
    soil: 'default',
    urban: false,
    bgc: false,
    crop: false,
    kernel_preset: 'default',
    obs_file: null,
    inputs: {
      start: '2000-01-01',
      end: '2010-01-01',
      output_start: '2000-01-01',
      spinup_years: 3,
      timestep: 1800,
    },
  };
  const defaultTiming = {
    start: '2000-01-01',
    end: '2010-01-01',
    output_start: '2000-01-01',
    spinup_years: 3,
  };
  const automatic = { profiles: 0, timing: 0 };
  const invoke = (command: string, args: Record<string, unknown>) =>
    new Promise((resolve, reject) => {
      if (command === 'remote_config') return resolve({ servers: [] });
      if (command === 'model_case_profiles') {
        automatic.profiles++;
        const dirs = Array.isArray(args.dirs) ? (args.dirs as string[]) : [];
        return resolve(
          Object.fromEntries(dirs.map((dir) => [dir, harness.profiles?.[dir] ?? defaultProfile])),
        );
      }
      if (command === 'read_timing') {
        automatic.timing++;
        return resolve(harness.timing ?? defaultTiming);
      }
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
    budgetAdvice,
    calibrationWindows,
    candidateBudget,
    compatibleKernel,
    completedCandidates,
    fitValidation,
    gridTargetDefaults,
    gridTargetSpec,
    hubMethods,
    homeResearchEntry,
    researchBaselineOf,
    IGBP,
    invoke,
    invokeJson: invoke,
    memberVerdict,
    methodFromStudy,
    networkDefaults,
    networkSpec,
    parameterSpec,
    clearParameterContextMarker,
    parameterContextKey,
    sameParameterContextIdentity,
    shouldStartParameterContextLoad,
    PFT,
    remoteStudyPermissions,
    remoteStudyReply,
    REMOTE_CHECK_MS,
    researchMethods,
    resultFileLabel,
    shownResultFiles,
    studyModelRevision,
    studyPermissions,
    calibrationSeed,
    deriveStudyDesign,
    frozenDesign,
    plannedCandidates,
    networkWeightCount,
    studyProgress,
    convergenceRows,
    nearOptimalRows,
    NEAR_OPTIMAL_MIN_MEMBERS,
    crossValidationRows,
    sensitivityRanking,
    sensitivityResponses,
    studyDiagnostics,
    studyRunOutcome,
    studyErrorText,
    suggestedWindows,
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
    automatic,
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
// 合法的不确定性研究：目标的 from/to 是唯一时段，没有 validation_*（后端 spec.rs 明确拒绝
// uncertainty 带验证期）。页面第 3 步的推荐时段会按算例模拟期给 2000–2007 + 验证 2007–2010，
// 正好覆盖派生设计要守住的 2000–2010。
const sourceFrom = Date.UTC(2000, 0, 1) / 1000;
const sourceTo = Date.UTC(2010, 0, 1) / 1000;
type Page = ReturnType<typeof researchPage>;
function uncertaintySourceEnvelope(from = sourceFrom, to = sourceTo) {
  const parameter = (name: string) => ({
    name,
    parameter_id: `case:${name}`,
    scope_instance: { kind: 'case-scalar', scheme: null, index: null },
    sample_min: 0,
    sample_max: 1,
    scale: 'linear',
  });
  const target = (site: string) => ({
    key: `Qle@${site}`,
    site,
    variable: 'Qle',
    metric: 'nrmse',
    weight: 1,
    from,
    to,
    min_pairs: 20,
  });
  return {
    manifest: {
      id: 's-uq',
      provenance: { spec_sha256: 'a'.repeat(64) },
      spec: {
        kind: 'uncertainty',
        method: 'morris',
        kernel_dir: '/kernels/default',
        base_cases: ['site1', 'site2'],
        observations: { site1: '/obs/site1_Flux.nc', site2: '/obs/site2_Flux.nc' },
        site_mode: 'shared',
        parameters: [parameter('DEF_A'), parameter('DEF_B')],
        targets: [target('site1'), target('site2')],
      },
    },
    state: { status: 'Completed', best_member: 'm1', candidates: { m1: { calibration: 0.1 } } },
    results: ['importance.json'],
    model_version: { model_revision: { version: { id: 'trace', revision: 7 } } },
    remote: null,
  };
}
const sensitivityRows = () =>
  ['DEF_A', 'DEF_B'].map((parameter, i) => ({
    site: null,
    variable: 'objective',
    parameter,
    method: 'morris',
    value: 0.3 - i * 0.1,
    mu: -0.1,
    sigma: 0.05,
    n: 10,
  }));
async function openUncertaintyStudy(page: Page, envelope = uncertaintySourceEnvelope()) {
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-uq');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  page.requests.at(-1)!.resolve(envelope);
  await page.settle();
  const importance = page.requests
    .filter((request) => request.command === 'study_result')
    .find((request) => request.args.path === 'importance.json');
  assert.ok(importance, 'the source sensitivity ranking is read');
  importance.resolve(JSON.stringify(sensitivityRows()));
  await page.settle();
}
// 带入率定之后页面处在第 2 步：先按派生出的两个站点重新读参数，再读参数目录。
async function carryDerivedStudy(page: Page, since: number) {
  const carry = page.actions().find((action) => action.id === 'study-carry-calibration');
  assert.ok(carry, 'the sensitivity ranking offers the carry action');
  page.press('study-carry-calibration');
  await page.settle();
  for (const dir of ['/p/site1', '/p/site2']) {
    const load = page.requests
      .slice(since)
      .filter((request) => request.command === 'model_case_context')
      .at(-1);
    assert.ok(load, `parameters are read for ${dir}`);
    assert.equal(load.args.dir, dir);
    load.resolve({ entry: { dir } });
    await page.settle();
  }
  const read = page.requests
    .slice(since)
    .find((request) => request.command === 'study_parameter_contexts');
  assert.ok(read, 'both derived sites are read in one parameter request');
  read.resolve(['DEF_A', 'DEF_B'].map((name) => ({ ...parameterRow(), id: `case:${name}`, name })));
  await page.settle();
}
function windowField(page: Page, id: string) {
  const field = (page.contract().fields as { id: string; value: string }[]).find(
    (item) => item.id === id,
  )!;
  assert.ok(field, `${id} is on screen`);
  return field;
}
// 不确定性分析第 3 步的时间框没有 PageContract id（只有"分析时段开始/结束"两个 datetime-local），
// 直接读渲染出来的输入值。
function localWindowValues(page: Page) {
  return page
    .nodes()
    .filter((node) => node.type === 'Input' && node.props.type === 'datetime-local')
    .map((node) => String(node.props.value ?? ''));
}
// 从派生的第 2 步走到第 3 步，核对并创建研究，返回真正交给后端的规格。
async function createDerivedStudy(page: Page) {
  page.press('research-step-3');
  await page.settle();
  assert.equal(windowField(page, 'training-from').value, '2000-01-01T00:00');
  assert.equal(windowField(page, 'training-to').value, '2010-01-01T00:00');
  assert.equal(windowField(page, 'validation-from').value, '');
  assert.equal(windowField(page, 'validation-to').value, '');
  const confirm = page
    .nodes()
    .find((node) => node.type === 'label' && node.props.className === 'param-check study-confirm')
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
  assert.ok(confirm);
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  page.press('study-create');
  await page.settle();
  const preflight = page.requests
    .filter((request) => request.command === 'study_preflight_json')
    .at(-1);
  assert.ok(preflight, 'create preflights the derived design first');
  preflight.resolve({});
  await page.settle();
  const created = page.requests.filter((request) => request.command === 'study_create_json').at(-1);
  assert.ok(created, 'the derived study is created');
  return JSON.parse(String(created.args.specJson)) as {
    targets: {
      site: string;
      from: number;
      to: number;
      validation_from: number | null;
      validation_to: number | null;
    }[];
  };
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
  // 第 3 步还有别的勾选框（订正观测），核对项在 study-confirm 那一行里。
  const confirm = page
    .nodes()
    .find((node) => node.type === 'label' && node.props.className === 'param-check study-confirm')
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
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
  assert.equal(field('population').value, '15', 'five sets per parameter');
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
// P1 复现：服务器记录已可提交（recovery.permissions.submit）时，运行按钮按 submit 放行、第 4 步的
// 核对框却只看 resubmit，于是页面提示"勾选上面"但上面根本没有框。框、按钮和 start() 必须共用
// canConfirmSubmission 这一个判断。
it('a server-submittable study shows the budget review box and unlocks the run button', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-submit');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  const envelope = {
    manifest: {
      id: 's-submit',
      spec: {
        kind: 'tuning',
        kernel_dir: '/kernels/default',
        parameters: [{ name: 'DEF_TEST', sample_min: 0, sample_max: 10, scale: 'linear' }],
      },
    },
    state: { status: 'Ready', candidates: {} },
    results: [],
    model_version: { model_revision: { version: { id: 'official', revision: 1 } } },
    remote: { host: 'cluster-a', job: 'job-7' },
  };
  page.requests.at(-1)!.resolve(envelope);
  await page.settle();
  page.press('remote-study-status');
  await page.settle();
  page.requests
    .filter((request) => request.command === 'study_status')
    .at(-1)!
    .resolve(envelope);
  await page.settle();
  page.requests
    .filter((request) => request.command === 'remote_study_status')
    .at(-1)!
    .resolve({
      host: 'cluster-a',
      job: 'job-7',
      status: { state: 'failed', exit_code: 1 },
      recovery: {
        outcome: 'build_failed',
        terminal: true,
        finished: false,
        study_created: false,
        fetched: false,
        permissions: {
          submit: true,
          resubmit: false,
          cancel: false,
          fetch: false,
          close: true,
          poll: false,
        },
        next_zh: '修复后可以重新提交',
        next_en: 'Fix the build and submit again',
      },
      warnings: [],
    });
  await page.settle();
  const run = () => page.actions().find((action) => action.id === 'study-run');
  assert.ok(run(), 'the run button is offered for a submittable server record');
  assert.equal(run()!.enabled, false, 'it waits for the review box');
  assert.match(pageText(page), /先勾选上面/, 'the page explains why the run button is disabled');
  const confirm = page
    .nodes()
    .find((node) => node.type === 'label' && node.props.className === 'param-check study-confirm')
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
  assert.ok(confirm, 'the review box the hint points at must actually render');
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  assert.equal(run()!.enabled, true, 'checking the box unlocks the submit run');
});
it('switching the model revision while a study runs keeps its frozen design and identity', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-run');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  page.requests.at(-1)!.resolve({
    manifest: {
      id: 's-run',
      spec: {
        kind: 'tuning',
        kernel_dir: '/kernels/default',
        parameters: [{ name: 'DEF_TEST', sample_min: 0, sample_max: 10, scale: 'linear' }],
      },
    },
    state: { status: 'Ready' },
    results: [],
    model_version: { model_revision: { version: { id: 'official', revision: 1 } } },
    remote: null,
  });
  await page.settle();
  // 研究还没跑：勾上冻结预算的核对项，点“开始运行”（runLock 置位）。
  const confirm = page
    .nodes()
    .find(
      (node) =>
        node.type === 'label' && String(node.props.className ?? '').includes('study-confirm'),
    )
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
  assert.ok(confirm);
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  page.press('study-run');
  assert.equal(page.requests.at(-1)!.command, 'study_run');
  // 运行请求还在飞时切模型版本：正在跑的研究要留着它冻结的输入（manifest/spec）和任务目录。
  page.state.versionRef = { id: 'official', revision: 2 };
  page.render();
  await page.settle();
  const stillOpen = () =>
    page.actions().some((action) => action.id === 'study-status' && action.enabled);
  assert.equal(stillOpen(), true, 'the running study is still open');
  const texts = JSON.stringify(page.nodes().map((node) => node.children));
  assert.match(texts, /DEF_TEST/, 'the frozen parameter design is still shown');
  assert.equal(
    page.requests.filter((r) => r.command === 'model_case_context').length,
    0,
    'a running study does not restart the parameter read',
  );
  page.requests.at(-1)!.resolve({});
  await page.settle();
  assert.equal(stillOpen(), true, 'the study identity survives the revision switch');
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
it('a member that does not beat the baseline is written back only after an explicit override', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('study-browse');
  page.requests.at(-1)!.resolve('/study');
  await page.settle();
  const envelope = completedEnvelope();
  envelope.state.candidates = { m1: { calibration: 0.3 }, m000000: { calibration: 0.2 } };
  (envelope.state as { warnings?: string[] }).warnings = [
    'bound: DEF_TEST upper 1.49',
    'm1 / site1 validation unavailable: x',
  ];
  page.requests.at(-1)!.resolve(envelope);
  await page.settle();
  const write = () => page.actions().find((action) => action.id === 'study-write-version')!;
  assert.equal(write().enabled, false);
  const texts = JSON.stringify(page.nodes().map((node) => node.children));
  assert.match(texts, /不比默认参数好/);
  assert.match(texts, /DEF_TEST 的最优值 1.49 贴着范围上限/);
  assert.doesNotMatch(texts, /validation unavailable/, 'per-member notes stay in the report');
  const override = page
    .nodes()
    .find(
      (node) =>
        node.type === 'input' &&
        node.props.type === 'checkbox' &&
        node.props.checked === false &&
        typeof node.props.onChange === 'function',
    );
  assert.ok(override);
  (override.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  assert.equal(write().enabled, true);
});
it('prefilled windows follow each case spin-up and are recomputed when the cases change', async () => {
  const filled: Record<string, string> = { from: '', to: '', valFrom: '', valTo: '' };
  const setter = (key: string) => (value: (old: string) => string) => {
    filled[key] = value(filled[key]!);
  };
  let note: unknown = null;
  const autoWindows = { current: { from: '', to: '', valFrom: '', valTo: '' } };
  type Run = (period: [string, string] | null, live: () => boolean) => Promise<void>;
  const prefill = (invoke: (command: string, args: unknown) => Promise<unknown>) =>
    pageCallback('prefillWindows', {
      tuning: true,
      sites: ['/p/site1'],
      invoke,
      suggestedWindows,
      autoWindows,
      setFrom: setter('from'),
      setTo: setter('to'),
      setValFrom: setter('valFrom'),
      setValTo: setter('valTo'),
      setCasePeriod: () => {},
      setWindowNote: (value: unknown) => (note = value),
    }) as unknown as Run;
  const timing = (end: string) => async (command: string) => {
    assert.equal(command, 'read_timing');
    return { start: '2002-01-01', end, output_start: '2002-01-01', spinup_years: 0 };
  };
  await prefill(timing('2012-12-31'))(['2002-01-01', '2012-12-31'], () => true);
  assert.deepEqual(filled, {
    from: '2003-01-01T00:00',
    to: '2010-01-01T00:00',
    valFrom: '2010-01-01T00:00',
    valTo: '2012-12-31T00:00',
  });
  assert.match(String((note as string[])[0]), /从第二年开始/);
  // 换成更短的算例：自动填的验证期跟着重算，用户改过的率定期开始不动。
  filled.from = '2004-06-01T00:00';
  await prefill(timing('2006-12-31'))(['2002-01-01', '2006-12-31'], () => true);
  assert.deepEqual(filled, {
    from: '2004-06-01T00:00',
    to: '2006-01-01T00:00',
    valFrom: '2006-01-01T00:00',
    valTo: '2006-12-31T00:00',
  });
  // 读不到预热设置：退回整段模拟期，不留验证期；没有共同模拟期：清掉自动填的值。
  filled.from = '';
  await prefill(async () => {
    throw new Error('no case.nml');
  })(['2002-01-01', '2012-12-31'], () => true);
  assert.deepEqual(filled, {
    from: '2002-01-01T00:00',
    to: '2012-12-31T00:00',
    valFrom: '',
    valTo: '',
  });
  assert.equal(note, null);
  await prefill(timing('2012-12-31'))(null, () => true);
  assert.deepEqual(filled, { from: '', to: '', valFrom: '', valTo: '' });
});
it('cancelling a study asks first and does nothing when declined', async () => {
  for (const answer of [false, true]) {
    const calls: string[] = [];
    const control = pageCallback('control', {
      remotePermissions: { exists: false },
      status: 'Running',
      tr: (zh: string) => zh,
      askConfirm: async (message: string) => {
        assert.match(message, /不能重试/);
        return answer;
      },
      epoch: { current: 1 },
      task: async (fn: (current: () => boolean) => Promise<unknown>) => await fn(() => true),
      invoke: async (command: string) => {
        calls.push(command);
        return {};
      },
      studyDir: '/s',
      readStatus: async () => ({}),
      notify: () => {},
    }) as unknown as (command: string) => Promise<unknown>;
    const value = await control('study_cancel').catch((e: unknown) => e);
    if (!answer) {
      assert.equal(value, null);
      assert.deepEqual(calls, []);
    } else assert.deepEqual(calls.slice(0, 1), ['study_cancel']);
  }
});
it('carrying sensitivity into calibration pins the source study identity and inherits its design', () => {
  const calls: Record<string, unknown> = {};
  const set = (name: string) => (value: unknown) => {
    calls[name] = value;
  };
  const loadedFor = { current: 'old' };
  const failedFor = { current: 'failed' };
  const applyingDerivation = { current: false };
  const prefilledFor = { current: 'old-key' };
  const autoWindows = {
    current: {
      from: '2000-01-01T00:00',
      to: '2010-12-31T00:00',
      valFrom: '',
      valTo: '',
    },
  };
  const revision = { id: 'trace', revision: 7 };
  const specSha256 = 'a'.repeat(64);
  const context = (overrides: Record<string, unknown> = {}) => ({
    frozenSpec: {
      kind: 'uncertainty',
      method: 'morris',
      base_cases: ['AT-Neu'],
      observations: { 'AT-Neu': '/obs/AT-Neu_Flux.nc' },
      targets: [
        {
          key: 'Qle@AT-Neu',
          site: 'AT-Neu',
          variable: 'Qle',
          metric: 'kge',
          weight: 2,
          from: 1_104_537_600,
          to: 1_136_073_600,
          min_pairs: 12,
          aggregate: 'monthly',
          corrected: true,
        },
      ],
      parameters: [
        {
          name: 'DEF_TUNING_TRSMX0',
          parameter_id: 'case:DEF_TUNING_TRSMX0',
          sample_min: 1e-4,
          sample_max: 3e-4,
          scale: 'log',
        },
        {
          name: 'DEF_TUNING_CNFAC',
          parameter_id: 'case:DEF_TUNING_CNFAC',
          sample_min: 0.1,
          sample_max: 0.9,
          scale: 'linear',
        },
      ],
    },
    carryKeys: ['DEF_TUNING_TRSMX0'],
    studyDir: '/p/.colm/studies/s-abc',
    envelope: {
      manifest: { provenance: { spec_sha256: specSha256 } },
      model_version: { model_revision: { version: revision } },
    },
    modelUsed: revision,
    state: { kernels: [{ dir: '/kernels/default' }] },
    calibrationSeed,
    deriveStudyDesign,
    tr: (zh: string) => zh,
    chooseMethod: set('method'),
    setSites: set('sites'),
    setVariables: set('variables'),
    setMetric: set('metric'),
    setMinPairs: set('minPairs'),
    setCorrected: set('corrected'),
    setAggregate: set('aggregate'),
    setWeights: set('weights'),
    setObservations: set('observations'),
    setFrom: set('from'),
    setTo: set('to'),
    setValFrom: set('valFrom'),
    setValTo: set('valTo'),
    setGridTargets: set('gridTargets'),
    setSeed: set('seed'),
    setKernel: set('kernel'),
    setDerivedDesign: set('derived'),
    loadedFor,
    failedFor,
    prefilledFor,
    autoWindows,
    // 当前研究基线（official r1）与派生要钉的修订（trace r7）不同：carry 要给模型身份 effect 留标记。
    versionId: 'official',
    revision: 1,
    applyingDerivation,
    update: set('update'),
    setStep: set('step'),
    ...overrides,
  });
  const carry = pageCallback('carryToCalibration', context()) as unknown as () => void;
  carry();
  assert.equal(calls.method, 'tuning');
  assert.deepEqual(calls.sites, ['/p/AT-Neu']);
  assert.equal(calls.variables, 'Qle');
  assert.equal(calls.metric, 'kge');
  assert.equal(calls.minPairs, '12');
  assert.equal(calls.corrected, true);
  assert.equal(calls.aggregate, 'monthly', '月均聚合必须从来源研究继承');
  assert.deepEqual(calls.weights, { Qle: '2' });
  assert.equal(calls.observations, 'AT-Neu | /obs/AT-Neu_Flux.nc');
  assert.equal(calls.from, '2005-01-01T00:00');
  assert.equal(calls.to, '2006-01-01T00:00');
  // 来源 UQ 没有验证期：派生写回的验证期必须是空串（明确设计值），不能沿用页面上一轮自动填的值。
  assert.equal(calls.valFrom, '');
  assert.equal(calls.valTo, '');
  assert.equal(loadedFor.current, '', 'step 2 reads the parameters again and applies the seed');
  assert.equal(failedFor.current, '', 'a carried seed can retry a read that failed before');
  assert.equal(prefilledFor.current, '', 'the derived windows are not treated as auto-filled');
  assert.deepEqual(
    autoWindows.current,
    { from: '', to: '', valFrom: '', valTo: '' },
    'the next prefill cannot mistake the derived windows for its own',
  );
  const update = calls.update as {
    researchSeed: { parameters: { parameter_id: string; scale: string }[] };
    researchBaseline: { id: string; revision: number };
  };
  // 模型修订钉在来源研究实际用的那一版：原来这里回落成全局当前版本（审计复现 trace r7 → official r1）。
  assert.deepEqual(update.researchBaseline, revision);
  assert.deepEqual(
    update.researchSeed.parameters.map((p) => [p.parameter_id, p.scale]),
    [['case:DEF_TUNING_TRSMX0', 'log']],
  );
  const derived = calls.derived as { parent: { study: string; spec_sha256: string } };
  assert.equal(derived.parent.study, '/p/.colm/studies/s-abc');
  assert.equal(derived.parent.spec_sha256, specSha256);
  assert.equal(calls.step, 2);
  // 研究基线真的变了才置标记：effect 看到它只作废参数缓存，不能清掉刚写入的 sites/observations。
  assert.equal(applyingDerivation.current, true, '派生引起的模型身份变化要标记给 effect');
});
it('carrying refuses a source study whose model revision or design hash was never recorded', () => {
  const failedFor = { current: '' };
  const carry = pageCallback('carryToCalibration', {
    frozenSpec: {
      kind: 'uncertainty',
      base_cases: ['AT-Neu'],
      observations: { 'AT-Neu': '/obs/AT-Neu_Flux.nc' },
      targets: [],
      parameters: [{ name: 'P', parameter_id: 'p', sample_min: 0, sample_max: 1, scale: 'linear' }],
    },
    carryKeys: ['P'],
    studyDir: '/p/.colm/studies/s-abc',
    envelope: { manifest: {} },
    modelUsed: null,
    state: { kernels: [] },
    calibrationSeed,
    deriveStudyDesign,
    tr: (zh: string) => zh,
    chooseMethod: () => {
      throw new Error('must not switch method before the derivation is valid');
    },
    setSites: () => {},
    setVariables: () => {},
    setMetric: () => {},
    setMinPairs: () => {},
    setCorrected: () => {},
    setAggregate: () => {},
    setWeights: () => {},
    setObservations: () => {},
    setFrom: () => {},
    setTo: () => {},
    setValFrom: () => {},
    setValTo: () => {},
    setGridTargets: () => {},
    setSeed: () => {},
    setKernel: () => {},
    setDerivedDesign: () => {},
    loadedFor: { current: '' },
    failedFor,
    update: () => {},
    setStep: () => {},
  }) as unknown as () => void;
  assert.throws(carry, /模型修订|model revision/);
});
// P1 回归：带入率定会在同一个页面里把研究基线钉到来源研究自己的修订上。研究基线一变，模型身份
// effect 原来会执行"换版本就回落到当前算例、清空观测"的旧规则，把 deriveStudyDesign 刚写进来的
// 两站点算例与观测一起清掉。这里走完整链路（打开研究 → 带入率定 → 重跑所有 effects → 读参数 →
// 建研究），断言最终交给 study_create_json 的设计仍是两站点、两观测、trace@7。
it('carrying sensitivity into calibration keeps its derived sites and creates that study', async () => {
  const specSha256 = 'a'.repeat(64);
  const frozenParameter = (name: string) => ({
    name,
    parameter_id: `case:${name}`,
    scope_instance: { kind: 'case-scalar', scheme: null, index: null },
    sample_min: 0,
    sample_max: 1,
    scale: 'linear',
  });
  const target = (site: string) => ({
    key: `Qle@${site}`,
    site,
    variable: 'Qle',
    metric: 'nrmse',
    weight: 1,
    from: 1_104_537_600,
    to: 1_136_073_600,
    min_pairs: 20,
  });
  const page = researchPage({
    versionRef: { id: 'official', revision: 1 },
    versions: [
      { version: { id: 'official', revision: 1 }, name: 'Official CoLM' },
      { version: { id: 'trace', revision: 7 }, name: 'Trace scheme' },
    ],
  });
  page.render();
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-carry');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  page.requests.at(-1)!.resolve({
    manifest: {
      id: 's-carry',
      provenance: { spec_sha256: specSha256 },
      spec: {
        kind: 'uncertainty',
        method: 'morris',
        kernel_dir: '/kernels/default',
        base_cases: ['site1', 'site2'],
        observations: {
          site1: '/obs/site1_Flux.nc',
          site2: '/obs/site2_Flux.nc',
        },
        site_mode: 'shared',
        parameters: [frozenParameter('DEF_A'), frozenParameter('DEF_B')],
        targets: [target('site1'), target('site2')],
      },
    },
    state: { status: 'Completed', best_member: 'm1', candidates: { m1: { calibration: 0.1 } } },
    results: ['importance.json'],
    model_version: { model_revision: { version: { id: 'trace', revision: 7 } } },
    remote: null,
  });
  await page.settle();
  const importance = page.requests.find((request) => request.command === 'study_result');
  assert.ok(importance, 'the finished uncertainty study reads its sensitivity ranking');
  importance!.resolve(
    JSON.stringify(
      ['DEF_A', 'DEF_B'].map((parameter, i) => ({
        site: null,
        variable: 'objective',
        parameter,
        method: 'morris',
        value: 0.3 - i * 0.1,
        mu: -0.1,
        sigma: 0.05,
        n: 10,
      })),
    ),
  );
  await page.settle();
  const carry = page.actions().find((action) => action.id === 'study-carry-calibration');
  assert.ok(carry, 'with a sensitivity ranking the carry action is offered');
  page.press('study-carry-calibration');
  await page.settle();
  // 模型身份换成 trace@7 之后，参数要按新修订、两站点重读；读不出来就说明派生出的站点被清掉了。
  const loads = () => page.requests.filter((request) => request.command === 'model_case_context');
  assert.deepEqual(
    loads().map((request) => request.args.dir),
    ['/p/site1'],
    'the derived study reads parameters for the first site',
  );
  assert.deepEqual(loads()[0]!.args.modelRevision, { id: 'trace', revision: 7 });
  loads()[0]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.deepEqual(
    loads().map((request) => request.args.dir),
    ['/p/site1', '/p/site2'],
    'the second derived site is still there',
  );
  loads()[1]!.resolve({ entry: { dir: '/p/site2' } });
  await page.settle();
  const read = page.requests.find((request) => request.command === 'study_parameter_contexts');
  assert.ok(read, 'both derived sites are read in one parameter request');
  assert.deepEqual(read!.args.dirs, ['/p/site1', '/p/site2']);
  read!.resolve(
    ['DEF_A', 'DEF_B'].map((name) => ({ ...parameterRow(), id: `case:${name}`, name })),
  );
  await page.settle();
  page.press('research-step-3');
  await page.settle();
  const observations = page
    .nodes()
    .filter(
      (node) => node.type === 'Input' && String(node.props.placeholder ?? '').includes('*_Flux.nc'),
    )
    .map((node) => node.props.value);
  assert.deepEqual(
    observations,
    ['/obs/site1_Flux.nc', '/obs/site2_Flux.nc'],
    'the derived observation files must survive the model-identity effect',
  );
  const confirm = page
    .nodes()
    .find((node) => node.type === 'label' && node.props.className === 'param-check study-confirm')
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
  assert.ok(confirm);
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  page.press('study-create');
  await page.settle();
  const preflight = page.requests.find((request) => request.command === 'study_preflight_json');
  assert.ok(preflight, 'create preflights the derived design first');
  preflight!.resolve({});
  await page.settle();
  const created = page.requests.find((request) => request.command === 'study_create_json');
  assert.ok(created, 'the derived study is created instead of failing on a cleared design');
  assert.deepEqual(created!.args.modelRevision, { id: 'trace', revision: 7 });
  const spec = JSON.parse(String(created!.args.specJson)) as {
    base_cases: string[];
    observations: Record<string, string>;
    targets: { site: string; variable: string; weight: number }[];
  };
  assert.deepEqual(spec.base_cases, ['/p/site1', '/p/site2']);
  assert.deepEqual(spec.observations, {
    site1: '/obs/site1_Flux.nc',
    site2: '/obs/site2_Flux.nc',
  });
  assert.deepEqual(
    spec.targets.map((row) => [row.site, row.variable, row.weight]),
    [
      ['site1', 'Qle', 1],
      ['site2', 'Qle', 1],
    ],
  );
});

// P1 复现：合法 UQ 来源（2000–2010，无验证期）带入率定后，第 3 步的普通自动预填不能按算例模拟期
// 把窗口改成 2000–2007 + 验证 2007–2010。来源没有验证期是明确的设计值，不是"空着等推荐"。
it('carrying a legal UQ study into calibration keeps its 2000–2010 window unchanged', async () => {
  const page = researchPage();
  page.render();
  await openUncertaintyStudy(page);
  const since = page.requests.length;
  await carryDerivedStudy(page, since);
  const spec = await createDerivedStudy(page);
  assert.equal(page.automatic.timing, 0, 'derived mode skips the ordinary auto prefill');
  assert.equal(spec.targets.length, 2);
  for (const target of spec.targets) {
    assert.equal(target.from, sourceFrom);
    assert.equal(target.to, sourceTo);
    assert.equal(target.validation_from, null, 'the source has no validation window');
    assert.equal(target.validation_to, null);
  }
});
// 第二条复现：页面先做过一项 UQ 研究并自动填过 2000–2010，再从这个来源带入率定。carry 要清掉
// prefilledFor/autoWindows，派生模式不再推荐，最终窗口仍是来源研究的。
it('a derived study keeps the source window after the page auto-filled the same UQ period', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-uncertainty');
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
  // 预热已设置，UQ 的自动预填正好是整个 2000–2010（不扣第一年、不留验证期）。
  assert.equal(page.automatic.timing, 1);
  assert.deepEqual(localWindowValues(page), ['2000-01-01T00:00', '2010-01-01T00:00']);
  await openUncertaintyStudy(page);
  const since = page.requests.length;
  await carryDerivedStudy(page, since);
  // 打开来源研究时页面还没离开第 3 步，模型身份变化可能又自动填过一次；记下此刻的计数，
  // 断言派生后进第 3 步不再增加。
  const prefills = page.automatic.timing;
  assert.ok(prefills >= 1);
  const spec = await createDerivedStudy(page);
  assert.equal(
    page.automatic.timing,
    prefills,
    'carry resets the prefill and derived mode does not re-suggest',
  );
  for (const target of spec.targets) {
    assert.equal(target.from, sourceFrom);
    assert.equal(target.to, sourceTo);
    assert.equal(target.validation_from, null);
    assert.equal(target.validation_to, null);
  }
});

it('a finished uncertainty study shows its sensitivity ranking with the top parameters ticked', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-uncertainty');
  page.press('study-browse');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-u');
  await page.settle();
  page.requests.at(-1)!.resolve({
    manifest: {
      id: 's-u',
      spec: {
        kind: 'uncertainty',
        method: 'morris',
        kernel_dir: '/kernels/default',
        base_cases: ['site1'],
        parameters: [
          { name: 'DEF_A', sample_min: 0, sample_max: 1, scale: 'linear' },
          { name: 'DEF_B', sample_min: 0, sample_max: 1, scale: 'linear' },
          { name: 'DEF_C', sample_min: 0, sample_max: 1, scale: 'linear' },
          { name: 'DEF_D', sample_min: 0, sample_max: 1, scale: 'linear' },
        ],
      },
    },
    state: { status: 'Completed', candidates: {} },
    results: ['importance.json', 'members.csv'],
    remote: null,
  });
  await page.settle();
  const read = page.requests.find((r) => r.command === 'study_result');
  assert.ok(read, 'importance.json is read without picking it from the file list');
  assert.equal(read.args.path, 'importance.json');
  read.resolve(
    JSON.stringify(
      ['DEF_A', 'DEF_B', 'DEF_C', 'DEF_D'].map((parameter, i) => ({
        site: null,
        variable: 'objective',
        parameter,
        method: 'morris',
        value: [0.3, 0, 0.1, 0.2][i],
        mu: -0.1,
        sigma: 0.05,
        n: 10,
      })),
    ),
  );
  await page.settle();
  const carry = page.actions().find((a) => a.id === 'study-carry-calibration');
  assert.ok(carry);
  assert.equal(carry.enabled, true);
  assert.match(carry.label, /3/);
  const ticked = page
    .nodes()
    .filter(
      (n) =>
        n.type === 'input' &&
        n.props.checked === true &&
        String(n.props['aria-label']).startsWith('DEF_'),
    )
    .map((n) => n.props['aria-label']);
  assert.deepEqual(ticked, ['DEF_A', 'DEF_D', 'DEF_C'], 'zero-impact DEF_B is left out');
});
it('an uncertainty study keeps the time aggregation of gridded station targets', () => {
  // 审计复现：同一组"区域通量站 + 月均"在率定分支传了 aggregate，在不确定性分支没传，
  // 于是界面选月均、冻结规格里却是逐时比较——目标函数被悄悄换掉。
  const build = pageCallback('spec', {
    sites: ['/p/region'],
    kernel: '/kernels/grid',
    confirmed: true,
    tr: (zh: string) => zh,
    contexts: [parameterRow()],
    parameters: { 0: { enabled: true, min: '0', max: '1', scale: 'linear' } },
    parameterSpec,
    ai: false,
    trainNetwork: false,
    tab: 'uncertainty',
    tuning: false,
    candidateBudget,
    method: 'lhs',
    count: '8',
    population: '8',
    generations: '2',
    jobs: '1',
    seed: '1',
    variables: 'Qle',
    outputs: 'f_lfevpa',
    spatialStudy: true,
    gridTargets: [
      {
        kind: 'station',
        variable: 'Qle',
        path: '/obs/AT-Neu_Flux.nc',
        obsVariable: '',
        lon: '11.2',
        lat: '47.1',
        scale: '1',
        utcOffset: '8',
        period: '',
        metric: 'kge',
        weight: '1',
      },
    ],
    corrected: true,
    fluxCorrected: (variable: string) => ['qle', 'qh'].includes(variable.trim().toLowerCase()),
    aggregate: 'monthly',
    scoreObjective: true,
    calibrationWindows,
    from: '2003-01-01T00:00',
    to: '2004-01-01T00:00',
    minPairs: '12',
    metric: 'nrmse',
    gridTargetSpec,
    network: () => ({}),
    derivedDesign: null,
  }) as unknown as (forCreate: boolean) => { specJson: string };
  const plan = build(false);
  const spec = JSON.parse(plan.specJson) as {
    kind: string;
    targets: { variable: string; metric: string; aggregate?: string; corrected?: boolean }[];
  };
  assert.equal(spec.kind, 'uncertainty');
  assert.equal(spec.targets.length, 1);
  assert.equal(spec.targets[0]!.aggregate, 'monthly', '不确定性目标也要带 aggregate');
  assert.equal(spec.targets[0]!.corrected, true);
  assert.equal(spec.targets[0]!.metric, 'kge');
});

it('a parameter read cut short by another design change lets step 2 read again', async () => {
  const requested = {
    version: { id: 'official', revision: 1 },
    kernel: '/kernel',
    sites: ['/p/AT-Neu'],
    spatial: false,
    types: [],
  };
  // 敏感性“带入率定”会在同一次更新里换掉算例：请求发出时的身份已经不是当前身份。
  const loadedFor = { current: parameterContextKey(requested) };
  const load = pageCallback('loadContexts', {
    task: async (fn: (current: () => boolean) => Promise<unknown>) => await fn(() => false),
    parameterContextKey,
    sameParameterContextIdentity,
    clearParameterContextMarker,
    contextIdentityRef: { current: { ...requested, sites: ['/p/other'] } },
    loadedFor,
    effectiveCase: async (path: string) => path,
  }) as unknown as (identity: unknown) => Promise<unknown>;
  await load(requested);
  assert.equal(loadedFor.current, '', 'the stale read clears the marker so the effect reads again');
});
it('a parameter read that finished under an older identity never reaches the page', async () => {
  const requested = {
    version: { id: 'official', revision: 1 },
    kernel: '/kernel',
    sites: ['/p/AT-Neu'],
    spatial: true,
    types: [1],
  };
  // 请求在飞的时候用户又勾了另一个地类：旧身份读出的行不能写进新选择。
  const loadedFor = { current: parameterContextKey(requested) };
  const load = pageCallback('loadContexts', {
    task: async (fn: (current: () => boolean) => Promise<unknown>) => await fn(() => true),
    parameterContextKey,
    sameParameterContextIdentity,
    clearParameterContextMarker,
    contextIdentityRef: { current: { ...requested, types: [2] } },
    loadedFor,
    effectiveCase: async (path: string) => `${path}@effective`,
    invokeJson: async () => {
      throw new Error('a stale identity must not reach the parameter read');
    },
  }) as unknown as (identity: unknown) => Promise<unknown>;
  await load(requested);
  assert.equal(loadedFor.current, '', 'the old identity releases its marker');
});
it('a failed parameter read is retried instead of being skipped forever', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  const loads = () => page.requests.filter((r) => r.command === 'model_case_context');
  const reads = () => page.requests.filter((r) => r.command === 'study_parameter_contexts');
  assert.equal(loads().length, 1);
  loads()[0]!.reject(new Error('case.nml went missing'));
  await page.settle();
  // 失败后 busy 翻回 false 会再触发自动读取 effect；失败身份被记下，不能立刻重试成死循环。
  assert.equal(loads().length, 1, 'a failed read is not retried in a loop');
  const text = JSON.stringify(page.nodes().map((node) => node.children));
  assert.match(text, /case\.nml went missing/, 'the failure stays visible');
  assert.match(text, /重新读取|Load again/, 'the page offers a visible retry');
  // 按钮“重新读取”真的能再试一次（同算例、同配置、同修订）。
  page.press('load-parameter-contexts');
  await page.settle();
  assert.equal(loads().length, 2, 'the visible retry starts a new read');
  assert.deepEqual(loads()[1]!.args.modelRevision, { id: 'official', revision: 1 });
  loads()[1]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(reads().length, 1);
  reads()[0]!.resolve([parameterRow()]);
  await page.settle();
  assert.ok(
    page.nodes().some((node) => node.type === 'input' && node.props['aria-label'] === 'DEF_TEST'),
  );
});
it('a parameter read from the previous revision never reaches the new revision', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  const loads = () => page.requests.filter((r) => r.command === 'model_case_context');
  const reads = () => page.requests.filter((r) => r.command === 'study_parameter_contexts');
  assert.equal(loads().length, 1);
  assert.deepEqual(loads()[0]!.args.modelRevision, { id: 'official', revision: 1 });
  // 旧修订的请求还在飞：换版本后，旧结果回来不能被当成新修订的参数读取。
  page.state.versionRef = { id: 'official', revision: 2 };
  page.render();
  await page.settle();
  assert.equal(loads().length, 1, 'the new revision waits for the in-flight read to finish');
  loads()[0]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(reads().length, 0, 'the stale revision never reaches the parameter read');
  assert.equal(loads().length, 2, 'the new revision is read once the stale read settles');
  assert.deepEqual(loads()[1]!.args.modelRevision, { id: 'official', revision: 2 });
  loads()[1]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(reads().length, 1);
  reads()[0]!.resolve([parameterRow()]);
  await page.settle();
  assert.ok(
    page.nodes().some((node) => node.type === 'input' && node.props['aria-label'] === 'DEF_TEST'),
  );
});
it('switching the model revision under the same case and kernel reads the parameters again', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  const loads = () => page.requests.filter((r) => r.command === 'model_case_context');
  const reads = () => page.requests.filter((r) => r.command === 'study_parameter_contexts');
  assert.equal(loads().length, 1);
  assert.deepEqual(loads()[0]!.args.modelRevision, { id: 'official', revision: 1 });
  loads()[0]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(reads().length, 1);
  reads()[0]!.resolve([parameterRow()]);
  await page.settle();
  const row = (name: string) =>
    page.nodes().find((node) => node.type === 'input' && node.props['aria-label'] === name);
  assert.ok(row('DEF_TEST'), 'revision 1 parameters are shown');
  // 普通重绘不重复读取（模型没变，缓存命中）。
  page.render();
  page.render();
  await page.settle();
  assert.equal(loads().length, 1);
  assert.equal(reads().length, 1);
  // 同一算例、同一编译配置换模型修订：参数要自动重读，旧修订的行不能留。
  page.state.versionRef = { id: 'official', revision: 2 };
  page.state.versions = [{ version: { id: 'official', revision: 2 }, name: 'Official CoLM r2' }];
  page.render();
  await page.settle();
  assert.equal(loads().length, 2, 'the new revision is read again automatically');
  assert.deepEqual(loads()[1]!.args.modelRevision, { id: 'official', revision: 2 });
  loads()[1]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  assert.equal(reads().length, 2);
  assert.deepEqual(reads()[1]!.args, {
    dirs: ['/p/site1'],
    kernelDir: '/kernels/default',
    types: null,
  });
  reads()[1]!.resolve([
    {
      ...parameterRow(),
      name: 'DEF_TEST_R2',
      label_en: 'Test parameter r2',
      label_zh: '测试参数 r2',
    },
  ]);
  await page.settle();
  assert.ok(row('DEF_TEST_R2'), 'the parameters read under revision 2 replace the old rows');
  assert.equal(row('DEF_TEST'), undefined, 'the old revision rows are gone');
});
it('an uncertainty analysis prefills the whole period after warm-up and sends it as the analysis window', async () => {
  const filled: Record<string, string> = { from: '', to: '', valFrom: '', valTo: '' };
  const setter = (key: string) => (value: (old: string) => string) => {
    filled[key] = value(filled[key]!);
  };
  const run = pageCallback('prefillWindows', {
    tuning: false,
    sites: ['/p/site1'],
    invoke: async () => ({
      start: '2002-01-01',
      end: '2012-12-31',
      output_start: '2002-01-01',
      spinup_years: 0,
    }),
    suggestedWindows,
    autoWindows: { current: { from: '', to: '', valFrom: '', valTo: '' } },
    setFrom: setter('from'),
    setTo: setter('to'),
    setValFrom: setter('valFrom'),
    setValTo: setter('valTo'),
    setCasePeriod: () => {},
    setWindowNote: () => {},
  }) as unknown as (period: [string, string] | null, live: () => boolean) => Promise<void>;
  await run(['2002-01-01', '2012-12-31'], () => true);
  assert.deepEqual(filled, {
    from: '2003-01-01T00:00',
    to: '2012-12-31T00:00',
    valFrom: '',
    valTo: '',
  });
});
it('a disabled next step says what is missing', async () => {
  const page = researchPage();
  page.render();
  page.press('research-method-tuning');
  page.press('research-step-2');
  await page.settle();
  page.requests.at(-1)!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  page.requests.at(-1)!.resolve([parameterRow()]);
  await page.settle();
  const text = JSON.stringify(page.nodes().map((node) => node.children));
  assert.match(text, /先在第 2 步勾选至少一个参数/);
  const step3 = page.nodes().find((n) => n.props.action === 'research-step-3');
  assert.match(String(step3?.props.title), /勾选至少一个参数/);
});
it('the uncertainty envelope names its site and units and draws the band apart from the baseline', () => {
  type Node = { type: unknown; props: Record<string, unknown>; children: unknown[] };
  // StudyResult 返回 JSX，pageCallback 只转译不编 JSX：这里按 React 方式转译整个函数。
  const path = new URL('./ResearchPage.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'StudyResult')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  const js = ts.transpileModule(`${body}`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
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
  const render = new Function('React', 'ScienceChart', `${js}; return StudyResult;`)(
    React,
    'ScienceChart',
  ) as (props: { text: string; theme: string; en: boolean }) => Node;
  const tree = render({
    text: JSON.stringify({
      site: 'AT-Neu',
      variable: 'f_lfevpa',
      units: 'W/m2',
      members: 20,
      time: [0, 1, 2],
      baseline: [1, 2, 3],
      p05: [0.5, null, 2.5],
      p50: [1, null, 3],
      p95: [1.5, null, 3.5],
      stable: [true, false, true],
    }),
    theme: 'light',
    en: false,
  });
  const all = (node: unknown): Node[] =>
    node && typeof node === 'object' && 'children' in (node as Node)
      ? [node as Node, ...(node as Node).children.flatMap(all)]
      : Array.isArray(node)
        ? node.flatMap(all)
        : [];
  const nodes = all(tree);
  const text = JSON.stringify(nodes.map((n) => n.children));
  assert.match(text, /AT-Neu/);
  assert.match(text, /W\/m2/);
  assert.match(text, /20 个样本成员/);
  assert.match(text, /有 1 个时刻成功的成员太少/);
  const chart = nodes.find((n) => n.type === 'ScienceChart')!;
  const styles = chart.props.styles as { stroke: string; dash?: number[] }[];
  assert.notEqual(styles[0]!.stroke, styles[3]!.stroke, 'P95 no longer shares the baseline colour');
  assert.ok(styles[1]!.dash && styles[3]!.dash, 'the band edges are dashed');
});
it('daily comparison and per-variable weights reach the study spec', async () => {
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
  const input = (match: (node: Tree) => boolean) => page.nodes().find(match)!;
  const set = (node: Tree, value: string) =>
    (node.props.onChange as (event: unknown) => void)({ target: { value } });
  set(
    input((n) => n.type === 'Input' && String(n.props.placeholder ?? '').includes('*_Flux.nc')),
    '/obs/site1_Flux.nc',
  );
  await page.settle();
  set(
    input((n) => n.type === 'Input' && n.props.value === 'Qle'),
    'Qle, Qh',
  );
  page.render();
  set(
    input((n) => n.type === 'Select' && n.props.value === ''),
    'daily',
  );
  page.render();
  set(
    input((n) => n.type === 'Input' && n.props.placeholder === '1'),
    '3',
  );
  page.render();
  for (const [id, value] of [
    ['training-from', '2003-01-01T00:00'],
    ['training-to', '2006-01-01T00:00'],
  ]) {
    const field = (page.contract().fields as { id: string; set: (v: string) => void }[]).find(
      (f) => f.id === id,
    );
    field?.set(value);
  }
  await page.settle();
  page.press('study-preflight');
  await page.settle();
  const request = page.requests.find((r) => r.command === 'study_preflight_json');
  assert.ok(request, 'preflight was sent');
  const spec = JSON.parse(String(request.args.specJson)) as {
    targets: { variable: string; weight: number; aggregate?: string }[];
  };
  assert.deepEqual(
    spec.targets.map((t) => [t.variable, t.weight, t.aggregate]),
    [
      ['Qle', 3, 'daily'],
      ['Qh', 1, 'daily'],
    ],
  );
});
it('without cases the home offers to open a project instead of methods that lead nowhere', () => {
  const page = researchPage({ cases: [], selectedCase: null });
  page.render();
  const enabled = (id: string) => page.actions().find((a) => a.id === id)?.enabled;
  assert.equal(enabled('research-method-tuning'), false);
  assert.equal(enabled('research-method-uncertainty'), false);
  assert.equal(enabled('research-method-manual'), true, 'manual edits need no case');
  assert.equal(enabled('research-open-project'), true);
  assert.equal(enabled('research-new-case'), true);
  page.press('research-open-project');
  assert.deepEqual(page.routes.at(-1), ['project', undefined]);
});

// U-01/U-02：从"新建修改版"进来的研究用自己的基线读参数、跑研究；两条入口共用 ResearchPage。
// 页面上还要能看到只读的研究身份摘要（项目/算例、研究基线、运行修订/草稿、计算位置）。
function pageText(page: ReturnType<typeof researchPage>): string {
  const out: string[] = [];
  const walk = (node: unknown) => {
    if (typeof node === 'string') out.push(node);
    else if (Array.isArray(node)) node.forEach(walk);
    else if (node && typeof node === 'object') walk((node as Tree).children);
  };
  walk(page.nodes());
  return out.join('|');
}
const studyVersions = [
  { version: { id: 'official', revision: 1 }, name: 'Official CoLM' },
  { version: { id: 'soil', revision: 3 }, name: 'Soil scheme' },
  { version: { id: 'trace', revision: 7 }, name: 'Trace scheme' },
];
it('a study from 新建修改版 reads parameters on its baseline and keeps the run revision', async () => {
  const page = researchPage({
    versionRef: { id: 'official', revision: 1 },
    researchBaseline: { id: 'soil', revision: 3 },
    versions: studyVersions,
    // Hub 入口同时带方法：研究页消费方法打开工作台，但基线必须留着。
    researchMethod: 'tuning',
  });
  page.render();
  assert.deepEqual(page.state.researchBaseline, { id: 'soil', revision: 3 });
  assert.deepEqual(page.state.versionRef, { id: 'official', revision: 1 });
  const summary = pageText(page);
  assert.match(summary, /项目\/算例/);
  assert.match(summary, /研究基线/);
  assert.match(summary, /Soil scheme r3/, '摘要里的研究基线是用户选的版本');
  assert.match(summary, /编辑草稿或运行修订/);
  assert.match(summary, /计算位置/);
  assert.match(summary, /本机/);
  page.press('research-step-2');
  await page.settle();
  const loads = () => page.requests.filter((r) => r.command === 'model_case_context');
  assert.equal(loads().length, 1);
  assert.deepEqual(loads()[0]!.args.modelRevision, { id: 'soil', revision: 3 });
  loads()[0]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  const reads = page.requests.filter((r) => r.command === 'study_parameter_contexts');
  assert.equal(reads.length, 1);
  reads[0]!.resolve([parameterRow()]);
  await page.settle();
  const box = page
    .nodes()
    .find((node) => node.type === 'input' && node.props['aria-label'] === 'DEF_TEST');
  (box!.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  // 选参数与读算例都没有改写模拟运行版本。
  assert.deepEqual(page.state.versionRef, { id: 'official', revision: 1 });
});
it('starting a method from the research home clears the previous target, seed and baseline', () => {
  const page = researchPage({
    researchBaseline: { id: 'soil', revision: 3 },
    researchTarget: { draftId: 'd-model', name: 'Soil draft' },
    researchSeed: { parameters: [] },
  });
  page.render();
  page.press('research-method-tuning');
  assert.equal(page.state.researchBaseline, null, '首页没有显式基线，退回当前运行修订');
  assert.equal(page.state.researchTarget, null);
  assert.equal(page.state.researchSeed, null);
  assert.equal(page.state.researchMethod, null);
});
// P2 复现：研究基线（soil@3）与当前运行版本（official@1）不同时，侧栏"研究"原先把 researchBaseline
// 清成 null，模型身份 effect 随即清掉 studyDir/envelope/remoteReply，首页"查看进度"和远程追踪入口
// 一起消失。侧栏回首页只是展示已打开研究，不是新建研究；清场由 startHomeMethod/chooseMethod 负责。
it('the sidebar research entry keeps an open study and its remote tracking', async () => {
  const page = researchPage({
    versionRef: { id: 'official', revision: 1 },
    researchBaseline: { id: 'soil', revision: 3 },
    versions: studyVersions,
  });
  page.render();
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-side');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  const envelope = {
    manifest: {
      id: 's-side',
      spec: {
        kind: 'tuning',
        kernel_dir: '/kernels/default',
        parameters: [{ name: 'DEF_TEST', sample_min: 0, sample_max: 10, scale: 'linear' }],
      },
    },
    state: { status: 'Ready', candidates: {} },
    results: [],
    model_version: { model_revision: { version: { id: 'trace', revision: 7 } } },
    remote: { host: 'cluster-a', job: 'job-42' },
  };
  page.requests.at(-1)!.resolve(envelope);
  await page.settle();
  page.press('remote-study-status');
  await page.settle();
  page.requests
    .filter((request) => request.command === 'study_status')
    .at(-1)!
    .resolve(envelope);
  await page.settle();
  page.requests
    .filter((request) => request.command === 'remote_study_status')
    .at(-1)!
    .resolve({
      host: 'cluster-a',
      job: 'job-42',
      status: { state: 'running' },
      recovery: {
        outcome: 'running',
        terminal: false,
        finished: false,
        study_created: true,
        fetched: false,
        permissions: {
          submit: false,
          resubmit: false,
          cancel: true,
          fetch: false,
          close: false,
          poll: true,
        },
        next_zh: '等待服务器作业完成',
        next_en: 'Wait for the server job to finish',
      },
      warnings: [],
    });
  await page.settle();
  const remoteReply = '等待服务器作业完成';
  assert.match(pageText(page), new RegExp(remoteReply));
  const kept = () => {
    assert.deepEqual(page.state.researchBaseline, { id: 'soil', revision: 3 });
    assert.deepEqual(page.state.versionRef, { id: 'official', revision: 1 });
    assert.ok(
      page.actions().some((action) => action.id === 'research-current'),
      'the open study still offers “查看进度”',
    );
    assert.match(pageText(page), /cluster-a/, 'the remote host is still shown');
  };
  // showHome 的既有行为就是保留研究上下文、只回首页；侧栏“研究”必须与它一致。
  page.press('research-home');
  await page.settle();
  kept();
  page.press('research-current');
  await page.settle();
  assert.match(pageText(page), new RegExp(remoteReply), 'the remote reply survives showHome');
  page.state.researchHome = true;
  page.render();
  await page.settle();
  assert.equal(page.state.researchHome, false);
  kept();
  page.press('research-current');
  await page.settle();
  assert.match(pageText(page), new RegExp(remoteReply), 'remote tracking is still available');
});
it('the parameter read identity follows the study baseline, not the run revision', async () => {
  const page = researchPage({
    versionRef: { id: 'official', revision: 1 },
    researchBaseline: { id: 'soil', revision: 3 },
    versions: studyVersions,
    researchMethod: 'tuning',
  });
  page.render();
  page.press('research-step-2');
  await page.settle();
  const loads = () => page.requests.filter((r) => r.command === 'model_case_context');
  assert.equal(loads().length, 1);
  assert.deepEqual(loads()[0]!.args.modelRevision, { id: 'soil', revision: 3 });
  // 运行版本变了不是研究换了模型：不重读，旧请求照常算当前身份。
  page.state.versionRef = { id: 'official', revision: 2 };
  page.render();
  await page.settle();
  assert.equal(loads().length, 1, '运行版本变化不改变研究基线身份');
  loads()[0]!.resolve({ entry: { dir: '/p/site1' } });
  await page.settle();
  const reads = page.requests.filter((r) => r.command === 'study_parameter_contexts');
  assert.equal(reads.length, 1);
  reads[0]!.resolve([parameterRow()]);
  await page.settle();
  // 研究基线换了才是另一个研究：自动按新基线重读（GUI-14 的旧结果核对规则不变）。
  page.state.researchBaseline = { id: 'soil', revision: 4 };
  page.render();
  await page.settle();
  assert.equal(loads().length, 2, '换研究基线要重读参数');
  assert.deepEqual(loads()[1]!.args.modelRevision, { id: 'soil', revision: 4 });
});
it('a running study keeps its recorded revision when the baseline and run revision change', async () => {
  const page = researchPage({
    versionRef: { id: 'official', revision: 1 },
    researchBaseline: { id: 'soil', revision: 3 },
    versions: studyVersions,
    // 从"新建修改版"进来的研究：打开一个已有研究，基线仍在。
    researchMethod: 'tuning',
  });
  page.render();
  page.press('study-browse');
  assert.equal(page.requests.at(-1)!.command, 'pick_folder');
  page.requests.at(-1)!.resolve('/p/.colm/studies/s-run');
  await page.settle();
  assert.equal(page.requests.at(-1)!.command, 'study_status');
  page.requests.at(-1)!.resolve({
    manifest: {
      id: 's-run',
      spec: {
        kind: 'tuning',
        kernel_dir: '/kernels/default',
        parameters: [{ name: 'DEF_TEST', sample_min: 0, sample_max: 10, scale: 'linear' }],
      },
    },
    state: { status: 'Ready' },
    results: [],
    model_version: { model_revision: { version: { id: 'trace', revision: 7 } } },
    remote: null,
  });
  await page.settle();
  const confirm = page
    .nodes()
    .find(
      (node) =>
        node.type === 'label' && String(node.props.className ?? '').includes('study-confirm'),
    )
    ?.children.flat()
    .find(
      (node): node is Tree =>
        !!node && typeof node === 'object' && (node as Tree).props?.type === 'checkbox',
    );
  assert.ok(confirm);
  (confirm.props.onChange as (event: unknown) => void)({ target: { checked: true } });
  page.render();
  page.press('study-run');
  assert.equal(page.requests.at(-1)!.command, 'study_run');
  const submitted = page.requests.at(-1)!;
  assert.deepEqual(
    submitted.args.modelRevision,
    { id: 'trace', revision: 7 },
    '已建研究用它自己记下的冻结修订运行',
  );
  // 运行请求还在飞时换研究基线、换运行版本：这个研究保留自己的冻结修订和身份。
  page.state.researchBaseline = { id: 'official', revision: 1 };
  page.state.versionRef = { id: 'official', revision: 2 };
  page.render();
  await page.settle();
  assert.ok(page.actions().some((action) => action.id === 'study-status' && action.enabled));
  const summary = pageText(page);
  assert.match(summary, /研究基线/, '摘要跟着入口状态更新');
  assert.match(summary, /Official CoLM r1/);
  assert.match(summary, /运行修订/, '摘要仍显示这个研究自己的运行修订');
  assert.match(summary, /Trace scheme r7/);
  assert.equal(
    page.requests.filter((r) => r.command === 'model_case_context').length,
    0,
    '运行中的研究不因基线/运行版本变化重读参数',
  );
  submitted.resolve({});
  await page.settle();
  assert.ok(page.actions().some((action) => action.id === 'study-status' && action.enabled));
});

// 建研究和预检交给后端的模型修订必须来自研究基线（U-01）；这段逻辑在页面的 check() 里。
it('study preflight and creation carry the study baseline, not the run version', async () => {
  const calls: [string, Record<string, unknown>][] = [];
  const check = pageCallback('check', {
    task: (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    spec: () => ({ specJson: '{"kind":"tuning"}' }),
    invoke: async (command: string, args: Record<string, unknown>) => {
      calls.push([command, args]);
      return {};
    },
    studyBaseline: { id: 'soil', revision: 3 },
    setPreflight: () => {},
  }) as unknown as () => Promise<unknown>;
  await check();
  assert.deepEqual(
    calls.map(([command]) => command),
    ['study_preflight_json'],
  );
  assert.deepEqual(calls[0]![1].modelRevision, { id: 'soil', revision: 3 });
});
