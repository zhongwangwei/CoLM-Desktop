import test from 'node:test';
import assert from 'node:assert/strict';
import { researchVersion, refreshDraftCode } from './research-version';
import type { ModelDraft, FrozenRevision } from '../../lib/types';
const draft = {
  id: 'edited',
  generation: 3,
  overlay: { physics: { DEF_USE_CROP: '.true.' } },
} as unknown as ModelDraft;
const parent = { version: { id: 'parent', revision: 1 } } as FrozenRevision,
  frozen = { version: { id: 'edited', revision: 2 } } as FrozenRevision;
test('research saves the displayed dirty draft before freezing, never selecting its parent', async () => {
  const calls: string[] = [];
  const result = await researchVersion(draft, true, parent, '/assets', {
    save: async (d) => {
      assert.equal(d, draft);
      calls.push('save');
      return { ...d, generation: 4 };
    },
    freeze: async (id, root) => {
      calls.push('freeze');
      assert.equal(id, 'edited');
      assert.equal(root, '/assets');
      return frozen;
    },
  });
  assert.deepEqual(calls, ['save', 'freeze']);
  assert.equal(result, frozen);
});
test('saved drafts still freeze and failed saves never freeze or select a parent', async () => {
  let freezes = 0;
  const operations = {
    save: async () => {
      throw new Error('generation conflict');
    },
    freeze: async () => {
      freezes++;
      return frozen;
    },
  };
  await assert.rejects(researchVersion(draft, true, parent, '', operations), /generation conflict/);
  assert.equal(freezes, 0);
  assert.equal(await researchVersion(draft, false, parent, '', operations), frozen);
  assert.equal(freezes, 1);
  assert.equal(await researchVersion(null, false, parent, '', operations), parent);
  assert.equal(freezes, 1);
});
test('freezing a configuration that already exists returns that version', async () => {
  assert.equal(
    await researchVersion(draft, false, parent, '', {
      save: async (d) => d,
      freeze: async () => parent,
    }),
    parent,
  );
});
test('code refresh cannot overwrite edits created before or during its pending operation', async () => {
  let dirty = true,
    calls = 0;
  const refresh = async () => {
    calls++;
    return { ...draft, generation: 4 };
  };
  await assert.rejects(
    refreshDraftCode(draft, () => dirty, refresh),
    /not overwritten/,
  );
  assert.equal(calls, 0);
  dirty = false;
  await assert.rejects(
    refreshDraftCode(
      draft,
      () => dirty,
      async () => {
        dirty = true;
        return refresh();
      },
    ),
    /not overwritten/,
  );
  dirty = false;
  assert.equal((await refreshDraftCode(draft, () => dirty, refresh)).generation, 4);
});
