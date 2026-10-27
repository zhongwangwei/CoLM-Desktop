import test from 'node:test';
import assert from 'node:assert/strict';
import { homeResearchEntry, hubResearchEntry, hubMethods, studyResearchEntry } from './hub-methods';

const baseline = { id: 'soil', revision: 2 };
const researchKeys = ['researchBaseline', 'researchMethod', 'researchSeed', 'researchTarget'];

/**
 * U-02：从"新建修改版"与从"研究"首页进入研究页时，research* 的初始化必须来自同一份定义，
 * 否则两条入口迟早会出现"一个清、一个不清"的漂移。这组测试直接守住 helper 的形状与清理语义。
 */
test('both research entries carry exactly the same research state fields', () => {
  const hub = hubResearchEntry('tuning', baseline);
  const home = homeResearchEntry();
  assert.deepEqual(Object.keys(hub).sort(), Object.keys(home).sort());
  assert.deepEqual(Object.keys(hub).sort(), [...researchKeys].sort());
});

test('the hub entry records the chosen version as the study baseline only', () => {
  const entry = hubResearchEntry('uncertainty', baseline);
  assert.deepEqual(entry.researchBaseline, baseline);
  assert.equal(entry.researchMethod, 'uncertainty');
  assert.equal(entry.researchSeed, null);
  assert.equal(entry.researchTarget, null);
  // U-01 的核心：入口状态里绝不能出现全局运行版本字段，选研究基线不得换模拟版本。
  for (const key of ['version', 'versionRef', 'chosenVersion']) assert.equal(key in entry, false);
  assert.equal(
    hubMethods.some((method) => method.id === 'uncertainty'),
    true,
  );
});

test('the home entry clears the baseline so the research page falls back to the run revision', () => {
  assert.deepEqual(homeResearchEntry(), {
    researchBaseline: null,
    researchMethod: null,
    researchSeed: null,
    researchTarget: null,
  });
});

test('the entry baseline is a copy, not the caller’s live version object', () => {
  const live = { id: 'soil', revision: 7 };
  const entry = hubResearchEntry('tuning', live);
  live.revision = 8;
  assert.deepEqual(entry.researchBaseline, { id: 'soil', revision: 7 });
});

test('a draft research tab carries the method and ticks without inventing a write-back target', () => {
  const picks = [
    {
      parameter_id: 'p1',
      raw_key: 'DEF_X',
      scheme: null,
      index: null,
      min: '0',
      max: '1',
      scale: 'linear',
    },
  ];
  assert.deepEqual(studyResearchEntry('tuning', picks), {
    researchMethod: 'tuning',
    researchSeed: { parameters: picks },
  });
  assert.deepEqual(studyResearchEntry('tuning', []), {
    researchMethod: 'tuning',
    researchSeed: null,
  });
});
