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
  configurationCaseValues,
  cropEnabled,
  selectChoice,
  type SchemaField,
  preparationSiteModes,
  forcingSaveConflict,
  prepDateValid,
  prepDateKey,
  preparedSiteNameValid,
  preparedSiteDestination,
  preparedDatasetDestinations,
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
  it('admits process switches while model-field filtering owns the parameter boundary', () => {
    assert.equal(isCaseField(field('DEF_USE_BGC', '生态与生地化')), true);
    assert.equal(isCaseField(field('DEF_TUNING_CSOILC', '生态与生地化')), true);
    assert.equal(isCaseField(field('DEF_file_SoilInit', '初始场')), true);
    assert.equal(isCaseField(field('DEF_simulation_time%spinup_year', '时间与预热')), true);
  });
  it('rejects empty coordinates rather than guessing zero', () => {
    assert.throws(() => requiredNumber('', 'longitude'));
    assert.equal(requiredNumber('0', 'longitude'), 0);
  });
  it('rejects names escaping the chosen parent', () => {
    for (const name of [
      '../other',
      ' .. ',
      ' station ',
      '.',
      'x:y',
      'x\n',
      '--flag',
      '站'.repeat(86),
    ])
      assert.throws(() => caseDirectory('/data', name), name);
    assert.throws(() => caseDirectory('/data', '--flag'), /--/);
    assert.equal(caseDirectory('/data/', 'station'), '/data/station');
    assert.equal(caseDirectory('/data///', 'station'), '/data/station');
    assert.equal(caseDirectory('/', 'station'), '/station');
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
  it('names prepared outputs with the same two layouts the scanner recognizes', () => {
    // 无约定后缀：按 PLUMBER2 追加；末尾斜杠归一。
    assert.equal(preparedDestination('/outputs', '/raw/station.nc'), '/outputs/station_Met.nc');
    assert.equal(preparedDestination('/outputs/', '/raw/met.nc'), '/outputs/met_Met.nc');
    assert.equal(
      preparedDestination('/outputs', '/raw/station.nc', 'Flux'),
      '/outputs/station_Flux.nc',
    );
    // PLUMBER2：同布局换目标后缀，只剥一层完整后缀。
    assert.equal(preparedDestination('/outputs', '/raw/A_site.nc'), '/outputs/A_Met.nc');
    assert.equal(preparedDestination('/outputs', '/raw/A_Flux.nc'), '/outputs/A_Met.nc');
    assert.equal(preparedDestination('/outputs', '/raw/A_site.nc', 'site'), '/outputs/A_site.nc');
    assert.equal(preparedDestination('/outputs', '/raw/A_Met.nc'), '/outputs/A_Met.nc');
    assert.equal(preparedDestination('/outputs', '/raw/A_Met.nc', 'Flux'), '/outputs/A_Flux.nc');
    // 合法站名 A_Met：扫描器会找 A_Met_Met.nc，不能把它缩成 A_Met.nc。
    assert.equal(preparedDestination('/outputs', '/raw/A_Met_site.nc'), '/outputs/A_Met_Met.nc');
  });
  it('mirrors the Urban-PLUMBER naming convention instead of appending _Met', () => {
    assert.equal(
      preparedDestination('/outputs', '/raw/A_site_v1.nc'),
      '/outputs/A_metforcing_v1.nc',
    );
    assert.equal(
      preparedDestination('/outputs', '/raw/A_metforcing_v1.nc'),
      '/outputs/A_metforcing_v1.nc',
      '已是 Urban 强迫场名时不能拼成 A_metforcing_v1_Met.nc',
    );
    assert.equal(
      preparedDestination('/outputs', '/raw/A_clean_observations_v1.nc'),
      '/outputs/A_metforcing_v1.nc',
    );
    assert.equal(
      preparedDestination('/outputs', '/raw/A_metforcing_v1.nc', 'Flux'),
      '/outputs/A_clean_observations_v1.nc',
    );
    assert.equal(
      preparedDestination('/outputs', '/raw/A_site_v1.nc', 'site'),
      '/outputs/A_site_v1.nc',
    );
    // 完全无后缀时仍按默认（PLUMBER2）布局。
    assert.equal(preparedDestination('/outputs', '/raw/A.nc'), '/outputs/A_Met.nc');
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
it('select choices match Fortran strings case-insensitively and keep unknown values visible', () => {
  const options = ['DAILY', 'HOURLY', 'MONTHLY', 'TIMESTEP', 'YEARLY', 'none'];
  assert.deepEqual(selectChoice('monthly', options), { value: 'MONTHLY', options });
  assert.deepEqual(selectChoice('NONE', options), { value: 'none', options });
  assert.deepEqual(selectChoice('weekly', options), {
    value: 'weekly',
    options: [...options, 'weekly'],
  });
  assert.deepEqual(selectChoice('', options), { value: '', options });
});

describe('configuration values sent to model_configuration', () => {
  const schema = [
    { name: 'DEF_USE_BGC', group: 'nl_colm' },
    { name: 'DEF_forcing%dataset', group: 'nl_colm_forcing' },
    { name: 'DEF_forcing%HEIGHT_V', group: 'nl_colm_forcing' },
    { name: 'DEF_simulation_time%timestep', group: 'nl_colm_simulation_time' },
  ];
  it('drops forcing.nml fields so the stale dataset whitelist cannot lock step 4', () => {
    assert.deepEqual(
      configurationCaseValues(
        {
          DEF_USE_BGC: '.true.',
          'DEF_forcing%dataset': 'CMFD',
          'DEF_forcing%HEIGHT_V': '6.0',
          'def_simulation_time%timestep': '1800',
        },
        schema,
      ),
      { DEF_USE_BGC: '.true.', 'DEF_simulation_time%timestep': '1800' },
    );
  });
  it('keeps keys outside the schema (temporary scheme fields) as-is', () => {
    assert.deepEqual(configurationCaseValues({ DEF_NOT_A_FIELD: '1' }, schema), {
      DEF_NOT_A_FIELD: '1',
    });
  });
});

it('CROP cases only offer site modes the crop audit accepts', () => {
  assert.deepEqual(preparationSiteModes(false), [
    'igbp',
    'usgs',
    'pft',
    'pc',
    'urban-igbp',
    'urban-usgs',
    'urban-pft',
    'urban-pc',
  ]);
  assert.deepEqual(preparationSiteModes(true), ['pft', 'pc', 'urban-pft', 'urban-pc']);
});

it('forcing path and forcing-field edits in one save are reported together', () => {
  const fields = [
    { path: 'DEF_dir_output', value: '/out' },
    { path: 'DEF_forcing%dataset', value: 'GSWP3' },
  ];
  assert.equal(forcingSaveConflict(false, fields), false);
  assert.equal(forcingSaveConflict(true, [{ path: 'DEF_dir_output' }]), false);
  assert.equal(forcingSaveConflict(true, fields), true);
});

describe('preparation validation matches the CLI', () => {
  it('accepts only real calendar dates in the two supported forms', () => {
    for (const value of ['2020-01-31', '20200131', '2000-02-29', '2024-12-31'])
      assert.equal(prepDateValid(value), true, value);
    for (const value of ['', '2020-13-01', '2020-02-30', '2021-02-29', '2020/01/01', '20-1-1'])
      assert.equal(prepDateValid(value), false, value);
    assert.equal(prepDateKey('2020-01-31'), prepDateKey('20200131'));
    assert.ok(prepDateKey('2020-01-31') < prepDateKey('2020-02-01'));
  });

  it('requires scanner-recognized site file suffixes and fills a destination name', () => {
    assert.equal(preparedSiteNameValid('/data/Sitedata/CN-Cng_site.nc'), true);
    assert.equal(preparedSiteNameValid('/data/Sitedata/CN-Cng_site_v1.nc'), true);
    assert.equal(preparedSiteNameValid('/data/Sitedata/CN-Cng.nc'), false);
    assert.equal(preparedSiteNameValid('/data/Sitedata/site.nc'), false);
    // 选目录后沿用当前词干，避免扫描器认不出站点文件（D-06）。
    assert.equal(
      preparedSiteDestination('/data/Sitedata/', '/in/CN-Cng_site.nc'),
      '/data/Sitedata/CN-Cng_site.nc',
    );
    assert.equal(preparedSiteDestination('/data/Sitedata', ''), '/data/Sitedata/site_site.nc');
    assert.equal(
      preparedSiteDestination('/data/Sitedata', '/old/met_forcing.csv'),
      '/data/Sitedata/met_forcing_site.nc',
    );
  });

  it('localizes required-number errors when the caller supplies a translator', () => {
    assert.throws(() => requiredNumber('', '经度', (zh) => zh), /请输入有效数字/);
    assert.throws(() => requiredNumber('abc', 'Longitude', (_zh, en) => en), /enter a number/);
    // 不传 t 时回退到 bilingual（无 document 时中文），与页面外的校验器行为一致。
    assert.throws(() => requiredNumber('', '经度'), /请输入有效数字|enter a number/);
  });
});

it('maps prepared products into the dataset Sitedata/Forcing layout', () => {
  assert.deepEqual(
    preparedDatasetDestinations(
      { site: '/out/CN-Cng_site.nc', met: '/out/CN-Cng_Met.nc' },
      '/data/PLUMBER2s/',
    ),
    {
      site: '/data/PLUMBER2s/Sitedata/CN-Cng_site.nc',
      met: '/data/PLUMBER2s/Forcing/CN-Cng_Met.nc',
      stemsMatch: true,
    },
  );
  const urban = preparedDatasetDestinations(
    { site: '/out/CN-Cng_site_v1.nc', met: '/out/CN-Cng_metforcing_v1.nc' },
    '/data/urban',
  );
  assert.equal(urban.site, '/data/urban/Sitedata/CN-Cng_site.nc');
  assert.equal(urban.met, '/data/urban/Forcing/CN-Cng_Met.nc');
  assert.equal(urban.stemsMatch, true);
  // 两个产物词干不同：安装后扫描仍能各自识别，但配对不到一起，页面要提示。
  assert.equal(
    preparedDatasetDestinations({ site: '/out/A_site.nc', met: '/out/B_Met.nc' }, '/data')
      .stemsMatch,
    false,
  );
  assert.deepEqual(preparedDatasetDestinations({}, '/data'), { stemsMatch: true });
  assert.deepEqual(
    preparedDatasetDestinations({ site: '/out/CN-Cng_site.nc', met: '/out/CN-Cng.nc' }, '/data'),
    {
      site: '/data/Sitedata/CN-Cng_site.nc',
      met: '/data/Forcing/CN-Cng_Met.nc',
      stemsMatch: true,
    },
  );
});
