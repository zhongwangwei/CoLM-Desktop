import test from 'node:test';
import assert from 'node:assert/strict';
import type { Descriptor } from './model';
import { applyResearchSeed, type ParameterContext } from '../studies/study';
const store = new Map<string, string>();
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => store.set(k, v),
  },
});
const { defaultRange, loadPicks } = await import('./StudyPicks');

const descriptor = (min: string | null, max: string | null) =>
  ({ validation: { min, max, allowed_values: [], finite: true } }) as unknown as Descriptor;

test('a study range starts at half to one and a half times the default, inside the catalog bounds', () => {
  assert.deepEqual(defaultRange(descriptor('0', null), 52), ['26', '78']);
  assert.deepEqual(defaultRange(descriptor('0', '1'), 0.8), ['0.4', '1']);
  assert.deepEqual(defaultRange(descriptor('-1', '1'), -0.3), ['-0.45', '-0.15']);
  assert.deepEqual(defaultRange(descriptor('0', '1'), 0), ['0', '1'], 'no default: catalog bounds');
  assert.deepEqual(defaultRange(descriptor(null, null), null), ['', '']);
});

test('picks remembered per version survive, and unreadable storage just means none', () => {
  store.set('colm.versions.studyPicks.model-1', JSON.stringify([{ parameter_id: 'x' }]));
  assert.equal(loadPicks('model-1').length, 1);
  store.set('colm.versions.studyPicks.model-2', '{broken');
  assert.deepEqual(loadPicks('model-2'), []);
  assert.deepEqual(loadPicks('model-3'), []);
});

test('picks from the version page tick the matching research rows only', () => {
  const row = (id: string, index: number | null, scheme: string | null = null) =>
    ({ id, scope_instance: { kind: 'x', index, scheme } }) as unknown as ParameterContext;
  const rows = [
    row('pft:DEF_PFT_VMAX25', 13),
    row('pft:DEF_PFT_VMAX25', 14),
    row('lct:IGBP:DEF_LC_VMAX25', 10, 'IGBP'),
    row('case:DEF_TUNING_CNFAC', null),
  ];
  const pick = (parameter_id: string, index: number | null, scheme: string | null = null) => ({
    parameter_id,
    raw_key: '',
    scheme,
    index,
    min: '1',
    max: '2',
    scale: 'linear',
  });
  const picked = applyResearchSeed(rows, [
    pick('pft:DEF_PFT_VMAX25', 14),
    pick('lct:IGBP:DEF_LC_VMAX25', 10, 'IGBP'),
    pick('case:DEF_TUNING_CNFAC', null),
    pick('pft:DEF_PFT_VMAX25', 7),
  ]);
  assert.deepEqual(Object.keys(picked).sort(), ['1', '2', '3'], 'PFT 7 is not in these cases');
  assert.deepEqual(picked['1'], { enabled: true, min: '1', max: '2', scale: 'linear' });
});
