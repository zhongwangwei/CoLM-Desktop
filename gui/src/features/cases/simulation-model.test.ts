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
  newCaseRangeProblem,
  newCaseGeneralProblem,
  simulationFields,
  caseRangeKeys,
  simulationSteps,
  spinupProblem,
  validDate,
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
    ['总的设定', '范围与网格', '数据与时段', '模型结构与方案', '检查与运行'],
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
