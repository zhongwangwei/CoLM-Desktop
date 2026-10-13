import test from 'node:test';
import assert from 'node:assert/strict';
import {
  applySetupRequest,
  setupChoices,
  setupHandledKeys,
  type SetupEnvironment,
} from './scientific-setup';
import type { Overlay, KernelEntry } from '../../lib/types';
import type { ModelField } from './model';
import type { ConfigurationField, ConfigurationResult } from './configuration-options';
const fields = setupHandledKeys.map(
  (name) =>
    ({
      name,
      kind: name === 'DEF_TUNING_CROP_PLANTING_DAY' ? 'integer' : 'logical',
      derived: false,
      values: [],
      default: '.false.',
      section: '',
    }) as ModelField,
);
const kernel = { dir: '/kernel', macros: ['GRIDBASED', 'GridRiverLakeFlow'] } as KernelEntry;
const environment: SetupEnvironment = {
  subgrid: 'PFT',
  domain: 'regional',
  grid: 'latlon',
  classification: 'IGBP',
  kernel,
  cropKernel: { ...kernel, macros: [...kernel.macros, 'CROP'] },
};
function overlay(): Overlay {
  return {
    physics: {
      DEF_USE_LCT: '.false.',
      DEF_USE_PFT: '.true.',
      DEF_USE_PC: '.false.',
      DEF_USE_BGC: '.false.',
      DEF_URBAN_RUN: '.false.',
      DEF_USE_Campbell_SOIL_MODEL: '.false.',
      DEF_USE_VariablySaturatedFlow: '.true.',
      DEF_UnitCatchment_regional: '.false.',
    },
    parameters: [],
    process_files: [],
  };
}
const apply = (o: Overlay, id: string, value: string, env = environment) =>
  applySetupRequest(o, fields, { id, value }, env);
test('ordered setup exposes every foundation and never fills frozen catalog defaults', () => {
  const options = setupChoices({ physics: {}, parameters: [], process_files: [] }, fields, {});
  assert.deepEqual(
    options.map((c) => c.id),
    [
      'soil',
      'DEF_USE_VariablySaturatedFlow',
      'DEF_USE_GridRiverLakeFlow',
      'DEF_UnitCatchment_regional',
      'DEF_USE_LULCC',
      'DEF_USE_BGC',
      'DEF_URBAN_RUN',
      'crop',
      'DEF_USE_TRACER',
    ],
  );
  assert.equal(options[0].value, '');
  assert.equal(options[0].pending, true);
  assert.match(options[5].choices[0].reason, /PFT/);
});
test('upstream edits reject active incompatible children without changing the original', () => {
  let o = apply(overlay(), 'DEF_USE_BGC', '.true.');
  o = apply(o, 'crop', '0');
  const before = structuredClone(o);
  assert.throws(() => apply(o, 'DEF_USE_BGC', '.false.'), /crop/);
  assert.throws(() => apply(o, 'subgrid', 'LCT'), /Unknown/);
  assert.deepEqual(o, before);
  o = apply(o, 'crop', '.false.');
  o = apply(o, 'DEF_USE_BGC', '.false.');
  let tracer = apply(overlay(), 'DEF_USE_TRACER', '.true.');
  assert.throws(() => apply(tracer, 'soil', 'Campbell'), /tracers/);
  assert.throws(() => apply(tracer, 'DEF_URBAN_RUN', '.true.'), /tracers/);
  assert.throws(() => apply(tracer, 'DEF_USE_VariablySaturatedFlow', '.false.'), /Campbell/);
  tracer = apply(tracer, 'DEF_USE_TRACER', '.false.');
  assert.equal(apply(tracer, 'soil', 'Campbell').physics.DEF_USE_Campbell_SOIL_MODEL, '.true.');
});
test('case scalar foundations migrate precisely and preserve unrelated child parameters', () => {
  const o = overlay();
  o.parameters = [
    {
      parameter_id: 'x',
      raw_key: 'DEF_USE_BGC',
      value: '.false.',
      unit: null,
      scope_instance: { kind: 'case-scalar' },
    },
    {
      parameter_id: 'y',
      raw_key: 'DEF_USE_FIRE',
      value: '.true.',
      unit: null,
      scope_instance: { kind: 'case-scalar' },
    },
  ];
  const next = apply(o, 'DEF_USE_BGC', '.true.');
  assert.equal(next.physics.DEF_USE_BGC, '.true.');
  assert.deepEqual(next.parameters, [o.parameters[1]]);
  assert.equal(o.parameters.length, 2);
  assert.equal(apply(next, 'DEF_USE_BGC', '.false.').parameters[0].value, '.true.');
});
test('crop accepts standard defaults and verified candidate macro; off preserves other crop values', () => {
  let o = apply(overlay(), 'DEF_USE_BGC', '.true.');
  assert.throws(() => apply(o, 'crop', '.true.'), /defaults/);
  assert.equal(
    applySetupRequest(
      o,
      fields,
      { id: 'crop', value: '.true.' },
      { ...environment, domain: 'single' },
      { fields: [], process_files: [], crop_defaults: { DEF_TUNING_CROP_PLANTING_DAY: '120' } },
    ).physics.DEF_TUNING_CROP_PLANTING_DAY,
    '120',
  );
  assert.throws(() => apply(o, 'crop', '120'), /spatial/);
  assert.throws(() => apply(o, 'crop', '367', { ...environment, domain: 'single' }));
  assert.throws(() => apply(o, 'crop', '0', { ...environment, cropKernel: undefined }), /Pending/);
  assert.throws(
    () => apply(o, 'crop', '0', { ...environment, cropKernel: { ...kernel, macros: [] } }),
    /CROP/,
  );
  o = apply(o, 'crop', '120', { ...environment, domain: 'single' });
  o.physics.DEF_TUNING_CROP_EXTRA = '9';
  assert.equal(apply(o, 'crop', '.false.').physics.DEF_TUNING_CROP_EXTRA, '9');
  assert.equal(o.physics.DEF_TUNING_CROP_PLANTING_DAY, '120');
});
test('river, tracer and LULCC honor scenario and capability prerequisites and schema', () => {
  const o = overlay();
  assert.throws(() =>
    apply(o, 'DEF_USE_GridRiverLakeFlow', '.true.', { ...environment, domain: 'single' }),
  );
  assert.throws(
    () => apply(o, 'DEF_USE_GridRiverLakeFlow', '.true.', { ...environment, kernel: undefined }),
    /Pending/,
  );
  assert.throws(() => apply(o, 'DEF_USE_TRACER', '.true.', { ...environment, grid: 'catchment' }));
  assert.throws(() =>
    apply(o, 'DEF_USE_LULCC', '.true.', { ...environment, classification: 'USGS' }),
  );
  o.physics.DEF_UnitCatchment_regional = '.true.';
  assert.throws(() => apply(o, 'DEF_USE_LULCC', '.true.'), /regional/);
  assert.throws(
    () => applySetupRequest(o, [], { id: 'soil', value: 'vG' }, environment),
    /writable/,
  );
});

test('site domain with null grid supports explicit tracer and crop choices and blocks urban BGC', () => {
  const env = { ...environment, domain: 'site', grid: null };
  let o = overlay();
  assert.equal(apply(o, 'DEF_USE_TRACER', '.true.', env).physics.DEF_USE_TRACER, '.true.');
  o = apply(o, 'DEF_USE_BGC', '.true.', env);
  assert.equal(apply(o, 'crop', '120', env).physics.DEF_TUNING_CROP_PLANTING_DAY, '120');
  assert.throws(() => apply(o, 'DEF_URBAN_RUN', '.true.', env), /BGC/);
  o.physics.DEF_USE_BGC = '.false.';
  o.physics.DEF_URBAN_RUN = '.true.';
  assert.throws(() => apply(o, 'DEF_USE_BGC', '.true.', env), /BGC/);
});
test('floodplain feedback and VG dependencies cannot silently be invalidated', () => {
  const o = overlay();
  assert.throws(() => apply(o, 'DEF_USE_VariablySaturatedFlow', '.false.'), /Campbell/);
  o.physics.DEF_GridRiverLake_FloodFeedback = '.true.';
  for (const [id, value] of [
    ['DEF_URBAN_RUN', '.true.'],
    ['DEF_USE_LULCC', '.true.'],
    ['DEF_USE_GridRiverLakeFlow', '.false.'],
  ])
    assert.throws(() => apply(o, id, value), /floodplain/);
});

test('confirmed backend disabled gates block activation, but allow explicit recovery off', () => {
  const context = [
    { name: 'DEF_USE_BGC', mode: 'disabled', reason: 'Source gate', pending: false },
  ] as ConfigurationField[];
  const c = setupChoices(overlay(), fields, environment, context).find(
    (c) => c.id === 'DEF_USE_BGC',
  )!;
  assert.equal(c.choices[0].reason, 'Source gate');
  assert.equal(c.choices[1].reason, '');
  assert.equal(
    setupChoices(overlay(), fields, environment).find((c) => c.id === 'crop')!.value,
    '',
  );
});

test('active tracer providers protect BGC, routing and LULCC upstream changes', () => {
  const o = overlay();
  o.physics.DEF_USE_TRACER = '.true.';
  o.physics.DEF_USE_BGC = '.true.';
  o.physics.DEF_USE_GridRiverLakeFlow = '.true.';
  o.physics.DEF_TRACER_NAMES = "'CH4'";
  assert.throws(() => apply(o, 'DEF_USE_BGC', '.false.'), /CH4|甲烷/);
  o.physics.DEF_TRACER_NAMES = "'SEDIMENT'";
  assert.throws(() => apply(o, 'DEF_USE_GridRiverLakeFlow', '.false.'), /SEDIMENT|泥沙/);
  o.physics.DEF_USE_BGC = '.false.';
  o.physics.DEF_TRACER_NAMES = "'HDO'";
  o.physics.DEF_TRACER_TYPES = "'isotope'";
  assert.throws(() => apply(o, 'DEF_USE_LULCC', '.true.'), /isotope|同位素/);
  assert.equal(
    apply(apply(o, 'DEF_USE_TRACER', '.false.'), 'DEF_USE_LULCC', '.true.').physics.DEF_USE_LULCC,
    '.true.',
  );
});
test('catchment urban activation is blocked by both explicit grid and actual kernel macro', () => {
  for (const env of [
    { ...environment, grid: 'catchment' },
    { ...environment, kernel: { ...kernel, macros: ['CATCHMENT'] } },
  ])
    assert.throws(() => apply(overlay(), 'DEF_URBAN_RUN', '.true.', env), /Catchment|流域/);
});

test('verified active methane inundation modes protect river while unknown remains pending', () => {
  const o = overlay();
  Object.assign(o.physics, {
    DEF_USE_TRACER: '.true.',
    DEF_USE_BGC: '.true.',
    DEF_TRACER_NAMES: "'CH4'",
    DEF_USE_GridRiverLakeFlow: '.true.',
  });
  const context = (mode: string, verified = true, active: boolean | null = true) =>
    ({
      fields: [],
      process_files: [
        {
          file: 'CH4.nml',
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
              options: ['wetwat', 'routing', 'hybrid'],
              editable: true,
              descriptor_id: null,
              pending: false,
            },
          ],
        },
      ],
    }) as Pick<ConfigurationResult, 'fields' | 'process_files'>;
  const call = (mode: string, verified = true, active: boolean | null = true) =>
    applySetupRequest(
      o,
      fields,
      { id: 'DEF_USE_GridRiverLakeFlow', value: '.false.' },
      environment,
      context(mode, verified, active),
    );
  for (const mode of ['routing', 'hybrid', 'dh_all_thr05', 'dyn_routing_hybrid'])
    assert.throws(() => call(mode), /甲烷|methane/);
  assert.equal(call('wetwat').physics.DEF_USE_GridRiverLakeFlow, '.false.');
  assert.throws(() => call('wetwat', false), /Pending|待核对/);
  assert.throws(() => call('wetwat', true, null), /Pending|待核对/);
  assert.throws(() => call('unknown'), /Pending|待核对/);
  assert.equal(
    apply(apply(o, 'DEF_USE_TRACER', '.false.'), 'DEF_USE_GridRiverLakeFlow', '.false.').physics
      .DEF_USE_GridRiverLakeFlow,
    '.false.',
  );
});

test('current compiled CROP remains a BGC prerequisite after removing the planting marker', () => {
  const o = overlay();
  o.physics.DEF_USE_BGC = '.true.';
  const env = { ...environment, kernel: { ...kernel, macros: [...kernel.macros, 'CROP'] } };
  assert.throws(() => apply(o, 'DEF_USE_BGC', '.false.', env), /CROP/);
  o.physics.DEF_TUNING_CROP_PLANTING_DAY = '0';
  const withoutDay = apply(o, 'crop', '.false.', env);
  assert.throws(() => apply(withoutDay, 'DEF_USE_BGC', '.false.', env), /CROP/);
  assert.equal(
    apply(withoutDay, 'DEF_USE_BGC', '.false.', environment).physics.DEF_USE_BGC,
    '.false.',
  );
});

test('regional routing is reachable before LULCC with explicit off allowed even when river is disabled', () => {
  const o = overlay();
  o.physics.DEF_USE_LULCC = '.false.';
  delete o.physics.DEF_UnitCatchment_regional;
  assert.throws(() => apply(o, 'DEF_USE_LULCC', '.true.'), /Pending/);
  const cleared = apply(o, 'DEF_UnitCatchment_regional', '.false.');
  assert.equal(apply(cleared, 'DEF_USE_LULCC', '.true.').physics.DEF_USE_LULCC, '.true.');
  assert.throws(() => apply(o, 'DEF_UnitCatchment_regional', '.true.'), /routing/);
  o.physics.DEF_USE_GridRiverLakeFlow = '.true.';
  assert.equal(
    apply(o, 'DEF_UnitCatchment_regional', '.true.').physics.DEF_UnitCatchment_regional,
    '.true.',
  );
  assert.throws(
    () => apply(o, 'DEF_UnitCatchment_regional', '.true.', { ...environment, domain: 'site' }),
    /spatial/,
  );
});
