import test from 'node:test';
import assert from 'node:assert/strict';
import { parameterType, recordType, type Descriptor } from './model';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { literalNumber, valueProblem } = await import('./ParameterDialog');

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
