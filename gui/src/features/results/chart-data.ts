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
