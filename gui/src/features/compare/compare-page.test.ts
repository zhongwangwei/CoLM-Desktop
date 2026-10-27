import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  buildOpenbenchConfig,
  configInvalidation,
  designFingerprint,
  simulationFormForPick,
  simulationInfoForSave,
  validateSimulation,
  type ComparisonDesign,
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
  error?: string;
};
type Site = { id: string; station: string; history_dir: string; prefix: string };
type Generated = { fingerprint: string; config: string; sites: Site[] };

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

/** 跑真实的 saveSimulation，收集它给出的字段问题与最终写入的 pick。 */
function saveSimulationRun(form: Form, existing: Pick[] = []) {
  const issues: SimulationIssue[] = [];
  const errors: string[] = [];
  const picks: Pick[] = existing.map((pick) => ({ ...pick }));
  let dialog: string | null = 'simulation';
  const save = execute<() => void>('saveSimulation', {
    simForm: form,
    picks,
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

test('generating records the fingerprint, text and station list of the exact design used', () => {
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
  let generatedValue: Generated | null = null;
  let yaml = '';
  let sites: Site[] = [];
  let missingExtent: string[] = [];
  const errors: string[] = [];
  const generate = execute<() => void>('generate', {
    comparisonDesign: () => design,
    buildOpenbenchConfig,
    designFingerprint,
    machine,
    generated: {
      get current() {
        return generatedValue;
      },
      set current(value: Generated | null) {
        generatedValue = value;
      },
    },
    setYaml: (value: string) => {
      yaml = value;
    },
    setSites: (value: Site[]) => {
      sites = value;
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
  assert.equal(
    generatedValue!.fingerprint,
    designFingerprint({ machine, ...design }),
    'the fingerprint is recorded together with the generated text',
  );
  assert.equal(generatedValue!.config, built.config);
  assert.equal(yaml, built.config);
  assert.deepEqual(sites, built.sites);
  assert.deepEqual(missingExtent, built.missingExtent);
  assert.deepEqual(generatedValue!.sites, built.sites);
  // 设置一变，同一份记录的指纹就对不上当前设计 → 旧配置失效（GUI-04）。
  assert.notEqual(
    designFingerprint({ machine, ...design, timRes: 'Hour' }),
    generatedValue!.fingerprint,
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

test('running a configuration generated for older settings is refused', async () => {
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const run = execute<() => Promise<unknown>>('run', {
    task: async (fn: () => Promise<unknown>) => fn(),
    generated: { current: { fingerprint: 'old-design', config: 'old: config\n', sites: [] } },
    design: 'new-design',
    name: 'comparison',
    t,
    comparisonName: (comparison: string) => comparison,
    remote: false,
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return { report: null };
    },
    yaml: 'old: config\n',
    sites: [],
    scanDir: '/refs',
    host: '',
    watch: async () => {},
    fetchReport: async () => {},
    view: async () => {},
  });
  await assert.rejects(run(), /generate the evaluation configuration again/);
  assert.deepEqual(calls, [], 'nothing is sent to the backend for a stale design (GUI-04)');
});

test('a current configuration runs with exactly the generated config and station list', async () => {
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const sites: Site[] = [
    {
      id: 'AT_Neu',
      station: 'AT-Neu',
      history_dir: '/p/AT-Neu/history',
      prefix: 'AT-Neu_hist_',
    },
  ];
  const run = execute<() => Promise<unknown>>('run', {
    task: async (fn: () => Promise<unknown>) => fn(),
    generated: {
      current: { fingerprint: 'current-design', config: 'project:\n  name: x\n', sites },
    },
    design: 'current-design',
    name: 'comparison',
    t,
    comparisonName: (comparison: string) => comparison,
    remote: false,
    invokeJson: async (command: string, args: Record<string, unknown>) => {
      calls.push({ command, args });
      return { report: null };
    },
    yaml: 'project:\n  name: x\n',
    sites,
    scanDir: '/refs',
    host: '',
    watch: async () => {},
    fetchReport: async () => {},
    view: async () => {},
  });
  await run();
  assert.equal(calls.length, 1);
  assert.equal(calls[0]!.command, 'bench_run');
  assert.deepEqual(calls[0]!.args, {
    name: 'comparison',
    config: 'project:\n  name: x\n',
    sites,
    refRoot: '/refs',
  });
});

test('a changed setting clears an untouched configuration but keeps a hand-edited one', () => {
  const invalidate = (generatedValue: Generated | null, yaml: string, design: string) => {
    const cleared: string[] = [];
    const clear = execute<() => void>('invalidateGeneratedConfig', {
      configInvalidation,
      design,
      generated: { current: generatedValue },
      yaml,
      setYaml: (value: string) => cleared.push(value),
      setSites: () => {},
      setMissingExtent: () => {},
    });
    clear();
    return cleared;
  };
  // 没手改过的自动配置：随设置变化作废，要求重新生成。
  assert.deepEqual(
    invalidate({ fingerprint: 'a', config: 'generated', sites: [] }, 'generated', 'b'),
    [''],
  );
  // 手改过的：原样保留，等用户自己决定（页面显示过期提示与重新生成入口）。
  assert.deepEqual(
    invalidate({ fingerprint: 'a', config: 'generated', sites: [] }, 'generated + my edit', 'b'),
    [],
  );
  // 设计没变：什么都不动。
  assert.deepEqual(
    invalidate({ fingerprint: 'a', config: 'generated', sites: [] }, 'generated', 'a'),
    [],
  );
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
