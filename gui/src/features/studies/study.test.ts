import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  budgetAdvice,
  calibrationSeed,
  calibrationWindows,
  candidateBudget,
  fitValidation,
  frozenDesign,
  frozenParameterKey,
  gridTargetDefaults,
  gridTargetSpec,
  memberVerdict,
  networkWeightCount,
  plannedCandidates,
  studyProgress,
  networkSpec,
  networkDefaults,
  parameterSpec,
  studyPermissions,
  sensitivityRanking,
  sensitivityResponses,
  studyDiagnostics,
  studyRunOutcome,
  studyWarningText,
  suggestedRange,
  suggestedWindows,
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
  it('prefills a range the spec accepts, stepping off open bounds', () => {
    const accepted = (row: ParameterContext, scale: string) => {
      const [min, max] = suggestedRange(row, scale);
      parameterSpec(row, min, max, scale);
      return [min, max];
    };
    assert.deepEqual(accepted({ ...parameter, default: 4 }, 'linear'), ['2', '6']);
    assert.deepEqual(accepted({ ...parameter, default: 8 }, 'linear'), ['4', '10']);
    // GRADM 一类：下界 1.6 是开区间，默认值的一半落到界外，收到界内而不是贴在 1.6 上。
    const gradm = { ...parameter, default: 2, min: 1.6, max: undefined, min_inclusive: false };
    const [low] = accepted(gradm, 'linear');
    assert.ok(Number(low) > 1.6);
    // 目录用最小正规数表示"严格大于 0"；默认 0 时用目录上下界。
    const tiny = { ...parameter, default: 0, min: Number.MIN_VALUE * 2 ** 52, max: 1 };
    assert.ok(Number(accepted(tiny, 'linear')[0]) > 0);
    assert.deepEqual(suggestedRange({ ...parameter, default: 0, max: undefined }, 'linear'), [
      '',
      '',
    ]);
    assert.deepEqual(suggestedRange({ ...parameter, default: -1, min: undefined }, 'log'), [
      '',
      '',
    ]);
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

it('prefilled windows drop a warm-up year without spin-up and hold out the last 30% for validation', () => {
  const site = (start: string, end: string, spinupYears = 0, outputStart = start) => ({
    start,
    end,
    outputStart,
    spinupYears,
  });
  assert.deepEqual(suggestedWindows([site('2002-01-01', '2012-12-31')]), {
    from: '2003-01-01',
    to: '2010-01-01',
    validationFrom: '2010-01-01',
    validationTo: '2012-12-31',
    warmup: true,
    validationYears: 3,
  });
  // 算例自带预热：history 从预热截止日开始，不再扣第一年。
  const warmed = suggestedWindows([site('2000-01-01', '2012-12-31', 2, '2002-01-01')])!;
  assert.equal(warmed.from, '2002-01-01');
  assert.equal(warmed.warmup, false);
  assert.equal(warmed.validationYears, 3);
  // 不足三年留不出验证期；不足两年也不扣预热。
  assert.deepEqual(suggestedWindows([site('2005-01-01', '2006-12-31')]), {
    from: '2006-01-01',
    to: '2006-12-31',
    validationFrom: '',
    validationTo: '',
    warmup: true,
    validationYears: 0,
  });
  assert.equal(suggestedWindows([site('2005-01-01', '2005-12-31')])!.warmup, false);
  // 多个站点取共同时段，有一个没预热就按没预热算。
  const both = suggestedWindows([
    site('2000-01-01', '2010-12-31', 1, '2001-01-01'),
    site('2003-01-01', '2012-12-31'),
  ])!;
  assert.equal(both.from, '2004-01-01');
  assert.equal(both.validationTo, '2010-12-31');
  assert.equal(suggestedWindows([site('2005-01-01', '2004-01-01')]), null);
  assert.equal(suggestedWindows([site('bad', '2004-01-01')]), null);
  // 预填的窗口一定能过 calibrationWindows。
  const w = suggestedWindows([site('2002-01-01', '2012-12-31')])!;
  calibrationWindows(
    `${w.from}T00:00`,
    `${w.to}T00:00`,
    `${w.validationFrom}T00:00`,
    `${w.validationTo}T00:00`,
  );
});

it('budget advice flags too many parameters, small populations and too few runs per parameter', () => {
  assert.deepEqual(budgetAdvice(3, 15, 4), []);
  assert.equal(budgetAdvice(3, 12, 6).length, 1, 'population below 5 x parameters');
  assert.equal(budgetAdvice(3, 8, 1).length, 2, 'and fewer than 20 runs per parameter');
  assert.match(budgetAdvice(10, 50, 4)[0]![0], /不确定性分析/);
  assert.deepEqual(budgetAdvice(0, 8, 4), []);
});

it('write-back is blocked unless the member beats the baseline in calibration and validation', () => {
  const base = { feasible: true, calibration: 1, validation: 1 };
  const verdict = (chosen: Record<string, unknown>, baseline: Record<string, unknown> = base) =>
    memberVerdict({ m000000: baseline, m000004: chosen }, 'm000004');
  assert.equal(verdict({ feasible: true, calibration: 0.8, validation: 0.9 })!.blocked, false);
  assert.match(verdict({ feasible: true, calibration: 0.8, validation: 0.9 })!.zh, /降低 20%/);
  assert.equal(verdict({ feasible: true, calibration: 1, validation: 0.5 })!.blocked, true);
  assert.match(verdict({ feasible: true, calibration: 0.8, validation: 1.1 })!.zh, /过拟合/);
  assert.equal(verdict({ feasible: false })!.blocked, true);
  const noValidation = verdict(
    { feasible: true, calibration: 0.8 },
    { feasible: true, calibration: 1 },
  )!;
  assert.equal(noValidation.blocked, false);
  assert.match(noValidation.zh, /没有验证期/);
  assert.equal(memberVerdict({ m000000: base }, 'm000000'), null);
  assert.equal(memberVerdict({ m000004: base }, 'm000004'), null, 'no baseline score');
  assert.equal(memberVerdict(undefined, 'm000004'), null);
});

it('study warnings are shown in both languages and per-member validation notes are counted', () => {
  assert.match(
    studyWarningText('overfitting: calibration improved while validation became worse')[0],
    /过拟合/,
  );
  const [zh, en] = studyWarningText('bound: DEF_TUNING_CNFAC upper 0.895');
  assert.match(zh, /DEF_TUNING_CNFAC 的最优值 0.895 贴着范围上限/);
  assert.match(en, /upper bound/);
  assert.match(studyWarningText('bound: DEF_LC_VMAX25(10) lower 20')[0], /下限/);
  assert.match(
    studyWarningText(
      'tuning baseline failed; candidates were not run and can be retried after fixing the baseline',
    )[0],
    /基准/,
  );
  assert.match(
    studyWarningText(
      'AT-Neu / f_lfevpa: 12 timestamps have insufficient ensemble support (need at least 20)',
    )[0],
    /AT-Neu 的 f_lfevpa 有 12 个时刻成功的成员不足 20 个/,
  );
  assert.match(
    studyWarningText('m000003 / AT-Neu / f_fsena was excluded: cannot open x.nc')[0],
    /m000003.*cannot open x.nc/,
  );
  assert.deepEqual(studyWarningText('something new'), ['something new', 'something new']);
  const notes = studyDiagnostics([
    'm000003 / site validation unavailable: x',
    'm000004 / site validation target Qle unavailable: y',
    'm000003 / site2 validation unavailable: z',
    'bound: A lower 1',
  ]);
  assert.equal(notes.length, 2);
  assert.match(notes[1]![0], /^2 组参数的验证期成绩算不出来/);
  assert.deepEqual(studyDiagnostics(undefined), []);
});

it('a cancelled run is a notice and a failed run shows only its reason', () => {
  const cancelled = studyRunOutcome(
    new Error(
      '{"type":"study_cancelled","kind":"study_cancelled","study_dir":"/s","reason":"运行已取消"}',
    ),
  );
  assert.equal(cancelled.cancelled, true);
  assert.doesNotMatch(cancelled.zh, /study_cancelled/);
  const failed = studyRunOutcome('{"type":"study_failed","reason":"kernel exited 2"}');
  assert.equal(failed.cancelled, false);
  assert.equal(failed.zh, '研究运行失败：kernel exited 2');
  assert.deepEqual(studyRunOutcome(new Error('ssh: connection refused')), {
    cancelled: false,
    zh: 'ssh: connection refused',
    en: 'ssh: connection refused',
  });
});

it('the frozen design lists every parameter range and the budget to review', () => {
  const frozen = frozenDesign({
    seed: 7,
    parameters: [
      {
        name: 'DEF_LC_VMAX25',
        scope_instance: { kind: 'land-cover-class', scheme: 'IGBP', index: 10 },
        sample_min: 26,
        sample_max: 78,
        scale: 'linear',
      },
      { name: 'DEF_TUNING_TRSMX0', sample_min: 1e-4, sample_max: 3e-4, scale: 'log' },
    ],
    budget: { population: 10, generations: 4, jobs: 12, patience: 3, include_baseline: true },
  });
  assert.deepEqual(frozen.parameters, [
    { name: 'DEF_LC_VMAX25 (IGBP 10)', range: '26 – 78', log: false },
    { name: 'DEF_TUNING_TRSMX0', range: '0.0001 – 0.0003', log: true },
  ]);
  assert.match(frozen.budget[0]![0], /种群 10、迭代 4 代、默认参数可进初代/);
  assert.ok(frozen.budget.some(([zh]) => zh === '随机种子 7'));
  assert.deepEqual(frozenDesign(null), { parameters: [], budget: [] });
});

it('sensitivity ranks by comparable impact and carries frozen ranges into calibration', () => {
  const spec = {
    parameters: [
      {
        name: 'DEF_LC_VMAX25',
        parameter_id: 'lct:IGBP:DEF_LC_VMAX25',
        scope_instance: { kind: 'land-cover-class', scheme: 'IGBP', index: 10 },
        sample_min: 26,
        sample_max: 78,
        scale: 'linear',
      },
      {
        name: 'DEF_PFT_G1',
        parameter_id: 'pft:DEF_PFT_G1',
        scope_instance: { kind: 'pft-type', index: 13 },
        sample_min: 1,
        sample_max: 8,
        scale: 'log',
      },
    ],
  };
  const rows = [
    {
      site: 'AT-Neu',
      variable: 'f_lfevpa',
      parameter: 'DEF_LC_VMAX25',
      method: 'morris',
      value: 2,
      mu: 2,
      sigma: 0.1,
      n: 10,
    },
    {
      site: null,
      variable: 'objective',
      parameter: 'DEF_LC_VMAX25',
      method: 'morris',
      value: 0.05,
      mu: -0.05,
      sigma: 0.01,
      n: 10,
    },
    {
      site: null,
      variable: 'objective',
      parameter: 'DEF_PFT_G1(14)',
      method: 'morris',
      value: 0.2,
      mu: 0.1,
      sigma: 0.2,
      n: 10,
    },
  ];
  assert.deepEqual(
    sensitivityResponses(rows).map((r) => r.key),
    ['objective', 'AT-Neu / f_lfevpa'],
  );
  const ranked = sensitivityRanking(rows, 'objective', spec);
  assert.deepEqual(
    ranked.map((r) => [r.parameter, r.impact, r.direction]),
    [
      ['DEF_PFT_G1(14)', 0.2, 1],
      ['DEF_LC_VMAX25', 0.05, -1],
    ],
  );
  // OAT 的斜率乘上抽样范围才和别的参数可比。
  const oat = sensitivityRanking(
    [
      {
        site: null,
        variable: 'objective',
        parameter: 'DEF_LC_VMAX25',
        method: 'oat_finite_difference_slope',
        value: -0.01,
        n: 2,
      },
    ],
    'objective',
    spec,
  );
  assert.ok(Math.abs(oat[0]!.impact! - 0.52) < 1e-12);
  assert.equal(oat[0]!.direction, -1);
  assert.equal(frozenParameterKey(spec.parameters[1]!), 'DEF_PFT_G1(14)');
  assert.deepEqual(calibrationSeed(spec, ['DEF_PFT_G1(14)']), [
    {
      parameter_id: 'pft:DEF_PFT_G1',
      raw_key: 'DEF_PFT_G1',
      scheme: null,
      index: 13,
      min: '1',
      max: '8',
      scale: 'log',
    },
  ]);
  assert.deepEqual(sensitivityResponses(null), []);
});

it('a Morris budget counts trajectories times parameters plus one', () => {
  assert.equal(candidateBudget('uncertainty', 'morris', 3, 10, 0, 0, 2).candidates, 40);
  assert.equal(candidateBudget('uncertainty', 'morris', 3, 10, 0, 0, 2).tasks, 82);
  assert.throws(
    () => candidateBudget('uncertainty', 'morris', 3, 1, 0, 0, 1),
    /至少要 2 条轨迹|at least two trajectories/,
  );
});

it('planned samples follow each method, not the raw candidate_count', () => {
  const parameters = [{}, {}, {}, {}];
  assert.equal(
    plannedCandidates({ method: 'morris', parameters, budget: { candidate_count: 8 } }),
    40,
  );
  assert.equal(plannedCandidates({ method: 'morris', parameters, budget: {} }), 50);
  assert.equal(plannedCandidates({ method: 'oat', parameters, budget: {} }), 8);
  assert.equal(
    plannedCandidates({ method: 'lhs', parameters, budget: { candidate_count: 40 } }),
    40,
  );
  assert.equal(
    plannedCandidates({
      method: 'differential-evolution',
      parameters,
      budget: { population: 10, generations: 4 },
    }),
    50,
  );
  assert.equal(plannedCandidates(null), null);
  assert.match(
    frozenDesign({ method: 'morris', parameters, budget: { candidate_count: 8 } }).budget[0]![0],
    /Morris 8 条轨迹，共 40 组/,
  );
});

it('network weights count every layer like the backend', () => {
  const linear = networkSpec(
    'pft',
    'clim_tair, clim_vpd, clim_prec',
    'DEF_PFT_VMAX25',
    '0.5',
    '2',
    'linear',
    true,
  );
  assert.equal(networkWeightCount(linear), 3 * 1 + 1);
  const hidden = networkSpec(
    'pft',
    'clim_tair, clim_vpd, clim_prec',
    'DEF_PFT_VMAX25',
    '0.5',
    '2',
    '8',
    true,
  );
  assert.equal(networkWeightCount(hidden), 3 * 8 + 8 + 8 * 1 + 1);
});

it('an early-stopped calibration reads as converged, not interrupted', () => {
  const de = {
    method: 'differential-evolution',
    parameters: [{}, {}],
    budget: { population: 10, generations: 4, patience: 3 },
  };
  const running = studyProgress(de, 20, 'Running');
  assert.equal(running.zh, '已完成 20 / 最多 50 组参数');
  assert.match(running.note![0], /连续 3 代没有改善会提前结束/);
  const stopped = studyProgress(de, 40, 'Completed');
  assert.equal(stopped.percent, 100);
  assert.match(stopped.note![0], /提前结束：这是收敛了/);
  assert.equal(studyProgress(de, 50, 'Completed').note, null);
  const lhs = studyProgress(
    { method: 'lhs', parameters: [{}], budget: { candidate_count: 40 } },
    10,
    'Running',
  );
  assert.deepEqual([lhs.percent, lhs.zh, lhs.note], [25, '已完成 10 / 40 组参数', null]);
  assert.match(
    frozenDesign({ ...de, budget: { ...de.budget, patience: 0 } }).budget.join(' '),
    /跑满全部代数/,
  );
});

it('a relative network output cannot be a negative multiplier', () => {
  assert.throws(
    () => networkSpec('pft', 'clim_tair', 'DEF_PFT_VMAX25', '-0.5', '2', 'linear', true),
    /倍数|multiplies/,
  );
  networkSpec('pft', 'clim_tair', 'DEF_PFT_VMAX25', '0', '2', 'linear', true);
});
