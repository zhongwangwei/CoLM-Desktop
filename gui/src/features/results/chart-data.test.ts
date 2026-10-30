import assert from 'node:assert/strict';
import test from 'node:test';
import { insertGapMarkers, sameSeries } from './chart-data';

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

test('inserts null markers for a 21-record gap that the 2400-point sampling skipped', () => {
  // 100001 条记录、图表最多 2400 点 → 步长 42。连续缺测 21 条比步长还短，
  // 抽样时整段被跳过（两端抽样行直接相邻），图上就会用直线连过缺测段。
  const sourceN = 100_001;
  const seconds = 60;
  const stride = Math.ceil(sourceN / 2400);
  const missingFrom = 5041;
  const missingCount = 21;
  const time: number[] = [];
  const gpp: (number | null)[] = [];
  const le: (number | null)[] = [];
  for (let index = 0; index < sourceN; index += stride) {
    time.push(index * seconds);
    gpp.push(index);
    le.push(index * 2);
  }
  if (time.at(-1) !== (sourceN - 1) * seconds) {
    time.push((sourceN - 1) * seconds);
    gpp.push(sourceN - 1);
    le.push((sourceN - 1) * 2);
  }
  const gapStart = missingFrom * seconds;
  const gapEnd = (missingFrom + missingCount) * seconds;
  assert.ok(!time.includes(gapStart) && !time.includes(gapEnd));
  const aligned = insertGapMarkers(time, { GPP: gpp, LE: le }, { GPP: [[gapStart, gapEnd]] });
  assert.equal(aligned.time.length, time.length + 2);
  assert.equal(aligned.columns[0].length, aligned.time.length);
  assert.equal(aligned.columns[1].length, aligned.time.length);
  for (let index = 1; index < aligned.time.length; index++)
    assert.ok(aligned.time[index] > aligned.time[index - 1], '时间轴必须保持递增');
  const startAt = aligned.time.indexOf(gapStart);
  const endAt = aligned.time.indexOf(gapEnd);
  assert.ok(startAt > 0 && endAt === startAt + 1);
  // 标记行所有列都是 null：该时刻没有抽样样本，不能给别的变量编造数值。
  assert.deepEqual(
    aligned.columns.map((column) => column[startAt]),
    [null, null],
  );
  assert.deepEqual(
    aligned.columns.map((column) => column[endAt]),
    [null, null],
  );
  // 两侧的真实抽样值原样保留。
  assert.equal(aligned.columns[0][startAt - 1], 5040);
  assert.equal(aligned.columns[0][endAt + 1], 5082);
  assert.equal(aligned.columns[1][startAt - 1], 10080);
  assert.equal(aligned.columns[1][endAt + 1], 10164);
});

test('ignores gaps that fall completely outside the displayed window', () => {
  const time = [0, 60, 120];
  const vars = { GPP: [1, 2, 3] };
  const aligned = insertGapMarkers(time, vars, {
    GPP: [
      [-120, -60],
      [180, 240],
      // 贴着边界但整段在窗口外的区间同样不插。
      [120, 180],
      [-60, 0],
    ],
  });
  assert.deepEqual(aligned, { time: [0, 60, 120], columns: [[1, 2, 3]] });
});

test('clips a partially visible gap to the end that lies inside the window', () => {
  const time = [0, 60, 120, 180];
  const vars = { GPP: [1, 2, 3, 4] };
  // 缺测从窗口之前开始：只插窗口内的右端。
  const fromLeft = insertGapMarkers(time, vars, { GPP: [[-60, 90]] });
  assert.deepEqual(fromLeft, { time: [0, 60, 90, 120, 180], columns: [[1, 2, null, 3, 4]] });
  // 缺测延续到窗口之后：只插窗口内的左端。
  const fromRight = insertGapMarkers(time, vars, { GPP: [[90, 300]] });
  assert.deepEqual(fromRight, { time: [0, 60, 90, 120, 180], columns: [[1, 2, null, 3, 4]] });
  // 缺测盖住整个窗口：没有窗口内的端点可插，保持原样。
  const whole = insertGapMarkers(time, vars, { GPP: [[-60, 300]] });
  assert.deepEqual(whole, { time: [0, 60, 120, 180], columns: [[1, 2, 3, 4]] });
});

test('reuses an existing time entry instead of duplicating a marker row', () => {
  const time = [0, 60, 120];
  const vars = { GPP: [1, null, 3] };
  const aligned = insertGapMarkers(time, vars, { GPP: [[60, 90]] });
  assert.deepEqual(aligned, { time: [0, 60, 90, 120], columns: [[1, null, null, 3]] });
});

test('merges and deduplicates markers from several variables and gaps', () => {
  const aligned = insertGapMarkers(
    [0, 100, 200, 300],
    { GPP: [1, 2, 3, 4], LE: [5, 6, 7, 8] },
    {
      GPP: [
        [240, 260],
        [40, 60],
      ],
      LE: [[40, 60]],
    },
  );
  assert.deepEqual(aligned, {
    time: [0, 40, 60, 100, 200, 240, 260, 300],
    columns: [
      [1, null, null, 2, 3, null, null, 4],
      [5, null, null, 6, 7, null, null, 8],
    ],
  });
});

test('keeps the aligned columns untouched when no gaps are reported', () => {
  const time = [0, 60];
  // columns 顺序 = Object.keys 顺序，不按字母序重排。
  const aligned = insertGapMarkers(time, { b: [1, 2], a: [3, 4] });
  assert.deepEqual(aligned, {
    time: [0, 60],
    columns: [
      [1, 2],
      [3, 4],
    ],
  });
  assert.deepEqual(insertGapMarkers([], {}), { time: [], columns: [] });
});

test('rejects series whose columns do not align with the time axis', () => {
  assert.throws(() => insertGapMarkers([0, 60], { GPP: [1] }), /align with time/);
  assert.throws(
    () => insertGapMarkers([0], { GPP: [1], LE: [] }),
    /series columns must align with time/,
  );
});
