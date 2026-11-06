import test from 'node:test';
import assert from 'node:assert/strict';
import { parseProjectRunSettings, serializeProjectRunSettings } from './project-run-settings';

test('project batch settings survive a valid round trip', () => {
  const settings = {
    root: '/project',
    cases: ['/project/a', '/project/b'],
    batch: true,
    host: 'A',
    ranks: '2',
    threads: '8',
    workers: '2',
    scheduler: 'slurm',
    cpus: '',
    stage: 'colm',
    force: true,
    preprocessors: 'rust',
    recordInputFingerprints: true,
  };
  assert.deepEqual(parseProjectRunSettings(serializeProjectRunSettings(settings)), settings);
});

test('corrupt or foreign shapes are ignored instead of restored', () => {
  for (const raw of [
    null,
    '',
    '{',
    '[]',
    'null',
    '"text"',
    '{}',
    JSON.stringify({ root: '', cases: [] }),
    JSON.stringify({ root: '/p', cases: 'not a list' }),
    JSON.stringify({ root: '/p', cases: [1, 2] }),
    JSON.stringify({ root: '/p', cases: new Array(1001).fill('/p/c') }),
  ])
    assert.equal(parseProjectRunSettings(raw as string | null), undefined, String(raw));
  assert.equal(serializeProjectRunSettings(undefined), '{}');
  assert.equal(serializeProjectRunSettings([1]), '{}');
});
