import assert from 'node:assert/strict';
import test from 'node:test';
import {
  comparisonCsv,
  comparisonCsvHeader,
  comparisonKey,
  exportGate,
  fresh,
  singleKey,
  type ComparisonRequest,
  type ComparisonResult,
  type SingleRequest,
  type SingleResult,
} from './analysis-state';

const singleRequest = (): SingleRequest => ({
  caseDir: '/case',
  versionId: 'official',
  revision: 1,
  source: '/case/runs/a',
  obs: '/obs/a.nc',
  from: '2008-01-01T00:00',
  to: '2008-01-02T00:00',
  spinup: '0',
  corrected: false,
});

const comparisonRequest = (): ComparisonRequest => ({
  entries: [
    { runCase: '/case/runs/a', obs: '/obs/a.nc' },
    { runCase: '/other/runs/b', obs: '/obs/b.nc' },
  ],
  from: '2008-01-01T00:00',
  to: '2008-01-02T00:00',
  spinup: '0',
  corrected: false,
});

function singleResult(): SingleResult {
  const request = singleRequest();
  return {
    scope: 'single',
    key: singleKey(request),
    request,
    rows: [{ name: 'Qle', n: 10, rmse: 2, mae: 1.5, bias: 0.5 }],
    evaluation: [{ name: 'Qle', available: true }],
    completedAt: 1,
  };
}

function comparisonResult(): ComparisonResult {
  const request = comparisonRequest();
  return {
    scope: 'multi',
    id: comparisonKey(request),
    request,
    rows: [
      {
        site: '/case/runs/a',
        comparison_obs: '/obs/a.nc',
        provenance: {
          case_name: 'CN-Cng',
          case_dir: '/case',
          status: 'succeeded',
          version: { id: 'official', revision: 2 },
        },
        name: 'Qle',
        units: 'W/m²',
        n: 10,
        rmse: 1.5,
        mae: 1,
        bias: 0.5,
        correlation: 0.9,
        nse: 0.7,
        kge: 0.6,
      },
    ],
    failures: [],
    completedAt: 2,
  };
}

test('single and comparison keys change with every evaluation condition', () => {
  const base = singleRequest();
  assert.equal(singleKey(base), singleKey({ ...base }));
  for (const patch of [
    { caseDir: '/other' },
    { versionId: 'soil' },
    { revision: 2 },
    { source: '/case/runs/b' },
    { obs: '/obs/b.nc' },
    { from: '2008-02-01T00:00' },
    { to: '2008-03-01T00:00' },
    { spinup: '8' },
    { corrected: true },
  ])
    assert.notEqual(singleKey(base), singleKey({ ...base, ...patch }), patch.toString());

  const comparison = comparisonRequest();
  assert.equal(comparisonKey(comparison), comparisonKey({ ...comparison }));
  assert.notEqual(
    comparisonKey(comparison),
    comparisonKey({ ...comparison, entries: [...comparison.entries].reverse() }),
    'reordering comparison rows changes the export order and is a condition change',
  );
  assert.notEqual(comparisonKey(comparison), comparisonKey({ ...comparison, spinup: '8' }));
});

test('fresh and exportGate separate no result, stale conditions and a ready snapshot', () => {
  const result = singleResult();
  assert.equal(fresh(result, result.key), true);
  assert.equal(fresh(result, 'other'), false);
  assert.equal(fresh(null, result.key), false);
  assert.deepEqual(exportGate(result, result.key), { ok: true });
  assert.deepEqual(exportGate(result, 'other'), {
    ok: false,
    reason: 'conditions-changed',
  });
  assert.deepEqual(exportGate(null, result.key), { ok: false, reason: 'none' });
  const empty = { ...result, rows: [] };
  assert.deepEqual(exportGate(empty, empty.key), { ok: false, reason: 'none' });

  const comparison = comparisonResult();
  assert.equal(fresh(comparison, comparison.id), true);
  assert.equal(
    fresh(comparison, result.key),
    false,
    'a single result cannot satisfy a comparison key',
  );
  assert.deepEqual(exportGate(comparison, comparison.id), { ok: true });
  assert.deepEqual(exportGate(comparison, 'other'), {
    ok: false,
    reason: 'conditions-changed',
  });
});

test('comparison csv takes its conditions and observation from the result snapshot', () => {
  const result = comparisonResult();
  const csv = comparisonCsv(result);
  const lines = csv.split('\n');
  assert.equal(lines[0], comparisonCsvHeader.join(','));
  assert.equal(
    lines[0],
    'site,run_case,status,version,observation,from,to,spinup,corrected,variable,units,n,rmse,mae,bias,r2,r,nse,kge',
  );
  const row = lines[1];
  assert.match(row, /CN-Cng · a/);
  assert.match(row, /\/case\/runs\/a/);
  assert.match(row, /succeeded/);
  assert.match(row, /official@2/);
  assert.match(row, /\/obs\/a\.nc/);
  assert.match(row, /2008-01-01T00:00/);
  assert.match(row, /2008-01-02T00:00/);
  assert.match(row, /Qle/);
  assert.match(row, /1\.5/);

  // 条件列必须来自 result.request：同一份行、换一份请求就换一份条件，不受"当前界面"影响。
  const later = comparisonResult();
  later.request = { ...later.request, spinup: '8', corrected: true };
  later.id = comparisonKey(later.request);
  const laterCsv = comparisonCsv(later).split('\n')[1];
  assert.match(laterCsv, /,8,1,/, 'spinup and corrected come from the snapshot');
});

test('comparison csv keeps row order and falls back to the entry observation', () => {
  const result = comparisonResult();
  result.request.entries.push({ runCase: '/other/runs/b', obs: '/obs/b.nc' });
  result.id = comparisonKey(result.request);
  result.rows.push({
    site: '/other/runs/b',
    name: 'Qh',
    n: 3,
    rmse: 2,
  });
  const lines = comparisonCsv(result).split('\n');
  assert.equal(lines.length, 3);
  assert.match(lines[1], /\/case\/runs\/a/);
  assert.match(lines[2], /\/other\/runs\/b/);
  assert.match(lines[2], /\/obs\/b\.nc/, 'observation falls back to the snapshot entry');
});

test('comparison failures are recorded beside the rows and never exported as metric rows', () => {
  const result = comparisonResult();
  result.failures = [
    { runCase: '/other/runs/b', obs: '/obs/b.nc', error: 'history missing' },
    {
      runCase: '/third/runs/c',
      obs: '/obs/c.nc',
      error: 'cancelled before it started',
      cancelled: true,
    },
  ];
  const csv = comparisonCsv(result);
  assert.equal(csv.split('\n').length, 2, 'a failed run has no metric row to export');
  assert.ok(!csv.includes('history missing'));
  assert.ok(!csv.includes('/other/runs/b'), 'request entries do not fabricate rows');
  assert.equal(result.failures.length, 2, 'failures stay on the snapshot for the page to list');
});
