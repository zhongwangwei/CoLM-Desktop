import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  benchErrorText,
  benchLogText,
  buildOpenbenchConfig,
  comparableVariables,
  commonYears,
  comparisonDir,
  comparisonName,
  configInvalidation,
  configSaveState,
  defaultGridRes,
  designFingerprint,
  evaluationSummary,
  formatExtent,
  jobContacted,
  jobEnded,
  jobUncertain,
  layerVariableProblems,
  parseExtent,
  parseReferences,
  parseRemoteJob,
  remoteJobTerminal,
  simulationFormForPick,
  simulationInfoForSave,
  simulationKey,
  validateSimulation,
  type BenchSimulation,
  type ComparisonDesign,
  type EvaluationDesign,
  type HistoryInfo,
  type RemoteJobIdentity,
  type SimulationFormValues,
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
      // 编辑模拟时确认的坐标也要进站点清单：合并脚本用它，而不是再读文件里的旧坐标。
      lon: 11.3,
      lat: 47.1,
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

const probedGrid: HistoryInfo = {
  history_dir: '/p/hb dir/out/hb/history',
  prefix: 'hb_hist_',
  tim_res: 'Day',
  syear: 2002,
  eyear: 2005,
  spatial: true,
  lon: null,
  lat: null,
  files: 48,
  extent: [111, 115.5, 21.5, 24.5],
  resolution: 0.008333,
  data_groupby: 'Month',
};
const probedStation: HistoryInfo = {
  history_dir: '/p/AT-Neu/out/AT-Neu/history',
  prefix: 'AT-Neu_hist_',
  tim_res: 'Hour',
  syear: 2003,
  eyear: 2006,
  spatial: false,
  lon: 11.3,
  lat: 47.1,
  files: 48,
  extent: null,
  resolution: null,
  data_groupby: 'Month',
};
const simForm = (patch: Partial<SimulationFormValues> = {}): SimulationFormValues => ({
  dir: '',
  dataType: 'auto',
  prefix: '',
  timRes: 'Day',
  from: '',
  to: '',
  dataGroupby: '',
  gridRes: '',
  lon: '',
  lat: '',
  extent: '',
  variables: [],
  ...patch,
});

it('says which field blocks saving a simulation instead of only throwing', () => {
  // GUI-11：空目录、未确认类型、缺起止年份都要能定位到字段，页面据此提示并聚焦。
  assert.equal(validateSimulation(null, simForm())?.field, 'dir');
  const untyped = validateSimulation(null, simForm({ dir: '/d' }));
  assert.equal(untyped?.field, 'dataType');
  assert.match(untyped!.message[1], /Inspect the folder or choose grid\/station first/);
  assert.match(untyped!.message[0], /读取目录概况/);
  const noYears = validateSimulation(null, simForm({ dir: '/d', dataType: 'grid' }));
  assert.equal(noYears?.field, 'from');
  assert.match(noYears!.message[1], /start and end years/);
  assert.equal(
    validateSimulation(null, simForm({ dir: '/d', dataType: 'grid', from: '2003' }))?.field,
    'from',
    'only one year is not enough',
  );
  // 读到了概况时年份可以留空沿用；其余非法输入逐项定位。
  assert.equal(validateSimulation(probedGrid, simForm({ dir: '/d' })), null);
  assert.equal(validateSimulation(probedStation, simForm({ dir: '/d' })), null);
  assert.equal(
    validateSimulation(probedGrid, simForm({ dir: '/d', from: '2004', to: '2003' }))?.field,
    'from',
  );
  assert.equal(
    validateSimulation(probedGrid, simForm({ dir: '/d', gridRes: '-1' }))?.field,
    'gridRes',
  );
  assert.equal(
    validateSimulation(probedGrid, simForm({ dir: '/d', extent: '1,2,3' }))?.field,
    'extent',
  );
  assert.equal(
    validateSimulation(probedGrid, simForm({ dir: '/d', extent: '115,111,24,21' }))?.field,
    'extent',
    'west/east and south/north must run in order',
  );
  assert.equal(validateSimulation(probedStation, simForm({ dir: '/d', lon: '999' }))?.field, 'lon');
  assert.equal(validateSimulation(probedStation, simForm({ dir: '/d', lat: '91' }))?.field, 'lat');
  assert.equal(
    validateSimulation(
      probedStation,
      simForm({ dir: '/d', variables: [{ variable: 'X', varname: '', unit: '' }] }),
    )?.field,
    'variables',
  );
});

it('saves the whole inspected object and only overrides explicitly edited fields', () => {
  // GUI-10：读取→保存后 extent/lon/lat/files/history_dir 都还在，不能被表单重新拼出来的 info 顶掉。
  const info = simulationInfoForSave(
    probedGrid,
    simForm({
      dir: probedGrid.history_dir,
      dataType: 'grid',
      prefix: 'edited_hist_',
      from: '2004',
      to: '2006',
      gridRes: '0.05',
      extent: '110,116,21,25',
    }),
  );
  assert.deepEqual(info, {
    ...probedGrid,
    prefix: 'edited_hist_',
    syear: 2004,
    eyear: 2006,
    resolution: 0.05,
    extent: [110, 116, 21, 25],
  });
  // 留空的字段沿用探测值（表单"空白=自动"的语义）；打开表单时时间分辨率会预填探测值。
  assert.deepEqual(
    simulationInfoForSave(
      probedStation,
      simForm({ dir: probedStation.history_dir, dataType: 'stn', timRes: 'Hour' }),
    ),
    probedStation,
  );
  // 手工登记（没有探测结果）：只写表单里明确给出的元数据，files 从 0 起而不是编一个。
  const manual = simulationInfoForSave(
    null,
    simForm({
      dir: '/p/AT-Neu/out/AT-Neu/history',
      dataType: 'stn',
      timRes: 'Hour',
      from: '2003',
      to: '2004',
      lon: '11.3',
      lat: '47.1',
    }),
  );
  assert.equal(manual.history_dir, '/p/AT-Neu/out/AT-Neu/history');
  assert.equal(manual.spatial, false);
  assert.equal(manual.files, 0);
  assert.equal(manual.extent, null);
  assert.equal(manual.data_groupby, null);
  assert.equal(manual.resolution, null);
  assert.equal(manual.lon, 11.3);
  assert.equal(manual.lat, 47.1);
});

it('switching a saved simulation between grid and station really changes its type', () => {
  // GUI-09：已有分支原来展开旧 info 时漏了 spatial，改类型不生效。
  const station = simulationInfoForSave(
    probedGrid,
    simForm({ dir: probedGrid.history_dir, dataType: 'stn' }),
  );
  assert.equal(station.spatial, false);
  assert.equal(
    station.files,
    probedGrid.files,
    'the probed file count is not lost with the type change',
  );
  assert.equal(station.extent, null, 'a station has no grid region');
  assert.equal(
    simulationFormForPick({ dir: probedGrid.history_dir, label: 'hb', info: station }).dataType,
    'stn',
  );

  const grid = simulationInfoForSave(
    probedStation,
    simForm({
      dir: probedStation.history_dir,
      dataType: 'grid',
      extent: '110,116,21,25',
      gridRes: '0.1',
    }),
  );
  assert.equal(grid.spatial, true);
  assert.deepEqual(grid.extent, [110, 116, 21, 25]);
  assert.equal(grid.resolution, 0.1);
  assert.equal(
    simulationFormForPick({ dir: probedStation.history_dir, label: 'neu', info: grid }).dataType,
    'grid',
  );
});

it('editing form values round-trips the saved simulation including extent and coordinates', () => {
  const form = simulationFormForPick({ dir: '/p/hb/history', label: 'hb', info: probedGrid });
  assert.equal(form.dataType, 'grid');
  assert.equal(form.extent, '111,115.5,21.5,24.5');
  assert.equal(form.gridRes, '0.008333');
  assert.equal(form.dataGroupby, 'Month');
  assert.equal(form.from, '2002');
  const station = simulationFormForPick({ dir: '/p/neu', label: 'neu', info: probedStation });
  assert.equal(station.dataType, 'stn');
  assert.equal(station.lon, '11.3');
  assert.equal(station.lat, '47.1');
  // 手动覆盖的分组/分辨率优先于 info 里的。
  const overridden = simulationFormForPick({
    dir: '/p/hb/history',
    label: 'hb',
    info: probedGrid,
    dataGroupby: 'Day',
    gridRes: 0.25,
  });
  assert.equal(overridden.dataGroupby, 'Day');
  assert.equal(overridden.gridRes, '0.25');
  // 还没读到概况时表单是自动类型，保存会要求先读数或明确选类型。
  assert.equal(simulationFormForPick({ dir: '/d', label: 'd', info: null }).dataType, 'auto');
});

it('opening a saved simulation and saving it again changes nothing', () => {
  // GUI-10 验收：读取→保存→生成，探测到的范围/经纬度/文件数等元数据原样保留。
  const roundTrip = (info: HistoryInfo) =>
    simulationInfoForSave(info, {
      ...simForm(),
      ...simulationFormForPick({ dir: info.history_dir, label: 'x', info }),
    });
  assert.deepEqual(roundTrip(probedGrid), probedGrid);
  assert.deepEqual(roundTrip(probedStation), probedStation);
});

it('parses and formats a region in the order OpenBench expects', () => {
  assert.deepEqual(parseExtent('111, 115.5, 21.5, 24.5'), [111, 115.5, 21.5, 24.5]);
  assert.deepEqual(parseExtent('111，115.5，21.5，24.5'), [111, 115.5, 21.5, 24.5]);
  assert.equal(parseExtent('111,115.5,21.5'), null);
  assert.equal(parseExtent('111,115.5,24.5,21.5'), null);
  assert.equal(parseExtent('111,115.5,21.5,95'), null);
  assert.equal(formatExtent([111.0000001, 115.5, 21.5, 24.5]), '111,115.5,21.5,24.5');
  assert.equal(formatExtent(null), '');
});

it('writes the target comparison resolution even when no region could be inspected', () => {
  // GUI-10：没有 extent 时原来连显式 grid_res 都不输出；现在只省区域范围并说明原因。
  const noRegion = {
    label: 'GreaterBay 1km',
    info: { ...basin.info, extent: null, resolution: 0.008333 },
  };
  const built = buildOpenbenchConfig({ ...base, simulations: [noRegion], gridRes: 0.1 });
  assert.match(built.config, / {2}grid_res: 0\.1/);
  assert.doesNotMatch(built.config, /lat_range|lon_range/);
  assert.deepEqual(built.missingExtent, ['GreaterBay 1km']);
  const known = {
    ...noRegion,
    info: {
      ...noRegion.info,
      extent: [111, 115.5, 21.5, 24.5] as [number, number, number, number],
    },
  };
  const complete = buildOpenbenchConfig({ ...base, simulations: [known], gridRes: 0.1 });
  assert.match(complete.config, /lat_range: \[21\.5, 24\.5\]/);
  assert.deepEqual(complete.missingExtent, []);
  // 只有单点模拟时不写网格分辨率（那是格点比较的设置）。
  const stations = buildOpenbenchConfig({ ...base, simulations: [site], gridRes: 0.25 });
  assert.doesNotMatch(stations.config, /grid_res/);
});

it('writes variables and per-simulation overrides in a stable order', () => {
  const forward = buildOpenbenchConfig({
    ...base,
    variables: { Latent_Heat: 'L', Sensible_Heat: 'S' },
    simulations: [
      {
        ...basin,
        variables: {
          Zeta: { varname: 'z', varunit: '1' },
          Alpha: { varname: 'a', varunit: '1' },
        },
      },
    ],
  }).config;
  const backward = buildOpenbenchConfig({
    ...base,
    variables: { Sensible_Heat: 'S', Latent_Heat: 'L' },
    simulations: [
      {
        ...basin,
        variables: {
          Alpha: { varname: 'a', varunit: '1' },
          Zeta: { varname: 'z', varunit: '1' },
        },
      },
    ],
  }).config;
  assert.equal(forward, backward, 'selection order does not change the configuration text');
  assert.ok(forward.indexOf('"Alpha"') < forward.indexOf('"Zeta"'), forward);
  assert.ok(
    forward.indexOf('reference:\n  Latent_Heat') < forward.indexOf('  Sensible_Heat'),
    forward,
  );
});

const machine = { host: '', openbenchRoot: '/ob', refRoot: '/refs' };
const fingerprint = (
  patch: Partial<ComparisonDesign> = {},
  machinePatch: Partial<typeof machine> = {},
) => designFingerprint({ machine: { ...machine, ...machinePatch }, ...base, ...patch });

it('fingerprints capture every input that changes the generated configuration', () => {
  const original = fingerprint();
  const changes: [string, Partial<ComparisonDesign>][] = [
    ['project name', { name: 'another comparison' }],
    ['years', { years: [2003, 2006] }],
    ['time scale', { timRes: 'Month' }],
    ['reference selection', { variables: { Latent_Heat: 'ERA5LAND_LowRes' } }],
    ['target resolution', { gridRes: 0.5 }],
    ['weight', { weight: 'mass' }],
    ['min years', { minYear: 2 }],
    ['time alignment', { timeAlignment: 'per_pair' }],
    ['cores', { numCores: 4 }],
    ['metrics', { metrics: ['bias'] }],
    ['scores', { scores: ['Overall_Score', 'nBiasScore'] }],
    [
      'simulation prefix (same folder and display name)',
      { simulations: [{ ...site, info: { ...site.info, prefix: 'AT-Neu2_hist_' } }, basin] },
    ],
    [
      'simulation type',
      { simulations: [{ ...site, info: { ...site.info, spatial: true } }, basin] },
    ],
    [
      'simulation time resolution',
      { simulations: [site, { ...basin, info: { ...basin.info, tim_res: 'Day' } }] },
    ],
    [
      'simulation years',
      { simulations: [site, { ...basin, info: { ...basin.info, syear: 2001 } }] },
    ],
    [
      'file grouping',
      { simulations: [site, { ...basin, info: { ...basin.info, data_groupby: 'Day' } }] },
    ],
    [
      'region',
      { simulations: [site, { ...basin, info: { ...basin.info, extent: [110, 116, 20, 25] } }] },
    ],
    [
      'native resolution',
      { simulations: [site, { ...basin, info: { ...basin.info, resolution: 0.05 } }] },
    ],
    [
      'file count (changes the inferred grouping)',
      { simulations: [site, { ...basin, info: { ...basin.info, files: 1 } }] },
    ],
    [
      'per-simulation variable mapping',
      {
        simulations: [
          site,
          { ...basin, variables: { Latent_Heat: { varname: 'lh', varunit: 'W m-2' } } },
        ],
      },
    ],
    [
      'per-simulation group/resolution override',
      { simulations: [site, { ...basin, dataGroupby: 'Day', gridRes: 0.25 }] },
    ],
  ];
  for (const [what, patch] of changes)
    assert.notEqual(fingerprint(patch), original, `a changed ${what} must change the fingerprint`);
  // 机器/OpenBench/参考数据路径也是这次评估的身份。
  for (const machinePatch of [
    { host: '7920land' },
    { openbenchRoot: '/other/ob' },
    { refRoot: '/other/refs' },
  ])
    assert.notEqual(fingerprint({}, machinePatch), original);
  // 同一份设计不管对象键顺序都得到同一个指纹。
  assert.equal(
    fingerprint({ variables: { Sensible_Heat: '', Latent_Heat: 'OpenBench_LH_Daily' } }),
    fingerprint(),
  );
  assert.equal(fingerprint(), original);
});

it('decides whether a generated configuration is still valid', () => {
  const generated = { fingerprint: 'aaaa', config: 'project:\n  name: x\n' };
  assert.equal(
    configInvalidation({ currentFingerprint: 'aaaa', generated, yaml: generated.config }),
    'none',
  );
  assert.equal(
    configInvalidation({ currentFingerprint: 'bbbb', generated: null, yaml: 'x' }),
    'none',
  );
  assert.equal(configInvalidation({ currentFingerprint: 'bbbb', generated, yaml: '' }), 'none');
  // 没手改过的自动配置随设置失效，直接作废。
  assert.equal(
    configInvalidation({ currentFingerprint: 'bbbb', generated, yaml: generated.config }),
    'clear',
  );
  // 手改过的必须保留：页面只标记过期并给重新生成入口，不静默覆盖。
  assert.equal(
    configInvalidation({
      currentFingerprint: 'bbbb',
      generated,
      yaml: `${generated.config}# mine`,
    }),
    'keep-edited',
  );
});

it('persists a remote job identity through a failed query and keeps it in history after a terminal', () => {
  // GUI-07：一次查询失败只把记录标成"待核对"，不能清掉；确认终态后活动记录删除、历史保留身份。
  const stored: RemoteJobIdentity = {
    job: 'bench-run-84692-1',
    kind: 'run',
    name: 'qfo_era5',
    host: '7920ts',
    submittedAt: '2026-10-27T20:00:00.000Z',
    lastState: 'running',
    lastConfirmedAt: '2026-10-27T20:01:00.000Z',
  };
  // 往返 localStorage 的 JSON 不丢字段（重开应用据此恢复核对）。
  const parsed = parseRemoteJob(JSON.stringify(stored))!;
  for (const key of [
    'job',
    'kind',
    'name',
    'host',
    'submittedAt',
    'lastState',
    'lastConfirmedAt',
  ] as const)
    assert.equal(parsed[key], stored[key], key);
  assert.equal(parsed.uncertain, false);
  // 通信失败：身份与全部上下文都在，只标记待核对与原因。
  const lost = jobUncertain(stored, 'ssh: connect to host 7920ts port 22: Connection timed out');
  assert.equal(lost.job, stored.job);
  assert.equal(lost.name, 'qfo_era5');
  assert.equal(lost.host, '7920ts');
  assert.equal(lost.uncertain, true);
  assert.match(lost.lastError!, /Connection timed out/);
  // 一次成功的查询清掉"不确定"并更新最后确认状态。
  const contacted = jobContacted(lost, 'running', '2026-10-27T20:02:00.000Z');
  assert.equal(contacted.uncertain, false);
  assert.equal(contacted.lastState, 'running');
  assert.equal(contacted.lastConfirmedAt, '2026-10-27T20:02:00.000Z');
  // 终态写历史：保留作业身份，写上结果与时间（重开应用仍能看到上次作业）。
  const ended = jobEnded(contacted, 'succeeded', '2026-10-27T20:10:00.000Z');
  assert.equal(ended.job, stored.job);
  assert.equal(ended.end, 'succeeded');
  assert.equal(ended.uncertain, false);
  assert.equal(ended.endedAt, '2026-10-27T20:10:00.000Z');
  // 坏记录当没有，不能挡住后续提交。
  for (const bad of ['', 'not json', '{}', '{"job":"x"}', '{"job":"x","kind":"other"}'])
    assert.equal(parseRemoteJob(bad), null, bad);
});

it('treats only a server-confirmed final state as terminal', () => {
  // GUI-07：还在跑、或这次查询没有终态信息时，不能误标成功/失败。
  assert.equal(remoteJobTerminal({ status: { state: 'running' }, ended: null }), null);
  assert.equal(remoteJobTerminal({ status: { state: 'queued' }, ended: null }), null);
  assert.equal(remoteJobTerminal({ status: { state: 'unknown' }, ended: null }), null);
  // finished 的退出码决定成功/失败。
  assert.equal(
    remoteJobTerminal({ status: { state: 'finished', exit_code: 0 }, ended: null }),
    'succeeded',
  );
  assert.equal(
    remoteJobTerminal({ status: { state: 'finished', exit_code: 2 }, ended: null }),
    'failed',
  );
  // 调度系统记下的终态（没有退出码的 lost）也认：取消与失败是两回事，复用普通远程运行的判定。
  assert.equal(remoteJobTerminal({ status: { state: 'lost' }, ended: 'cancelled' }), 'cancelled');
  assert.equal(remoteJobTerminal({ status: { state: 'lost' }, ended: 'failed' }), 'failed');
});

it('states in English that a failed fetch kept the previous local report', () => {
  // GUI-08：后端保证旧镜像没动；这句固定说明要在英文界面里说得清楚，不能夹中文。
  const [zh, en] = benchErrorText(
    'Error: 取回失败：ssh: connect to host 7920ts port 22: Connection timed out。本机旧报告未改动，仍是上次成功版本。',
  );
  assert.match(zh, /本机旧报告未改动/);
  assert.match(en, /^Fetch failed: ssh: connect to host 7920ts port 22: Connection timed out\./);
  assert.match(en, /previous report on this computer is unchanged/);
  assert.match(en, /last successful version/);
  // 本机原本没有旧报告时措辞不同，但同样说清楚没有旧版本可回退。
  const [, none] = benchErrorText(
    '取回失败：服务器上这次比较没有找到可显示的报告（output/<项目>/reports/ 下没有 HTML）。本机原先没有这个比较的报告。',
  );
  assert.match(none, /There was no previous report of this comparison here/);
  // 同一比较并发取回被后端拒绝时的固定句。
  const [, busy] = benchErrorText('这个比较的报告正在取回中，等这次取回结束后再试。');
  assert.match(busy, /being fetched already/);
});

it('waits for an exit code instead of inventing a terminal state', () => {
  // 审查报告第 6 条：COMPLETED 已写、exit_code 还在 NFS 路上时不能当失败；terminal=true 只代表"不再写"。
  assert.equal(remoteJobTerminal({ status: { state: 'finished' }, ended: null }), null);
  assert.equal(
    remoteJobTerminal({ status: { state: 'finished', exit_code: null }, ended: null }),
    null,
  );
  assert.equal(remoteJobTerminal({ status: { state: 'lost' }, ended: null, terminal: true }), null);
  assert.equal(
    remoteJobTerminal({ status: { state: 'running' }, ended: null, terminal: false }),
    null,
  );
  // 退出码/调度系统结论出现后照常判终态。
  assert.equal(
    remoteJobTerminal({ status: { state: 'finished', exit_code: 0 }, ended: null, terminal: true }),
    'succeeded',
  );
  assert.equal(remoteJobTerminal({ status: { state: 'running' }, ended: 'failed' }), 'failed');
});

it('maps the new run-identity and job-query errors into English', () => {
  // 本机运行身份（run-identity）：失败原因带 run_id 与冻结目录；英文界面不能漏出中文句子。
  const failed = benchErrorText(
    '运行 20261028T132345Z-38134-000010 失败：check failed (1):\nboom\n运行目录：/ob/projects/cmp/runs/20261028T132345Z-38134-000010；配置与站点清单已冻结在这次运行里。上次成功的报告未受影响。',
  )[1];
  assert.match(failed, /^Run 20261028T132345Z-38134-000010 failed:/);
  assert.match(failed, /Run folder: \/ob\/projects\/cmp\/runs\//);
  assert.match(failed, /last successful report is unaffected/);
  assert.doesNotMatch(failed, /[\u4e00-\u9fa5]/);
  const never = benchErrorText(
    '运行 r-1 失败：boom\n运行目录：/ob/runs/r-1；配置与站点清单已冻结在这次运行里。这个比较还没有成功过的报告。',
  )[1];
  assert.match(never, /no successful report yet/);

  // 远程查询执行失败（调度器不可用）不是"作业失败"：走 uncertain 路径，英文说清是查不到状态。
  const query = benchErrorText('查询作业状态失败：squeue: command not found')[1];
  assert.match(query, /^Cannot query the job status:/);
  assert.doesNotMatch(query, /[\u4e00-\u9fa5]/);
});

it('summarises exactly the design that is about to be submitted', () => {
  // U-04：摘要里的机器/路径/年份/模拟/变量等必须能在真实生成的配置里逐项对上，证明摘要与提交同源。
  const gba = {
    label: 'GreaterBay 1km',
    info: {
      ...basin.info,
      history_dir: '/p/gba/history',
      prefix: 'gba_hist_',
      extent: [111.0, 115.5, 21.5, 24.5] as [number, number, number, number],
      resolution: 0.008333,
    },
  };
  const design: EvaluationDesign = {
    machine: { host: 'serverA', openbenchRoot: '/ob', refRoot: '/refs' },
    name: 'versions vs flux',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: ['ERA5LAND_LowRes', 'OpenBench_LH_Daily'], Sensible_Heat: 'FLUX' },
    simulations: [gba, site],
    projectDir: '/ob/projects/versions_vs_flux',
    gridRes: 0.25,
    weight: 'mass',
    minYear: 0,
    timeAlignment: 'per_pair',
    numCores: 4,
    metrics: ['bias', 'KGE'],
    scores: ['Overall_Score'],
  };
  const summary = evaluationSummary(design);
  const value = (label: string) => summary.find((item) => item.label[1] === label)?.value;
  assert.equal(value('Machine'), 'server serverA');
  assert.equal(value('OpenBench installed in'), '/ob');
  assert.equal(value('Reference data root'), '/refs');
  assert.equal(value('Comparison project folder'), '/ob/projects/versions_vs_flux');
  assert.equal(value('Evaluation years'), '2002–2005');
  assert.equal(value('Time scale'), 'Day');
  const firstSimulation = value('Simulation 1')!;
  for (const part of [
    'GreaterBay 1km',
    'gridded',
    'gba_hist_',
    '2002–2005',
    'Month',
    '0.008333°',
    '/p/gba/history',
  ])
    assert.ok(firstSimulation.includes(part), `${part} in ${firstSimulation}`);
  const secondSimulation = value('Simulation 2')!;
  assert.ok(secondSimulation.includes('AT-Neu (official)'));
  assert.ok(secondSimulation.includes('station'));
  assert.equal(
    value('Region and target resolution'),
    'target 0.25° · lat [21.5, 24.5] · lon [111, 115.5]',
  );
  assert.equal(
    value('Variables and reference data'),
    'Latent_Heat: ERA5LAND_LowRes, OpenBench_LH_Daily; Sensible_Heat: FLUX',
  );
  assert.equal(value('Metrics and scores'), 'metrics [bias, KGE] · scores [Overall_Score]');
  assert.equal(
    value('Advanced options'),
    'weight=mass · min_year=0 · time_alignment=per_pair · num_cores=4',
  );
  // 同一份设计生成的配置里，摘要中的每一项都有对应内容（同源，不是另抄一份页面状态）。
  const built = buildOpenbenchConfig(design);
  for (const line of [
    '  years: [2002, 2005]',
    '  tim_res: Day',
    '  grid_res: 0.25',
    '  lat_range: [21.5, 24.5]',
    '  lon_range: [111, 115.5]',
    '    root_dir: "/p/gba/history"',
    '    prefix: "gba_hist_"',
    '  Latent_Heat:',
    '    - "ERA5LAND_LowRes"',
    '    - "OpenBench_LH_Daily"',
    '  Sensible_Heat: "FLUX"',
    '  weight: mass',
    '  min_year_threshold: 0',
    '  time_alignment: per_pair',
    '  num_cores: 4',
    'metrics: ["bias", "KGE"]',
    'scores: ["Overall_Score"]',
  ])
    assert.ok(built.config.includes(line), line);
  // 换了机器身份或输入后摘要随之变化（摘要不是静态文案）。
  const moved = evaluationSummary({ ...design, machine: { ...design.machine, host: 'serverB' } });
  assert.equal(moved.find((item) => item.label[1] === 'Machine')?.value, 'server serverB');
  const changed = evaluationSummary({
    ...design,
    simulations: [{ ...gba, info: { ...gba.info, prefix: 'edited_hist_' } }, site],
  });
  assert.ok(
    changed.find((item) => item.label[1] === 'Simulation 1')?.value.includes('edited_hist_'),
  );
});

it('explains a missing region instead of pretending a default one', () => {
  const noExtent: EvaluationDesign = {
    machine: { host: '', openbenchRoot: '/ob', refRoot: '' },
    name: 'comparison',
    years: [2002, 2005],
    timRes: 'Day',
    variables: { Latent_Heat: 'ERA5' },
    simulations: [{ label: 'hb', info: { ...basin.info, extent: null } }],
    projectDir: '/ob/projects/comparison',
    gridRes: 0.25,
  };
  const summary = evaluationSummary(noExtent);
  const region = summary.find((item) => item.label[1] === 'Region and target resolution')?.value;
  assert.match(region ?? '', /target 0\.25°/);
  assert.match(region ?? '', /region unknown/);
  assert.match(region ?? '', /no region: hb/);
  assert.equal(summary.find((item) => item.label[1] === 'Reference data root')?.value, 'not set');
  assert.equal(summary.find((item) => item.label[1] === 'OpenBench installed in')?.value, '/ob');
  assert.equal(summary.find((item) => item.label[1] === 'Machine')?.value, 'this computer');
});

it('maps the three evaluation-configuration save states used by SaveBadge', () => {
  // U-05：没生成/设置变了 = dirty；提交时 = saving；文本与当前设计完全一致 = saved。
  const current = {
    yaml: 'project: x',
    currentFingerprint: 'aaa',
    generated: { fingerprint: 'aaa', config: 'project: x' },
  };
  assert.equal(configSaveState({ ...current, submitting: false }), 'saved');
  // 手改过的文本还没写进比较项目目录（运行时才写），必须显示未保存：
  // 审查报告已复现"只改 YAML 后徽标仍显示已保存"。
  assert.equal(
    configSaveState({ ...current, yaml: 'project: x # edit', submitting: false }),
    'dirty',
  );
  assert.equal(configSaveState({ ...current, submitting: true }), 'saving');
  assert.equal(configSaveState({ ...current, yaml: '', submitting: false }), 'dirty');
  assert.equal(configSaveState({ ...current, generated: null, submitting: false }), 'dirty');
  assert.equal(
    configSaveState({ ...current, currentFingerprint: 'bbb', submitting: false }),
    'dirty',
  );
  assert.equal(
    configSaveState({ ...current, currentFingerprint: 'bbb', submitting: true }),
    'saving',
  );
});

it('maps the server-side unfinished evaluation job rejection in both languages', () => {
  // GUI-07 服务器侧：sidecar 拒绝"提交另一个评估"和"取回报告"时都以这句话开头。
  // 真实服务端原文（remote_bench.rs 的 active_job_error）括号里是"作业号 <id>"，英文必须剥掉
  // 这个中文前缀，否则英文界面出现"(作业号 bench-run-42)"的中英混排（交付审计实测）。
  const suffix = '; confirm its final state or stop it before resubmitting.';
  const [zh, en] = benchErrorText(
    '这个比较还有未完成的评估作业（作业号 bench-run-42）；先核对终态或停止它，避免重复写同一个成果目录。',
  );
  assert.match(zh, /^这个比较还有未完成的评估作业（作业号 bench-run-42）/);
  assert.equal(
    en,
    `This comparison already has an unfinished evaluation job (bench-run-42)${suffix}`,
  );
  assert.doesNotMatch(en, /[\u4e00-\u9fa5]/, 'no Chinese leaks into the English message');
  assert.doesNotMatch(en, /作业号/, 'the Chinese job prefix is stripped');
  // 没有作业号时也不夹中文。
  assert.equal(
    benchErrorText('这个比较还有未完成的评估作业')[1],
    `This comparison already has an unfinished evaluation job${suffix}`,
  );
  // 两条路径的错误都经过 task → shown（去掉 Error: 前缀）再映射。
  assert.equal(
    benchErrorText('Error: 这个比较还有未完成的评估作业（job-1）：x')[1],
    `This comparison already has an unfinished evaluation job (job-1)${suffix}`,
  );
  // 取回报告失败时后端会把原因包进"取回失败：…。本机旧报告未改动…"：里层这句也要翻成英文，
  // 且服务器侧固定句子自带句号时外层不能再叠出".."。
  const [, wrapped] = benchErrorText(
    '取回失败：这个比较还有未完成的评估作业（作业号 job-2）。本机旧报告未改动，仍是上次成功版本。',
  );
  assert.equal(
    wrapped,
    `Fetch failed: This comparison already has an unfinished evaluation job (job-2)${suffix} The previous report on this computer is unchanged; it is still the last successful version.`,
  );
  assert.doesNotMatch(wrapped, /[\u4e00-\u9fa5]/);
  // 半角括号（日志/包装层可能改写）同样识别。
  assert.equal(
    benchErrorText('这个比较还有未完成的评估作业(job-3)')[1],
    `This comparison already has an unfinished evaluation job (job-3)${suffix}`,
  );
});

it('writes the cross-simulation comparison phase only with two or more simulations', () => {
  const built = buildOpenbenchConfig({
    ...base,
    metrics: ['bias'],
    scores: ['Overall_Score'],
    comparisonItems: ['Taylor_Diagram', 'HeatMap'],
  });
  assert.ok(
    built.config.includes('comparison:\n  enabled: true\n  items: ["Taylor_Diagram", "HeatMap"]'),
    built.config,
  );
  // 单个模拟没有跨模拟比较；一个图表都不勾 = 不写这一段（OpenBench 默认关闭）。
  assert.doesNotMatch(
    buildOpenbenchConfig({ ...base, simulations: [site], comparisonItems: ['Taylor_Diagram'] })
      .config,
    /comparison:/,
  );
  assert.doesNotMatch(buildOpenbenchConfig({ ...base, comparisonItems: [] }).config, /comparison:/);
  // 拼错的名字、缺少所需输出都在本地拒绝，不留给 OpenBench check。
  assert.throws(
    () => buildOpenbenchConfig({ ...base, comparisonItems: ['Taylor_Diagrm'] }),
    /OpenBench 3\.0\.8/,
  );
  assert.throws(
    () =>
      buildOpenbenchConfig({
        ...base,
        metrics: [],
        scores: ['Overall_Score'],
        comparisonItems: ['Taylor_Diagram'],
      }),
    /metric|指标/,
  );
  assert.throws(
    () =>
      buildOpenbenchConfig({
        ...base,
        metrics: ['bias'],
        scores: [],
        comparisonItems: ['HeatMap'],
      }),
    /score|评分/,
  );
});

it('summarises the cross-simulation comparison selection', () => {
  const machineDesign: EvaluationDesign = {
    machine: { host: '', openbenchRoot: '/ob', refRoot: '/refs' },
    ...base,
  };
  const summary = evaluationSummary({ ...machineDesign, comparisonItems: ['Taylor_Diagram'] });
  const row = summary.find((item) => item.label[1] === 'Cross-simulation comparison');
  assert.match(row?.value ?? '', /Taylor_Diagram/);
  const off = evaluationSummary({ ...machineDesign, comparisonItems: [] });
  assert.match(
    off.find((item) => item.label[1] === 'Cross-simulation comparison')?.value ?? '',
    /disabled/,
  );
});

it('blocks a selected variable that the station preparation would drop', () => {
  const layered: BenchSimulation = {
    label: 'hb layered',
    info: {
      ...basin.info,
      unsupported_variables: [
        { name: 'f_wliq_soisno', dims: ['time', 'patch', 'soilsnow'], reason: 'soil layers' },
      ],
    },
    variables: { Latent_Heat: { varname: 'f_wliq_soisno', varunit: 'kg m-2' } },
  };
  const layerDesign: ComparisonDesign = {
    ...base,
    variables: { Latent_Heat: 'ERA5' },
    simulations: [layered, site],
  };
  const problems = layerVariableProblems(layerDesign);
  assert.equal(problems.length, 1);
  assert.match(problems[0]![0], /f_wliq_soisno/);
  assert.match(problems[0]![1], /station\/surface evaluation/);
  assert.throws(() => buildOpenbenchConfig(layerDesign), /f_wliq_soisno/);
});

it('maps the remote run-identity errors into English', () => {
  for (const [chinese, expected] of [
    ['这个比较的活动作业锁一直被占用', /active-job lock/],
    ['这个比较还没有成功过的运行', /no successful run yet/],
    ['服务器上还没有可显示的评估报告', /no report to show yet/],
    [
      '这次运行没有成功，没有可显示的评估报告（运行 r-1，状态 failed）',
      /did not succeed.*run r-1, state failed/,
    ],
  ] as const)
    assert.match(benchErrorText(chinese)[1], expected);
  assert.match(
    benchErrorText('服务器上没有这次运行的快照：bench-run-9')[1],
    /no snapshot for run bench-run-9/,
  );
  assert.match(benchErrorText('服务器上的运行记录读不懂：bad json')[1], /cannot be read: bad json/);
});

it('uses one internal name for the OpenBench project and the comparison folder', () => {
  // R11 遗留：ASCII 名以下划线/数字开头时，旧实现 project.name 用 simulationKey（sim_ 前缀/去首尾下划线），
  // 比较目录用 comparisonName → 两者分叉；现在统一成 comparisonName。
  for (const name of ['versions vs flux', '2026 run', '_draft', '123', 'AT-Neu_vs_draft2']) {
    const built = buildOpenbenchConfig({ ...base, name });
    const projectName = /name: "([^"]+)"/.exec(built.config)?.[1];
    assert.equal(projectName, comparisonName(name), name);
    assert.equal(comparisonDir('/ob', name).split('/').pop(), comparisonName(name), name);
  }
});
