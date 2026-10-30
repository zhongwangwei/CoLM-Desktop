/** 两份图表数据是否逐点相同（null 与 NaN 也按相等算）。 */
export function sameSeries(
  a: readonly (ArrayLike<number | null | undefined> | null | undefined)[],
  b: readonly (ArrayLike<number | null | undefined> | null | undefined)[],
): boolean {
  if (a === b) return true;
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i++) {
    const x = a[i],
      y = b[i];
    if (x === y) continue;
    if (!x || !y || x.length !== y.length) return false;
    for (let j = 0; j < x.length; j++) {
      const u = x[j],
        v = y[j];
      if (u !== v && !(Number.isNaN(u) && Number.isNaN(v))) return false;
    }
  }
  return true;
}

/**
 * 把缺测区间插回降采样后的时间轴，让图上的折线在缺测处断开。
 *
 * 为什么需要：2400 点抽样是按步长跳着取的，一小段连续缺测（比如 21 条记录）比步长还短时
 * 会被整段跳过，抽样结果里既没有 null 也没有空点，uPlot 就会用直线连过去，看起来像有数据。
 * 这里只在缺测区间的端点插入“所有列均为 null”的行：该时刻没有任何抽样样本，给其它变量
 * 编造插值就是造假，所以哪怕别的变量在这个区间外有值，标记行也一律 null、让所有线一起断。
 *
 * 只保留显示窗口内的端点：窗口外的缺测画不到；缺测从窗口左边界之前就开始时只插右端，
 * 延续到右边界之外时只插左端（贴着窗口边缘插标记只会制造假断点）。时间轴里已有同值时刻时
 * 复用原行、不改它的值 —— 那个时刻确实存在抽样样本，说明它不是缺测区间的内部。
 */
export function insertGapMarkers(
  time: number[],
  vars: Record<string, (number | null)[]>,
  gaps?: Record<string, [number, number][]>,
): { time: number[]; columns: (number | null)[][] } {
  const names = Object.keys(vars);
  for (const name of names)
    if (vars[name].length !== time.length) throw new Error('series columns must align with time');
  const columns = names.map((name) => vars[name]);
  if (!gaps || !time.length) return { time, columns };
  const first = time[0];
  const last = time[time.length - 1];
  const markers = new Set<number>();
  for (const name of names) {
    for (const [start, end] of gaps[name] ?? []) {
      if (
        !Number.isFinite(start) ||
        !Number.isFinite(end) ||
        start >= end ||
        // [start, end) 与显示窗口 [first, last] 没有交集。
        end <= first ||
        start >= last
      )
        continue;
      // 严格落在窗口内部的端点才插：正好压在边界上的那一端贴着图的边缘。
      if (start > first && start < last) markers.add(start);
      if (end > first && end < last) markers.add(end);
    }
  }
  if (!markers.size) return { time, columns };
  const sorted = [...markers].sort((a, b) => a - b);
  const mergedTime: number[] = [];
  const merged: (number | null)[][] = names.map(() => []);
  let at = 0;
  let next = 0;
  while (next < sorted.length) {
    // 时间轴本来递增；把标记点按序并进去。
    while (at < time.length && time[at] < sorted[next]) {
      mergedTime.push(time[at]);
      for (let column = 0; column < columns.length; column++)
        merged[column].push(columns[column][at]);
      at++;
    }
    if (at < time.length && time[at] === sorted[next]) {
      // 已有同一时刻的抽样行：复用，不重复插、也不动它原来的数值。
      mergedTime.push(time[at]);
      for (let column = 0; column < columns.length; column++)
        merged[column].push(columns[column][at]);
      at++;
    } else {
      mergedTime.push(sorted[next]);
      for (const column of merged) column.push(null);
    }
    next++;
  }
  while (at < time.length) {
    mergedTime.push(time[at]);
    for (let column = 0; column < columns.length; column++)
      merged[column].push(columns[column][at]);
    at++;
  }
  return { time: mergedTime, columns: merged };
}
