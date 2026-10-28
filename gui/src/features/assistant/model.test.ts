import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  rejectedBeforeTurn,
  appendDelta,
  apiEfforts,
  assistantModels,
  selectAssistantModel,
  API_PROVIDERS,
  defaultSettings,
  historyInput,
  parseApiOptions,
  rememberProfile,
  retrievalSettingsProblem,
  retrievalStateText,
  selectProvider,
  settingsForSave,
  thinkSettings,
  thinkValue,
  withChoice,
  withContextWindowCleared,
  EXAMPLE_STARTERS,
  examplePrompt,
  providerLabel,
  egressTarget,
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

it('defaults Xerj retrieval off and keeps it out of provider profiles', () => {
  assert.deepEqual(defaultSettings.retrieval, {
    enabled: false,
    endpoint: 'http://127.0.0.1:9200',
    index: 'colm-docs-v1',
    timeout_ms: 1500,
  });
  const saved = rememberProfile({
    ...defaultSettings,
    retrieval: {
      enabled: true,
      endpoint: 'http://localhost:9200',
      index: 'docs-v2',
      timeout_ms: 2000,
    },
  });
  // 检索是应用级设置：不能进服务商 profile，否则切服务商会被旧 profile 覆盖。
  assert.equal('retrieval' in (saved.api_profiles.deepseek as object), false);
  // 切服务商不丢检索设置；保存（含 profile 归一）也原样保留。
  const switched = selectProvider(saved, 'openai');
  assert.deepEqual(switched.retrieval, saved.retrieval);
  const forSave = settingsForSave(
    { ...switched, retrieval: { ...switched.retrieval, index: 'docs-v3' } },
    saved,
  );
  assert.deepEqual(forSave.retrieval, {
    enabled: true,
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
