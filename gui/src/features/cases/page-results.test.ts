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
  const reportData = callback(page, 'reportData', {
    source: '/copy',
    runs: [],
    catalog: { steps: 2 },
    evaluation: [],
    metrics: [metric],
    diagnostic: null,
    baseline: null,
    baselineNote: '',
    obs: '/obs.csv',
    from: '',
    to: '',
    spinup: '0',
    corrected: false,
    comparison: '',
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
    en: true,
    reportFile,
    setExportNotice: noop,
    exportMessage: callback(page, 'exportMessage', { tr: (zh: string) => zh }),
    tr: (zh: string) => zh,
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
  const published: unknown[] = [],
    calls: Record<string, unknown>[] = [];
  const context = {
    task: async (fn: (current: () => boolean) => Promise<unknown>) => fn(() => true),
    obs: '/obs',
    spinup: '0',
    source: '/run',
    corrected: false,
    selectionKey: 'verified-selection',
    tr: (zh: string) => zh,
    windowArgs: () => ({ from: null, to: null }),
    setEvaluation: () => {},
    setMetrics: (value: unknown) => published.push(value),
    setSingleEvaluation: (value: unknown) => published.push(value),
    setSettingsOpen: () => {},
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push(args);
      return command === 'evaluation_catalog' ? [{ name: 'Qle', available: true }] : rows;
    },
  };
  assert.equal(await callback(page, 'evaluate', context)(), rows);
  assert.deepEqual(published.at(-1), { key: 'verified-selection', rows });
  assert.equal(calls[1].summaryOnly, false);
  assert.deepEqual(calls[1].pairVars, ['Qle']);
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
  assert.deepEqual(published.at(-1), { key: 'verified-selection', rows: [] });
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
    setSeries: (value: unknown) => {
      curve = value;
    },
    setSeriesKey: (value: string) => {
      key = value;
    },
    setMetrics: noop,
    setDiagnostic: noop,
    setSingleEvaluation: noop,
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
