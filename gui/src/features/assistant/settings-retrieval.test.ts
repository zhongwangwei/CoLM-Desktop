import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { equalJson } from './settings-draft';
import { defaultSettings, type AgentEvent } from './model';

const source = ts.createSourceFile(
  'Settings.tsx',
  readFileSync(new URL('./Settings.tsx', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
function find(match: (node: ts.Node) => boolean): ts.Node {
  let found: ts.Node | undefined;
  function visit(node: ts.Node) {
    if (match(node)) found = node;
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(found);
  return found;
}
function evaluate<T>(expression: string, context: Record<string, unknown>): T {
  const js = ts.transpileModule(`const result = (${expression});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${js}; return result;`)(...Object.values(context));
}
function hint(enabled: boolean, docsIndex: Partial<AgentEvent> | null, language = 'en') {
  const fn = find((node) => ts.isFunctionDeclaration(node) && node.name?.text === 'docsIndexHint');
  return evaluate<() => string>(fn.getText(source), {
    settings: { retrieval: { enabled } },
    docsIndex,
    t: (zh: string, en: string) => (language === 'en' ? en : zh),
  })();
}

test('saved disabled retrieval cannot be presented as ready by an earlier status event', () => {
  assert.match(hint(false, { state: 'ready', documents: 10, chunks: 30 }), /disabled.*keyword/);
  assert.match(hint(false, null, 'zh'), /未启用.*关键词/);
});

test('retrieval hints distinguish startup, building, readiness, empty scope and fallback', () => {
  assert.match(hint(true, null), /when it starts/);
  for (const state of ['building', 'pending'])
    assert.match(hint(true, { state }), /keep asking.*keyword/);
  assert.match(
    hint(true, { state: 'ready', documents: 4, chunks: 12 }),
    /actual search method and evidence/,
  );
  assert.match(hint(true, { state: 'ready', documents: 0, chunks: 0 }), /no searchable documents/);
  for (const state of ['error', 'not_connected', 'timeout', 'stale', 'future_state'])
    assert.match(hint(true, { state }), /fall back.*keyword/);
});

test('the document retrieval enable switch is visible without expanding advanced settings', () => {
  const checked = find(
    (node) =>
      ts.isJsxAttribute(node) &&
      node.name.getText(source) === 'checked' &&
      node.initializer?.getText(source) === '{draft.retrieval.enabled}',
  );
  for (let parent = checked.parent; parent; parent = parent.parent) {
    if (ts.isJsxElement(parent))
      assert.notEqual(parent.openingElement.tagName.getText(source), 'Advanced');
  }
});

test('index actions wait until enabled retrieval changes are saved', () => {
  const button = find(
    (node) =>
      ts.isJsxElement(node) &&
      node.openingElement.tagName.getText(source) === 'Button' &&
      node.getText(source).includes("invoke('assistant_docs_index', { action: 'refresh' })"),
  ) as ts.JsxElement;
  const disabled = button.openingElement.attributes.properties.find(
    (node) => ts.isJsxAttribute(node) && node.name.getText(source) === 'disabled',
  ) as ts.JsxAttribute;
  const expression = (disabled.initializer as ts.JsxExpression).expression!.getText(source);
  const saved = { ...defaultSettings, retrieval: { ...defaultSettings.retrieval, enabled: true } };
  const isDisabled = (draft: typeof saved, settings = saved) =>
    evaluate<boolean>(expression, { draft, settings, equalJson });
  assert.equal(isDisabled(saved), false);
  assert.equal(
    isDisabled({ ...saved, retrieval: { ...saved.retrieval, index: 'other-docs' } }),
    true,
  );
  const off = { ...saved, retrieval: { ...saved.retrieval, enabled: false } };
  assert.equal(isDisabled(off, off), true);
});
