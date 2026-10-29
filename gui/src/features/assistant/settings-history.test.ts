import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { defaultSettings, historySettingsOf, withHistoryPath, type Settings } from './model';

/**
 * U01“仓库数据”入口的页面级回归：从 Settings.tsx 抽出真实的状态读取/同步/路径保存回调，
 * 用记录请求的 invoke 执行，断言按钮参数、busy/错误、失败保留旧状态，以及挂载时只读 status。
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

type Request = { command: string; args: Record<string, unknown> };

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
  };
  const requests: Request[] = [];
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
      Object.assign(ui, { currentBusy: value });
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
    setDraft: (update: unknown) => {
      ui.draft = typeof update === 'function' ? (update as (old: unknown) => unknown)({}) : update;
    },
    settingsError: (error: unknown) => String(error).replace(/^Error: /, ''),
    withHistoryPath,
  };
  // 页面里 saveHistoryPath 会调用 loadHistoryStatus、pickHistoryRepository 会调用 saveHistoryPath：
  // 抽出来的函数保持同样的调用链，测试才是在验证真实失败/刷新行为。
  const loadHistoryStatus = compile<() => Promise<unknown>>('loadHistoryStatus', context);
  const saveHistoryPathContext = { ...context, loadHistoryStatus };
  const saveHistoryPath = compile<(repository: string, path: string | null) => Promise<void>>(
    'saveHistoryPath',
    saveHistoryPathContext,
  );
  return {
    ui,
    requests,
    savedNext: () => (ui as { savedNext?: Settings }).savedNext,
    loadHistoryStatus,
    syncHistory: compile<(repository: string, offline: boolean) => Promise<unknown>>(
      'syncHistory',
      context,
    ),
    saveHistoryPath,
    pickHistoryRepository: compile<(repository: string) => Promise<void>>('pickHistoryRepository', {
      ...context,
      saveHistoryPath,
      historyBusy: false,
    }),
  };
}

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

test('explicit sync sends repository and offline exactly as chosen', async () => {
  const harness = historyHarness({ respond: () => statusPayload });
  await harness.syncHistory('both', false);
  assert.deepEqual(harness.requests.at(-1), {
    command: 'assistant_history_sync',
    args: { repository: 'both', offline: false },
  });
  assert.deepEqual(harness.ui.status, statusPayload, '同步返回就是最新状态');
  assert.deepEqual(harness.ui.result, statusPayload);

  await harness.syncHistory('both', true);
  assert.deepEqual(harness.requests.at(-1), {
    command: 'assistant_history_sync',
    args: { repository: 'both', offline: true },
  });
  await harness.syncHistory('desktop', false);
  assert.deepEqual(harness.requests.at(-1)?.args, { repository: 'desktop', offline: false });
});

test('a failed sync keeps the last good status and reports the failure', async () => {
  const harness = historyHarness({
    respond: (command) => {
      if (command === 'assistant_history_sync') throw new Error('git fetch origin failed');
    },
  });
  const before = harness.ui.status;
  const result = await harness.syncHistory('both', false);
  assert.equal(result, null);
  assert.equal(harness.ui.status, before, '同步失败不能把仓库行清空');
  assert.equal(harness.ui.result, null);
  assert.match(harness.ui.error, /git fetch origin failed/);
  assert.equal(harness.ui.busy.at(-1), false);
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

test('the settings panel renders both rows, separate times and busy/error states', () => {
  assert.match(settingsSource, /assistant_history_status/);
  assert.match(settingsSource, /assistant_history_sync/);
  assert.match(settingsSource, /assistant_history_cancel/, '在跑同步时有停止按钮');
  assert.match(settingsSource, /historyRepoLines\(repo, state\.language\)/);
  assert.match(settingsSource, /lines\.lastSync/, '成功同步时间单独一行');
  assert.match(settingsSource, /lines\.checkedAt/, '检查时间单独一行');
  assert.match(settingsSource, /lines\.preparation/, 'partial/未准备时的说明');
  assert.match(settingsSource, /historyStatus\?\.history_root/, '数据目录只读展示');
  assert.match(settingsSource, /正在更新仓库数据/, '同步中 busy 文案');
  assert.match(settingsSource, /disabled=\{disabled \|\| historyBusy\}/, '忙时按钮禁用');
});
