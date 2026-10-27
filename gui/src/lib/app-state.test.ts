import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import type { VersionRef } from './types';
import { plainError } from './bilingual';
import { closeReasons } from './navigation-guard';
import {
  missingRevision,
  rebindQuestion,
  selectionAfterPrune,
  tidyModelsAfterStartup,
  versionLabel,
  withoutRevisions,
} from './model-tidy';

function opening(
  chosenVersion: VersionRef | null = null,
  recentFails = false,
  rebind = true,
  listing?: Promise<void>,
  root = '',
) {
  const path = new URL('./app-state.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'openCase')
      body = node.initializer!.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, 'exercise the actual openCase callback');
  const official = { id: 'official', revision: 3 };
  const soil = { id: 'soil', revision: 1 };
  const current = {
    current: {
      chosenVersion,
      root,
      versionRef: chosenVersion ?? official,
      versions: [soil, { id: 'soil', revision: 2 }, official, { id: 'official', revision: 1 }].map(
        (version) => ({ version }),
      ),
      busy: false,
    },
  };
  const reads: VersionRef[] = [];
  const notices: string[] = [];
  const questions: string[] = [];
  const migrations: Record<string, unknown>[] = [];
  const opens: string[] = [];
  const openEpoch = { current: 0 };
  const bindings: Record<string, VersionRef> = {
    '/soil': soil,
    '/official': { id: 'official', revision: 1 },
    '/missing': { id: 'removed', revision: 1 },
  };
  const invoke = async (command: string, args: Record<string, unknown>) => {
    switch (command) {
      case 'open_case':
        opens.push(String(args.dir));
        return { entry: { dir: args.dir, name: args.dir }, root: '/project' };
      case 'model_migrate_case':
        migrations.push(args);
        if (args.dir === '/tidied' && !args.target)
          throw 'Error: model revision official#2 is no longer in the model registry (unused old official revisions are tidied away); rebind the case to an available revision';
        return { version: (args.target as VersionRef) ?? bindings[String(args.dir)] };
      case 'model_get': {
        const version = { id: String(args.id), revision: Number(args.revision) };
        reads.push(version);
        return { version };
      }
      case 'list_cases':
        await listing;
        return [];
      case 'save_recent':
        if (recentFails) throw new Error('recent store is read-only');
        return;
      default:
        throw new Error(`Unexpected command: ${command}`);
    }
  };
  const context = {
    current,
    openEpoch,
    pendingOpen: { current: null },
    inflightOpen: { current: null },
    blocked: () => null,
    update: (patch: object) => Object.assign(current.current, patch),
    notify: (message: string) => notices.push(message),
    invoke,
    invokeJson: invoke,
    bilingual: (_zh: string, en: string) => en,
    useCallback: (callback: unknown) => callback,
    missingRevision,
    rebindQuestion,
    versionLabel,
    askToRebind: (question: string) => {
      questions.push(question);
      return rebind;
    },
  };
  const code = ts.transpileModule(`const callback = (${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const openCase = new Function(...Object.keys(context), `${code}; return callback;`)(
    ...Object.values(context),
  ) as (
    dir: string,
    tab?: 'settings' | 'results',
  ) => Promise<{ binding: { version: VersionRef } } | null>;
  return {
    openCase,
    current,
    reads,
    notices,
    questions,
    migrations,
    official,
    soil,
    opens,
    openEpoch,
  };
}

test('openCase respects an explicitly chosen revision over a saved custom binding', async () => {
  const chosen = { id: 'soil', revision: 2 };
  const app = opening(chosen);
  const opened = await app.openCase('/soil');
  assert.deepEqual(app.reads, [chosen]);
  assert.deepEqual(app.current.current.versionRef, chosen);
  assert.deepEqual(opened?.binding.version, app.soil);
  assert.equal(app.current.current.busy, false);
});

test('an explicit choice does not follow the user into another project', async () => {
  const chosen = { id: 'soil', revision: 2 };
  const same = opening(chosen, false, true, undefined, '/project');
  await same.openCase('/official');
  assert.deepEqual(same.current.current.versionRef, chosen);
  const other = opening(chosen, false, true, undefined, '/elsewhere');
  await other.openCase('/soil');
  assert.deepEqual(other.reads, [other.soil]);
  assert.deepEqual(other.current.current.versionRef, other.soil);
  assert.equal(other.current.current.chosenVersion, null);
});

test('openCase enters the requested tab as part of the same navigation', async () => {
  const app = opening();
  await app.openCase('/soil', 'results');
  assert.equal((app.current.current as unknown as { caseTab: string }).caseTab, 'results');
});

test('openCase restores saved custom bindings without making them an explicit choice', async () => {
  const app = opening();
  await app.openCase('/soil');
  assert.deepEqual(app.current.current.versionRef, app.soil);
  assert.equal(app.current.current.chosenVersion, null);
  await app.openCase('/official');
  assert.deepEqual(app.reads, [app.soil, app.official]);
  assert.deepEqual(app.current.current.versionRef, app.official);
});

test('openCase uses the latest official revision for old official or missing bindings', async () => {
  const app = opening();
  await app.openCase('/official');
  await app.openCase('/missing');
  assert.deepEqual(app.reads, [app.official, app.official]);
});

test('openCase respects explicit custom and official choices for either saved binding', async () => {
  for (const chosen of [
    { id: 'soil', revision: 2 },
    { id: 'official', revision: 1 },
  ]) {
    const app = opening(chosen);
    await app.openCase('/soil');
    await app.openCase('/official');
    assert.deepEqual(app.reads, [chosen, chosen]);
    assert.deepEqual(app.current.current.chosenVersion, chosen);
  }
});

test('openCase still reports success when only remembering the project root fails', async () => {
  const { openCase, current, notices } = opening(null, true);
  const opened = await openCase('/soil', 'results');
  assert.equal(opened?.binding.version.id, 'soil');
  assert.equal((current.current as { route?: string }).route, 'case');
  assert.equal(current.current.busy, false);
  assert.deepEqual(notices, ['Error: recent store is read-only']);
});

test('error notices keep their error style until they scroll out or are dismissed', () => {
  const path = new URL('./app-state.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'notify')
      body = node.initializer!.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body);
  const state: { notices: string[]; errorNotices?: string[] } = { notices: [] };
  const update = (patch: (old: typeof state) => Partial<typeof state>) =>
    Object.assign(state, patch(state));
  const code = ts.transpileModule(`const callback = (${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const notify = new Function('update', 'useCallback', 'plainError', `${code}; return callback;`)(
    update,
    (callback: unknown) => callback,
    plainError,
  ) as (message: string, error?: boolean) => void;
  notify('saved');
  notify('Error: cannot reach server', true);
  assert.deepEqual(state.notices, ['saved', 'cannot reach server']);
  assert.deepEqual(state.errorNotices, ['cannot reach server']);
  for (const message of ['a', 'b', 'c', 'd', 'e']) notify(message);
  assert.ok(!state.notices.includes('cannot reach server'));
  assert.deepEqual(state.errorNotices, [], 'an error that scrolled out is forgotten');
});

test('openCase offers to rebind a case whose revision was tidied away, then opens it', async () => {
  const app = opening();
  const opened = await app.openCase('/tidied');
  assert.equal(app.questions.length, 1);
  assert.match(
    app.questions[0],
    /model revision official#2, which is no longer in the model registry/,
  );
  assert.match(app.questions[0], /Rebind it to official#3/);
  assert.deepEqual(app.migrations, [{ dir: '/tidied' }, { dir: '/tidied', target: app.official }]);
  assert.deepEqual(opened?.binding.version, app.official);
  assert.deepEqual(app.current.current.versionRef, app.official);
  assert.deepEqual(app.notices, ['Rebound the case to official#3 (official#2 was tidied away).']);
});

test('openCase rebinds a tidied case to the explicitly chosen revision', async () => {
  const chosen = { id: 'soil', revision: 2 };
  const app = opening(chosen);
  await app.openCase('/tidied');
  assert.deepEqual(app.migrations[1], { dir: '/tidied', target: chosen });
});

test('declining the rebind leaves the case unopened with a plain explanation', async () => {
  const app = opening(null, false, false);
  await assert.rejects(
    app.openCase('/tidied'),
    /official#2 is no longer in the model registry; it was not rebound or opened/,
  );
  assert.equal(app.migrations.length, 1, 'nothing rebinds the case');
  assert.equal(app.current.current.busy, false);
  assert.equal((app.current.current as { route?: string }).route, undefined);
});

function booting(
  prune: () => Promise<unknown>,
  failOn = '',
  timeoutOn = '',
  options: {
    /** 第 n 次（从 1 起）`model_list` 的返回值；默认直接给出三个官方修订。 */
    modelList?: (call: number) => Promise<{ version: VersionRef }[]>;
    /** 第 n 次（从 1 起）`list_kernels` 的返回值；默认空列表。 */
    kernels?: (call: number) => Promise<unknown>;
    /** 第 n 次（从 1 起）`backend_ready` 的返回值。 */
    backendReady?: (call: number) => Promise<unknown>;
    /** 只让前几次匹配 `timeoutOn` 的调用超时（默认每次匹配的都超时）。 */
    timeouts?: number;
  } = {},
) {
  let failing = failOn;
  const path = new URL('./app-state.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.getText(source) === 'boot')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, 'exercise the actual boot function');
  const versions = [1, 2, 3].map((revision) => ({ version: { id: 'official', revision } }));
  const calls: string[] = [];
  const notices: string[] = [];
  const state: Record<string, unknown> = {};
  const guard = (command: string) => {
    if (command === failing) throw new Error(`${command} is unreachable`);
  };
  let modelListCalls = 0,
    kernelCalls = 0,
    backendCalls = 0;
  const invoke = async (command: string) => {
    calls.push(command);
    guard(command);
    if (command === 'backend_ready') {
      backendCalls++;
      if (options.backendReady) return options.backendReady(backendCalls);
      return undefined;
    }
    if (command === 'load_recent') return { root: '/last/project' };
    if (command === 'list_cases') return [{ dir: '/last/project/a', name: 'a' }];
    if (command === 'list_kernels') {
      kernelCalls++;
      return options.kernels ? options.kernels(kernelCalls) : [];
    }
    return undefined;
  };
  const invokeJson = async (command: string) => {
    calls.push(command);
    guard(command);
    if (command === 'model_list') {
      modelListCalls++;
      return options.modelList ? options.modelList(modelListCalls) : versions;
    }
    if (command === 'model_prune') return prune();
    throw new Error(`Unexpected command: ${command}`);
  };
  let timedOut = 0;
  const bootEpoch = { current: 0 };
  const context = {
    alive: true,
    update: (patch: object | ((old: Record<string, unknown>) => object)) =>
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch),
    notify: (message: string) => notices.push(message),
    invoke,
    invokeJson,
    bootEpoch,
    withTimeout: <T>(promise: Promise<T>, _ms: number, what: string) =>
      what === timeoutOn && (options.timeouts === undefined || timedOut++ < options.timeouts)
        ? Promise.reject(new Error(`${what} timed out`))
        : promise,
    bilingual: (_zh: string, en: string) => en,
    errorText: (error: unknown) => (error instanceof Error ? error.message : String(error)),
    tidyModelsAfterStartup,
    selectionAfterPrune,
    withoutRevisions,
  };
  const code = ts.transpileModule(body, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const boot = new Function(...Object.keys(context), `${code}; return boot;`)(
    ...Object.values(context),
  ) as () => Promise<void>;
  return {
    boot,
    calls,
    notices,
    state,
    versions,
    bootEpoch,
    setFailure: (command = '') => {
      failing = command;
    },
  };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

test('startup tidies old official revisions only after the version list is loaded', async () => {
  const app = booting(async () => ({ removed: [{ id: 'official', revision: 2 }] }));
  await app.boot();
  await settle();
  assert.ok(app.calls.indexOf('model_prune') > app.calls.indexOf('model_list'), String(app.calls));
  assert.deepEqual(
    (app.state.versions as { version: VersionRef }[]).map((v) => v.version.revision),
    [1, 3],
  );
  assert.equal(app.state.busy, false);
});

test('a prune that removes the selected official revision falls back to the latest kept one', async () => {
  const app = booting(async () => ({ removed: [{ id: 'official', revision: 3 }] }));
  await app.boot();
  await settle();
  assert.deepEqual(
    (app.state.versions as { version: VersionRef }[]).map((v) => v.version.revision),
    [1, 2],
  );
  assert.deepEqual(app.state.versionRef, { id: 'official', revision: 2 });
});

test('a refused or failed startup tidy is only logged, never shown to the user', async () => {
  const warn = console.warn;
  const logged: string[] = [];
  console.warn = (message: string) => logged.push(message);
  try {
    const app = booting(async () => {
      throw '有模拟正在运行或算例正在创建，稍后再整理模型缓存';
    });
    await app.boot();
    await settle();
    assert.deepEqual(app.notices, []);
    assert.equal((app.state.versions as unknown[]).length, 3);
    assert.equal(logged.length, 1);
    assert.match(logged[0], /model cache tidy skipped/);
  } finally {
    console.warn = warn;
  }
});

test('the start cover shows from the first paint until the first load finishes', () => {
  const state = readFileSync(new URL('./app-state.tsx', import.meta.url), 'utf8');
  const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8');
  const app = readFileSync(new URL('../App.tsx', import.meta.url), 'utf8');
  const cover = readFileSync(new URL('../components/StartCover.tsx', import.meta.url), 'utf8');
  // The page itself carries a static cover, so there is no blank window before the scripts load.
  assert.match(html, /<div id="root"><div class="start-cover"/);
  // The app starts in the "starting" state and always leaves it: after the load, or at once
  // without a backend (a plain browser).
  assert.match(state, /busy: false,\n {2}starting: true,/);
  assert.match(
    state,
    /startup: \{ phase: 'booting', stage: 'backend', startedAt: Date\.now\(\) \}/,
  );
  assert.match(state, /update\(\{ busy: false, starting: false \}\)/);
  assert.match(state, /update\(\{ starting: false, startup: undefined \}\)/);
  // A failed boot stays on the cover with a retry, and a stale boot can be retried after 30 s.
  assert.match(cover, /retryStartup/);
  assert.match(cover, /重试启动/);
  assert.match(app, /<StartCover \/>/);
});

test('a startup command that never answers times out instead of spinning forever', async () => {
  const { withTimeout } = await import('./ipc');
  await assert.rejects(
    withTimeout(new Promise(() => {}), 20, 'backend_ready'),
    /timed out .*backend_ready/i,
  );
});

test('a failed startup stage is kept on the cover state, and retrying can recover', async () => {
  const app = booting(async () => ({ removed: [] }), 'model_list');
  await app.boot();
  await settle();
  const failed = app.state.startup as { phase: string; stage: string; error: string };
  assert.equal(failed.phase, 'failed');
  assert.equal(failed.stage, 'kernels');
  assert.match(failed.error, /model versions/i);
  assert.doesNotMatch(
    failed.error,
    /Error: /,
    'the cover shows the message, not the exception class',
  );
  // Kernels succeeded before versions failed: keep that half instead of clearing everything.
  assert.deepEqual(app.state.kernels, []);
  assert.equal(app.state.busy, false);
  assert.deepEqual(app.notices, [], 'the cover is the error outlet, not a transient notice');

  app.setFailure('');
  await app.boot();
  await settle();
  assert.equal(app.state.startup, undefined);
  assert.deepEqual(
    (app.state.versions as { version: VersionRef }[]).map((v) => v.version.revision),
    [1, 2, 3],
  );
  assert.equal(app.state.starting, false);
  assert.equal(app.state.busy, false);
});

test('an open overtaken by leaving the page or by another open ends quietly', async () => {
  let release!: () => void;
  const page = opening(null, false, true, new Promise<void>((done) => (release = done)));
  const first = page.openCase('/official');
  await new Promise<void>((done) => setImmediate(done));
  // The user clicks elsewhere while the case is still opening (`navigate` advances the epoch).
  page.openEpoch.current++;
  release();
  assert.equal(await first, null, 'no error for an open the user already left');
  assert.deepEqual(page.notices, []);
  assert.notEqual((page.current.current as { route?: string }).route, 'case');
});

test('clicking the same case again while it opens waits for the first open', async () => {
  let release!: () => void;
  const page = opening(null, false, true, new Promise<void>((done) => (release = done)));
  const first = page.openCase('/official', 'results');
  await new Promise<void>((done) => setImmediate(done));
  const second = page.openCase('/official', 'results');
  release();
  const [a, b] = await Promise.all([first, second]);
  assert.ok(a && a === b, 'both clicks get the same opened case');
  assert.deepEqual(page.opens, ['/official'], 'the case is read once');
});

test('startup does not reopen the project worked on last', async () => {
  // 2026-10-21 用户：重新打开总显示上次的"当前项目"，不合理；首页列出最近的项目，点了才打开。
  const app = booting(async () => ({ removed: [] }));
  await app.boot();
  await settle();
  assert.equal(app.state.root, undefined);
  assert.ok(!app.calls.includes('list_cases'), String(app.calls));
  assert.equal(app.state.busy, false);
});

// 首帧就用记住的外观与语言：同源脚本紧跟静态封面，先于模块脚本执行（GUI 审查 S10）。
test('the remembered theme and language apply before the app scripts load', () => {
  const html = readFileSync(new URL('../../index.html', import.meta.url), 'utf8');
  const boot = readFileSync(new URL('../../public/theme-boot.js', import.meta.url), 'utf8');
  assert.ok(html.indexOf('/theme-boot.js') < html.indexOf('/src/main.tsx'));
  assert.ok(html.indexOf('start-cover-status') < html.indexOf('/theme-boot.js'));
  assert.match(boot, /colm-ui-theme/);
  assert.match(boot, /colm-ui-language/);
});

// 超时只是不再干等：命令晚到的结果照样接上，失败封面随之收起（2026-10-22 复查）。
test('a startup call that answers after its timeout still completes the startup', async () => {
  const late = booting(
    async () => ({ removed: [], kept: [], bytes: 0, dry_run: false }),
    '',
    // 超时描述是给用户看的话（测试里 bilingual 取英文）。
    'the model kernels',
  );
  await late.boot();
  for (let i = 0; i < 5; i++) await settle();
  assert.deepEqual(late.state.kernels, []);
  assert.ok(Array.isArray(late.state.versions));
  assert.notEqual((late.state.startup as { phase?: string } | undefined)?.phase, 'failed');
});

// GUI-06 复核证据：首次 model_list 超时但命令还在跑 → 重试成功 → 用户选修改版 → 旧请求迟到。
// 修复前 `versionsCall.then` 无条件写 `version`/`versionRef` 为最新官方版，覆盖用户的选择。
test('a late model list from a superseded startup cannot reset the revision or catalogue', async () => {
  // 两次请求看到的是不同版本的目录：第一次出发时只有两个官方修订，重试时已经多出一个。
  // 旧请求晚到若写回列表，就是把过期目录恢复出来（任务书 GUI-06 验收第 2 条）。
  const oldCatalogue = [1, 2].map((revision) => ({ version: { id: 'official', revision } }));
  const freshCatalogue = [1, 2, 3].map((revision) => ({
    version: { id: 'official', revision },
  }));
  let releaseFirst!: (value: { version: VersionRef }[]) => void;
  const firstList = new Promise<{ version: VersionRef }[]>((resolve) => {
    releaseFirst = resolve;
  });
  const app = booting(async () => ({ removed: [] }), '', 'the model versions', {
    timeouts: 1, // 只有第一次读取超时，重试那一次正常返回
    modelList: (call) => (call === 1 ? firstList : Promise.resolve(freshCatalogue)),
  });
  await app.boot();
  await settle();
  assert.equal((app.state.startup as { phase: string }).phase, 'failed');
  const firstGeneration = app.bootEpoch.current;
  await app.boot(); // 用户在失败封面上点"重试启动"
  await settle();
  assert.equal(app.state.startup, undefined, 'the retry finishes');
  assert.deepEqual(
    app.state.versionRef,
    freshCatalogue[2].version,
    'no choice yet: latest official',
  );
  // 用户在版本页选了修改版：三处语义一起写入（VersionsPage/ProjectPage 都是这样选的）。
  const soil = { version: { id: 'soil', revision: 4 } };
  Object.assign(app.state, {
    version: soil,
    versionRef: soil.version,
    chosenVersion: soil.version,
  });
  releaseFirst(oldCatalogue); // 第一次请求现在才返回
  for (let i = 0; i < 5; i++) await settle();
  assert.equal(app.bootEpoch.current, firstGeneration + 1);
  assert.deepEqual(
    app.state.versionRef,
    soil.version,
    'the late result must not write official back',
  );
  assert.deepEqual(app.state.version, soil);
  assert.deepEqual(app.state.chosenVersion, soil.version);
  assert.deepEqual(
    app.state.versions,
    freshCatalogue,
    'the stale catalogue is discarded, not restored',
  );
  assert.equal(app.state.startup, undefined);
  assert.equal(app.calls.filter((command) => command === 'model_list').length, 2);
});

// GUI-06 验收："旧请求不能恢复已经过期的目录或失败封面"。
test('a superseded startup cannot restore its kernel directory or hide the newer failure', async () => {
  let releaseKernels!: (value: { preset: string; dir: string }[]) => void;
  const firstKernels = new Promise<{ preset: string; dir: string }[]>((resolve) => {
    releaseKernels = resolve;
  });
  const app = booting(async () => ({ removed: [] }), '', 'the model kernels', {
    timeouts: 1,
    kernels: (call) => (call === 1 ? firstKernels : Promise.resolve([])),
  });
  await app.boot();
  await settle();
  assert.equal((app.state.startup as { phase: string }).phase, 'failed');
  // 重试这一次在连接后台时就失败：失败封面属于当前这一代。
  app.setFailure('backend_ready');
  await app.boot();
  await settle();
  assert.deepEqual(
    [
      (app.state.startup as { phase: string }).phase,
      (app.state.startup as { stage: string }).stage,
    ],
    ['failed', 'backend'],
  );
  releaseKernels([{ preset: 'stale', dir: '/stale/kernel' }]);
  for (let i = 0; i < 5; i++) await settle();
  assert.equal(app.state.kernelDir, undefined, 'the stale kernel directory is not restored');
  assert.equal(app.state.kernels, undefined);
  assert.equal(
    (app.state.startup as { stage: string }).stage,
    'backend',
    'the superseded attempt cannot clear the newer failure cover',
  );
});

// GUI-06 验收："无用户选择的首次慢启动仍可以正确完成"。
test('a first model list answering after its timeout completes startup with the latest official', async () => {
  const official = [1, 2, 3].map((revision) => ({ version: { id: 'official', revision } }));
  let release!: (value: { version: VersionRef }[]) => void;
  const list = new Promise<{ version: VersionRef }[]>((resolve) => {
    release = resolve;
  });
  const app = booting(async () => ({ removed: [] }), '', 'the model versions', {
    modelList: () => list,
  });
  await app.boot();
  await settle();
  assert.equal((app.state.startup as { phase: string }).phase, 'failed');
  release(official);
  for (let i = 0; i < 5; i++) await settle();
  assert.equal(app.state.startup, undefined, 'the late answer retires the failure cover');
  assert.deepEqual(app.state.versionRef, official[2].version);
  assert.deepEqual(app.state.version, official[2]);
  assert.equal(app.state.starting, false);
});

// GUI-06："用户已选模型时，背景结果不得重新初始化为官方版"——同一次启动内也成立：
// 用户先在失败封面上点"先进入应用"并选好修改版，迟到的首次模型列表不能再改回来。
test('a late model list does not reset a revision chosen while that startup was still running', async () => {
  const official = [1, 2, 3].map((revision) => ({ version: { id: 'official', revision } }));
  let release!: (value: { version: VersionRef }[]) => void;
  const list = new Promise<{ version: VersionRef }[]>((resolve) => {
    release = resolve;
  });
  const app = booting(async () => ({ removed: [] }), '', 'the model versions', {
    modelList: () => list,
  });
  await app.boot();
  await settle();
  const soil = { version: { id: 'soil', revision: 5 } };
  Object.assign(app.state, {
    startup: undefined, // "先进入应用（部分功能不可用）"
    version: soil,
    versionRef: soil.version,
    chosenVersion: soil.version,
  });
  release([...official, soil]);
  for (let i = 0; i < 5; i++) await settle();
  assert.deepEqual(app.state.versionRef, soil.version);
  assert.deepEqual(app.state.version, soil);
  assert.deepEqual(app.state.chosenVersion, soil.version);
  assert.equal(app.state.startup, undefined);
  // 同一代次内迟到的列表仍然接上（超时只是不再干等，代次没有变）：只更新目录、不改选择。
  assert.ok(
    (app.state.versions as { version: VersionRef }[]).some((entry) => entry.version.id === 'soil'),
    'a late list within the same startup still refreshes the catalogue',
  );
});

// GUI-06："过期结果不能覆盖当前启动状态"——首次启动还卡在连接后台时又重试，旧的那次最后失败，
// 不能把新一次已经收起的封面重新放出来，也不能把它标成还在启动。
test('a slow first startup cannot overwrite the finished state of a newer retry', async () => {
  let rejectBackend!: (error: unknown) => void;
  const firstReady = new Promise<unknown>((_resolve, reject) => {
    rejectBackend = reject;
  });
  const app = booting(async () => ({ removed: [] }), '', '', {
    backendReady: (call) => (call === 1 ? firstReady : Promise.resolve(undefined)),
  });
  const first = app.boot();
  await settle();
  await app.boot();
  await settle();
  assert.equal(app.state.startup, undefined, 'the retry finishes');
  rejectBackend(new Error('the first connection finally failed'));
  await first;
  await settle();
  assert.equal(app.state.startup, undefined, 'the superseded failure does not put the cover back');
  assert.equal(app.state.busy, false);
  assert.equal(app.state.starting, false);
});

test('closing asks about unsaved edits once each and about local runs, never about remote jobs', () => {
  const unsaved = () => 'The case settings are not saved.';
  const reasons = closeReasons(
    [unsaved, unsaved, () => null],
    {
      '/a': { running: true },
      '/b': { running: true, location: 'remote' },
      '/c': { running: false },
    },
    (count) => `${count} local`,
  );
  assert.deepEqual(reasons, ['The case settings are not saved.', '1 local']);
  assert.deepEqual(closeReasons([], {}, String), []);
});
