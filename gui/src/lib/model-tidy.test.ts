import test from 'node:test';
import assert from 'node:assert/strict';
import { formatBytes, missingRevision, withoutRevisions } from './model-tidy';
import type { FrozenRevision } from './types';

test('a tidied revision is recognized in the CLI error however it arrives', () => {
  const text =
    'Error: model revision official#42 is no longer in the model registry (unused old official revisions are tidied away); rebind the case to an available revision';
  for (const error of [text, new Error(text)])
    assert.deepEqual(missingRevision(error), { id: 'official', revision: 42 });
  assert.equal(missingRevision('read revision.json: No such file or directory'), null);
});

test('removed revisions leave the version list and sizes read naturally', () => {
  const versions = [
    { version: { id: 'official', revision: 1 } },
    { version: { id: 'official', revision: 2 } },
    { version: { id: 'soil', revision: 1 } },
  ] as FrozenRevision[];
  assert.deepEqual(
    withoutRevisions(versions, [{ id: 'official', revision: 1 }]).map((v) => v.version),
    [
      { id: 'official', revision: 2 },
      { id: 'soil', revision: 1 },
    ],
  );
  assert.equal(formatBytes(8.57 * 1024 ** 3), '8.57 GB');
  assert.equal(formatBytes(126 * 1024 ** 2), '126 MB');
  assert.equal(formatBytes(10), '1 KB');
});
