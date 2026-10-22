import test from 'node:test';
import assert from 'node:assert/strict';
import {
  replayVerificationText,
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
  assert.equal(historyObject({ ...operation, case_path: null, version_id: null }), '#op-123');
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

test('replay notices always state that forcing contents were not verified', () => {
  const text = (other?: string) =>
    replayVerificationText(other ? { verification: { other_inputs: other } } : {}, false);
  for (const other of ['verified', 'server', 'not-recorded', undefined])
    assert.match(text(other), /强迫场内容未核对/);
  assert.match(text('verified'), /已核对一致/);
  assert.match(text('not-recorded'), /也未核对/);
  assert.match(replayVerificationText({}, true), /Forcing contents were not verified/);
});

test('history cards name what was done, to what, with which model, where and for how long', async () => {
  const {
    historyActionLabel,
    historyDurationLabel,
    historyInternal,
    historySubject,
    historyVersionLabel,
  } = await import('./history');
  type Operation = import('../../lib/types').HistoryOperation;
  const base = {
    operation_id: '7850f3ff9555c89a3f313e5d499356c8ada1b72c3b8590a9c54cb9ee5541639f',
    kind: 'study',
    case_path: null,
    case_id: null,
    version_id: null,
    revision: null,
    host: null,
    detail: {},
    command: null,
  } as unknown as Operation;
  // Recorded subjects win; older records fall back to the command's object or --out/--name.
  assert.deepEqual(
    historySubject({
      ...base,
      detail: { subject: { kind: 'case', name: 'AT-Neu', project: 'PLUMBER2' } },
    }),
    { name: 'AT-Neu', project: 'PLUMBER2', kind: 'case' },
  );
  assert.deepEqual(
    historySubject({ ...base, detail: { action: 'study-run', object: '/work/studies/lai-fit' } }),
    { name: 'lai-fit', project: 'studies' },
  );
  assert.deepEqual(
    historySubject({
      ...base,
      kind: 'new',
      detail: { action: 'new', args: { '--out': '/p/heihe/basin', '--name': 'Heihe' } },
    }),
    { name: 'Heihe', project: 'heihe' },
  );
  assert.deepEqual(
    historySubject({
      ...base,
      command: { program: 'colm-cli', argv: ['ws-create', '--from', '/x'], cwd: null },
    }),
    { name: '#7850f3ff' },
  );
  assert.deepEqual(historySubject({ ...base, host: 'tms17l' }), { name: 'tms17l', kind: 'server' });

  assert.equal(historyActionLabel({ ...base, detail: { action: 'study-run' } }), '运行研究');
  assert.equal(historyActionLabel({ ...base, detail: { action: 'future-action' } }), '研究');
  assert.equal(historyActionLabel(base), '研究');

  assert.equal(historyVersionLabel({ version_id: 'official', revision: '64' }), '官方版 r64');
  assert.equal(
    historyVersionLabel({ version_id: 'model-1', revision: '2' }, { 'model-1': '雪方案' }),
    '雪方案 r2',
  );
  assert.equal(
    historyVersionLabel({ version_id: 'official', revision: null }, {}, true),
    'Official',
  );
  assert.equal(historyVersionLabel({ version_id: null, revision: null }), null);

  assert.equal(historyDurationLabel(158), '不到 1 秒');
  assert.equal(historyDurationLabel(42_000), '42 秒');
  assert.equal(historyDurationLabel(5 * 60_000), '5 分钟');
  assert.equal(historyDurationLabel(3_900_000, true), '1 h 5 min');
  assert.equal(historyDurationLabel(null), null);

  assert.equal(historyInternal({ ...base, kind: 'case-config-save' }), true);
  assert.equal(
    historyInternal({ ...base, kind: 'server', detail: { action: 'remote-probe' } }),
    true,
  );
  assert.equal(historyInternal({ ...base, kind: 'run', detail: { action: 'run' } }), false);
});

test('project timelines batch consecutive actions and summarize the latest result per case', async () => {
  const { projectTimeline, projectSummary, statusCountsLabel } = await import('./history');
  type Operation = import('../../lib/types').HistoryOperation;
  const op = (id: string, action: string, status: string, time: number, site = id) =>
    ({
      operation_id: id,
      kind: 'run',
      phase: 'finished',
      status,
      needs_reconciliation: false,
      time_ms: time,
      started_ms: time,
      case_path: `/p/${site}`,
      detail: { action },
    }) as unknown as Operation;
  const members = [
    op('a', 'remote-run', 'succeeded', 10 * 60_000),
    op('b', 'remote-run', 'failed', 9 * 60_000),
    op('c', 'remote-run', 'succeeded', 60 * 60_000 + 9 * 60_000, 'a'),
    op('d', 'new', 'succeeded', 0),
  ];
  const rows = projectTimeline(members);
  assert.deepEqual(
    rows.map((row) => [row.action, row.members.length]),
    [
      ['remote-run', 1],
      ['remote-run', 2],
      ['new', 1],
    ],
    'an hour apart is another batch',
  );
  assert.deepEqual(rows[1].counts, { succeeded: 1, failed: 1, running: 0, pending: 0 });
  const summary = projectSummary({ path: '/p', name: 'p', members });
  assert.equal(summary.cases, 3);
  assert.deepEqual(summary.counts, { succeeded: 2, failed: 1, running: 0, pending: 0 });
  assert.equal(statusCountsLabel(summary.counts), '成功 2 · 失败 1');
  assert.equal(
    statusCountsLabel({ succeeded: 0, failed: 0, running: 1, pending: 2 }, true),
    '1 running · 2 to reconcile',
  );
});

test('a record awaiting reconciliation says why and how it becomes completed', async () => {
  const { pendingReason, hasNewerRun } = await import('./history');
  const op = (fields: Record<string, unknown>) =>
    ({
      operation_id: 'op',
      kind: 'run',
      phase: 'status',
      status: 'running',
      location: 'remote',
      case_path: '/p/AT-Neu',
      time_ms: null,
      started_ms: 1,
      needs_reconciliation: true,
      ...fields,
    }) as unknown as Parameters<typeof pendingReason>[0];
  assert.match(pendingReason(op({})), /上次向服务器查到：运行中.*打开这个项目的运行页/);
  assert.match(
    pendingReason(op({ status: 'cancel_requested', kind: 'remote-cancel' })),
    /已请求取消/,
  );
  assert.match(pendingReason(op({ phase: 'imported', status: 'needs_reconciliation' })), /导入/);
  assert.match(pendingReason(op({ location: 'local' })), /软件被关闭/);
  assert.match(pendingReason(op({ status: 'needs_reconciliation' }), true), /could not confirm/);
  const older = op({});
  const newer = op({ operation_id: 'op-2', started_ms: 5 });
  assert.ok(hasNewerRun(older, [older, newer]));
  assert.ok(!hasNewerRun(newer, [older, newer]));
  assert.ok(
    !hasNewerRun(older, [older, op({ operation_id: 'op-3', started_ms: 5, case_path: '/p/b' })]),
  );
  assert.match(pendingReason(older, false, true), /只核对最新的那次/);
});

// 台账记下的每个命令（colm-cli history_cmd::action_kind）都要有中文名，否则历史页显示成命令名
//（2026-10-22 实测"研究结果写进草稿"显示成 model-import-study）。
test('every command the ledger records has a history label', async () => {
  const { readFileSync } = await import('node:fs');
  const { historyKindLabel } = await import('./history');
  const source = readFileSync(
    new URL('../../../../crates/colm-cli/src/history_cmd.rs', import.meta.url),
    'utf8',
  );
  const start = source.indexOf('fn action_kind');
  const body = source.slice(start, source.indexOf('})', start));
  const commands = [...body.matchAll(/"([a-z0-9-]+)"/g)].map((match) => match[1]!);
  assert.ok(commands.length > 50);
  for (const command of commands)
    assert.notEqual(historyKindLabel(command), command, `${command} has no label`);
});
