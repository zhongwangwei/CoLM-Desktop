import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  rejectedBeforeTurn,
  appendDelta,
  apiEfforts,
  assistantModels,
  canonicalCommitUrl,
  canonicalRepositorySlug,
  selectAssistantModel,
  API_PROVIDERS,
  defaultHistorySettings,
  defaultSettings,
  formatHistoryTime,
  historyContextLabel,
  historyInput,
  historyEvidence,
  historyFileOpenPath,
  historyRepoLines,
  historyRepositoryLabel,
  historyToolKind,
  historyScopeHint,
  historyScopeLabel,
  historySettingsOf,
  historySyncSummary,
  localPathsInAction,
  localPathsInJsonText,
  managedNodeText,
  parseApiOptions,
  pendingReconciliation,
  safeHistoryRelativePath,
  rememberProfile,
  retrievalSettingsProblem,
  retrievalStateText,
  selectProvider,
  settingsForSave,
  shortSha,
  sourceRetentionFields,
  taskActionOutput,
  taskActionTitle,
  taskSnapshotOf,
  thinkSettings,
  thinkValue,
  withChoice,
  withContextWindowCleared,
  withHistoryPath,
  withHistoryScope,
  formatUnixSeconds,
  EXAMPLE_STARTERS,
  examplePrompt,
  providerLabel,
  egressTarget,
  type HistoryRepoStatus,
  type RetrievalSettings,
} from './model';
import { approvalSnapshot, matchesGrant, respondToUiRequest, uiGrant } from './ui-request';

describe('assistant settings and local conversation state', () => {
  it('retains each provider profile without copying external choices into API settings', () => {
    const initial = {
      ...defaultSettings,
      model: 'my-custom-model',
      api_options: { temperature: 0.3 },
    };
    const next = selectProvider(selectProvider(initial, 'openai'), 'deepseek');
    assert.equal(next.model, 'my-custom-model');
    assert.deepEqual(next.api_options, { temperature: 0.3 });
    const external = withChoice(
      { ...next, backend: 'opencode' },
      { model: 'provider/model', effort: 'high' },
    );
    assert.deepEqual(external.external.opencode, { model: 'provider/model', effort: 'high' });
    assert.equal(external.model, 'my-custom-model');
  });
  it('keeps a valid API profile when saving an external backend with incomplete custom input', () => {
    const saved = settingsForSave(
      {
        ...defaultSettings,
        backend: 'codex',
        base_url: '',
        model: '',
        api_profiles: { custom: { ...defaultSettings, model: '' } },
      },
      defaultSettings,
    );
    assert.equal(saved.base_url, defaultSettings.base_url);
    assert.deepEqual(saved.api_profiles, {});
    assert.equal(saved.backend, 'codex');
  });
  it('rejects non-object options and preserves off as a true protocol switch', () => {
    assert.throws(() => parseApiOptions('[]'));
    assert.throws(() => parseApiOptions('null'));
    assert.deepEqual(thinkSettings('off'), { thinking: false, reasoning_effort: null });
  });
  it('offers only confirmed effort levels for each model, preserving saved unknown choices', () => {
    assert.deepEqual(apiEfforts('gemini', 'gemini-3.8-flash'), ['', 'low', 'medium', 'high']);
    assert.deepEqual(apiEfforts('qwen', 'qwen3.8-flash'), ['', 'low', 'medium', 'xhigh', 'off']);
    assert.deepEqual(apiEfforts('qwen', 'qwen3.8-max'), ['', 'low', 'medium', 'xhigh', 'off']);
    assert.deepEqual(apiEfforts('openai', 'gpt-6.1-sol'), [
      '',
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
    assert.deepEqual(apiEfforts('anthropic', 'claude-sonnet-5-5'), [
      '',
      'low',
      'medium',
      'high',
      'xhigh',
      'max',
    ]);
    for (const provider of API_PROVIDERS)
      assert.deepEqual(apiEfforts(provider.id, 'unknown-model'), ['']);
    const saved = {
      ...defaultSettings,
      model: 'unknown-model',
      reasoning_effort: 'provider_custom_effort',
    };
    assert.equal(thinkValue(saved), 'provider_custom_effort');
    assert.equal(
      selectProvider(selectProvider(saved, 'openai'), 'deepseek').reasoning_effort,
      'provider_custom_effort',
    );
  });
  it('restores unsent draft after traversing chronological input history', () => {
    assert.deepEqual(historyInput(['old', 'new'], null, 'draft', -1), { index: 1, value: 'new' });
    assert.deepEqual(historyInput(['old', 'new'], 1, 'draft', 1), { index: null, value: 'draft' });
    assert.deepEqual(historyInput(['old'], 0, 'draft', -1), { index: 0, value: 'old' });
  });
  it('does not append assistant text to a tool or reasoning message', () => {
    assert.equal(
      appendDelta([{ kind: 'reasoning', text: 'hidden' }], 'assistant', 'answer').length,
      2,
    );
    assert.equal(appendDelta([{ kind: 'assistant', text: 'a' }], 'assistant', 'b')[0].text, 'ab');
  });
});

describe('assistant UI response boundary', () => {
  it('awaits completion before sending a success result', async () => {
    const order: string[] = [];
    await respondToUiRequest(
      { type: 'ui_request', id: 'r', action: 'click', args: { target: 'scan' } },
      async () => {
        await Promise.resolve();
        order.push('completed');
        return { files: 3 };
      },
      async (id, ok, result) => {
        order.push('response');
        assert.deepEqual([id, ok, result], ['r', true, { files: 3 }]);
      },
    );
    assert.deepEqual(order, ['completed', 'response']);
  });
  it('returns real action errors and never retries a failed response transport', async () => {
    const replies: unknown[] = [];
    await respondToUiRequest(
      { type: 'ui_request', id: 'r', action: 'commit' },
      async () => {
        throw new Error('permission denied');
      },
      async (...args) => {
        replies.push(args);
      },
    );
    assert.deepEqual(replies, [['r', false, 'permission denied']]);
    let calls = 0;
    await assert.rejects(
      respondToUiRequest(
        { type: 'ui_request', id: 'r', action: 'state' },
        () => ({}),
        async () => {
          calls++;
          throw new Error('offline');
        },
      ),
      /offline/,
    );
    assert.equal(calls, 1);
  });
  it('requires an approved matching action and target, not the unrelated request ID', () => {
    const grant = uiGrant({
      type: 'tool_call',
      id: 'tool',
      name: 'ui_commit',
      arguments: '{"target":"run","purpose":"run case"}',
      preapproved: false,
    });
    const request = {
      type: 'ui_request',
      id: 'request',
      action: 'commit',
      args: { target: 'run' },
    };
    assert.equal(matchesGrant(grant, request), false);
    grant!.approved = true;
    assert.equal(matchesGrant(grant, request), true);
    assert.equal(matchesGrant(grant, { ...request, args: { target: 'delete' } }), false);
    assert.equal(matchesGrant(grant, { ...request, action: 'set' }), false);
    assert.equal(
      uiGrant({ type: 'tool_call', id: 'x', name: 'ui_commit', arguments: 'invalid' }),
      null,
    );
  });
});

describe('approval binds stable page identity and editable inputs', () => {
  const identity = {
    route: 'case',
    caseDir: '/cases/a',
    versionRef: { id: 'official', revision: 1 },
  };
  const page = {
    step: 'runs',
    status: 'idle',
    fields: [
      { id: 'threads', value: '4' },
      { id: 'progress', value: '10%', readOnly: true },
    ],
  };
  it('ignores background status, log and read-only progress changes', () => {
    assert.equal(
      approvalSnapshot(identity, page),
      approvalSnapshot(identity, {
        ...page,
        status: 'busy',
        log: 'more output',
        progress: 99,
        fields: [
          { id: 'threads', value: '4' },
          { id: 'progress', value: '90%', readOnly: true },
        ],
      }),
    );
  });
  it('invalidates a grant after changing editable inputs, page, source case or version', () => {
    const previous = approvalSnapshot(identity, page);
    assert.notEqual(
      previous,
      approvalSnapshot(identity, { ...page, fields: [{ id: 'threads', value: '8' }] }),
    );
    assert.notEqual(previous, approvalSnapshot(identity, { ...page, step: 'settings' }));
    assert.notEqual(previous, approvalSnapshot({ ...identity, route: 'versions' }, page));
    assert.notEqual(previous, approvalSnapshot({ ...identity, caseDir: '/cases/b' }, page));
    assert.notEqual(
      previous,
      approvalSnapshot({ ...identity, versionRef: { id: 'official', revision: 2 } }, page),
    );
  });
});

it('restores missing profile fields from provider defaults', () => {
  const profile = { base_url: 'https://custom.example', model: 'custom' };
  const next = selectProvider(
    { ...defaultSettings, api_profiles: { custom: profile as typeof defaultSettings } },
    'custom',
  );
  assert.equal(next.model, 'custom');
  assert.equal(next.max_output_tokens, 16384);
  assert.equal(next.timeout_seconds, 600);
  assert.equal(next.context_window, null);
  assert.deepEqual(next.api_options, {});
});
it('keeps a context window per provider profile and drops it when the connection changes', () => {
  // 恢复完整匹配的已保存 profile 时保留它自己的窗口。
  const saved = rememberProfile({ ...defaultSettings, context_window: 1_000_000 });
  assert.equal(selectProvider(saved, 'deepseek').context_window, 1_000_000);
  // 切到没有保存过窗口的服务：不沿用 DeepSeek 的 1M；切回来再恢复。
  const anthropic = selectProvider(saved, 'anthropic');
  assert.equal(anthropic.context_window, null);
  assert.equal(selectProvider(anthropic, 'deepseek').context_window, 1_000_000);
  // 直接改模型/地址/协议：窗口属于旧配置，必须清空。
  for (const patch of [
    { model: 'deepseek-v4-pro' },
    { base_url: 'https://proxy.example/v1' },
    { api_format: 'responses' },
  ])
    assert.equal(withContextWindowCleared(saved, patch).context_window, null);
  assert.equal(selectAssistantModel(saved, 'deepseek-v4-pro').context_window, null);
  // 其它字段（输出上限、思考、超时）不代表端点换了，窗口保留。
  assert.equal({ ...saved, max_output_tokens: 2048 }.context_window, 1_000_000);
});
it('offers all four real bundled examples in both languages without sending', () => {
  assert.deepEqual(
    EXAMPLE_STARTERS.map((e) => e.id),
    ['CN-Cng', 'AT-Neu', 'US-Ne3', 'AU-Preston'],
  );
  for (const e of EXAMPLE_STARTERS) {
    assert.ok(examplePrompt(e.id, 'zh').includes(e.id));
    assert.ok(examplePrompt(e.id, 'en').includes('before starting'));
  }
  assert.throws(() => examplePrompt('unknown', 'en'));
  assert.equal(
    providerLabel(
      { id: 'custom', name: '自定义服务', base_url: '', model: '', api_format: 'chat_completions' },
      'en',
    ),
    'Custom provider',
  );
});

it('compact model choice preserves provider and external settings, resetting incompatible effort', () => {
  assert.deepEqual(assistantModels(defaultSettings), ['deepseek-flash', 'deepseek-v4-pro']);
  const custom = { ...defaultSettings, model: 'custom', reasoning_effort: 'max' };
  assert.equal(assistantModels(custom)[0], 'custom');
  const changed = selectAssistantModel(custom, 'unknown');
  assert.equal(changed.backend, 'builtin');
  assert.equal(changed.base_url, custom.base_url);
  assert.equal(changed.reasoning_effort, null);
  const external = selectAssistantModel({ ...custom, backend: 'opencode' }, 'provider/model');
  assert.equal(external.model, 'custom');
  assert.equal(external.external.opencode.model, 'provider/model');
});

it('names both the model service and the web-search service in the egress confirmation', () => {
  assert.equal(
    egressTarget({ ...defaultSettings, base_url: 'https://api.openai.com/v1', web_search: true }),
    'https://api.openai.com/v1 (web search & fetch: DeepSeek APIs)',
  );
  assert.equal(egressTarget({ ...defaultSettings, web_search: false }), 'https://api.deepseek.com');
  assert.equal(
    egressTarget({
      ...defaultSettings,
      backend: 'codex',
      web_search: true,
    }),
    'codex',
  );
  assert.equal(
    egressTarget({
      ...defaultSettings,
      backend: 'opencode',
      web_search: true,
      external: { opencode: { model: 'provider/model', effort: null } },
    }),
    'opencode:provider/model',
  );
});

it('approval snapshots ignore volatile fields that change while polling', () => {
  const identity = { route: 'research' };
  const page = {
    step: 'research',
    fields: [
      { id: 'name', value: 'coLM', volatile: false },
      { id: 'study-member', value: 'm000001', volatile: true },
    ],
  };
  const before = approvalSnapshot(identity, page);
  page.fields[1].value = 'm000002';
  assert.equal(approvalSnapshot(identity, page), before);
});

it('ships finite per-conversation limits and keeps them on the saved settings', () => {
  assert.equal(defaultSettings.session_turn_limit, 100);
  assert.equal(defaultSettings.session_token_limit, 5_000_000);
  const saved = settingsForSave(
    { ...defaultSettings, session_turn_limit: 7, session_token_limit: 0 },
    defaultSettings,
  );
  assert.equal(saved.session_turn_limit, 7);
  assert.equal(saved.session_token_limit, 0);
});

it('refusals before a turn starts are recognised so the input can be restored', () => {
  assert.equal(
    rejectedBeforeTurn('session limit reached: this conversation has used its 100 turns'),
    true,
  );
  assert.equal(rejectedBeforeTurn('Error: still answering the previous message'), true);
  assert.equal(rejectedBeforeTurn('no API key is stored for this service; set it'), true);
  assert.equal(rejectedBeforeTurn('HTTP 500 from the model'), false);
});

it('defaults Xerj retrieval to an external node and keeps it out of provider profiles', () => {
  assert.deepEqual(defaultSettings.retrieval, {
    enabled: false,
    mode: 'external',
    binary_path: null,
    port: null,
    endpoint: 'http://127.0.0.1:9200',
    index: 'colm-docs-v1',
    timeout_ms: 1500,
  });
  const saved = rememberProfile({
    ...defaultSettings,
    retrieval: {
      enabled: true,
      mode: 'managed',
      binary_path: '/opt/xerj/bin/xerj',
      port: 9300,
      endpoint: 'http://localhost:9200',
      index: 'docs-v2',
      timeout_ms: 2000,
    },
  });
  // 检索是应用级设置：不能进服务商 profile，否则切服务商会被旧 profile 覆盖。
  assert.equal('retrieval' in (saved.api_profiles.deepseek as object), false);
  // 切服务商不丢检索设置（含 managed 字段）；保存（含 profile 归一）也原样保留。
  const switched = selectProvider(saved, 'openai');
  assert.deepEqual(switched.retrieval, saved.retrieval);
  const forSave = settingsForSave(
    { ...switched, retrieval: { ...switched.retrieval, index: 'docs-v3', port: null } },
    saved,
  );
  assert.deepEqual(forSave.retrieval, {
    enabled: true,
    mode: 'managed',
    binary_path: '/opt/xerj/bin/xerj',
    port: null,
    endpoint: 'http://localhost:9200',
    index: 'docs-v3',
    timeout_ms: 2000,
  });
});

it('validates Xerj retrieval settings with bilingual messages even when disabled', () => {
  const base = defaultSettings.retrieval;
  assert.equal(retrievalSettingsProblem(base, 'zh'), null);
  // 关闭时也校验：关着存进非法值、等打开才报错就晚了。
  for (const endpoint of [
    'https://127.0.0.1:9200',
    'http://192.168.1.10:9200',
    'http://docs.example.com:9200',
    'http://127.0.0.1:9200/v1',
    'http://127.0.0.1:9200?key=secret',
    'http://127.0.0.1:9200#fragment',
    'http://user:pass@127.0.0.1:9200',
    'http://127.0.0.1',
    'not a url',
  ])
    assert.ok(retrievalSettingsProblem({ ...base, endpoint }, 'zh'), endpoint);
  for (const index of ['', '-docs', '_docs', 'Docs', 'docs v1', 'docs/v1', 'a'.repeat(65), '文档'])
    assert.ok(retrievalSettingsProblem({ ...base, index }, 'en'), index);
  for (const timeout_ms of [0, 99, 10_001, 1.5, Number.NaN])
    assert.ok(retrievalSettingsProblem({ ...base, timeout_ms }, 'en'), String(timeout_ms));
  // 合法值（含末尾斜杠、localhost 与 64 字节索引名）都能通过。
  for (const patch of [
    { endpoint: 'http://localhost:9200/' },
    { index: 'a' },
    { index: 'a'.repeat(64) },
    { timeout_ms: 100 },
    { timeout_ms: 10_000 },
  ])
    assert.equal(
      retrievalSettingsProblem({ ...base, ...patch }, 'en'),
      null,
      JSON.stringify(patch),
    );
  assert.match(retrievalSettingsProblem({ ...base, index: 'Docs' }, 'zh')!, /索引名/);
  assert.match(retrievalSettingsProblem({ ...base, index: 'Docs' }, 'en')!, /index name/i);

  // 接入方式只接受 external|managed；managed 的托管字段只在 managed 下校验。
  for (const mode of ['', 'MANAGED', 'automatic'])
    assert.ok(
      retrievalSettingsProblem({ ...base, mode: mode as RetrievalSettings['mode'] }, 'en'),
      mode,
    );
  const managed: RetrievalSettings = { ...base, mode: 'managed' };
  assert.equal(retrievalSettingsProblem(managed, 'en'), null);
  for (const binary_path of ['relative/xerj', 'xerj', '.\\xerj.exe', '/opt/bad\npath', '\t'])
    assert.ok(retrievalSettingsProblem({ ...managed, binary_path }, 'zh'), binary_path);
  for (const binary_path of [null, '', '  ', '/usr/local/bin/xerj', 'C:\\xerj\\xerj.exe'])
    assert.equal(
      retrievalSettingsProblem({ ...managed, binary_path }, 'en'),
      null,
      String(binary_path),
    );
  for (const port of [null, 1024, 9200, 65_000])
    assert.equal(retrievalSettingsProblem({ ...managed, port }, 'en'), null, String(port));
  for (const port of [0, 1023, 65_001, 1024.5, Number.NaN])
    assert.ok(retrievalSettingsProblem({ ...managed, port }, 'zh'), String(port));
  // external 模式不校验（也不理会）托管字段：切回去不应被上次 managed 的残留值挡住。
  assert.equal(
    retrievalSettingsProblem({ ...base, binary_path: 'relative/xerj', port: 1 }, 'en'),
    null,
  );
  // managed 仍要能通过整条保存校验（endpoint 是回退值，也必须合法）。
  assert.ok(retrievalSettingsProblem({ ...managed, endpoint: 'https://127.0.0.1:9200' }, 'en'));
});

it('translates the managed Xerj node status and keeps unknown values visible', () => {
  assert.equal(managedNodeText(null, 'zh'), '节点状态尚未获取');
  assert.equal(managedNodeText(null, 'en'), 'Node status has not been loaded');
  const running = {
    mode: 'managed',
    running: true,
    endpoint: 'http://127.0.0.1:9300',
    port: 9300,
    version: 'v1.0.0-rc.100',
    pinned_version: 'v1.0.0-rc.100',
    pinned_match: true,
    binary: '/opt/xerj/bin/xerj',
    data_dir: '/tmp/colm-xerj',
    detail: null,
  };
  assert.match(managedNodeText(running, 'zh'), /运行中/);
  assert.match(managedNodeText(running, 'zh'), /v1\.0\.0-rc\.100/);
  assert.match(managedNodeText(running, 'zh'), /一致/);
  assert.match(managedNodeText(running, 'en'), /Running/);
  assert.match(managedNodeText(running, 'en'), /http:\/\/127\.0\.0\.1:9300/);
  const stopped = {
    ...running,
    running: false,
    version: 'v1.0.0-rc.99',
    pinned_match: false,
    detail: 'version differs from the pinned release; replace the binary manually',
  };
  assert.match(managedNodeText(stopped, 'zh'), /未运行/);
  assert.match(managedNodeText(stopped, 'zh'), /不一致/);
  assert.match(managedNodeText(stopped, 'en'), /does not match/);
  // detail 是后端给的原样说明，不翻译、不吞掉。
  assert.match(managedNodeText(stopped, 'en'), /replace the binary manually/);
  assert.match(managedNodeText(stopped, 'zh'), /replace the binary manually/);
  // 未知 mode 原样显示；external 说清楚不探测。
  assert.match(
    managedNodeText({ ...running, mode: 'future_mode' }, 'en'),
    /Unknown node mode future_mode/,
  );
  assert.match(managedNodeText({ ...running, mode: 'external' }, 'en'), /External node/);
});

it('translates known Xerj index states and leaves unknown states untouched', () => {
  assert.equal(retrievalStateText('ready', 'zh'), '索引就绪');
  assert.equal(retrievalStateText('ready', 'en'), 'Index ready');
  assert.equal(retrievalStateText('disabled', 'zh'), '未启用');
  assert.equal(retrievalStateText('building', 'en'), 'Building index');
  assert.equal(retrievalStateText('stale', 'en'), 'Index stale');
  // 后端将来新增的状态原样显示，不能悄悄变成空白或"就绪"。
  assert.equal(
    retrievalStateText('rejected_by_future_backend', 'en'),
    'rejected_by_future_backend',
  );
  assert.equal(retrievalStateText('', 'zh'), '');
});

// ---------------------------------------------------------------------------
// U01/U02：只读历史仓库与聊天历史范围
// ---------------------------------------------------------------------------

const HISTORY_SUCCESS_MS = 1_790_000_000_000;
const HISTORY_CHECKED_MS = 1_730_000_000_000;

function historyRepo(over: Partial<HistoryRepoStatus> = {}): HistoryRepoStatus {
  return {
    identity: { state: 'verified', root: null, remote: null },
    coverage: { shallow: false, commits: 42, complete_history: true, truncated: false },
    ...over,
  };
}

it('defaults the read-only history scope to auto and keeps it out of provider profiles', () => {
  assert.deepEqual(defaultSettings.history, {
    scope: 'auto',
    desktop_path: null,
    upstream_path: null,
  });
  // 旧设置缺 history：读入时补默认值，保留已有路径字段。
  assert.deepEqual(historySettingsOf(undefined), defaultHistorySettings);
  assert.deepEqual(historySettingsOf({}), defaultHistorySettings);
  assert.deepEqual(
    historySettingsOf({ history: { scope: 'future' as never, desktop_path: '/repos/desktop' } }),
    { scope: 'auto', desktop_path: '/repos/desktop', upstream_path: null },
  );
  const saved = rememberProfile({
    ...defaultSettings,
    history: { scope: 'desktop', desktop_path: '/repos/desktop', upstream_path: null },
  });
  // 历史是应用级设置：不能进服务商 profile，否则切服务商会被旧 profile 覆盖。
  assert.equal('history' in (saved.api_profiles.deepseek as object), false);
  assert.deepEqual(selectProvider(saved, 'openai').history, saved.history);
  // 只改范围/只改一个仓库路径，其余设置与 profile 原样保留。
  const scoped = withHistoryScope(saved, 'both');
  assert.equal(scoped.history.scope, 'both');
  assert.equal(scoped.history.desktop_path, '/repos/desktop');
  assert.deepEqual(scoped.api_profiles, saved.api_profiles);
  const pathed = withHistoryPath(scoped, 'colm_upstream', '/repos/upstream');
  assert.deepEqual(pathed.history, {
    scope: 'both',
    desktop_path: '/repos/desktop',
    upstream_path: '/repos/upstream',
  });
  const forSave = settingsForSave(pathed, saved);
  assert.deepEqual(forSave.history, pathed.history);
  assert.deepEqual(withHistoryPath(pathed, 'desktop', null).history.desktop_path, null);
});

it('formats sync times and short SHAs deterministically', () => {
  assert.equal(formatHistoryTime(HISTORY_SUCCESS_MS), '2026-09-21 14:13 UTC');
  assert.equal(formatHistoryTime(0), '');
  assert.equal(formatHistoryTime(null), '');
  assert.equal(formatHistoryTime(undefined), '');
  assert.equal(shortSha('0123456789abcdef0123456789abcdef01234567'), '0123456');
  assert.equal(shortSha(''), '');
  assert.equal(shortSha(null), '');
  assert.equal(shortSha('not-a-sha'), 'not-a-sha');
});

it('translates history scopes, repository names and the routing hint bilingually', () => {
  assert.equal(historyScopeLabel('auto', 'zh'), '自动（按问题）');
  assert.equal(historyScopeLabel('auto', 'en'), 'Automatic (by question)');
  assert.equal(historyScopeLabel('selected', 'en'), 'Current model');
  assert.equal(historyScopeLabel('desktop', 'zh'), 'Desktop 仓库');
  assert.equal(historyScopeLabel('upstream', 'en'), 'CoLM upstream');
  assert.equal(historyScopeLabel('both', 'zh'), '两个仓库');
  // 未知范围原样显示，不能悄悄变成"自动"。
  assert.equal(historyScopeLabel('future_scope', 'en'), 'future_scope');
  assert.match(historyScopeHint('en'), /chosen scope takes precedence/);
  assert.match(historyScopeHint('zh'), /范围优先/);
  assert.equal(historyRepositoryLabel('desktop', 'en'), 'Desktop repository');
  assert.equal(historyRepositoryLabel('colm_upstream', 'zh'), 'CoLM 官方仓库');
});

it('keeps last successful sync, checked time and legacy records separate', () => {
  const success = historyRepoLines(
    historyRepo({
      last_successful_sync: {
        ref: 'main',
        sha: '0123456789abcdef0123456789abcdef01234567',
        at_unix_ms: HISTORY_SUCCESS_MS,
      },
      checked_at_unix_ms: HISTORY_CHECKED_MS,
    }),
    'en',
  );
  assert.equal(success.state, 'Verified');
  assert.match(success.lastSync, /Last successful sync/);
  assert.match(success.lastSync, /main @ 0123456/);
  assert.ok(success.lastSync.includes(formatHistoryTime(HISTORY_SUCCESS_MS)));
  // 成功时间与检查时间必须是两行：checked_at 不能被写成同步成功时间。
  assert.match(success.checkedAt, /Last checked/);
  assert.ok(success.checkedAt.includes(formatHistoryTime(HISTORY_CHECKED_MS)));
  assert.ok(!success.lastSync.includes(formatHistoryTime(HISTORY_CHECKED_MS)));
  assert.deepEqual(success.errors, []);
  assert.equal(success.preparation, null);

  // 旧版 synced 记录不是成功证据：只能标"legacy/未证实"，绝不能出现在"上次成功同步"里。
  const legacy = historyRepoLines(
    historyRepo({
      last_successful_sync: null,
      legacy_synced: { ref: 'main', sha: '0123456', at_unix_ms: HISTORY_CHECKED_MS },
      checked_at_unix_ms: HISTORY_CHECKED_MS,
    }),
    'zh',
  );
  assert.match(legacy.lastSync, /旧记录/);
  assert.match(legacy.lastSync, /未证实同步成功/);
  assert.ok(!legacy.lastSync.includes('上次成功同步'));
  assert.match(legacy.preparation!, /只有旧版同步记录/);

  const missing = historyRepoLines({ identity: { state: 'missing' } }, 'zh');
  assert.equal(missing.state, '缺失');
  assert.match(missing.lastSync, /还没有成功同步记录/);
  assert.match(missing.checkedAt, /还没有检查记录/);
  assert.match(missing.preparation!, /准备数据/);

  const failed = historyRepoLines(
    historyRepo({
      last_successful_sync: null,
      last_attempt: { action: 'error', error: 'git fetch origin failed' },
      errors: ['git fetch origin failed', 'network unreachable'],
    }),
    'en',
  );
  // last_attempt.error 与持久 errors 去重后都要以错误样式露出来。
  assert.deepEqual(failed.errors, ['git fetch origin failed', 'network unreachable']);
  assert.match(failed.preparation!, /last update failed/i);

  const unverified = historyRepoLines({ identity: { state: 'unverified' } }, 'en');
  assert.equal(unverified.state, 'Unverified identity');
  assert.match(unverified.preparation!, /identity is unverified/);

  const partial = historyRepoLines(
    historyRepo({ coverage: { shallow: true, commits: 3, complete_history: false } }),
    'zh',
  );
  assert.match(partial.state, /部分历史/);
  assert.match(partial.preparation!, /浅克隆/);

  // 后端将来新增的身份状态原样显示。
  assert.match(
    historyRepoLines({ identity: { state: 'future_state' } }, 'en').state,
    /future_state/,
  );
});

it('summarises explicit sync actions and marks failures without inventing success', () => {
  const ok = historySyncSummary({ actions: { desktop: 'fetched', colm_upstream: 'cloned' } }, 'en');
  assert.equal(ok?.failed, false);
  assert.match(ok!.text, /Desktop repository updated/);
  assert.match(ok!.text, /CoLM upstream repository cloned/);
  const offline = historySyncSummary(
    { actions: { desktop: 'offline', colm_upstream: 'skipped' } },
    'zh',
  );
  assert.equal(offline?.failed, false);
  assert.match(offline!.text, /离线/);
  assert.match(offline!.text, /跳过/);
  const failed = historySyncSummary(
    { actions: { desktop: 'error', colm_upstream: 'future_action' } },
    'en',
  );
  assert.equal(failed?.failed, true);
  assert.match(failed!.text, /with failures/);
  assert.match(failed!.text, /future_action/);
  assert.equal(historySyncSummary(null, 'en'), null);
  assert.equal(historySyncSummary({}, 'en'), null);
});

it('names the current backend, model and frozen revision for the history hint', () => {
  const frozen = {
    version: { id: 'soil-process', revision: 3 },
    code: { commit: '0123456789abcdef0123456789abcdef01234567' },
  };
  const builtin = historyContextLabel(
    defaultSettings,
    frozen,
    { id: 'soil-process', revision: 3 },
    'en',
  );
  assert.equal(builtin.backend, 'DeepSeek (Pro / Flash)');
  assert.equal(builtin.model, 'deepseek-flash');
  assert.equal(builtin.revision, '0123456');
  // 没有 commit 的冻结记录退回 id@revision；官方/内置明确写出来，不猜 SHA。
  assert.equal(
    historyContextLabel(
      defaultSettings,
      { version: { id: 'v1', revision: 2 }, code: { commit: null } },
      { id: 'v1', revision: 2 },
      'zh',
    ).revision,
    'v1@2',
  );
  assert.equal(
    historyContextLabel(defaultSettings, null, { id: 'official', revision: 1 }, 'zh').revision,
    '官方/应用内置',
  );
  const codex = historyContextLabel(
    {
      ...defaultSettings,
      backend: 'codex',
      external: { codex: { model: 'gpt-codex', effort: null } },
    },
    null,
    { id: 'official', revision: 1 },
    'en',
  );
  assert.equal(codex.backend, 'Local Codex');
  assert.equal(codex.model, 'gpt-codex');
  assert.equal(codex.revision, 'official / built-in');
});

// ---------------------------------------------------------------------------
// U03：只读历史证据卡
// ---------------------------------------------------------------------------

const historyToolResult = (value: unknown) => JSON.stringify(value);

it('recognises history tools with external-backend prefixes only', () => {
  for (const name of [
    'source_history',
    'mcp__colm__source_history',
    'colm/source_history',
    'colm.source_history',
    'commit_details',
    'mcp__colm__commit_details',
    'commit_pull_requests',
  ])
    assert.ok(historyToolKind(name), name);
  for (const name of ['source_history_extra', 'read_file', 'commit', '', undefined, null])
    assert.equal(historyToolKind(name), null, String(name));
});

it('keeps only safe relative history paths and builds open_path targets from trusted roots', () => {
  assert.equal(safeHistoryRelativePath('src/foo.F90'), 'src/foo.F90');
  assert.equal(safeHistoryRelativePath('src\\foo.F90'), 'src/foo.F90');
  for (const path of ['/etc/passwd', '../secret', 'a/../../b', 'a//b', 'file:///etc', 'a\nb', ''])
    assert.equal(safeHistoryRelativePath(path), null, JSON.stringify(path));
  assert.equal(historyFileOpenPath('/repos/desktop', 'src/foo.F90'), '/repos/desktop/src/foo.F90');
  // 可疑相对路径/非绝对根目录都不给出可点击目标。
  assert.equal(historyFileOpenPath('/repos/desktop', '../etc/passwd'), null);
  assert.equal(historyFileOpenPath('relative/root', 'src/foo.F90'), null);
  assert.equal(historyFileOpenPath('/repos/desktop', 'https://x/y'), null);
  assert.equal(historyFileOpenPath(undefined, 'src/foo.F90'), null);
});

it('renders source_history as structured evidence without inventing motivation', () => {
  const result = historyToolResult({
    schema: 1,
    query: {
      symbol: 'f_fevg',
      repository: 'both',
      path: null,
      source: {
        mode: 'frozen_model',
        source_dir: '/models/src',
        dirty: true,
        commit: 'a'.repeat(40),
      },
      endpoints: {
        desktop: { ref: 'main', sha: 'b'.repeat(40) },
        colm_upstream: { ref: 'upstream/main', sha: 'c'.repeat(40) },
      },
    },
    repos: {
      desktop: {
        key: 'desktop',
        identity: { state: 'verified', root: '/repos/desktop', remote: null },
        endpoint: { ref: 'main', sha: 'b'.repeat(40) },
        history: 'verified',
        locations: [
          { path: 'src/a.F90', line: 42, scope: 'module', kind: 'fortran', commit: 'b'.repeat(40) },
        ],
        commits: [
          {
            sha: 'b'.repeat(40),
            author: 'A U Thor',
            author_time_unix: 1_790_000_000,
            committer_time_unix: 1_730_000_000,
            summary: 'change f_fevg',
            reason: {
              status: 'unchecked',
              basis: 'commit_message',
              excerpt: 'because',
              evidence_candidates: [],
            },
            relevance: ['grep'],
            truncated: false,
          },
        ],
        uncommitted: [{ path: 'src/b.F90', uncommitted: true, hunks: [] }],
        notes: [],
        errors: [],
      },
      colm_upstream: {
        key: 'colm_upstream',
        identity: { state: 'unverified', root: '/repos/upstream', remote: null },
        endpoint: null,
        history: 'unavailable',
        locations: [],
        commits: [],
        uncommitted: [],
        notes: ['repository history is unavailable'],
        errors: ['not a git checkout'],
      },
    },
    cross: {
      links: [
        {
          state: 'verified',
          basis: ['path+blob', 'structured-upstream-commit'],
          upstream: {
            repository: 'colm_upstream',
            commit: 'c'.repeat(40),
            path: 'src/a.F90',
            blob: 'blob',
          },
          desktop: {
            repository: 'desktop',
            vendor_path: 'vendor/CoLM202X/src/a.F90',
            import_commit: { sha: 'd'.repeat(40) },
            later_commits: [],
            blob: 'blob',
          },
          candidates: [],
          notes: [],
        },
        {
          state: 'candidate',
          basis: ['path'],
          upstream: { repository: 'colm_upstream', commit: null, path: 'src/c.F90', blob: null },
          desktop: null,
          candidates: [{ sha: 'e'.repeat(40), blob_equal: true }],
          notes: ['multiple candidates'],
        },
        { state: 'unmapped', basis: [], upstream: null, desktop: null, candidates: [], notes: [] },
      ],
    },
    limits: ['cross links capped at 20'],
    truncated: true,
    history_root: '/data/history',
  });
  const evidence = historyEvidence('mcp__colm__source_history', result, 'en');
  assert.ok(evidence);
  assert.match(evidence!.summary, /f_fevg/);
  assert.match(evidence!.summary, /1 locations/);
  assert.match(evidence!.summary, /1 commits/);
  assert.match(evidence!.summary, /truncated/);
  const types = evidence!.sections.map((section) => section.title);
  assert.ok(types.includes('query'));
  assert.ok(types.some((title) => title.includes('Desktop')));
  // 作者时间与提交时间分开显示，不合并。
  const desktop = evidence!.sections.find((section) => section.title.includes('Desktop'))!;
  const commitItem = desktop.items!.find((item) => item.title.includes('bbbbbbb'))!;
  const labels = commitItem.fields.map((field) => field.label);
  assert.ok(labels.some((label) => /author time/i.test(label)));
  assert.ok(labels.some((label) => /committer time/i.test(label)));
  // reason 只显示原文候选状态，不写成已证实动机。
  const reasonField = commitItem.fields.find((field) => /reason status/i.test(field.label))!;
  assert.equal(reasonField.value, 'unchecked');
  // 文件可点击目标只来自可信仓库根目录。
  const location = desktop.items!.find((item) => item.title === 'src/a.F90:42')!;
  assert.equal(location.path, '/repos/desktop/src/a.F90');
  // 上游不可用时保留 error 文本，不伪造位置。
  const upstream = evidence!.sections.find((section) => section.title.includes('CoLM upstream'))!;
  assert.match(upstream.note!, /not a git checkout/);
  const cross = evidence!.sections.find((section) => section.title === 'cross-repository links')!;
  assert.deepEqual(
    cross.items!.map((item) => item.title.split(' · ')[0]),
    ['verified', 'candidate', 'unmapped'],
  );
  assert.ok(cross.items!.every((item) => item.fields.some((field) => /basis/i.test(field.label))));
  assert.match(cross.note!, /formal Git trailer/i);
  const limits = evidence!.sections.find((section) => section.title === 'limits and truncation')!;
  assert.match(limits.note!, /cross links capped/);
});

it('renders commit_details with separated times, reason candidates, limited diffs and safe files', () => {
  const result = historyToolResult({
    schema: 1,
    repository: {
      key: 'desktop',
      owner: 'zhongwangwei',
      repo: 'CoLM-Desktop',
      url: 'https://github.com/zhongwangwei/CoLM-Desktop',
      path: '/repos/desktop',
      identity: { state: 'verified', root: '/repos/desktop', remote: null },
    },
    commit: {
      sha: 'b'.repeat(40),
      parents: ['c'.repeat(40)],
      author: 'A U Thor',
      author_email: 'a@example.com',
      author_time_unix: 1_790_000_000,
      committer: 'C O Mitter',
      committer_time_unix: 1_730_000_000,
      message: 'subject\n\nbody of the change',
      compare_parent: 'c'.repeat(40),
      files: [
        {
          path: 'src/a.F90',
          hunks: [
            {
              old_start: 1,
              old_lines: 1,
              new_start: 1,
              new_lines: 1,
              old_code: ['x=1'],
              new_code: ['x=2'],
              truncated: false,
            },
          ],
        },
        { path: '../etc/passwd', hunks: [] },
      ],
      truncated: true,
    },
    reason: {
      status: 'unchecked',
      basis: 'commit_message',
      excerpt: 'subject',
      evidence_candidates: [
        {
          source: 'commit_message',
          author: null,
          url: 'https://github.com/o/r/commit/abc',
          scope: 'commit_message',
          excerpt: 'subject',
        },
      ],
    },
    notes: ['merge commit with 2 parents; diff compares against the first parent'],
  });
  const evidence = historyEvidence('commit_details', result, 'zh');
  assert.ok(evidence);
  assert.match(evidence!.summary, /bbbbbbb/);
  const commitSection = evidence!.sections.find((section) => section.title === '提交')!;
  assert.ok(
    commitSection.items!.some(
      (item) => item.title === '提交说明' && /body of the change/.test(item.detail!),
    ),
    '完整提交说明放在可展开原文里',
  );
  const labels = commitSection.fields.map((field) => field.label);
  assert.ok(labels.includes('作者时间'));
  assert.ok(labels.includes('提交时间'));
  assert.ok(
    !commitSection.fields.some((field) => field.label === '作者时间' && field.value === ''),
  );
  const reason = evidence!.sections.find((section) => section.title.includes('修改理由'))!;
  assert.match(reason.fields[0].value, /unchecked/);
  assert.equal(reason.items![0].url, 'https://github.com/o/r/commit/abc');
  const files = evidence!.sections.find((section) => section.title === '改动文件')!;
  assert.equal(files.items![0].title, 'src/a.F90');
  assert.equal(files.items![0].path, '/repos/desktop/src/a.F90');
  assert.match(files.items![0].detail!, /@@ -1,1 \+1,1 @@/);
  assert.match(files.items![0].detail!, /\+x=2/);
  // 可疑路径不给可点击目标。
  assert.equal(files.items![1].path, undefined);
  const notes = evidence!.sections.find((section) => section.title === '说明')!;
  assert.match(notes.note!, /merge commit/);
});

it('renders commit_pull_requests with pagination, reason candidates and unverified markers', () => {
  const result = historyToolResult({
    schema: 2,
    repository: {
      owner: 'zhongwangwei',
      repo: 'CoLM-Desktop',
      url: 'https://github.com/zhongwangwei/CoLM-Desktop',
    },
    commit: 'b'.repeat(40),
    fetched_at_unix_ms: 1_790_000_000_000,
    offline: false,
    from_cache: false,
    complete: false,
    pulls: [
      {
        number: 7,
        owner_repo: 'zhongwangwei/CoLM-Desktop',
        url: 'https://github.com/zhongwangwei/CoLM-Desktop/pull/7',
        title: 'Explain the change',
        state: 'closed',
        merged_at_unix_ms: 1_780_000_000_000,
        author: 'A U Thor',
        head_sha: 'd'.repeat(40),
        base_ref: 'main',
        body_excerpt: 'why',
        comments: { complete: false, excerpt_count: 1 },
        reason: {
          status: 'unchecked',
          basis: 'pull_body',
          excerpt: 'why',
          evidence_candidates: [
            {
              source: 'pull_body',
              author: 'A U Thor',
              url: 'https://github.com/o/r/pull/7',
              scope: 'body',
              excerpt: 'why',
            },
          ],
        },
        evidence_id: 'github:zhongwangwei/CoLM-Desktop#7',
      },
    ],
    page: { offset: 0, page_size: 3, returned: 3, total: 5, has_more: true, next_cursor: '3' },
    notices: ['cached GitHub data was fetched without PR discussion'],
    errors: [{ kind: 'rate_limit', status: 403, message: 'API rate limit exceeded' }],
  });
  const evidence = historyEvidence('mcp__colm__commit_pull_requests', result, 'en');
  assert.ok(evidence);
  assert.match(evidence!.summary, /bbbbbbb/);
  assert.match(evidence!.summary, /incomplete list|complete/i);
  const query = evidence!.sections.find((section) => section.title === 'query')!;
  assert.ok(query.fields.some((field) => /total PRs/i.test(field.label) && field.value === '5'));
  assert.ok(query.fields.some((field) => /next cursor/i.test(field.label) && field.value === '3'));
  const pulls = evidence!.sections.find((section) => section.title === 'associated pull requests')!;
  assert.equal(pulls.items![0].title, '#7 Explain the change');
  assert.equal(pulls.items![0].url, 'https://github.com/zhongwangwei/CoLM-Desktop/pull/7');
  const labels = pulls.items![0].fields.map((field) => field.label);
  assert.ok(labels.some((label) => /reason status/i.test(label)));
  assert.ok(labels.some((label) => /discussion completeness/i.test(label)));
  // 理由候选是独立条目，带出处链接。
  assert.ok(pulls.items!.some((item) => item.url === 'https://github.com/o/r/pull/7'));
  const notices = evidence!.sections.find((section) => section.title === 'notices')!;
  assert.match(notices.note!, /without PR discussion/);
  const errors = evidence!.sections.find((section) => section.title === 'errors')!;
  assert.match(errors.items![0].title, /rate limit/);
  const how = evidence!.sections.find((section) => section.title === 'how to continue')!;
  assert.match(how.note!, /cursor=3/);
  assert.match(how.note!, /do not claim the whole PR list/i);
});

it('shows detail_unverified as unverified, never as unassociated', () => {
  const evidence = historyEvidence(
    'commit_pull_requests',
    historyToolResult({
      repository: { owner: 'o', repo: 'r', url: 'https://github.com/o/r' },
      commit: 'b'.repeat(40),
      offline: true,
      from_cache: false,
      complete: false,
      pulls: [],
      page: null,
      detail_unverified: {
        pr_number: 9,
        unverified: true,
        list_complete: false,
        offline: true,
        from_cache: false,
        reason: 'GitHub was not contacted (offline cache miss)',
      },
      notices: [],
      errors: [],
    }),
    'zh',
  );
  assert.ok(evidence);
  const section = evidence!.sections.find((item) => item.title === '未核验的 PR 详情')!;
  assert.match(section.note!, /未核验 ≠ 未关联/);
  assert.match(section.note!, /offline cache miss/);
  // 空 pulls 不能在没有完整列表时写成"没有关联"。
  const pulls = evidence!.sections.find((item) => item.title === '关联 PR')!;
  assert.match(pulls.note!, /只有完整读过的关联列表/);
});

it('falls back to the raw JSON display for non-JSON or unknown tool results', () => {
  assert.equal(historyEvidence('source_history', 'not json', 'en'), null);
  assert.equal(historyEvidence('source_history', '[]', 'en'), null);
  assert.equal(historyEvidence('read_file', historyToolResult({ a: 1 }), 'en'), null);
  assert.equal(historyEvidence('source_history', null, 'en'), null);
});

// ---------------------------------------------------------------------------
// U04：任务快照与未知副作用核对
// ---------------------------------------------------------------------------

it('extracts pending reconciliation actions and ids from a task snapshot', () => {
  const snapshot = taskSnapshotOf({
    goal: 'run something',
    state: 'failed',
    phase: 'validate',
    requires_reconciliation: true,
    effect_unknown_actions: [
      {
        id: 'call-1',
        name: 'run_case',
        arguments: '{"case_dir":"/cases/a"}',
        effect: 'EffectUnknown',
        state: 'effect_unknown',
      },
      { id: '', name: 'run_remote', arguments: '{}' },
    ],
    recent_actions: [{ id: 'call-1', name: 'run_case' }],
    next_step: 'inspect existing outputs',
  });
  assert.ok(snapshot);
  const review = pendingReconciliation(snapshot);
  assert.equal(review.required, true);
  assert.equal(review.actions.length, 2);
  assert.deepEqual(review.ids, ['call-1'], '空 id 不能提交核对');
  assert.equal(snapshot!.next_step, 'inspect existing outputs');
  // 没有明确的 requires_reconciliation 时横幅不出现。
  assert.equal(pendingReconciliation({ requires_reconciliation: false }).required, false);
  assert.equal(pendingReconciliation(null).required, false);
  // 缺 effect_unknown_actions 的旧快照：保守地从 actions 里取 EffectUnknown。
  const legacy = taskSnapshotOf({
    requires_reconciliation: true,
    actions: [
      { id: 'old-1', name: 'run_case', mutating: true, effect: 'EffectUnknown' },
      { id: 'old-2', name: 'read_file', mutating: false, effect: 'FailedNoEffect' },
      {
        id: 'old-3',
        name: 'run_remote',
        mutating: true,
        effect: 'EffectUnknown',
        reconciled: true,
      },
    ],
  });
  assert.deepEqual(pendingReconciliation(legacy).ids, ['old-1'], '已核对过的旧动作不再提交');
  assert.equal(pendingReconciliation(legacy).actions.length, 1);
});

it('summarises review actions and finds local paths without opening arbitrary URLs', () => {
  const action = {
    id: 'call-1',
    name: 'run_case',
    arguments: '{"case_dir":"/cases/a","note":"https://example.com/x"}',
    result: '{"output_dir":"/cases/a/output","log":"relative/log.txt"}',
    effect: 'EffectUnknown',
    state: 'effect_unknown',
  };
  const title = taskActionTitle(action, 'en');
  assert.match(title, /run_case/);
  assert.match(title, /call-1/);
  assert.match(title, /EffectUnknown/);
  assert.match(taskActionOutput(action), /\/cases\/a/);
  assert.deepEqual(localPathsInAction(action), ['/cases/a', '/cases/a/output']);
  // 只提取绝对本地路径：URL 与相对路径不产生开文件请求。
  assert.deepEqual(localPathsInJsonText('{"a":"https://x/y","b":"rel/z"}'), []);
  assert.equal(formatUnixSeconds(1_790_000_000), '2026-09-21 14:13 UTC');
  assert.equal(formatUnixSeconds(0), '');
});

// ---------------------------------------------------------------------------
// A2：规范 GitHub 仓库的 commit 打开链接
// ---------------------------------------------------------------------------

it('A2: builds commit URLs only for canonical repositories and full lowercase SHAs', () => {
  const sha = 'b'.repeat(40);
  assert.equal(
    canonicalCommitUrl('desktop', sha),
    `https://github.com/zhongwangwei/CoLM-Desktop/commit/${sha}`,
  );
  assert.equal(
    canonicalCommitUrl('colm_upstream', sha),
    `https://github.com/CoLM-SYSU/CoLM/commit/${sha}`,
  );
  // owner/repo 与 GitHub URL 只要匹配注册表就归一成规范大小写。
  assert.equal(
    canonicalCommitUrl('zhongwangwei/colM-desktop', sha),
    `https://github.com/zhongwangwei/CoLM-Desktop/commit/${sha}`,
  );
  assert.equal(
    canonicalCommitUrl('https://github.com/CoLM-SYSU/CoLM', sha),
    `https://github.com/CoLM-SYSU/CoLM/commit/${sha}`,
  );
  assert.equal(canonicalRepositorySlug('desktop'), 'zhongwangwei/CoLM-Desktop');
  assert.equal(canonicalRepositorySlug('selected'), null);
  // selected/local、未知仓库、短 SHA/大写 SHA/别名都不生成链接（不猜 URL）。
  for (const repository of ['selected', 'selected_source', 'local', '', 'other/repo', null])
    assert.equal(canonicalCommitUrl(repository, sha), null, String(repository));
  for (const digest of ['b'.repeat(39), 'b'.repeat(41), 'B'.repeat(40), 'bbbbbbb', '', 'HEAD'])
    assert.equal(canonicalCommitUrl('desktop', digest), null, digest);
});

it('A2: only canonical source_history and commit_details commits get an open-commit action', () => {
  const sha = 'b'.repeat(40);
  const source = historyEvidence(
    'source_history',
    historyToolResult({
      query: { symbol: 'x', endpoints: { desktop: { ref: 'main', sha } } },
      repos: {
        desktop: {
          key: 'desktop',
          identity: { state: 'verified', root: '/repos/desktop' },
          commits: [{ sha, summary: 'change', reason: { status: 'unchecked' } }],
          notes: [],
          errors: [],
        },
      },
      limits: [],
      truncated: false,
    }),
    'zh',
  );
  const desktop = source!.sections.find((section) => section.title === 'Desktop 仓库')!;
  const commit = desktop.items!.find((item) => item.title.includes('bbbbbbb'))!;
  const link = commit.fields.find((field) => field.linkLabel === '打开 commit')!;
  assert.equal(link.url, `https://github.com/zhongwangwei/CoLM-Desktop/commit/${sha}`);

  // selected 来源即使有完整 SHA 也不生成 GitHub 链接（只在本地 Git 证据里）。
  const selected = historyEvidence(
    'source_history',
    historyToolResult({
      query: { symbol: 'x', endpoints: { selected: { ref: 'HEAD', sha } } },
      repos: {
        selected: {
          key: 'selected',
          identity: { state: 'verified', root: '/models/src' },
          commits: [{ sha, summary: 'local change' }],
          notes: [],
          errors: [],
        },
      },
      limits: [],
      truncated: false,
    }),
    'en',
  );
  const selectedSection = selected!.sections.find((section) => /selected/i.test(section.title))!;
  const localCommit = selectedSection.items!.find((item) => item.title.includes('bbbbbbb'))!;
  assert.equal(
    localCommit.fields.some((field) => field.url),
    false,
  );

  const details = historyEvidence(
    'commit_details',
    historyToolResult({
      repository: { key: 'desktop', path: '/repos/desktop', identity: { state: 'verified' } },
      commit: { sha, parents: [], message: 'subject', files: [] },
      reason: { status: 'not_recorded' },
      notes: [],
    }),
    'en',
  );
  const commitSection = details!.sections.find((section) => section.title === 'commit')!;
  assert.ok(commitSection.fields.some((field) => field.linkLabel === 'Open commit'));

  // selected_source 的 commit_details 不给 GitHub 链接。
  const localDetails = historyEvidence(
    'commit_details',
    historyToolResult({
      repository: { key: 'selected_source', path: '/models/src', kind: 'selected_source' },
      commit: { sha, parents: [], message: 'local', files: [] },
      reason: { status: 'not_recorded' },
      notes: [],
    }),
    'en',
  );
  const localCommitSection = localDetails!.sections.find((section) => section.title === 'commit')!;
  assert.equal(
    localCommitSection.fields.some((field) => field.url),
    false,
  );
});

// ---------------------------------------------------------------------------
// A1：source_history 当前源码保留状态（三态，不猜测）
// ---------------------------------------------------------------------------

it('A1: renders retention as retained / not_retained / unknown without inferring success', () => {
  const retained = sourceRetentionFields(
    {
      state: 'retained',
      basis: ['line-grep'],
      checked_ref: 'origin/main',
      checked_sha: 'c'.repeat(40),
      commit: 'b'.repeat(40),
      paths: ['src/a.F90', 'src/b.F90'],
      note: 'line-level check only',
    },
    'zh',
  );
  const retainedState = retained.find((field) => field.label === '当前源码保留状态')!;
  assert.match(retainedState.value, /当前终点仍含该提交新增行/);
  assert.match(retainedState.value, /不等于数值影响已证实/);
  assert.ok(retained.some((field) => /origin\/main @ ccccccc/.test(field.value)));
  assert.ok(retained.some((field) => field.label === '该提交' && field.value === 'bbbbbbb'));
  assert.ok(
    retained.some((field) => field.label === '保留判断依据' && field.value === 'line-grep'),
  );
  assert.ok(
    retained.some(
      (field) => field.label === '保留状态说明' && field.value === 'line-level check only',
    ),
  );

  const notRetained = sourceRetentionFields(
    { state: 'not_retained', checked_ref: 'main', checked_sha: 'c'.repeat(40) },
    'en',
  );
  assert.match(notRetained[0].value, /not found at the current endpoint/);
  assert.match(notRetained[0].value, /does not prove reversion/);

  const unknown = sourceRetentionFields({ state: 'unknown', note: 'shallow clone' }, 'en');
  assert.match(unknown[0].value, /cannot be determined \(shallow clone\)/);

  // 缺 retention 字段的旧结果：明确“工具未提供”，不能当成 retained。
  const missing = sourceRetentionFields(undefined, 'zh');
  assert.equal(missing.length, 1);
  assert.equal(missing[0].label, '当前源码保留状态');
  assert.equal(missing[0].value, '工具未提供');
  // 未知枚举原样显示，不翻译成 retained。
  const future = sourceRetentionFields({ state: 'future_state', note: 'x' }, 'en');
  assert.match(future[0].value, /unknown state: future_state/);
});

it('A1: source_history repo rows include retention, including the missing case', () => {
  const evidence = historyEvidence(
    'source_history',
    historyToolResult({
      query: { symbol: 'x', endpoints: { desktop: { ref: 'main', sha: 'b'.repeat(40) } } },
      repos: {
        desktop: {
          key: 'desktop',
          identity: { state: 'verified', root: '/repos/desktop' },
          retention: { state: 'retained', checked_ref: 'main', checked_sha: 'c'.repeat(40) },
          commits: [],
          notes: [],
          errors: [],
        },
        colm_upstream: {
          key: 'colm_upstream',
          identity: { state: 'verified', root: '/repos/upstream' },
          commits: [],
          notes: [],
          errors: [],
        },
      },
      limits: [],
      truncated: false,
    }),
    'en',
  );
  const desktop = evidence!.sections.find((section) => section.title === 'Desktop repository')!;
  const desktopRetention = desktop.fields.find(
    (field) => field.label === 'current source retention',
  )!;
  assert.match(desktopRetention.value, /still contains lines added/);
  const upstream = evidence!.sections.find(
    (section) => section.title === 'CoLM upstream repository',
  )!;
  const upstreamRetention = upstream.fields.find(
    (field) => field.label === 'current source retention',
  )!;
  assert.equal(upstreamRetention.value, 'not provided by the tool');
});
