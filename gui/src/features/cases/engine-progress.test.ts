import test from 'node:test';
import assert from 'node:assert/strict';
import { engineStep } from './engine-progress';

test('engine preparation steps follow what the server reports', () => {
  assert.equal(engineStep('checking'), 0);
  assert.equal(engineStep('building', null), 1, 'no engine folder yet: still uploading');
  assert.equal(engineStep('building', { engine: 'abc', stage: 'installing CMake' }), 2);
  assert.equal(engineStep('building', { engine: 'abc', stage: 'building the Rust engine' }), 3);
  assert.equal(engineStep('building', { engine: 'abc', done: 312, total: 664 }), 3);
  // The last four units are the engine programs, which take most of the time.
  assert.equal(engineStep('building', { engine: 'abc', done: 660, total: 664 }), 4);
});
