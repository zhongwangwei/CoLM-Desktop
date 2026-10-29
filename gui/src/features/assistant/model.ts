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
/** Xerj 节点的接入方式：external = 用户自己运行；managed = 应用启动并管理。 */
export type XerjMode = 'external' | 'managed';
/** Xerj 文档检索设置：应用级，不随服务商 profile 切换；默认关闭。 */
export interface RetrievalSettings {
  enabled: boolean;
  /** 接入方式；旧配置缺这个字段时按 external 处理。 */
  mode: XerjMode;
  /** managed 模式的可选二进制路径；null/空 = 依次找 COLM_XERJ_BINARY、资源目录、PATH。 */
  binary_path: string | null;
  /** managed 模式的可选端口；null = 自动选连续 3 个空闲 loopback 端口。 */
  port: number | null;
  endpoint: string;
  index: string;
  timeout_ms: number;
}
/** 只读历史数据的默认范围：auto = 按问题路由到仓库；其余是用户显式指定的范围。 */
export type HistoryScope = 'auto' | 'selected' | 'desktop' | 'upstream' | 'both';
/** 双仓库历史（U01/U02）：scope 随助手设置保存，两个路径是用户已有的本地仓库（null = 用应用管理副本）。 */
export interface HistorySettings {
  scope: HistoryScope;
  desktop_path: string | null;
  upstream_path: string | null;
}
export const defaultHistorySettings: HistorySettings = {
  scope: 'auto',
  desktop_path: null,
  upstream_path: null,
};
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
  /** 只读历史仓库的默认查询范围（应用级）；旧配置缺字段时按 auto/null/null。 */
  history: HistorySettings;
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
    mode: 'external',
    binary_path: null,
    port: null,
    endpoint: 'http://127.0.0.1:9200',
    index: 'colm-docs-v1',
    timeout_ms: 1500,
  },
  history: { ...defaultHistorySettings },
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
  /** 事件所属的轮次/请求身份（后端若提供）。迟到事件靠它与当前待发送轮次对齐（U05）。 */
  turn?: number | string;
  turn_id?: number | string;
  request_id?: number | string;
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

/** 控制字符会被当成命令行参数或路径的一部分，必须显式拒绝；不用正则以免触发 no-control-regex。 */
function hasControlCharacters(text: string) {
  return [...text].some((char) => {
    const code = char.codePointAt(0) ?? 0;
    return code < 0x20 || code === 0x7f;
  });
}

/**
 * 前端按后端 validate_retrieval 的同一套规则先校验 Xerj 设置（关闭也要校验）：
 * 接入方式与托管字段合法，endpoint 始终是本机回环 http + 显式端口（managed 也存一个回退值），
 * 索引名安全，超时 100–10000 ms。合法返回 null。
 */
export function retrievalSettingsProblem(
  settings: RetrievalSettings,
  language: string,
): string | null {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  // 旧后端返回的设置可能还没有 mode 字段；按 external 处理，与后端"缺字段按 external"一致。
  const mode: XerjMode = settings.mode ?? 'external';
  if (mode !== 'external' && mode !== 'managed')
    return t(
      'Xerj 接入方式只能是使用本机节点（external）或由应用管理（managed）。',
      'The Xerj connection mode must be external (a local node you run) or managed (started by the app).',
    );
  if (mode === 'managed') {
    // binary_path 空 = 自动探测；有值就必须是绝对路径，且不能带控制字符（会作为命令行参数）。
    const rawBinary = settings.binary_path ?? '';
    const binary = rawBinary.trim();
    if (
      hasControlCharacters(rawBinary) ||
      (binary !== '' && !/^(?:\/|[A-Za-z]:[\\/])/.test(binary))
    )
      return t(
        'Xerj 二进制路径必须是绝对路径（以 / 或盘符开头），且不能含控制字符；留空表示自动探测。',
        'The Xerj binary path must be absolute (starting with / or a drive letter) and must not contain control characters; leave it blank to detect automatically.',
      );
    if (
      settings.port !== null &&
      (!Number.isSafeInteger(settings.port) || settings.port < 1024 || settings.port > 65_000)
    )
      return t(
        'Xerj 节点端口须为 1024–65000 的整数；留空表示自动选择。',
        'The Xerj node port must be an integer between 1024 and 65000; leave it blank for automatic selection.',
      );
  }
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

/** `assistant_xerj_node` 返回的托管节点状态；字段与 Tauri 命令一致，缺失/未知值原样显示。 */
export interface XerjNodeStatus {
  mode?: string;
  running?: boolean;
  endpoint?: string | null;
  port?: number | null;
  version?: string | null;
  pinned_version?: string | null;
  pinned_match?: boolean | null;
  binary?: string | null;
  data_dir?: string | null;
  detail?: string | null;
}

/**
 * 托管 Xerj 节点状态的中英文字：运行中/未运行、版本、是否与固定 release 一致、endpoint 与
 * 后端给的 detail。未知 mode 与 detail 原样显示 —— 后端加字段时界面不能把它悄悄吞掉。
 */
export function managedNodeText(status: XerjNodeStatus | null, language: string): string {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  if (!status) return t('节点状态尚未获取', 'Node status has not been loaded');
  if (status.mode === 'external')
    return t(
      '外部节点：使用你自行运行的 Xerj（应用不探测、不启动）',
      'External node: use the Xerj you run yourself (the app does not probe or start it)',
    );
  if (status.mode !== undefined && status.mode !== 'managed')
    return `${t('未知节点模式', 'Unknown node mode')} ${status.mode}${
      status.detail ? ` · ${status.detail}` : ''
    }`;
  const parts = [status.running ? t('运行中', 'Running') : t('未运行', 'Not running')];
  if (status.version) parts.push(`${t('版本', 'version')} ${status.version}`);
  if (status.pinned_match === true) parts.push(t('与固定版本一致', 'matches the pinned release'));
  if (status.pinned_match === false)
    parts.push(
      status.pinned_version
        ? `${t('与固定版本不一致', 'does not match the pinned release')} (${status.pinned_version})`
        : t('与固定版本不一致', 'does not match the pinned release'),
    );
  if (status.endpoint) parts.push(status.endpoint);
  if (status.detail) parts.push(status.detail);
  return parts.join(' · ');
}

// ---------------------------------------------------------------------------------------------
// 只读历史仓库（U01/U02）：status 来自 `assistant_history_status`（colm-cli history-repos），
// 同步来自 `assistant_history_sync`（history-sync）。两者共用同一份 per-repo 形状。
// ---------------------------------------------------------------------------------------------

/** 一次成功 clone/fetch 的记录；legacy synced 也可能是这个形状，但它不是成功证据。 */
export interface HistorySyncRecord {
  ref?: string | null;
  sha?: string | null;
  at_unix_ms?: number | null;
}

export interface HistoryRepoStatus {
  key?: string;
  owner?: string;
  repo?: string;
  url?: string;
  path?: string;
  identity?: {
    state?: string;
    root?: string | null;
    remote?: { url?: string | null; owner?: string | null; repo?: string | null } | null;
  } | null;
  default_branch?: string | null;
  /** 兼容别名，语义同 last_successful_sync。 */
  synced?: HistorySyncRecord | null;
  last_successful_sync?: HistorySyncRecord | null;
  legacy_synced?: HistorySyncRecord | null;
  sync_notes?: string[];
  last_attempt?: { action?: string; at_unix_ms?: number | null; error?: string | null } | null;
  checked_at_unix_ms?: number | null;
  coverage?: {
    shallow?: boolean;
    commits?: number;
    complete_history?: boolean;
    truncated?: boolean;
  } | null;
  errors?: string[];
}

/** `history-repos` / `history-sync` 的顶层形状。 */
export interface HistoryReposStatus {
  history_root?: string;
  sync_semantics?: string;
  repos?: { desktop?: HistoryRepoStatus | null; colm_upstream?: HistoryRepoStatus | null };
}

export interface HistorySyncResult extends HistoryReposStatus {
  actions?: Record<string, string>;
}

export type HistoryRepository = 'desktop' | 'colm_upstream';

const HISTORY_SCOPES: HistoryScope[] = ['auto', 'selected', 'desktop', 'upstream', 'both'];

/** 旧设置没有 history 字段时按默认（auto/null/null）补全，读和保存都不能因此丢字段。 */
export function historySettingsOf(
  settings: { history?: Partial<HistorySettings> | null } | null | undefined,
): HistorySettings {
  const saved = settings?.history;
  return {
    scope: HISTORY_SCOPES.includes(saved?.scope as HistoryScope)
      ? (saved?.scope as HistoryScope)
      : 'auto',
    desktop_path: saved?.desktop_path ?? null,
    upstream_path: saved?.upstream_path ?? null,
  };
}

export function historyScopeLabel(scope: string, language: string) {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  switch (scope) {
    case 'auto':
      return t('自动（按问题）', 'Automatic (by question)');
    case 'selected':
      return t('当前模型', 'Current model');
    case 'desktop':
      return t('Desktop 仓库', 'Desktop repository');
    case 'upstream':
      return t('CoLM 官方', 'CoLM upstream');
    case 'both':
      return t('两个仓库', 'Both repositories');
    default:
      // 后端/旧配置里的未知范围原样显示，不悄悄变成"自动"。
      return scope;
  }
}

export function historyScopeHint(language: string) {
  return language === 'en'
    ? 'Automatic routes by question; your chosen scope takes precedence unless the question explicitly asks for another repository.'
    : '自动会按问题路由仓库；你选的范围优先，除非问题明确要求别的仓库。';
}

/** 只改历史范围，保留其余设置（不碰服务商 profile）。 */
export function withHistoryScope(settings: Settings, scope: HistoryScope): Settings {
  return { ...settings, history: { ...historySettingsOf(settings), scope } };
}

/** 只改一个仓库的本地路径；null = 清掉路径、回到应用管理副本。 */
export function withHistoryPath(
  settings: Settings,
  repository: HistoryRepository,
  path: string | null,
): Settings {
  const history = historySettingsOf(settings);
  return {
    ...settings,
    history: {
      ...history,
      ...(repository === 'desktop' ? { desktop_path: path } : { upstream_path: path }),
    },
  };
}

/** unix 毫秒 → `YYYY-MM-DD HH:MM UTC`。用 UTC 而不是本地时区，跨平台测试与显示都稳定。 */
export function formatHistoryTime(value: number | null | undefined) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) return '';
  const iso = new Date(value).toISOString();
  return `${iso.slice(0, 16).replace('T', ' ')} UTC`;
}

/** 40 hex SHA 取前 7 位；不是 SHA 时原样（最多截断到 40 位以内）返回。 */
export function shortSha(sha: string | null | undefined) {
  if (typeof sha !== 'string') return '';
  const text = sha.trim();
  if (!text) return '';
  return /^[0-9a-f]{7,40}$/i.test(text) ? text.slice(0, 7) : text;
}

export function historyRepositoryLabel(repository: string, language: string) {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  if (repository === 'desktop') return t('Desktop 仓库', 'Desktop repository');
  if (repository === 'colm_upstream') return t('CoLM 官方仓库', 'CoLM upstream repository');
  return repository;
}

export interface HistoryRepoLines {
  /** 身份/可用性：verified/unverified/missing/partial 等（读不懂的状态原样保留）。 */
  state: string;
  /** 上次成功同步（ref + 短 SHA + 时间）。legacy_synced 只写"未证实"，绝不显示为成功。 */
  lastSync: string;
  /** 上次检查时间；与成功同步时间分开显示。 */
  checkedAt: string;
  /** 最后一次尝试的错误与持久错误（去重）。 */
  errors: string[];
  /** partial / 未准备时的一句说明；已就绪为 null。 */
  preparation: string | null;
}

/**
 * 把一个仓库的 status 翻译成设置面板要显示的四段文字。success 与 checked 永远分开：
 * `checked_at_unix_ms` 只是检查时间，`legacy_synced` 是旧版未证实记录，两者都不能说成"同步成功"。
 */
export function historyRepoLines(
  repo: HistoryRepoStatus | null | undefined,
  language: string,
): HistoryRepoLines {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const identityState = repo?.identity?.state;
  const shallow = repo?.coverage?.shallow === true;
  let state: string;
  if (!repo) state = t('状态未知', 'Unknown');
  else if (identityState === 'missing') state = t('缺失', 'Missing');
  else if (identityState === 'unverified') state = t('身份未核验', 'Unverified identity');
  else if (identityState === 'verified' && shallow)
    state = t('部分历史（浅克隆）', 'Partial history (shallow clone)');
  else if (identityState === 'verified') state = t('已核验', 'Verified');
  else state = `${t('身份未知', 'Unknown identity')}：${identityState ?? t('无记录', 'no record')}`;

  const success = repo?.last_successful_sync;
  const legacy = repo?.legacy_synced;
  const successName = success
    ? [success.ref?.trim() || '', shortSha(success.sha)].filter(Boolean).join(' @ ')
    : '';
  let lastSync: string;
  if (success && (successName || success.at_unix_ms))
    lastSync = `${t('上次成功同步', 'Last successful sync')}：${successName}${
      success.at_unix_ms
        ? `${successName ? ' · ' : ''}${formatHistoryTime(success.at_unix_ms)}`
        : ''
    }`;
  else if (legacy)
    lastSync = `${t('旧记录', 'Legacy record')}：${t(
      '未证实同步成功',
      'not proof of a successful sync',
    )}${legacy.at_unix_ms ? `（${t('截至', 'as of')} ${formatHistoryTime(legacy.at_unix_ms)}）` : ''}`;
  else lastSync = t('还没有成功同步记录', 'No successful sync recorded');

  const checkedAt = repo?.checked_at_unix_ms
    ? `${t('上次检查', 'Last checked')}：${formatHistoryTime(repo.checked_at_unix_ms)}`
    : t('还没有检查记录', 'Not checked yet');

  const errors: string[] = [];
  const attemptError = repo?.last_attempt?.error;
  if (typeof attemptError === 'string' && attemptError.trim()) errors.push(attemptError.trim());
  for (const error of repo?.errors ?? [])
    if (typeof error === 'string' && error.trim() && !errors.includes(error.trim()))
      errors.push(error.trim());

  let preparation: string | null = null;
  if (!repo || identityState === 'missing')
    preparation = t(
      '数据目录里还没有这个仓库；点“准备数据”下载后再用。',
      'This repository is not in the data directory yet; use “Prepare data” to download it.',
    );
  else if (!success && legacy)
    preparation = t(
      '只有旧版同步记录，未证实成功；建议重新准备/更新。',
      'Only a legacy sync record exists and it is not proof of success; prepare or update again.',
    );
  else if (errors.length)
    preparation = t(
      '上次更新失败；按上面的错误处理后重试。',
      'The last update failed; fix the error above and retry.',
    );
  else if (identityState === 'unverified')
    preparation = t(
      '仓库身份未核验；请确认路径指向官方仓库。',
      'The repository identity is unverified; check that the path points at the official repository.',
    );
  else if (shallow)
    preparation = t(
      '历史是浅克隆（不完整）；完整历史查询会标 partial。',
      'History is a shallow clone (incomplete); full-history queries will be marked partial.',
    );

  return { state, lastSync, checkedAt, errors, preparation };
}

/** 同步结果（actions）→ 一行提示；未知动作原样显示，只要有一个 error 就整体标失败。 */
export function historySyncSummary(
  result: HistorySyncResult | null,
  language: string,
): { text: string; failed: boolean } | null {
  const actions = result?.actions;
  if (!actions) return null;
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const parts: string[] = [];
  let failed = false;
  for (const repository of ['desktop', 'colm_upstream'] as HistoryRepository[]) {
    const action = actions[repository];
    if (!action) continue;
    let text: string;
    switch (action) {
      case 'fetched':
        text = t('已更新', 'updated');
        break;
      case 'cloned':
        text = t('已克隆', 'cloned');
        break;
      case 'offline':
        text = t('离线（未联网）', 'offline (no network)');
        break;
      case 'skipped':
        text = t('跳过', 'skipped');
        break;
      case 'error':
        text = t('失败', 'failed');
        failed = true;
        break;
      default:
        text = action;
    }
    parts.push(`${historyRepositoryLabel(repository, language)} ${text}`);
  }
  if (!parts.length) return null;
  const prefix = failed
    ? t('同步结果（有失败）', 'Sync result (with failures)')
    : t('同步结果', 'Sync result');
  return { text: `${prefix}：${parts.join('；')}`, failed };
}

const EXTERNAL_BACKEND_LABELS: Record<string, { zh: string; en: string }> = {
  codex: { zh: '本机 Codex', en: 'Local Codex' },
  claude_code: { zh: '本机 Claude Code', en: 'Local Claude Code' },
  opencode: { zh: '本机 OpenCode', en: 'Local OpenCode' },
};

export interface HistoryContextLabel {
  backend: string;
  model: string;
  /** 冻结模型的短 SHA（有提交）或 `id@revision`；官方/应用内置时明确写出来。 */
  revision: string;
}

/**
 * U02 输入框上方的"当前后端/模型/冻结 revision"文字。revision 优先用冻结记录里的
 * code.commit 短 SHA；没有提交时退回 version 的 id@revision；官方/内置则明确显示，不猜 SHA。
 */
export function historyContextLabel(
  settings: Settings,
  version:
    | {
        version?: { id: string; revision: number } | null;
        code?: { commit?: string | null } | null;
      }
    | null
    | undefined,
  versionRef: { id: string; revision: number } | null | undefined,
  language: string,
): HistoryContextLabel {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const preset = API_PROVIDERS.find((item) => item.id === providerId(settings));
  const backend =
    settings.backend === 'builtin'
      ? preset
        ? providerLabel(preset, language)
        : settings.base_url || t('API 服务', 'API service')
      : EXTERNAL_BACKEND_LABELS[settings.backend]
        ? language === 'en'
          ? EXTERNAL_BACKEND_LABELS[settings.backend].en
          : EXTERNAL_BACKEND_LABELS[settings.backend].zh
        : settings.backend;
  const chosen =
    settings.backend === 'builtin'
      ? settings.model
      : settings.external[settings.backend]?.model || '';
  const model = chosen || t('默认模型', 'default model');
  const commit = version?.code?.commit;
  const revision =
    typeof commit === 'string' && commit.trim()
      ? shortSha(commit)
      : version?.version
        ? `${version.version.id}@${version.version.revision}`
        : versionRef && versionRef.id !== 'official'
          ? `${versionRef.id}@${versionRef.revision}`
          : t('官方/应用内置', 'official / built-in');
  return { backend, model, revision };
}

// ---------------------------------------------------------------------------------------------
// U03 只读历史证据卡：把 source_history / commit_details / commit_pull_requests 的 JSON 结果
// 变成结构化、可展开的字段/条目。只读结构字段，绝不根据文字猜作者动机或补造缺字段。
// ---------------------------------------------------------------------------------------------

export type HistoryToolKind = 'source_history' | 'commit_details' | 'commit_pull_requests';
const HISTORY_TOOL_KINDS: HistoryToolKind[] = [
  'source_history',
  'commit_details',
  'commit_pull_requests',
];

/** 识别历史工具名；兼容外部后端的前缀（mcp__colm__source_history、colm/source_history、colm.source_history）。 */
export function historyToolKind(name: string | null | undefined): HistoryToolKind | null {
  const value = (name ?? '').trim();
  if (!value) return null;
  for (const kind of HISTORY_TOOL_KINDS)
    if (
      value === kind ||
      value.endsWith(`__${kind}`) ||
      value.endsWith(`/${kind}`) ||
      value.endsWith(`.${kind}`) ||
      value.endsWith(`:${kind}`)
    )
      return kind;
  return null;
}

export interface EvidenceField {
  label: string;
  value: string;
  /** SHA/时间/证据 ID 等：等宽、可选中复制。 */
  mono?: boolean;
  /** 本地文件路径：只由安全拼接得到，点击用 open_path。 */
  path?: string;
  /** http/https 链接：点击用 assistant_open_url。 */
  url?: string;
}

export interface EvidenceItem {
  title: string;
  fields: EvidenceField[];
  /** 可展开的原文片段（diff/正文/候选 JSON），已做行数/字符限制。 */
  detail?: string;
  detailLabel?: string;
  path?: string;
  url?: string;
}

export interface EvidenceSection {
  title: string;
  fields: EvidenceField[];
  items?: EvidenceItem[];
  note?: string;
}

export interface HistoryEvidence {
  /** 折叠状态下的一行短答案。 */
  summary: string;
  sections: EvidenceSection[];
}

type JsonRecord = Record<string, unknown>;

function asRecord(value: unknown): JsonRecord | null {
  return value && typeof value === 'object' && !Array.isArray(value) ? (value as JsonRecord) : null;
}
function asArray(value: unknown): unknown[] {
  return Array.isArray(value) ? value : [];
}
function asText(value: unknown): string {
  if (typeof value === 'string') return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return '';
}
function asNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}
function yesNo(value: boolean, language: string) {
  return language === 'en' ? (value ? 'yes' : 'no') : value ? '是' : '否';
}
function isAbsoluteLocalPath(text: string) {
  return text.startsWith('/') || /^[A-Za-z]:[\\/]/.test(text);
}
function isHttpUrl(text: string) {
  return /^https?:\/\/\S+$/i.test(text.trim());
}
function field(label: string, value: string, extra: Partial<EvidenceField> = {}): EvidenceField {
  return { label, value, ...extra };
}
function clip(value: string, limit: number) {
  const chars = [...value];
  return chars.length > limit ? `${chars.slice(0, limit).join('')}…` : value;
}

/** unix 秒 → `YYYY-MM-DD HH:MM UTC`（Git 时间戳是秒；毫秒时间轴用 formatHistoryTime）。 */
export function formatUnixSeconds(value: unknown) {
  const seconds = asNumber(value);
  if (seconds === null || seconds <= 0) return '';
  return formatHistoryTime(seconds * 1000);
}

/** 历史仓库里的相对文件路径：拒绝绝对路径、控制字符、URL、`..` 与空段，避免打开任意文件。 */
export function safeHistoryRelativePath(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const text = value.trim();
  if (!text || text.length > 1024 || hasControlCharacters(text)) return null;
  if (isAbsoluteLocalPath(text) || text.includes('://')) return null;
  const parts = text.split(/[\\/]/);
  if (parts.length === 0 || parts.some((part) => part === '' || part === '..' || part === '.'))
    return null;
  return parts.join('/');
}

/** 由可信仓库根目录 + 仓库内相对路径拼出可 open_path 的绝对路径；任一不可信就返回 null。 */
export function historyFileOpenPath(base: unknown, relative: unknown): string | null {
  if (typeof base !== 'string' || hasControlCharacters(base)) return null;
  const root = base.trim();
  if (!isAbsoluteLocalPath(root)) return null;
  const path = safeHistoryRelativePath(relative);
  if (!path) return null;
  return `${root.replace(/[\\/]+$/, '')}/${path}`;
}

/** JSON/文本里的绝对本地路径（U04 查看运行/输出用）；拒绝 URL 与相对路径。 */
export function localPathsInJson(value: unknown, limit = 5): string[] {
  const paths: string[] = [];
  const visit = (node: unknown) => {
    if (paths.length >= limit) return;
    if (typeof node === 'string') {
      const text = node.trim();
      if (isAbsoluteLocalPath(text) && !hasControlCharacters(text) && !paths.includes(text))
        paths.push(text);
      return;
    }
    if (Array.isArray(node)) {
      for (const item of node) visit(item);
      return;
    }
    const record = asRecord(node);
    if (record) for (const item of Object.values(record)) visit(item);
  };
  visit(value);
  return paths;
}

export function localPathsInJsonText(text: string | null | undefined, limit = 5): string[] {
  if (typeof text !== 'string' || !text.trim()) return [];
  try {
    return localPathsInJson(JSON.parse(text), limit);
  } catch {
    // 不是 JSON 的参数/输出原文：只按明显的绝对路径片段提取，绝不猜。
    const paths: string[] = [];
    for (const match of text.matchAll(/["'(\s：:=]((?:\/|[A-Za-z]:[\\/])[^\s"',;)\]}>]+)/g)) {
      const path = match[1].trim();
      if (!path || hasControlCharacters(path) || !isAbsoluteLocalPath(path)) continue;
      if (!paths.includes(path)) paths.push(path);
      if (paths.length >= limit) break;
    }
    return paths;
  }
}

/**
 * 一个历史工具结果 → 证据卡模型。非历史工具、失败结果或非 JSON 文本返回 null，
 * 调用方保留原来的原始 JSON 显示（绝不半解析后编造字段）。
 */
export function historyEvidence(
  name: string | null | undefined,
  result: string | null | undefined,
  language: string,
): HistoryEvidence | null {
  const kind = historyToolKind(name);
  if (!kind || typeof result !== 'string' || !result.trim()) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(result);
  } catch {
    return null;
  }
  const root = asRecord(parsed);
  if (!root) return null;
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  switch (kind) {
    case 'source_history':
      return sourceHistoryEvidence(root, language, t);
    case 'commit_details':
      return commitDetailsEvidence(root, language, t);
    case 'commit_pull_requests':
      return pullRequestsEvidence(root, language, t);
  }
}

function commitReasonFields(
  reason: JsonRecord | null,
  language: string,
  t: (zh: string, en: string) => string,
) {
  if (!reason) return [];
  const status = asText(reason.status);
  const fields = [
    field(
      t('修改理由状态（原文候选，不是动机结论）', 'reason status (raw candidate, not a verdict)'),
      status || t('未提供', 'not provided'),
    ),
  ];
  if (asText(reason.basis)) fields.push(field(t('理由依据', 'reason basis'), asText(reason.basis)));
  if (asText(reason.excerpt))
    fields.push(field(t('原文摘录', 'reason excerpt'), asText(reason.excerpt)));
  const candidates = asArray(reason.evidence_candidates);
  if (candidates.length)
    fields.push(field(t('原文候选数', 'evidence candidates'), String(candidates.length)));
  return fields;
}

function commitReasonItems(
  reason: JsonRecord | null,
  language: string,
  t: (zh: string, en: string) => string,
): EvidenceItem[] {
  const items: EvidenceItem[] = [];
  for (const value of asArray(reason?.evidence_candidates)) {
    const candidate = asRecord(value);
    if (!candidate) continue;
    const fields: EvidenceField[] = [];
    for (const [key, label] of [
      ['source', t('来源', 'source')],
      ['author', t('作者', 'author')],
      ['scope', t('范围', 'scope')],
    ] as [string, string][])
      if (asText(candidate[key])) fields.push(field(label, asText(candidate[key])));
    const url = asText(candidate.url);
    const excerpt = asText(candidate.excerpt);
    items.push({
      title: `${t('原文候选', 'evidence candidate')} · ${
        asText(candidate.source) || t('未知来源', 'unknown source')
      }`,
      fields,
      detail: excerpt ? clip(excerpt, 1200) : undefined,
      detailLabel: t('原文摘录', 'excerpt'),
      url: isHttpUrl(url) ? url : undefined,
    });
  }
  return items;
}

function hunkDiffText(value: unknown, maxLines = 30) {
  const hunk = asRecord(value);
  if (!hunk) return '';
  const header = `@@ -${asNumber(hunk.old_start) ?? 0},${asNumber(hunk.old_lines) ?? 0} +${
    asNumber(hunk.new_start) ?? 0
  },${asNumber(hunk.new_lines) ?? 0} @@`;
  const lines = [header];
  for (const line of asArray(hunk.old_code).map(asText).slice(0, maxLines)) lines.push(`-${line}`);
  for (const line of asArray(hunk.new_code).map(asText).slice(0, maxLines)) lines.push(`+${line}`);
  if (hunk.truncated === true) lines.push('…');
  return clip(lines.join('\n'), 4000);
}

function sourceHistoryEvidence(
  root: JsonRecord,
  language: string,
  t: (zh: string, en: string) => string,
): HistoryEvidence {
  const query = asRecord(root.query) ?? {};
  const repos = asRecord(root.repos) ?? {};
  const endpoints = asRecord(query.endpoints) ?? {};
  const source = asRecord(query.source) ?? {};
  const singletonSourceDir = asText(source.source_dir);
  const overview: EvidenceField[] = [];
  if (asText(query.symbol))
    overview.push(field(t('符号', 'symbol'), asText(query.symbol), { mono: true }));
  if (asText(query.repository))
    overview.push(field(t('仓库参数', 'repository'), asText(query.repository)));
  if (asText(query.path)) overview.push(field(t('路径过滤', 'path filter'), asText(query.path)));
  if (singletonSourceDir)
    overview.push(
      field(t('源码目录', 'source directory'), singletonSourceDir, { path: singletonSourceDir }),
    );
  if (asText(source.mode)) overview.push(field(t('来源模式', 'source mode'), asText(source.mode)));
  if (asText(source.commit))
    overview.push(
      field(t('记录 commit', 'recorded commit'), shortSha(asText(source.commit)), { mono: true }),
    );
  if (typeof source.dirty === 'boolean')
    overview.push(
      field(t('工作区未提交修改', 'working tree dirty'), yesNo(source.dirty, language)),
    );
  const sections: EvidenceSection[] = [{ title: t('查询', 'query'), fields: overview }];

  let locationsTotal = 0;
  let commitsTotal = 0;
  for (const key of ['selected', 'desktop', 'colm_upstream']) {
    const repo = asRecord(repos[key]);
    const endpoint = asRecord(endpoints[key]) ?? asRecord(repo?.endpoint);
    if (!repo && !endpoint) continue;
    const base =
      asText(asRecord(repo?.identity)?.root) || (key === 'selected' ? singletonSourceDir : '');
    const fields: EvidenceField[] = [];
    if (endpoint)
      fields.push(
        field(
          t('查询终点', 'endpoint'),
          `${asText(endpoint.ref) || t('未提供', 'not provided')} @ ${shortSha(asText(endpoint.sha)) || t('未提供', 'not provided')}`,
          { mono: true },
        ),
      );
    const identityState = asText(asRecord(repo?.identity)?.state);
    if (identityState) fields.push(field(t('身份', 'identity'), identityState));
    if (asText(repo?.history))
      fields.push(field(t('历史可用性', 'history'), asText(repo?.history)));
    const items: EvidenceItem[] = [];
    for (const value of asArray(repo?.locations)) {
      const location = asRecord(value);
      if (!location) continue;
      locationsTotal++;
      const path = asText(location.path);
      const itemFields: EvidenceField[] = [];
      if (asText(location.kind)) itemFields.push(field(t('类型', 'kind'), asText(location.kind)));
      if (asText(location.scope))
        itemFields.push(field(t('作用域', 'scope'), asText(location.scope)));
      items.push({
        title: `${path}:${asText(location.line) || '?'}`,
        fields: itemFields,
        path: historyFileOpenPath(base, path) ?? undefined,
      });
    }
    for (const value of asArray(repo?.commits)) {
      const commit = asRecord(value);
      if (!commit) continue;
      commitsTotal++;
      const fields: EvidenceField[] = [];
      if (asText(commit.author)) fields.push(field(t('作者', 'author'), asText(commit.author)));
      const authorTime = formatUnixSeconds(commit.author_time_unix);
      if (authorTime) fields.push(field(t('作者时间', 'author time'), authorTime, { mono: true }));
      const committerTime = formatUnixSeconds(commit.committer_time_unix);
      // 作者时间与提交时间分开显示：rebase/cherry-pick 后它们本来就可能不同。
      if (committerTime)
        fields.push(field(t('提交时间', 'committer time'), committerTime, { mono: true }));
      const relevance = asArray(commit.relevance).map(asText).filter(Boolean);
      if (relevance.length) fields.push(field(t('相关性', 'relevance'), relevance.join(', ')));
      fields.push(...commitReasonFields(asRecord(commit.reason), language, t));
      if (commit.truncated === true)
        fields.push(field(t('截断', 'truncated'), yesNo(true, language)));
      const reasonItems = commitReasonItems(asRecord(commit.reason), language, t);
      items.push({
        title: `${shortSha(asText(commit.sha))} ${asText(commit.summary)}`.trim(),
        fields,
        detail: reasonItems.length
          ? reasonItems
              .map((item) => `${item.title}\n${item.detail ?? ''}`)
              .join('\n\n')
              .trim() || undefined
          : undefined,
        detailLabel: t('理由原文候选', 'reason evidence candidates'),
      });
    }
    for (const value of asArray(repo?.uncommitted)) {
      const entry = asRecord(value);
      if (!entry) continue;
      const path = asText(entry.path);
      const detail = asArray(entry.hunks)
        .map((hunk) => hunkDiffText(hunk))
        .filter(Boolean)
        .join('\n\n');
      items.push({
        title: `${path} · ${t('未提交', 'uncommitted')}`,
        fields:
          entry.uncommitted === true
            ? [field(t('未提交', 'uncommitted'), yesNo(true, language))]
            : [],
        detail: detail ? clip(detail, 4000) : undefined,
        detailLabel: t('未提交 diff（已限制行数）', 'uncommitted diff (line-limited)'),
        path: historyFileOpenPath(base, path) ?? undefined,
      });
    }
    const notes = [...asArray(repo?.notes), ...asArray(repo?.errors)].map(asText).filter(Boolean);
    if (fields.length || items.length || notes.length)
      sections.push({
        title: historyRepositoryLabel(key, language),
        fields,
        items,
        note: notes.length ? notes.join('；') : undefined,
      });
  }

  let crossCount = 0;
  const links = asArray(asRecord(root.cross)?.links);
  if (links.length) {
    const items: EvidenceItem[] = [];
    for (const value of links) {
      const link = asRecord(value);
      if (!link) continue;
      crossCount++;
      const state = asText(link.state);
      const fields: EvidenceField[] = [
        field(t('对应状态', 'state'), state || t('未提供', 'not provided')),
        field(
          t('依据', 'basis'),
          asArray(link.basis).map(asText).filter(Boolean).join(', ') || t('无', 'none'),
        ),
      ];
      const upstream = asRecord(link.upstream);
      if (upstream)
        fields.push(
          field(
            t('上游', 'upstream'),
            `${asText(upstream.repository)} ${asText(upstream.commit)} ${asText(upstream.path)}`.trim(),
            { mono: true },
          ),
        );
      const desktop = asRecord(link.desktop);
      if (desktop) {
        const importCommit = asText(desktop.import_commit);
        fields.push(
          field(
            t('Desktop 侧', 'desktop'),
            `${asText(desktop.vendor_path)}${
              importCommit ? ` @ ${shortSha(importCommit)}` : ''
            }`.trim(),
          ),
        );
        if (asText(desktop.vendor_path))
          fields.push(field(t('vendor 路径', 'vendor path'), asText(desktop.vendor_path)));
      }
      const candidates = asArray(link.candidates);
      if (candidates.length)
        fields.push(field(t('候选数', 'candidates'), String(candidates.length)));
      const notes = asArray(link.notes).map(asText).filter(Boolean);
      items.push({
        title: [
          state || t('未映射', 'unmapped'),
          asText(upstream?.path) || asText(desktop?.vendor_path),
        ]
          .filter(Boolean)
          .join(' · '),
        fields,
        detail: candidates.length
          ? clip(candidates.map((candidate) => JSON.stringify(candidate)).join('\n'), 4000)
          : undefined,
        detailLabel: t('候选（结构化原文）', 'candidates (structured)'),
        path:
          historyFileOpenPath(
            asText(asRecord(asRecord(repos.desktop)?.identity)?.root),
            asText(desktop?.vendor_path),
          ) ?? undefined,
      });
      if (notes.length)
        items[items.length - 1].fields.push(field(t('说明', 'notes'), notes.join('；')));
    }
    sections.push({
      title: t('跨仓库对应', 'cross-repository links'),
      fields: [],
      items,
      note: t(
        'verified 只来自正式 Git trailer；candidate 是候选不是已证实来源；unmapped 表示无法对应',
        'verified only comes from a formal Git trailer; candidate is not a proven source; unmapped means no mapping',
      ),
    });
  }

  const limits = asArray(root.limits).map(asText).filter(Boolean);
  if (root.truncated === true || limits.length)
    sections.push({
      title: t('范围与截断', 'limits and truncation'),
      fields: [field(t('截断', 'truncated'), yesNo(root.truncated === true, language))],
      note: limits.join('；') || undefined,
    });

  const symbol = asText(query.symbol) || t('未提供符号', 'no symbol');
  const summary = [
    symbol,
    `${locationsTotal} ${t('处位置', 'locations')}`,
    `${commitsTotal} ${t('个提交', 'commits')}`,
    crossCount ? `${crossCount} ${t('条跨仓库对应', 'cross links')}` : '',
    root.truncated === true ? t('已截断', 'truncated') : '',
  ]
    .filter(Boolean)
    .join(' · ');
  return { summary, sections };
}

function commitDetailsEvidence(
  root: JsonRecord,
  language: string,
  t: (zh: string, en: string) => string,
): HistoryEvidence {
  const repository = asRecord(root.repository) ?? {};
  const commit = asRecord(root.commit) ?? {};
  const sections: EvidenceSection[] = [];

  const repoFields: EvidenceField[] = [];
  const repositoryLabel =
    asText(repository.key) ||
    [asText(repository.owner), asText(repository.repo)].filter(Boolean).join('/');
  if (repositoryLabel) repoFields.push(field(t('仓库', 'repository'), repositoryLabel));
  const repoPath = asText(repository.path) || asText(repository.source_path);
  if (repoPath) repoFields.push(field(t('本地路径', 'local path'), repoPath, { path: repoPath }));
  const identity = asText(asRecord(repository.identity)?.state);
  if (identity) repoFields.push(field(t('身份', 'identity'), identity));
  if (asText(repository.kind))
    repoFields.push(field(t('来源类型', 'source kind'), asText(repository.kind)));
  if (asText(repository.history_path))
    repoFields.push(
      field(t('历史对象库', 'history object store'), asText(repository.history_path), {
        path: asText(repository.history_path),
      }),
    );
  sections.push({ title: t('仓库', 'repository'), fields: repoFields });

  const sha = asText(commit.sha);
  const commitFields: EvidenceField[] = [];
  if (sha) commitFields.push(field(t('提交', 'commit'), shortSha(sha), { mono: true }));
  const parents = asArray(commit.parents)
    .map((parent) => shortSha(asText(parent)))
    .filter(Boolean);
  if (parents.length)
    commitFields.push(field(t('父提交', 'parents'), parents.join(', '), { mono: true }));
  if (asText(commit.author)) commitFields.push(field(t('作者', 'author'), asText(commit.author)));
  const authorTime = formatUnixSeconds(commit.author_time_unix);
  if (authorTime)
    commitFields.push(field(t('作者时间', 'author time'), authorTime, { mono: true }));
  if (asText(commit.committer))
    commitFields.push(field(t('提交者', 'committer'), asText(commit.committer)));
  const committerTime = formatUnixSeconds(commit.committer_time_unix);
  if (committerTime)
    commitFields.push(field(t('提交时间', 'committer time'), committerTime, { mono: true }));
  if (asText(commit.compare_parent))
    commitFields.push(
      field(t('对比父提交', 'compared parent'), shortSha(asText(commit.compare_parent)), {
        mono: true,
      }),
    );
  if (commit.truncated === true)
    commitFields.push(field(t('diff 截断', 'diff truncated'), yesNo(true, language)));
  const message = asText(commit.message);
  sections.push({
    title: t('提交', 'commit'),
    fields: commitFields,
    items: message
      ? [
          {
            title: t('提交说明', 'commit message'),
            fields: [],
            detail: clip(message, 4000),
          },
        ]
      : undefined,
    note: message ? undefined : t('提交说明未提供', 'commit message not provided'),
  });

  const reason = asRecord(root.reason);
  if (reason) {
    sections.push({
      title: t('修改理由（原文候选，不是动机结论）', 'reason (raw candidates, not a verdict)'),
      fields: commitReasonFields(reason, language, t),
      items: commitReasonItems(reason, language, t),
    });
  }

  const files = asArray(commit.files);
  if (files.length) {
    const items: EvidenceItem[] = [];
    for (const value of files) {
      const file = asRecord(value);
      if (!file) continue;
      const path = asText(file.path);
      const detail = asArray(file.hunks)
        .map((hunk) => hunkDiffText(hunk))
        .filter(Boolean)
        .join('\n\n');
      items.push({
        title: path,
        fields: [field(t('改动块', 'hunks'), String(asArray(file.hunks).length))],
        detail: detail ? clip(detail, 4000) : undefined,
        detailLabel: t('diff（已限制行数）', 'diff (line-limited)'),
        path: historyFileOpenPath(repoPath, path) ?? undefined,
      });
    }
    sections.push({
      title: t('改动文件', 'changed files'),
      fields: [field(t('文件数', 'files'), String(files.length))],
      items,
    });
  }

  const notes = asArray(root.notes).map(asText).filter(Boolean);
  if (notes.length)
    sections.push({ title: t('说明', 'notes'), fields: [], note: notes.join('；') });

  const summary =
    `${shortSha(sha)} ${asText(commit.summary)}`.trim() || t('提交详情', 'commit details');
  return { summary, sections };
}

function pullRequestsEvidence(
  root: JsonRecord,
  language: string,
  t: (zh: string, en: string) => string,
): HistoryEvidence {
  const repository = asRecord(root.repository) ?? {};
  const sections: EvidenceSection[] = [];
  const repoUrl = asText(repository.url);
  const repoLabel =
    [asText(repository.owner), asText(repository.repo)].filter(Boolean).join('/') ||
    t('未提供', 'not provided');
  const overview: EvidenceField[] = [
    field(t('仓库', 'repository'), repoLabel, { url: isHttpUrl(repoUrl) ? repoUrl : undefined }),
  ];
  if (asText(root.commit))
    overview.push(field(t('提交', 'commit'), shortSha(asText(root.commit)), { mono: true }));
  const fetchedAt = asNumber(root.fetched_at_unix_ms);
  if (fetchedAt !== null)
    overview.push(field(t('获取时间', 'fetched at'), formatHistoryTime(fetchedAt), { mono: true }));
  if (typeof root.offline === 'boolean')
    overview.push(field(t('离线', 'offline'), yesNo(root.offline, language)));
  if (typeof root.from_cache === 'boolean')
    overview.push(field(t('来自缓存', 'from cache'), yesNo(root.from_cache, language)));
  if (typeof root.complete === 'boolean')
    overview.push(
      field(t('关联列表完整', 'association list complete'), yesNo(root.complete, language)),
    );
  const page = asRecord(root.page);
  if (page) {
    const pageFields: EvidenceField[] = [];
    if (page.total !== undefined)
      pageFields.push(field(t('关联总数', 'total PRs'), String(asNumber(page.total) ?? 0)));
    if (page.returned !== undefined)
      pageFields.push(field(t('本页', 'this page'), String(asNumber(page.returned) ?? 0)));
    if (page.has_more === true)
      pageFields.push(field(t('还有更多', 'more available'), yesNo(true, language)));
    if (asText(page.next_cursor))
      pageFields.push(
        field(t('下一页游标', 'next cursor'), asText(page.next_cursor), { mono: true }),
      );
    overview.push(...pageFields);
  }
  sections.push({ title: t('查询', 'query'), fields: overview });

  const pulls = asArray(root.pulls);
  const pullItems: EvidenceItem[] = [];
  const continuation: string[] = [];
  for (const value of pulls) {
    const pull = asRecord(value);
    if (!pull) continue;
    const number = asNumber(pull.number);
    const fields: EvidenceField[] = [];
    if (asText(pull.state)) fields.push(field(t('状态', 'state'), asText(pull.state)));
    const mergedAt = asNumber(pull.merged_at_unix_ms);
    if (mergedAt !== null)
      fields.push(field(t('合并时间', 'merged at'), formatHistoryTime(mergedAt), { mono: true }));
    if (asText(pull.author)) fields.push(field(t('作者', 'author'), asText(pull.author)));
    if (asText(pull.head_sha))
      fields.push(field(t('head', 'head'), shortSha(asText(pull.head_sha)), { mono: true }));
    if (asText(pull.base_ref)) fields.push(field(t('base', 'base'), asText(pull.base_ref)));
    if (asText(pull.evidence_id))
      fields.push(field(t('证据 ID', 'evidence id'), asText(pull.evidence_id), { mono: true }));
    const reason = asRecord(pull.reason);
    if (reason) {
      fields.push(
        field(
          t('理由状态（原文候选，不是动机结论）', 'reason status (raw candidate, not a verdict)'),
          asText(reason.status) || t('未提供', 'not provided'),
        ),
      );
      if (asText(reason.excerpt))
        fields.push(field(t('理由摘录', 'reason excerpt'), asText(reason.excerpt)));
    }
    const comments = asRecord(pull.comments);
    if (comments) {
      const completeValue = comments.complete;
      fields.push(
        field(
          t('讨论是否读完', 'discussion completeness'),
          typeof completeValue === 'boolean'
            ? completeValue
              ? t('完整', 'complete')
              : t('不完整（未核验）', 'incomplete (unverified)')
            : t('未提供', 'not provided'),
        ),
      );
    }
    if (asNumber(pull.next_body_offset) !== null)
      continuation.push(
        t(
          `PR #${number} 正文未读完：用 pr_number=${number} + body_offset=${asNumber(pull.next_body_offset)} 继续`,
          `PR #${number} body continues: call pr_number=${number} with body_offset=${asNumber(pull.next_body_offset)}`,
        ),
      );
    const commentsCursor = asRecord(pull.comments)?.next_cursor;
    if (asText(commentsCursor))
      continuation.push(
        t(
          `PR #${number} 讨论未读完：用 pr_number=${number} + comments_cursor=${asText(commentsCursor)} 继续`,
          `PR #${number} discussion continues: call pr_number=${number} with comments_cursor=${asText(commentsCursor)}`,
        ),
      );
    const detailParts: string[] = [];
    if (asText(pull.body)) detailParts.push(`${t('正文', 'body')}:\n${asText(pull.body)}`);
    for (const excerpt of asArray(comments?.excerpts)) {
      const record = asRecord(excerpt);
      if (!record) continue;
      detailParts.push(
        `— ${asText(record.author) || t('未知作者', 'unknown author')}:\n${asText(record.body)}`,
      );
    }
    const url = asText(pull.url);
    pullItems.push({
      title: `#${number ?? '?'} ${asText(pull.title)}`.trim(),
      fields,
      detail: detailParts.length ? clip(detailParts.join('\n\n'), 4000) : undefined,
      detailLabel: t('正文与讨论摘录', 'body and discussion excerpts'),
      url: isHttpUrl(url) ? url : undefined,
    });
    for (const candidate of commitReasonItems(reason, language, t))
      pullItems.push({ ...candidate, title: `#${number ?? '?'} · ${candidate.title}` });
  }
  sections.push({
    title: t('关联 PR', 'associated pull requests'),
    fields: [field(t('本页 PR 数', 'PRs in this page'), String(pulls.length))],
    items: pullItems,
    note: pulls.length
      ? undefined
      : t(
          '本页没有 PR 记录——只有完整读过的关联列表才能说“没有关联”',
          'No PR in this page; only a complete association list can support "not associated"',
        ),
  });

  const unverified = asRecord(root.detail_unverified);
  if (unverified) {
    const fields: EvidenceField[] = [];
    if (asNumber(unverified.pr_number) !== null)
      fields.push(field(t('PR 编号', 'PR number'), String(asNumber(unverified.pr_number))));
    if (typeof unverified.list_complete === 'boolean')
      fields.push(
        field(
          t('关联列表完整', 'association list complete'),
          yesNo(unverified.list_complete, language),
        ),
      );
    if (typeof unverified.offline === 'boolean')
      fields.push(field(t('离线', 'offline'), yesNo(unverified.offline, language)));
    sections.push({
      title: t('未核验的 PR 详情', 'unverified PR detail'),
      fields,
      note: `${t('未核验 ≠ 未关联', 'unverified is not the same as unassociated')}；${asText(
        unverified.reason,
      )}`,
    });
  }

  const notices = asArray(root.notices).map(asText).filter(Boolean);
  if (notices.length)
    sections.push({ title: t('提示', 'notices'), fields: [], note: notices.join('；') });

  const errors = asArray(root.errors)
    .map((value) => {
      const error = asRecord(value);
      if (!error) return null;
      const fields: EvidenceField[] = [];
      if (asText(error.kind)) fields.push(field(t('错误类型', 'kind'), asText(error.kind)));
      if (asNumber(error.status) !== null)
        fields.push(field(t('HTTP 状态', 'status'), String(asNumber(error.status))));
      return {
        title: asText(error.message) || asText(error.kind) || t('错误', 'error'),
        fields,
      };
    })
    .filter((item): item is EvidenceItem => item !== null);
  if (errors.length) sections.push({ title: t('错误', 'errors'), fields: [], items: errors });

  if (page?.has_more === true || continuation.length) {
    const hints: string[] = [];
    if (page?.has_more === true && asText(page.next_cursor))
      hints.push(
        t(
          `还有更多关联 PR：用 cursor=${asText(page.next_cursor)} 继续分页`,
          `More associated PRs: continue with cursor=${asText(page.next_cursor)}`,
        ),
      );
    hints.push(...continuation);
    hints.push(
      t(
        '一次调用只返回一页；不要声称读完了全部 PR 或讨论',
        'One call returns one page only; do not claim the whole PR list or discussion was read',
      ),
    );
    sections.push({
      title: t('如何继续读取', 'how to continue'),
      fields: [],
      note: hints.join('；'),
    });
  }

  const totalPages =
    page && asNumber(page.total) !== null
      ? `${t('关联', 'associated')} ${asNumber(page.total)} · ${t('本页', 'page')} ${
          asNumber(page.returned) ?? 0
        }`
      : `${pulls.length} ${t('个 PR', 'PRs')}`;
  const summary = [
    shortSha(asText(root.commit)) || t('未提供提交', 'no commit'),
    totalPages,
    root.complete === false ? t('列表不完整', 'incomplete list') : '',
    root.offline === true ? t('离线', 'offline') : '',
  ]
    .filter(Boolean)
    .join(' · ');
  return { summary, sections };
}

// ---------------------------------------------------------------------------------------------
// U04 未知副作用核对：任务快照（task_state.event.task）只信结构字段；新回答/新事件不能清除。
// ---------------------------------------------------------------------------------------------

export interface TaskEffectAction {
  id?: string;
  name?: string;
  arguments?: string;
  mutating?: boolean;
  effect?: string;
  state?: string;
  result?: string | null;
  summary?: string | null;
  /** 用户已明确核对过（后端完整 actions 里携带）；已核对的旧动作不再要求核对。 */
  reconciled?: boolean;
}

export interface AgentTaskSnapshot {
  goal?: string;
  state?: string;
  phase?: string;
  requires_reconciliation?: boolean;
  effect_unknown_actions?: TaskEffectAction[];
  recent_actions?: TaskEffectAction[];
  actions?: TaskEffectAction[];
  next_step?: string;
}

function taskAction(value: unknown): TaskEffectAction | null {
  const record = asRecord(value);
  if (!record) return null;
  return {
    id: asText(record.id) || undefined,
    name: asText(record.name) || undefined,
    arguments: asText(record.arguments) || undefined,
    mutating: typeof record.mutating === 'boolean' ? record.mutating : undefined,
    effect: asText(record.effect) || undefined,
    state: asText(record.state) || undefined,
    result: typeof record.result === 'string' ? record.result : undefined,
    summary: typeof record.summary === 'string' ? record.summary : undefined,
    reconciled: record.reconciled === true,
  };
}

/** task_state 的 task 对象 → 快照；缺字段/类型不对时保留缺省，不补造。 */
export function taskSnapshotOf(value: unknown): AgentTaskSnapshot | null {
  const record = asRecord(value);
  if (!record) return null;
  const list = (key: string) =>
    asArray(record[key])
      .map(taskAction)
      .filter((action): action is TaskEffectAction => action !== null);
  return {
    goal: asText(record.goal) || undefined,
    state: asText(record.state) || undefined,
    phase: asText(record.phase) || undefined,
    requires_reconciliation:
      typeof record.requires_reconciliation === 'boolean'
        ? record.requires_reconciliation
        : undefined,
    effect_unknown_actions: Array.isArray(record.effect_unknown_actions)
      ? list('effect_unknown_actions')
      : undefined,
    recent_actions: Array.isArray(record.recent_actions) ? list('recent_actions') : undefined,
    actions: Array.isArray(record.actions) ? list('actions') : undefined,
    next_step: asText(record.next_step) || undefined,
  };
}

function actionEffectIsUnknown(effect: string | undefined) {
  const value = (effect ?? '').trim().toLowerCase();
  return value === 'effect_unknown' || value === 'effectunknown';
}

export interface PendingReconciliation {
  /** 任务快照明确要求核对（只有新的 task_state 能把它变回 false）。 */
  required: boolean;
  /** 待核对的旧动作；后端给出 effect_unknown_actions 时以它为准。 */
  actions: TaskEffectAction[];
  /** 传给 assistant_reconcile_effects 的旧动作 id（只核对当前横幅里的这些 id）。 */
  ids: string[];
}

/** 从任务快照取出“执行结果待核对”的横幅内容；缺字段时保守地退回 actions 里的 effect_unknown。 */
export function pendingReconciliation(
  task: AgentTaskSnapshot | null | undefined,
): PendingReconciliation {
  const required = task?.requires_reconciliation === true;
  const explicit = task?.effect_unknown_actions;
  const actions =
    explicit ??
    (task?.actions ?? []).filter(
      (action) =>
        action.mutating !== false &&
        action.reconciled !== true &&
        actionEffectIsUnknown(action.effect),
    );
  const ids = actions
    .map((action) => (action.id ?? '').trim())
    .filter((id): id is string => id !== '');
  return { required, actions, ids };
}

/** 待核对动作的标题：名字 + 身份 + 结构化结局，原样显示未知状态。 */
export function taskActionTitle(action: TaskEffectAction, language: string): string {
  const t = (zh: string, en: string) => (language === 'en' ? en : zh);
  const parts = [
    action.name || t('未知操作', 'unknown action'),
    action.id ? `#${action.id}` : '',
    action.state ? `${t('状态', 'state')} ${action.state}` : '',
    action.effect ? `${t('结局', 'effect')} ${action.effect}` : '',
  ].filter(Boolean);
  return parts.join(' · ');
}

/** 待核对动作可展开的原文（参数 + 结果）。 */
export function taskActionOutput(action: TaskEffectAction): string {
  return [action.arguments ?? '', action.result ?? ''].filter(Boolean).join('\n\n');
}

/** 动作参数/结果里可打开的本地绝对路径（U04“查看运行/输出”）。 */
export function localPathsInAction(action: TaskEffectAction, limit = 5): string[] {
  const paths = localPathsInJsonText(action.arguments, limit);
  for (const path of localPathsInJsonText(action.result ?? '', limit)) {
    if (paths.length >= limit) break;
    if (!paths.includes(path)) paths.push(path);
  }
  return paths;
}
