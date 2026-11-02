import test from 'node:test';
import assert from 'node:assert/strict';
import { makeRecord, upsertParameter, importParameters, type Descriptor } from './model';
const d = {
  id: 'lct:IGBP:x',
  raw_key: 'x',
  storage: 'case-nml',
  scope: 'land-cover-class',
  unit: 'm',
} as Descriptor;
test('typed scope distinguishes land classes and validates indices', () => {
  const a = makeRecord(d, '1', 1, ''),
    b = makeRecord(d, '2', 2, '');
  assert.equal(upsertParameter([a], b).length, 2);
  assert.equal(upsertParameter([a], { ...a, value: '3' })[0].value, '3');
  assert.throws(() => makeRecord(d, '1', 18, ''));
});
test('imports reject dataset fields and duplicate overrides', () => {
  const r = makeRecord(d, '1', 1, '');
  assert.throws(() => importParameters(JSON.stringify([r]), [d], [], []));
  assert.throws(() => importParameters(JSON.stringify([r, r]), [d], [{ name: 'x' } as never], []));
});

import { foundationControls, foundationValue, setFoundation, type ModelField } from './model';
const foundationFields = foundationControls
  .flatMap((c) => c.keys)
  .map((name) => ({ name, derived: false }) as ModelField);
test('model foundation cannot change the case vegetation scheme', () => {
  assert.throws(() => setFoundation({}, [], foundationFields, 'subgrid', 'PC'), /case/);
});
test('vG enables variably saturated flow and preserves unrelated physics', () => {
  const next = setFoundation({ keep: '1' }, [], foundationFields, 'soil', 'vG');
  assert.equal(next.DEF_USE_Campbell_SOIL_MODEL, '.false.');
  assert.equal(next.DEF_USE_VariablySaturatedFlow, '.true.');
  assert.equal(next.keep, '1');
  assert.equal(foundationValue(next, 'soil'), 'vG');
  assert.equal(foundationValue({ DEF_USE_Campbell_SOIL_MODEL: '.false.' }, 'soil'), '');
  assert.equal(
    setFoundation(next, [], foundationFields, 'soil', 'Campbell').DEF_USE_VariablySaturatedFlow,
    '.true.',
  );
});
test('foundation controls read explicit case-scalar parameter overrides', () => {
  const parameters = ['LCT', 'PFT', 'PC'].map((name) => ({
    raw_key: `def_use_${name.toLowerCase()}`,
    value: name === 'PFT' ? '.true.' : '.false.',
    scope_instance: { kind: 'case-scalar' },
  })) as Parameters<typeof foundationValue>[2];
  assert.equal(foundationValue({}, 'subgrid', parameters), 'PFT');
  assert.equal(
    setFoundation({}, parameters!, foundationFields, 'DEF_USE_BGC', '.true.', 'PFT').DEF_USE_BGC,
    '.true.',
  );
  const soil = [
    {
      raw_key: 'DEF_USE_Campbell_SOIL_MODEL',
      value: '.false.',
      scope_instance: { kind: 'case-scalar' },
    },
    {
      raw_key: 'DEF_USE_VariablySaturatedFlow',
      value: '.true.',
      scope_instance: { kind: 'case-scalar' },
    },
  ] as Parameters<typeof foundationValue>[2];
  assert.equal(foundationValue({}, 'soil', soil), 'vG');
});
test('BGC uses case scheme rather than stale version flags', () => {
  const physics = { DEF_USE_PFT: '.true.' };
  assert.throws(
    () => setFoundation(physics, [], foundationFields, 'DEF_USE_BGC', '.true.', 'IGBP'),
    /PFT/,
  );
  assert.throws(() => setFoundation(physics, [], foundationFields, 'DEF_USE_BGC', '.true.'), /PFT/);
  assert.equal(
    setFoundation({}, [], foundationFields, 'DEF_USE_BGC', '.true.', 'PC').DEF_USE_BGC,
    '.true.',
  );
  assert.deepEqual(physics, { DEF_USE_PFT: '.true.' });
  const record = { raw_key: 'DEF_USE_BGC', value: '.false.' } as never;
  assert.throws(
    () => setFoundation({}, [record], foundationFields, 'DEF_USE_BGC', '.true.', 'PFT'),
    /conflicting/,
  );
  assert.throws(
    () =>
      setFoundation(
        {},
        [],
        foundationFields.map((f) => ({ ...f, derived: true })),
        'soil',
        'vG',
      ),
    /writable/,
  );
});

test('parameter imports report contextual errors for malformed JSON and invalid record shapes', () => {
  assert.throws(() => importParameters('{', [d], [], []), /Parameter import.*JSON/);
  for (const text of ['null', '1', 'true', '"text"', '[null]', '[1]', '[{}]']) {
    assert.throws(
      () => importParameters(text, [d], [], []),
      (error: unknown) => {
        assert.ok(error instanceof Error);
        assert.equal(error instanceof TypeError, false);
        assert.match(error.message, /Parameter import|records array/);
        return true;
      },
      text,
    );
  }
});

test('parameter import validates field types with localized errors before building records', () => {
  const record = makeRecord(d, '1', 1, '');
  const fields = [{ name: 'x' } as never];
  const invalid = [
    ...[null, 42, {}].map((value) => ({ ...record, value })),
    ...[42, {}].map((process_file) => ({
      ...record,
      scope_instance: { ...record.scope_instance, process_file },
    })),
    { ...record, parameter_id: 42 },
    { ...record, raw_key: {} },
    { ...record, scope_instance: [] },
    ...[{ kind: 42 }, { scheme: {} }, { index: '1' }].map((scope) => ({
      ...record,
      scope_instance: { ...record.scope_instance, ...scope },
    })),
  ];
  const previous = document.documentElement.lang;
  try {
    for (const [lang, message] of [
      ['en', 'Parameter import failed: invalid parameter record'],
      ['zh', '参数导入失败：参数记录格式不正确'],
    ]) {
      document.documentElement.lang = lang;
      for (const row of invalid) {
        assert.throws(
          () => importParameters(JSON.stringify([row]), [d], fields, []),
          (error: unknown) => {
            assert.ok(error instanceof Error);
            assert.equal(error instanceof TypeError, false);
            assert.equal(error.message, message);
            return true;
          },
        );
      }
      assert.deepEqual(importParameters(JSON.stringify([record]), [d], fields, []), [record]);
    }
  } finally {
    document.documentElement.lang = previous;
  }
});
