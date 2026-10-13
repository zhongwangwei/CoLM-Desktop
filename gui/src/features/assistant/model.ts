export interface ApiProfile {
  base_url: string;
  model: string;
  api_format: string;
  api_options: Record<string, unknown>;
  max_output_tokens: number;
  timeout_seconds: number;
  thinking: boolean | null;
  reasoning_effort: string | null;
  strict: boolean;
}
export interface Settings extends ApiProfile {
  provider_id: string;
  api_profiles: Record<string, ApiProfile>;
  approval: string;
  web_search: boolean;
  backend: string;
  egress_acknowledged: string | null;
  external: Record<string, { model: string | null; effort: string | null }>;
}
export const defaultSettings: Settings = {
  base_url: 'https://api.deepseek.com',
  model: 'deepseek-flash',
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
  backend: 'builtin',
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
    name: '智谱 / GLM',
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
    name: '阿里云 / Qwen',
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
  return settings.backend === 'builtin'
    ? settings.base_url
    : settings.backend === 'opencode'
      ? `opencode:${settings.external.opencode?.model || ''}`
      : settings.backend;
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
export function selectAssistantModel(settings: Settings, model: string) {
  if (settings.backend !== 'builtin') return withChoice(settings, { model });
  const effort = thinkValue(settings);
  return {
    ...settings,
    model,
    ...(!apiEfforts(providerId(settings), model).includes(effort)
      ? { thinking: null, reasoning_effort: null }
      : {}),
  };
}
