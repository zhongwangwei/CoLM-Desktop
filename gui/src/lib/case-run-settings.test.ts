import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CASE_RUN_SETTINGS_LIMIT,
  parseCaseRunSettings,
  serializeCaseRunSettings,
} from './case-run-settings';

test('case run settings only restore object-shaped entries', () => {
  assert.equal(parseCaseRunSettings(null), undefined);
  assert.equal(parseCaseRunSettings('not json'), undefined);
  assert.equal(parseCaseRunSettings('[]'), undefined);
  assert.equal(parseCaseRunSettings('{}'), undefined);
  assert.deepEqual(parseCaseRunSettings('{"case":{"threads":"6"}}'), {
    case: { threads: '6' },
  });
  // 坏条目丢弃，好条目保留。
  assert.deepEqual(parseCaseRunSettings('{"bad":1,"ok":{"threads":"2"}}'), {
    ok: { threads: '2' },
  });
});

test('case run settings keep a bounded number of case directories', () => {
  const settings: Record<string, Record<string, unknown>> = {};
  for (let index = 0; index < CASE_RUN_SETTINGS_LIMIT + 5; index++)
    settings[`/case/${index}`] = { threads: String(index) };
  const parsed = parseCaseRunSettings(serializeCaseRunSettings(settings))!;
  assert.equal(Object.keys(parsed).length, CASE_RUN_SETTINGS_LIMIT);
  assert.equal(parsed['/case/0'], undefined, 'oldest entries are evicted');
  assert.equal(
    parsed[`/case/${CASE_RUN_SETTINGS_LIMIT + 4}`]?.threads,
    String(CASE_RUN_SETTINGS_LIMIT + 4),
  );
  assert.equal(serializeCaseRunSettings(undefined), '{}');
});
