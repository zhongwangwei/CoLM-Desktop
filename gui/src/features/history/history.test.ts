import test from 'node:test';
import assert from 'node:assert/strict';
import {
  emptyFilter,
  historyFilter,
  canReplay,
  canDeleteHistory,
  historyDirectoryContains,
  historyStateAfterDirectoryDeletion,
  canCompare,
  historyPath,
  comparisonArgs,
  comparisonWinner,
  historySections,
  historyReferences,
  historyRecordExport,
  historyObject,
} from './history';
test('filters preserve pagination and reject reversed dates', () => {
  assert.deepEqual(historyFilter({ ...emptyFilter, casePath: '/a' }, 30), {
    offset: 30,
    limit: 30,
    casePath: '/a',
  });
  assert.throws(() => historyFilter({ ...emptyFilter, from: '2026-02-01', to: '2026-01-01' }));
});

test('replay and comparison controls require frozen run metadata and reconciled outcomes', () => {
  const run = {
    kind: 'run',
    version_id: 'v',
    revision: '1',
    status: 'succeeded',
    needs_reconciliation: false,
  } as import('../../lib/types').HistoryOperation;
  assert.equal(canReplay(run), true);
  assert.equal(canReplay({ ...run, kind: 'remote-run' }), true);
  assert.equal(canCompare({ ...run, kind: 'remote-run' }), true);
  assert.equal(canCompare(run), true);
  assert.equal(canReplay({ ...run, kind: 'model-create' }), false);
  assert.equal(canReplay({ ...run, version_id: null }), false);
  assert.equal(canReplay({ ...run, needs_reconciliation: true }), false);
  assert.equal(canCompare({ ...run, status: 'failed' }), false);
});

test('history paths use recorded copies and never local/remote location labels', () => {
  const operation = {
    case_path: '/case/source',
    location: 'local',
    detail: {},
  } as import('../../lib/types').HistoryOperation;
  const detail = {
    operation,
    related: [],
    events: [{ detail: { model_run: { run_case: '/runs/frozen' } } }],
  };
  assert.equal(historyPath(detail), '/runs/frozen');
  assert.equal(historyPath({ ...detail, events: [] }), '/case/source');
  assert.equal(
    historyPath({ ...detail, events: [], operation: { ...operation, case_path: null } }),
    null,
  );
});
test('comparison uses UTC seconds and explicit skipped-record count', () => {
  const input = {
    obs: '/obs.nc',
    variable: 'LE',
    from: '2026-01-01T00:00',
    to: '2026-01-02T00:00',
    spinup: '2',
    corrected: false,
  };
  assert.equal(comparisonArgs('a', 'b', input).from, 1767225600);
  assert.equal(comparisonArgs('a', 'b', input).spinup, 2);
  assert.throws(() => comparisonArgs('a', 'b', { ...input, to: input.from }));
  assert.throws(() => comparisonArgs('a', 'b', { ...input, variable: '' }));
});
test('comparison cannot rank mismatched or unverified pairs', () => {
  const result = {
    same_design: true,
    better: 'a',
    evaluation: { obs_sha256: 'a'.repeat(64) },
    variables: [{ same_pairs: true, better: 'a' }],
  };
  assert.equal(comparisonWinner(result), 'a');
  assert.equal(comparisonWinner({ ...result, same_design: false }), null);
  assert.equal(
    comparisonWinner({ ...result, variables: [{ same_pairs: false, better: 'a' }] }),
    null,
  );
  assert.equal(comparisonWinner({ ...result, evaluation: {} }), null);
});
test('organized history uses recorded identities, preserves zero exits and large evidence references', () => {
  const operation = {
    operation_id: 'op-a',
    kind: 'run',
    status: 'succeeded',
    phase: 'end',
    started_ms: 1000,
    finished_ms: 2000,
    duration_ms: 1000,
    case_path: '/source',
    version_id: 'v',
    revision: '2',
    source_reference: '/import/record.json',
    detail: {
      status: { exit_code: 0 },
      provenance: {
        kind: 'provenance-reference',
        path: '/out/produced-by.json',
        sha256: 'a'.repeat(64),
        bytes: 4000000,
        summary: {
          engine: 'rust',
          ranks: 4,
          full_input_identity: { complete: false, data_sha256: 'b'.repeat(64) },
        },
      },
    },
  } as unknown as import('../../lib/types').HistoryOperation;
  const detail = {
    operation,
    related: [],
    events: [
      {
        detail: {
          model_run: {
            run_case: '/runs/frozen',
            code: { commit: 'commit-a', source_sha256: 'c'.repeat(64) },
            overlay_sha256: 'd'.repeat(64),
            inputs: { forcing: 'e'.repeat(64) },
          },
        },
      },
    ],
  };
  const sections = historySections(detail),
    row = (label: string) => sections.flatMap((s) => s.rows).find((r) => r.en === label)?.value;
  assert.equal(row('Exit code'), '0');
  assert.equal(row('Frozen run copy'), '/runs/frozen');
  assert.equal(row('Commit'), 'commit-a');
  assert.equal(row('Input identity complete'), 'false');
  assert.equal(row('Provenance file SHA-256'), 'a'.repeat(64));
  assert.equal(row('Duration (seconds)'), '1');
  assert.deepEqual(
    historyReferences(detail).map((r) => r.path),
    ['/import/record.json', '/out/produced-by.json'],
  );
  const exported = historyRecordExport(detail);
  assert.equal(exported.name, 'colm-history-op-a.json');
  assert.deepEqual(JSON.parse(exported.content), detail);
});
test('history does not invent missing metadata or render an unbounded manifest', () => {
  const operation = {
    operation_id: '../bad/id',
    detail: {
      model_run: {
        inputs: Object.fromEntries(
          Array.from({ length: 1000 }, (_, i) => [`file-${i}`, 'f'.repeat(64)]),
        ),
      },
      provenance: {
        kind: 'provenance-reference',
        path: 'relative.json',
        summary: { full_input_identity: { files: { huge: 'secret'.repeat(100000) } } },
      },
    },
  } as unknown as import('../../lib/types').HistoryOperation;
  const detail = { operation, events: [], related: [] };
  const rows = historySections(detail).flatMap((s) => s.rows);
  assert.equal(rows.find((r) => r.en === 'Input hash entries')?.value, '1000');
  assert.equal(rows.find((r) => r.en === 'Commit')?.value, null);
  assert.ok(rows.length < 100);
  assert.equal(historyReferences(detail).length, 0);
  assert.ok(!historyRecordExport(detail).name.includes('/'));
});
test('history keeps every command argument and its boundaries for reproducibility', () => {
  const argv = Array.from({ length: 40 }, (_, i) =>
    i === 30 ? 'a space and "quote"' : `argument-${i}`,
  );
  const detail = {
    operation: {
      detail: {},
      command: { program: '/bin/model', argv, cwd: '/runs' },
    } as import('../../lib/types').HistoryOperation,
    events: [],
    related: [],
  };
  const command = historySections(detail).find((s) => s.en === 'Command')!;
  const recorded = command.rows.find((r) => r.en.startsWith('Complete arguments'))!.value!;
  assert.deepEqual(
    recorded.split('\n').map((line) => JSON.parse(line)),
    argv,
  );
});

test('embedded case history cannot broaden its scope through an edited filter', async () => {
  const { scopedHistoryFilter } = await import('./history');
  const edited = { ...emptyFilter, casePath: '/another', query: 'failure' };
  assert.deepEqual(historyFilter(scopedHistoryFilter(edited, '/selected')), {
    offset: 0,
    limit: 30,
    casePath: '/selected',
    query: 'failure',
  });
  assert.equal(scopedHistoryFilter(edited).casePath, '/another');
  assert.equal(edited.casePath, '/another');
});
test('history presentation uses real recorded names and preserves unknown vocabulary', async () => {
  const { historyObject, historyKindLabel, historyStatusLabel } = await import('./history');
  const operation = {
    operation_id: 'op-123',
    case_path: 'C:\\runs\\CN-Cng\\',
    case_id: null,
    version_id: 'v1',
  } as import('../../lib/types').HistoryOperation;
  assert.equal(historyObject(operation), 'CN-Cng');
  assert.equal(historyObject({ ...operation, case_path: null }), 'v1');
  assert.equal(historyObject({ ...operation, case_path: null, version_id: null }), 'op-123');
  assert.equal(historyKindLabel('run'), '运行');
  assert.equal(historyStatusLabel('succeeded'), '已完成');
  assert.equal(historyStatusLabel('completed_with_failures'), '部分失败');
  assert.equal(historyStatusLabel('completed_with_failures', true), 'Partially failed');
  assert.equal(historyKindLabel('future-kind'), 'future-kind');
  assert.equal(historyStatusLabel('future-status'), 'future-status');
});

test('deletion accepts terminal records only and rejects reconciliation regardless of status', () => {
  const operation = {
    status: 'succeeded',
    needs_reconciliation: false,
  } as import('../../lib/types').HistoryOperation;
  for (const status of ['succeeded', 'failed', 'completed', 'completed_with_failures', 'cancelled'])
    assert.equal(canDeleteHistory({ ...operation, status }), true);
  for (const status of ['running', 'pending', 'unknown', 'interrupted', ''])
    assert.equal(canDeleteHistory({ ...operation, status }), false);
  assert.equal(canDeleteHistory({ ...operation, needs_reconciliation: true }), false);
});

test('deleted directory matching handles Windows separators and case without broadening Unix paths', () => {
  assert.equal(historyDirectoryContains('C:\\Runs\\Job\\', 'c:/runs/job'), true);
  assert.equal(historyDirectoryContains('C:\\Runs\\Job', 'c:\\RUNS\\job\\case'), true);
  assert.equal(historyDirectoryContains('C:/Runs/Job', 'c:\\runs\\job\\restart'), true);
  assert.equal(historyDirectoryContains('C:\\Runs\\Job', 'C:\\Runs\\Job2'), false);
  assert.equal(
    historyDirectoryContains('\\\\Server\\Share\\Runs', '//server/share/runs/case'),
    true,
  );
  assert.equal(historyDirectoryContains('/Runs/Job', '/runs/job/case'), false);
  assert.equal(historyDirectoryContains('/Runs/Job/', '/Runs/Job/case'), true);
  assert.equal(historyDirectoryContains('/Runs/Job', '/Runs/Job2'), false);
  assert.equal(historyDirectoryContains('/Runs/Job', '/Runs/Job\\case'), false);
});

test('deleted Windows canonical paths match ordinary drive and UNC paths', () => {
  assert.equal(
    historyDirectoryContains(String.raw`\\?\C:\Runs\Job`, String.raw`c:\runs\job\case`),
    true,
  );
  assert.equal(
    historyDirectoryContains(
      String.raw`\\?\UNC\Server\Share\Runs`,
      String.raw`\\server\share\runs\case`,
    ),
    true,
  );
  assert.equal(historyDirectoryContains('C:/Runs/Job', String.raw`\\?\C:\Runs\Job\case`), true);
  assert.equal(historyDirectoryContains(String.raw`\\?\C:\Runs\Job`, 'C:/Runs/Job2'), false);
  assert.equal(
    historyDirectoryContains(String.raw`\\?\UNC\Server\Share\Runs`, '//server/share/runs2'),
    false,
  );
});

test('remote case deletion clears only matching host mirrors and their run state', () => {
  const cases = [
    {
      dir: '/mirrors/a',
      name: 'a',
      has_history: true,
      spatial: false,
      run_host: 'server-a',
      remote_directory: '/cases/site',
    },
    {
      dir: '/mirrors/b',
      name: 'b',
      has_history: true,
      spatial: false,
      run_host: 'server-b',
      remote_directory: '/cases/site',
    },
    { dir: '/cases/site', name: 'local', has_history: true, spatial: false },
  ];
  const state = {
    selectedCase: { entry: cases[0], dir: cases[0].dir } as import('../../lib/types').OpenedCase,
    cases,
    running: Object.fromEntries(
      cases.map((entry) => [
        entry.dir,
        {
          case: entry.dir,
          run_case: entry.dir + '/runs/1',
          status: 'succeeded',
          running: false,
          logs: [],
          stages: {},
        },
      ]),
    ),
  };
  const result = historyStateAfterDirectoryDeletion(state, '/cases/site', 'server-a');
  assert.equal(result.selectedCase, null);
  assert.deepEqual(result.cases, cases.slice(1));
  assert.deepEqual(Object.keys(result.running), ['/mirrors/b', '/cases/site']);
  const otherSelection = {
    ...state,
    selectedCase: { entry: cases[1], dir: cases[1].dir } as import('../../lib/types').OpenedCase,
  };
  assert.equal(
    historyStateAfterDirectoryDeletion(otherSelection, '/cases/site', 'server-a').selectedCase,
    otherSelection.selectedCase,
  );
});

test('group objects name the common project without enabling representative replay or comparison', () => {
  const member = {
    kind: 'run',
    status: 'succeeded',
    version_id: 'v',
    revision: '1',
    needs_reconciliation: false,
    case_path: '/projects/Test/AT-Neu',
  } as import('../../lib/types').HistoryOperation;
  const group = {
    ...member,
    run_group: {
      run_id: 'batch',
      members: [member, { ...member, case_path: '/projects/Test/CN-Cng' }],
    },
  };
  assert.equal(historyObject(group), 'Test');
  assert.equal(canReplay(group), false);
  assert.equal(canCompare(group), false);
  assert.equal(canDeleteHistory(group), true);
  group.run_group.members[1] = {
    ...member,
    case_path: '/different/CN-Cng',
    needs_reconciliation: true,
  };
  assert.equal(historyObject(group, true), 'Multi-site run');
  assert.equal(canDeleteHistory(group), false);
});
