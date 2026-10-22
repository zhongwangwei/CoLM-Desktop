import assert from 'node:assert/strict';
import test from 'node:test';
import { sameSeries } from './chart-data';

test('chart data rebuilt with the same values counts as unchanged', () => {
  const time = [1, 2, 3];
  assert.ok(
    sameSeries(
      [time, [1, null, NaN]],
      [
        [1, 2, 3],
        [1, null, NaN],
      ],
    ),
  );
  assert.ok(!sameSeries([time, [1, 2, 3]], [time, [1, 2, 4]]));
  assert.ok(!sameSeries([time, [1, 2]], [time, [1, 2, 3]]));
  assert.ok(!sameSeries([time], [time, [1, 2, 3]]));
});
