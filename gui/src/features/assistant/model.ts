import { bilingual } from '../../lib/bilingual';
export interface ApiProfile {
  base_url: string;
  model: string;
  /** 当前端点/模型/协议的窗口（token）；null = 用内置的已确认预设，未知模型发送前要求填写。 */
  context_window: number | null;
  api_format: string;
  api_options: Record<string, unknown>;
  max_output_tokens: number;
  timeout_seconds: number;
  thinking: boolean | null;
  reasoning_effort: string | null;
  strict: boolean;
}
/** Xerj 文档检索设置：应用级，不随服务商 profile 切换；默认关闭。 */
export interface RetrievalSettings {
  enabled: boolean;
  endpoint: string;
  index: string;
  timeout_ms: number;
}
export interface Settings extends ApiProfile {
  provider_id: string;
  api_profiles: Record<string, ApiProfile>;
  approval: string;
  web_search: boolean;
  /** 每个会话最多多少条用户消息；0 = 不限（B-12）。 */
  session_turn_limit: number;
  /** 每个会话累计 prompt+completion token 上限；0 = 不限。 */
  session_token_limit: number;
  backend: string;
  /** Xerj 文档检索（可选，应用级）：关闭时不访问本机 Xerj。 */
  retrieval: RetrievalSettings;
  egress_acknowledged: string | null;
  external: Record<string, { model: string | null; effort: string | null }>;
}
export const defaultSettings: Settings = {
  base_url: 'https://api.deepseek.com',
  model: 'deepseek-flash',
  context_window: null,
  provider_id: 'deepseek',
  api_format: 'chat_completions',
  api_options: {},
  max_output_tokens: 16384,
  timeout_seconds: 600,
  strict: false,
  api_profiles: {},
  thinking: null,
  reasoning_effort: null,
  approval: 'ask',
  web_search: true,
  session_turn_limit: 100,
  session_token_limit: 5_000_000,
  backend: 'builtin',
  retrieval: {
    enabled: false,
    endpoint: 'http://127.0.0.1:9200',
    index: 'colm-docs-v1',
    timeout_ms: 1500,
  },
  egress_acknowledged: null,
  external: {},
};
// Presets are editable starting points; live model discovery never replaces a typed model.
export const API_PROVIDERS = [
  {
    id: 'deepseek',
    name: 'DeepSeek（Pro / Flash）',
    base_url: 'https://api.deepseek.com',
    model: 'deepseek-flash',
    models: ['deepseek-flash', 'deepseek-v4-pro'],
    api_format: 'chat_completions',
  },
  {
    id: 'openai',
    name: 'OpenAI / ChatGPT API',
    base_url: 'https://api.openai.com/v1',
    model: 'gpt-6.1-sol',
    api_format: 'responses',
  },
  {
    id: 'anthropic',
    name: 'Anthropic / Claude API',
    base_url: 'https://api.anthropic.com/v1',
    model: 'claude-sonnet-5-5',
    api_format: 'anthropic',
  },
  {
    id: 'grok',
    name: 'xAI / Grok',
    base_url: 'https://api.x.ai/v1',
    model: 'grok-4.7',
    api_format: 'chat_completions',
  },
  {
    id: 'glm',
    name: bilingual('智谱', 'GLM'),
    base_url: 'https://open.bigmodel.cn/api/paas/v4',
    model: 'glm-5.3',
    models: ['glm-5.3', 'glm-5.3-flash', 'glm-5.3-flashx'],
    api_format: 'chat_completions',
  },
  {
    id: 'gemini',
    name: 'Google / Gemini',
    base_url: 'https://generativelanguage.googleapis.com/v1beta/openai',
    model: 'gemini-3.8-flash',
    api_format: 'chat_completions',
  },
  {
    id: 'kimi',
    name: 'Moonshot / Kimi',
    base_url: 'https://api.moonshot.cn/v1',
    model: 'kimi-k3',
    api_format: 'chat_completions',
  },
  {
    id: 'qwen',
    name: bilingual('阿里云', 'Qwen'),
    base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1',
    model: 'qwen3.8-flash',
    models: ['qwen3.8-max', 'qwen3.8-flash'],
    api_format: 'chat_completions',
  },
  { id: 'custom', name: '自定义服务', base_url: '', model: '', api_format: 'chat_completions' },
];
export function providerId(settings: Partial<Settings>) {
  if (settings.provider_id && API_PROVIDERS.some((p) => p.id === settings.provider_id))
    return settings.provider_id;
  try {
    const host = new URL(settings.base_url || '').hostname;
    return (
      API_PROVIDERS.find((p) => p.base_url && new URL(p.base_url).hostname === host)?.id || 'custom'
    );
  } catch {
    return 'custom';
  }
}
const PROFILE_FIELDS: (keyof ApiProfile)[] = [
  'base_url',
  'model',
  'context_window',
  'api_format',
  'api_options',
  'max_output_tokens',
  'timeout_seconds',
  'thinking',
  'reasoning_effort',
  'strict',
];
export function rememberProfile(settings: Settings): Settings {
  const id = providerId(settings);
  const profile = Object.fromEntries(
    PROFILE_FIELDS.filter((k) => settings[k] !== undefined).map((k) => [k, settings[k]]),
  );
  return {
    ...settings,
    provider_id: id,
    api_profiles: { ...settings.api_profiles, [id]: profile as unknown as ApiProfile },
  };
}
export function selectProvider(settings: Settings, id: string): Settings {
  const saved = rememberProfile(settings);
  const preset = API_PROVIDERS.find((p) => p.id === id);
  if (!preset) throw new Error('Unknown provider');
  const profile = Object.assign(
    {
      base_url: preset.base_url,
      model: preset.model,
      // 没有为这个服务保存过窗口时不沿用上一个服务的值；预设模型由 agent 用已确认预设解析。
      context_window: null,
      api_format: preset.api_format,
      api_options: {},
      max_output_tokens: 16384,
      timeout_seconds: 600,
      strict: false,
      thinking: null,
      reasoning_effort: null,
    },
    saved.api_profiles?.[id],
  );
  return { ...saved, ...profile, provider_id: id };
}
export function apiEfforts(id: string, model = '') {
  if (id === 'deepseek' && ['deepseek-flash', 'deepseek-v4-pro'].includes(model))
    return ['', 'low', 'high', 'max', 'off'];
  if (id === 'openai' && model === 'gpt-6.1-sol')
    return ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  if (id === 'gemini' && ['gemini-3.7-flash', 'gemini-3.8-flash'].includes(model))
    return ['', 'low', 'medium', 'high'];
  if (id === 'anthropic' && model === 'claude-sonnet-5-5')
    return ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  if (id === 'kimi' && model === 'kimi-k3') return ['', 'low', 'high', 'max'];
  if (id === 'kimi' && model === 'kimi-k2.6') return ['', 'on', 'off'];
  if (id === 'glm' && ['glm-5.3', 'glm-5.3-flash', 'glm-5.3-flashx'].includes(model))
    return ['', 'low', 'high', 'max'];
  if (id === 'qwen' && ['qwen3.8-max', 'qwen3.8-max-0902', 'qwen3.8-flash'].includes(model))
    return ['', 'low', 'medium', 'xhigh', 'off'];
  if (id === 'grok' && ['grok-4.5', 'grok-4.6', 'grok-4.7'].includes(model))
    return model === 'grok-4.5'
      ? ['', 'low', 'medium', 'high']
      : ['', 'low', 'medium', 'high', 'xhigh'];
  // The callers retain a saved effort; unknown model capabilities belong in explicit API options.
  return [''];
}
export function parseApiOptions(text: string): Record<string, unknown> {
  const options = JSON.parse(text.trim() || '{}');
  if (!options || typeof options !== 'object' || Array.isArray(options))
    throw new Error('API options must be a JSON object');
  return options;
}

// Incomplete inactive profiles remain in the open form; the backend only stores usable profiles.
export function completeProfiles(
  profiles: Record<string, ApiProfile> = {},
): Record<string, ApiProfile> {
  return Object.fromEntries(
    Object.entries(profiles).filter(([, profile]) => {
      if (!profile.model?.trim()) return false;
      try {
        return ['https:', 'http:'].includes(new URL(profile.base_url).protocol);
      } catch {
        return false;
      }
    }),
  );
}

export function settingsForSave(draft: Settings, previous: Settings): Settings {
  let settings = { ...draft, api_profiles: completeProfiles(draft.api_profiles) };
  if (settings.backend !== 'builtin' && !completeProfiles({ current: draft }).current) {
    const api =
      completeProfiles({ previous }).previous || selectProvider(defaultSettings, 'deepseek');
    settings = { ...settings, provider_id: providerId(api) };
    for (const key of PROFILE_FIELDS) {
      Object.assign(settings, { [key]: api[key] });
    }
  }
  return settings;
}

export function thinkValue(settings: Settings) {
  return settings.thinking === false
    ? 'off'
    : settings.thinking === true && !settings.reasoning_effort
      ? 'on'
      : settings.reasoning_effort || '';
}
export function thinkSettings(value: string) {
  return ['on', 'off'].includes(value)
    ? { thinking: value === 'on', reasoning_effort: null }
    : { thinking: null, reasoning_effort: value || null };
}
export function withChoice(
  settings: Settings,
  patch: { model?: string; effort?: string },
): Settings {
  const choice = {
    ...(settings.external[settings.backend] || { model: null, effort: null }),
    ...patch,
  };
  return {
    ...settings,
    external: {
      ...settings.external,
      [settings.backend]: { model: choice.model || null, effort: choice.effort || null },
    },
  };
}
export function egressTarget(settings: Settings) {
  const model =
    settings.backend === 'builtin'
      ? settings.base_url
      : settings.backend === 'opencode'
        ? `opencode:${settings.external.opencode?.model || ''}`
        : settings.backend;
  // 联网搜索固定用 DeepSeek 的 Search API（即使会话模型是别家），确认文案必须把它写出来。
  return settings.backend === 'builtin' && settings.web_search
    ? `${model} (web search & fetch: DeepSeek APIs)`
    : model;
}
export interface ModelChoice {
  id: string;
  name?: string;
  efforts?: string[];
  default?: boolean;
  default_effort?: string;
}
export interface AgentEvent {
  type: string;
  id?: string;
  action?: string;
  args?: Record<string, unknown>;
  session?: string;
  model?: string;
  text?: string;
  name?: string;
  arguments?: string;
  tier?: string;
  summary?: string;
  preapproved?: boolean;
  explicit_only?: boolean;
  ok?: boolean;
  result?: string;
  message?: string;
  content?: string;
  task?: Record<string, unknown>;
  prompt_tokens?: number;
  completion_tokens?: number;
  session_prompt_tokens?: number;
  session_completion_tokens?: number;
  session_turns?: number;
  estimated_tokens?: number;
  window_tokens?: number;
  compressed_tools?: number;
  /** 窗口减去估算输入、输出预留与安全余量后还能容纳的输入估算；null/缺省 = 未启用预算（外部后端）。 */
  available_input_tokens?: number | null;
  output_reserve_tokens?: number;
  safety_margin_tokens?: number;
  /** 窗口来源："user" 用户填写 / "preset" 文档确认的预设。 */
  window_source?: string | null;
  limit_reached?: boolean;
  step_limit?: number;
  /** `docs_index_status`：Xerj 文档索引的应用级状态（不随会话切换）。 */
  backend?: string;
  /** 索引状态：disabled|invalid|not_connected|timeout|invalid_response|pending|building|ready|stale|error；
   * 后端将来新增的状态前端原样显示，别吞掉。 */
  state?: string;
  scope?: Record<string, unknown> | null;
  endpoint?: string;
  index?: string;
  documents?: number;
  chunks?: number;
  generation?: number | null;
  indexed_at_unix_ms?: number | null;
  detail?: string | null;
}
export interface Message {
  kind: string;
  text?: string;
  id?: string;
  name?: string;
  arguments?: string;
  result?: string | null;
  ok?: boolean;
  task?: Record<string, unknown>;
}
export function appendDelta(messages: Message[], kind: string, text: string): Message[] {
  const last = messages.at(-1);
  return last?.kind === kind
    ? [...messages.slice(0, -1), { ...last, text: (last.text || '') + text }]
    : [...messages, { kind, text }];
}
/**
 * 撤回最后一条用户气泡：这一轮在模型开始前就被拒绝（GUI-18）。没有用户消息时原样返回，
 * 避免迟到的错误事件误删助手自己的输出。
 */
export function withoutLastUser(messages: Message[]): Message[] {
  const at = messages.map((item) => item.kind).lastIndexOf('user');
  return at >= 0 ? [...messages.slice(0, at), ...messages.slice(at + 1)] : messages;
}
export function historyInput(
  history: string[],
  index: number | null,
  draft: string,
  direction: -1 | 1,
) {
  const next = Math.max(0, Math.min(history.length, (index ?? history.length) + direction));
  return { index: next === history.length ? null : next, value: history[next] ?? draft };
}

export const EXAMPLE_STARTERS = [
  { id: 'CN-Cng', zh: '草地入门 · CN-Cng', en: 'Grassland introduction · CN-Cng' },
  { id: 'AT-Neu', zh: '甲烷过程 · AT-Neu', en: 'Methane processes · AT-Neu' },
  { id: 'US-Ne3', zh: '作物模拟 · US-Ne3', en: 'Crop simulation · US-Ne3' },
  { id: 'AU-Preston', zh: '城市模拟 · AU-Preston', en: 'Urban simulation · AU-Preston' },
] as const;
export function examplePrompt(id: string, language: string) {
  const example = EXAMPLE_STARTERS.find((item) => item.id === id);
  if (!example) throw new Error('Unknown bundled example');
  return language === 'en'
    ? `I would like to try the bundled ${example.en} example. Guide me through selecting the model settings, loading its data, creating a case and checking prerequisites. Let me review the settings before starting a simulation.`
    : `我想试试自带的${example.zh}示例。请通过界面引导我选择模型设置、加载数据、创建算例并检查运行条件；实际开始计算前让我核对设置。`;
}
export function providerLabel(provider: (typeof API_PROVIDERS)[number], language: string) {
  if (language !== 'en') return provider.name;
  return (
    (
      {
        custom: 'Custom provider',
        glm: 'Zhipu / GLM',
        qwen: 'Alibaba Cloud / Qwen',
        deepseek: 'DeepSeek (Pro / Flash)',
      } as Record<string, string>
    )[provider.id] ?? provider.name
  );
}

export function assistantModels(settings: Settings, activeModel = '') {
  const preset = API_PROVIDERS.find((item) => item.id === providerId(settings));
  const chosen =
    settings.backend === 'builtin'
      ? settings.model
      : settings.external[settings.backend]?.model || '';
  return [
    ...new Set([
      chosen,
      ...(settings.backend === 'builtin'
        ? (preset?.models ?? (preset?.model ? [preset.model] : []))
        : ['', activeModel]),
    ]),
  ].filter((value, index) => value || index === 0);
}
/**
 * 直接改接口三要素（地址/模型/协议）后，旧窗口不再对应当前配置，必须清空；
 * 切换服务商走 selectProvider 恢复完整匹配的 profile 时才保留各自保存的值。
 * 其它字段（输出上限、思考、超时）不代表端点换了，窗口不动。
 */
export function withContextWindowCleared(settings: Settings, patch: Partial<ApiProfile>): Settings {
  return { ...settings, ...patch, context_window: null };
}
export function selectAssistantModel(settings: Settings, model: string) {
  if (settings.backend !== 'builtin') return withChoice(settings, { model });
  const effort = thinkValue(settings);
  // 换模型等于换了端点能力，旧的窗口不能再沿用（外部后端的模型不占此字段）。
  return withContextWindowCleared(settings, {
    model,
    ...(!apiEfforts(providerId(settings), model).includes(effort)
      ? { thinking: null, reasoning_effort: null }
      : {}),
  });
}

/**
 * 助手在开始这一轮之前就拒绝了这条消息（没有记进会话）：会话到上限、上一条还在答、没有登记 Key。
 * 这时前端要撤回刚显示的气泡、恢复输入与附件（GUI 审查 D-06）。按 colm-agent 的固定英文识别。
 */
export function rejectedBeforeTurn(message: string) {
  const text = message.replace(/^(?:Error: )+/, '');
  return (
    text.startsWith('session limit reached:') ||
    text.includes('still answering the previous message') ||
    text.includes('no API key is stored') ||
    text.includes('no API key is configured')
  );
}

// Xerj 文档检索由 colm-agent 维护；这些状态经 docs_index_status 事件到达。只翻译已知状态，
// 后端新增取值原样显示（总比显示空白或误报"就绪"好）。
const RETRIEVAL_STATES: Record<string, { zh: string; en: string }> = {
  disabled: { zh: '未启用', en: 'Disabled' },
  invalid: { zh: '设置无效', en: 'Invalid settings' },
  not_connected: { zh: '无法连接 Xerj', en: 'Cannot reach Xerj' },
  timeout: { zh: '连接 Xerj 超时', en: 'Xerj request timed out' },
  invalid_response: { zh: 'Xerj 响应无法解析', en: 'Unreadable Xerj response' },
  pending: { zh: '索引待构建', en: 'Index pending' },
  building: { zh: '正在构建索引', en: 'Building index' },
  ready: { zh: '索引就绪', en: 'Index ready' },
  stale: { zh: '索引已过期', en: 'Index stale' },
  error: { zh: '索引出错', en: 'Index error' },
};

/** `docs_index_status.state` 的中英文字；未知状态原样返回。 */
export function retrievalStateText(state: string, language: string) {
  const text = RETRIEVAL_STATES[state];
  if (!text) return state;
  return language === 'en' ? text.en : text.zh;
}

/**
 * 前端按后端 validate_retrieval 的同一套规则先校验 Xerj 设置（关闭也要校验）：
 * 只接受本机回环 http + 显式端口，索引名安全，超时 100–10000 ms。合法返回 null。
 */
export function retrievalSettingsProblem(
  settings: RetrievalSettings,
  language: string,
): string | null {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  let url: URL;
  try {
    url = new URL(settings.endpoint.trim());
  } catch {
    return t('Xerj 服务地址不是有效的网址。', 'The Xerj service URL is not a valid URL.');
  }
  // URL.hostname 对 IPv6 通常带方括号（"[::1]"）；个别实现会去掉，两种写法都接受。
  const loopback = ['localhost', '127.0.0.1', '[::1]', '::1'].includes(url.hostname);
  if (url.protocol !== 'http:' || !loopback)
    return t(
      'Xerj 服务地址只能是本机回环的 http://127.0.0.1、http://localhost 或 http://[::1]。',
      'The Xerj service URL must be loopback http://127.0.0.1, http://localhost or http://[::1].',
    );
  // URL 的 port 为空串表示没有显式端口（http 的默认 80 会被规范化掉，Xerj 也不用 80）。
  if (!url.port)
    return t(
      'Xerj 服务地址必须写明端口，例如 http://127.0.0.1:9200。',
      'The Xerj service URL must include an explicit port, for example http://127.0.0.1:9200.',
    );
  if (url.username || url.password || url.pathname !== '/' || url.search || url.hash)
    return t(
      'Xerj 服务地址不能含用户名、密码、路径、查询参数或片段（可带末尾斜杠）。',
      'The Xerj service URL must not contain credentials, a path, query parameters or a fragment (a trailing slash is allowed).',
    );
  if (!/^[a-z0-9][a-z0-9_-]{0,63}$/.test(settings.index))
    return t(
      'Xerj 索引名只能含小写字母、数字、下划线或连字符，1–64 字节，且以字母或数字开头。',
      'The Xerj index name may only contain lowercase letters, digits, underscores or hyphens, 1–64 bytes, and must start with a letter or digit.',
    );
  if (
    !Number.isSafeInteger(settings.timeout_ms) ||
    settings.timeout_ms < 100 ||
    settings.timeout_ms > 10_000
  )
    return t(
      'Xerj 检索超时须为 100–10000 毫秒的整数。',
      'The Xerj retrieval timeout must be an integer between 100 and 10000 milliseconds.',
    );
  return null;
}
