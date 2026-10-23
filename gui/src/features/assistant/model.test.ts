import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  appendDelta,
  apiEfforts,
  assistantModels,
  selectAssistantModel,
  API_PROVIDERS,
  defaultSettings,
  historyInput,
  parseApiOptions,
  selectProvider,
  settingsForSave,
  thinkSettings,
  thinkValue,
  withChoice,
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
  assert.deepEqual(next.api_options, {});
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
