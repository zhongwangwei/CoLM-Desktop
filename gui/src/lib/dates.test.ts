import test from 'node:test';
import assert from 'node:assert/strict';
import { utcSeconds } from './dates';

test('UTC seconds accept date-only and minute/second forms without relying on engine-specific Date.parse', () => {
  assert.equal(utcSeconds('2020-01-01'), 1577836800);
  assert.equal(utcSeconds('2020-01-01T00:00'), 1577836800);
  assert.equal(utcSeconds('2020-01-01T01:02:03'), 1577836800 + 3723);
  for (const bad of ['', '2020-1-1', '2020-02-30', '2020-01-01T24:00', '2020-01-01Z', 'x'])
    assert.ok(Number.isNaN(utcSeconds(bad)), bad);
});
