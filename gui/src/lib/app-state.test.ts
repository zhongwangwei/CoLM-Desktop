import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import type { VersionRef } from './types';
import {
  missingRevision,
  rebindQuestion,
  tidyModelsAfterStartup,
  versionLabel,
  withoutRevisions,
} from './model-tidy';

function opening(chosenVersion: VersionRef | null = null, recentFails = false, rebind = true) {
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
  const bindings: Record<string, VersionRef> = {
    '/soil': soil,
    '/official': { id: 'official', revision: 1 },
    '/missing': { id: 'removed', revision: 1 },
  };
  const invoke = async (command: string, args: Record<string, unknown>) => {
    switch (command) {
      case 'open_case':
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
    openEpoch: { current: 0 },
    pendingOpen: { current: null },
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
  ) as (dir: string, tab?: 'settings' | 'results') => Promise<{ binding: { version: VersionRef } }>;
  return { openCase, current, reads, notices, questions, migrations, official, soil };
}

test('openCase respects an explicitly chosen revision over a saved custom binding', async () => {
  const chosen = { id: 'soil', revision: 2 };
  const app = opening(chosen);
  const opened = await app.openCase('/soil');
  assert.deepEqual(app.reads, [chosen]);
  assert.deepEqual(app.current.current.versionRef, chosen);
  assert.deepEqual(opened.binding.version, app.soil);
  assert.equal(app.current.current.busy, false);
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
  assert.equal(opened.binding.version.id, 'soil');
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
  const notify = new Function('update', 'useCallback', `${code}; return callback;`)(
    update,
    (callback: unknown) => callback,
  ) as (message: string, error?: boolean) => void;
  notify('saved');
  notify('cannot reach server', true);
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
  assert.deepEqual(opened.binding.version, app.official);
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

function booting(prune: () => Promise<unknown>) {
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
  const invoke = async (command: string) => {
    calls.push(command);
    if (command === 'load_recent') return {};
    if (command === 'list_kernels') return [];
    return undefined;
  };
  const invokeJson = async (command: string) => {
    calls.push(command);
    if (command === 'model_list') return versions;
    if (command === 'model_prune') return prune();
    throw new Error(`Unexpected command: ${command}`);
  };
  const context = {
    alive: true,
    update: (patch: object | ((old: Record<string, unknown>) => object)) =>
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch),
    notify: (message: string) => notices.push(message),
    invoke,
    invokeJson,
    tidyModelsAfterStartup,
    withoutRevisions,
  };
  const code = ts.transpileModule(body, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const boot = new Function(...Object.keys(context), `${code}; return boot;`)(
    ...Object.values(context),
  ) as () => Promise<void>;
  return { boot, calls, notices, state };
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
  // The page itself carries a static cover, so there is no blank window before the scripts load.
  assert.match(html, /<div id="root"><div class="start-cover"/);
  // The app starts in the "starting" state and always leaves it: after the load, or at once
  // without a backend (a plain browser).
  assert.match(state, /busy: false,\n {2}starting: true,\n\};/);
  assert.match(state, /update\(\{ busy: false, starting: false \}\)/);
  assert.match(state, /if \(!hasBackend\(\)\) \{\n {6}update\(\{ starting: false \}\);/);
  assert.match(app, /<StartCover \/>/);
});
