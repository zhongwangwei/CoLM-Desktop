import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

const created = [{ created: true, dir: '/proj/case', name: 'case' }];

/**
 * 从 NewCasePage 源码里抽出真实的 `openCreatedProject`，在给定 IPC 结果下执行。
 * 这是"创建成功 → 打开项目"的最后一段：项目内容读取与最近记录写入都发生在这里（GUI-15）。
 */
function opening(
  options: {
    listCases?: () => Promise<unknown>;
    saveRecent?: () => Promise<unknown>;
    navigateOpens?: boolean;
  } = {},
) {
  const path = new URL('./NewCasePage.tsx', import.meta.url);
  const source = ts.createSourceFile(
    path.pathname,
    readFileSync(path, 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.getText(source) === 'openCreatedProject')
      body = node.getText(source);
    ts.forEachChild(node, visit);
  };
  visit(source);
  assert.ok(body, 'exercise the actual openCreatedProject function');
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const notices: string[] = [];
  const updates: Record<string, unknown>[] = [];
  const navigations: string[] = [];
  const revision = { id: 'official', revision: 2 };
  const invoke = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'list_cases')
      return options.listCases ? options.listCases() : [{ dir: '/proj/case', name: 'case' }];
    if (command === 'save_recent') return options.saveRecent ? options.saveRecent() : undefined;
    throw new Error(`Unexpected command: ${command}`);
  };
  const context = {
    t: (_zh: string, en: string) => en,
    form: { host: '', root: '/proj', name: 'case' },
    caseDirectory: (root: string, name: string) => `${root}/${name}`,
    invoke,
    alive: { current: true },
    update: (patch: Record<string, unknown>) => updates.push(patch),
    lock: { current: true },
    navigate: (route: string) => {
      navigations.push(route);
      return options.navigateOpens ?? true;
    },
    state: { versionRef: revision },
    runSettings: { workers: '4' },
    notify: (message: string) => notices.push(message),
  };
  const code = ts.transpileModule(body, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const openCreatedProject = new Function(
    ...Object.keys(context),
    `${code}; return openCreatedProject;`,
  )(...Object.values(context)) as (
    results: unknown[],
    revision?: { id: string; revision: number },
  ) => Promise<void>;
  return { openCreatedProject, calls, notices, updates, navigations, revision };
}

// GUI-15 复核证据：`list_cases` 后直接 await `save_recent`，存储只读时已创建的项目被报成失败。
test('a recent-entry write failure still opens the project that was created', async () => {
  const app = opening({
    saveRecent: async () => {
      throw new Error('recent store is read-only');
    },
  });
  await app.openCreatedProject(created);
  assert.equal(app.notices.length, 1);
  assert.match(app.notices[0], /recent-project entry could not be updated/);
  assert.match(app.notices[0], /project was created and can still be used/);
  assert.deepEqual(app.navigations, ['project'], 'the created project opens anyway');
  assert.equal(app.updates.length, 1);
  const patch = app.updates[0];
  assert.equal(patch.root, '/proj/case');
  assert.deepEqual(patch.cases, [{ dir: '/proj/case', name: 'case' }]);
  assert.equal(patch.selectedCase, null);
  assert.equal(patch.newCaseSeed, null);
  assert.deepEqual(patch.versionRef, app.revision);
  assert.equal(patch.projectCasesOpen, true);
  assert.deepEqual(patch.projectRunSettings, {
    workers: '4',
    root: '/proj/case',
    cases: ['/proj/case'],
  });
});

// GUI-15 验收："重试查看已有项目不会重复创建站点"——重试只重新读列表与最近记录，不再创建。
test('retrying "View created project" never creates the sites again', async () => {
  let saved = 0;
  const app = opening({
    saveRecent: async () => {
      saved++;
      if (saved === 1) throw new Error('recent store is read-only');
    },
  });
  await app.openCreatedProject(created);
  await app.openCreatedProject(created); // 用户点"查看已创建的项目"
  assert.equal(saved, 2);
  assert.equal(app.notices.length, 1, 'the second attempt writes the entry without a warning');
  assert.deepEqual(
    app.calls.map((call) => call.command),
    ['list_cases', 'save_recent', 'list_cases', 'save_recent'],
    'no create command runs on retry',
  );
  assert.deepEqual(app.navigations, ['project', 'project']);
  assert.equal(app.updates.length, 2);
});

// GUI-15 验收："错误信息区分项目创建失败和最近记录未更新"：项目已创建、只是读不到内容时，
// 报错必须说清项目已创建，并指向只读列表的重试入口。
test('an unreadable created project is reported as created, not as a failed creation', async () => {
  const app = opening({
    listCases: async () => {
      throw new Error('cannot open the project folder');
    },
  });
  await assert.rejects(
    app.openCreatedProject(created),
    /The project was created, but reading its cases failed: Error: cannot open the project folder.*View created project/,
  );
  assert.equal(app.notices.length, 0);
  assert.equal(app.updates.length, 0, 'nothing is written when the project content is unreadable');
  assert.deepEqual(app.navigations, []);
});
