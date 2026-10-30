import assert from 'node:assert/strict';
import test from 'node:test';
import { colorFraction, colorRange, scaleValueText, type MapColorScale } from './GridMap';

test('auto color range follows each frame min/max', () => {
  // 自动模式：换一帧就换一套 min/max，保持原行为。
  assert.deepEqual(colorRange(12.5, 31.5), { low: 12.5, high: 31.5 });
  assert.deepEqual(colorRange(-3.25, 9), { low: -3.25, high: 9 });
  // 全缺测帧 / 旧 sidecar 没有 min/max：没有可用范围，画布不着色。
  assert.equal(colorRange(null, null), undefined);
  assert.equal(colorRange(undefined, undefined), undefined);
  assert.equal(colorRange(1, null), undefined);
  assert.equal(colorRange(null, 1), undefined);
  assert.equal(colorRange(Number.NaN, 2), undefined);
  assert.equal(colorRange(1, Number.POSITIVE_INFINITY), undefined);
});

test('fixed color range survives frames whose min/max changed or disappeared', () => {
  const fixed: MapColorScale = { low: 0, high: 10 };
  // 固定色标跨帧不变：新帧的 min/max 再大再小都不影响着色范围。
  assert.equal(colorRange(2, 4, fixed), fixed);
  assert.equal(colorRange(-50, 500, fixed), fixed);
  // 固定后切到全缺测帧（后端 min/max 为 null）也保留锁定范围：
  // 画布按 null 值保持空白，不能重新按 0..1 把整图涂成一色。
  assert.equal(colorRange(null, null, fixed), fixed);
  // 解除固定 = 不再传范围，回到每帧自动。
  assert.deepEqual(colorRange(2, 4, null), { low: 2, high: 4 });
});

test('fixed mode without a captured range falls back to auto or empty', () => {
  // 切到固定时当前帧全缺测：没有可锁定的范围，保持 undefined（画布空白）。
  assert.equal(colorRange(null, null, undefined), undefined);
  assert.equal(colorRange(null, null, null), undefined);
  // 之后再有有效帧时仍可回退到自动范围着色，而不是把 undefined 当 0..1 用。
  assert.deepEqual(colorRange(3, 7, undefined), { low: 3, high: 7 });
});

test('color fraction clamps values outside the range to 0..1', () => {
  const range: MapColorScale = { low: 10, high: 20 };
  assert.equal(colorFraction(10, range), 0);
  assert.equal(colorFraction(15, range), 0.5);
  assert.equal(colorFraction(20, range), 1);
  assert.equal(colorFraction(-1, range), 0);
  assert.equal(colorFraction(99, range), 1);
  // min == max 的退化范围按跨度 1 处理，与旧画布 `(max - min) || 1` 一致。
  assert.equal(colorFraction(5, { low: 5, high: 5 }), 0);
  assert.equal(colorFraction(6, { low: 5, high: 5 }), 1);
});

test('scale endpoint text keeps the existing compact number format', () => {
  assert.equal(scaleValueText(2.5), '2.50');
  assert.equal(scaleValueText(999.99), '999.99');
  assert.equal(scaleValueText(1500), '1.50e+3');
  assert.equal(scaleValueText(-1500), '-1.50e+3');
  assert.equal(scaleValueText(0.005), '5.00e-3');
  assert.equal(scaleValueText(0.02), '0.02');
  assert.equal(scaleValueText(0), '0.00');
  // 没有范围（全缺测帧）沿用旧图例的长横线。
  assert.equal(scaleValueText(null), '—');
  assert.equal(scaleValueText(undefined), '—');
});
