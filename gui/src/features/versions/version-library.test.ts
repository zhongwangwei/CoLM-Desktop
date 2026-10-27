import test from 'node:test';
import assert from 'node:assert/strict';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { removable, usageLabel } = await import('./VersionLibrary');

const official = (revision: number) => ({ id: 'official', revision });
const usage = (
  revision: number,
  cases: number,
  history: number,
  blocker: string | null = null,
) => ({
  version: official(revision),
  cases,
  history,
  blocker,
});

test('a revision only history names can be removed; bound, selected and protected ones cannot', () => {
  const selected = official(92);
  assert.ok(removable(usage(83, 0, 95), selected));
  assert.equal(usageLabel(usage(83, 0, 95), selected, false), '只有 95 条历史记录');
  assert.ok(removable(usage(6, 0, 0), selected));
  assert.equal(usageLabel(usage(6, 0, 0), selected, true), 'Unused');
  assert.ok(!removable(usage(70, 90, 270), selected));
  assert.equal(usageLabel(usage(70, 90, 270), selected, false), '90 个算例在用');
  assert.ok(!removable(usage(92, 0, 0), selected), 'the selected version stays');
  assert.equal(usageLabel(usage(92, 0, 0), selected, false), '当前选中');
  const latest = usage(
    100,
    0,
    0,
    'the latest official revision is the bundled baseline and cannot be deleted',
  );
  assert.ok(!removable(latest, selected));
  assert.equal(usageLabel(latest, selected, false), '当前官方版本');
  const parent = usage(95, 0, 1, 'draft model-1 is based on it; delete that draft first');
  assert.ok(!removable(parent, selected));
  assert.equal(usageLabel(parent, selected, true), 'Has derived versions or drafts');
  assert.ok(!removable(undefined, selected), 'nothing is offered before usage is known');
});

test('every draft can be deleted from the library', async () => {
  const { createElement } = await import('react');
  const { renderToStaticMarkup } = await import('react-dom/server');
  const { default: RevisionTree } = await import('./VersionLibrary');
  const official = {
    version: { id: 'official', revision: 101 },
    name: 'Official CoLM',
    parent: null,
  } as unknown as import('../../lib/types').FrozenRevision;
  const draft = (id: string, workspace: string | null) =>
    ({
      id,
      name: id,
      parent: official.version,
      code: { kind: workspace ? 'workspace' : 'official', workspace, commit: null },
      overlay: { parameters: [], physics: {}, process_files: [], hybrid: null },
      created_unix: 0,
    }) as unknown as import('../../lib/types').ModelDraft;
  const html = renderToStaticMarkup(
    createElement(RevisionTree, {
      versions: [official],
      drafts: [draft('PFT test', null), draft('roots', 'ver-1')],
      inUse: official,
      disabled: false,
      en: false,
      onOpen: () => {},
      onUse: () => {},
      onOpenDraft: () => {},
      onDeleted: async () => {},
      onDraftDeleted: async () => {},
    }),
  );
  // Each draft row offers editing and deleting; drafts name what they are based on.
  const rows = html.split('<tr').filter((row) => row.includes('继续编辑'));
  assert.equal(rows.length, 2);
  assert.ok(rows.every((row) => row.includes('>删除<')));
  assert.ok(rows.every((row) => row.includes('Official CoLM · r101')));
  assert.ok(rows[1]!.includes('自己的代码'), 'a draft with its own code says so');
});

test('version and draft rows register their buttons while deletion stays manual', async () => {
  const { createElement } = await import('react');
  const { renderToStaticMarkup } = await import('react-dom/server');
  const { default: RevisionLibrary } = await import('./VersionLibrary');
  const official = {
    version: { id: 'official', revision: 101 },
    name: 'Official CoLM',
    parent: null,
    code: { kind: 'official', workspace: null, commit: null },
    overlay: { parameters: [], physics: {}, process_files: [], hybrid: null },
    created_unix: 0,
    note: '',
  } as unknown as import('../../lib/types').FrozenRevision;
  const modified = {
    ...official,
    version: { id: 'soil', revision: 2 },
    name: 'Soil change',
    parent: official.version,
    code: { kind: 'workspace', workspace: 'ver-1', commit: 'abc' },
  } as unknown as import('../../lib/types').FrozenRevision;
  const draft = {
    id: 'draft-1',
    name: 'Draft',
    parent: modified.version,
    code: { kind: 'workspace', workspace: 'ver-2', commit: 'def' },
    overlay: { parameters: [], physics: {}, process_files: [], hybrid: null },
    created_unix: 0,
  } as unknown as import('../../lib/types').ModelDraft;
  const html = renderToStaticMarkup(
    createElement(RevisionLibrary, {
      versions: [official, modified],
      drafts: [draft],
      inUse: official,
      disabled: false,
      en: true,
      onOpen: () => {},
      onUse: () => {},
      onOpenDraft: () => {},
      onDeleted: async () => {},
      onDraftDeleted: async () => {},
    }),
  );
  // 点版本名、行里的打开/设为运行模型、草稿名和继续编辑，都是助手能按的动作。
  for (const action of [
    'version-select-official-101',
    'version-open-official-101',
    'version-select-soil-2',
    'version-open-soil-2',
    'version-use-soil-2',
    'version-draft-draft-1',
    'version-resume-draft-1',
  ])
    assert.ok(html.includes(`data-action="${action}"`), action);
  // 删除版本、删除草稿、"清理没用的版本"属于 D 级：界面上有按钮，但没有登记。
  assert.ok(html.includes('>Delete<'), '草稿的删除按钮在界面上');
  assert.doesNotMatch(html, /data-action="[^"]*(delete|remove|cleanup)/i);
});
