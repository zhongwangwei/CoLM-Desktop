import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
const source = await readFile(new URL('../dist/app/assistant-providers.js', import.meta.url), 'utf8');
const { API_PROVIDERS, providerId, rememberProfile, selectProvider, apiEfforts, parseApiOptions, completeProfiles, settingsForSave } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
assert.equal(API_PROVIDERS.length, 9);
assert.equal(providerId({ base_url: 'https://api.deepseek.com/v1/' }), 'deepseek');
assert.equal(providerId({ base_url: 'https://api.deepseek.com.evil.test' }), 'custom');
assert.equal(providerId({ base_url: 'http://localhost:8080', provider_id: 'openai' }), 'openai');
const original = { base_url: 'https://api.deepseek.com', model: 'my-edited-flash', thinking: false,
  timeout_seconds: 55, max_output_tokens: 222, strict: true, api_options: { temperature: 0.4 },
  api_key: 'never-in-profile', external: { codex: { model: 'local-choice' } } };
const openai = selectProvider(original, 'openai');
assert.equal(openai.api_format, 'responses');
assert.equal(openai.thinking, null);
assert.equal(openai.timeout_seconds, 600);
assert.equal(openai.api_profiles.deepseek.api_key, undefined);
const edited = { ...openai, base_url: 'https://proxy.example/v1', model: 'future-model', reasoning_effort: 'xhigh', api_options: { temperature: 0.2 } };
const restored = selectProvider(edited, 'deepseek');
assert.equal(restored.model, original.model);
assert.equal(restored.timeout_seconds, 55);
assert.equal(restored.strict, true);
assert.deepEqual(restored.api_options, original.api_options);
assert.equal(restored.thinking, false);
assert.deepEqual(restored.external, original.external);
const again = selectProvider(restored, 'openai');
assert.equal(again.model, 'future-model');
assert.equal(again.base_url, 'https://proxy.example/v1');
assert.equal(again.reasoning_effort, 'xhigh');
assert.equal(rememberProfile({ ...again, reasoning_effort: 'low' }).api_profiles.openai.reasoning_effort, 'low');
assert.equal(apiEfforts('openai').includes('off'), false);
assert.deepEqual(apiEfforts('kimi', 'kimi-k3'), ['', 'low', 'high', 'max']);
assert.deepEqual(apiEfforts('kimi', 'kimi-k2.7-code'), ['']);
assert.deepEqual(apiEfforts('kimi', 'kimi-k2.6'), ['', 'on', 'off']);
assert.deepEqual(parseApiOptions(''), {});
assert.deepEqual(parseApiOptions('{"temperature":0.2}'), { temperature: 0.2 });
for (const value of ['null', '[]', '3', '{bad']) assert.throws(() => parseApiOptions(value));
console.log('assistant providers: legacy inference, independent drafts, option validation, reasoning capabilities passed');

assert.equal(API_PROVIDERS.find(p => p.id === 'glm').model, 'glm-5.3');
assert.equal(API_PROVIDERS.find(p => p.id === 'qwen').model, 'qwen3.8-flash');
assert.deepEqual(apiEfforts('glm', 'glm-5.3'), ['', 'low', 'high', 'max']);
assert.deepEqual(apiEfforts('glm', 'glm-4.7'), ['', 'on', 'off']);
const incomplete = selectProvider(original, 'custom');
const leaveIncomplete = selectProvider(incomplete, 'deepseek');
assert.ok(leaveIncomplete.api_profiles.custom);
assert.equal(completeProfiles(leaveIncomplete.api_profiles).custom, undefined);
assert.equal(selectProvider(leaveIncomplete, 'custom').base_url, '');
assert.ok(completeProfiles(leaveIncomplete.api_profiles).deepseek);

// An unfinished custom API must not prevent saving a subscription backend.
for (const backend of ['codex', 'claude_code']) {
  const draft = { ...rememberProfile(incomplete), backend,
    external: { [backend]: { model: 'subscription-model', effort: 'high' } } };
  const saved = settingsForSave(draft, original);
  assert.equal(saved.backend, backend);
  assert.deepEqual(saved.external, draft.external);
  assert.equal(saved.base_url, original.base_url);
  assert.equal(saved.model, original.model);
  assert.equal(saved.api_profiles.custom, undefined);
  assert.equal(draft.base_url, ''); // The editable draft was not overwritten.
  assert.ok(draft.api_profiles.custom);
  const defaults = settingsForSave(draft, {});
  assert.equal(defaults.base_url, 'https://api.deepseek.com');
  assert.equal(defaults.model, 'deepseek-flash');
}
assert.equal(settingsForSave({ ...incomplete, backend: 'builtin' }, original).base_url, '');
