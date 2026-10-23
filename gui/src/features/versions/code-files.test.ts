import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
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

test('a slow file read cannot replace the file the user clicked last', async () => {
  const path = new URL('./CodeFiles.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'openFile')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, 'exercise the actual openFile');
  const pending: Record<string, (file: unknown) => void> = {};
  const shown: string[] = [];
  const context = {
    edited: false,
    askConfirm: async () => true,
    t: (_zh: string, en: string) => en,
    setError: () => {},
    opening: { current: 0 },
    source: undefined,
    name: 'ver-1',
    invokeJson: (_command: string, args: { path: string }) =>
      new Promise((resolve) => (pending[args.path] = resolve)),
    setOpen: (file: { path: string }) => shown.push(file.path),
    setDraft: () => {},
  };
  const code = ts.transpileModule(`${body}; return openFile;`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const openFile = new Function(...Object.keys(context), code)(...Object.values(context)) as (
    path: string,
  ) => Promise<void>;
  const first = openFile('a.F90');
  const second = openFile('b.F90');
  await new Promise((resolve) => setTimeout(resolve, 0));
  pending['b.F90']({ path: 'b.F90', text: 'b' });
  await second;
  pending['a.F90']({ path: 'a.F90', text: 'a' });
  await first;
  assert.deepEqual(shown, ['b.F90']);
});
