import test from 'node:test';
import assert from 'node:assert/strict';
import { timeTicks } from './time-ticks';

const at = (iso: string) => Date.parse(iso) / 1000;

test('sub-daily ticks are 24-hour UTC times with the date on day changes', () => {
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
