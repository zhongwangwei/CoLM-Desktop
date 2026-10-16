import test from 'node:test';
import assert from 'node:assert/strict';
import { firstYearCutoff, planMultiSites, siteDraft } from './multisite-model';
import type { Site } from './case-model';

const natural: Site = {
  name: 'forest',
  site_file: '/forest.nc',
  met_file: '/forest-met.nc',
  obs_file: null,
  crop: false,
  urban: false,
  lon: 0,
  lat: 0,
  start: '2001-01-01',
  end: '2004-12-31',
  problem: null,
};
const crop = {
  ...natural,
  name: 'crop',
  site_file: '/crop.nc',
  met_file: '/crop-met.nc',
  crop: true,
  start: '2005-06-01',
  end: '2008-06-01',
};
const urban = { ...natural, name: 'urban', site_file: '/urban.nc', urban: true };
const base = {
  form: {
    engine: 'rust',
    kind: 'site',
    root: '/projects',
    name: 'batch',
    runtime: '/runtime',
    landcover: 'IGBP',
    subgrid: 'PC',
  },
  sites: [natural, crop, urban],
  drafts: [natural, crop, urban].map(siteDraft),
  mode: 'first-year' as const,
  repeat: '3',
  physics: { DEF_USE_BGC: '.true.', DEF_USE_TRACER: '.true.', DEF_TUNING_CROP_PLANTING_DAY: '120' },
  cropDefaults: { DEF_TUNING_CROP_PLANTING_DAY: '100', DEF_USE_BGC: '.true.' },
  step: 5 as const,
};

test('mixed station plans preserve individual inputs, periods and supported structures', () => {
  const [forest, farm, city] = planMultiSites(base);
  assert.equal(forest.form.root, '/projects/batch');
  assert.equal(forest.form.name, 'forest');
  assert.equal(forest.form.met, natural.met_file);
  assert.equal(forest.form.spinup_end, '2002-01-01');
  assert.equal(farm.form.spinup_end, '2006-06-01');
  assert.equal(forest.mode, 'pc');
  assert.equal(farm.mode, 'pc');
  assert.equal(city.mode, 'urban-igbp');
  assert.equal(farm.fields.DEF_TUNING_CROP_PLANTING_DAY, '120');
  assert.equal(farm.fields.DEF_USE_BGC, '.true.');
  assert.equal(farm.fields.DEF_USE_LULCC, '.false.');
  assert.equal(city.fields.DEF_USE_BGC, '.false.');
  assert.equal(city.fields.DEF_USE_TRACER, '.false.');
  assert.equal(forest.fields.DEF_USE_BGC, '.true.');
  assert.ok(!('DEF_TUNING_CROP_PLANTING_DAY' in forest.fields));
  assert.ok(!('DEF_TUNING_CROP_PLANTING_DAY' in city.fields));
  assert.equal(farm.fields['DEF_simulation_time%spinup_repeat'], '3');
  assert.equal(farm.fields['DEF_simulation_time%spinup_sec'], '0');
  assert.equal(base.physics.DEF_TUNING_CROP_PLANTING_DAY, '120');
});

test('per-site settings allow disabled or individually configured spinup', () => {
  const drafts = base.drafts.map((draft, i) => ({
    ...draft,
    spinup_end: i === 1 ? '2007-06-01' : '',
    spinup_repeat: i === 1 ? '5' : 'not-used',
  }));
  const plans = planMultiSites({ ...base, mode: 'per-site', drafts });
  assert.equal(plans[0].fields['DEF_simulation_time%spinup_year'], '0');
  assert.equal(plans[1].fields['DEF_simulation_time%spinup_repeat'], '5');
  assert.equal(plans[1].fields['DEF_simulation_time%spinup_year'], '2007');
});

test('planning rejects ambiguous destinations and invalid or stale data before creation', () => {
  for (const drafts of [
    [],
    [base.drafts[0], base.drafts[0]],
    [base.drafts[0], { ...base.drafts[1], name: 'FOREST' }],
    [{ ...base.drafts[0], name: '../escape' }],
    [{ ...base.drafts[0], site: '/missing.nc' }],
  ]) {
    assert.throws(() => planMultiSites({ ...base, drafts }), { name: 'MultiSiteValidationError' });
  }
  for (const changed of [
    { met_file: null },
    { start: null },
    { start: 'bad-date' },
    { problem: 'broken forcing' },
  ]) {
    assert.throws(() =>
      planMultiSites({
        ...base,
        sites: [{ ...natural, ...changed }],
        drafts: [siteDraft(natural)],
      }),
    );
  }
  assert.throws(() =>
    planMultiSites({ ...base, drafts: [{ ...base.drafts[0], start: '2000-01-01' }] }),
  );
});

test('first-year validation requires a complete year and bounded integer repetitions', () => {
  assert.equal(firstYearCutoff('2000-01-01'), '2001-01-01');
  assert.throws(() => firstYearCutoff('2000-02-29'));
  assert.throws(() => firstYearCutoff('2001-02-30'));
  for (const repeat of ['', '0', '-1', '1.5', '2147483648', 'Infinity'])
    assert.throws(() => planMultiSites({ ...base, repeat }));
  for (const end of ['2001-12-31', '2002-01-01'])
    assert.throws(() => planMultiSites({ ...base, drafts: [{ ...base.drafts[0], end }] }));
});

test('review requires natural classification and crop defaults; data step allows unfinished scheme', () => {
  const form = { ...base.form, landcover: '', subgrid: '' };
  assert.throws(() => planMultiSites({ ...base, form }));
  assert.equal(planMultiSites({ ...base, form, step: 3 })[0].mode, 'igbp');
  assert.throws(() => planMultiSites({ ...base, cropDefaults: {} }));
  const [farm] = planMultiSites({
    ...base,
    form: { ...base.form, subgrid: 'IGBP' },
    drafts: [siteDraft(crop)],
  });
  assert.equal(farm.mode, 'pft');
});

test('per-site spinup checks its own interval and repetition bounds', () => {
  for (const patch of [
    { spinup_end: '2001-01-01', spinup_repeat: '2' },
    { spinup_end: '2005-01-01', spinup_repeat: '2' },
    { spinup_end: '2002-01-01', spinup_repeat: '2147483648' },
    { spinup_end: '2002-01-01', spinup_repeat: '1.2' },
  ])
    assert.throws(() =>
      planMultiSites({ ...base, mode: 'per-site', drafts: [{ ...base.drafts[0], ...patch }] }),
    );
  const source = { ...natural, start: '2001-01-01T00:00:00', end: '2004-12-31T00:00:00' };
  assert.equal(
    planMultiSites({ ...base, sites: [source], drafts: [siteDraft(source)] })[0].form.start,
    '2001-01-01',
  );
});

test('shared irrigation remains enabled only for PFT station structures', () => {
  const physics = { ...base.physics, DEF_USE_IRRIGATION: '.true.' };
  for (const subgrid of ['IGBP', 'PC', 'PFT']) {
    const [forest, farm, city] = planMultiSites({
      ...base,
      physics,
      form: { ...base.form, subgrid },
    });
    assert.equal(city.fields.DEF_USE_IRRIGATION, '.false.');
    assert.equal(forest.fields.DEF_USE_IRRIGATION, subgrid === 'PFT' ? '.true.' : '.false.');
    assert.equal(farm.fields.DEF_USE_IRRIGATION, subgrid === 'PC' ? '.false.' : '.true.');
  }
  assert.equal(physics.DEF_USE_IRRIGATION, '.true.');
});
