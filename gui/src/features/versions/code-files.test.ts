import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { default: CodeFiles } = await import('./CodeFiles');

const render = (message: string) =>
  renderToStaticMarkup(
    createElement(CodeFiles, {
      name: 'ver-1',
      message,
      en: true,
      disabled: false,
      onChanged: async () => {},
    }),
  );

test('code files say where each engine lives and ask for a change message before changing', () => {
  const html = render('');
  assert.ok(html.includes('The Rust engine is under crates/'));
  assert.ok(html.includes('Fill in the change message above first.'));
  assert.match(html, /<button[^>]*disabled[^>]*>Add a file from this computer/);
  const ready = render('new soil scheme');
  assert.ok(!ready.includes('Fill in the change message above first.'));
  assert.doesNotMatch(ready, /<button[^>]*disabled[^>]*>Add a file from this computer/);
});
