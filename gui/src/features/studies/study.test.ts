import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  calibrationWindows,
  candidateBudget,
  fitValidation,
  gridTargetDefaults,
  gridTargetSpec,
  networkSpec,
  networkDefaults,
  parameterSpec,
  studyPermissions,
  type ParameterContext,
} from './study';
const parameter: ParameterContext = {
  name: 'TEST',
  id: 'test',
  label_en: 'test',
  label_zh: 'test',
  scope: 'pft',
  scope_instance: { kind: 'pft-type', index: 2, scheme: 'PFT' },
  default: 1,
  min: 0,
  max: 10,
  min_inclusive: false,
  max_inclusive: true,
  scale: 'linear',
};
describe('study contracts', () => {
  it('does not equate missing validation with a passed gate', () => {
    assert.equal(fitValidation({ validation: null }), null);
    assert.equal(fitValidation({ validation: { passed: false } }), false);
    assert.equal(fitValidation({ validation: { passed: true } }), true);
  });
  it('retains exact parameter scope and refuses invalid scientific ranges', () => {
    assert.deepEqual(
      parameterSpec(parameter, '1', '3', 'log').scope_instance,
      parameter.scope_instance,
    );
    assert.throws(() => parameterSpec(parameter, '0', '3', 'linear'));
    assert.throws(() => parameterSpec(parameter, '', '3', 'linear'));
    assert.throws(() => parameterSpec(parameter, '1', '11', 'linear'));
  });
  it('sends only the study scope fields that the study spec accepts', () => {
    const catalogRow: ParameterContext = {
      ...parameter,
      scope_instance: {
        kind: 'case-scalar',
        scheme: null,
        index: null,
        type_name: null,
        process_file: null,
      },
    };
    assert.deepEqual(parameterSpec(catalogRow, '1', '3', 'linear').scope_instance, {
      kind: 'case-scalar',
    });
    assert.throws(
      () =>
        parameterSpec(
          { ...catalogRow, scope_instance: { kind: 'case-scalar', process_file: 'x.csv' } },
          '1',
          '3',
          'linear',
        ),
      /process-file/,
    );
  });
  it('rejects training/validation leakage', () => {
    assert.throws(() =>
      calibrationWindows(
        '2020-01-01T00:00',
        '2020-03-01T00:00',
        '2020-02-01T00:00',
        '2020-04-01T00:00',
      ),
    );
    assert.equal(
      calibrationWindows(
        '2020-01-01T00:00',
        '2020-03-01T00:00',
        '2020-03-01T00:00',
        '2020-04-01T00:00',
      ).to,
      1583020800,
    );
  });
  it('takes an empty validation window as none, but not half of one', () => {
    assert.deepEqual(calibrationWindows('2008-01-01T00:00', '2008-03-31T23:30', '', ''), {
      from: 1199145600,
      to: 1207006200,
      validation_from: null,
      validation_to: null,
    });
    assert.throws(
      () => calibrationWindows('2008-01-01T00:00', '2008-03-31T23:30', '2008-04-01T00:00', ''),
      /validation window/,
    );
    assert.throws(() => calibrationWindows('', '2008-03-31T23:30', '', ''), /calibration window/);
  });
  it('budgets baseline and every stage for every site', () => {
    assert.deepEqual(candidateBudget('tuning', '', 2, 0, 8, 4, 3), {
      candidates: 40,
      tasks: 123,
      stageRuns: 369,
    });
    assert.throws(() => candidateBudget('tuning', '', 2, 0, 100, 20, 3));
  });
  it('resumes only paused studies and exposes failure retry', () => {
    assert.equal(studyPermissions('Paused').resume, true);
    assert.equal(studyPermissions('Running').resume, false);
    assert.equal(studyPermissions('CompletedWithFailures').retry, true);
  });
  it('uses actual hybrid network serialization and process slot', () => {
    assert.deepEqual(
      networkSpec(
        'soil_stress',
        'beta_physics, root_saturation',
        'beta',
        '0.5',
        '2',
        'linear',
        true,
      ),
      {
        slot: 'soil_stress',
        features: ['beta_physics', 'root_saturation'],
        outputs: [{ name: 'beta', range: [0.5, 2], transform: 'sigmoid', relative: true }],
        hidden: [],
      },
    );
    assert.throws(() => networkSpec('soil_stress', 'beta_physics', 'other', '0.5', '2', '4', true));
    assert.throws(() =>
      networkSpec('pft', 'pftclass', 'DEF_LC_VMAX25', '10', '150', 'linear', false),
    );
  });
});

it('switching network slots starts with compatible complete defaults', () => {
  for (const slot of ['pft', 'land_class', 'soil_stress']) {
    const d = networkDefaults(slot);
    assert.doesNotThrow(() =>
      networkSpec(d.slot, d.features, d.output, d.lower, d.upper, 'linear', d.relative),
    );
  }
  assert.equal(networkDefaults('land_class').output, 'DEF_LC_VMAX25');
  assert.equal(networkDefaults('soil_stress').relative, false);
  assert.throws(() =>
    networkSpec('soil_stress', 'beta_physics', 'beta', '0', '2', 'linear', false),
  );
  assert.throws(() =>
    networkSpec('soil_stress', 'beta_physics', 'beta', '-1', '2', 'linear', true),
  );
});

describe('gridded study targets', () => {
  const windows = { from: 0, to: 86_400, validation_from: null, validation_to: null };
  it('turns each kind into a target with its own observation source', () => {
    const grid = gridTargetSpec(
      { ...gridTargetDefaults.grid, path: '/obs/gleam.nc', obsVariable: 'E', scale: '0.5' },
      0,
      windows,
      'kge',
      20,
    );
    assert.deepEqual(grid.source, {
      kind: 'grid',
      path: '/obs/gleam.nc',
      variable: 'E',
      scale: 0.5,
    });
    assert.equal(grid.key, 'f_lfevpa@grid1');
    const station = gridTargetSpec(
      {
        ...gridTargetDefaults.station,
        path: '/obs/flux.nc',
        lon: '123.5',
        lat: '44.6',
        utcOffset: '8',
      },
      1,
      windows,
      'nrmse',
      20,
    );
    assert.deepEqual(station.source, {
      kind: 'station',
      path: '/obs/flux.nc',
      lon: 123.5,
      lat: 44.6,
      utc_offset: 8,
    });
    const gauge = gridTargetSpec(
      { ...gridTargetDefaults.gauge, path: '/obs/q.csv', lon: '113.2', lat: '23.4' },
      2,
      windows,
      'nse',
      20,
    );
    assert.deepEqual(gauge.source, {
      kind: 'gauge',
      path: '/obs/q.csv',
      lon: 113.2,
      lat: 23.4,
      period: 'daily',
    });
  });
  it('lets one target use its own metric and the rest follow the study metric', () => {
    const flux = gridTargetSpec(
      { ...gridTargetDefaults.grid, path: '/obs/gleam.nc', obsVariable: 'E', scale: '1' },
      0,
      windows,
      'nrmse',
      20,
    );
    const discharge = gridTargetSpec(
      { ...gridTargetDefaults.gauge, path: '/obs/q.csv', lon: '113.2', lat: '23.4', metric: 'nse' },
      1,
      windows,
      'nrmse',
      20,
    );
    assert.equal(flux.metric, 'nrmse');
    assert.equal(discharge.metric, 'nse');
  });
  it('refuses incomplete targets before they reach the engine', () => {
    assert.throws(() => gridTargetSpec(gridTargetDefaults.grid, 0, windows, 'kge', 20));
    assert.throws(() =>
      gridTargetSpec({ ...gridTargetDefaults.grid, path: '/o.nc' }, 0, windows, 'kge', 20),
    );
    assert.throws(() =>
      gridTargetSpec(
        { ...gridTargetDefaults.station, path: '/o.nc', lon: '113', lat: '95' },
        0,
        windows,
        'kge',
        20,
      ),
    );
    assert.throws(() =>
      gridTargetSpec(
        { ...gridTargetDefaults.grid, path: '/o.nc', obsVariable: 'E', scale: '0' },
        0,
        windows,
        'kge',
        20,
      ),
    );
  });
});
