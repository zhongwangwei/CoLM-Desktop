/**
 * Time ticks in model time (UTC), 24-hour and ISO dates: uPlot's default "12pm 1/1/08" reads
 * wrong in Chinese and is ambiguous across locales.
 */
export function timeTicks(
  _u: unknown,
  splits: number[],
  _axis?: number,
  _space?: number,
  increment = 0,
) {
  const pad = (n: number) => String(n).padStart(2, '0');
  let lastDay = '';
  return splits.map((seconds) => {
    const d = new Date(seconds * 1000);
    const day = `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`;
    if (increment >= 86400) return increment >= 86400 * 28 ? day.slice(0, 7) : day;
    const time = `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}`;
    const label = day === lastDay ? time : `${time}\n${day}`;
    lastDay = day;
    return label;
  });
}
