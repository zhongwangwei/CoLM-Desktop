import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import type { VersionRef } from './types';

function opening(chosenVersion: VersionRef | null = null, recentFails = false) {
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
        return { version: bindings[String(args.dir)] };
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
  };
  const code = ts.transpileModule(`const callback = (${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const openCase = new Function(...Object.keys(context), `${code}; return callback;`)(
    ...Object.values(context),
  ) as (dir: string, tab?: 'settings' | 'results') => Promise<{ binding: { version: VersionRef } }>;
  return { openCase, current, reads, notices, official, soil };
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
