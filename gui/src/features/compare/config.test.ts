import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildOpenbenchConfig,
  comparisonName,
  designFingerprint,
  simulationKey,
  type BenchSimulation,
  type ComparisonDesign,
  type EvaluationDesign,
} from './compare';
import {
  configFingerprint,
  effectiveSummary,
  parseOpenbenchConfig,
  structuralProblems,
} from './config';

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
    extent: [-160.5, -158.5, 18.5, 22.5],
    resolution: 1,
  },
};
const base: ComparisonDesign = {
  name: 'versions vs flux',
  years: [2003, 2005],
  timRes: 'Day',
  variables: { Latent_Heat: 'OpenBench_LH_Daily' },
  simulations: [site, basin],
  projectDir: '/data/openbench/projects/versions_vs_flux',
};
const machine = { host: '', openbenchRoot: '/opt/ob', refRoot: '/data/ref' };
const design: EvaluationDesign = { machine, ...base };

/** 生成配置 → 解析 → 再生成，用来确认解析器接受的是生成器自己的格式。 */
function generated(overrides: Partial<ComparisonDesign> = {}) {
  return buildOpenbenchConfig({ ...base, ...overrides }).config;
}

it('parses the configuration the page generates (round trip)', () => {
  const parsed = parseOpenbenchConfig(generated());
  assert.deepEqual(parsed.problems, []);
  const config = parsed.config!;
  assert.deepEqual(config.project.years, [2003, 2005]);
  assert.equal(config.project.timRes, 'Day');
  assert.equal(config.project.name, 'versions_vs_flux');
  assert.deepEqual(config.variables, ['Latent_Heat']);
  assert.deepEqual(config.reference, { Latent_Heat: ['OpenBench_LH_Daily'] });
  assert.deepEqual(
    config.simulations.map((simulation) => simulation.key),
    ['AT_Neu_official', 'hb_draft_2'],
  );
  const station = config.simulations[0]!;
  assert.equal(station.dataType, 'stn');
  assert.equal(
    station.fulllist,
    '/data/openbench/projects/versions_vs_flux/sim/AT_Neu_official.csv',
  );
  assert.equal(station.timRes, 'Hour');
  const grid = config.simulations[1]!;
  assert.equal(grid.dataType, 'grid');
  assert.equal(grid.rootDir, '/p/hb dir/out/hb/history');
  assert.equal(grid.dataGroupby, 'Month');
});

it('reads hand edits as the effective evaluation conditions', () => {
  const edited = generated()
    .replace('years: [2003, 2005]', 'years: [2004, 2004]')
    .replace('tim_res: Day', 'tim_res: Month')
    .replace('variables: [Latent_Heat]', 'variables: [Latent_Heat, Sensible_Heat]')
    .replace(
      'reference:\n  Latent_Heat: "OpenBench_LH_Daily"',
      'reference:\n  Latent_Heat: "OpenBench_LH_Daily"\n  Sensible_Heat:\n    - "OpenBench_SH_Daily"\n    - "FLUXNET_SH"',
    );
  const parsed = parseOpenbenchConfig(edited);
  assert.deepEqual(parsed.problems, []);
  const config = parsed.config!;
  assert.deepEqual(config.project.years, [2004, 2004]);
  assert.equal(config.project.timRes, 'Month');
  assert.deepEqual(config.variables, ['Latent_Heat', 'Sensible_Heat']);
  assert.deepEqual(config.reference.Sensible_Heat, ['OpenBench_SH_Daily', 'FLUXNET_SH']);

  const summary = effectiveSummary(design, config);
  const values = summary.map((item) => item.value).join('\n');
  assert.match(values, /2004–2004/);
  assert.match(values, /Month/);
  assert.match(values, /Sensible_Heat: OpenBench_SH_Daily, FLUXNET_SH/);
  // 摘要与配置是同一份：表单设计仍是 2003–2005 / Day，但摘要必须显示有效的 2004–2004 / Month。
  assert.doesNotMatch(values, /2003–2005/);
});

it('changes the fingerprint when the effective conditions change, not when only layout does', () => {
  const first = parseOpenbenchConfig(generated()).config!;
  const commented = parseOpenbenchConfig(`# 手写注释\n${generated()}`).config!;
  assert.equal(configFingerprint(design, first), configFingerprint(design, commented));
  const edited = parseOpenbenchConfig(
    generated().replace('years: [2003, 2005]', 'years: [2004, 2004]'),
  ).config!;
  assert.notEqual(configFingerprint(design, first), configFingerprint(design, edited));
});

it('refuses edits the page cannot honestly summarize', () => {
  const reversed = parseOpenbenchConfig(
    generated().replace('years: [2003, 2005]', 'years: [2005, 2003]'),
  );
  assert.equal(reversed.config, null);
  assert.ok(reversed.problems.length);

  // comparison 现在受支持；真正超出页面格式的顶层键（如 statistics）仍然拒绝。
  const unknownTop = parseOpenbenchConfig(
    generated().replace('evaluation:', 'statistics:\n  enabled: true\nevaluation:'),
  );
  assert.equal(unknownTop.config, null);
  assert.match(unknownTop.problems[0]![0], /statistics/);

  const unknownSim = parseOpenbenchConfig(
    generated().replace('    tim_res: Hour', '    tim_res: Hour\n    grid_res: 0.5'),
  );
  assert.equal(unknownSim.config, null);

  const badType = parseOpenbenchConfig(generated().replace('data_type: stn', 'data_type: station'));
  assert.equal(badType.config, null);

  const unquotedColon = parseOpenbenchConfig(
    generated().replace(
      'reference:\n  Latent_Heat: "OpenBench_LH_Daily"',
      'reference:\n  Latent_Heat: OpenBench LH: Daily',
    ),
  );
  assert.equal(unquotedColon.config, null);

  const missingColon = parseOpenbenchConfig('project\n  name: "x"');
  assert.equal(missingColon.config, null);
});

it('keeps quoted paths, block lists and escapes intact', () => {
  const parsed = parseOpenbenchConfig(
    generated({
      variables: { Latent_Heat: ['ref,one', 'ref # two'] },
      projectDir: '/data/open bench/projects/w',
    }),
  );
  assert.deepEqual(parsed.problems, []);
  assert.deepEqual(parsed.config!.reference.Latent_Heat, ['ref,one', 'ref # two']);
  assert.equal(
    parsed.config!.simulations[0]!.fulllist,
    '/data/open bench/projects/w/sim/AT_Neu_official.csv',
  );
});

it('blocks structural edits that would run on data the page did not prepare', () => {
  assert.deepEqual(structuralProblems(design, parseOpenbenchConfig(generated()).config!), []);

  const added = generated().replace(
    '  hb_draft_2:',
    '  extra_sim:\n    model: CoLM2024\n    root_dir: "/p/extra/history"\n    prefix: "extra_hist_"\n    data_groupby: single\n    data_type: grid\n    tim_res: Day\n  hb_draft_2:',
  );
  const addedProblems = structuralProblems(design, parseOpenbenchConfig(added).config!);
  assert.ok(addedProblems.length);
  assert.match(addedProblems[0]![0], /extra_sim/);

  const movedStation = generated().replace(
    'fulllist: "/data/openbench/projects/versions_vs_flux/sim/AT_Neu_official.csv"',
    'fulllist: "/tmp/elsewhere.csv"',
  );
  assert.ok(structuralProblems(design, parseOpenbenchConfig(movedStation).config!).length);
});

it('makes Chinese names distinct internal ids while keeping ASCII names unchanged', () => {
  assert.equal(simulationKey('2026 run'), 'sim_2026_run');
  assert.equal(simulationKey('AT-Neu (official)'), 'AT_Neu_official');
  const first = simulationKey('基准模拟');
  const second = simulationKey('改进模拟');
  assert.notEqual(first, second);
  assert.equal(simulationKey('基准模拟'), first, 'stable across calls');
  assert.match(first, /^[A-Za-z0-9_-]+$/);

  assert.equal(comparisonName('versions vs flux'), 'versions_vs_flux');
  const a = comparisonName('基准方案');
  const b = comparisonName('改进方案');
  assert.notEqual(a, b);
  assert.match(a, /^[A-Za-z0-9_-]{1,60}$/);
  assert.ok(a.length <= 60);
});

it('does not silently fix an impossible region into a backwards range', () => {
  const broken: BenchSimulation = {
    ...basin,
    info: { ...basin.info, extent: [199.5, 180, 21.5, 26.9] },
  };
  const { config, sites } = buildOpenbenchConfig({ ...base, simulations: [broken, site] });
  assert.doesNotMatch(config, /lon_range/);
  assert.equal(sites.length, 1);
  assert.equal(designFingerprint({ machine, ...base }), designFingerprint({ machine, ...base }));
});

it('parses and summarises the cross-simulation comparison section', () => {
  const text = generated({
    comparisonItems: ['Taylor_Diagram', 'HeatMap'],
    metrics: ['bias'],
    scores: ['Overall_Score'],
  });
  const parsed = parseOpenbenchConfig(text);
  assert.deepEqual(parsed.problems, []);
  assert.deepEqual(parsed.config!.comparison, {
    enabled: true,
    items: ['Taylor_Diagram', 'HeatMap'],
  });
  const row =
    effectiveSummary(design, parsed.config!).find(
      (item) => item.label[1] === 'Cross-simulation comparison',
    )?.value ?? '';
  assert.match(row, /Taylor_Diagram/);

  // 不写 comparison = 关闭（OpenBench 默认）；摘要如实说未启用。
  const off = parseOpenbenchConfig(generated()).config!;
  assert.deepEqual(off.comparison, { enabled: false, items: null });
  assert.match(
    effectiveSummary(design, off).find((item) => item.label[1] === 'Cross-simulation comparison')
      ?.value ?? '',
    /disabled/,
  );

  // 手改成未知图表 / enabled 配空 items：明确拒绝，不给"看起来对"的配置。
  const unknown = text.replace('items: ["Taylor_Diagram", "HeatMap"]', 'items: ["Not_A_Chart"]');
  assert.equal(parseOpenbenchConfig(unknown).config, null);
  const empty = text.replace('items: ["Taylor_Diagram", "HeatMap"]', 'items: []');
  assert.equal(parseOpenbenchConfig(empty).config, null);
});

it('accepts the full OpenBench 3.0.8 tim_res value set', () => {
  for (const timRes of ['6Hour', '8Day', 'climatology-month', 'climatology-year']) {
    const parsed = parseOpenbenchConfig(generated({ timRes }));
    assert.deepEqual(parsed.problems, [], timRes);
    assert.equal(parsed.config!.project.timRes, timRes);
  }
  assert.equal(
    parseOpenbenchConfig(generated({ timRes: '5Day' })).config,
    null,
    'an unsupported tim_res is refused rather than guessed',
  );
});
