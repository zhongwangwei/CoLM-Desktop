import test from 'node:test';
import assert from 'node:assert/strict';
import {
  chartVariables,
  isGridCatalog,
  readErrorText,
  variableOption,
  variableTitle,
} from './variable-labels';
import { overviewVariable } from './result-overview';

const grid = (name: string, extra: Record<string, unknown> = {}) => ({
  name,
  kind: 'grid',
  dimensions: [],
  ...extra,
});

test('result variables use the simulation page names, else the file long_name, else the raw name', () => {
  assert.equal(variableTitle('f_fsena', false), '感热通量');
  assert.equal(variableTitle('f_fsena', true), 'Sensible heat flux');
  assert.equal(variableTitle('f_xy_odd', false, 'an odd quantity'), 'an odd quantity');
  assert.equal(variableTitle('f_xy_odd', false), 'f_xy_odd');
  assert.equal(
    variableOption({ ...grid('f_fsena'), units: 'W/m2' }, false),
    '感热通量（f_fsena） · W/m2',
  );
  assert.equal(variableOption(grid('f_xy_odd'), true), 'f_xy_odd');
});

test('only time-varying series and grids are listed, core variables first', () => {
  const listed = chartVariables([
    grid('f_zz_extra'),
    { name: 'lat', kind: 'static', dimensions: [] },
    { name: 'time', kind: 'series', dimensions: [] },
    { name: 'f_t_soisno', kind: 'profile', dimensions: [] },
    grid('f_fevpa'),
    grid('f_fsena'),
    grid('f_aa_extra'),
  ]).map((item) => item.name);
  assert.deepEqual(listed, ['f_fsena', 'f_fevpa', 'f_aa_extra', 'f_zz_extra']);
  assert.ok(isGridCatalog([grid('f_fsena')]));
  assert.ok(!isGridCatalog([{ name: 'f_fsena', kind: 'series', dimensions: [] }]));
  assert.equal(overviewVariable([grid('f_fsena'), grid('f_lfevpa')]), 'f_lfevpa');
});

test('an old server engine that cannot read grids is explained instead of quoted', () => {
  assert.match(
    readErrorText(
      'Error: f_fsena has 39424 values for 56 time steps; it is not a (time, patch) series',
      false,
    ),
    /引擎版本较旧/,
  );
  assert.match(readErrorText('error: unknown command history-map', true), /too old/);
  assert.equal(readErrorText('Error: no history at /x', false), 'Error: no history at /x');
});
