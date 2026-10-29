import test from 'node:test';
import assert from 'node:assert/strict';
import type { Overlay } from '../../lib/types';
import type { Descriptor, ModelField } from './model';
import {
  caseFieldVisible,
  tracerFieldVisible,
  cnInitialStatePatch,
  vicParameterDefault,
  type ConfigurationField,
  tracerFamilyState,
  tracerSetupIssues,
  appendTracerFamily,
  applyTracerSelection,
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
  const d = descriptor('process:init', 'DEF_SOLUTE%init_conc');
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
test('tracer table maps family switches and invalid input does not mutate the draft', () => {
  const fields = tracerKeys.map((k) =>
    field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character'),
  );
  const overlay = empty();
  // O18 不是上游支持的物种名（只认 H2_18O / HDO），应先拒绝而不是写进配置。
  assert.throws(() =>
    setTracerRows(overlay, fields, [
      { NAMES: 'CH4', TYPES: 'gas', PARAM_FILES: 'methane.nml' },
      { NAMES: 'O18', TYPES: 'isotope', PARAM_FILES: 'isotope.nml' },
    ]),
  );
  assert.deepEqual(overlay, empty());
  const valid = [
    { NAMES: 'CH4', TYPES: 'gas', PARAM_FILES: 'methane.nml' },
    { NAMES: 'HDO', TYPES: 'isotope', PARAM_FILES: 'isotope.nml' },
  ];
  const next = setTracerRows(overlay, fields, valid);
  assert.equal(next.physics.DEF_USE_METHANE, '.true.');
  assert.equal(next.physics.DEF_METHANE_PARAM_FILE, "'methane.nml'");
  assert.equal(next.physics.DEF_USE_ISOTOPE, '.true.');
  assert.equal(next.physics.DEF_ISOTOPE_SPECIES, "'HDO'");
  assert.equal(next.physics.DEF_ISOTOPE_PARAM_FILE, "'isotope.nml'");
  const names = (rows: ReturnType<typeof tracerRows>) => rows.map((row) => row.NAMES).sort();
  assert.deepEqual(names(tracerRows(next)), names(valid));
  assert.throws(() => setTracerRows(overlay, fields, [valid[0], { ...valid[1], NAMES: 'CH4' }]));
  assert.deepEqual(overlay, empty());
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
test('solute files resolve keyed and positional mappings and serialize one name:path entry per solute', () => {
  const fields = tracerKeys.map((k) =>
    field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character'),
  );
  const overlay = empty();
  overlay.physics = {
    DEF_SOLUTE_NAMES: "'Cl,NO3'",
    DEF_SOLUTE_PARAM_FILES: "'NO3:no3.nml; Cl:cl.nml'",
  };
  let rows = tracerRows(overlay);
  assert.deepEqual(
    rows.map((row) => row.PARAM_FILES),
    ['cl.nml', 'no3.nml'],
  );
  assert.equal(
    setTracerRows(overlay, fields, rows).physics.DEF_SOLUTE_PARAM_FILES,
    "'Cl:cl.nml,NO3:no3.nml'",
  );
  rows[0].PARAM_FILES = 'new-cl.nml';
  const next = setTracerRows(overlay, fields, rows);
  assert.equal(next.physics.DEF_SOLUTE_PARAM_FILES, "'Cl:new-cl.nml,NO3:no3.nml'");
  assert.equal(next.physics.DEF_SOLUTE_NAMES, "'Cl,NO3'");
  overlay.physics.DEF_SOLUTE_PARAM_FILES = "'null; no3.nml'";
  rows = tracerRows(overlay);
  assert.deepEqual(
    rows.map((row) => row.PARAM_FILES),
    ['', 'no3.nml'],
  );
  overlay.physics.DEF_SOLUTE_PARAM_FILES = "'C:\\data\\cl.nml,no3.nml'";
  assert.deepEqual(
    tracerRows(overlay).map((row) => row.PARAM_FILES),
    ['C:\\data\\cl.nml', 'no3.nml'],
  );
  rows[0].PARAM_FILES = '';
  assert.equal(
    setTracerRows(overlay, fields, rows).physics.DEF_SOLUTE_PARAM_FILES,
    "'Cl:null,NO3:no3.nml'",
  );
});
test('tracer table rejects unsupported types and duplicate names atomically', () => {
  const fields = tracerKeys.map((k) =>
    field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character'),
  );
  const overlay = empty();
  assert.throws(() =>
    setTracerRows(overlay, fields, [{ NAMES: 'custom', TYPES: 'reactive', PARAM_FILES: '' }]),
  );
  assert.throws(() =>
    setTracerRows(overlay, fields, [
      { NAMES: 'CH4', TYPES: 'gas', PARAM_FILES: '' },
      { NAMES: 'CH4', TYPES: 'solute', PARAM_FILES: '' },
    ]),
  );
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

test('family presets only add missing names/types with built-in files without mutating existing rows', () => {
  const rows = [
    {
      NAMES: 'custom',
      TYPES: 'solute',
      PARAM_FILES: 'own.nml',
    },
  ];
  const result = appendTracerFamily(rows, 'isotope');
  assert.equal(result.length, 3);
  assert.deepEqual(result[0], rows[0]);
  assert.deepEqual(
    result.slice(1).map((r) => [r.NAMES, r.TYPES, r.PARAM_FILES]),
    [
      ['H2_18O', 'isotope', 'standard_isotope_parameter.nml'],
      ['HDO', 'isotope', 'standard_isotope_parameter.nml'],
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
    DEF_USE_ISOTOPE: '.true.',
    DEF_USE_METHANE: '.true.',
    DEF_USE_SEDIMENT: '.true.',
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

test('appending a solute family keeps an existing matching row and writes family fields', () => {
  const overlay = empty();
  overlay.physics = { DEF_SOLUTE_NAMES: "'Cl'", DEF_SOLUTE_PARAM_FILES: "'Cl:cl.nml'" };
  const fields = tracerKeys.map((k) =>
    field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character'),
  );
  const rows = appendTracerFamily(tracerRows(overlay), 'chloride');
  assert.equal(rows.length, 1);
  const result = setTracerRows(overlay, fields, rows);
  assert.equal(result.physics.DEF_SOLUTE_NAMES, "'Cl'");
  assert.equal(result.physics.DEF_SOLUTE_PARAM_FILES, "'Cl:cl.nml'");
  assert.equal(result.physics.DEF_USE_METHANE, '.false.');
});

test('methane requires an explicit case scheme and ignores version scheme flags', () => {
  const overlay = empty();
  overlay.physics = {
    DEF_USE_ISOTOPE: '.true.',
    DEF_USE_METHANE: '.true.',
    DEF_USE_SEDIMENT: '.true.',
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
      PARAM_FILES: 'methane.nml',
    },
  ];
  assert.deepEqual(appendTracerFamily(rows, 'methane'), rows);
});

test('case options override stale model switches without promoting defaults or editing parameters', async () => {
  const { caseOptionOverlay, caseOptionPatch } = await import('./configuration-options');
  const original = empty();
  original.physics = { DEF_USE_BGC: '.true.', DEF_TUNING_CSOILC: '2' };
  const fields = [field('DEF_USE_BGC'), field('DEF_USE_ISOTOPE')];
  const effective = caseOptionOverlay(original, fields, { DEF_USE_BGC: '.false.' }, [
    {
      name: 'DEF_USE_ISOTOPE',
      configured_value: null,
      built_in_default: '.true.',
      mode: 'editable',
      allowed_values: [],
      pending: true,
    },
  ]);
  assert.equal(effective.physics.DEF_USE_BGC, '.false.');
  assert.equal(effective.physics.DEF_USE_ISOTOPE, undefined);
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

test('isotope fields require an isotope family without changing unrelated fields', () => {
  const names = ['DEF_ISO_USE_FRACTIONATION', 'DEF_ISO_KINETIC_SCHEME', 'DEF_MT_SOIL_INIT_FILE'];
  for (const isotope of [undefined, false])
    for (const name of names) assert.equal(tracerFieldVisible(name, isotope), false);
  for (const name of names) assert.equal(tracerFieldVisible(name.toLowerCase(), true), true);
  assert.equal(tracerFieldVisible('DEF_SOLUTE_NAMES'), true);
  assert.equal(tracerFieldVisible('UNKNOWN'), true);
  const fields: ConfigurationField[] = [
    {
      name: 'DEF_USE_ISOTOPE',
      configured_value: '.true.',
      mode: 'editable',
      pending: true,
      allowed_values: [],
    },
  ];
  assert.equal(caseFieldVisible(names[0], fields), true);
  assert.equal(caseFieldVisible(names[0], fields, { def_use_isotope: '.false.' }), false);
});

test('fractionation visibility requires current explicit activation without guessing unrelated unknown fields', () => {
  const parent = 'DEF_ISO_USE_FRACTIONATION';
  const children = ['DEF_ISO_KINETIC_SCHEME', 'DEF_ISO_OPEN_WATER_KINETIC', 'DEF_ISO_SOIL_KINETIC'];
  for (const child of children) {
    const metadata: ConfigurationField[] = [
      {
        name: child,
        mode: 'editable',
        pending: true,
        configured_value: 'saved',
        allowed_values: [],
        parent_gate: parent,
        parent_gate_enabled: true,
      },
    ];
    for (const value of [undefined, '.false.', 'unknown']) {
      const values: Record<string, string> = {
        DEF_USE_ISOTOPE: '.true.',
        ...(value === undefined ? {} : { [parent]: value }),
      };
      assert.equal(caseFieldVisible(child, [], values), false);
      assert.equal(caseFieldVisible(child, metadata, values), false);
    }
    assert.equal(
      caseFieldVisible(child, metadata, { DEF_USE_ISOTOPE: '.true.', [parent]: '.true.' }),
      true,
    );
    metadata[0].parent_gate_enabled = false;
    assert.equal(
      caseFieldVisible(child, metadata, { DEF_USE_ISOTOPE: '.true.', [parent]: '.true.' }),
      false,
    );
  }
  assert.equal(caseFieldVisible('UNKNOWN_DATA', [], {}), true);
  assert.equal(
    caseFieldVisible(children[0].toLowerCase(), [], {
      def_use_isotope: '.true.',
      [parent.toLowerCase()]: '.TRUE.',
    }),
    true,
  );
  assert.equal(
    caseFieldVisible(children[0], []),
    false,
    'unknown tracer types cannot expose isotope-only options',
  );
});

test('path placeholders accept actual filenames while scientific enums remain closed', () => {
  for (const name of [
    'DEF_UnitCatchment_file',
    'DEF_file_cn_init',
    'DEF_MT_SOIL_INIT_FILE',
    'DEF_dir_forcing',
  ]) {
    const field = { name, kind: 'Character { len: 256 }', values: ['', 'NONE', 'NULL'] };
    assert.equal(
      fieldLiteral(field, "/Volumes/Data/O'Brien data.nc"),
      "'/Volumes/Data/O''Brien data.nc'",
    );
    assert.throws(() => fieldLiteral(field, 'bad\npath'), /Newlines|换行/);
  }
  assert.throws(
    () =>
      fieldLiteral(
        {
          name: 'DEF_ISO_KINETIC_SCHEME',
          kind: 'Character',
          values: ['MERLIVAT1978', 'CAPPA2003'],
        },
        '/a/file.nc',
      ),
    /supported|支持/,
  );
  assert.throws(
    () =>
      fieldLiteral(
        { name: 'DEF_PROCESS_MODE', kind: 'Character', values: ['NONE', 'NULL'] },
        '/a/file.nc',
      ),
    /supported|支持/,
  );
});

test('path placeholder contract recognizes complete name tokens and paired quoted sentinels', () => {
  const values = [" ' NONE ' ", ' " null " ', "'  '"];
  for (const name of [
    'DEF_namelist_forcing',
    'DEF_INIT%FiLe',
    'DEF_PATH%initial',
    'DEF_dir_forcing',
  ])
    assert.equal(
      fieldLiteral({ name, kind: 'Character', values }, '/data/input.nc'),
      "'/data/input.nc'",
    );
  for (const name of ['DEF_profile', 'DEF_direction', 'DEF_filepath', 'DEF_namelistmode'])
    assert.throws(
      () => fieldLiteral({ name, kind: 'Character', values }, '/data/input.nc'),
      /supported|支持/,
    );
  for (const invalid of ['\'NULL"', 'NOT_NULL'])
    assert.throws(
      () =>
        fieldLiteral(
          { name: 'DEF_file_input', kind: 'Character', values: [invalid] },
          '/data/input.nc',
        ),
      /supported|支持/,
    );
});

test('CN runtime defaults preserve quoting and platform paths without inventing missing data', () => {
  for (const [runtime, expected] of [
    ["'/data/runtime/'", "'/data/runtime/cnsteadystate.nc'"],
    ["'/'", "'/cnsteadystate.nc'"],
    ["'C:\\runtime\\'", "'C:\\runtime\\cnsteadystate.nc'"],
    ["'data/O''Brien'", "'data/O''Brien/cnsteadystate.nc'"],
  ]) {
    const values = {
      DEF_USE_CN_INIT: '.true.',
      DEF_dir_runtime: runtime,
      DEF_file_cn_init: "'null'",
    };
    assert.deepEqual(cnInitialStatePatch(empty(), values, {}), { DEF_file_cn_init: expected });
    assert.deepEqual(cnInitialStatePatch(empty(), values, { DEF_file_cn_init: "'custom.nc'" }), {});
    assert.deepEqual(cnInitialStatePatch(empty(), values, { DEF_USE_CN_INIT: '.false.' }), {});
  }
  assert.deepEqual(cnInitialStatePatch(empty(), { DEF_USE_CN_INIT: '.true.' }, {}), {});
  assert.deepEqual(cnInitialStatePatch(empty(), {}, {}), {});
});

test('interception editing offers only CoLM schemes 1 and 8', () => {
  const field = {
    name: 'DEF_Interception_scheme',
    kind: 'integer',
    values: ['1', '2', '3', '4', '5', '6', '7', '8'],
  };
  assert.equal(fieldLiteral(field, '1'), '1');
  assert.equal(fieldLiteral(field, '8'), '8');
  assert.throws(() => fieldLiteral(field, '2'), /supported/);
});

test('VIC file defaults preserve the runtime convention and missing-data sentinels', () => {
  assert.equal(vicParameterDefault('DEF_file_VIC_para', "'/runtime'"), '/runtime/vic/vic_para.txt');
  assert.equal(vicParameterDefault('DEF_file_VIC_OPT', "'/runtime/'"), '/runtime/vic/vic_para.nc');
  for (const value of ['', "'null'", 'NONE'])
    assert.equal(vicParameterDefault('DEF_file_VIC_para', value), '');
});

test('VIC controls follow runoff and parameter source even while applicability is loading', () => {
  for (const runoff of [undefined, '0', '2', '3'])
    for (const name of ['DEF_VIC_OPT', 'DEF_file_VIC_para', 'DEF_file_VIC_OPT'])
      assert.equal(caseFieldVisible(name, [], runoff ? { DEF_Runoff_SCHEME: runoff } : {}), false);
  for (const grid of [false, true]) {
    const values = { DEF_Runoff_SCHEME: '1', DEF_VIC_OPT: grid ? '.true.' : '.false.' };
    assert.equal(caseFieldVisible('DEF_VIC_OPT', [], values), true);
    assert.equal(caseFieldVisible('DEF_file_VIC_para', [], values), !grid);
    assert.equal(caseFieldVisible('DEF_file_VIC_OPT', [], values), grid);
  }
});

test('tracer family defaults match VG runtime without requiring an unspecified generic kernel', () => {
  const overlay = empty();
  overlay.physics.DEF_USE_ISOTOPE = '.true.';
  overlay.physics.DEF_USE_METHANE = '.true.';
  overlay.physics.DEF_USE_SEDIMENT = '.true.';
  overlay.physics.DEF_USE_VariablySaturatedFlow = '.false.';
  const defaults = {
    def_use_campbell_soil_model: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_BGC: '.true.',
    DEF_USE_LULCC: '.false.',
  };
  const context = { defaults, subgrid: 'PC' };
  const before = structuredClone(overlay);
  assert.equal(tracerFamilyState(overlay, 'methane', context).status, 'ready');
  assert.equal(tracerFamilyState(overlay, 'isotope', context).status, 'ready');
  assert.equal(tracerFamilyState(overlay, 'sediment', context).status, 'pending');
  assert.equal(
    tracerFamilyState(overlay, 'methane', {
      ...context,
      engine: 'fortran',
      kernelMacros: ['OPENMP'],
    }).status,
    'blocked',
  );
  assert.equal(
    tracerFamilyState(overlay, 'methane', { ...context, engine: 'rust', kernelMacros: ['OPENMP'] })
      .status,
    'ready',
  );
  assert.deepEqual(overlay, before);
  overlay.physics.DEF_URBAN_RUN = '.true.';
  assert.equal(tracerFamilyState(overlay, 'methane', context).status, 'blocked');
});

test('tracer readiness requires actual species and required metadata while preserving external paths', () => {
  const overlay = empty();
  const defaults = {
    DEF_USE_ISOTOPE: '.true.',
    DEF_USE_METHANE: '.true.',
    DEF_USE_SEDIMENT: '.true.',
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_BGC: '.true.',
    DEF_USE_LULCC: '.false.',
  };
  const context = { defaults, subgrid: 'PFT', engine: 'rust' };
  assert.equal(tracerSetupIssues(overlay, context).length, 1);
  Object.assign(overlay.physics, {
    DEF_USE_METHANE: '.true.',
    DEF_METHANE_PARAM_FILE: "'null'",
  });
  assert.equal(tracerRows(overlay)[0].PARAM_FILES, '');
  assert.equal(tracerSetupIssues(overlay, context).length, 1);
  for (const path of [
    'standard_methane_parameter.nml',
    '/server/input/methane.nml',
    'process/frozen.nml',
  ]) {
    overlay.physics.DEF_METHANE_PARAM_FILE = `'${path}'`;
    assert.deepEqual(tracerSetupIssues(overlay, context), []);
  }
  overlay.physics.DEF_USE_METHANE = '.false.';
  overlay.physics.DEF_USE_ISOTOPE = '.true.';
  overlay.physics.DEF_ISOTOPE_SPECIES = "'HDO'";
  overlay.physics.DEF_ISOTOPE_PARAM_FILE = "'none'";
  for (const value of ['0', '-1', 'NaN', 'Infinity', '']) {
    overlay.physics.DEF_MT_AQUIFER_MIXING_WATER_MM = value;
    assert.equal(tracerSetupIssues(overlay, context).length, 1);
  }
  delete overlay.physics.DEF_MT_AQUIFER_MIXING_WATER_MM;
  assert.deepEqual(
    tracerSetupIssues(overlay, {
      ...context,
      defaults: { ...defaults, DEF_MT_AQUIFER_MIXING_WATER_MM: '1.d2' },
    }),
    [],
  );
});

test('all tracer definitions enforce common prerequisites', () => {
  const overlay = empty();
  Object.assign(overlay.physics, {
    DEF_SOLUTE_NAMES: "'Cl'",
    DEF_SOLUTE_PARAM_FILES: "'Cl:null'",
  });
  const defaults = {
    DEF_USE_ISOTOPE: '.true.',
    DEF_USE_METHANE: '.true.',
    DEF_USE_SEDIMENT: '.true.',
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
  };
  assert.deepEqual(tracerSetupIssues(overlay, { defaults }), []);
  assert.deepEqual(tracerSetupIssues(overlay, {}), []);
  for (const key of ['DEF_USE_Campbell_SOIL_MODEL', 'DEF_URBAN_RUN']) {
    overlay.physics[key] = '.true.';
    assert.equal(tracerSetupIssues(overlay, { defaults }).length, 1);
    delete overlay.physics[key];
  }
  assert.equal(tracerSetupIssues(overlay, { defaults, kernelMacros: ['CATCHMENT'] }).length, 1);
  assert.equal(
    tracerSetupIssues(overlay, { defaults, kernelMacros: ['OPENMP'], engine: 'fortran' }).length,
    1,
  );
  assert.deepEqual(
    tracerSetupIssues(overlay, { defaults, kernelMacros: ['OPENMP'], engine: 'rust' }),
    [],
  );
});

test('verified active methane inundation modes require routing without rejecting external parameter references', () => {
  const overlay = empty();
  Object.assign(overlay.physics, {
    DEF_USE_METHANE: '.true.',
    DEF_METHANE_PARAM_FILE: "'/server/input/methane.nml'",
  });
  const defaults = {
    DEF_USE_ISOTOPE: '.true.',
    DEF_USE_METHANE: '.true.',
    DEF_USE_SEDIMENT: '.true.',
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_BGC: '.true.',
    DEF_USE_GridRiverLakeFlow: '.false.',
  };
  const context = { defaults, subgrid: 'PFT' };
  const processFiles = (
    mode: string,
    verified = true,
    active = true,
  ): NonNullable<import('./configuration-options').FamilyContext['processFiles']> => [
    {
      file: 'methane.nml',
      title: '',
      section: '',
      verified,
      active_methane_asset: active,
      entries: [
        {
          path: 'DEF_METHANE%inundation_mode',
          value: mode,
          unset: false,
          group: 'methane',
          kind: 'choice',
          asset_value: mode,
          default: 'wetwat',
          options: [],
          editable: true,
          descriptor_id: null,
          pending: false,
        },
      ],
    },
  ];
  assert.deepEqual(tracerSetupIssues(overlay, context), []);
  for (const mode of ['routing', 'hybrid', 'dh_all_thr05', 'dyn_routing_hybrid']) {
    assert.equal(
      tracerSetupIssues(overlay, { ...context, processFiles: processFiles(mode) }).length,
      1,
    );
    assert.deepEqual(
      tracerSetupIssues(overlay, { ...context, processFiles: processFiles(mode, false) }),
      [],
    );
    assert.deepEqual(
      tracerSetupIssues(overlay, { ...context, processFiles: processFiles(mode, true, false) }),
      [],
    );
    assert.deepEqual(
      tracerSetupIssues(overlay, {
        ...context,
        defaults: { ...defaults, DEF_USE_GridRiverLakeFlow: '.true.' },
        processFiles: processFiles(mode),
      }),
      [],
    );
  }
  assert.deepEqual(
    tracerSetupIssues(overlay, { ...context, processFiles: processFiles('wetwat') }),
    [],
  );
});

test('applying methane selection enables BGC atomically without changing dry-run validation or unrelated metadata', () => {
  const fields = [
    ...tracerKeys.map((k) => field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character')),
    field('DEF_USE_BGC'),
  ];
  const overlay = empty();
  overlay.physics.DEF_USE_BGC = '.false.';
  overlay.physics.KEEP = 'saved';
  const d = descriptor('process:init', 'DEF_SOLUTE%init_conc');
  overlay.process_files = [{ relative_path: 'custom.nml', sha256: 'a' }];
  const saved = setScopedOption(overlay, d, '2.', 0, 'custom.nml');
  const before = structuredClone(saved);
  for (const name of ['CH4', 'ch4', 'METHANE', 'Methane']) {
    const rows = appendTracerFamily([], 'methane').map((row) => ({ ...row, NAMES: name }));
    assert.equal(setTracerRows(saved, fields, rows).physics.DEF_USE_BGC, '.false.');
    const next = applyTracerSelection(saved, fields, rows);
    assert.equal(next.physics.DEF_USE_BGC, '.true.');
    assert.equal(next.physics.KEEP, 'saved');
    assert.deepEqual(next.parameters, saved.parameters);
    assert.deepEqual(next.process_files, saved.process_files);
  }
  assert.deepEqual(saved, before);
  const rows = appendTracerFamily([], 'methane');
  assert.throws(
    () =>
      applyTracerSelection(
        saved,
        fields.filter((f) => f.name !== 'DEF_USE_BGC'),
        rows,
      ),
    /not writable/,
  );
  assert.throws(
    () =>
      applyTracerSelection(
        saved,
        fields.map((f) => (f.name === 'DEF_USE_BGC' ? { ...f, derived: true } : f)),
        rows,
      ),
    /not writable/,
  );
  assert.deepEqual(saved, before);
});

test('nonmethane selections retain the existing BGC choice', () => {
  const fields = tracerKeys.map((k) =>
    field(k, k.startsWith('DEF_USE_') ? 'logical' : 'character'),
  );
  for (const bgc of ['.true.', '.false.']) {
    const overlay = empty();
    overlay.physics.DEF_USE_BGC = bgc;
    for (const family of ['isotope', 'chloride', 'sediment'] as const)
      assert.equal(
        applyTracerSelection(overlay, fields, appendTracerFamily([], family)).physics.DEF_USE_BGC,
        bgc,
      );
    const solute = appendTracerFamily([], 'methane').map((row) => ({
      ...row,
      NAMES: 'Cl',
      TYPES: 'solute',
    }));
    assert.equal(applyTracerSelection(overlay, fields, solute).physics.DEF_USE_BGC, bgc);
  }
});
