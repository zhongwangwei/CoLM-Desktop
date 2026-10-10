// Presets are editable starting points; live model discovery never replaces a typed model.
export const API_PROVIDERS = [
  { id: 'deepseek', name: 'DeepSeek（Pro / Flash）', base_url: 'https://api.deepseek.com', model: 'deepseek-flash', api_format: 'chat_completions' },
  { id: 'openai', name: 'OpenAI / ChatGPT API', base_url: 'https://api.openai.com/v1', model: 'gpt-6.1-sol', api_format: 'responses' },
  { id: 'anthropic', name: 'Anthropic / Claude API', base_url: 'https://api.anthropic.com/v1', model: 'claude-sonnet-5-5', api_format: 'anthropic' },
  { id: 'grok', name: 'xAI / Grok', base_url: 'https://api.x.ai/v1', model: 'grok-4.7', api_format: 'chat_completions' },
  { id: 'glm', name: '智谱 / GLM', base_url: 'https://open.bigmodel.cn/api/paas/v4', model: 'glm-5.3', models: ['glm-5.3', 'glm-5.3-flash', 'glm-5.3-flashx'], api_format: 'chat_completions' },
  { id: 'gemini', name: 'Google / Gemini', base_url: 'https://generativelanguage.googleapis.com/v1beta/openai', model: 'gemini-3.8-flash', api_format: 'chat_completions' },
  { id: 'kimi', name: 'Moonshot / Kimi', base_url: 'https://api.moonshot.cn/v1', model: 'kimi-k3', api_format: 'chat_completions' },
  { id: 'qwen', name: '阿里云 / Qwen', base_url: 'https://dashscope.aliyuncs.com/compatible-mode/v1', model: 'qwen3.8-flash', models: ['qwen3.8-max', 'qwen3.8-flash'], api_format: 'chat_completions' },
  { id: 'custom', name: '自定义服务', base_url: '', model: '', api_format: 'chat_completions' },
];
export function providerId(settings) {
  if (settings.provider_id && API_PROVIDERS.some(p => p.id === settings.provider_id)) return settings.provider_id;
  try {
    const host = new URL(settings.base_url).hostname;
    return API_PROVIDERS.find(p => p.base_url && new URL(p.base_url).hostname === host)?.id || 'custom';
  } catch { return 'custom'; }
}
const PROFILE_FIELDS = ['base_url', 'model', 'api_format', 'api_options', 'max_output_tokens', 'timeout_seconds', 'thinking', 'reasoning_effort', 'strict'];
export function rememberProfile(settings) {
  const id = providerId(settings);
  const profile = Object.fromEntries(PROFILE_FIELDS.filter(k => settings[k] !== undefined).map(k => [k, settings[k]]));
  return { ...settings, provider_id: id, api_profiles: { ...settings.api_profiles, [id]: profile } };
}
export function selectProvider(settings, id) {
  const saved = rememberProfile(settings);
  const preset = API_PROVIDERS.find(p => p.id === id);
  if (!preset) throw new Error('Unknown provider');
  return { ...saved, base_url: preset.base_url, model: preset.model, api_format: preset.api_format,
    api_options: {}, max_output_tokens: 16384, timeout_seconds: 600, strict: false, thinking: null, reasoning_effort: null,
    ...saved.api_profiles[id], provider_id: id };
}
export function apiEfforts(id, model = '') {
  if (id === 'deepseek') return ['', 'low', 'high', 'max', 'off'];
  if (id === 'openai') return ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  if (id === 'gemini') return ['', 'minimal', 'low', 'medium', 'high'];
  if (id === 'anthropic') return ['', 'low', 'medium', 'high', 'xhigh', 'max'];
  if (id === 'kimi') {
    if (model.startsWith('kimi-k3')) return ['', 'low', 'high', 'max'];
    if (model.startsWith('kimi-k2.7')) return [''];
    return ['', 'on', 'off'];
  }
  if (id === 'glm' && model.startsWith('glm-5.3')) return ['', 'low', 'high', 'max'];
  if (['glm', 'qwen'].includes(id)) return ['', 'on', 'off'];
  if (id === 'grok') return model.startsWith('grok-4.5') ? ['', 'low', 'medium', 'high'] : ['', 'low', 'medium', 'high', 'xhigh'];
  return ['', 'low', 'medium', 'high', 'xhigh', 'max', 'off'];
}
export function parseApiOptions(text) {
  const options = JSON.parse(text.trim() || '{}');
  if (!options || typeof options !== 'object' || Array.isArray(options)) throw new Error('API options must be a JSON object');
  return options;
}

// Incomplete inactive profiles remain in the open form; the backend only stores usable profiles.
export function completeProfiles(profiles = {}) {
  return Object.fromEntries(Object.entries(profiles).filter(([, profile]) => {
    if (!profile.model?.trim()) return false;
    try { return ['https:', 'http:'].includes(new URL(profile.base_url).protocol); }
    catch { return false; }
  }));
}

export function settingsForSave(draft, previous) {
  let settings = { ...draft, api_profiles: completeProfiles(draft.api_profiles) };
  if (settings.backend !== 'builtin' && !completeProfiles({ current: draft }).current) {
    const api = completeProfiles({ previous }).previous || selectProvider({}, 'deepseek');
    settings = { ...settings, provider_id: providerId(api) };
    for (const key of PROFILE_FIELDS) {
      if (api[key] === undefined) delete settings[key];
      else settings[key] = api[key];
    }
  }
  return settings;
}
