import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { defaultSettings, historySettingsOf, withHistoryPath, type Settings } from './model';

/**
 * U01“仓库数据”入口的页面级回归：从 Settings.tsx 抽出真实的状态读取/逐仓库同步/路径保存回调，
 * 用记录请求的 invoke 执行，断言请求参数、逐步进度、部分失败汇总、取消语义与 busy/错误。
 */
const settingsSource = readFileSync(new URL('./Settings.tsx', import.meta.url), 'utf8');
const source = ts.createSourceFile(
  'Settings.tsx',
  settingsSource,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
function functionBody(name: string) {
  let body: string | undefined;
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, `Settings.tsx defines ${name}`);
  return body!;
}
function effectsContaining(marker: string) {
  const found: { body: string; deps: string }[] = [];
  const visit = (node: ts.Node) => {
    if (ts.isCallExpression(node) && node.expression.getText(source) === 'useEffect') {
      const body = node.arguments[0].getText(source);
      if (body.includes(marker))
        found.push({ body, deps: node.arguments[1]?.getText(source) ?? '' });
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return found;
}
function compile<T>(name: string, context: Record<string, unknown>): T {
  const body = functionBody(name);
  const js = ts.transpileModule(`const run = (${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return run;`)(
    ...Object.values(context),
  ) as T;
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

type Request = { command: string; args: Record<string, unknown> };
type Progress = { done: number; total: number; current: string; offline: boolean };

const statusPayload = {
  history_root: '/app/data/history',
  repos: {
    desktop: {
      identity: { state: 'verified' },
      last_successful_sync: { ref: 'main', sha: '0123456' },
    },
    colm_upstream: { identity: { state: 'missing' } },
  },
};
const syncResult = (repository: string) => ({
  history_root: '/app/data/history',
  actions: { [repository]: repository === 'desktop' ? 'fetched' : 'cloned' },
  repos: statusPayload.repos,
});

function historyHarness(
  options: {
    settings?: Settings;
    respond?: (command: string, args: Record<string, unknown>) => unknown;
    saveFails?: Error;
  } = {},
) {
  const saved = options.settings ?? defaultSettings;
  const ui = {
    busy: [] as boolean[],
    error: '',
    status: { history_root: '/old/root' } as unknown,
    result: null as unknown,
    draft: null as unknown,
    progress: [] as (Progress | null)[],
    outcome: null as string | null,
    failures: [] as { repository: string; message: string }[],
  };
  const requests: Request[] = [];
  const syncCancelled = { current: false };
  const invoke = async (command: string, args: Record<string, unknown> = {}) => {
    requests.push({ command, args });
    return options.respond ? options.respond(command, args) : undefined;
  };
  const context = {
    hasBackend: () => true,
    invoke,
    settings: saved,
    // 与真实页面一致：onSave 是 AssistantPanel 的 save()，最终会 invoke assistant_save_settings。
    onSave: async (next: Settings) => {
      requests.push({ command: 'assistant_save_settings', args: { settings: next } });
      Object.assign(ui, { savedNext: next });
      if (options.saveFails) throw options.saveFails;
    },
    setHistoryBusy: (value: boolean) => {
      ui.busy.push(value);
    },
    setHistoryError: (value: string) => {
      ui.error = value;
    },
    setHistoryStatus: (value: unknown) => {
      ui.status = value;
    },
    setHistoryResult: (value: unknown) => {
      ui.result = value;
    },
    setSyncProgress: (value: Progress | null) => {
      ui.progress.push(value);
    },
    setSyncOutcome: (value: string | null) => {
      ui.outcome = value;
    },
    setSyncFailures: (value: { repository: string; message: string }[]) => {
      ui.failures = value;
    },
    setDraft: (update: unknown) => {
      ui.draft = typeof update === 'function' ? (update as (old: unknown) => unknown)({}) : update;
    },
    settingsError: (error: unknown) => String(error).replace(/^Error: /, ''),
    withHistoryPath,
    syncCancelled,
    syncRunning: { current: false },
    syncProgress: null,
  };
  // saveHistoryPath 会调用 loadHistoryStatus、syncHistory 会调用 loadHistoryStatus：
  // 抽出来的函数保持同样的调用链，测试才是在验证真实刷新/失败行为。
  const loadHistoryStatus = compile<(options?: { quiet?: boolean }) => Promise<unknown>>(
    'loadHistoryStatus',
    context,
  );
  const saveHistoryPath = compile<(repository: string, path: string | null) => Promise<void>>(
    'saveHistoryPath',
    { ...context, loadHistoryStatus },
  );
  return {
    ui,
    requests,
    savedNext: () => (ui as { savedNext?: Settings }).savedNext,
    loadHistoryStatus,
    cancelHistorySync: compile<() => Promise<void>>('cancelHistorySync', context),
    syncHistory: compile<(offline: boolean) => Promise<unknown>>('syncHistory', {
      ...context,
      loadHistoryStatus,
    }),
    saveHistoryPath,
    pickHistoryRepository: compile<(repository: string) => Promise<void>>('pickHistoryRepository', {
      ...context,
      saveHistoryPath,
      historyBusy: false,
    }),
  };
}

test('opening settings reads repository status only — never syncs', async () => {
  const effects = effectsContaining('loadHistoryStatus');
  assert.equal(effects.length, 1, '设置对话框打开时有一个只读 status effect');
  assert.equal(effects[0].deps, '[]', '只在挂载（打开设置）时读一次');
  assert.ok(!effects[0].body.includes('syncHistory'), '只读打开绝不自动触发同步');

  const harness = historyHarness({ respond: () => statusPayload });
  await harness.loadHistoryStatus();
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['assistant_history_status'],
  );
  assert.deepEqual(harness.requests[0].args, {});
  assert.deepEqual(harness.ui.status, statusPayload);
  assert.equal(harness.ui.error, '');
  assert.deepEqual(harness.ui.busy, [true, false], '读取期间 busy，结束恢复');
});

test('a failed status read keeps the previous rows and shows the error', async () => {
  const harness = historyHarness({
    respond: () => {
      throw new Error('colm-cli history-repos failed');
    },
  });
  const before = harness.ui.status;
  await harness.loadHistoryStatus();
  assert.equal(harness.ui.status, before, '失败不能清空上一次的仓库状态');
  assert.match(harness.ui.error, /history-repos failed/);
  assert.equal(harness.ui.busy.at(-1), false, '失败也要退出 busy');
});

test('online update syncs desktop then colm_upstream, refreshing status between steps', async () => {
  const harness = historyHarness({
    respond: (command, args) => {
      if (command === 'assistant_history_sync') return syncResult(String(args.repository));
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  await harness.syncHistory(false);
  // 顺序：desktop → status → colm_upstream → status → 结束再刷一次。
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    [
      'assistant_history_sync',
      'assistant_history_status',
      'assistant_history_sync',
      'assistant_history_status',
      'assistant_history_status',
    ],
  );
  const syncs = harness.requests.filter((request) => request.command === 'assistant_history_sync');
  assert.deepEqual(
    syncs.map((request) => request.args),
    [
      { repository: 'desktop', offline: false },
      { repository: 'colm_upstream', offline: false },
    ],
  );
  // 进度：Desktop 1/2 → CoLM 官方 2/2 → 结束清空。
  assert.deepEqual(harness.ui.progress, [
    { done: 0, total: 2, current: 'desktop', offline: false },
    { done: 1, total: 2, current: 'colm_upstream', offline: false },
    null,
  ]);
  assert.equal(harness.ui.outcome, 'success');
  assert.deepEqual(harness.ui.failures, []);
  assert.deepEqual(harness.ui.result, {
    ...statusPayload,
    actions: { desktop: 'fetched', colm_upstream: 'cloned' },
  });
  assert.equal(harness.ui.busy[0], true);
  assert.equal(harness.ui.busy.at(-1), false, '结束回到非 busy');
});

test('a failed first repository still syncs the second and reports partial failure', async () => {
  const harness = historyHarness({
    respond: (command, args) => {
      if (command === 'assistant_history_sync') {
        if (args.repository === 'desktop') throw new Error('git fetch origin failed');
        return syncResult(String(args.repository));
      }
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  await harness.syncHistory(false);
  const syncs = harness.requests.filter((request) => request.command === 'assistant_history_sync');
  assert.deepEqual(
    syncs.map((request) => request.args.repository),
    ['desktop', 'colm_upstream'],
    '一个仓库失败不阻塞另一个',
  );
  assert.equal(harness.ui.outcome, 'partial');
  assert.deepEqual(harness.ui.failures, [
    { repository: 'desktop', message: 'git fetch origin failed' },
  ]);
  // 失败步骤之后也刷新了状态（显示 last_attempt）。
  const order = harness.requests.map((request) => request.command);
  assert.ok(
    order.indexOf('assistant_history_status') < order.lastIndexOf('assistant_history_sync'),
    '两步之间刷新状态',
  );
  assert.deepEqual(harness.ui.result, {
    ...statusPayload,
    actions: { desktop: 'error', colm_upstream: 'cloned' },
  });
  assert.equal(harness.ui.busy.at(-1), false);
});

test('offline update runs both repositories with offline=true', async () => {
  const harness = historyHarness({
    respond: (command, args) => {
      if (command === 'assistant_history_sync') return syncResult(String(args.repository));
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  await harness.syncHistory(true);
  const syncs = harness.requests.filter((request) => request.command === 'assistant_history_sync');
  assert.deepEqual(
    syncs.map((request) => request.args),
    [
      { repository: 'desktop', offline: true },
      { repository: 'colm_upstream', offline: true },
    ],
  );
  assert.equal(harness.ui.outcome, 'success');
});

test('failure on both repositories keeps the last good status and reports all failed', async () => {
  const harness = historyHarness({
    respond: () => {
      throw new Error('network unreachable');
    },
  });
  const before = harness.ui.status;
  await harness.syncHistory(false);
  assert.equal(harness.ui.status, before, '状态读取也失败时保留上次好行');
  assert.equal(harness.ui.outcome, 'failed');
  assert.deepEqual(
    harness.ui.failures.map((failure) => failure.repository),
    ['desktop', 'colm_upstream'],
  );
  assert.equal(harness.ui.busy.at(-1), false);
});

test('stopping during the first repository never starts the second', async () => {
  let release!: (value: unknown) => void;
  const gate = new Promise<unknown>((resolve) => {
    release = resolve;
  });
  const harness = historyHarness({
    respond: (command, args) => {
      if (command === 'assistant_history_sync' && args.repository === 'desktop') return gate;
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  const running = harness.syncHistory(false);
  await settle();
  assert.deepEqual(harness.ui.progress.at(-1), {
    done: 0,
    total: 2,
    current: 'desktop',
    offline: false,
  });
  await harness.cancelHistorySync();
  assert.ok(
    harness.requests.some((request) => request.command === 'assistant_history_cancel'),
    '停止按钮调用 assistant_history_cancel',
  );
  release(syncResult('desktop'));
  await running;
  const syncs = harness.requests.filter((request) => request.command === 'assistant_history_sync');
  assert.deepEqual(
    syncs.map((request) => request.args.repository),
    ['desktop'],
    '取消后不再发起第二个仓库',
  );
  assert.equal(harness.ui.outcome, 'cancelled');
  assert.equal(harness.ui.progress.at(-1), null);
  assert.equal(harness.ui.busy.at(-1), false);
});

test('sync progress and outcome copy is bilingual', () => {
  const progressText = compile<(progress: Progress | null) => string>('historySyncProgressText', {
    t: (zh: string, en: string) => en,
    historySyncRepositoryName: (repository: string) =>
      repository === 'desktop' ? 'Desktop' : 'CoLM upstream',
  });
  assert.equal(
    progressText({ done: 0, total: 2, current: 'desktop', offline: false }),
    'Syncing Desktop (1/2)…',
  );
  assert.equal(
    progressText({ done: 1, total: 2, current: 'colm_upstream', offline: false }),
    'Syncing CoLM upstream (2/2)…',
  );
  assert.equal(
    progressText({ done: 0, total: 2, current: 'desktop', offline: true }),
    'Offline update Desktop (1/2)…',
  );
  const zhProgress = compile<(progress: Progress | null) => string>('historySyncProgressText', {
    t: (zh: string) => zh,
    historySyncRepositoryName: (repository: string) =>
      repository === 'desktop' ? 'Desktop' : 'CoLM 官方',
  });
  assert.equal(
    zhProgress({ done: 0, total: 2, current: 'desktop', offline: false }),
    '正在同步 Desktop（1/2）…',
  );
  assert.equal(
    zhProgress({ done: 1, total: 2, current: 'colm_upstream', offline: false }),
    '正在同步 CoLM 官方（2/2）…',
  );

  const outcomeText = compile<(outcome: string | null, failures: unknown[]) => string>(
    'historySyncOutcomeText',
    { t: (zh: string, en: string) => en },
  );
  assert.match(outcomeText('success', []), /both repositories were updated/);
  assert.match(outcomeText('partial', [{ repository: 'desktop' }]), /Partially failed/);
  assert.match(outcomeText('failed', []), /All failed/);
  assert.match(outcomeText('cancelled', []), /remaining repository steps were not started/);
  const zhOutcome = compile<(outcome: string | null, failures: unknown[]) => string>(
    'historySyncOutcomeText',
    { t: (zh: string) => zh },
  );
  assert.match(zhOutcome('partial', [{ repository: 'desktop' }]), /部分失败/);
  assert.match(zhOutcome('cancelled', []), /已停止同步/);
});

test('picking an existing repository saves the path and re-reads status', async () => {
  const harness = historyHarness({
    respond: (command) => {
      if (command === 'pick_folder') return '/repos/desktop';
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  await harness.pickHistoryRepository('desktop');
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['pick_folder', 'assistant_save_settings', 'assistant_history_status'],
  );
  assert.deepEqual(harness.requests[0].args, { key: 'assistant-history-desktop' });
  const saved = harness.savedNext()!;
  assert.equal(saved.history.desktop_path, '/repos/desktop');
  // 保存以已保存设置为基底（不带对话框里的未保存草稿），其余字段原样保留。
  assert.equal(saved.model, defaultSettings.model);
  assert.equal(saved.history.scope, 'auto');
});

test('cancelling the folder picker saves nothing', async () => {
  const harness = historyHarness({ respond: () => null });
  await harness.pickHistoryRepository('colm_upstream');
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['pick_folder'],
  );
  assert.equal(harness.savedNext(), undefined);
});

test('clearing one local path keeps the other repository path', async () => {
  const settings = withHistoryPath(
    withHistoryPath(defaultSettings, 'desktop', '/repos/desktop'),
    'colm_upstream',
    '/repos/upstream',
  );
  const harness = historyHarness({
    settings,
    respond: (command) => {
      if (command === 'assistant_history_status') return statusPayload;
    },
  });
  await harness.saveHistoryPath('colm_upstream', null);
  const saved = harness.savedNext()!;
  assert.deepEqual(historySettingsOf(saved), {
    scope: 'auto',
    desktop_path: '/repos/desktop',
    upstream_path: null,
  });
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['assistant_save_settings', 'assistant_history_status'],
  );
});

test('a failed path save keeps the path out of status reloads and reports the error', async () => {
  const harness = historyHarness({ saveFails: new Error('disk full') });
  await harness.saveHistoryPath('desktop', '/repos/desktop');
  assert.deepEqual(
    harness.requests.map((request) => request.command),
    ['assistant_save_settings'],
    '保存失败不再读状态',
  );
  assert.match(harness.ui.error, /disk full/);
  assert.equal(harness.ui.busy.at(-1), false);
});

test('the settings panel renders both rows, separate times, progress and busy/error states', () => {
  assert.match(settingsSource, /assistant_history_status/);
  assert.match(settingsSource, /assistant_history_sync/);
  assert.match(settingsSource, /assistant_history_cancel/, '在跑同步时有停止按钮');
  assert.match(settingsSource, /historyRepoLines\(repo, state\.language\)/);
  assert.match(settingsSource, /lines\.lastSync/, '成功同步时间单独一行');
  assert.match(settingsSource, /lines\.checkedAt/, '检查时间单独一行');
  assert.match(settingsSource, /lines\.preparation/, 'partial/未准备时的说明');
  assert.match(settingsSource, /historyStatus\?\.history_root/, '数据目录只读展示');
  // 逐仓库进度 + 部分失败汇总 + 取消。
  assert.match(settingsSource, /historySyncProgressText\(syncProgress\)/);
  assert.match(settingsSource, /historySyncOutcomeText\(syncOutcome, syncFailures\)/);
  assert.match(settingsSource, /syncFailures\.length > 0/);
  assert.match(settingsSource, /cancelHistorySync\(\)/);
  assert.match(settingsSource, /正在同步/);
  assert.match(settingsSource, /离线更新/);
  assert.match(
    settingsSource,
    /disabled=\{disabled \|\| historyBusy \|\| syncProgress !== null\}/,
    '忙时按钮禁用',
  );
});
