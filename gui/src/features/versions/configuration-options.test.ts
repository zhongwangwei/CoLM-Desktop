import test from 'node:test';
import assert from 'node:assert/strict';
import type { Overlay } from '../../lib/types';
import type { Descriptor, ModelField } from './model';
import {
  caseFieldVisible,
  type ConfigurationField,
  tracerFamilyState,
  appendTracerFamily,
  setStomatalScheme,
  groupedField,
  setModelOption,
  resetModelOption,
  setScopedOption,
  resetScopedOption,
  setProcessOption,
  resetProcessOption,
  tracerRows,
  setTracerRows,
  tracerKeys,
  fieldLiteral,
} from './configuration-options';
const empty = (): Overlay => ({ physics: {}, parameters: [], process_files: [] });
const field = (name: string, kind = 'logical'): ModelField => ({
  name,
  kind,
  default: '.false.',
  values: [],
  section: '',
  derived: false,
});
const descriptor = (
  id: string,
  key: string,
  storage = 'process-parameter-file',
  kind = 'real',
): Descriptor => ({
  id,
  raw_key: key,
  label_zh: key,
  label_en: key,
  scope: storage === 'process-parameter-file' ? 'process-file' : 'pft-type',
  storage,
  section: '',
  value_kind: kind,
  unit: null,
  visibility: 'editable',
  activation: [],
  default: '1.',
  doc_zh: '',
  doc_en: '',
  validation: { min: null, max: null, allowed_values: [], finite: true },
});
test('paired switches replace case-scalar duplicates atomically and preserve inactive children', () => {
  const fields = [
    'DEF_USE_MEDLYNST',
    'DEF_USE_WUEST',
    'DEF_USE_BGC',
    'DEF_USE_Forcing_Downscaling',
    'DEF_USE_Forcing_Downscaling_Simple',
  ].map((k) => field(k));
  let overlay = empty();
  overlay.physics = { DEF_USE_WUEST: '.true.', DEF_NITRIF_DENITRIF: '.true.' };
  overlay.parameters = [
    {
      parameter_id: 'case:DEF_USE_MEDLYNST',
      raw_key: 'def_use_medlynst',
      value: '.false.',
      expected_old_value: '.false.',
      scope_instance: { kind: 'case-scalar' },
    },
  ];
  overlay = setModelOption(overlay, fields, 'DEF_USE_MEDLYNST', '.true.');
  assert.equal(overlay.physics.DEF_USE_WUEST, '.false.');
  assert.equal(overlay.parameters.length, 1);
  assert.equal(overlay.parameters[0].value, '.true.');
  assert.equal(overlay.parameters[0].expected_old_value, '.false.');
  assert.equal(overlay.physics.DEF_USE_MEDLYNST, undefined);
  overlay = setModelOption(overlay, fields, 'DEF_USE_BGC', '.false.');
  assert.equal(overlay.physics.DEF_NITRIF_DENITRIF, '.true.');
  overlay = setModelOption(overlay, fields, 'DEF_USE_Forcing_Downscaling', '.true.');
  assert.equal(overlay.physics.DEF_USE_Forcing_Downscaling_Simple, '.false.');
  assert.equal(resetModelOption(overlay, 'DEF_USE_BGC').physics.DEF_USE_BGC, undefined);
  assert.throws(() => setModelOption(overlay, fields, 'DEF_USE_BGC', 'garbage'));
  assert.equal(overlay.physics.DEF_USE_BGC, '.false.');
});
test('Fortran typed input rejects incomplete and nonfinite numeric literals', () => {
  assert.equal(fieldLiteral(field('x', 'real'), '1.d-3'), '1.d-3');
  for (const value of ['', '1e', 'NaN', 'Infinity'])
    assert.throws(() => fieldLiteral(field('x', 'real'), value));
  assert.throws(() => fieldLiteral(field('x', 'integer'), '1.5'));
  assert.equal(fieldLiteral(field('x', 'character'), "O'Brien"), "'O''Brien'");
});
test('process and PFT overrides keep file/kind/index identity and reset only their own entry', () => {
  const d = descriptor('process:init', 'DEF_TRACER%init_conc');
  const overlay = empty();
  overlay.process_files = [
    { relative_path: 'a.nml', sha256: 'a' },
    { relative_path: 'b.nml', sha256: 'b' },
  ];
  let next = setScopedOption(overlay, d, '2.', 0, 'a.nml');
  next.parameters[0].expected_old_value = '1.';
  next = setScopedOption(next, d, '4.', 0, 'a.nml');
  assert.equal(next.parameters[0].expected_old_value, '1.');
  next = setScopedOption(next, d, '3.', 0, 'b.nml');
  next = resetScopedOption(next, d, 0, 'a.nml');
  assert.equal(next.parameters.length, 1);
  assert.equal(next.parameters[0].scope_instance.process_file, 'b.nml');
  assert.throws(() => setScopedOption(next, d, '2.', 0, 'missing.nml'));
  const p = descriptor('pft:leaf', 'leaf', 'pft-override');
  next = setScopedOption(next, p, '4.', 1, '');
  next = setScopedOption(next, p, '5.', 2, '');
  assert.equal(next.parameters.length, 3);
  next = resetScopedOption(next, p, 1, '');
  assert.equal(next.parameters.find((r) => r.scope_instance.index === 2)?.value, '5.');
});
test('methane research edits update only the process parameter and reset it without case switches', () => {
  const d = descriptor(
    'process:mode',
    'DEF_METHANE%inundation_mode',
    'process-parameter-file',
    'character',
  );
  const overlay = empty();
  overlay.process_files = [{ relative_path: 'CH4.nml', sha256: 'a' }];
  let next = setProcessOption(overlay, d, 'hybrid', 'CH4.nml', undefined);
  assert.equal(next.physics.DEF_USE_Dynamic_Wetland, undefined);
  assert.equal(setProcessOption(overlay, d, 'routing', 'CH4.nml', undefined).parameters.length, 1);
  assert.equal(overlay.parameters.length, 0);
  next = resetProcessOption(next, d, 'CH4.nml');
  assert.equal(next.parameters.length, 0);
  assert.equal(next.physics.DEF_USE_Dynamic_Wetland, undefined);
  assert.equal(resetProcessOption(next, d, 'CH4.nml').parameters.length, 0);
});
test('tracer table applies aligned definitions once and invalid partial input does not mutate the draft', () => {
  const fields = tracerKeys.map((k) => field(k, k === 'DEF_TRACER_NUM' ? 'integer' : 'character'));
  const overlay = empty();
  const rows = [
    {
      NAMES: 'CH4',
      TYPES: 'reactive',
      MRAT: '',
      REF_RATIO: '',
      INIT_DELTA: '',
      REACTIVE_DECAY_RATE: '',
      PARAM_FILES: 'CH4.nml',
    },
    {
      NAMES: 'O18',
      TYPES: 'isotope',
      MRAT: '',
      REF_RATIO: '',
      INIT_DELTA: '',
      REACTIVE_DECAY_RATE: '',
      PARAM_FILES: 'O18.nml',
    },
  ];
  const next = setTracerRows(overlay, fields, rows);
  assert.equal(next.physics.DEF_TRACER_NUM, '2');
  assert.equal(next.physics.DEF_TRACER_MRAT, "''");
  assert.deepEqual(tracerRows(next), rows);
  assert.throws(() => setTracerRows(overlay, fields, [rows[0], { ...rows[1], MRAT: '1.1' }]));
  assert.deepEqual(overlay, empty());
  assert.throws(() => setTracerRows(overlay, fields, [rows[0], { ...rows[1], NAMES: 'CH4' }]));
});
test('process arrays retain numeric and character list types, per-element bounds and guarded reset', () => {
  const d = descriptor(
    'forcing:dtime',
    'DEF_forcing%forcing_dtime',
    'process-parameter-file',
    'real',
  );
  d.validation.min = '0.';
  const role = descriptor(
    'forcing:role',
    'DEF_forcing%forcing_role',
    'process-parameter-file',
    'character',
  );
  const overlay = empty();
  overlay.process_files = [{ relative_path: 'forcing.nml', sha256: 'a' }];
  let next = setProcessOption(overlay, d, '21600, 21600', 'forcing.nml', 'list');
  assert.equal(next.parameters[0].value, '21600, 21600');
  next.parameters[0].expected_old_value = '3600, 3600';
  next = setProcessOption(next, d, '1800., 1800.', 'forcing.nml', 'list');
  assert.equal(next.parameters[0].expected_old_value, '3600, 3600');
  next = setProcessOption(next, role, "'TAIR', 'WIND'", 'forcing.nml', 'list');
  assert.equal(next.parameters[1].value, "'TAIR', 'WIND'");
  next = setProcessOption(next, role, "'O''Brien', 'a,b'", 'forcing.nml', 'list');
  assert.equal(next.parameters[1].value, "'O''Brien', 'a,b'");
  for (const value of ['21600, Infinity', '21600, -1', '21600,', ''])
    assert.throws(() => setProcessOption(next, d, value, 'forcing.nml', 'list'));
  assert.throws(() => setProcessOption(next, role, "'TAIR, 'WIND'", 'forcing.nml', 'list'));
  next = resetProcessOption(next, d, 'forcing.nml');
  assert.equal(next.parameters.length, 1);
  assert.equal(next.parameters[0].raw_key, 'DEF_forcing%forcing_role');
  assert.equal(overlay.parameters.length, 0);
});
test('methane legacy mode aliases restore only the research asset parameter', () => {
  const d = descriptor(
    'process:mode',
    'DEF_METHANE%inundation_mode',
    'process-parameter-file',
    'character',
  );
  const overlay = empty();
  overlay.process_files = [{ relative_path: 'CH4.nml', sha256: 'a' }];
  for (const mode of ['dynamic-wtd', 'dh_all_thr05', 'dyn_routing_hybrid']) {
    const edited = setProcessOption(overlay, d, mode, 'CH4.nml');
    assert.equal(edited.parameters[0].value, `'${mode}'`);
    assert.deepEqual(edited.physics, {});
    const result = resetProcessOption(edited, d, 'CH4.nml');
    assert.deepEqual(result.physics, {});
    assert.equal(result.parameters.length, 0);
  }
});
test('tracer files resolve keyed and positional mappings and preserve untouched original serialization', () => {
  const fields = tracerKeys.map((k) => field(k, k === 'DEF_TRACER_NUM' ? 'integer' : 'character'));
  const overlay = empty();
  overlay.physics = {
    DEF_TRACER_NUM: '2',
    DEF_TRACER_NAMES: "'CH4,O18'",
    DEF_TRACER_TYPES: "'gas,isotope'",
    DEF_TRACER_PARAM_FILES: "'O18:oxygen.nml; CH4:methane.nml'",
  };
  let rows = tracerRows(overlay);
  assert.deepEqual(
    rows.map((row) => row.PARAM_FILES),
    ['methane.nml', 'oxygen.nml'],
  );
  assert.equal(
    setTracerRows(overlay, fields, rows).physics.DEF_TRACER_PARAM_FILES,
    overlay.physics.DEF_TRACER_PARAM_FILES,
  );
  rows[0].PARAM_FILES = 'new-ch4.nml';
  const next = setTracerRows(overlay, fields, rows);
  assert.equal(next.physics.DEF_TRACER_PARAM_FILES, "'CH4:new-ch4.nml,O18:oxygen.nml'");
  overlay.physics.DEF_TRACER_PARAM_FILES = "'null; O18:oxygen.nml'";
  rows = tracerRows(overlay);
  assert.deepEqual(
    rows.map((row) => row.PARAM_FILES),
    ['null', 'oxygen.nml'],
  );
  overlay.physics.DEF_TRACER_PARAM_FILES = "'C:\\data\\CH4.nml,oxygen.nml'";
  assert.deepEqual(
    tracerRows(overlay).map((row) => row.PARAM_FILES),
    ['C:\\data\\CH4.nml', 'oxygen.nml'],
  );
  rows[0].PARAM_FILES = '';
  assert.equal(
    setTracerRows(overlay, fields, rows).physics.DEF_TRACER_PARAM_FILES,
    "'CH4:null,O18:oxygen.nml'",
  );
});
test('tracer numeric metadata accepts Fortran exponents and rejects invalid finite values atomically', () => {
  const fields = tracerKeys.map((k) => field(k, k === 'DEF_TRACER_NUM' ? 'integer' : 'character')),
    overlay = empty();
  const row = {
    NAMES: 'CH4',
    TYPES: 'gas',
    MRAT: '',
    REF_RATIO: '',
    INIT_DELTA: '',
    REACTIVE_DECAY_RATE: '',
    PARAM_FILES: '',
  };
  for (const column of ['MRAT', 'REF_RATIO', 'INIT_DELTA', 'REACTIVE_DECAY_RATE']) {
    for (const bad of ['not-a-number', 'NaN', 'Infinity'])
      assert.throws(() => setTracerRows(overlay, fields, [{ ...row, [column]: bad }]));
    assert.equal(
      setTracerRows(overlay, fields, [{ ...row, [column]: '1d-3' }]).physics[
        `DEF_TRACER_${column}`
      ],
      "'1d-3'",
    );
  }
  assert.deepEqual(overlay, empty());
});
test('active and inactive methane research assets preserve all case-owned wetland switches', () => {
  const d = descriptor(
      'process:mode',
      'DEF_METHANE%inundation_mode',
      'process-parameter-file',
      'character',
    ),
    overlay = empty();
  overlay.process_files = [
    { relative_path: 'active.nml', sha256: 'a' },
    { relative_path: 'unused.nml', sha256: 'b' },
  ];
  overlay.physics.DEF_USE_Dynamic_Wetland = '.false.';
  for (const file of ['active.nml', 'unused.nml']) {
    const edited = setProcessOption(overlay, d, 'hybrid', file);
    assert.equal(edited.physics.DEF_USE_Dynamic_Wetland, '.false.');
    assert.equal(edited.parameters[0].scope_instance.process_file, file);
    const reset = resetProcessOption(edited, d, file);
    assert.equal(reset.physics.DEF_USE_Dynamic_Wetland, '.false.');
    assert.equal(reset.parameters.length, 0);
  }
});

test('generic groups exclude indexed catalog scopes while retaining ordinary scalar fields', () => {
  const land = {
      ...descriptor('land:IGBP:vmax', 'DEF_LC_VMAX25', 'case-nml'),
      scope: 'land-cover-class',
    },
    pft = { ...descriptor('pft:vmax', 'DEF_PFT_VMAX25', 'pft-override'), scope: 'pft-type' },
    pc = {
      ...descriptor('pc:vmax', 'DEF_PC_VMAX25', 'pc-pft-override'),
      scope: 'pc-pft-component',
    },
    scalar = {
      ...descriptor('case:soil', 'DEF_SOIL_REFL_SCHEME', 'case-nml'),
      scope: 'case-scalar',
    };
  const catalog = [land, pft, pc, scalar];
  for (const d of [land, pft, pc])
    assert.equal(groupedField(field(d.raw_key.toLowerCase(), 'real'), catalog), false);
  assert.equal(groupedField(field(scalar.raw_key, 'integer'), catalog), true);
  assert.equal(groupedField(field('DEF_USE_NITRIF_DENITRIF'), catalog), true);
});

test('family presets only add missing names/types without mutating existing or inventing numeric and asset values', () => {
  const rows = [
    {
      NAMES: 'custom',
      TYPES: 'custom-type',
      MRAT: '16',
      REF_RATIO: '1',
      INIT_DELTA: '0',
      REACTIVE_DECAY_RATE: '0',
      PARAM_FILES: 'own.nml',
    },
  ];
  const result = appendTracerFamily(rows, 'isotope');
  assert.equal(result.length, 3);
  assert.deepEqual(result[0], rows[0]);
  assert.deepEqual(
    result.slice(1).map((r) => [r.NAMES, r.TYPES, r.MRAT, r.PARAM_FILES]),
    [
      ['H2_18O', 'isotope', '', ''],
      ['HDO', 'isotope', '', ''],
    ],
  );
  assert.deepEqual(appendTracerFamily(result, 'isotope'), result);
  assert.deepEqual(appendTracerFamily([{ ...rows[0], NAMES: 'CH4' }], 'methane'), [
    { ...rows[0], NAMES: 'CH4' },
  ]);
});
test('family prerequisites distinguish explicit conflicts from unresolved conditions', () => {
  const overlay = empty();
  assert.equal(tracerFamilyState(overlay, 'methane', {}).status, 'pending');
  overlay.physics = {
    DEF_USE_TRACER: '.true.',
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_BGC: '.true.',
    DEF_USE_PFT: '.true.',
    DEF_USE_PC: '.false.',
    DEF_USE_LCT: '.false.',
    DEF_USE_LULCC: '.false.',
    DEF_USE_GridRiverLakeFlow: '.true.',
  };
  const context = {
    subgrid: 'PFT',
    kernelMacros: ['GridRiverLakeFlow'],
    caseScope: 'spatial' as const,
  };
  assert.equal(tracerFamilyState(overlay, 'methane', context).status, 'ready');
  assert.equal(tracerFamilyState(overlay, 'sediment', context).status, 'ready');
  assert.equal(
    tracerFamilyState(overlay, 'sediment', { kernelMacros: [], caseScope: 'spatial' }).status,
    'blocked',
  );
  assert.equal(
    tracerFamilyState(overlay, 'sediment', { kernelMacros: ['SinglePoint'], caseScope: 'site' })
      .status,
    'blocked',
  );
  overlay.physics.DEF_USE_LULCC = '.true.';
  assert.equal(tracerFamilyState(overlay, 'isotope', context).status, 'blocked');
  assert.equal(tracerFamilyState(overlay, 'chloride', context).status, 'ready');
  overlay.physics.DEF_URBAN_RUN = '.true.';
  assert.equal(tracerFamilyState(overlay, 'chloride', context).status, 'blocked');
});
test('stomatal choice writes both flags atomically while retaining their scalar guards', () => {
  const fields = ['DEF_USE_MEDLYNST', 'DEF_USE_WUEST'].map((name) => field(name));
  const overlay = empty();
  overlay.physics.DEF_USE_WUEST = '.true.';
  let next = setStomatalScheme(overlay, fields, 'MEDLYN');
  assert.equal(next.physics.DEF_USE_MEDLYNST, '.true.');
  assert.equal(next.physics.DEF_USE_WUEST, '.false.');
  next = setStomatalScheme(next, fields, 'BALL_BERRY');
  assert.equal(next.physics.DEF_USE_MEDLYNST, '.false.');
  assert.equal(next.physics.DEF_USE_WUEST, '.false.');
  assert.throws(() => setStomatalScheme(overlay, fields, 'bogus'));
});

test('appending a family preserves a custom type already stored by the chosen source', () => {
  const overlay = empty();
  overlay.physics = {
    DEF_TRACER_NUM: '1',
    DEF_TRACER_NAMES: "'custom'",
    DEF_TRACER_TYPES: "'custom-type'",
  };
  const fields = tracerKeys.map((k) => field(k, k === 'DEF_TRACER_NUM' ? 'integer' : 'character'));
  const rows = appendTracerFamily(tracerRows(overlay), 'chloride');
  const result = setTracerRows(overlay, fields, rows);
  assert.equal(result.physics.DEF_TRACER_TYPES, "'custom-type,solute'");
  assert.throws(() => setTracerRows(empty(), fields, rows));
});

test('methane requires an explicit case scheme and ignores version scheme flags', () => {
  const overlay = empty();
  overlay.physics = {
    DEF_USE_TRACER: '.true.',
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_BGC: '.true.',
    DEF_USE_PFT: '.true.',
  };
  const context = { kernelMacros: ['SinglePoint'], caseScope: 'site' as const };
  assert.equal(tracerFamilyState(overlay, 'methane', context).status, 'pending');
  for (const subgrid of ['IGBP', 'USGS'])
    assert.equal(tracerFamilyState(overlay, 'methane', { ...context, subgrid }).status, 'blocked');
  for (const subgrid of ['PFT', 'PC'])
    assert.equal(tracerFamilyState(overlay, 'methane', { ...context, subgrid }).status, 'ready');
});
test('a saved METHANE alias prevents a second methane family row', () => {
  const rows = [
    {
      NAMES: 'METHANE',
      TYPES: 'gas',
      MRAT: '16.04',
      REF_RATIO: '1',
      INIT_DELTA: '0',
      REACTIVE_DECAY_RATE: '',
      PARAM_FILES: 'methane.nml',
    },
  ];
  assert.deepEqual(appendTracerFamily(rows, 'methane'), rows);
});

test('case options override stale model switches without promoting defaults or editing parameters', async () => {
  const { caseOptionOverlay, caseOptionPatch } = await import('./configuration-options');
  const original = empty();
  original.physics = { DEF_USE_BGC: '.true.', DEF_TUNING_CSOILC: '2' };
  const fields = [field('DEF_USE_BGC'), field('DEF_USE_TRACER')];
  const effective = caseOptionOverlay(original, fields, { DEF_USE_BGC: '.false.' }, [
    {
      name: 'DEF_USE_TRACER',
      configured_value: null,
      built_in_default: '.true.',
      mode: 'editable',
      allowed_values: [],
      pending: true,
    },
  ]);
  assert.equal(effective.physics.DEF_USE_BGC, '.false.');
  assert.equal(effective.physics.DEF_USE_TRACER, undefined);
  assert.equal(effective.physics.DEF_TUNING_CSOILC, '2');
  assert.deepEqual(caseOptionPatch(effective, resetModelOption(effective, 'DEF_USE_BGC'), fields), {
    DEF_USE_BGC: null,
  });
  assert.equal(original.physics.DEF_USE_BGC, '.true.');
});

test('case child visibility follows ancestor gates without treating readonly parents as off', () => {
  const gate = (
    name: string,
    parent_gate?: string,
    parent_gate_enabled?: boolean,
  ): ConfigurationField => ({
    name,
    parent_gate,
    parent_gate_enabled,
    mode: 'editable',
    pending: true,
    configured_value: null,
    allowed_values: [],
  });
  const fields = [gate('SNICAR'), gate('AEROSOL', 'SNICAR', false), gate('FILE', 'AEROSOL', true)];
  const before = JSON.stringify(fields);
  assert.equal(caseFieldVisible('FILE', fields), false);
  assert.equal(JSON.stringify(fields), before);
  fields[1].parent_gate_enabled = true;
  assert.equal(caseFieldVisible('file', fields), true);
  fields[0] = { ...fields[0], mode: 'disabled', pending: false };
  assert.equal(
    caseFieldVisible('FILE', fields),
    true,
    'derived readonly parent is not a disabled gate',
  );
  assert.equal(caseFieldVisible('UNKNOWN', fields), true);
  fields[0].parent_gate = 'FILE';
  fields[0].parent_gate_enabled = true;
  assert.equal(caseFieldVisible('FILE', fields), false, 'cyclic metadata cannot recurse forever');
});
