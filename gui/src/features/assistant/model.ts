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
