import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  defaultSettings,
  llmEndpointDecision,
  normalizeModelEndpoint,
  parseApiOptions,
  rememberProfile,
  retrievalSettingsProblem,
  settingsForSave,
  withContextWindowCleared,
  type LlmEndpointGuard,
  type Settings,
} from './model';

/**
 * 天河 LLM 底线：从 Settings.tsx 抽出真实的保存/改地址回调，用可控 guard 响应执行，
 * 断言 blocked 不写盘、自定义端点需确认、ack 只对精确地址有效、外部后端必须确认。
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
function compile<T>(name: string, context: Record<string, unknown>): T {
  const body = functionBody(name);
  const js = ts.transpileModule(`const run = (${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return run;`)(
    ...Object.values(context),
  ) as T;
}

function draftWith(patch: Partial<Settings>): Settings {
  return { ...defaultSettings, ...patch };
}

function saveHarness(options: {
  draft: Settings;
  guard?: LlmEndpointGuard | Error;
  language?: 'zh' | 'en';
}) {
  const requests: { command: string; args: Record<string, unknown> }[] = [];
  const state = {
    saved: null as Settings | null,
    saving: [] as boolean[],
    status: '',
    draft: null as Settings | null,
    options: '',
    guard: null as LlmEndpointGuard | null,
    guardError: '',
  };
  const invoke = async (command: string, args: Record<string, unknown> = {}) => {
    requests.push({ command, args });
    if (command === 'assistant_llm_endpoint_guard') {
      const guard = options.guard ?? { verdict: 'allowed', host: 'api.deepseek.com' };
      if (guard instanceof Error) throw guard;
      return guard;
    }
    return undefined;
  };
  const context = {
    key: '',
    retrievalKey: '',
    retrievalMode: 'external',
    draft: options.draft,
    settings: defaultSettings,
    state: { language: options.language ?? 'en' },
    options: JSON.stringify(options.draft.api_options, null, 2),
    invoke,
    // 保存后的托管节点状态刷新（F-A-03）；这些用例不涉及 managed 检索，保持 false 不产生额外 IPC。
    hasBackend: () => false,
    setXerjNode: () => {},
    setEndpointGuard: (value: LlmEndpointGuard | null) => {
      state.guard = value;
    },
    setEndpointGuardError: (value: string) => {
      state.guardError = value;
    },
    setSaving: (value: boolean) => {
      state.saving.push(value);
    },
    setDraft: (value: Settings) => {
      state.draft = value;
    },
    setOptions: (value: string) => {
      state.options = value;
    },
    setStatus: (value: string) => {
      state.status = value;
    },
    onSave: async (next: Settings) => {
      state.saved = next;
    },
    t: (zh: string, en: string) => (options.language === 'zh' ? zh : en),
    settingsForSave,
    rememberProfile,
    parseApiOptions,
    retrievalSettingsProblem,
    normalizeModelEndpoint,
    llmEndpointDecision,
    withContextWindowCleared,
    TIANHE_LLM_POLICY_ZH:
      '这是软件底线：禁止在天河上直接运行 DeepSeek 等 LLM。请改用官方云服务或其他机器。',
    TIANHE_LLM_POLICY_EN:
      'This is a hard policy: DeepSeek and other LLMs must not be run directly on Tianhe. Use an official cloud service or another machine.',
  };
  return {
    state,
    requests,
    saveDraft: compile<() => Promise<Settings>>('saveDraft', context),
  };
}

test('blocked Tianhe endpoint refuses to save and never calls onSave', async () => {
  const blocked = {
    verdict: 'blocked',
    host: 'llm.tianhe.example',
    known_public: false,
    matched_tianhe: true,
    message: '检测到模型服务端点疑似位于天河（llm.tianhe.example）',
  };
  const harness = saveHarness({
    draft: draftWith({ base_url: 'https://llm.tianhe.example/v1' }),
    guard: blocked,
  });
  await assert.rejects(harness.saveDraft(), /hard policy|软件底线/i);
  assert.equal(harness.state.saved, null, 'blocked 不写设置');
  assert.deepEqual(harness.requests[0], {
    command: 'assistant_llm_endpoint_guard',
    args: { baseUrl: 'https://llm.tianhe.example/v1' },
  });
  assert.deepEqual(harness.state.guard, blocked);
  // 不提供绕过：确认勾选也进不来（保存前置校验直接按 guard 拒绝）。
  assert.match(settingsSource, /TIANHE_LLM_POLICY_ZH/);
  assert.match(settingsSource, /disabled=\{endpointBlocked\}/, 'blocked 时保存按钮禁用');
});

test('a custom endpoint must be confirmed before saving', async () => {
  const harness = saveHarness({
    draft: draftWith({ base_url: 'https://custom.example/v1' }),
    guard: { verdict: 'needs_confirmation', host: 'custom.example' },
  });
  await assert.rejects(harness.saveDraft(), /Confirm first|勾选确认/i);
  assert.equal(harness.state.saved, null, '未确认不写设置');
  assert.equal(harness.state.saving.length, 0);
});

test('confirming a custom endpoint saves the normalized URL as the ack', async () => {
  const harness = saveHarness({
    draft: draftWith({
      base_url: ' https://Custom.Example/v1/ ',
      model_endpoint_ack: 'https://Custom.Example/v1/',
    }),
    guard: { verdict: 'needs_confirmation', host: 'custom.example' },
  });
  const next = await harness.saveDraft();
  assert.deepEqual(harness.requests[0], {
    command: 'assistant_llm_endpoint_guard',
    args: { baseUrl: 'https://custom.example/v1' },
  });
  assert.equal(next.base_url, 'https://custom.example/v1');
  assert.equal(next.model_endpoint_ack, 'https://custom.example/v1');
  assert.equal(next.external_llm_ack, false, 'builtin 不要求外部后端确认');
  assert.deepEqual(harness.state.saved, next);
  assert.deepEqual(harness.state.saving, [true, false]);
});

test('changing the service URL clears the endpoint ack (only the exact address counts)', () => {
  const patch = (value: Partial<Settings>) => {
    captured.push(value);
  };
  const captured: Partial<Settings>[] = [];
  const patchServiceUrl = compile<(value: string) => void>('patchServiceUrl', {
    draft: draftWith({
      base_url: 'https://custom.example/v1',
      model_endpoint_ack: 'https://custom.example/v1',
    }),
    patch,
    withContextWindowCleared,
  });
  patchServiceUrl('https://other.example/v1');
  assert.equal(captured.length, 1);
  assert.equal(captured[0].model_endpoint_ack, null, '改地址必须清空 ack');
  assert.equal(captured[0].base_url, 'https://other.example/v1');
  assert.equal(captured[0].context_window, null);
});

test('allowed official cloud endpoints need no confirmation and clear stale acks', async () => {
  const harness = saveHarness({
    draft: draftWith({
      base_url: 'https://api.deepseek.com/',
      model_endpoint_ack: 'https://old.example/v1',
    }),
    guard: { verdict: 'allowed', host: 'api.deepseek.com', known_public: true },
  });
  const next = await harness.saveDraft();
  assert.equal(next.base_url, 'https://api.deepseek.com');
  assert.equal(next.model_endpoint_ack, null, '官方云不需要 ack');
  assert.equal(harness.state.saved, next);
});

test('external backends require external_llm_ack and do not call the endpoint guard', async () => {
  const unacked = saveHarness({
    draft: draftWith({ backend: 'codex', external_llm_ack: false }),
  });
  await assert.rejects(unacked.saveDraft(), /Confirm first|勾选确认/i);
  assert.equal(unacked.state.saved, null);
  assert.deepEqual(
    unacked.requests.filter((request) => request.command === 'assistant_llm_endpoint_guard'),
    [],
    '外部后端没有可探测端点，不调用 guard',
  );

  const acked = saveHarness({
    draft: draftWith({ backend: 'codex', external_llm_ack: true }),
  });
  const next = await acked.saveDraft();
  assert.equal(next.external_llm_ack, true);
  assert.equal(next.model_endpoint_ack, null);
  assert.equal(acked.state.saved, next);
});

test('a failed guard call is shown and blocks the save', async () => {
  const harness = saveHarness({
    draft: draftWith({ base_url: 'https://custom.example/v1' }),
    guard: new Error('guard is unavailable'),
  });
  await assert.rejects(harness.saveDraft(), /guard is unavailable/);
  assert.equal(harness.state.saved, null);
});

test('guard verdicts are handled conservatively when unknown or missing', () => {
  assert.equal(
    llmEndpointDecision(null, 'https://x.example/v1', null).verdict,
    'needs_confirmation',
  );
  assert.equal(
    llmEndpointDecision({ verdict: 'future_verdict' }, 'https://x.example/v1', null).verdict,
    'needs_confirmation',
  );
  const blocked = llmEndpointDecision({ verdict: 'blocked' }, 'https://x', null);
  assert.equal(blocked.verdict, 'blocked');
  assert.equal(blocked.ack, null, 'blocked 永远不写 ack');
  const confirmed = llmEndpointDecision(
    { verdict: 'needs_confirmation' },
    'https://X.Example/v1/',
    'https://x.example/v1',
  );
  assert.equal(confirmed.confirmed, true);
  assert.equal(confirmed.ack, 'https://x.example/v1');
  // 旧配置缺字段：ack undefined 视为未确认。
  assert.equal(
    llmEndpointDecision({ verdict: 'needs_confirmation' }, 'https://x.example/v1', undefined)
      .confirmed,
    false,
  );
});

test('endpoint normalization matches the backend ack comparison shape', () => {
  assert.equal(
    normalizeModelEndpoint(' HTTPS://API.DeepSeek.COM/v1/ '),
    'https://api.deepseek.com/v1',
  );
  assert.equal(normalizeModelEndpoint('https://api.deepseek.com/'), 'https://api.deepseek.com');
  assert.equal(normalizeModelEndpoint('https://api.deepseek.com'), 'https://api.deepseek.com');
  assert.equal(normalizeModelEndpoint('http://127.0.0.1:8080/v1//'), 'http://127.0.0.1:8080/v1');
  assert.equal(normalizeModelEndpoint('https://x.example/v1?token=1#frag'), 'https://x.example/v1');
  assert.equal(normalizeModelEndpoint(''), '');
  // 解析不了的输入退回空白/末尾斜杠处理（保存前 guard 会拒绝非法地址）。
  assert.equal(normalizeModelEndpoint('  not a url/ '), 'not a url');
});

test('old settings missing the new ack fields default to unconfirmed', () => {
  assert.equal(defaultSettings.model_endpoint_ack, null);
  assert.equal(defaultSettings.external_llm_ack, false);
  const legacy = { ...defaultSettings, model_endpoint_ack: undefined, external_llm_ack: undefined };
  assert.equal(
    llmEndpointDecision({ verdict: 'needs_confirmation' }, 'https://x', legacy.model_endpoint_ack)
      .confirmed,
    false,
  );
  assert.notEqual(legacy.external_llm_ack, true);
});

test('the settings panel renders the endpoint policy area in both languages', () => {
  assert.match(settingsSource, /assistant_llm_endpoint_guard/);
  assert.match(settingsSource, /TIANHE_LLM_POLICY_ZH/, 'blocked 红线文案');
  assert.match(settingsSource, /LLM_ENDPOINT_CONFIRM_ZH/, '自定义端点必选确认');
  assert.match(settingsSource, /EXTERNAL_LLM_CONFIRM_ZH/, '外部后端必选确认');
  assert.match(settingsSource, /LLM_SCOPE_NOTE_ZH/, '只限制 LLM 端点、不影响远程模拟的说明');
  assert.match(settingsSource, /模型服务端点（天河底线）/);
  assert.match(settingsSource, /官方云服务：不需要额外确认/);
  assert.match(settingsSource, /正在检查模型端点/);
  assert.match(settingsSource, /endpointGuardError/, 'guard 失败原样显示');
  assert.match(settingsSource, /external_llm_ack/, '外部后端确认写进设置');
  assert.match(settingsSource, /model_endpoint_ack: null/, '改地址清空 ack');
  assert.match(settingsSource, /disabled=\{endpointBlocked\}/, 'blocked 保存禁用');
});

test('refuses budgets above the supported upper bounds before saving', async () => {
  const harness = saveHarness({
    draft: draftWith({ max_output_tokens: 2_000_000 }),
  });
  await assert.rejects(harness.saveDraft(), /1000000|Output limit/i);
  assert.equal(harness.state.saved, null);
});

test('session limits use the same upper bounds as the backend', async () => {
  // 后端 gui/src-tauri/src/assistant.rs validate_settings：轮数 ≤100000、累计 token ≤1000000000。
  for (const budget of [{ session_turn_limit: 100_001 }, { session_token_limit: 1_000_000_001 }]) {
    const harness = saveHarness({ draft: draftWith(budget) });
    await assert.rejects(harness.saveDraft(), /100000|Session limits|会话/i);
    assert.equal(harness.state.saved, null, '超界不得写盘');
  }
  // 边界值本身要能保存（100000 轮 = 后端允许的最大值）。
  const accepted = saveHarness({ draft: draftWith({ session_turn_limit: 100_000 }) });
  await accepted.saveDraft();
  assert.equal(accepted.state.saved?.session_turn_limit, 100_000);
});

test('fixed backend validation errors are bilingual in the English UI', () => {
  const compileError = (language: 'zh' | 'en') =>
    compile<(error: unknown) => string>('settingsError', {
      t: (zh: string, en: string) => (language === 'zh' ? zh : en),
    });
  const english = compileError('en');
  assert.match(english(new Error('会话上限过大')), /Session limits exceed/);
  assert.match(
    english(new Error('超时须为 1–3600 秒，最大输出须为 1–1000000 token')),
    /Timeout must be 1–3600/,
  );
  const chinese = compileError('zh');
  assert.match(chinese(new Error('会话上限过大')), /每会话最多 100000 轮/);
  // A-06/A-05 残余：地址、模型能力、历史路径与天河 guard 的固定中文也要翻译。
  assert.match(english(new Error('服务地址不是有效的网址')), /not a valid URL/);
  assert.match(
    english(new Error('服务地址必须是 https://，或本机的 http://127.0.0.1 / localhost')),
    /must use https:\/\//,
  );
  assert.match(
    english(new Error('gpt-6.1-sol 的助手工具调用须使用 Responses 接口')),
    /gpt-6.1-sol tool calls require the Responses API/,
  );
  assert.match(
    english(new Error('claude-sonnet-5-5 须使用 Anthropic Messages 接口')),
    /claude-sonnet-5-5 requires the Anthropic Messages API/,
  );
  assert.match(
    english(new Error('qwen3.8 不支持思考强度 xhigh；可用：low、medium、high')),
    /does not support reasoning effort xhigh; available: low、medium、high/,
  );
  assert.match(
    english(new Error('Desktop 历史仓库路径不存在或不是目录：/missing')),
    /Desktop history repository path does not exist or is not a directory: \/missing/,
  );
  assert.match(
    english(
      new Error(
        '检测到模型服务端点疑似位于天河（llm.tianhe.example）。软件底线：禁止在天河上直接运行 DeepSeek 等 LLM；请改用官方云服务或其他机器。',
      ),
    ),
    /appears to be on Tianhe/,
  );
});

test('web-search copy matches the DeepSeek tools and the external backends', () => {
  // 内置后端（任意回答服务商）的搜索/读网页由 DeepSeek 提供、用 DeepSeek Key（GUI 审查 A-05/A-13）。
  assert.match(settingsSource, /联网搜索与读网页（DeepSeek API）/);
  assert.match(settingsSource, /与当前回答服务商无关/);
  // 外部后端用自带联网，不产生 DeepSeek 费用，文案必须说清。
  assert.match(settingsSource, /外部后端自带的联网/);
  assert.match(settingsSource, /不使用 DeepSeek Key、也不产生 DeepSeek 费用/);
  // 旧文案与实际实现不符，不能回退。
  assert.doesNotMatch(settingsSource, /联网能力由所选服务商/);
  assert.doesNotMatch(settingsSource, /providerId\(settings\) === 'deepseek'/);
});
