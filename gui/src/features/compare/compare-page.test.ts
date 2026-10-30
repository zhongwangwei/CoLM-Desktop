import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as compareConfig from './config';
import type { CompareSnapshot } from './config';
import {
  buildOpenbenchConfig,
  configInvalidation,
  designFingerprint,
  simulationFormForPick,
  simulationInfoForSave,
  validateSimulation,
  type ComparisonDesign,
  type EvaluationDesign,
  type HistoryInfo,
  type SimulationField,
  type SimulationFormValues,
  type SimulationIssue,
} from './compare';

/**
 * 这些用例执行的是 ComparePage.tsx 里的**真实函数**（用 TypeScript AST 从页面源码抽出来，再用
 * `new Function` 在受控上下文里跑），不是对源码文本的断言：证明保存模拟、读概况、生成/运行配置、
 * 配置失效这些用户行为在真实页面逻辑下的结果（GUI-04/09/10/11）。
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

/** 取页面里一个 `const name = ...` 声明（保持真实源码）。 */
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
  // GUI-05 之后页面函数会核对机器代次（hostEpoch/stillCurrent），有的还会写提示（setNotice）、
  // 标记评估配置的保存状态（U-05）；不关心这些的用例给一个"始终属于当前上下文"的默认值。
  const withDefaults = {
    hostEpoch: { current: 0 },
    stillCurrent: () => true,
    setNotice: () => {},
    setConfigSaving: () => {},
    setWrittenFingerprint: () => {},
    benchTasks: { begin: () => '', update: () => {}, finish: () => {}, clear: () => {} },
    benchTaskId: { current: '' },
    store: () => {},
    compareDraftKey: (host: string) => `colm.compareDraft@${host}`,
    showRunNotice: () => {},
    ...context,
  };
  const code = ts.transpileModule(`${declaration(name)}; return ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(withDefaults), code)(...Object.values(withDefaults)) as T;
}

type Form = SimulationFormValues & {
  label: string;
  suffix: string;
  probe: HistoryInfo | null;
  probeDir: string;
};
type Pick = {
  dir: string;
  label: string;
  info: HistoryInfo | null;
  variables?: Record<string, { varname: string; varunit: string }>;
  dataGroupby?: string | null;
  gridRes?: number | null;
  profile?: string | null;
  stationId?: string | null;
  error?: string;
};
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

const baseForm = (patch: Partial<Form> = {}): Form => ({
  dir: '',
  label: '',
  dataType: 'auto',
  prefix: '',
  suffix: '',
  timRes: 'Day',
  from: '',
  to: '',
  dataGroupby: '',
  gridRes: '',
  profile: 'CoLM2024',
  stationId: '',
  lon: '',
  lat: '',
  extent: '',
  variables: [],
  probe: null,
  probeDir: '',
  ...patch,
});

/** 执行页面里的一个模块级常量/函数声明（测试不要复制一份会漂移的实现）。 */
function evaluate<T>(name: string): T {
  const code = ts.transpileModule(`${declaration(name)}; return ${name};`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(code)() as T;
}

/** 页面真实的 `pickLabel`：名称只来自路径。 */
const pagePickLabel = evaluate<(dir: string) => string>('pickLabel');
const pageClearProbe = evaluate<Record<string, string>>('CLEAR_PROBED_SIM_FIELDS');
const t = (_zh: string, english: string) => english;

/** 与页面 generate() 同一条路径生成的冻结快照；测试用它模拟"已生成、尚未改动"的状态。 */
function snapshotFor(design: EvaluationDesign): CompareSnapshot {
  return compareConfig.freezeCompareSnapshot(design, buildOpenbenchConfig(design));
}

/** 跑真实的 saveSimulation，收集它给出的字段问题与最终写入的 pick。 */
function saveSimulationRun(form: Form, existing: Pick[] = []) {
  const issues: SimulationIssue[] = [];
  const errors: string[] = [];
  const picks: Pick[] = existing.map((pick) => ({ ...pick }));
  let dialog: string | null = 'simulation';
  const save = execute<() => void>('saveSimulation', {
    simForm: form,
    picks,
    // 页面上"编辑已有模拟"先记下打开弹窗的那条目录；用 existing[0] 模拟这个状态。
    simEditing: existing[0]?.dir ?? null,
    validateSimulation,
    simulationInfoForSave,
    showSimulationIssue: (issue: SimulationIssue) => issues.push(issue),
    setPicks: (update: (old: Pick[]) => Pick[]) => {
      picks.splice(0, picks.length, ...update(picks));
    },
    setDialog: (value: string | null) => {
      dialog = value;
    },
    setSimEditing: () => {},
    setSimError: () => {},
    setError: (value: string) => errors.push(value),
    shown: (error: unknown) => String(error),
    t,
    pickLabel: pagePickLabel,
  });
  save();
  return { issues, errors, picks, dialog: () => dialog };
}

test('a failed save shows the field error and keeps the dialog, input and current picks', () => {
  // GUI-11：空目录、未确认类型、缺起止年份都由真实 saveSimulation 定位到字段。
  const empty = saveSimulationRun(baseForm());
  assert.equal(empty.issues[0]?.field, 'dir');
  assert.equal(empty.dialog(), 'simulation', 'the dialog stays open so the input is not lost');
  assert.equal(empty.picks.length, 0);
  assert.equal(empty.errors.length, 0, 'a validation problem is not a program exception');

  const untyped = saveSimulationRun(baseForm({ dir: '/d' }));
  assert.equal(untyped.issues[0]?.field, 'dataType');
  assert.match(untyped.issues[0]!.message[1], /Inspect the folder/);

  const noYears = saveSimulationRun(baseForm({ dir: '/d', dataType: 'grid' }));
  assert.equal(noYears.issues[0]?.field, 'from');
  assert.match(noYears.issues[0]!.message[1], /start and end years/);
});

test('the first offending field receives the focus and the message is visible', () => {
  const queried: string[] = [];
  const focused: string[] = [];
  const observed = globalThis.document;
  Object.defineProperty(globalThis, 'document', {
    configurable: true,
    value: {
      documentElement: { lang: 'en' },
      querySelector: (selector: string) => {
        queried.push(selector);
        return { focus: () => focused.push(selector) };
      },
    },
  });
  try {
    const errors: { field: SimulationField; message: string }[] = [];
    const show = execute<(issue: SimulationIssue) => void>('showSimulationIssue', {
      setSimError: (value: { field: SimulationField; message: string }) => errors.push(value),
      t,
    });
    show({ field: 'from', message: ['请填起止年份。', 'Enter start and end years.'] });
    assert.deepEqual(errors, [{ field: 'from', message: 'Enter start and end years.' }]);
    assert.deepEqual(queried, ['[data-sim-field="from"]']);
    assert.deepEqual(focused, ['[data-sim-field="from"]']);
  } finally {
    Object.defineProperty(globalThis, 'document', { configurable: true, value: observed });
  }
});

test('a backend inspection failure is shown and the same form can retry', async () => {
  let attempts = 0;
  let form = baseForm({ dir: '/p/hb/out/hb/history' });
  const errors: string[] = [];
  const inspect = execute<() => Promise<unknown>>('inspectSimulation', {
    simForm: form,
    showSimulationIssue: () => {},
    task: async (fn: () => Promise<unknown>) => {
      try {
        await fn();
      } catch (error) {
        errors.push(String(error));
      }
    },
    invokeJson: async () => {
      attempts++;
      if (attempts === 1) throw new Error('读不了这个目录');
      return probedGrid;
    },
    remote: false,
    host: '',
    setSimForm: (update: (old: Form) => Form) => {
      form = update(form);
    },
    pickLabel: pagePickLabel,
    simulationFormForPick,
    setSimError: () => {},
    t,
  });
  await inspect();
  assert.deepEqual(errors, ['Error: 读不了这个目录'], 'the read failure is visible, not swallowed');
  assert.equal(attempts, 1);
  assert.equal(form.probe, null, 'a failed read does not fake a probe');
  assert.equal(form.dir, '/p/hb/out/hb/history', 'the typed folder is kept for the retry');
  // 同一个表单再按一次"读取概况"：这次成功，说明失败后仍可重试。
  await inspect();
  assert.equal(attempts, 2);
  assert.deepEqual(errors.length, 1);
  assert.equal(form.probe, probedGrid);
  assert.equal(form.probeDir, probedGrid.history_dir);
});

test('a slow inspection cannot write the old folder probe into a folder the user just switched to', async () => {
  let form = baseForm({ dir: '/p/old/history' });
  let resolveProbe: (info: HistoryInfo) => void = () => {};
  const pending = new Promise<HistoryInfo>((resolve) => {
    resolveProbe = resolve;
  });
  const inspect = execute<() => Promise<unknown>>('inspectSimulation', {
    simForm: form,
    showSimulationIssue: () => {},
    task: async (fn: () => Promise<unknown>) => {
      await fn();
    },
    invokeJson: async () => pending,
    remote: false,
    host: '',
    setSimForm: (update: (old: Form) => Form) => {
      form = update(form);
    },
    pickLabel: pagePickLabel,
    simulationFormForPick,
    setSimError: () => {},
    t,
  });
  const reading = inspect();
  // 读取还没回来，用户把目录改成了另一个（真实页面里 probeDir 已经不匹配）。
  form = baseForm({ dir: '/p/new/history' });
  resolveProbe({ ...probedGrid, history_dir: '/p/old/history' });
  await reading;
  assert.equal(form.probe, null, 'the old folder result is dropped');
  assert.equal(form.probeDir, '');
  assert.equal(form.dir, '/p/new/history');
});

test('a successful inspection fills the form and keeps the whole probe object', async () => {
  let form = baseForm({ dir: probedGrid.history_dir });
  const errors: string[] = [];
  const inspect = execute<() => Promise<unknown>>('inspectSimulation', {
    simForm: form,
    showSimulationIssue: () => {},
    task: async (fn: () => Promise<unknown>) => {
      try {
        await fn();
      } catch (error) {
        errors.push(String(error));
      }
    },
    invokeJson: async () => probedGrid,
    remote: false,
    host: '',
    setSimForm: (update: (old: Form) => Form) => {
      form = update(form);
    },
    pickLabel: pagePickLabel,
    simulationFormForPick,
    setSimError: () => {},
    t,
  });
  await inspect();
  assert.deepEqual(errors, []);
  assert.equal(form.probe, probedGrid, 'the full probe object is kept for saving');
  assert.equal(form.probeDir, probedGrid.history_dir);
  assert.equal(form.dataType, 'grid');
  assert.equal(form.extent, '111,115.5,21.5,24.5');
  assert.equal(form.gridRes, '0.008333');
  assert.equal(form.from, '2002');
  assert.equal(form.to, '2005');
});

test('changing the folder retires the old probe and its metadata', () => {
  let form = baseForm({
    dir: probedGrid.history_dir,
    dataType: 'grid',
    prefix: 'hb_hist_',
    from: '2002',
    to: '2005',
    gridRes: '0.008333',
    lon: '11.3',
    lat: '47.1',
    extent: '111,115.5,21.5,24.5',
    probe: probedGrid,
    probeDir: probedGrid.history_dir,
  });
  const change = execute<(dir: string) => void>('changeSimulationDir', {
    CLEAR_PROBED_SIM_FIELDS: pageClearProbe,
    setSimForm: (update: (old: Form) => Form) => {
      form = update(form);
    },
  });
  change('/p/another/history');
  assert.equal(form.dir, '/p/another/history');
  assert.equal(form.probe, null, 'the old folder probe is retired (GUI-10)');
  assert.equal(form.probeDir, '');
  assert.equal(form.prefix, '');
  assert.equal(form.from, '');
  assert.equal(form.to, '');
  assert.equal(form.extent, '');
  assert.equal(form.gridRes, '');
  assert.equal(form.lon, '');
  assert.equal(form.lat, '');
  // 用新目录保存时不能拿旧目录的年份凑：自动类型没有新探测结果 → 明确提示先读数或选类型。
  const result = saveSimulationRun(form);
  assert.equal(result.issues[0]?.field, 'dataType');
  assert.equal(result.picks.length, 0);
});

test('saving a type change persists the new type and the whole probe', () => {
  // GUI-09：格点改站点后保存，重新打开表单显示站点，生成配置按站点走并同步站点清单。
  const existing: Pick[] = [{ dir: probedGrid.history_dir, label: 'hb', info: probedGrid }];
  const form: Form = {
    ...baseForm({
      ...simulationFormForPick(existing[0]!),
      probe: probedGrid,
      probeDir: probedGrid.history_dir,
    }),
    dataType: 'stn',
  };
  const result = saveSimulationRun(form, existing);
  assert.deepEqual(result.issues, []);
  assert.equal(result.dialog(), null, 'a valid save closes the dialog');
  const saved = result.picks[0]!;
  assert.equal(saved.info?.spatial, false, 'grid → station really changes the saved type');
  assert.equal(
    saved.info?.files,
    probedGrid.files,
    'the probed metadata is kept across the type change',
  );
  assert.equal(
    simulationFormForPick(saved).dataType,
    'stn',
    'reopening the form shows the new type',
  );
  const built = buildOpenbenchConfig({
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: saved.label, info: saved.info!, variables: saved.variables }],
    projectDir: '/ob/projects/comparison',
  });
  assert.match(built.config, /data_type: stn/);
  assert.doesNotMatch(built.config, /data_type: grid/);
  assert.deepEqual(built.sites, [
    {
      id: 'hb',
      station: 'hb',
      history_dir: probedGrid.history_dir,
      prefix: 'hb_hist_',
      // 探测到的坐标原样带上（这里 hb 是格点，坐标为空）。
      lon: probedGrid.lon,
      lat: probedGrid.lat,
    },
  ]);
});

test('saving with only an edited prefix keeps extent, coordinates and file count', () => {
  // GUI-10：读取→保存→生成配置，探测到的范围/经纬度/文件数都不能丢。
  const form: Form = {
    ...baseForm({
      ...simulationFormForPick({ dir: probedGrid.history_dir, label: 'hb', info: probedGrid }),
    }),
    prefix: 'edited_hist_',
    probe: probedGrid,
    probeDir: probedGrid.history_dir,
  };
  const result = saveSimulationRun(form, [
    { dir: probedGrid.history_dir, label: 'hb', info: probedGrid },
  ]);
  assert.deepEqual(result.issues, []);
  const saved = result.picks[0]!.info!;
  assert.equal(saved.prefix, 'edited_hist_');
  assert.deepEqual(saved.extent, probedGrid.extent);
  assert.equal(saved.lon, probedGrid.lon);
  assert.equal(saved.lat, probedGrid.lat);
  assert.equal(saved.files, probedGrid.files);
  assert.equal(saved.history_dir, probedGrid.history_dir);
  const built = buildOpenbenchConfig({
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: 'hb', info: saved }],
    projectDir: '/ob/projects/comparison',
    gridRes: 0.25,
  });
  assert.match(built.config, /lat_range: \[21\.5, 24\.5\]/);
  assert.match(built.config, /grid_res: 0\.25/);
});

test('generating freezes the design, text, station list and fingerprint into one snapshot', () => {
  const design: ComparisonDesign = {
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: 'hb', info: probedGrid }],
    projectDir: '/ob/projects/comparison',
    gridRes: 0.25,
  };
  const machine = { host: '', openbenchRoot: '/ob', refRoot: '/refs' };
  const formDesign: EvaluationDesign = { machine, ...design };
  let generatedValue: CompareSnapshot | null = null;
  let yaml = '';
  let missingExtent: string[] = [];
  const errors: string[] = [];
  const generate = execute<() => void>('generate', {
    comparisonDesign: () => design,
    buildOpenbenchConfig,
    machine,
    freezeCompareSnapshot: compareConfig.freezeCompareSnapshot,
    generated: {
      get current() {
        return generatedValue;
      },
      set current(value: CompareSnapshot | null) {
        generatedValue = value;
      },
    },
    setYaml: (value: string) => {
      yaml = value;
    },
    setMissingExtent: (value: string[]) => {
      missingExtent = value;
    },
    setError: (value: string) => errors.push(value),
    shown: (error: unknown) => String(error),
  });
  generate();
  assert.ok(
    errors.every((message) => !message),
    'generating a valid design reports no error',
  );
  const built = buildOpenbenchConfig(design);
  // 配置文本、站点清单、设计指纹和一个有效指纹是同一份快照（R01）。
  assert.equal(generatedValue!.generatedText, built.config);
  assert.deepEqual(generatedValue!.sites, built.sites);
  assert.equal(generatedValue!.designFingerprint, designFingerprint(formDesign));
  assert.equal(
    generatedValue!.fingerprint,
    compareConfig.configFingerprint(
      formDesign,
      compareConfig.parseOpenbenchConfig(built.config).config!,
      built.sites,
    ),
    'the fingerprint is recorded together with the text and station list it came from',
  );
  assert.equal(yaml, built.config);
  assert.deepEqual(missingExtent, built.missingExtent);
  // 设置一变，同一份记录就对不上当前设计 → 快照过期（GUI-04/R01）。
  assert.equal(
    compareConfig.snapshotStale(generatedValue!, { ...formDesign, timRes: 'Hour' }),
    true,
  );
});

test('an edited per-simulation grouping or resolution override really reaches the configuration', () => {
  // GUI-09/10 的相邻缺口：pick 上的手工覆盖原来会在保存后继续盖住表单里改过的分组/分辨率。
  const existing: Pick[] = [
    {
      dir: probedGrid.history_dir,
      label: 'hb',
      info: probedGrid,
      dataGroupby: 'Month',
      gridRes: 0.5,
    },
  ];
  const form: Form = {
    ...baseForm({
      ...simulationFormForPick(existing[0]!),
      probe: probedGrid,
      probeDir: probedGrid.history_dir,
    }),
    dataGroupby: 'Day',
    gridRes: '0.25',
  };
  const result = saveSimulationRun(form, existing);
  assert.deepEqual(result.issues, []);
  const saved = result.picks[0]!;
  assert.equal(saved.dataGroupby, null, 'the stale per-pick override is cleared');
  assert.equal(saved.gridRes, null);
  const built = buildOpenbenchConfig({
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [
      {
        label: saved.label,
        info: saved.info!,
        dataGroupby: saved.dataGroupby ?? undefined,
        gridRes: saved.gridRes ?? undefined,
      },
    ],
    projectDir: '/ob/projects/comparison',
  });
  assert.match(built.config, /data_groupby: Day/);
  assert.match(built.config, /grid_res: 0\.25/);
});

test('running refuses a missing/stale snapshot and configurations that cannot run', async () => {
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const machine = { host: '', openbenchRoot: '/ob', refRoot: '/refs' };
  const simulation = {
    label: 'AT-Neu',
    info: {
      ...probedGrid,
      history_dir: '/p/AT-Neu/history',
      prefix: 'AT-Neu_hist_',
      tim_res: 'Hour',
      spatial: false,
      extent: null,
      lon: 11.3,
      lat: 47.1,
    },
  };
  const baseDesign: ComparisonDesign = {
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5' },
    simulations: [simulation],
    projectDir: '/ob/projects/comparison',
  };
  const formDesign: EvaluationDesign = { machine, ...baseDesign };
  const built = buildOpenbenchConfig(baseDesign);
  const snapshot = snapshotFor(formDesign);
  assert.deepEqual(snapshot.sites, built.sites);
  const runWith = (args: {
    yaml?: string;
    snapshot?: CompareSnapshot | null;
    form?: EvaluationDesign;
  }) =>
    execute<() => Promise<unknown>>('run', {
      task: async (fn: () => Promise<unknown>) => fn(),
      generated: { current: args.snapshot === undefined ? snapshot : args.snapshot },
      formDesign: args.form ?? formDesign,
      designFingerprint,
      snapshotStale: compareConfig.snapshotStale,
      parseOpenbenchConfig: compareConfig.parseOpenbenchConfig,
      structuralProblems: compareConfig.structuralProblems,
      configFingerprint: compareConfig.configFingerprint,
      name: 'comparison',
      t,
      comparisonName: (comparison: string) => comparison,
      remote: false,
      invokeJson: async (command: string, args2: Record<string, unknown>) => {
        calls.push({ command, args: args2 });
        return { report: null };
      },
      yaml: args.yaml ?? built.config,
      sites: [],
      scanDir: '/refs',
      host: '',
      watch: async () => {},
      fetchReport: async () => {},
      view: async () => {},
      setLastRun: () => {},
      setConfigSaving: () => {},
      showRunNotice: () => {},
    });
  // 没有生成配置就没有可信的站点清单：明确拒绝，不发后端调用。
  await assert.rejects(runWith({ snapshot: null }), /No evaluation configuration yet/);
  // 生成之后改了站点目录（表单变化）→ 快照过期，明确阻断（R01 的 f0059ee 操作链）。
  const moved = {
    ...formDesign,
    simulations: [
      { ...simulation, info: { ...simulation.info, history_dir: '/p/elsewhere/history' } },
    ],
  };
  await assert.rejects(
    runWith({ form: moved }),
    /settings changed after the configuration was generated/,
  );
  // 手改出语法错误：明确拒绝。
  await assert.rejects(runWith({ yaml: 'project:\n  name: [unclosed\n' }), /cannot run/);
  // 手改删掉/改了单点站点清单路径：文件是后端按页面目录准备的，运行前就拒绝。
  const station = built.config.replace(
    'fulllist: "/ob/projects/comparison/sim/AT_Neu.csv"',
    'fulllist: "/tmp/stale.csv"',
  );
  await assert.rejects(runWith({ yaml: station }), /station list path of simulation/);
  // R07 剩余缺口：手改 YAML 把 varname 换成层/类别维度变量 → 按有效配置在运行前拒绝。
  const layerSimulation = {
    ...simulation,
    variables: { Latent_Heat: { varname: 'Qle', varunit: 'W/m2' } },
    info: {
      ...simulation.info,
      unsupported_variables: [
        { name: 'f_wliq_soisno', dims: ['soil_layer'], reason: 'layer dimension' },
      ],
    },
  };
  const layerDesign: ComparisonDesign = { ...baseDesign, simulations: [layerSimulation] };
  const layerBuilt = buildOpenbenchConfig(layerDesign);
  const layerForm: EvaluationDesign = { machine, ...layerDesign };
  await assert.rejects(
    runWith({
      form: layerForm,
      snapshot: snapshotFor(layerForm),
      yaml: layerBuilt.config.replace('varname: "Qle"', 'varname: "f_wliq_soisno"'),
    }),
    /beyond a station\/surface evaluation/,
  );
  assert.deepEqual(calls, [], 'nothing is sent to the backend for a configuration that cannot run');
});

test('a current configuration runs with exactly the effective config, frozen site list and fingerprint', async () => {
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const simulation = {
    label: 'AT-Neu',
    info: {
      ...probedGrid,
      history_dir: '/p/AT-Neu/history',
      prefix: 'AT-Neu_hist_',
      tim_res: 'Hour',
      spatial: false,
      extent: null,
      lon: 11.3,
      lat: 47.1,
    },
  };
  const baseDesign: ComparisonDesign = {
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [simulation],
    projectDir: '/ob/projects/comparison',
  };
  const built = buildOpenbenchConfig(baseDesign);
  const formDesign: EvaluationDesign = {
    machine: { host: '', openbenchRoot: '/ob', refRoot: '/refs' },
    ...baseDesign,
  };
  const snapshot = snapshotFor(formDesign);
  const notices: string[] = [];
  const runs: unknown[] = [];
  const run = execute<() => Promise<unknown>>('run', {
    task: async (fn: () => Promise<unknown>) => fn(),
    generated: { current: snapshot },
    formDesign,
    designFingerprint,
    snapshotStale: compareConfig.snapshotStale,
    parseOpenbenchConfig: compareConfig.parseOpenbenchConfig,
    structuralProblems: compareConfig.structuralProblems,
    configFingerprint: compareConfig.configFingerprint,
    name: 'comparison',
    t,
    comparisonName: (comparison: string) => comparison,
    remote: false,
    // 后端返回 run 身份：页面必须把它记成这次运行的结果，而不是"目录里找到了一份报告"。
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return { report: null, run_id: 'run-42', config_sha: 'abc123', state: 'succeeded' };
    },
    yaml: built.config,
    sites: [],
    scanDir: '/refs',
    host: '',
    view: async () => {},
    setLastRun: (value: unknown) => runs.push(value),
    showRunNotice: (message: string) => notices.push(message),
    setConfigSaving: () => {},
    watch: async () => ({}),
    fetchReport: async () => {},
  });
  await run();
  assert.equal(calls.length, 1);
  // 提交的是快照冻结的站点清单 + 当前配置文本（这里 == 生成文本）；二者一起进指纹。
  assert.deepEqual(calls[0]!.args, {
    name: 'comparison',
    config: built.config,
    sites: snapshot.sites,
    refRoot: '/refs',
  });
  const frozen = runs[0] as {
    runId: string;
    configSha: string;
    fingerprint: string;
    report: string | null;
  };
  assert.equal(frozen.runId, 'run-42');
  assert.equal(frozen.configSha, 'abc123');
  assert.equal(
    frozen.fingerprint,
    compareConfig.configFingerprint(
      formDesign,
      compareConfig.parseOpenbenchConfig(built.config).config!,
      snapshot.sites,
    ),
    'the recorded fingerprint covers the configuration and the frozen station list',
  );
  assert.equal(frozen.report, null);
  assert.match(notices[0] ?? '', /Evaluation finished \(run run-42\)/);
});

test('a changed setting clears an untouched configuration but keeps a hand-edited one', () => {
  const placeholderSnapshot = (fingerprint: string, generatedText: string): CompareSnapshot => ({
    design: {
      machine: { host: '', openbenchRoot: '', refRoot: '' },
      name: 'x',
      years: [2000, 2001],
      timRes: 'Day',
      variables: {},
      simulations: [],
      projectDir: '/p',
    },
    designFingerprint: fingerprint,
    sites: [],
    generatedText,
    fingerprint: 'effective',
  });
  const invalidate = (generatedValue: CompareSnapshot | null, yaml: string, design: string) => {
    const cleared: string[] = [];
    const clear = execute<() => void>('invalidateGeneratedConfig', {
      configInvalidation,
      design,
      generated: { current: generatedValue },
      generatedConfig: generatedValue,
      yaml,
      setYaml: (value: string) => cleared.push(value),
      setMissingExtent: () => {},
    });
    clear();
    return cleared;
  };
  // 没手改过的自动配置：随设置变化作废，要求重新生成。
  assert.deepEqual(invalidate(placeholderSnapshot('a', 'generated'), 'generated', 'b'), ['']);
  // 手改过的：原样保留（页面显示"已过期、不能运行"与重新生成入口）。
  assert.deepEqual(
    invalidate(placeholderSnapshot('a', 'generated'), 'generated + my edit', 'b'),
    [],
  );
  // 设计没变：什么都不动。
  assert.deepEqual(invalidate(placeholderSnapshot('a', 'generated'), 'generated', 'a'), []);
  // 记录不在（例如换了机器）时不误清用户输入。
  assert.deepEqual(invalidate(null, 'manual text', 'b'), []);
});

test('regenerating asks first and never silently replaces the user edits', async () => {
  let generated = 0;
  const regenerateNo = execute<() => Promise<void>>('regenerate', {
    yamlHandEdited: true,
    askConfirm: async () => false,
    t,
    generate: () => {
      generated++;
    },
  });
  await regenerateNo();
  assert.equal(generated, 0, 'without confirmation the hand-edited text is kept');

  let generatedWithConsent = 0;
  const regenerateYes = execute<() => Promise<void>>('regenerate', {
    yamlHandEdited: true,
    askConfirm: async () => true,
    t,
    generate: () => {
      generatedWithConsent++;
    },
  });
  await regenerateYes();
  assert.equal(generatedWithConsent, 1);
});

test('the pure comparison design helpers the page relies on are the ones under test', () => {
  // 防止页面以后绕开 compare.ts 自拼一份：这里引用到的两个函数都参与了上面的用例。
  assert.equal(typeof configInvalidation, 'function');
  assert.equal(typeof simulationInfoForSave, 'function');
  const design: ComparisonDesign = {
    name: 'x',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5LAND_LowRes' },
    simulations: [{ label: 'hb', info: probedGrid }],
    projectDir: '/ob/projects/x',
  };
  assert.match(buildOpenbenchConfig(design).config, /data_type: grid/);
});

test('saving a simulation keeps the model profile and station ID overrides', () => {
  const form = baseForm({
    dir: '/p/AT-Neu/history',
    dataType: 'stn',
    prefix: 'AT-Neu_hist_',
    from: '2003',
    to: '2006',
    profile: 'CoLM2024_DEV',
    stationId: 'AT-Neu',
  });
  const result = saveSimulationRun(form);
  assert.deepEqual(result.issues, []);
  assert.equal(result.picks[0]!.profile, 'CoLM2024_DEV');
  assert.equal(result.picks[0]!.stationId, 'AT-Neu');
});

test('the simulation form rejects an empty profile or an unsafe station ID', () => {
  const emptyProfile = saveSimulationRun(
    baseForm({ dir: '/d', dataType: 'stn', from: '2003', to: '2006', profile: '' }),
  );
  assert.equal(emptyProfile.issues[0]?.field, 'profile');

  const badProfile = saveSimulationRun(
    baseForm({ dir: '/d', dataType: 'stn', from: '2003', to: '2006', profile: 'bad profile' }),
  );
  assert.equal(badProfile.issues[0]?.field, 'profile');

  const badStation = saveSimulationRun(
    baseForm({
      dir: '/d',
      dataType: 'stn',
      from: '2003',
      to: '2006',
      stationId: 'AT,Neu',
    }),
  );
  assert.equal(badStation.issues[0]?.field, 'stationId');
});
