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

test('writing files registers commit actions while deletion and discarding stay manual', () => {
  const html = render('new soil scheme');
  // 新建文件、从本机加入文件都是写工作区的 C 级动作，按 commit 登记。
  assert.ok(html.includes('data-action="code-file-create"'));
  assert.ok(html.includes('data-action="code-file-add"'));
  // 删除、替换整个目录、放弃修改属于 D 级/撤回，界面上有按钮但不登记。
  assert.doesNotMatch(html, /data-action="[^"]*(delete|remove|discard|revert)/i);
});
