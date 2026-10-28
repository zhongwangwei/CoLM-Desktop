import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { defaultSettings, type Settings } from './model';
import {
  draftApiOptions,
  draftSaveState,
  equalJson,
  sendingSummary,
  settingsDraftDirty,
} from './settings-draft';

const saved = defaultSettings;
const optionsText = JSON.stringify(saved.api_options, null, 2);

test('a fresh settings form is saved; changing any saved field makes it dirty', () => {
  assert.equal(settingsDraftDirty(saved, { ...saved }, optionsText), false);
  for (const draft of [
    { ...saved, model: 'other-model' },
    { ...saved, base_url: 'https://example.com/v1' },
    { ...saved, context_window: 128_000 },
    { ...saved, session_turn_limit: 7 },
    { ...saved, web_search: !saved.web_search },
    { ...saved, external: { opencode: { model: 'x', effort: null } } },
  ])
    assert.equal(settingsDraftDirty(saved, draft, optionsText), true);
  // 改回已保存值就又回到"已保存"。
  assert.equal(settingsDraftDirty(saved, { ...saved, model: saved.model }, optionsText), false);
});

test('the API options textarea participates in the dirty state', () => {
  assert.equal(settingsDraftDirty(saved, saved, JSON.stringify(saved.api_options, null, 2)), false);
  // 同样的内容不同排版不算修改。
  assert.equal(settingsDraftDirty(saved, saved, JSON.stringify(saved.api_options)), false);
  assert.equal(settingsDraftDirty(saved, saved, '{"temperature": 0.2}'), true);
  assert.equal(settingsDraftDirty(saved, saved, '{not json'), true);
  assert.equal(settingsDraftDirty(saved, saved, '[]'), true, '数组不是合法的 API 选项对象');
  assert.equal(draftApiOptions('{"temperature": 0.2}')?.temperature, 0.2);
});

test('save badge prefers "saving" over dirty and "saved" is the clean state', () => {
  assert.equal(draftSaveState(false, false), 'saved');
  assert.equal(draftSaveState(true, false), 'dirty');
  assert.equal(draftSaveState(false, true), 'saving');
  assert.equal(draftSaveState(true, true), 'saving');
});

test('the actual sending summary always comes from saved settings, not the draft', () => {
  assert.deepEqual(sendingSummary(saved, 'en'), {
    service: 'DeepSeek (Pro / Flash)',
    model: 'deepseek-flash',
  });
  const draft = { ...saved, model: 'unsaved-model' };
  assert.equal(sendingSummary(draft, 'en').model, 'unsaved-model', 'summary of the draft shows it');
  assert.equal(sendingSummary(saved, 'en').model, 'deepseek-flash', 'saved summary unchanged');
  const codex = {
    ...saved,
    backend: 'codex',
    external: { codex: { model: 'gpt-codex', effort: null } },
  };
  assert.deepEqual(sendingSummary(codex, 'en'), { service: 'Codex', model: 'gpt-codex' });
  const opencode = {
    ...saved,
    backend: 'opencode',
    external: { opencode: { model: 'provider/model', effort: 'high' } },
  };
  assert.deepEqual(sendingSummary(opencode, 'en'), {
    service: 'OpenCode',
    model: 'provider/model',
  });
});

test('key order does not make equal settings look dirty', () => {
  assert.equal(equalJson({ a: 1, b: [2, { c: 3 }] }, { b: [2, { c: 3 }], a: 1 }), true);
  assert.equal(equalJson({ a: 1 }, { a: 2 }), false);
  assert.equal(equalJson([1, 2], [1, 2, 3]), false);
  assert.equal(equalJson(null, undefined), false);
});

// ---------------------------------------------------------------------------
// 从真实源码里抽出放弃草稿与关闭确认逻辑，验证设置草稿状态机
// ---------------------------------------------------------------------------

function sourceOf(file: string) {
  const path = new URL(file, import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const functions = new Map<string, string>();
  function visit(node: ts.Node) {
    if (ts.isFunctionDeclaration(node) && node.name)
      functions.set(node.name.text, node.getText(source));
    ts.forEachChild(node, visit);
  }
  visit(source);
  return { source, functions };
}
const settingsSource = sourceOf('./Settings.tsx');
const panelSource = sourceOf('./AssistantPanel.tsx');

function evaluate<T>(code: string, name: string, context: Record<string, unknown>): T {
  const js = ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return ${name};`)(
    ...Object.values(context),
  ) as T;
}

test('discarding the settings draft restores every field and clears the textarea buffer', () => {
  const body = settingsSource.functions.get('discardDraft');
  assert.ok(body, 'Settings.tsx defines discardDraft');
  const restored: Settings[] = [];
  const options: string[] = [];
  const status: string[] = [];
  const discard = evaluate<() => void>(body, 'discardDraft', {
    settings: saved,
    setDraft: (value: Settings) => restored.push(value),
    setOptions: (value: string) => options.push(value),
    setStatus: (value: string) => status.push(value),
    t: (_zh: string, en: string) => en,
  });
  discard();
  assert.deepEqual(restored, [saved]);
  assert.deepEqual(options, [JSON.stringify(saved.api_options, null, 2)]);
  assert.equal(status.length, 1);
});

test('closing dirty settings asks first and keeps the panel open when the user cancels', async () => {
  const body = panelSource.functions.get('closeSettings');
  assert.ok(body, 'AssistantPanel.tsx defines closeSettings');
  let answer = false;
  const updates: unknown[] = [];
  const dirtyWrites: boolean[] = [];
  const confirmations: string[] = [];
  const close = evaluate<() => Promise<boolean>>(body, 'closeSettings', {
    state: { assistantSettingsOpen: true },
    settingsDirty: true,
    askConfirm: async (message: string) => {
      confirmations.push(message);
      return answer;
    },
    setSettingsDirty: (value: boolean) => dirtyWrites.push(value),
    update: (value: unknown) => updates.push(value),
    t: (_zh: string, en: string) => en,
  });
  assert.equal(await close(), false);
  assert.deepEqual(updates, [], '取消时不能把设置面板关掉');
  assert.deepEqual(dirtyWrites, []);
  assert.equal(confirmations.length, 1, '未保存时必须先确认');

  answer = true;
  assert.equal(await close(), true);
  assert.deepEqual(updates, [{ assistantSettingsOpen: false }]);
  assert.deepEqual(dirtyWrites, [false]);
});

test('closing clean settings needs no confirmation; a closed panel is a no-op', async () => {
  const body = panelSource.functions.get('closeSettings');
  assert.ok(body);
  const calls: string[] = [];
  const closeClean = evaluate<() => Promise<boolean>>(body, 'closeSettings', {
    state: { assistantSettingsOpen: true },
    settingsDirty: false,
    askConfirm: async () => {
      calls.push('confirm');
      return true;
    },
    setSettingsDirty: () => {},
    update: (value: unknown) => calls.push(JSON.stringify(value)),
    t: (_zh: string, en: string) => en,
  });
  assert.equal(await closeClean(), true);
  assert.deepEqual(calls, ['{"assistantSettingsOpen":false}']);

  const closeClosed = evaluate<() => Promise<boolean>>(body, 'closeSettings', {
    state: { assistantSettingsOpen: false },
    settingsDirty: true,
    askConfirm: async () => {
      calls.push('confirm');
      return true;
    },
    setSettingsDirty: () => {},
    update: () => calls.push('update'),
    t: (_zh: string, en: string) => en,
  });
  assert.equal(await closeClosed(), true);
  assert.deepEqual(calls, ['{"assistantSettingsOpen":false}']);
});

test('editing the settings dialog only changes a local draft, never the saved settings', () => {
  const body = settingsSource.functions.get('patch');
  assert.ok(body, 'Settings.tsx defines patch');
  const drafts: Partial<Settings>[] = [];
  const saves: unknown[] = [];
  const statuses: string[] = [];
  const patch = evaluate<(value: Partial<Settings>) => void>(body, 'patch', {
    setDraft: (updater: (old: Settings) => Settings) => drafts.push(updater(saved)),
    setStatus: (value: string) => statuses.push(value),
    onSave: (value: unknown) => saves.push(value),
  });
  patch({ model: 'typed-but-unsaved' });
  assert.equal(drafts.length, 1);
  assert.equal(drafts[0].model, 'typed-but-unsaved');
  assert.equal(drafts[0].base_url, saved.base_url, '只合并改动字段，其余仍取已保存值');
  assert.deepEqual(saves, [], '改草稿不能直接触发保存');
  assert.deepEqual(statuses, [''], '字段一改就清掉过时的"已保存"提示');
});

test('sending reads the saved settings ref, never the settings draft', () => {
  const send = panelSource.functions.get('send');
  assert.ok(send, 'AssistantPanel.tsx defines send');
  assert.match(send, /settingsRef\.current/);
  assert.doesNotMatch(send, /settingsDraft|draft\./);
});

test('the settings dialog renders the shared save badge and the saved sending target', () => {
  const patch = settingsSource.functions.get('discardDraft');
  assert.ok(patch);
  assert.match(settingsSource.source.text, /<SaveBadge/);
  assert.match(settingsSource.source.text, /sendingSummary\(settings, state\.language\)/);
  assert.match(settingsSource.source.text, /实际发送/);
  assert.match(settingsSource.source.text, /放弃修改/);
});
