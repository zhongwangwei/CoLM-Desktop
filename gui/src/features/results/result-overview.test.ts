import test from 'node:test';
import assert from 'node:assert/strict';
import {
  overviewMetric,
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
