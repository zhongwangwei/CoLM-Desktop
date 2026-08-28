//! 结果工作台的纯数据层：范围、并发、缓存、降采样与导出。
//!
//! 这里不碰 DOM，也不依赖 Tauri。多站点结果的关键规则因此能在 Node 下直接测，
//! 而不是只能打开窗口后靠点击发现旧算例混进来或并发失控。

/** 结果只认本次创建的算例；`completedOnly` 再按 history 收窄。 */
export function resultCases(cases, createdCases, completedOnly = false) {
  return cases.filter(c => createdCases.has(c.dir) && (!completedOnly || c.has_history));
}

/** 小型 LRU。缓存的是有限的图表请求，不让 90 站点 × 几百变量常驻内存。 */
export class LruCache {
  constructor(limit = 12) {
    this.limit = Math.max(1, Math.trunc(limit) || 1);
    this.items = new Map();
  }

  get size() { return this.items.size; }
  has(key) { return this.items.has(key); }
  get(key) {
    if (!this.items.has(key)) return undefined;
    const value = this.items.get(key);
    this.items.delete(key);
    this.items.set(key, value);
    return value;
  }
  set(key, value) {
    this.items.delete(key);
    this.items.set(key, value);
    while (this.items.size > this.limit) this.items.delete(this.items.keys().next().value);
    return value;
  }
  delete(key) { return this.items.delete(key); }
  clear() { this.items.clear(); }
  deleteWhere(predicate) {
    for (const key of this.items.keys()) if (predicate(key)) this.items.delete(key);
  }
}

/**
 * 有上限的异步池。单项失败被包装进对应结果，不会短路整批。
 * `options.signal.aborted` 时不再启动新项，未启动项标成 cancelled。
 */
export async function boundedMap(items, limit, worker, options = {}) {
  const input = [...items];
  const out = new Array(input.length);
  const width = Math.max(1, Math.min(input.length || 1, Math.trunc(limit) || 1));
  let next = 0;
  let completed = 0;
  async function lane() {
    while (true) {
      if (options.signal?.aborted) return;
      const index = next++;
      if (index >= input.length) return;
      try {
        out[index] = { ok: true, value: await worker(input[index], index), index };
      } catch (error) {
        out[index] = { ok: false, error: String(error), index };
      }
      completed += 1;
      options.onProgress?.({ completed, total: input.length, result: out[index] });
    }
  }
  await Promise.all(Array.from({ length: width }, lane));
  for (let i = 0; i < out.length; i++) {
    out[i] ??= { ok: false, cancelled: true, error: 'cancelled', index: i };
  }
  return out;
}

function csvCell(value) {
  if (value == null) return '';
  const text = String(value);
  return /[",\r\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
}

export function rowsToCsv(rows, columns) {
  const keys = columns ?? [...new Set(rows.flatMap(row => Object.keys(row)))];
  return [
    keys.map(csvCell).join(','),
    ...rows.map(row => keys.map(key => csvCell(row[key])).join(',')),
  ].join('\n') + '\n';
}

export function finite(value) {
  if (value == null || value === '') return null;
  const number = Number(value);
  return Number.isFinite(number) ? number : null;
}

export function seriesStats(values) {
  const clean = values.map(finite).filter(value => value != null);
  if (!clean.length) return { n: 0, missing: values.length, min: null, max: null, mean: null, sd: null };
  const mean = clean.reduce((sum, value) => sum + value, 0) / clean.length;
  const variance = clean.length > 1
    ? clean.reduce((sum, value) => sum + (value - mean) ** 2, 0) / (clean.length - 1)
    : 0;
  return {
    n: clean.length,
    missing: values.length - clean.length,
    min: Math.min(...clean),
    max: Math.max(...clean),
    mean,
    sd: Math.sqrt(variance),
  };
}

/** 供报告和缓存使用的稳定键。 */
export function metricKey({
  caseDir, obs, spinup = 0, corrected = false, summaryOnly = false,
  pairVars = [], maxPoints = '',
}) {
  const variables = Array.isArray(pairVars) ? [...new Set(pairVars)].sort().join(',') : String(pairVars ?? '');
  return [caseDir, obs, Math.max(0, Number(spinup) || 0), corrected ? 1 : 0,
    summaryOnly ? 1 : 0, variables, maxPoints].join('\u001f');
}

export function seriesKey({ caseDir, variable, from = '', to = '', maxPoints = 2400 }) {
  return [caseDir, variable, from, to, maxPoints].join('\u001f');
}

export const METRIC_META = {
  rmse: { label: 'RMSE', better: 'low' },
  mae: { label: 'MAE', better: 'low' },
  bias: { label: 'Bias', better: 'zero' },
  r2: { label: 'R²', better: 'high' },
  correlation: { label: 'r', better: 'high' },
  nse: { label: 'NSE', better: 'high' },
  kge: { label: 'KGE', better: 'high' },
};

/** 排名条长度只负责相对展示；指标原值仍在表格里。 */
export function ranking(rows, key) {
  const valid = rows.filter(row => finite(row[key]) != null);
  if (!valid.length) return [];
  const meta = METRIC_META[key] ?? { better: 'high' };
  const score = row => meta.better === 'low' ? -Number(row[key])
    : meta.better === 'zero' ? -Math.abs(Number(row[key])) : Number(row[key]);
  const sorted = [...valid].sort((a, b) => score(b) - score(a));
  const values = sorted.map(score);
  const lo = Math.min(...values);
  const hi = Math.max(...values);
  return sorted.map(row => ({
    ...row,
    rankFraction: hi === lo ? 1 : 0.12 + 0.88 * (score(row) - lo) / (hi - lo),
  }));
}

const finiteNumber = value => typeof value === 'number' && Number.isFinite(value);
const absInfluence = row => finiteNumber(row?.value) ? Math.abs(row.value) : -1;

export function sortedImportanceRows(rows) {
  return [...(rows || [])].sort((a, b) => absInfluence(b) - absInfluence(a));
}

export function envelopeDiagnostics(data) {
  let minNEff = Infinity;
  let maxNEff = -Infinity;
  for (const value of data?.n_eff || []) {
    if (!Number.isFinite(value)) continue;
    if (value < minNEff) minNEff = value;
    if (value > maxNEff) maxNEff = value;
  }
  let unsupported = 0;
  for (const value of data?.stable || []) if (value === false) unsupported += 1;
  let widthSum = 0;
  let widthCount = 0;
  let maxWidth = -Infinity;
  for (let index = 0; index < (data?.p95 || []).length; index += 1) {
    const hi = data.p95[index];
    const lo = data?.p05?.[index];
    if (!finiteNumber(hi) || !finiteNumber(lo)) continue;
    const width = hi - lo;
    widthSum += width;
    widthCount += 1;
    if (width > maxWidth) maxWidth = width;
  }
  let diffSum = 0;
  let diffCount = 0;
  for (let index = 0; index < (data?.p50 || []).length; index += 1) {
    const median = data.p50[index];
    const baseline = data?.baseline?.[index];
    if (!finiteNumber(median) || !finiteNumber(baseline)) continue;
    diffSum += Math.abs(median - baseline);
    diffCount += 1;
  }
  return {
    minNEff: Number.isFinite(minNEff) ? minNEff : 0,
    maxNEff: Number.isFinite(maxNEff) ? maxNEff : 0,
    unsupported,
    meanWidth: widthCount ? widthSum / widthCount : null,
    maxWidth: Number.isFinite(maxWidth) ? maxWidth : null,
    meanMedianBaselineDiff: diffCount ? diffSum / diffCount : null,
  };
}
