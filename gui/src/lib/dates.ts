export function validDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || Number(value.slice(0, 4)) < 1) return false;
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}
/**
 * Seconds since the epoch for a UTC `YYYY-MM-DD[THH:MM[:SS]]` value, or NaN.
 * WebKit's JavaScriptCore rejects `Date.parse('2020-01-01Z')` while V8 accepts it,
 * so build the full ISO form explicitly instead of appending `Z`.
 */
export function utcSeconds(value: string): number {
  const match = /^(\d{4}-\d{2}-\d{2})(?:T(\d{2}:\d{2})(:\d{2})?)?$/.exec(value.trim());
  if (!match) return NaN;
  const iso = `${match[1]}T${match[2] ?? '00:00'}${match[3] ?? ':00'}`;
  const ms = Date.parse(`${iso}Z`);
  return Number.isFinite(ms) && new Date(ms).toISOString().slice(0, 19) === iso ? ms / 1000 : NaN;
}
