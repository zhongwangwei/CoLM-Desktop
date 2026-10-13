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
  simulationFields,
  simulationSteps,
  processDataGroupId,
  processDataSections,
  spinupProblem,
  validDate,
} from './simulation-model';
import type { SchemaField, Site } from './case-model';
const form = {
  root: '/project',
  name: 'CN-Cng',
  kind: 'site',
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
  assert.equal(newCaseProblem(form, 2, undefined, {}), null);
  assert.equal(newCaseProblem({ ...form, end: form.start }, 1, undefined, {}), null);
  assert.ok(newCaseProblem({ ...form, end: '2007-12-31' }, 1, undefined, {}));
  assert.ok(newCaseProblem({ ...form, met: '' }, 2, undefined, {}));
  assert.ok(newCaseProblem(form, 2, undefined, { DEF_USE_BGC: '.true.' }));
  const site = {
    site_file: form.site,
    met_file: form.met,
    crop: false,
    urban: false,
    start: '2008-01-01',
    end: '2008-01-10',
    problem: null,
  } as Site;
  assert.ok(newCaseProblem(form, 2, site, {}));
  assert.ok(newCaseProblem(form, 2, { ...site, end: form.end, crop: true }, {}));
  assert.equal(newCaseProblem(form, 2, { ...site, end: form.end }, {}), null);
  assert.equal(scannedForcing(site, form.met), true);
  assert.equal(scannedForcing(site, '/other.nc'), false);
  assert.equal(newCaseProblem({ ...form, met: '/other.nc' }, 2, site, {}), null);
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
  assert.equal(newCaseProblem(spatial, 2, undefined, {}), null);
  assert.equal(newCaseProblem({ ...spatial, west: '170', east: '-170' }, 2, undefined, {}), null);
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
      2,
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
      2,
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
    assert.ok(newCaseProblem({ ...spatial, ...change }, 2, undefined, {}));
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
  ];
  assert.deepEqual(
    simulationSteps.map((step) => step[0]),
    ['范围与网格', '模型结构与方案', '数据与时段', '预热与输出', '检查与运行'],
  );
  assert.deepEqual(simulationFields(schema, 1, false), []);
  assert.deepEqual(
    simulationFields(schema, 1, true).map((f) => f.name),
    ['DEF_nx_blocks'],
  );
  assert.deepEqual(
    simulationFields(schema, 3, false).map((f) => f.name),
    ['DEF_forcing_namelist'],
  );
  assert.deepEqual(
    simulationFields(schema, 3, true).map((f) => f.name),
    ['DEF_simulation_time%start_sec', 'DEF_forcing_namelist', 'DEF_Forcing_Interp_Method'],
  );
  assert.deepEqual(
    simulationFields(schema, 4, false).map((f) => f.name),
    ['DEF_simulation_time%spinup_repeat', 'DEF_HIST_FREQ'],
  );
  assert.deepEqual(
    simulationFields(schema, 4, true).map((f) => f.name),
    [
      'DEF_simulation_time%spinup_repeat',
      'DEF_HIST_FREQ',
      'DEF_hist_vars%tref',
      'DEF_USE_RangeCheck',
    ],
  );
  assert.deepEqual(simulationFields(schema, 2, true), []);
  assert.deepEqual(simulationFields(schema, 5, true), []);
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

test('process input groups retain their domain-specific fields without absorbing ordinary case data', () => {
  const field = (name: string, section: string) => ({ name, section }) as SchemaField;
  assert.deepEqual(
    processDataSections.map(([id]) => id),
    ['bgc', 'river', 'tracer', 'urban'],
  );
  for (const [name, section, group] of [
    ['DEF_USE_CN_INIT', '初始场', 'bgc'],
    ['DEF_file_cn_init', '初始场', 'bgc'],
    ['DEF_file_ndep', '强迫场', 'bgc'],
    ['DEF_UnitCatchment_file', '河道与水库', 'river'],
    ['DEF_TRACER_SOIL_INIT_FILE', '初始场', 'tracer'],
    ['DEF_file_GIEMS', '初始场', 'tracer'],
    ['DEF_URBAN_type_scheme', '地表数据', 'urban'],
    ['DEF_dir_rawdata', '地表数据', null],
    ['DEF_HIST_FREQ', '输出与重启', null],
  ] as const)
    assert.equal(processDataGroupId(field(name, section)), group, name);
});
