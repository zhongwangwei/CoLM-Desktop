import test from 'node:test';
import assert from 'node:assert/strict';
import { timeBasisLabel, timeBasisOf, unknownTimeBasis, type TimeBasis } from './time-basis';

test('a missing or malformed time basis degrades to the model clock, never to UTC', () => {
  const fallback = timeBasisOf({ time: [1, 2] });
  assert.equal(fallback.kind, 'model_clock');
  assert.equal(fallback.utc_offset_seconds, null);
  assert.equal(timeBasisOf(undefined).kind, 'model_clock');
  assert.equal(timeBasisOf(null).kind, 'model_clock');
  assert.equal(timeBasisOf({ time_basis: { kind: 'local_time' } }).kind, 'model_clock');
  assert.equal(timeBasisOf({ time_basis: 'utc' }).kind, 'model_clock');
  // 模型钟即使带了一个偏移也不采用：没有可靠依据的偏移不能拿来换算。
  const contaminated = timeBasisOf({
    time_basis: { kind: 'model_clock', utc_offset_seconds: 28800, calendar: 'standard' },
  });
  assert.equal(contaminated.kind, 'model_clock');
  assert.equal(contaminated.utc_offset_seconds, null);
});

test('a declared basis keeps its offset, calendar and provenance', () => {
  const basis = timeBasisOf({
    time_basis: {
      kind: 'local_standard_time',
      utc_offset_seconds: 28800,
      calendar: 'standard',
      source: 'case.nml DEF_simulation_time%greenwich=.false.',
    },
  });
  assert.deepEqual(basis, {
    kind: 'local_standard_time',
    utc_offset_seconds: 28800,
    calendar: 'standard',
    source: 'case.nml DEF_simulation_time%greenwich=.false.',
  });
  // 偏移不是有限数时按「未记录」处理，但仍是地方标准时。
  const noOffset = timeBasisOf({
    time_basis: { kind: 'local_standard_time', utc_offset_seconds: 'x', calendar: '' },
  });
  assert.equal(noOffset.utc_offset_seconds, null);
  assert.equal(noOffset.calendar, 'standard');
  assert.equal(noOffset.source, 'unspecified');
});

test('the basis label is bilingual and states the clock instead of assuming UTC', () => {
  const utc: TimeBasis = {
    kind: 'utc',
    utc_offset_seconds: 0,
    calendar: 'standard',
    source: 'grid case',
  };
  const east: TimeBasis = { ...utc, kind: 'local_standard_time', utc_offset_seconds: 28800 };
  const west: TimeBasis = { ...utc, kind: 'local_standard_time', utc_offset_seconds: -19800 };
  assert.equal(timeBasisLabel(utc), 'UTC');
  assert.equal(timeBasisLabel(utc, true), 'UTC');
  assert.equal(timeBasisLabel(east), 'UTC+08:00');
  assert.equal(timeBasisLabel(west), 'UTC-05:30');
  assert.equal(timeBasisLabel(unknownTimeBasis()), '模型钟（本地标准时，偏移未知）');
  assert.equal(
    timeBasisLabel(unknownTimeBasis(), true),
    'model clock (local standard time, offset unknown)',
  );
  assert.equal(timeBasisLabel({ ...east, utc_offset_seconds: null }), '地方标准时（偏移未记录）');
  assert.equal(
    timeBasisLabel({ ...east, utc_offset_seconds: null }, true),
    'local standard time (offset not recorded)',
  );
  assert.equal(timeBasisLabel(undefined), '模型钟（本地标准时，偏移未知）');
});
