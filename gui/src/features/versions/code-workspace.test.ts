import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

type Call = { command: string; args: Record<string, unknown> };

/** 从页面源码里抽出一个函数/箭头函数声明的真实函数体（仓库惯例：别用源码字符串断言代替行为）。 */
function bodyOf(file: string, name: string): string {
  // Windows 上 URL.pathname 是 "/D:/..."，直接交给 readFileSync 会拼成 "D:\D:\..."。
  const path = fileURLToPath(new URL(file, import.meta.url));
  const source = ts.createSourceFile(
    path,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    if (
      ts.isVariableDeclaration(node) &&
      ts.isIdentifier(node.name) &&
      node.name.text === name &&
      node.initializer
    )
      body = node.initializer.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, `${name} must exist in ${file}`);
  return body;
}

function evaluate<T>(
  file: string,
  name: string,
  declaration: 'const' | 'function',
  context: Record<string, unknown>,
): T {
  const source =
    declaration === 'const'
      ? `const ${name} = ${bodyOf(file, name)}; return ${name};`
      : `${bodyOf(file, name)}; return ${name};`;
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const factory = new Function(...Object.keys(context), code) as (...values: unknown[]) => unknown;
  return factory(...Object.values(context)) as T;
}

function harness(sourceChoiceAnswer: boolean) {
  const calls: Call[] = [];
  const executed: number[] = [];
  const context = {
    selectedCheck: { value: 'build-engine', needs: undefined },
    checkCase: '',
    sourceChoice: { ask: async () => sourceChoiceAnswer },
    sourceRef: { current: { dirty: true } },
    execute: (work: () => Promise<unknown>) => {
      executed.push(executed.length + 1);
      return work();
    },
    invokeJson: (command: string, args: Call['args']) => {
      calls.push({ command, args });
      return Promise.resolve({});
    },
    name: 'ver-1',
    preset: 'default',
    pkg: 'colm-core',
  };
  return {
    runCheck: evaluate<() => void>('./CodeWorkspace.tsx', 'runCheck', 'const', context),
    calls,
    executed,
  };
}

test('running a compile-like check first resolves unsaved source edits', async () => {
  // 用户取消：编译不开始，缓冲区原样保留。
  const cancelled = harness(false);
  cancelled.runCheck();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.deepEqual(cancelled.executed, [], 'a cancelled prompt must not start the check');
  assert.deepEqual(cancelled.calls, []);

  // 用户保存或放弃：检查照常发出，命令参数与界面上选的一致。
  const proceed = harness(true);
  proceed.runCheck();
  await new Promise((resolve) => setTimeout(resolve, 0));
  assert.equal(proceed.executed.length, 1);
  assert.equal(proceed.calls[0].command, 'workspace_check');
  assert.deepEqual(proceed.calls[0].args, {
    name: 'ver-1',
    action: 'build-engine',
    preset: 'default',
    package: 'colm-core',
    case: null,
  });
});

test('switching version or workspace is refused while source edits are unsaved', () => {
  const guard = evaluate<(state?: unknown) => void>(
    './VersionsPage.tsx',
    'assertSourceSaved',
    'function',
    {
      sourceState: { current: { dirty: true } },
      parameterPending: { current: false },
      t: (zh: string) => zh,
    },
  );
  assert.throws(() => guard(), /源码文件有未保存的修改，请先保存或放弃/);
  const clean = evaluate<() => void>('./VersionsPage.tsx', 'assertSourceSaved', 'function', {
    sourceState: { current: { dirty: false } },
    parameterPending: { current: false },
    t: (zh: string) => zh,
  });
  assert.doesNotThrow(() => clean());
  const empty = evaluate<() => void>('./VersionsPage.tsx', 'assertSourceSaved', 'function', {
    sourceState: { current: null },
    parameterPending: { current: false },
    t: (zh: string) => zh,
  });
  assert.doesNotThrow(() => empty());
});

test('freezing first resolves unsaved source edits before taking the page lock', async () => {
  // 源码未保存的三选一在拿页面锁之前问：冻结时页面 busy 会把 CodeWorkspace 整个 disabled，
  // 那时再问，对话框里的"保存并继续"会被 CodeFiles 的 disabled 检查拒掉。
  const calls: string[] = [];
  const context = {
    sourceChoice: {
      ask: async (state: unknown) => {
        calls.push(`ask:${(state as { dirty?: boolean } | null)?.dirty ?? 'none'}`);
        return false;
      },
    },
    sourceState: { current: { dirty: true } },
    dirtyNow: { current: false },
  };
  const ready = evaluate<() => Promise<boolean>>(
    './VersionsPage.tsx',
    'sourceReady',
    'const',
    context,
  );
  assert.equal(await ready(), false, 'a cancelled prompt keeps the page and the buffer');
  assert.deepEqual(calls, ['ask:true']);
  // 用户放行后才继续，保存/放弃已在对话框里完成。
  context.sourceChoice = {
    ask: async (state: unknown) => {
      calls.push(`ask:${(state as { dirty?: boolean } | null)?.dirty ?? 'none'}`);
      return true;
    },
  };
  const proceed = evaluate<() => Promise<boolean>>(
    './VersionsPage.tsx',
    'sourceReady',
    'const',
    context,
  );
  assert.equal(await proceed(), true);
  assert.deepEqual(calls, ['ask:true', 'ask:true']);
});

test('unfinished parameter input blocks switching and protects an automatically created draft', () => {
  const parameterPending = { current: true };
  const guard = evaluate<() => void>('./VersionsPage.tsx', 'assertSourceSaved', 'function', {
    sourceState: { current: null },
    parameterPending,
    t: (zh: string) => zh,
  });
  assert.throws(guard, /参数输入尚未完成/);
  const drop = evaluate<() => void>('./VersionsPage.tsx', 'dropIfUntouched', 'function', {
    parameterPending,
    dirtyNow: { current: false },
    draftNow: { current: { id: 'new', generation: 1, code: { workspace: null } } },
    autoCreated: { current: new Set(['new']) },
    invokeJson: () => assert.fail('unfinished input must not delete its draft'),
  });
  drop();
});

test('unfinished parameter input cannot be saved or frozen, and explicit discard clears it', async () => {
  const parameterPending = { current: true };
  for (const name of ['save', 'freeze']) {
    const action = evaluate<(draft: object, unsaved: boolean) => Promise<unknown>>(
      './VersionsPage.tsx',
      name,
      'function',
      { parameterPending, t: (zh: string) => zh },
    );
    await assert.rejects(action({}, false), /参数输入尚未完成/);
  }
  const closed: boolean[] = [];
  const discard = evaluate<() => Promise<void>>('./VersionsPage.tsx', 'discardLocal', 'function', {
    parameterPending,
    draft: { id: 'new' },
    invokeJson: async () => ({ id: 'new' }),
    setParamsOpen: (open: boolean) => closed.push(open),
    setDraft: () => {},
    dirtyNow: { current: true },
    setDirty: () => {},
  });
  await discard();
  assert.equal(parameterPending.current, false);
  assert.deepEqual(closed, [false]);
});

test('discarding invalid parameter text does not delete a draft with valid edits', () => {
  const drop = evaluate<() => void>('./VersionsPage.tsx', 'dropIfUntouched', 'function', {
    parameterPending: { current: false },
    dirtyNow: { current: true },
    draftNow: { current: { id: 'new', generation: 1, code: { workspace: null } } },
    autoCreated: { current: new Set(['new']) },
    invokeJson: () =>
      assert.fail('valid edits must keep their draft after invalid input is discarded'),
  });
  drop();
});

test('freezing saves parameter edits before offering to save a dirty source buffer', async () => {
  const calls: string[] = [];
  const context = {
    sourceState: { current: { dirty: true } },
    dirtyNow: { current: true },
    draftNow: { current: { id: 'draft-1' } },
    execute: async (run: () => Promise<unknown>) => run(),
    save: async () => {
      calls.push('save-draft');
      return { id: 'draft-1' };
    },
    sourceChoice: {
      ask: async () => {
        calls.push('source-choice');
        return true;
      },
    },
  };
  const ready = () =>
    evaluate<() => Promise<boolean>>('./VersionsPage.tsx', 'sourceReady', 'const', context)();
  assert.equal(await ready(), true);
  assert.deepEqual(calls, ['save-draft', 'source-choice']);
  calls.length = 0;
  context.save = async () => {
    throw Error('write failed');
  };
  assert.equal(await ready(), false);
  assert.deepEqual(calls, [], 'do not continue after a failed draft save');
});

test('a delayed code branch preserves parameter edits made while the request was pending', async () => {
  const original = { id: 'd', generation: 1, code: { workspace: null }, overlay: { value: 1 } };
  let release!: (value: unknown) => void;
  const response = new Promise((resolve) => {
    release = resolve;
  });
  const draftNow = { current: original };
  const dirtyNow = { current: false };
  const written: unknown[] = [];
  const dirty: boolean[] = [];
  const branch = evaluate<() => Promise<void>>('./VersionsPage.tsx', 'branchCode', 'function', {
    draft: original,
    draftNow,
    dirtyNow,
    epoch: { current: { active: true } },
    save: async () => original,
    invokeJson: (command: string) =>
      command === 'model_branch_code' ? response : Promise.resolve([]),
    setDraft: (value: unknown) => written.push(value),
    setDirty: (value: boolean) => dirty.push(value),
    setDrafts: () => {},
  });
  const pending = branch();
  draftNow.current = { ...original, overlay: { value: 7 } };
  dirtyNow.current = true;
  release({ ...original, generation: 2, code: { workspace: 'copy' } });
  await pending;
  assert.deepEqual(written, [
    { ...original, generation: 2, code: { workspace: 'copy' }, overlay: { value: 7 } },
  ]);
  assert.deepEqual(dirty, [true]);
});

test('a workspace referenced by the open draft cannot be deleted', async () => {
  const errors: string[] = [];
  const remove = evaluate<() => Promise<void>>('./CodeWorkspace.tsx', 'deleteWorkspace', 'const', {
    name: 'copy',
    draftName: 'Modified model',
    setError: (value: string) => errors.push(value),
    t: (_zh: string, en: string) => en,
    askConfirm: () => {
      throw Error('must not ask to delete a referenced workspace');
    },
  });
  await remove();
  assert.match(errors[0], /still referenced by a draft/);
});

test('patch import and both revert actions notify the open source buffer of disk changes', async () => {
  for (const [name, declaration, argument] of [
    ['patch', 'function', undefined],
    ['revertFile', 'const', 'a.F90'],
    ['revertToCommit', 'const', { id: 'abc', subject: 'old source' }],
  ] as const) {
    let token = 0,
      refreshed = 0;
    const pending: Promise<unknown>[] = [];
    const action = evaluate<(value?: unknown) => Promise<unknown>>(
      './CodeWorkspace.tsx',
      name,
      declaration,
      {
        name: 'copy',
        message: 'change',
        invoke: async () => '/tmp/change.patch',
        invokeJson: async () => ({}),
        askConfirm: async () => true,
        t: (_zh: string, en: string) => en,
        commitId: () => 'abc',
        shortHash: () => 'abc',
        setExternalChangeToken: (update: (old: number) => number) => {
          token = update(token);
        },
        onChanged: async () => {
          refreshed++;
        },
        run: (work: () => Promise<unknown>) => {
          pending.push(work());
        },
      },
    );
    await action(argument);
    await Promise.all(pending);
    assert.equal(token, 1, name);
    assert.equal(refreshed, 1, name);
  }
});
