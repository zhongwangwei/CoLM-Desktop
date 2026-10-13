import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  caseDirectory,
  requiredNumber,
  isCaseField,
  versionMode,
  displayLiteral,
  siteVersionMismatch,
  preparedDestination,
  effectiveModelFields,
  cropEnabled,
  type SchemaField,
} from './case-model';
const field = (name: string, section: string): SchemaField => ({
  name,
  section,
  kind: 'Logical',
  default: '.false.',
  derived: false,
  group: 'nl_colm',
  values: [],
});
describe('case data boundary', () => {
  it('keeps parameters and process switches in model versions', () => {
    assert.equal(isCaseField(field('DEF_USE_BGC', '生态与生地化')), false);
    assert.equal(isCaseField(field('DEF_TUNING_CSOILC', '生态与生地化')), false);
    assert.equal(isCaseField(field('DEF_file_SoilInit', '初始场')), true);
    assert.equal(isCaseField(field('DEF_simulation_time%spinup_year', '时间与预热')), true);
  });
  it('rejects empty coordinates rather than guessing zero', () => {
    assert.throws(() => requiredNumber('', 'longitude'));
    assert.equal(requiredNumber('0', 'longitude'), 0);
  });
  it('rejects names escaping the chosen parent', () => {
    assert.throws(() => caseDirectory('/data', '../other'));
    assert.equal(caseDirectory('/data/', 'station'), '/data/station');
  });
  it('uses runtime overlay names for model structure', () => {
    assert.equal(versionMode({ DEF_USE_PFT: '.true.' }), 'pft');
    assert.equal(versionMode({ DEF_USE_PC: 'true' }), 'pc');
  });
  it('does not silently change model physics to fit a station', () => {
    assert.match(siteVersionMismatch({ crop: true, urban: false }, {}) ?? '', /CROP/);
    assert.equal(
      siteVersionMismatch({ crop: true, urban: false }, { DEF_TUNING_CROP_PLANTING_DAY: '120' }),
      null,
    );
  });
  it('recognizes crop from the actual scoped parameter contract', () => {
    const fields = effectiveModelFields({
      physics: { DEF_USE_BGC: '.true.' },
      parameters: [
        {
          raw_key: 'DEF_TUNING_CROP_PLANTING_DAY',
          value: '120',
          scope_instance: { kind: 'case-scalar' },
        },
      ],
    });
    assert.equal(cropEnabled(fields), true);
    assert.equal(fields.DEF_USE_BGC, '.true.');
    assert.equal(cropEnabled({ DEF_USE_CROP: '.true.' }), false);
  });
  it('turns a suggested directory into a distinct NetCDF filename', () => {
    assert.equal(preparedDestination('/outputs', '/raw/station.nc'), '/outputs/station_colm.nc');
  });
  it('displays quoted Fortran character literals as editable values', () => {
    assert.equal(displayLiteral("'MONTHLY'"), 'MONTHLY');
    assert.equal(displayLiteral('.true.'), '.true.');
  });
  it('retains non-model forcing, diagnostics, surface and parallel settings', () => {
    for (const [name, section] of [
      ['DEF_Forcing_Interp_Method', '强迫场'],
      ['DEF_USE_RangeCheck', '调试与诊断'],
      ['DEF_LC_YEAR', '地表数据'],
      ['DEF_nx_blocks', '网格与并行'],
    ])
      assert.equal(isCaseField(field(name, section)), true);
    assert.equal(isCaseField(field('DEF_USE_USGS', '地表数据')), false);
    assert.equal(isCaseField({ ...field('DEF_HIST_FREQ', '输出与重启'), derived: true }), false);
  });
});
