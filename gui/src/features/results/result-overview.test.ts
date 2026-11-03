import test from 'node:test';
import assert from 'node:assert/strict';
import {
  overviewMetric,
  overviewVariables,
  metricWarningText,
  overviewRunStatus,
  overviewVariable,
  overviewRunLabel,
  overviewFinding,
  overviewRunLocation,
} from './result-overview';
const metric = {
  name: 'Qle',
  model_var: 'f_lfevpa',
  n: 9000,
  rmse: 2,
  bias: -1,
  mae: 1,
  r2: null,
  nse: null,
  kge: null,
  time: [0, 1],
};
test('overview uses full paired count and matches native model variable, never multi-site metrics', () => {
  assert.equal(overviewMetric([metric], 'f_lfevpa')?.n, 9000);
  assert.equal(overviewMetric([{ ...metric, site: '/other' }], 'f_lfevpa'), undefined);
  assert.equal(overviewMetric([metric], 'f_fsena'), undefined);
});
test('history availability does not claim success and reconciliation overrides status', () => {
  assert.match(overviewRunStatus({ run_case: '/run' }, false), /未确认/);
  assert.match(
    overviewRunStatus({ status: 'completed', needs_reconciliation: true }, false),
    /待核对/,
  );
  assert.equal(overviewRunStatus({ status: 'failed' }, false), '运行失败');
});

test('overview defaults to an actual latent-heat series and compact labels retain run identity', () => {
  assert.equal(
    overviewVariable([
      { name: 'f_assim', kind: 'series' },
      { name: 'f_lfevpa', kind: 'series' },
    ]),
    'f_lfevpa',
  );
  assert.equal(
    overviewVariable([
      { name: 'time', kind: 'series' },
      { name: 'f_assim', kind: 'series' },
    ]),
    'f_assim',
  );
  assert.equal(overviewVariable([{ name: 'f_lfevpa', kind: 'static' }]), '');
  assert.equal(
    overviewRunLabel(
      {
        operation_id: '9b72ff93-23a7-40cb',
        run_case: '/private/model/copy',
        version: { id: 'official', revision: 3 },
      },
      false,
    ),
    '运行 9b72ff93-23a · official@3',
  );
  assert.equal(
    overviewRunLabel({ run_case: '/private/legacy' }, true),
    'Run legacy · Unknown version',
  );
});

test('reference summary uses actual paired bias and never infers a validated result or execution location', () => {
  assert.match(overviewFinding(undefined, 'f_lfevpa', false), /尚无有效/);
  assert.match(overviewFinding({ ...metric, n: 0 }, 'f_lfevpa', false), /尚无有效/);
  assert.match(
    overviewFinding({ ...metric, bias: 58.099, units: 'W/m²' }, 'f_lfevpa', false),
    /58.1 W\/m²/,
  );
  assert.match(overviewFinding({ ...metric, bias: null }, 'f_lfevpa', false), /暂不可用/);
  assert.equal(overviewRunLocation(undefined, false), '');
  assert.equal(
    overviewRunLocation({ provenance: { engine: 'rust', location: { kind: 'local' } } }, false),
    '本机 · Rust',
  );
  assert.equal(
    overviewRunLocation(
      { provenance: { engine: 'fortran', location: { kind: 'remote', host: 'T7920' } } },
      false,
    ),
    'T7920 · Fortran',
  );
});

test('composite evaluations are separate choices and never evidence for a component variable', () => {
  const respiration = {
    ...metric,
    name: 'Resp',
    model_var: 'f_ar + f_hr',
    units: 'µmol/m²/s',
    label_en: 'Ecosystem respiration',
  };
  assert.equal(overviewMetric([respiration], 'f_ar'), undefined);
  const choices = overviewVariables(
    [{ name: 'f_ar', kind: 'series', dimensions: [] }],
    [respiration],
    true,
  );
  assert.deepEqual(
    choices.map((choice) => choice.name),
    ['f_ar', 'Resp'],
  );
  assert.equal(overviewMetric([respiration], 'Resp'), respiration);
});
test('metric warnings follow the current UI language without changing values', () => {
  assert.equal(
    metricWarningText('KGE 不可信：观测均值 0.1 接近零', true),
    'KGE is unreliable: observed mean 0.1 is near zero',
  );
  assert.equal(
    metricWarningText('4 个输出区间因观测覆盖不足未参与配对（阈值 80%）', true),
    '4 output intervals were excluded for insufficient observation coverage (threshold 80%)',
  );
  assert.equal(
    metricWarningText('观测覆盖率均值 60% 偏低（<80%），整段均值代表的是部分时段', true),
    'Mean observation coverage is low (60%, below 80%); interval means represent only part of each period',
  );
  assert.equal(
    metricWarningText('KGE 不可信：模型与观测的均值反号', false),
    'KGE 不可信：模型与观测的均值反号',
  );
});

test('schedule and history-source warnings are localized without losing their evidence', () => {
  for (const warning of [
    '读不到 /case/case.nml（permission denied）；history 区间起点按名义边界推断',
    'DEF_HIST_FREQ 无法识别（bad value）；输出频率改由 history 标签判断',
    'DEF_simulation_time%timestep = -1 不能当作 history 输出步长；TIMESTEP 频率将用标签间隔推断',
    'case.nml 的模拟起点不可用（bad date）；history 区间起点按名义边界推断',
    'case.nml 的预热截止时刻不可用（bad date）；history 区间起点按模拟起点处理',
    'case.nml 声明 DEF_HIST_FREQ = DAILY，history 标签显示是 MONTHLY；按 history 为准',
    'case.nml 的模拟起点晚于首条 history 记录（配置被改过？）；区间起点改按名义边界推断',
  ]) {
    assert.doesNotMatch(metricWarningText(warning, true), /[\u3400-\u9fff]/);
    assert.equal(metricWarningText(warning, false), warning);
  }
});
