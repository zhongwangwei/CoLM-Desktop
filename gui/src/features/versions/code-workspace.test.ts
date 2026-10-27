import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

type Call = { command: string; args: Record<string, unknown> };

/** 从页面源码里抽出一个函数/箭头函数声明的真实函数体（仓库惯例：别用源码字符串断言代替行为）。 */
function bodyOf(file: string, name: string): string {
  const path = new URL(file, import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path.pathname, 'utf8'),
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
      t: (zh: string) => zh,
    },
  );
  assert.throws(() => guard(), /源码文件有未保存的修改，请先保存或放弃/);
  const clean = evaluate<() => void>('./VersionsPage.tsx', 'assertSourceSaved', 'function', {
    sourceState: { current: { dirty: false } },
    t: (zh: string) => zh,
  });
  assert.doesNotThrow(() => clean());
  const empty = evaluate<() => void>('./VersionsPage.tsx', 'assertSourceSaved', 'function', {
    sourceState: { current: null },
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
