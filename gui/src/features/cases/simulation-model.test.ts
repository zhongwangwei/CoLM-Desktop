import test from 'node:test';
import assert from 'node:assert/strict';
import {
  caseDate,
  caseDatePatch,
  casePeriodValid,
  caseSpinupValid,
  casePeriodText,
  scannedForcing,
  newCaseProblem,
  nodePlan,
  nodePlanText,
  newCaseRangeProblem,
  newCaseGeneralProblem,
  simulationFields,
  caseRangeKeys,
  simulationSteps,
  spinupProblem,
  validDate,
  modelSetup,
  setupRechecks,
} from './simulation-model';
import type { SchemaField, Site } from './case-model';
const form = {
  engine: 'rust',
  root: '/project',
  name: 'CN-Cng',
  kind: 'site',
  landcover: 'IGBP',
  site: '/site.nc',
  met: '/met.nc',
  start: '2008-01-01',
  end: '2008-01-11',
  runtime: '',
};
test('dates reject calendar errors and edit only the selected date without resetting seconds or spinup', () => {
  assert.equal(validDate('2008-02-29'), true);
  assert.equal(validDate('2009-02-29'), false);
  assert.equal(validDate('2026-02-31'), false);
  const values = {
    'DEF_simulation_time%start_sec': '3600',
    'DEF_simulation_time%spinup_repeat': '3',
    ...caseDatePatch('2008-01-01', 'start'),
  };
  assert.equal(caseDate(values, 'start'), '2008-01-01');
  assert.equal(
    { ...values, ...caseDatePatch('2008-02-29', 'start') }['DEF_simulation_time%start_sec'],
    '3600',
  );
  assert.equal(
    { ...values, ...caseDatePatch('2008-02-29', 'start') }['DEF_simulation_time%spinup_repeat'],
    '3',
  );
  assert.throws(() => caseDatePatch('2009-02-29', 'end'));
  assert.equal(caseDate({}, 'end'), '');
});
test('review rejects missing input, impossible dates, incompatible model and forcing coverage', () => {
  assert.equal(newCaseProblem(form, 3, undefined, {}), null);
  assert.equal(newCaseProblem({ ...form, end: form.start }, 3, undefined, {}), null);
  assert.ok(newCaseProblem({ ...form, end: '2007-12-31' }, 3, undefined, {}));
  assert.ok(newCaseProblem({ ...form, met: '' }, 3, undefined, {}));
  assert.ok(newCaseProblem(form, 3, undefined, { DEF_USE_BGC: '.true.' }));
  const site = {
    site_file: form.site,
    met_file: form.met,
    crop: false,
    urban: false,
    start: '2008-01-01',
    end: '2008-01-10',
    problem: null,
  } as Site;
  assert.ok(newCaseProblem(form, 3, site, {}));
  assert.ok(newCaseProblem(form, 3, { ...site, end: form.end, crop: true }, {}));
  assert.equal(newCaseProblem(form, 3, { ...site, end: form.end }, {}), null);
  assert.equal(scannedForcing(site, form.met), true);
  assert.equal(scannedForcing(site, '/other.nc'), false);
  assert.equal(newCaseProblem({ ...form, met: '/other.nc' }, 3, site, {}), null);
});
test('general setup enforces engine, root, and name', () => {
  assert.equal(newCaseGeneralProblem(form), null);
  assert.ok(newCaseGeneralProblem({ ...form, engine: '' }));
  assert.ok(newCaseGeneralProblem({ ...form, root: '' }));
  assert.ok(newCaseGeneralProblem({ ...form, name: '' }));
  assert.ok(newCaseGeneralProblem({ ...form, name: '..' }));
});
test('existing intra-day and midnight windows compare their actual seconds', () => {
  const values = {
    ...caseDatePatch('2008-01-01', 'start'),
    ...caseDatePatch('2008-01-01', 'end'),
    'DEF_simulation_time%start_sec': '0',
    'DEF_simulation_time%end_sec': '86400',
  };
  assert.equal(casePeriodValid(values), true);
  assert.equal(casePeriodText(values), '2008-01-01 00:00:00 — 2008-01-02 00:00:00');
  assert.equal(
    casePeriodValid({
      ...values,
      'DEF_simulation_time%start_sec': '3600',
      'DEF_simulation_time%end_sec': '7200',
    }),
    true,
  );
  for (const end of ['0', 'NaN', '-1'])
    assert.equal(casePeriodValid({ ...values, 'DEF_simulation_time%end_sec': end }), false);
});
test('spatial setup keeps mandatory data, mesh and numeric constraints', () => {
  const spatial = {
    ...form,
    kind: 'region',
    grid: 'latlon',
    west: '0',
    east: '1',
    south: '0',
    north: '1',
    dlon: '0.1',
    dlat: '0.1',
    rawdata: '/raw',
    runtime: '/runtime',
    forcing: '/forcing.nml',
    timestep: '1800',
  };
  assert.equal(newCaseProblem(spatial, 3, undefined, {}), null);
  assert.equal(newCaseProblem({ ...spatial, west: '170', east: '-170' }, 3, undefined, {}), null);
  assert.equal(
    newCaseProblem(
      {
        ...spatial,
        kind: 'watershed',
        shapefile: '/basin.shp',
        west: '',
        east: '',
        south: '',
        north: '',
      },
      3,
      undefined,
      {},
    ),
    null,
  );
  assert.equal(
    newCaseProblem(
      {
        ...spatial,
        grid: 'unstructured',
        mesh: '/mesh.nc',
        west: '',
        east: '',
        south: '',
        north: '',
      },
      3,
      undefined,
      {},
    ),
    null,
  );
  const changes: Record<string, string>[] = [
    { west: '' },
    { east: '0' },
    { north: '91' },
    { dlon: '0' },
    { runtime: '' },
    { timestep: '-1' },
    { grid: 'unstructured', mesh: '' },
    { kind: 'watershed', shapefile: '' },
  ];
  for (const change of changes)
    assert.ok(newCaseProblem({ ...spatial, ...change }, 3, undefined, {}));
});
test('forcing data paths cannot change meaning between creation and model working directories', () => {
  const spatial = {
    ...form,
    kind: 'global',
    grid: 'latlon',
    dlon: '1',
    dlat: '1',
    rawdata: '/raw',
    runtime: '/runtime',
    forcing_mode: 'directory',
    forcing_dataset: 'MSWX',
    timestep: '1800',
  };
  for (const spatial_forcingdir of ['relative/data', '../forcing', '~/forcing'])
    assert.match(
      newCaseProblem({ ...spatial, spatial_forcingdir }, 3, undefined, {}) ?? '',
      /绝对路径/,
    );
  for (const spatial_forcingdir of [
    '/data/forcing',
    'C:\\data\\forcing',
    '\\\\server\\share\\forcing',
  ])
    assert.equal(newCaseProblem({ ...spatial, spatial_forcingdir }, 3, undefined, {}), null);
  assert.match(
    newCaseProblem(
      { ...spatial, host: 'compute', spatial_forcingdir: 'C:\\data' },
      3,
      undefined,
      {},
    ) ?? '',
    /绝对路径/,
  );
});
test('five steps expose data, model and output separately without dropping advanced fields', () => {
  const field = (name: string, section: string) => ({ name, section }) as SchemaField;
  const schema = [
    field('DEF_simulation_time%start_year', '时间与预热'),
    field('DEF_simulation_time%start_sec', '时间与预热'),
    field('DEF_simulation_time%spinup_repeat', '时间与预热'),
    field('DEF_forcing_namelist', '强迫场'),
    field('DEF_HIST_FREQ', '输出与重启'),
    field('DEF_hist_vars%tref', '输出变量'),
    field('DEF_Forcing_Interp_Method', '强迫场'),
    field('DEF_USE_RangeCheck', '调试与诊断'),
    field('DEF_nx_blocks', '网格与并行'),
    field('DEF_domain%latlon_dlon', '网格与并行'),
  ];
  assert.deepEqual(
    simulationSteps.map((step) => step[0]),
    ['运行环境与保存位置', '范围与网格', '数据与时段', '模型结构与方案', '检查与运行'],
  );
  assert.deepEqual(simulationFields(schema, 1, false), []);
  assert.deepEqual(simulationFields(schema, 1, true), []);
  assert.deepEqual(simulationFields(schema, 2, true, 'rust'), []);
  assert.deepEqual(
    simulationFields(schema, 2, true, 'fortran').map((f) => f.name),
    ['DEF_domain%latlon_dlon'],
  );
  // 预热（起止与重复次数）与模拟时段同在“数据与时段”这一步。
  assert.deepEqual(
    simulationFields(schema, 3, false).map((f) => f.name),
    ['DEF_simulation_time%spinup_repeat', 'DEF_forcing_namelist'],
  );
  assert.deepEqual(
    simulationFields(schema, 3, true).map((f) => f.name),
    [
      'DEF_simulation_time%start_sec',
      'DEF_simulation_time%spinup_repeat',
      'DEF_forcing_namelist',
      'DEF_Forcing_Interp_Method',
    ],
  );
  assert.deepEqual(simulationFields(schema, 4, true), []);
  assert.deepEqual(
    simulationFields(schema, 5, false).map((f) => f.name),
    ['DEF_HIST_FREQ'],
  );
  assert.deepEqual(
    simulationFields(schema, 5, true).map((f) => f.name),
    // 历史开关交给输出变量卡片，不再混在“更多配置”的通用字段里。
    ['DEF_HIST_FREQ', 'DEF_USE_RangeCheck'],
  );
});
test('range selection can precede dates; years stay exact and spinup uses a cutoff', () => {
  assert.equal(newCaseRangeProblem({ ...form, start: '', end: '' }), null);
  assert.equal(validDate('2001-01-01'), true);
  assert.equal(validDate('0200-01-01'), true);
  for (const date of ['2001', '2001-', '2001-01', '0000-01-01', '2001-02-29'])
    assert.equal(validDate(date), false);
  assert.equal(caseDate({ ...caseDatePatch('2001-01-01', 'start') }, 'start'), '2001-01-01');
  assert.equal(spinupProblem('2001-01-01', '2003-01-01', '', '1'), null);
  assert.equal(spinupProblem('2001-01-01', '2003-01-01', '2002-01-01', '2'), null);
  assert.ok(spinupProblem('2001-01-01', '2003-01-01', '2003-01-01', '2'));
  assert.ok(spinupProblem('2001-01-01', '2003-01-01', '2002-01-01', '0'));
});
test('existing spinup cutoff respects actual seconds and moved simulation dates', () => {
  const values = {
    ...caseDatePatch('2001-01-01', 'start'),
    ...caseDatePatch('2001-01-02', 'end'),
    ...caseDatePatch('2001-01-01', 'spinup'),
    'DEF_simulation_time%start_sec': '3600',
    'DEF_simulation_time%end_sec': '7200',
    'DEF_simulation_time%spinup_sec': '7200',
  };
  assert.equal(caseSpinupValid(values), true);
  assert.equal(caseSpinupValid({ ...values, ...caseDatePatch('2030-01-01', 'spinup') }), false);
  assert.equal(caseSpinupValid({ ...values, ...caseDatePatch('2001-01-02', 'start') }), false);
  assert.equal(caseSpinupValid({ ...values, 'DEF_simulation_time%spinup_sec': '3600' }), false);
  assert.equal(caseSpinupValid({ ...values, 'DEF_simulation_time%spinup_year': '0' }), true);
  assert.equal(
    caseSpinupValid({
      ...values,
      ...caseDatePatch('2001-01-02', 'spinup'),
      'DEF_simulation_time%spinup_sec': '3600',
    }),
    true,
    'same end date remains valid before its actual end seconds',
  );
});

test('case range controls match site and spatial inputs', () => {
  assert.deepEqual(caseRangeKeys(), ['SITE_lon_location', 'SITE_lat_location']);
  assert.ok(caseRangeKeys('latlon').includes('DEF_GRIDBASED_lon_res'));
  assert.ok(caseRangeKeys('catchment').includes('DEF_CatchmentMesh_data'));
  assert.ok(caseRangeKeys('unstructured').includes('DEF_file_mesh'));
  assert.ok(!caseRangeKeys('latlon').includes('SITE_fsitedata'));
});

test('general identity and range coordinates stay separate from model source settings', async () => {
  const { simulationFieldStep } = await import('./simulation-model');
  for (const [name, section, expected] of [
    ['DEF_CASE_NAME', '算例', 1],
    ['SITE_lon_location', '站点', 2],
    ['SITE_lat_location', '站点', 2],
    ['SITE_fsitedata', '站点', 3],
    ['USE_SITE_LAI', '站点', 3],
    ['USE_SITE_pctcrop', '站点', 3],
    ['USE_SITE_urban_geometry', '城市', 3],
    ['USE_SITE_HistWriteBack', '输出与重启', 5],
    ['USE_SITE_ForcingReadAhead', '站点', 3],
    ['DEF_Forcing_Interp_Method', '强迫场', 3],
    ['DEF_LAI_MONTHLY', '地表数据', 4],
    ['DEF_LC_YEAR', '地表数据', 3],
  ] as const)
    assert.equal(simulationFieldStep({ name, section }), expected, name);
});

test('a run bigger than one node plans its nodes from the cores a node has', () => {
  const plan = (cpus: number, ranks: number, coresPerNode: number, nodes = 0) =>
    nodePlan({ cpus, ranks, coresPerNode, nodes });
  // Nothing to plan: it fits on one node, or the node size is unknown.
  assert.equal(plan(4, 1, 48), null);
  assert.equal(plan(1, 48, 48), null);
  assert.equal(plan(1, 96, 0), null);
  // 摆渡船 (48 cores a node).
  assert.deepEqual(plan(1, 96, 48), { total: 96, nodes: 2, perNode: 48, explicit: false });
  assert.deepEqual(plan(2, 48, 48), { total: 96, nodes: 2, perNode: 48, explicit: false });
  assert.deepEqual(plan(1, 100, 48), { total: 100, nodes: 4, perNode: 25, explicit: false });
  // An explicit node count wins.
  assert.deepEqual(plan(1, 96, 48, 4), { total: 96, nodes: 4, perNode: 24, explicit: true });
  // Cannot be placed.
  assert.deepEqual(plan(96, 1, 48), { error: 'single', total: 96, coresPerNode: 48 });
  assert.deepEqual(plan(64, 4, 48), { error: 'rank', total: 256, coresPerNode: 48 });
  assert.deepEqual(plan(1, 97, 48), { error: 'uneven', total: 97, coresPerNode: 48 });
  assert.equal(
    nodePlanText(plan(1, 96, 48)),
    '将占用 2 个节点，每个节点 48 核，共 96 核（自动计算）。',
  );
  assert.match(nodePlanText(plan(96, 1, 48)), /单个进程要 96 个核/);
  assert.equal(nodePlanText(null), '');
});

test('a simulation folder with spaces is refused at the first step, in both languages', () => {
  const form = { engine: 'rust', root: '/Users/me/My Projects', name: 'site1' };
  assert.match(newCaseGeneralProblem(form) ?? '', /不能有空格/);
  assert.match(newCaseGeneralProblem(form, true) ?? '', /must not contain spaces/);
  assert.equal(newCaseGeneralProblem({ ...form, root: '/Users/me/projects' }), null);
});

// U-03：向导顺序把"数据与时段"放在"模型结构与方案"之前。方案一改，前面核对过的数据可能失效；
// 这里验证"哪些改变真的会让哪一步需要重新检查"，避免正常的首次选方案也让人返工。
test('model setup captures only the dimensions that can invalidate earlier wizard data', () => {
  const plain = modelSetup({
    engine: 'rust',
    mode: 'pft',
    classification: 'igbp',
    revision: 'official#2',
  });
  assert.deepEqual(plain, {
    engine: 'rust',
    mode: 'PFT',
    classification: 'IGBP',
    crop: false,
    urban: false,
    bgc: false,
    tracer: false,
    revision: 'official#2',
  });
  const processes = modelSetup({
    physics: {
      DEF_TUNING_CROP_PLANTING_DAY: '120',
      DEF_URBAN_RUN: '.TRUE.',
      DEF_USE_BGC: 'true',
      DEF_USE_ISOTOPE: '1',
    },
  });
  assert.deepEqual(
    [processes.crop, processes.urban, processes.bgc, processes.tracer],
    [true, true, true, true],
  );
});

test('a scheme change flags the data step with the reason and the new required folder', () => {
  const before = modelSetup({ engine: 'rust', mode: 'IGBP', classification: 'IGBP' });
  const enriched = modelSetup({
    engine: 'rust',
    mode: 'PFT',
    classification: 'IGBP',
    physics: { DEF_USE_BGC: '.true.' },
  });
  const notices = setupRechecks(before, enriched, 3, { runtimeFilled: false });
  assert.equal(notices.length, 1);
  assert.equal(notices[0].step, 3);
  assert.match(notices[0].text, /下垫面结构从 IGBP 改为 PFT/);
  assert.match(notices[0].text, /BGC/);
  assert.match(notices[0].text, /Runtime/);
  assert.deepEqual(notices[0].fields, ['runtime'], 'the new required folder is named');
  // 英文同样给出原因与必填项。
  const english = setupRechecks(before, enriched, 3, { runtimeFilled: false }, true);
  assert.match(english[0].text, /vegetation structure changed from IGBP to PFT/);
  assert.match(english[0].text, /runtime data folder/);
});

test('a scheme change keeps the data step valid when nothing it depends on changed', () => {
  const before = modelSetup({
    engine: 'rust',
    mode: 'PFT',
    classification: 'IGBP',
    physics: { DEF_USE_BGC: '.true.' },
  });
  // 只是关掉 BGC 并填好了 runtime：旧数据仍然可用，不需要重新检查。
  const after = modelSetup({ engine: 'rust', mode: 'PFT', classification: 'IGBP' });
  assert.deepEqual(setupRechecks(before, after, 3, { runtimeFilled: true }), []);
  // 同样的模型设置、且 BGC 需要的目录也填好了，不提示。
  assert.deepEqual(setupRechecks(before, before, 3, { runtimeFilled: true }), []);
  // 还没核对过（没有基线）的步骤不提示。
  assert.deepEqual(setupRechecks(null, after, 3, {}), []);
});

test('the first scheme selection is not treated as a change of already checked data', () => {
  const empty = modelSetup({ engine: 'rust' });
  const selected = modelSetup({ engine: 'rust', mode: 'IGBP', classification: 'IGBP' });
  assert.deepEqual(setupRechecks(empty, selected, 3, { runtimeFilled: false }), []);
});

test('a station that no longer matches the crop or urban configuration is named', () => {
  const before = modelSetup({ engine: 'rust', mode: 'IGBP' });
  const urban = modelSetup({
    engine: 'rust',
    mode: 'IGBP',
    physics: { DEF_URBAN_RUN: '.true.' },
  });
  const notices = setupRechecks(before, urban, 3, {
    sites: [
      { name: 'CN-Cng', crop: false, urban: false },
      { name: 'urban-site', crop: false, urban: true },
    ],
    runtimeFilled: true,
  });
  assert.equal(notices.length, 1);
  assert.match(notices[0].text, /CN-Cng/);
  assert.match(notices[0].text, /城市/);
  assert.doesNotMatch(notices[0].text, /urban-site/);
});

test('an engine change flags the range and review steps, never the data step', () => {
  const before = modelSetup({ engine: 'rust', mode: 'IGBP' });
  const fortran = modelSetup({ engine: 'fortran', mode: 'IGBP' });
  const range = setupRechecks(before, fortran, 2, {});
  assert.equal(range.length, 1);
  assert.equal(range[0].step, 2);
  assert.match(range[0].text, /并行布局/);
  assert.match(range[0].text, /编译配置/);
  const review = setupRechecks(before, fortran, 5, {});
  assert.equal(review.length, 1);
  assert.equal(review[0].step, 5);
  assert.match(review[0].text, /线程、进程与 CPU/);
  assert.deepEqual(setupRechecks(before, fortran, 3, {}), []);
  // 切回同一个引擎不提示。
  assert.deepEqual(setupRechecks(before, before, 2, {}), []);
});
