import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  buildOpenbenchConfig,
  comparisonDir,
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
  freezeCompareSnapshot,
  parseOpenbenchConfig,
  snapshotStale,
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

// ---- R02/R10/R11：复查报告里仍可复现的三项（HEAD 61182d6e） ----

/** 带 per-simulation 变量覆盖的设计：生成器写 varname/varunit，解析器要完整保留。 */
const mappedSite: BenchSimulation = {
  ...site,
  variables: { Latent_Heat: { varname: 'Qle', varunit: 'W/m2' } },
};
const mappedDesign: ComparisonDesign = { ...base, simulations: [mappedSite] };

it('R02: refuses variable-mapping fields the page cannot carry into the config or its fingerprint', () => {
  const text = buildOpenbenchConfig(mappedDesign).config;
  const parsed = parseOpenbenchConfig(text);
  assert.deepEqual(parsed.problems, []);
  // 支持的字段全部进入有效配置（因此也进指纹）。
  assert.deepEqual(parsed.config!.simulations[0]!.variables, {
    Latent_Heat: { varname: 'Qle', varunit: 'W/m2' },
  });
  const changedUnit = parseOpenbenchConfig(
    text.replace('varunit: "W/m2"', 'varunit: "mm/day"'),
  ).config!;
  assert.notEqual(
    configFingerprint(design, changedUnit),
    configFingerprint(design, parsed.config!),
  );

  // `convert` 会改实际计算，但页面不进有效配置也不进指纹：必须明确拒绝，不能接收后丢弃。
  const withConvert = text.replace(
    '        varunit: "W/m2"',
    '        varunit: "W/m2"\n        convert: "value * 2"',
  );
  const refused = parseOpenbenchConfig(withConvert);
  assert.equal(refused.config, null);
  assert.ok(
    refused.problems.some(([zh, en]) => zh.includes('convert') && en.includes('convert')),
    JSON.stringify(refused.problems),
  );
});

it('R10: reads single-quoted YAML scalars like OpenBench, including # and doubled quotes', () => {
  const stationFile = '/data/openbench/projects/versions_vs_flux/sim/AT_Neu_official.csv';
  const line = `    fulllist: "${stationFile}"`;
  // 单引号里 # 不是注释：OpenBench 读到完整路径，页面不能读成 "# 之前" 的残路径。
  const quoted = parseOpenbenchConfig(
    generated().replace(line, `    fulllist: '${stationFile} # keep'`),
  );
  assert.deepEqual(quoted.problems, []);
  assert.equal(quoted.config!.simulations[0]!.fulllist, `${stationFile} # keep`);

  // YAML 单引号里的 '' 是一个单引号。
  const escaped = parseOpenbenchConfig(
    generated().replace(line, "    fulllist: '/data/it''s/history # x.csv'"),
  );
  assert.deepEqual(escaped.problems, []);
  assert.equal(escaped.config!.simulations[0]!.fulllist, "/data/it's/history # x.csv");

  // 块列表项也按同样的引号语义读（参考数据名里带 #）。
  const listed = parseOpenbenchConfig(
    generated({ variables: { Latent_Heat: ['ref # one', 'ref,two'] } }).replace(
      '    - "ref # one"',
      "    - 'ref # one'",
    ),
  );
  assert.deepEqual(listed.problems, []);
  assert.deepEqual(listed.config!.reference.Latent_Heat, ['ref # one', 'ref,two']);

  // 行内列表里的单引号同样保护内部的逗号与 #（手改指标列表时不会把一项拆成两项）。
  const inline = parseOpenbenchConfig(
    generated({ metrics: ['bias', 'RMSE'] }).replace(
      'metrics: ["bias", "RMSE"]',
      "metrics: ['bias', 'RMSE']",
    ),
  );
  assert.deepEqual(inline.problems, []);
  assert.deepEqual(inline.config!.metrics, ['bias', 'RMSE']);

  // 不回归：引号外、前面是空白的 # 仍是注释；普通标量里的撇号不开启引号（don't 后注释照常去掉）。
  const commented = parseOpenbenchConfig(
    generated().replace(line, '    fulllist: manually # a real comment'),
  );
  assert.deepEqual(commented.problems, []);
  assert.equal(commented.config!.simulations[0]!.fulllist, 'manually');
  const apostrophe = parseOpenbenchConfig(
    generated({ weight: "don't" }).replace("  weight: don't", "  weight: don't # a real comment"),
  );
  assert.deepEqual(apostrophe.problems, []);
  assert.equal(apostrophe.config!.project.weight, "don't");

  // 没配对的单引号不能静默读成残缺路径。
  const unclosed = parseOpenbenchConfig(
    generated().replace(line, "    fulllist: '/data/has # space"),
  );
  assert.equal(unclosed.config, null);
  assert.ok(unclosed.problems.length > 0);
});

it('R07: refuses a hand-edited YAML variable mapping that hits a layer/category variable', () => {
  // 生成路径的 layerVariableProblems 只检查表单 design 里的映射；手改 YAML 换 varname 必须同样在运行前拒绝。
  const layerSimulation: BenchSimulation = {
    ...site,
    variables: { Latent_Heat: { varname: 'Qle', varunit: 'W/m2' } },
    info: {
      ...site.info,
      unsupported_variables: [
        { name: 'f_wliq_soisno', dims: ['soil_layer'], reason: 'layer dimension' },
      ],
    },
  };
  const layerDesign: EvaluationDesign = { machine, ...base, simulations: [layerSimulation] };
  const text = buildOpenbenchConfig({ ...base, simulations: [layerSimulation] }).config;
  const pristine = parseOpenbenchConfig(text).config!;
  assert.deepEqual(structuralProblems(layerDesign, pristine), []);
  const handEdited = parseOpenbenchConfig(
    text.replace('varname: "Qle"', 'varname: "f_wliq_soisno"'),
  ).config!;
  const problems = structuralProblems(layerDesign, handEdited);
  assert.ok(problems.length, 'the hand-edited layer mapping is refused before running');
  assert.ok(
    problems.some(([zh, en]) => zh.includes('f_wliq_soisno') && en.includes('f_wliq_soisno')),
    JSON.stringify(problems),
  );
  assert.ok(
    problems.some(([zh, en]) => zh.includes('soil_layer') && en.includes('soil_layer')),
    'the bilingual problem names the dimension',
  );
});

it('R11: keeps ASCII comparison ids exactly, so _experiment_ cannot merge with experiment', () => {
  assert.equal(comparisonName('experiment'), 'experiment');
  assert.equal(comparisonName('_experiment_'), '_experiment_');
  assert.notEqual(
    comparisonDir('/data/openbench', '_experiment_'),
    comparisonDir('/data/openbench', 'experiment'),
  );
  // ASCII 原名映射不变：已有比较目录不迁移。
  assert.equal(comparisonName('versions vs flux'), 'versions_vs_flux');
  assert.equal(comparisonName('AT-Neu_vs_draft2'), 'AT-Neu_vs_draft2');
  // 后端 valid_name 最多 60 字符：超长 ASCII 名要截断后加稳定指纹，不能生成被后端拒绝的名字。
  const long = 'a'.repeat(70);
  const longId = comparisonName(long);
  assert.ok(longId.length <= 60, longId);
  assert.match(longId, /^[A-Za-z0-9_-]+$/);
  assert.notEqual(comparisonName(`${long}b`), longId);
  // 非 ASCII 行为不回归。
  const chineseA = comparisonName('基准方案');
  const chineseB = comparisonName('改进方案');
  assert.notEqual(chineseA, chineseB);
  assert.match(chineseA, /^[A-Za-z0-9_-]{1,60}$/);
});

// ---- R01：生成时冻结的输入快照：摘要/指纹/运行提交都读同一份输入 ----

/** 只用于计算指纹的生成→解析往返：与页面 generate() 里冻结快照的步骤一致。 */
function generatedConfigFor(target: ComparisonDesign) {
  const built = buildOpenbenchConfig(target);
  const parsed = parseOpenbenchConfig(built.config);
  assert.deepEqual(parsed.problems, []);
  return { built, config: parsed.config! };
}

it('R01: freezes the generated text, the actual site list and one fingerprint of both', () => {
  const oneStation: ComparisonDesign = { ...base, simulations: [site] };
  const { built, config } = generatedConfigFor(oneStation);
  const snapshot = freezeCompareSnapshot(design, built);
  assert.equal(snapshot.generatedText, built.config);
  assert.deepEqual(snapshot.sites, built.sites);
  assert.equal(snapshot.designFingerprint, designFingerprint(design));
  assert.equal(snapshot.fingerprint, configFingerprint(design, config, built.sites));
  // 配置文本本身进指纹（手改配置会换指纹），站点清单一并进指纹。
  const editedConfig = parseOpenbenchConfig(
    built.config.replace('years: [2003, 2005]', 'years: [2004, 2004]'),
  ).config!;
  assert.notEqual(snapshot.fingerprint, configFingerprint(design, editedConfig, built.sites));
  // 表单没变：快照有效。
  assert.equal(snapshotStale(snapshot, design), false);
});

it('R01: site folders/prefixes/coordinates are part of the effective fingerprint even though the YAML cannot show them', () => {
  const movedSite: BenchSimulation = {
    ...site,
    info: {
      ...site.info,
      history_dir: '/p/AT-Neu-v2/history',
      prefix: 'AT-Neu-v2_hist_',
      lon: 12.4,
    },
  };
  const first = generatedConfigFor({ ...base, simulations: [site] });
  const moved = generatedConfigFor({ ...base, simulations: [movedSite] });
  // 单点模拟的 history 目录/前缀/坐标不在生成的 YAML 里：两份配置文本相同。
  assert.equal(first.built.config, moved.built.config);
  assert.notDeepEqual(first.built.sites, moved.built.sites);
  // 但有效指纹必须不同，否则"改了站点输入、指纹没变"（审查报告 R01）。
  assert.notEqual(
    configFingerprint(design, first.config, first.built.sites),
    configFingerprint(design, moved.config, moved.built.sites),
  );
  // 摘要的站点部分读冻结的站点清单，不读当前页面记录。
  const summary = effectiveSummary(design, first.config, first.built.sites);
  const stationRow = summary.find((item) => item.label[1] === 'Simulation 1')!.value;
  assert.match(stationRow, /\/p\/AT-Neu\/out\/AT-Neu\/history/);
  assert.match(stationRow, /47\.1°, 11\.3°/);
  assert.doesNotMatch(stationRow, /AT-Neu-v2/);
});

it('R01: the snapshot expires when the form changes, and stays valid for text-only edits', () => {
  const oneStation: ComparisonDesign = { ...base, simulations: [site] };
  const oneDesign: EvaluationDesign = { machine, ...oneStation };
  const { built } = generatedConfigFor(oneStation);
  const snapshot = freezeCompareSnapshot(oneDesign, built);
  // 编辑站点目录/坐标 → 表单设计变了 → 快照过期（运行前据此阻断）。
  const editedForm: EvaluationDesign = {
    ...oneDesign,
    simulations: [
      { ...site, info: { ...site.info, history_dir: '/p/elsewhere/history', lon: 12.4 } },
    ],
  };
  assert.equal(snapshotStale(snapshot, editedForm), true);
  // 只改文本（表单不动）：快照仍有效，运行用"冻结站点 + 当前文本解析结果"，摘要也读这两部分。
  const editedText = built.config.replace('years: [2003, 2005]', 'years: [2004, 2004]');
  assert.equal(snapshotStale(snapshot, oneDesign), false);
  const editedConfig = parseOpenbenchConfig(editedText).config!;
  assert.deepEqual(structuralProblems(oneDesign, editedConfig), []);
  const summary = effectiveSummary(oneDesign, editedConfig, snapshot.sites);
  const byLabel = (label: string) => summary.find((item) => item.label[1] === label)?.value;
  assert.equal(byLabel('Evaluation years'), '2004–2004');
  assert.match(byLabel('Simulation 1') ?? '', /\/p\/AT-Neu\/out\/AT-Neu\/history/);
  assert.match(byLabel('Simulation 1') ?? '', /47\.1°, 11\.3°/);
});

it('R03: refuses output directories that are absolute, escape the run directory or name it itself', () => {
  const pristine = generated();
  for (const bad of ['/tmp/out', '../shared', '~', '.', 'sim/..', 'C:\\tmp']) {
    const parsed = parseOpenbenchConfig(
      pristine.replace('  output_dir: ./output', `  output_dir: "${bad.replace(/\\/g, '\\\\')}"`),
    );
    assert.equal(parsed.config, null, bad);
    assert.ok(
      parsed.problems.some(([zh, en]) => zh.includes('output_dir') && en.includes('output_dir')),
      `${bad}: ${JSON.stringify(parsed.problems)}`,
    );
  }
  // 运行目录内的相对路径（含可归一化的写法）仍然接受，并进入有效配置/指纹。
  const custom = parseOpenbenchConfig(
    pristine.replace('  output_dir: ./output', '  output_dir: a/../custom-output'),
  );
  assert.deepEqual(custom.problems, []);
  assert.equal(custom.config!.project.outputDir, 'a/../custom-output');
  assert.deepEqual(structuralProblems(design, custom.config!), []);
  assert.notEqual(
    configFingerprint(design, custom.config!),
    configFingerprint(design, parseOpenbenchConfig(pristine).config!),
    'a different output directory is a different effective input',
  );
});
