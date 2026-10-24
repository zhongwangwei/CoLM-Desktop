import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  benchErrorText,
  benchLogText,
  buildOpenbenchConfig,
  comparableVariables,
  commonYears,
  comparisonDir,
  defaultGridRes,
  parseReferences,
  simulationKey,
  type BenchSimulation,
} from './compare';

const site: BenchSimulation = {
  label: 'AT-Neu (official)',
  info: {
    history_dir: '/p/AT-Neu/out/AT-Neu/history',
    prefix: 'AT-Neu_hist_',
    tim_res: 'Hour',
    syear: 2003,
    eyear: 2006,
    spatial: false,
    lon: 11.3,
    lat: 47.1,
    files: 48,
  },
};
const basin: BenchSimulation = {
  label: 'hb draft 2',
  info: {
    ...site.info,
    history_dir: '/p/hb dir/out/hb/history',
    prefix: 'hb_hist_',
    spatial: true,
    syear: 2002,
    eyear: 2005,
  },
};
const base = {
  name: 'versions vs flux',
  years: [2003, 2005] as [number, number],
  timRes: 'Day',
  variables: { Latent_Heat: 'OpenBench_LH_Daily', Sensible_Heat: '' } as Record<string, string>,
  simulations: [site, basin],
  projectDir: '/data/openbench/projects/versions_vs_flux',
};

it('builds a config with grid simulations pointing at history and sites as station lists', () => {
  const { config, sites } = buildOpenbenchConfig(base);
  assert.match(config, /name: "versions_vs_flux"/);
  assert.match(config, /years: \[2003, 2005\]/);
  assert.match(config, /variables: \[Latent_Heat\]/, 'variables without a reference are left out');
  assert.ok(
    config.includes(
      [
        '  AT_Neu_official:',
        '    model: CoLM2024',
        '    root_dir: "/data/openbench/projects/versions_vs_flux/sim"',
        '    data_type: stn',
        '    data_groupby: single',
        '    tim_res: Hour',
        // 绝对路径：OpenBench 的 check 与 run 对相对 fulllist 的解析不一致。
        '    fulllist: "/data/openbench/projects/versions_vs_flux/sim/AT_Neu_official.csv"',
      ].join('\n'),
    ),
    config,
  );
  assert.match(
    config,
    /hb_draft_2:[\s\S]*root_dir: "\/p\/hb dir\/out\/hb\/history"[\s\S]*data_type: grid/,
  );
  assert.deepEqual(sites, [
    {
      id: 'AT_Neu_official',
      station: 'AT-Neu',
      history_dir: '/p/AT-Neu/out/AT-Neu/history',
      prefix: 'AT-Neu_hist_',
    },
  ]);
  assert.deepEqual(commonYears([site, basin]), [2003, 2005]);
  assert.equal(commonYears([]), null);
});

it('station paths follow the platform separator of the comparison folder', () => {
  const { config } = buildOpenbenchConfig({
    ...base,
    simulations: [site],
    projectDir: 'C:\\Users\\me\\openbench\\projects\\w',
  });
  assert.ok(
    config.includes(
      `fulllist: ${JSON.stringify('C:\\Users\\me\\openbench\\projects\\w\\sim\\AT_Neu_official.csv')}`,
    ),
    config,
  );
});

it('refuses designs OpenBench would reject', () => {
  assert.throws(() => buildOpenbenchConfig({ ...base, years: [2006, 2003] }), /年份|years/);
  assert.throws(() => buildOpenbenchConfig({ ...base, variables: {} }), /变量|variable/);
  assert.throws(
    () => buildOpenbenchConfig({ ...base, simulations: [site, { ...site }] }),
    /重复|unique/,
  );
  assert.equal(simulationKey('2026 run'), 'sim_2026_run');
});

it('reads the reference list in either shape', () => {
  const rows = [
    { name: 'GLEAM', data_type: 'grid', tim_res: 'Month', variables: ['Evapotranspiration'] },
  ];
  assert.deepEqual(parseReferences(rows)[0]!.variables, ['Evapotranspiration']);
  assert.deepEqual(
    parseReferences({ datasets: [{ name: 'LH', variables: { Latent_Heat: {} } }] })[0]!.variables,
    ['Latent_Heat'],
  );
  assert.deepEqual(parseReferences(null), []);
});

it('comparison folders match the backend layout on both separators', () => {
  assert.equal(comparisonDir('/data/openbench', 'a b'), '/data/openbench/projects/a_b');
  assert.equal(comparisonDir('C:\\ob', 'x'), 'C:\\ob\\projects\\x');
});

it('lists every variable with its references, same-kind ones first', () => {
  const refs = [
    { name: 'GLEAM', data_type: 'grid', variables: ['Evapotranspiration', 'Latent_Heat'] },
    { name: 'FLUX', data_type: 'stn', variables: ['Latent_Heat', 'Sensible_Heat'] },
    { name: 'Any', variables: ['Albedo'] },
  ];
  assert.deepEqual(comparableVariables(refs, []), []);
  // 站点模拟也能拿格点产品比（OpenBench 在站点处取值）：不排除，只把同种的排前面。
  const station = comparableVariables(refs, [{ spatial: false }]);
  assert.deepEqual(
    station.map((v) => [v.variable, v.references.map((r) => r.name)]),
    [
      ['Albedo', ['Any']],
      ['Evapotranspiration', ['GLEAM']],
      ['Latent_Heat', ['FLUX', 'GLEAM']],
      ['Sensible_Heat', ['FLUX']],
    ],
  );
  const gridded = comparableVariables(refs, [{ spatial: true }], 'heat');
  assert.deepEqual(
    gridded.map((v) => [v.variable, v.references.map((r) => r.name)]),
    [
      ['Latent_Heat', ['GLEAM', 'FLUX']],
      ['Sensible_Heat', ['FLUX']],
    ],
  );
});

it('backend messages read in both languages', () => {
  assert.deepEqual(benchErrorText('Error: 已停止'), ['已停止', 'Stopped']);
  assert.equal(
    benchErrorText('check 失败（exit status: 1）：\nboom')[1],
    'check failed (exit status: 1):\nboom',
  );
  assert.equal(
    benchErrorText('报告有 52 MB，太大不在软件里显示；请用浏览器打开')[1],
    'The report is 52 MB, too large to show here; open it in a browser',
  );
  assert.deepEqual(benchErrorText('Traceback x'), ['Traceback x', 'Traceback x']);
  assert.equal(benchLogText('合并 AT_Neu 的 history'), 'Merging the history of AT_Neu');
  assert.equal(benchLogText('Resolved 55 packages'), 'Resolved 55 packages');
});

it('gridded simulations are clipped to their own extent and compared at a chosen resolution', () => {
  const gba = {
    label: 'GreaterBay 1km',
    info: {
      ...basin.info,
      extent: [111.0, 115.5, 21.5, 24.5] as [number, number, number, number],
      resolution: 0.008333,
    },
  };
  const pearl = {
    label: 'Pearl 0.1',
    info: {
      ...gba.info,
      extent: [104.004, 117.2, 21.5, 26.9] as [number, number, number, number],
      resolution: 0.1,
    },
  };
  assert.equal(defaultGridRes([gba.info]), 0.25);
  assert.equal(defaultGridRes([gba.info, { spatial: true, resolution: 0.5 }]), 0.5);
  assert.equal(defaultGridRes([site.info]), null);
  const { config } = buildOpenbenchConfig({
    ...base,
    simulations: [gba, pearl],
    gridRes: 0.25,
  });
  assert.match(config, /lat_range: \[21\.5, 26\.9\]/);
  assert.match(config, /lon_range: \[104, 117\.2\]/);
  assert.match(config, /grid_res: 0\.25/);
  // 只有单点模拟时不写范围。
  const stations = buildOpenbenchConfig({ ...base, simulations: [site] }).config;
  assert.doesNotMatch(stations, /lat_range|grid_res/);
});

it('builds one reference list per variable and per-simulation variable overrides', () => {
  const { config } = buildOpenbenchConfig({
    ...base,
    variables: {
      Latent_Heat: ['ERA5LAND_LowRes', 'OpenBench_LH_Daily'],
      Sensible_Heat: 'ERA5LAND_LowRes',
    },
    simulations: [
      {
        ...site,
        info: { ...site.info, resolution: 0.1, data_groupby: 'Month' },
        gridRes: 0.25,
        dataGroupby: 'Day',
        variables: {
          New_Module_Flux: { varname: 'f_newflux', varunit: 'W m-2' },
        },
      },
      basin,
    ],
    weight: 'mass',
    minYear: 0,
    timeAlignment: 'per_pair',
    numCores: 4,
    metrics: ['bias', 'KGE'],
    scores: ['Overall_Score'],
  });
  assert.ok(
    config.includes(
      'reference:\n  Latent_Heat:\n    - "ERA5LAND_LowRes"\n    - "OpenBench_LH_Daily"',
    ),
    config,
  );
  for (const line of [
    '  weight: mass',
    '  min_year_threshold: 0',
    '  time_alignment: per_pair',
    '  num_cores: 4',
  ])
    assert.ok(config.includes(line), config);
  assert.ok(config.includes('metrics: ["bias", "KGE"]'), config);
  assert.ok(config.includes('scores: ["Overall_Score"]'), config);
  assert.ok(
    config.includes(
      'variables:\n      "New_Module_Flux":\n        varname: "f_newflux"\n        varunit: "W m-2"',
    ),
    config,
  );
});

it('writes each gridded simulation native resolution and grouping', () => {
  const gba = {
    ...basin,
    info: { ...basin.info, resolution: 0.0083333, data_groupby: 'Day' },
  };
  const { config } = buildOpenbenchConfig({ ...base, simulations: [gba] });
  assert.match(config, /data_groupby: Day/);
  assert.match(config, /grid_res: 0\.0083333/);
});

it('passes the two climatology time scales through to OpenBench', () => {
  for (const scale of ['climatology-year', 'climatology-month']) {
    const { config } = buildOpenbenchConfig({ ...base, timRes: scale });
    assert.ok(config.includes(`  tim_res: ${scale}`), config);
  }
});

it('writes an explicitly cleared metric list and refuses clearing both', () => {
  const { config } = buildOpenbenchConfig({ ...base, metrics: [], scores: ['Overall_Score'] });
  assert.ok(config.includes('metrics: []'), config);
  assert.throws(
    () => buildOpenbenchConfig({ ...base, metrics: [], scores: [] }),
    /at least one metric or score/,
  );
});
