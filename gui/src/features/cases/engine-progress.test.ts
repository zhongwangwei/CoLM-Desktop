import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement as h } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { engineStep } from './engine-progress';
import EngineProgress from './EngineProgress';

test('engine preparation steps follow what the server reports', () => {
  assert.equal(engineStep('checking'), 0);
  assert.equal(engineStep('building', null), 1, 'no engine folder yet: still uploading');
  assert.equal(engineStep('building', { engine: 'abc', stage: 'installing CMake' }), 2);
  assert.equal(engineStep('building', { engine: 'abc', stage: 'building the Rust engine' }), 3);
  assert.equal(engineStep('building', { engine: 'abc', done: 312, total: 664 }), 3);
  // The last four units are the engine programs, which take most of the time.
  assert.equal(engineStep('building', { engine: 'abc', done: 660, total: 664 }), 4);
});

test('checking says a matching engine is reused, building names the snapshot and why once', () => {
  const checking = renderToStaticMarkup(
    h(EngineProgress, { host: 'tms17l', phase: 'checking', waited: '1 秒', en: false }),
  );
  assert.match(checking, /tms17l 正在核对计算引擎/);
  assert.match(checking, /有就直接复用/);
  assert.doesNotMatch(checking, /正在准备与当前版本一致/);

  const building = renderToStaticMarkup(
    h(EngineProgress, {
      host: 'tms17l',
      phase: 'building',
      waited: '27 秒',
      en: false,
      progress: {
        engine: 'snap-0123456789abcdef',
        stage: 'building the Rust engine',
        done: 3,
        total: 664,
        current: 'colm-core',
      },
      engineId: 'snap-0123456789abcdef',
    }),
  );
  assert.match(building, /正在为当前源码快照编译计算引擎/);
  assert.match(
    building,
    /snap-012/,
    'the server-side engine folder id is shown as its first 8 chars',
  );
  assert.match(building, /没有当前源码快照的引擎/);
  assert.match(building, /同一快照编好后会复用，不会每次运行都重编/);
  assert.doesNotMatch(building, /正在准备与当前版本一致/);

  const en = renderToStaticMarkup(
    h(EngineProgress, {
      host: 'tms17l',
      phase: 'building',
      waited: '27 s',
      en: true,
      progress: {
        engine: 'snap-0123456789abcdef',
        stage: 'building the Rust engine',
        done: 3,
        total: 664,
      },
      engineId: 'snap-0123456789abcdef',
    }),
  );
  assert.match(en, /snapshot snap-012/);
  assert.match(en, /reuse it instead of rebuilding/);
});
