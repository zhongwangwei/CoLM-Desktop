import test from 'node:test';
import assert from 'node:assert/strict';
import { choosableVersions, latestOfficial, schemeVersionName } from './version-tree';
import type { FrozenRevision, VersionRef } from '../../lib/types';

const revision = (
  id: string,
  rev: number,
  parent: VersionRef | null,
  created = rev,
): FrozenRevision =>
  ({
    version: { id, revision: rev },
    name: id,
    note: `${id} note`,
    parent,
    created_unix: created,
  }) as FrozenRevision;

test('only the newest revision of each version is offered, as a source tree', () => {
  const versions = [
    revision('official', 1, null),
    revision('official', 2, null),
    revision('official', 3, null),
    revision('pc', 1, { id: 'official', revision: 2 }, 10),
    revision('pc-soil', 1, { id: 'pc', revision: 1 }, 11),
    revision('bgc', 1, { id: 'official', revision: 3 }, 12),
    revision('bgc', 2, { id: 'official', revision: 3 }, 13),
  ];
  const rows = choosableVersions(versions);
  assert.deepEqual(
    rows.map((row) => [`${row.version.version.id}@${row.version.version.revision}`, row.depth]),
    [
      ['official@3', 0],
      ['pc@1', 1],
      ['pc-soil@1', 2],
      ['bgc@2', 1],
    ],
  );
  assert.equal(latestOfficial(versions)?.version.revision, 3);
});

test('scheme names replace earlier scheme suffixes instead of growing', () => {
  assert.equal(schemeVersionName('Official CoLM', 'PFT'), 'Official CoLM · PFT');
  assert.equal(schemeVersionName('Official CoLM · PFT', 'PC'), 'Official CoLM · PC');
  assert.equal(schemeVersionName('Official CoLM · PC · LCT · PC', 'PFT'), 'Official CoLM · PFT');
  assert.equal(schemeVersionName('Soil test', 'LCT'), 'Soil test · LCT');
});
