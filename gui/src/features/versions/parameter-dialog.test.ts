import test from 'node:test';
import assert from 'node:assert/strict';
import { parameterType, recordType, type Descriptor } from './model';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { docOf, literalNumber, rangeText, valueProblem } = await import('./ParameterDialog');

const descriptor = (patch: Partial<Descriptor>): Descriptor => ({
  id: 'case:DEF_X',
  raw_key: 'DEF_X',
  label_zh: 'x',
  label_en: 'x',
  scope: 'case-scalar',
  storage: 'case-nml',
  section: '',
  value_kind: 'real',
  unit: null,
  visibility: 'editable-expert',
  activation: [],
  default: null,
  doc_zh: '',
  doc_en: '',
  validation: { min: '0', max: '1', allowed_values: [], finite: true },
  ...patch,
});

test('each parameter goes to the editor of its kind and records map back to the same kind', () => {
  assert.equal(parameterType(descriptor({})), 'global');
  assert.equal(
    parameterType(descriptor({ id: 'lct:USGS:DEF_LC_VMAX25', scope: 'land-cover-class' })),
    'usgs',
  );
  assert.equal(
    parameterType(descriptor({ id: 'lct:IGBP:DEF_LC_VMAX25', scope: 'land-cover-class' })),
    'igbp',
  );
  assert.equal(parameterType(descriptor({ scope: 'pft-type', storage: 'pft-override' })), 'pft');
  assert.equal(
    parameterType(descriptor({ scope: 'pc-pft-component', storage: 'pc-pft-override' })),
    'pc',
  );
  assert.equal(
    parameterType(descriptor({ scope: 'process-file', storage: 'process-parameter-file' })),
    'process',
  );
  const record = (kind: string, scheme?: string) =>
    ({
      parameter_id: 'x',
      raw_key: 'x',
      value: '1',
      unit: null,
      scope_instance: { kind, scheme },
    }) as Parameters<typeof recordType>[0];
  assert.equal(recordType(record('land-cover-class', 'USGS')), 'usgs');
  assert.equal(recordType(record('land-cover-class', 'IGBP')), 'igbp');
  assert.equal(recordType(record('pc-pft')), 'pc');
  assert.equal(recordType(record('case-scalar')), 'global');
});

test('namelist numbers are read in Fortran spellings and checked against the catalog range', () => {
  assert.equal(literalNumber('1.5d-3'), 0.0015);
  assert.equal(literalNumber('0.01_r8'), 0.01);
  assert.equal(literalNumber('.5'), 0.5);
  assert.equal(literalNumber('abc'), null);
  assert.equal(literalNumber(''), null);
  const d = descriptor({});
  assert.equal(valueProblem(d, '0.3', false), '');
  assert.equal(valueProblem(d, '1.5', false), '不能大于 1');
  assert.equal(valueProblem(d, '-1', true), 'Below 0');
  assert.equal(valueProblem(d, 'x', false), '不是数');
  assert.equal(valueProblem(descriptor({ value_kind: 'integer' }), '0.5', false), '须为整数');
  assert.equal(valueProblem(descriptor({ value_kind: 'logical' }), '.TRUE.', false), '');
  assert.equal(
    valueProblem(descriptor({ value_kind: 'logical' }), 'yes', false),
    '只能是 .true. 或 .false.',
  );
});

test('ranges read as > 0 or < 0 instead of the spelled-out smallest double', () => {
  // The catalog writes "must be > 0" as the smallest positive double, 300-odd digits long.
  const tiny = `0.${'0'.repeat(307)}22250738585072014`;
  const sqrtdi = descriptor({
    validation: { min: tiny, max: null, allowed_values: [], finite: true },
  });
  const psi50 = descriptor({
    validation: { min: null, max: `-${tiny}`, allowed_values: [], finite: true },
  });
  assert.equal(rangeText(sqrtdi, false), '> 0');
  assert.equal(rangeText(psi50, false), '< 0');
  assert.equal(rangeText(descriptor({}), false), '0 … 1');
  assert.equal(
    rangeText(
      descriptor({ validation: { min: '1.e3_r8', max: null, allowed_values: [], finite: true } }),
      true,
    ),
    '≥ 1000',
  );
  assert.equal(
    rangeText(
      descriptor({ validation: { min: null, max: null, allowed_values: [], finite: true } }),
      false,
    ),
    '',
  );
  assert.equal(valueProblem(sqrtdi, '0', false), '必须大于 0');
  assert.equal(valueProblem(psi50, '0', true), 'Must be below 0');
  assert.equal(valueProblem(sqrtdi, '5', false), '');
});

test('placeholder docs that only repeat the key are not shown', () => {
  const generic = descriptor({
    doc_zh: 'CoLM 配置字段 DEF_LC_PSI50_ROOT。',
    doc_en: 'CoLM configuration field DEF_LC_PSI50_ROOT.',
  });
  assert.equal(docOf(generic, false), '');
  assert.equal(docOf(generic, true), '');
  const real = descriptor({ doc_zh: '根50%失导水势；按所选 PFT/CFT 槽位稀疏覆盖。' });
  assert.equal(docOf(real, false), '根50%失导水势；按所选 PFT/CFT 槽位稀疏覆盖。');
});
