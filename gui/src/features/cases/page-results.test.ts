import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import {
  matchesRemoteRun,
  reconcileRemoteRun,
  remoteEndStatus,
  remoteRunPermissions,
} from '../runs/run-model';
import { caseSimulationDomain } from './model-settings';
import { chartVariables, readErrorText, variableOption } from '../results/variable-labels';
import { withCatalogLabels } from '../../lib/param-presentation';
import {
  clearParameterContextMarker,
  parameterContextKey,
  sameParameterContextIdentity,
} from '../studies/parameter-context-cache';
import { boundedMap, boundedMapSettled } from '../results/series';
import { comparisonCsv, comparisonKey, exportGate } from '../results/analysis-state';

// Run the actual page callbacks with native IPC replaced; no duplicate task implementation.
function callback(page: string, name: string, context: Record<string, unknown>) {
  const path = new URL(page, import.meta.url),
    text = readFileSync(path, 'utf8');
  const source = ts.createSourceFile(
    path.pathname,
    text,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body: string | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === name && node.initializer)
      body = node.initializer.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, `${name}: actual callback must exist`);
  const js = ts.transpileModule(`const run=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return run;`)(...Object.values(context)) as (
    ...args: unknown[]
  ) => Promise<unknown>;
}
const prep = './PreparationPage.tsx',
  research = '../studies/ResearchPage.tsx';
test('preparation callbacks retain native evidence and errors through both async layers', async () => {
  const shown: unknown[] = [],
    errors: string[] = [];
  const lock = { current: false };
  const work = callback(prep, 'work', {
    lock,
    setBusy: () => {},
    setResult: (value: unknown) => shown.push(value),
    setError: (value: string) => errors.push(value),
    setDeclined: () => {},
    notify: () => {},
    t: (zh: string) => zh,
  });
  const inspection = { slots: [], time_start: '2008-01-01', time_end: '2010-01-01' };
  const inspect = callback(prep, 'inspect', {
    work,
    mode: 'netcdf',
    form: { src: '/source' },
    invoke: async () => inspection,
    setProbe: () => {},
    setSlots: () => {},
    setForm: () => {},
    preparedDestination: () => '/output',
  });
  assert.equal(await inspect(), inspection);
  assert.equal(shown.at(-1), inspection);
  const downloaded = { path: '/era5.nc', days: 7 };
  const execute = callback(prep, 'execute', {
    work,
    mode: 'era5',
    form: { dst: '/era5.nc', lat: '1', lon: '2' },
    num: () => 1,
    invoke: async () => downloaded,
  });
  assert.equal(await execute(), downloaded);
  const report = { unresolved: 0, filled: 12 };
  const finish = callback(prep, 'finish', {
    work,
    slots: [],
    gapOptions: () => ({}),
    invoke: async () => report,
    setImported: () => {},
  });
  assert.deepEqual(await finish({ staged_path: '/stage', final_path: '/forcing.nc' }), {
    path: '/forcing.nc',
    ...report,
  });
  await assert.rejects(
    work(async () => {
      throw new Error('forcing missing');
    }),
    /forcing missing/,
  );
  assert.equal(errors.at(-1), 'Error: forcing missing');
  assert.equal(lock.current, false);
});
test('stopping data preparation reads as a decision, not a failure', async () => {
  const errors: string[] = [],
    declined: string[] = [],
    notices: [string, boolean | undefined][] = [];
  const work = callback(prep, 'work', {
    lock: { current: false },
    setBusy: () => {},
    setResult: () => {},
    setError: (value: string) => errors.push(value),
    setDeclined: (value: string) => declined.push(value),
    notify: (message: string, error?: boolean) => notices.push([message, error]),
    t: (zh: string) => zh,
  });
  await assert.rejects(
    work(async () => {
      throw '已停止：数据准备被中断，输出文件可能不完整';
    }),
  );
  assert.equal(errors.at(-1), '');
  assert.match(declined.at(-1) ?? '', /^已停止。输出文件可能不完整/);
  assert.ok(!notices.some(([, error]) => error), 'no error banner for a stop');
  const calls: string[] = [];
  const stop = callback(prep, 'stopPreparation', {
    invoke: async (command: string) => (calls.push(command), 0),
    notify: (message: string) => notices.push([message, undefined]),
    t: (zh: string) => zh,
  });
  assert.equal(await stop(), 0);
  assert.deepEqual(calls, ['cancel_preparation']);
  assert.match(notices.at(-1)?.[0] ?? '', /没有正在进行的数据准备可停/);
});
test('research inspection and saved-model callbacks return backend evidence even if display context changes', async () => {
  const task = callback(research, 'task', {
    lock: { current: false },
    epoch: { current: 1 },
    setBusy: () => {},
    setError: () => {},
  });
  const evidence = {
    ok: false,
    warnings: ['parameter unavailable'],
    version: { id: 'soil', revision: 1 },
  };
  const ignored = () => {};
  const common = {
    task,
    invoke: async () => evidence,
    invokeJson: async () => evidence,
    state: { versionRef: { id: 'soil', revision: 1 }, root: '/project' },
    // U-01：研究页按“研究基线 / 已建研究记录的修订”读模型，不再直接读全局运行版本。
    studyBaseline: { id: 'soil', revision: 1 },
    studyModel: { id: 'soil', revision: 1 },
    kernel: '/kernel',
    preprocessors: 'rust',
    dir: '/case',
    effectiveCase: async () => '/preview',
    setPreflight: ignored,
    setHybridInfo: ignored,
    setResult: ignored,
    setHybridStudies: ignored,
    setContexts: ignored,
    spec: () => ({ caseRoot: '/project', specJson: '{}' }),
    member: 'm000001',
    aiCase: '/case',
    name: 'soil',
    studyDir: '/study',
    notify: ignored,
    refreshVersions: async () => {},
  };
  for (const name of ['check', 'inspectHybrid', 'checkHybrid', 'findStudies'])
    assert.equal(
      await callback(research, name, common)(),
      evidence,
      `${name} retains actual response`,
    );
  const rows = [{ name: 'DEF_TEST', scope_instance: { kind: 'global' } }];
  // 参数读取现在带着身份（模型修订/站点/编译配置/类型）出去再核对（GUI-14）：这里给两次读取
  // 各一份与 common 一致的桩身份，测试语义不变——仍验证返回行与改类型后的勾选搬迁。
  const contextIdentity = (spatial: boolean, types: number[]) => ({
    version: { id: 'soil', revision: 1 },
    kernel: '/kernel',
    sites: ['/case'],
    spatial,
    types,
  });
  const loadContext = (identity: ReturnType<typeof contextIdentity>) => ({
    ...common,
    parameterContextKey,
    sameParameterContextIdentity,
    clearParameterContextMarker,
    contextIdentityRef: { current: identity },
    loadedFor: { current: '' },
    failedFor: { current: '' },
  });
  const point = contextIdentity(false, []);
  assert.equal(
    await callback(research, 'loadContexts', {
      ...loadContext(point),
      sites: ['/case'],
      invokeJson: async () => rows,
      withCatalogLabels,
      spatialStudy: false,
      types: [],
      contextsRef: { current: [] },
      setParameters: ignored,
    })(point),
    rows,
  );
  // 格点研究改勾类型后行序会变：已勾的参数按参数与作用域搬到新行号上，不留在旧行号指向别的参数。
  const at = (index: number) => ({
    id: 'gradm',
    name: 'DEF_LC_GRADM',
    scope_instance: { kind: 'land-cover-class', scheme: 'IGBP', index },
  });
  let ticks: Record<string, unknown> = { '0': { enabled: true, min: '4', max: '9' } };
  const gridded = contextIdentity(true, [1, 10]);
  await callback(research, 'loadContexts', {
    ...loadContext(gridded),
    sites: ['/case'],
    invokeJson: async () => [at(1), at(10)],
    withCatalogLabels,
    spatialStudy: true,
    types: [1, 10],
    contextsRef: { current: [at(10)] },
    setParameters: (next: (old: typeof ticks) => typeof ticks) => {
      ticks = next(ticks);
    },
  })(gridded);
  assert.deepEqual(ticks, { '1': { enabled: true, min: '4', max: '9' } });
  const text = '[{"member":"m000001","validated":false}]';
  assert.equal(
    await callback(research, 'loadResult', {
      ...common,
      resultPath: 'metrics.json',
      invoke: async () => text,
    })(),
    text,
  );
  let updated = false;
  assert.equal(
    await callback(research, 'checkHybrid', {
      ...common,
      task: async (fn: (current: () => boolean) => Promise<unknown>) => fn(() => false),
      setResult: () => {
        updated = true;
      },
    })(),
    evidence,
  );
  assert.equal(updated, false, 'late native response does not overwrite a newly selected page');
});

test('resource and run reconciliation callbacks return the recorded failure to the assistant', async () => {
  const evidence = {
    record: { host: 'server', job: 'j1' },
    pending: null,
    status: { state: 'finished', exit_code: 1, log_tail: 'failed' },
  };
  const noop = () => {};
  const token = { active: true };
  const execute = callback('../resources/ResourcesPage.tsx', 'execute', {
    lock: { current: false },
    epoch: { current: token },
    setBusy: noop,
    setError: noop,
    setOutput: noop,
    t: (a: string) => a,
  });
  assert.equal(await execute('remote', async () => evidence), evidence);
  await assert.rejects(
    execute('remote', async () => {
      throw new Error('disconnected');
    }),
    /disconnected/,
  );
  let app = {
    running: {
      '/case': {
        case: '/case',
        run_id: 'r1',
        status: 'unknown',
        location: 'remote',
        progress: { remote: { host: 'server', job: 'j1' } },
        running: true,
        logs: [],
        stages: {},
      },
    },
  };
  const remote = { host: 'server', job: 'j1', pending: true, needs_reconciliation: true };
  const reconcile = callback('../runs/RunPanel.tsx', 'remoteAction', {
    lock: { current: false },
    monitor: false,
    remotePermissions: remoteRunPermissions(remote),
    latest: { current: app.running },
    casePath: '/case',
    currentCase: { current: '/case' },
    remote,
    invokeJson: async () => evidence,
    alive: { current: true },
    setRemote: noop,
    setMonitor: noop,
    notify: noop,
    reconcileRemoteRun,
    matchesRemoteRun,
    remoteEndStatus,
    remoteRunPermissions,
    update: (fn: (state: typeof app) => Partial<typeof app>) => {
      app = { ...app, ...fn(app) };
    },
  });
  const reply = (await reconcile('remote_status')) as Record<string, unknown>;
  assert.equal((reply.status as { exit_code: number }).exit_code, 1);
  assert.equal(app.running['/case'].status, 'failed');
  assert.equal(app.running['/case'].running, false);
});

test('Results full CSV uses the native unlimited reader and returns actual download-request evidence', async () => {
  const { fullSeriesCsv } = await import('../results/series');
  const page = '../results/ResultsPage.tsx',
    noop = () => {},
    epoch = { current: 1 };
  const task = callback(page, 'task', {
    lock: { current: false },
    epoch,
    setBusy: noop,
    setError: noop,
  });
  const data = {
    time: [0, 60],
    vars: { f_fsena: [1, null] },
    source_n: 2,
    n: 2,
    downsampled: false,
  };
  const calls: Record<string, unknown>[] = [],
    downloads: unknown[][] = [];
  const cache = {
    current: new Map([
      [
        JSON.stringify({ case: '/copy', vars: 'f_fsena', from: 0, to: 60, maxPoints: 2400 }),
        { ...data, source_n: 100, downsampled: true },
      ],
    ]),
  };
  const readSeries = callback(page, 'readSeries', {
    source: '/copy',
    en: false,
    readErrorText,
    useCallback: (fn: unknown) => fn,
    windowArgs: () => ({ from: 0, to: 60 }),
    cache,
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      assert.equal(command, 'series');
      calls.push(args);
      return data;
    },
  });
  const context = {
    task,
    source: '/copy',
    variable: 'f_fsena',
    readSeries,
    fullSeriesCsv,
    download: (...args: unknown[]) => {
      downloads.push(args);
      return { status: 'download_requested', filename: args[0], mime_type: args[2] };
    },
    setExportNotice: noop,
    exportMessage: callback(page, 'exportMessage', { tr: (zh: string) => zh }),
    tr: (zh: string) => zh,
  };
  const result = (await callback(page, 'exportFullSeries', context)()) as Record<string, unknown>;
  assert.equal(calls[0].maxPoints, null);
  assert.equal(cache.current.size, 1);
  assert.equal(downloads.length, 1);
  assert.equal(downloads[0][0], 'copy-f_fsena.csv');
  assert.equal(result.status, 'download_requested');
  assert.equal(result.n, 2);
  assert.equal(result.downsampled, false);
  downloads.length = 0;
  await assert.rejects(
    callback(page, 'exportFullSeries', {
      ...context,
      task: async (fn: (current: () => boolean) => Promise<unknown>) => fn(() => false),
    })(),
    /selection changed/,
  );
  assert.equal(downloads.length, 0, 'a late response does not download a different selection');
});

const resultRun = (index: number) => ({
  run_case: `/case/runs/${index}`,
  case_dir: '/case',
  case_name: 'case',
  status: 'succeeded',
});
const resultMetric = (index: number) => ({
  name: 'Qle',
  n: 2,
  rmse: index + 1,
  mae: 1,
  bias: 0,
  r2: 1,
  correlation: 1,
  nse: 1,
  kge: 1,
});
/**
 * ResultsPage 多站比较/重试的共享上下文：从页面源码抽出真实回调（比对/合并/失败分类/进度绑定），
 * 只有 metrics 网络层由用例给。这样测的是页面真实逻辑，而不是测试里重写一遍。
 */
function comparisonPage(
  metricRows: (
    args: Record<string, unknown>,
  ) => Promise<Record<string, unknown>[]> | Record<string, unknown>[],
  options: {
    entries?: number[];
    requestSeq?: { current: { single: number; multi: number } };
  } = {},
) {
  const page = '../results/ResultsPage.tsx';
  const indexes = options.entries ?? [0, 1, 2];
  const runs = indexes.map((index) => resultRun(index));
  const comparisonEntries = runs.map((run, at) => ({ run, obs: `/obs/${indexes[at]}.nc` }));
  const requestSeq = options.requestSeq ?? { current: { single: 0, multi: 0 } };
  const published: Record<string, unknown>[] = [];
  const progress: unknown[] = [];
  const requests: Record<string, unknown>[] = [];
  const busy: boolean[] = [];
  const lock = { current: false };
  let progressState: unknown = null;
  const task = callback(page, 'task', {
    lock,
    epoch: { current: 1 },
    setBusy: (value: boolean) => busy.push(value),
    setError: () => {},
  });
  const comparisonSources = callback(page, 'comparisonSources', {
    projectRuns: runs,
    caseDirOfRunPath: () => null,
    boundedMap,
    invokeJson: async () => [],
  });
  const comparisonMetricsFor = callback(page, 'comparisonMetricsFor', {
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      requests.push({ command, ...args });
      return metricRows(args);
    },
    windowArgs: () => ({ from: null, to: null }),
  });
  const comparisonFailureReason = callback(page, 'comparisonFailureReason', {
    bilingual: (zh: string) => zh,
  });
  const splitSettledComparison = callback(page, 'splitSettledComparison', {
    comparisonFailureReason,
  });
  const common = {
    task,
    comparisonEntries,
    projectRuns: runs,
    from: '',
    to: '',
    spinup: '0',
    corrected: false,
    requestSeq,
    comparisonTaskId: { current: 0 },
    comparisonKey,
    boundedMapSettled,
    comparisonSources,
    comparisonMetricsFor,
    comparisonFailureReason,
    splitSettledComparison,
    windowArgs: () => ({ from: null, to: null }),
    setComparisonProgress: (value: unknown) => {
      progressState =
        typeof value === 'function'
          ? (value as (previous: unknown) => unknown)(progressState)
          : value;
      progress.push(progressState);
    },
    setComparisonResult: (value: unknown) => published.push(value as Record<string, unknown>),
    bilingual: (zh: string) => zh,
  };
  return {
    task,
    runs,
    requests,
    published,
    progress,
    busy,
    lock,
    requestSeq,
    compare: callback(page, 'compare', common),
    retry: (comparisonFresh: boolean, snapshot?: unknown) =>
      callback(page, 'retryFailedComparison', {
        ...common,
        comparisonResult: snapshot ?? published.at(-1) ?? null,
        comparisonFresh,
      }),
  };
}

test('a comparison whose conditions change mid-flight cannot publish its stale rows', async () => {
  const requestSeq = { current: { single: 0, multi: 0 } };
  const page = comparisonPage(
    () => {
      // 计算进行中条件变了（例如换了窗口/编辑了清单）：请求序号前进，这份结果必须作废。
      requestSeq.current.multi++;
      return [resultMetric(0)];
    },
    { entries: [0], requestSeq },
  );
  const rows = (await page.compare()) as Record<string, unknown>[];
  assert.deepEqual(rows, [], 'a stale comparison returns no rows to its caller');
  assert.equal(
    page.published.length,
    0,
    'a late comparison cannot overwrite the current scope state',
  );
  assert.equal(page.progress.at(-1), null, 'its own progress is cleared even when stale');
});

test('a failed station keeps the successful comparison rows and is listed for retry', async () => {
  const page = comparisonPage((args) => {
    if (args.obs === '/obs/1.nc') throw new Error('history missing');
    return [resultMetric(Number(String(args.obs).match(/\d/)?.[0] ?? 0))];
  });
  const rows = (await page.compare()) as Record<string, unknown>[];
  assert.equal(rows.length, 2, 'the failed station no longer discards the successful rows');
  assert.deepEqual(
    rows.map((row) => row.site),
    ['/case/runs/0', '/case/runs/2'],
  );
  assert.equal(page.published.length, 1);
  const snapshot = page.published[0] as {
    id: string;
    rows: Record<string, unknown>[];
    failures: { runCase: string; obs: string; error: string; cancelled?: boolean }[];
    completedAt: number;
  };
  assert.equal(snapshot.rows.length, 2);
  assert.equal(snapshot.failures.length, 1);
  assert.equal(snapshot.failures[0].runCase, '/case/runs/1');
  assert.equal(snapshot.failures[0].obs, '/obs/1.nc');
  assert.match(snapshot.failures[0].error, /history missing/);
  assert.equal(snapshot.failures[0].cancelled, undefined, 'a real backend error is not a cancel');
  assert.equal(typeof snapshot.completedAt, 'number');
  // task 收尾：按钮不再忙、锁已释放、进度不属于任何任务了。
  assert.equal(page.busy.at(-1), false);
  assert.deepEqual(page.lock.current, false);
  assert.equal(page.progress.at(-1), null);
});

test('retry recomputes only the failed comparison entries and merges them into the snapshot', async () => {
  const failing = new Set(['/obs/1.nc']);
  const page = comparisonPage((args) => {
    if (failing.has(String(args.obs))) throw new Error('history missing');
    return [resultMetric(1)];
  });
  await page.compare();
  const first = page.published.at(-1)!;
  assert.equal((first.failures as unknown[]).length, 1);
  const firstCompletedAt = first.completedAt as number;
  // 条件已经改掉的快照不允许重试（按钮此时也是灰的）。
  const requestsBeforeStale = page.requests.length;
  await assert.rejects(page.retry(false)(), /条件已改/);
  assert.equal(page.requests.length, requestsBeforeStale, 'a stale retry does not call metrics');
  failing.clear();
  const requestedBefore = page.requests.length;
  const retryRows = (await page.retry(true)()) as Record<string, unknown>[];
  const retried = page.requests.slice(requestedBefore);
  assert.equal(retried.length, 1, 'only the failed entry is recomputed');
  assert.equal(retried[0].obs, '/obs/1.nc');
  assert.equal(retryRows.length, 3, 'retried rows merge with the rows kept from the first run');
  const next = page.published.at(-1)!;
  assert.equal(next.id, first.id, 'the retried snapshot keeps the same comparison key');
  assert.deepEqual(next.failures, []);
  assert.equal((next.rows as unknown[]).length, 3);
  assert.ok((next.completedAt as number) >= firstCompletedAt);
  assert.equal(page.progress.at(-1), null);
});

test('cancelling a comparison stops claiming entries that have not started', async () => {
  const requestSeq = { current: { single: 0, multi: 0 } };
  let release!: () => void;
  const gate = new Promise<void>((resolve) => {
    release = resolve;
  });
  let calls = 0;
  const page = comparisonPage(
    async () => {
      calls++;
      await gate;
      return [resultMetric(calls)];
    },
    { entries: [0, 1, 2, 3, 4, 5, 6, 7], requestSeq },
  );
  const pending = page.compare();
  // 让 6 路并发把前 6 条都领走（已在飞）；后 2 条还没开始。
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(calls, 6, 'the concurrency limit claims six entries');
  requestSeq.current.multi++;
  release();
  const rows = (await pending) as unknown[];
  assert.deepEqual(rows, []);
  assert.equal(calls, 6, 'the remaining entries are never claimed after the conditions change');
  assert.equal(page.published.length, 0);
  assert.equal(page.progress.at(-1), null, 'a cancelled task still clears its own progress');
});

test('cancelled comparison entries are recorded as cancelled failures, not backend errors', () => {
  const page = '../results/ResultsPage.tsx';
  const failureReason = callback(page, 'comparisonFailureReason', {
    bilingual: (zh: string) => zh,
  });
  const split = callback(page, 'splitSettledComparison', {
    comparisonFailureReason: failureReason,
  });
  const entries = [
    { run: resultRun(0), obs: '/obs/0.nc' },
    { run: resultRun(1), obs: '/obs/1.nc' },
    { run: resultRun(2), obs: '/obs/2.nc' },
  ];
  const { rows, failures } = split(entries, [
    { value: [resultMetric(0)] },
    { cancelled: true },
    { error: new Error('boom') },
  ]) as unknown as {
    rows: Record<string, unknown>[];
    failures: { runCase: string; obs: string; error: string; cancelled?: boolean }[];
  };
  assert.equal(rows.length, 1);
  assert.equal(failures.length, 2);
  assert.equal(failures[0].runCase, '/case/runs/1');
  assert.equal(failures[0].cancelled, true);
  assert.match(failures[0].error, /取消/);
  assert.equal(failures[1].cancelled, undefined);
  assert.match(failures[1].error, /boom/);
});

test('the default result prefers a successful run whose outputs are really available', () => {
  const page = '../results/ResultsPage.tsx';
  const statusOf = callback(page, 'statusOf', {}) as unknown as (run: unknown) => string;
  const preparedOnly = callback(page, 'preparedOnly', {}) as unknown as (run: unknown) => boolean;
  const usableRun = callback(page, 'usableRun', { preparedOnly }) as unknown as (
    run: unknown,
  ) => boolean;
  const preferredRun = callback(page, 'preferredRun', {
    statusOf,
    usableRun,
    preparedOnly,
  }) as unknown as (runs: unknown[], justCompleted?: string) => string;
  // 旧后端：只有 outputs_local 时按 complete/missing 回退；新后端 local_results 优先。
  assert.equal(statusOf({}), 'complete');
  assert.equal(statusOf({ outputs_local: false }), 'missing');
  assert.equal(statusOf({ outputs_local: false, local_results: { status: 'partial' } }), 'partial');
  assert.equal(statusOf({ local_results: { status: 'invalid' } }), 'invalid');
  assert.equal(statusOf({ outputs_local: true, local_results: { status: 'missing' } }), 'missing');
  const runs = [
    // legacy 占位（指向算例本身、没有结果）与只跑到预处理的运行都不能当默认。
    { run_case: '/case', legacy: true, status: 'needs_reconciliation', version: null },
    { run_case: '/case/runs/prep', status: 'succeeded', provenance: { stage: 'mksrfdata' } },
    { run_case: '/case/runs/failed', status: 'failed' },
    // 更新的成功运行本机却没有结果、也不在服务器上：默认应退到更早但结果完整的运行。
    { run_case: '/case/runs/newer', status: 'succeeded', outputs_local: false },
    { run_case: '/case/runs/complete', status: 'succeeded' },
  ];
  assert.equal(preferredRun(runs), '/case/runs/complete');
  // 刚跑完的运行优先（若它本身可用）。
  assert.equal(preferredRun(runs, '/case/runs/newer'), '/case/runs/newer');
  assert.equal(preferredRun(runs, '/case/runs/failed'), '/case/runs/complete');
  // 结果还在服务器上的运行可远程读，也可当默认。
  assert.equal(
    preferredRun([{ run_case: '/case/runs/remote', status: 'succeeded', server: 'hpc' }]),
    '/case/runs/remote',
  );
  // 只有占位/预处理运行时不选任何一条（页面会显示"尚无已记录的运行"）。
  assert.equal(
    preferredRun([
      { run_case: '/case', legacy: true, status: 'needs_reconciliation' },
      { run_case: '/case/runs/prep', status: 'succeeded', provenance: { stage: 'mkinidata' } },
    ]),
    '',
  );
});

test('comparison export refuses a stale snapshot and writes the snapshot conditions', async () => {
  const page = '../results/ResultsPage.tsx';
  const task = callback(page, 'task', {
    lock: { current: false },
    epoch: { current: 1 },
    setBusy: () => {},
    setError: () => {},
  });
  const request = {
    entries: [{ runCase: '/case/runs/a', obs: '/obs/a.nc' }],
    from: '2008-01-01T00:00',
    to: '2008-01-02T00:00',
    spinup: '8',
    corrected: true,
  };
  const result = {
    scope: 'multi',
    id: comparisonKey(request),
    request,
    rows: [
      {
        name: 'Qle',
        site: '/case/runs/a',
        comparison_obs: '/obs/a.nc',
        n: 2,
        rmse: 1,
      },
    ],
    failures: [],
    completedAt: 1,
  };
  const downloads: unknown[][] = [];
  const notices: string[] = [];
  const context = (gate: { ok: boolean; reason?: string }) => ({
    task,
    comparisonExportGate: gate,
    comparisonResult: result,
    comparisonCsv,
    download: (...args: unknown[]) => {
      downloads.push(args);
      return { status: 'saved', path: '/saved.csv' };
    },
    tr: (zh: string) => zh,
    exportMessage: () => 'saved',
    setExportNotice: (value: string) => notices.push(value),
  });
  await assert.rejects(
    callback(page, 'exportComparison', context({ ok: false, reason: 'conditions-changed' }))(),
    /重新运行比较/,
    'changing a comparison condition must block the old snapshot',
  );
  await assert.rejects(
    callback(page, 'exportComparison', context({ ok: false, reason: 'none' }))(),
    /先运行比较/,
  );
  assert.equal(downloads.length, 0);
  await callback(page, 'exportComparison', context({ ok: true }))();
  assert.equal(downloads.length, 1);
  const text = String(downloads[0][1]);
  assert.match(text, /2008-01-01T00:00/);
  assert.match(text, /2008-01-02T00:00/);
  assert.match(
    text,
    /,8,1,Qle/,
    'spinup and corrected columns come from the compare-time snapshot',
  );
  assert.equal(notices.at(-1), 'saved');
});

test('Results report formats retain summary metrics and route native PDF through print_report', async () => {
  const { reportFile } = await import('../results/series');
  const page = '../results/ResultsPage.tsx',
    noop = () => {};
  const task = callback(page, 'task', {
    lock: { current: false },
    epoch: { current: 1 },
    setBusy: noop,
    setError: noop,
  });
  const metric = {
    name: 'Qh',
    n: 2,
    rmse: 1,
    mae: 1,
    bias: 0,
    r2: 0.81,
    correlation: 0.9,
    nse: 0.5,
    kge: 0.7,
    alpha: 1.1,
    beta: 1,
    time: [0, 1],
    model: [1, 2],
    obs: [1, 2],
  };
  const reportRequest = {
    caseDir: '/case',
    versionId: 'official',
    revision: 1,
    source: '/copy',
    obs: '/obs.csv',
    from: '',
    to: '',
    spinup: '0',
    corrected: false,
  };
  const reportData = callback(page, 'reportData', {
    source: '/copy',
    runs: [],
    catalog: { steps: 2 },
    // 报告只描述单站评估快照；条件列取快照 request。
    singleResult: {
      scope: 'single',
      key: 'report-key',
      request: reportRequest,
      rows: [metric],
      evaluation: [],
      completedAt: 1,
    },
    diagnostic: null,
    baseline: null,
    baselineNote: '',
    obs: '/obs.csv',
    from: '',
    to: '',
    spinup: '0',
    corrected: false,
    comparisonEntries: [],
    comparisonText: () => '',
  }) as unknown as () => Record<string, unknown>;
  const snapshot = reportData();
  const summary = (snapshot.metrics as Record<string, unknown>[])[0];
  assert.equal(summary.correlation, 0.9);
  assert.equal(summary.alpha, 1.1);
  assert.equal(summary.time, undefined);
  assert.equal(metric.time.length, 2);
  const downloads: unknown[][] = [],
    commands: string[] = [];
  const common = {
    task,
    catalog: { steps: 2 },
    reportData,
    singleExportGate: exportGate(
      {
        scope: 'single',
        key: 'report-key',
        rows: [metric],
      },
      'report-key',
    ),
    en: true,
    reportFile,
    setExportNotice: noop,
    exportMessage: callback(page, 'exportMessage', { tr: (zh: string) => zh }),
    tr: (zh: string) => zh,
    // 报告闸门的两条拒绝信息用 bilingual 拼；此前的用例从未走到抛错分支。
    bilingual: (zh: string) => zh,
    download: (...args: unknown[]) => {
      downloads.push(args);
      return { status: 'download_requested', filename: args[0], mime_type: args[2] };
    },
    hasBackend: () => true,
    invoke: async (command: string) => {
      commands.push(command);
    },
    printReport: async (_data: unknown, _en: boolean, native: () => Promise<unknown>) => {
      await native();
      return { status: 'printdialog_requested', format: 'pdf' };
    },
  };
  for (const format of ['json', 'csv', 'markdown', 'html'])
    assert.equal(
      (
        (await callback(page, 'report', { ...common, reportFormat: format })()) as Record<
          string,
          unknown
        >
      ).status,
      'download_requested',
    );
  assert.equal(downloads.length, 4);
  await assert.rejects(
    callback(page, 'report', {
      ...common,
      singleExportGate: { ok: false, reason: 'conditions-changed' },
    })(),
    /重新评估/,
    'a report never describes a stale single-station snapshot',
  );
  await assert.rejects(
    callback(page, 'report', {
      ...common,
      singleExportGate: { ok: false, reason: 'none' },
    })(),
    /先完成一次观测评估/,
  );
  assert.equal(downloads.length, 4, 'stale report attempts do not download');
  const result = await callback(page, 'report', { ...common, reportFormat: 'pdf' })();
  assert.deepEqual(result, { status: 'printdialog_requested', format: 'pdf' });
  assert.deepEqual(commands, ['print_report']);
  assert.equal(downloads.length, 4, 'PDF uses the print dialog rather than claiming a saved file');
  await assert.rejects(
    callback(page, 'report', {
      ...common,
      reportFormat: 'pdf',
      invoke: async () => {
        throw new Error('native print failed');
      },
    })(),
    /native print failed/,
  );
  const css = readFileSync(new URL('../../styles.css', import.meta.url), 'utf8');
  assert.match(
    css,
    /body\s*>\s*\*:not\(#print-report\)\s*\{\s*display:\s*none\s*!important;?\s*\}/,
  );
  assert.match(css, /#print-report\s*\{\s*display:\s*block\s*!important/);
});

test('Results evaluation binds full metrics to the selected conditions and ignores stale responses', async () => {
  const page = '../results/ResultsPage.tsx';
  const rows = [
    { name: 'Qle', model_var: 'f_lfevpa', n: 9000, time: [0, 1], model: [1, 2], obs: [2, 3] },
  ];
  const published: Record<string, unknown>[] = [],
    calls: Record<string, unknown>[] = [];
  // evaluate 发布的是「条件快照」：key 与 request 在发起时定死，晚返回的结果不能写进来。
  const singleRequest = {
    caseDir: '/case',
    versionId: 'official',
    revision: 1,
    source: '/run',
    obs: '/obs',
    from: '',
    to: '',
    spinup: '0',
    corrected: false,
  };
  const requestSeq = { current: { single: 0, multi: 0 } };
  const context = {
    task: async (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    singleRequest,
    requestSeq,
    selectionKey: 'verified-selection',
    tr: (zh: string) => zh,
    windowArgs: () => ({ from: null, to: null }),
    setSingleResult: (value: Record<string, unknown>) => published.push(value),
    setSettingsOpen: () => {},
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push(args);
      return command === 'evaluation_catalog' ? [{ name: 'Qle', available: true }] : rows;
    },
  };
  assert.equal(await callback(page, 'evaluate', context)(), rows);
  const snapshot = published.at(-1)!;
  assert.equal(snapshot.scope, 'single');
  assert.equal(snapshot.key, 'verified-selection');
  assert.deepEqual(snapshot.request, singleRequest);
  assert.deepEqual(snapshot.rows, rows);
  assert.deepEqual(snapshot.evaluation, [{ name: 'Qle', available: true }]);
  assert.equal(typeof snapshot.completedAt, 'number');
  assert.equal(calls[1].summaryOnly, false);
  assert.deepEqual(calls[1].pairVars, ['Qle']);
  assert.equal(calls[1].spinup, 0, 'metrics uses the request snapshot spinup');
  published.length = 0;
  let current = true;
  await callback(page, 'evaluate', {
    ...context,
    task: async (fn: (current: () => boolean) => Promise<unknown>) => fn(() => current),
    invokeJson: async (command: string) => {
      if (command === 'evaluation_catalog') return [{ name: 'Qle', available: true }];
      current = false;
      return rows;
    },
  })();
  assert.equal(
    published.length,
    0,
    'late response cannot overwrite metrics or the overview binding',
  );
  assert.deepEqual(
    await callback(page, 'evaluate', {
      ...context,
      invokeJson: async (command: string) => {
        assert.equal(command, 'evaluation_catalog', 'no metrics query without shared variables');
        return [{ name: 'FCH4', available: false, missing_model: ['f_methane_surf_flux_tot'] }];
      },
    })(),
    [],
  );
  const empty = published.at(-1)!;
  assert.equal(empty.key, 'verified-selection');
  assert.deepEqual(empty.rows, []);
  assert.deepEqual(empty.evaluation, [
    { name: 'FCH4', available: false, missing_model: ['f_methane_surf_flux_tot'] },
  ]);
});

test('result navigation opens 新建修改版 from the model-improvement action', async () => {
  const updates: unknown[] = [],
    routes: string[] = [];
  const improve = callback('../results/ResultsPage.tsx', 'improveModel', {
    update: (patch: unknown) => updates.push(patch),
    navigate: (route: string) => {
      routes.push(route);
      return true;
    },
  }) as unknown as () => boolean;
  assert.equal(improve(), true);
  // 模型改进 goes to 新建修改版 (research has no sidebar entry of its own).
  assert.deepEqual(updates, [{ versionsView: 'create' }]);
  assert.deepEqual(routes, ['versions']);
});

test('overview contract exposes optional evaluation conditions only while advanced settings are visible', () => {
  const page = readFileSync(new URL('../results/ResultsPage.tsx', import.meta.url), 'utf8');
  const start = page.indexOf('const fields: UiField[] ='),
    end = page.indexOf('const displayedSeries', start);
  const js = ts.transpileModule(page.slice(start, end), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const noop = () => {};
  const context = {
    tr: (zh: string) => zh,
    state: { cases: [] },
    caseDir: '/case',
    en: false,
    source: '/run',
    runs: [],
    tabs: ['overview', 'catalog', 'series', 'evaluation', 'comparison', 'diagnostics', 'report'],
    tabIndex: (name: string) =>
      [
        'overview',
        'catalog',
        'series',
        'evaluation',
        'comparison',
        'diagnostics',
        'report',
      ].indexOf(name),
    // 分析范围（单站点/多站点）与"切回原站点页签"的 ref 也被 fields 回调引用。
    multiStation: false,
    // 窗口标签按时间基准显示（表单里的「（UTC）」不再硬编码）。
    timeBasisText: 'UTC',
    lastStationTab: { current: 'overview' },
    // 多站点比较的契约字段把结构化条目序列化成文本；这两个函数也在 fields 回调里被调用。
    comparisonEntries: [],
    comparisonText: () => '',
    applyComparisonText: () => {},
    // 页签的中英文字面量：源文件里的 tabLabels 对象也被 `fields` 回调引用。
    tabLabels: {
      en: [
        'Overview',
        'Data catalog',
        'Time series',
        'Observation evaluation',
        'Site comparison',
        'Diagnostics',
        'Report',
      ],
      zh: ['结果总览', '数据目录', '时序分析', '观测评估', '多站比较', '闭合诊断', '报告导出'],
    },
    tab: 'overview',
    series: null,
    diagnostic: null,
    advanced: false,
    settingsOpen: false,
    runChooser: false,
    busy: false,
    from: '',
    to: '',
    variable: 'f_lfevpa',
    catalog: { variables: [] },
    obs: '',
    spinup: '0',
    corrected: false,
    comparison: '',
    reportFormat: 'json',
    overviewRunLabel: () => '',
    chartVariables,
    variableOption,
    setSource: noop,
    chooseRecordedRun: noop,
    setTab: noop,
    setAdvanced: noop,
    setFrom: noop,
    setTo: noop,
    setVariable: noop,
    chooseObservation: noop,
    setSpinup: noop,
    setCorrected: noop,
    setComparison: noop,
    setReportFormat: noop,
  };
  const fields = (extra: Record<string, unknown>) => {
    const values = { ...context, ...extra };
    return new Function(...Object.keys(values), `${js};return fields;`)(
      ...Object.values(values),
    ) as { id: string; set: (value: string) => void }[];
  };
  const hidden = fields({});
  assert.ok(!hidden.some((field) => field.id === 'result-advanced'));
  assert.ok(!hidden.some((field) => field.id === 'result-observation'));
  assert.ok(!hidden.some((field) => field.id === 'result-source'));
  assert.deepEqual(
    fields({ runChooser: true }).map((field) => field.id),
    ['result-source'],
  );
  assert.ok(fields({ settingsOpen: true }).some((field) => field.id === 'result-observation'));
  for (const id of ['result-from', 'result-to', 'result-spinup', 'result-corrected']) {
    assert.ok(!hidden.some((field) => field.id === id), `${id} must be hidden`);
    assert.ok(
      fields({ settingsOpen: true, advanced: true }).some((field) => field.id === id),
      `${id} must be available when expanded`,
    );
    assert.ok(
      fields({ tab: 'evaluation' }).some((field) => field.id === id),
      `${id} stays in the original evaluation tab`,
    );
  }
  let expanded = false;
  fields({
    settingsOpen: true,
    setAdvanced: (value: boolean) => {
      expanded = value;
    },
  })
    .find((field) => field.id === 'result-advanced')!
    .set('true');
  assert.equal(expanded, true);
});

test('automatic result preview requires the current catalog and rejects late replies', async () => {
  const path = new URL('../results/ResultsPage.tsx', import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  let body = '';
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.arguments[0]?.getText(source).includes('readSeries(variable)')
    )
      body = node.arguments[0].getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const js = ts.transpileModule(`const effect=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const shown: unknown[] = [],
    errors: unknown[] = [];
  let reads = 0;
  let resolve!: (value: unknown) => void;
  const pending = new Promise((resolveData) => {
    resolve = resolveData;
  });
  const context = {
    tab: 'overview',
    stationTarget: { current: '/case' },
    caseDir: '/case',
    source: '/current',
    variable: 'f_lfevpa',
    catalogKey: '/current',
    selectedSeriesKey: 'current-series',
    readSeries: () => {
      reads++;
      return pending;
    },
    setSeries: (value: unknown) => shown.push(value),
    setSeriesKey: (value: unknown) => shown.push(value),
    setError: (value: unknown) => errors.push(value),
  };
  const effect = (extra: Record<string, unknown>) => {
    const values = { ...context, ...extra };
    return new Function(...Object.keys(values), `${js};return effect;`)(
      ...Object.values(values),
    ) as () => undefined | (() => void);
  };
  effect({ catalogKey: '/old' })();
  assert.equal(reads, 0, 'a prior run catalog cannot trigger an unverified variable read');
  const cancel = effect({})();
  assert.equal(reads, 1);
  cancel?.();
  resolve({ time: [1], vars: { f_lfevpa: [2] } });
  await pending;
  await Promise.resolve();
  assert.deepEqual(shown, []);
  assert.deepEqual(errors, []);
  effect({ readSeries: async () => ({ time: [1], vars: { f_lfevpa: [2] } }) })();
  await Promise.resolve();
  await Promise.resolve();
  assert.equal(shown.at(-1), 'current-series');
});

test('result time-window changes clear sampled curves while observation edits preserve the simulation', () => {
  const path = new URL('../results/ResultsPage.tsx', import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  const effects: { dependencies: string; body: string }[] = [];
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.arguments[1]
    )
      effects.push({
        dependencies: node.arguments[1].getText(source).replace(/\s+/g, ''),
        body: node.arguments[0].getText(source),
      });
    ts.forEachChild(node, visit);
  };
  visit(source);
  let curve: unknown = 'old-window-series',
    key = 'old-window';
  const noop = () => {};
  const context = {
    epoch: { current: 0 },
    requestSeq: { current: { single: 0, multi: 0 } },
    setSeries: (value: unknown) => {
      curve = value;
    },
    setSeriesKey: (value: string) => {
      key = value;
    },
    setSingleResult: noop,
    setDiagnostic: noop,
  };
  const run = (effect: { body: string }) => {
    const js = ts.transpileModule(`const effect=(${effect.body});`, {
      compilerOptions: { target: ts.ScriptTarget.ES2022 },
    }).outputText;
    new Function(...Object.keys(context), `${js};effect();`)(...Object.values(context));
  };
  const observations = effects.find((effect) => effect.dependencies.includes('obs'));
  assert.ok(observations);
  run(observations);
  assert.equal(curve, 'old-window-series');
  assert.equal(key, 'old-window');
  const window = effects.find((effect) => effect.dependencies === '[from,to]');
  assert.ok(window);
  run(window);
  assert.equal(curve, null);
  assert.equal(key, '');
});

test('preparation sends only active case or pending case settings to native data generation', async () => {
  const { casePreparationOptions } = await import('./case-model');
  const legacy = {
    physics: { DEF_URBAN_RUN: '.true.', DEF_TUNING_CROP_PLANTING_DAY: '0', DEF_USE_PC: '.true.' },
    parameters: [],
  };
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const state = {
    version: { overlay: legacy },
    selectedCase: {
      profile: { classification: 'USGS' as const, subgrid: 'USGS', urban: false, crop: false },
    },
  };
  const produced: Record<string, string>[] = [];
  const run = async (preparation: ReturnType<typeof casePreparationOptions>, mode: string) => {
    const execute = callback(prep, 'execute', {
      // 做好的站点文件记下来，"用于新算例"带着它去新建模拟。
      setProduced: (next: (old: Record<string, string>) => Record<string, string>) =>
        produced.push(next({})),
      work: async (fn: () => Promise<unknown>) => fn(),
      state,
      preparation,
      mode,
      form: { dst: '/new-data.nc', rawdata: '/raw' },
      num: () => 1,
      optional: () => null,
      invoke: async (command: string, args: Record<string, unknown>) => {
        calls.push({ command, args });
        return [];
      },
      probe: {},
      choices: () => [],
      heights: () => null,
      setImported: () => {},
    });
    await execute();
    return calls.at(-1)!;
  };
  let call = await run(casePreparationOptions(state.selectedCase.profile), 'site');
  assert.deepEqual(produced.at(-1), { site: '/new-data.nc' });
  assert.equal(call.args.mode, 'usgs');
  assert.equal(call.args.crop, false);
  call = await run(
    casePreparationOptions(state.selectedCase.profile, {
      form: { subgrid: 'PFT', landcover: 'IGBP' },
      caseValues: { DEF_TUNING_CROP_PLANTING_DAY: '120' },
    }),
    'site',
  );
  assert.equal(call.args.mode, 'pft');
  assert.equal(call.args.crop, true);
  call = await run(
    casePreparationOptions(
      { classification: 'IGBP', subgrid: 'PFT', urban: true, crop: true },
      { form: { landcover: 'USGS' } },
    ),
    'site',
  );
  assert.equal(call.args.mode, 'usgs');
  assert.equal(call.args.crop, false);
  call = await run(casePreparationOptions(state.selectedCase.profile), 'table');
  assert.equal((call.args.options as { land_cover_scheme: string }).land_cover_scheme, 'usgs');
  assert.equal(legacy.physics.DEF_USE_PC, '.true.');
});

test('process saves refresh history availability in both directions without resetting drafts', async () => {
  for (const bgc of [true, false]) {
    const request = { active: true },
      epoch = { current: request };
    let drafts: Record<string, string> = { 'DEF_hist_vars%ar': '.false.', DEF_other: 'draft' };
    let histories: unknown[] = [],
      reasons: Record<string, string> = {},
      disabled: string[] = [];
    const context = { entry: { dir: '/new-preview' }, profile: { bgc } };
    const refreshHistory = callback('./CasesPage.tsx', 'refreshHistory', {
      epoch,
      state: { kernels: [{ dir: '/new-kernel' }] },
      compatibleKernel: (_kernels: unknown, profile: unknown) => {
        assert.equal(profile, context.profile);
        return { dir: '/new-kernel' };
      },
      invoke: async (name: string, args: Record<string, unknown>) => {
        if (name === 'read_case_nml') {
          assert.equal(args.caseDir, '/new-preview');
          return 'effective';
        }
        assert.equal(name, 'hist_vars');
        assert.deepEqual(args, {
          text: 'effective',
          kernelDir: '/new-kernel',
          dir: '/new-preview',
        });
        return [
          {
            name: 'ar',
            on: true,
            settable: true,
            writable: bgc,
            blocked_by: bgc ? undefined : 'requires BGC',
          },
        ];
      },
      setHistoryVars: (v: unknown[]) => {
        histories = v;
      },
      setContextReason: (v: Record<string, string>) => {
        reasons = v;
      },
      setDisabledFields: (v: string[]) => {
        disabled = v;
      },
    });
    const change = callback('./CasesPage.tsx', 'changeCaseOptions', {
      dir: '/case',
      editingDisabled: false,
      dirty: false,
      lock: { current: false },
      t: (_zh: string, en: string) => en,
      physics: {},
      values: drafts,
      effectiveProfile: {},
      configurationKernel: undefined,
      exclusionPatch: () => ({ patch: { DEF_USE_BGC: String(bgc) }, closed: [] }),
      caseSimulationDomain,
      work: (fn: () => Promise<unknown>) => fn(),
      epoch,
      state: { versionRef: {}, selectedCase: undefined },
      versionId: 'official',
      revision: 1,
      invoke: async (name: string) => (name === 'model_case_context' ? context : undefined),
      setModelContext: () => {},
      setValues: (apply: (old: Record<string, string>) => Record<string, string>) => {
        drafts = apply(drafts);
      },
      setSaved: () => {},
      setExplicitValues: () => {},
      setError: () => {},
      refreshHistory,
    });
    await change({ DEF_USE_BGC: String(bgc) });
    assert.deepEqual(histories, [
      { name: 'ar', settable: bgc, reason: bgc ? undefined : 'requires BGC', longName: undefined },
    ]);
    assert.deepEqual(disabled, bgc ? [] : ['DEF_hist_vars%ar']);
    assert.deepEqual(reasons, bgc ? {} : { 'DEF_hist_vars%ar': 'requires BGC' });
    assert.deepEqual(drafts, {
      'DEF_hist_vars%ar': '.false.',
      DEF_other: 'draft',
      DEF_USE_BGC: String(bgc),
    });
  }
});

test('history refresh ignores a response after leaving the case', async () => {
  const request = { active: true },
    epoch = { current: request };
  const refresh = callback('./CasesPage.tsx', 'refreshHistory', {
    epoch,
    state: { kernels: [] },
    compatibleKernel: () => ({ dir: '/kernel' }),
    invoke: async (name: string) => {
      if (name === 'read_case_nml') return 'effective';
      request.active = false;
      return [];
    },
    setHistoryVars: () => assert.fail('stale history update'),
    setDisabledFields: () => assert.fail('stale disabled update'),
    setContextReason: () => assert.fail('stale reason update'),
  });
  await refresh({ entry: { dir: '/old-preview' }, profile: {} }, request);
});
