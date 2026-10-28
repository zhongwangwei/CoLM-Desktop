import test from 'node:test';
import assert from 'node:assert/strict';
import { formatModelClock, modelClockIso, timeAxisLabel, timeTicks } from './time-ticks';
import type { TimeBasis } from './time-basis';

const at = (iso: string) => Date.parse(iso) / 1000;

test('sub-daily ticks are 24-hour wall-clock times with the date on day changes', () => {
  const ticks = timeTicks(
    null,
    [at('2008-01-01T12:00Z'), at('2008-01-01T18:00Z'), at('2008-01-02T00:00Z')],
    0,
    0,
    21600,
  );
  assert.deepEqual(ticks, ['12:00\n2008-01-01', '18:00', '00:00\n2008-01-02']);
});

test('daily and monthly ticks are ISO dates', () => {
  assert.deepEqual(timeTicks(null, [at('2008-03-05T00:00Z')], 0, 0, 86400), ['2008-03-05']);
  assert.deepEqual(timeTicks(null, [at('2008-03-01T00:00Z')], 0, 0, 86400 * 31), ['2008-03']);
});

test('wall-clock formatting stays UTC-based and never appends a timezone marker', () => {
  // 数值是「模型钟当 UTC 编码」的 epoch 秒；用本地 getter 会整体平移一个时区。
  assert.equal(formatModelClock(at('2008-01-01T00:30Z')), '2008-01-01 00:30');
  assert.equal(modelClockIso(at('2008-01-01T00:30Z')), '2008-01-01T00:30:00.000');
  assert.ok(!modelClockIso(at('2008-01-01T00:30Z')).endsWith('Z'));
  assert.equal(modelClockIso(at('2008-07-01T23:59:59.500Z')), '2008-07-01T23:59:59.500');
  assert.equal(formatModelClock(Number.NaN), '');
});

test('the axis label comes from the basis instead of a hard-coded UTC', () => {
  const local: TimeBasis = {
    kind: 'local_standard_time',
    utc_offset_seconds: 28800,
    calendar: 'standard',
    source: 'case.nml',
  };
  assert.equal(timeAxisLabel(undefined), '模型钟（本地标准时，偏移未知）');
  assert.equal(timeAxisLabel(local), 'UTC+08:00');
  assert.equal(timeAxisLabel({ ...local, kind: 'utc', utc_offset_seconds: 0 }, true), 'UTC');
});
