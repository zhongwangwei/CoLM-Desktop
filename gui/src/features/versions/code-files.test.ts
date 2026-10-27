import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { isContentConflict, workspaceErrorText } from './workspace-errors';
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

function pageSource(): ts.SourceFile {
  const path = new URL('./CodeFiles.tsx', import.meta.url);
  return ts.createSourceFile(
    path.pathname,
    readFileSync(path.pathname, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
}

/** 从页面源码里抽出真实函数体，用 new Function 在受控上下文里执行（仓库惯例）。 */
function extractFunctions(names: string[]): string {
  const source = pageSource();
  const found = new Map<string, string>();
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name)
      found.set(node.name.text, node.getText(source));
    else if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.initializer &&
      (ts.isArrowFunction(node.initializer) || ts.isFunctionExpression(node.initializer))
    )
      found.set(node.name.text, `const ${node.name.text} = ${node.initializer.getText(source)};`);
    ts.forEachChild(node, visit);
  };
  visit(source);
  for (const name of names) assert.ok(found.has(name), `${name} must exist in CodeFiles.tsx`);
  return names.map((name) => found.get(name)).join('\n');
}

function compile(code: string): string {
  return ts.transpileModule(code, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
}

function run<T>(code: string, context: Record<string, unknown>, names: string[]): T {
  const factory = new Function(
    ...Object.keys(context),
    `${compile(code)}\nreturn { ${names.join(', ')} };`,
  ) as (...values: unknown[]) => Record<string, unknown>;
  return factory(...Object.values(context)) as T;
}

type Call = { command: string; args: Record<string, unknown> };

/** act() 依赖的那一堆组件状态；测试能看见父锁、错误与成功提示。 */
function operationContext(calls: Call[], invoke: (command: string, args: Call['args']) => unknown) {
  const pending: boolean[] = [];
  const errors: string[] = [];
  const notices: string[] = [];
  let changed = 0;
  const context = {
    busyRef: { current: false },
    disabled: false,
    opEpoch: { current: 0 },
    contextRef: { current: 'ver-1\u0000\u0000' },
    alive: { current: true },
    setBusy: () => {},
    onPendingChange: (value: boolean) => pending.push(value),
    setError: (value: string) => {
      if (value) errors.push(value);
    },
    setNotice: (value: string) => {
      if (value) notices.push(value);
    },
    load: async () => {},
    latest: {
      current: {
        message: 'fix k',
        onChanged: async () => {
          changed += 1;
        },
      },
    },
    t: (_zh: string, en: string) => en,
    workspaceErrorText,
    invokeJson: (command: string, args: Call['args']) => {
      calls.push({ command, args });
      return invoke(command, args);
    },
  };
  return { context, pending, errors, notices, changed: () => changed };
}

test('creating a file uses the atomic create command and never a plain write', async () => {
  const calls: Call[] = [];
  const opened: { path?: string; hash?: string | null; text?: string | null }[] = [];
  const harness = operationContext(calls, async () => ({ hash: 'a'.repeat(64) }));
  const functions = run<{ createFile: () => Promise<boolean> }>(
    extractFunctions(['act', 'createFile']),
    {
      ...harness.context,
      joinPath: (dir: string, name: string) => (dir ? `${dir}/${name}` : name),
      dir: 'vendor/CoLM202X/main',
      newPath: '  MOD_New.F90 ',
      name: 'ver-1',
      setNewPath: () => {},
      setOpen: (file: { path?: string }) => opened.push(file),
      setDraft: () => {},
      setConflict: () => {},
    },
    ['createFile'],
  );
  assert.equal(await functions.createFile(), true);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].command, 'workspace_create_file');
  assert.equal(calls[0].args.path, 'vendor/CoLM202X/main/MOD_New.F90');
  assert.equal(calls[0].args.text, '', '新建传空文本，但走的是后端原子 create_new，不是覆盖写');
  assert.equal(calls[0].args.message, 'fix k');
  // 父工作台在整个写操作期间被锁住。
  assert.deepEqual(harness.pending, [true, false]);
  assert.equal(opened[0].path, 'vendor/CoLM202X/main/MOD_New.F90');
  assert.equal(opened[0].text, '');
  assert.equal(opened[0].hash, 'a'.repeat(64), '后端返回的新哈希直接当基线，不靠再读一次');
  assert.equal(harness.notices.length, 1);
});

test('a name clash from the backend keeps the existing file and reports it', async () => {
  const calls: Call[] = [];
  const opened: unknown[] = [];
  const harness = operationContext(calls, async () => {
    throw new Error(
      'Error: already-exists: vendor/CoLM202X/main/MOD_Demo.F90 already exists in the workspace; open it instead, or replace it explicitly',
    );
  });
  const functions = run<{ createFile: () => Promise<boolean> }>(
    extractFunctions(['act', 'createFile']),
    {
      ...harness.context,
      joinPath: (dir: string, name: string) => (dir ? `${dir}/${name}` : name),
      dir: 'vendor/CoLM202X/main',
      newPath: 'MOD_Demo.F90',
      name: 'ver-1',
      setNewPath: () => {},
      setOpen: (file: unknown) => opened.push(file),
      setDraft: () => {},
      setConflict: () => {},
    },
    ['createFile'],
  );
  assert.equal(await functions.createFile(), false);
  // 失败留在原上下文：不打开文件、不提示成功、不改基线、父锁释放。
  assert.deepEqual(opened, []);
  assert.deepEqual(harness.notices, []);
  assert.deepEqual(harness.pending, [true, false]);
  assert.equal(harness.errors.length, 1);
  assert.match(harness.errors[0], /already exists; the existing file was not changed/);
});

test('saving sends the opened baseline and moves it to the hash the write returned', async () => {
  const calls: Call[] = [];
  const openRef = { current: { path: 'a.F90', size: 3, text: 'old', hash: 'h1' } };
  const draftRef = { current: 'new' };
  const opened: (typeof openRef.current)[] = [];
  const harness = operationContext(calls, async () => ({
    hash: calls.length === 1 ? 'h2' : 'h3',
  }));
  const functions = run<{ save: () => Promise<boolean> }>(
    extractFunctions(['showConflict', 'act', 'save']),
    {
      ...harness.context,
      openRef,
      draftRef,
      open: { path: 'a.F90' },
      name: 'ver-1',
      byteLength: (text: string) => new TextEncoder().encode(text).length,
      isContentConflict,
      readFile: async () => {
        throw new Error('not used in this test');
      },
      setOpen: (file: typeof openRef.current) => {
        openRef.current = file;
        opened.push(file);
      },
      setDraft: (text: string) => {
        draftRef.current = text;
      },
      setConflict: () => {},
    },
    ['save'],
  );
  assert.equal(await functions.save(), true);
  assert.equal(calls[0].args.expectedHash, 'h1', '保存带打开时的内容基线');
  assert.equal(opened.at(-1)?.hash, 'h2');
  assert.equal(harness.changed(), 1, '保存成功后刷新草稿的代码提交');

  // 连续保存：下一次用写入返回的新基线，不能拿旧基线把自己刚写的内容判成冲突。
  draftRef.current = 'newer';
  assert.equal(await functions.save(), true);
  assert.equal(calls[1].args.expectedHash, 'h2');
  assert.equal(opened.at(-1)?.hash, 'h3');
});

test('a stale baseline opens the conflict view instead of overwriting the external edit', async () => {
  const calls: Call[] = [];
  const openRef = { current: { path: 'a.F90', size: 3, text: 'old', hash: 'h1' } };
  const conflicts: unknown[] = [];
  const harness = operationContext(calls, async () => {
    throw new Error(
      'Error: content-conflict: a.F90 changed on disk since it was opened; reload it or overwrite explicitly',
    );
  });
  const functions = run<{ save: () => Promise<boolean> }>(
    extractFunctions(['showConflict', 'act', 'save']),
    {
      ...harness.context,
      openRef,
      draftRef: { current: 'mine' },
      open: { path: 'a.F90' },
      name: 'ver-1',
      byteLength: (text: string) => new TextEncoder().encode(text).length,
      isContentConflict,
      readFile: async () => ({ path: 'a.F90', size: 6, text: 'theirs', hash: 'h9' }),
      setOpen: () => {
        throw new Error('a conflict must not replace the buffer');
      },
      setDraft: () => {
        throw new Error('a conflict must not replace the draft');
      },
      setConflict: (value: unknown) => conflicts.push(value),
    },
    ['save'],
  );
  assert.equal(await functions.save(), false);
  assert.deepEqual(conflicts, [
    { path: 'a.F90', exists: true, text: 'theirs', hash: 'h9', size: 6 },
  ]);
  // 冲突不是普通错误：不进错误条，用户文本留在缓冲区，由冲突面板处理。
  assert.deepEqual(harness.errors, []);
  assert.deepEqual(harness.pending, [true, false]);
});

test('a file deleted outside the app becomes a recreate/conflict decision, not a silent write', async () => {
  const calls: Call[] = [];
  const openRef = { current: { path: 'a.F90', size: 3, text: 'old', hash: 'h1' } };
  const conflicts: unknown[] = [];
  const harness = operationContext(calls, async () => {
    throw new Error(
      'Error: content-conflict: a.F90 was deleted or replaced outside the app since it was opened; reload it or save it again explicitly',
    );
  });
  const functions = run<{ save: () => Promise<boolean> }>(
    extractFunctions(['showConflict', 'act', 'save']),
    {
      ...harness.context,
      openRef,
      draftRef: { current: 'mine' },
      open: { path: 'a.F90' },
      name: 'ver-1',
      byteLength: (text: string) => new TextEncoder().encode(text).length,
      isContentConflict,
      readFile: async () => {
        throw new Error('a.F90 does not exist');
      },
      setOpen: () => {
        throw new Error('a conflict must not touch the buffer');
      },
      setDraft: () => {},
      setConflict: (value: unknown) => conflicts.push(value),
    },
    ['save'],
  );
  assert.equal(await functions.save(), false);
  assert.deepEqual(conflicts, [
    { path: 'a.F90', exists: false, text: null, hash: null, size: null },
  ]);
});

test('a clean buffer reloads after an external commit, a dirty one only shows the conflict', async () => {
  const openRef = { current: { path: 'a.F90', size: 3, text: 'old', hash: 'h1' } };
  const draftRef = { current: 'old' };
  const conflicts: unknown[] = [];
  const context = {
    openRef,
    draftRef,
    conflictRef: { current: null },
    recheckEpoch: { current: 0 },
    contextRef: { current: 'ver-1\u0000\u0000' },
    alive: { current: true },
    readFile: async () => ({ path: 'a.F90', size: 3, text: 'new', hash: 'h2' }),
    setOpen: (file: typeof openRef.current) => {
      openRef.current = file;
    },
    setDraft: (text: string) => {
      draftRef.current = text;
    },
    setConflict: (value: unknown) => conflicts.push(value),
  };
  const functions = run<{ recheckOpen: () => Promise<void> }>(
    extractFunctions(['recheckOpen']),
    context,
    ['recheckOpen'],
  );
  // 干净缓冲区：外部提交变化后自动换成磁盘内容，不打断、不弹冲突。
  await functions.recheckOpen();
  assert.equal(openRef.current.text, 'new');
  assert.equal(openRef.current.hash, 'h2');
  assert.deepEqual(conflicts, [null], 'auto-reload clears any earlier conflict state');
  // 两边都改过：保留用户文本，只显示冲突（重挂载会连文本一起销毁，所以不能那样刷新）。
  draftRef.current = 'mine';
  openRef.current = { path: 'a.F90', size: 3, text: 'old', hash: 'h1' };
  await functions.recheckOpen();
  assert.equal(openRef.current.text, 'old', '脏缓冲区的文本不能被外部内容替换');
  assert.equal(draftRef.current, 'mine');
  assert.deepEqual(conflicts, [
    null,
    { path: 'a.F90', exists: true, text: 'new', hash: 'h2', size: 3 },
  ]);
});

test('a recheck response for a file the user already switched away from is ignored', async () => {
  const openRef = { current: { path: 'a.F90', size: 3, text: 'old', hash: 'h1' } };
  const draftRef = { current: 'old' };
  let resolveRead: (file: unknown) => void = () => {};
  const context = {
    openRef,
    draftRef,
    conflictRef: { current: null },
    recheckEpoch: { current: 0 },
    contextRef: { current: 'ver-1\u0000\u0000' },
    alive: { current: true },
    readFile: () => new Promise((resolve) => (resolveRead = resolve)),
    setOpen: () => {
      throw new Error('a late response must not replace the newly opened file');
    },
    setDraft: () => {},
    setConflict: () => {},
  };
  const functions = run<{ recheckOpen: () => Promise<void> }>(
    extractFunctions(['recheckOpen']),
    context,
    ['recheckOpen'],
  );
  const running = functions.recheckOpen();
  // 用户在这期间点了另一个文件。
  openRef.current = { path: 'b.F90', size: 1, text: 'b', hash: 'hb' };
  resolveRead({ path: 'a.F90', size: 3, text: 'new', hash: 'h2' });
  await running;
  assert.equal(openRef.current.path, 'b.F90');
});

test('read-only frozen versions never register unsaved state', () => {
  const source = pageSource();
  let body = '';
  const visit = (node: ts.Node) => {
    if (
      ts.isCallExpression(node) &&
      node.expression.getText(source) === 'useEffect' &&
      node.arguments[0]?.getText(source).includes('onStateChangeRef')
    )
      body = node.arguments[0].getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, 'the state registration effect must exist');
  const runEffect = (readOnly: boolean, handler: (state: unknown) => void) => {
    const factory = new Function(
      'readOnly',
      'onStateChangeRef',
      'sourceState',
      `${compile(`const effect = ${body};`)}\nreturn effect();`,
    ) as (
      readOnly: boolean,
      onStateChangeRef: { current: (state: unknown) => void },
      sourceState: unknown,
    ) => (() => void) | undefined;
    return factory(readOnly, { current: handler }, { dirty: true });
  };
  const seen: unknown[] = [];
  const handler = (state: unknown) => seen.push(state);
  const cleanup = runEffect(true, handler);
  assert.equal(cleanup, undefined);
  assert.deepEqual(seen, [], 'browsing a frozen version must not raise an unsaved prompt');
  const cleanupLive = runEffect(false, handler);
  assert.equal(seen.length, 1, 'a writable workspace registers its buffer state');
  cleanupLive?.();
  assert.deepEqual(seen.at(-1), null, 'unmount clears the parent-side state');
});

test('a slow file read cannot replace the file the user clicked last', async () => {
  const source = pageSource();
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
    editedRef: { current: false },
    askConfirm: async () => true,
    t: (_zh: string, en: string) => en,
    setError: () => {},
    setConflict: () => {},
    setShowTheirs: () => {},
    opening: { current: 0 },
    contextRef: { current: 'ver-1\u0000\u0000' },
    alive: { current: true },
    readFile: (path: string) => new Promise((resolve) => (pending[path] = resolve)),
    setOpen: (file: { path: string }) => shown.push(file.path),
    setDraft: () => {},
    workspaceErrorText,
  };
  const code = compile(`${body}; return openFile;`);
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
