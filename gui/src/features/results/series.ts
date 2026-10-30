import { bilingual } from '../../lib/bilingual';
import { download as exportText } from '../../lib/utils';
import { timeBasisOf, type TimeBasis } from './time-basis';
import { modelClockIso } from './time-ticks';
export interface SeriesData {
  time: number[];
  vars: Record<string, (number | null)[]>;
  source_n: number;
  n: number;
  downsampled: boolean;
  /**
   * 抽样后仍然缺测的时段，Unix 秒的半开区间 [start, end)。
   *
   * 2400 点抽样会把连续的缺测记录整段跳过，抽样结果里一点痕迹都没有，图上就会用直线
   * 连过缺测段。Rust 在 series 响应里把缺测区间单独给出来，前端据此在图上断线。
   * 旧版 sidecar 的响应没有这个字段，缺失时按“没有缺测”处理（不能凭空造断点）。
   */
  gaps?: Record<string, [number, number][]>;
  /**
   * 时间数值的基准。旧版 sidecar 的响应没有这个字段，此时按模型钟处理，
   * 导出不会把它们冒称成 UTC。
   */
  time_basis?: TimeBasis;
}
export interface EnergyDiagnostic {
  schema: 1;
  expression: string;
  closure_assessed: false;
  /** 区域模拟：序列是逐格残差的纬度加权区域平均。 */
  area_mean?: boolean;
  full_stats: {
    basis: 'all_records_in_selected_window';
    mean_kind: 'arithmetic_mean_of_common_finite_records';
    status: 'computed' | 'unavailable';
    source_n: number;
    n: number;
    missing: number;
    mean: number | null;
    units: 'W/m²';
  };
  series: SeriesData;
  window: {
    from: number | null;
    to: number | null;
    end_inclusive: true;
    time_basis: 'model_history_clock';
  };
  limitation: string;
}
export function fullEnergyStats(report: EnergyDiagnostic) {
  const summary = report.full_stats;
  if (
    report.schema !== 1 ||
    report.closure_assessed !== false ||
    summary?.basis !== 'all_records_in_selected_window' ||
    summary.mean_kind !== 'arithmetic_mean_of_common_finite_records' ||
    summary.units !== 'W/m²'
  )
    throw new Error('Full-record energy statistics are unavailable');
  if (
    ![summary.source_n, summary.n, summary.missing].every(
      (value) => Number.isSafeInteger(value) && value >= 0,
    ) ||
    summary.n + summary.missing !== summary.source_n ||
    report.series?.source_n !== summary.source_n
  )
    throw new Error('Inconsistent full-record energy support');
  if (
    summary.status === 'computed'
      ? summary.n === 0 || typeof summary.mean !== 'number' || !Number.isFinite(summary.mean)
      : summary.status !== 'unavailable' || summary.n !== 0 || summary.mean !== null
  )
    throw new Error('Invalid full-record energy statistic');
  return summary;
}
export async function boundedMap<T, R>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<R>,
): Promise<R[]> {
  const output = new Array<R>(items.length);
  let index = 0;
  await Promise.all(
    Array.from({ length: Math.min(Math.max(1, limit), items.length) }, async () => {
      while (index < items.length) {
        const at = index++;
        output[at] = await fn(items[at]);
      }
    }),
  );
  return output;
}
/**
 * boundedMap 的“不抛”版本：多站比较时单站失败不能丢掉其它站的成功结果。
 * 输出与 items 按下标对齐；每项独立捕获错误。shouldStop 只拦还没领取的项，
 * 已经在飞的请求照常等它结束（取消不是 abort，不能假装它没发生）。
 */
export async function boundedMapSettled<T, R>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<R>,
  shouldStop?: () => boolean,
): Promise<Array<{ value?: R; error?: unknown; cancelled?: boolean }>> {
  const output: Array<{ value?: R; error?: unknown; cancelled?: boolean }> = new Array(
    items.length,
  );
  let index = 0;
  await Promise.all(
    Array.from({ length: Math.min(Math.max(1, limit), items.length) }, async () => {
      while (index < items.length) {
        if (shouldStop?.()) break;
        const at = index++;
        try {
          output[at] = { value: await fn(items[at]) };
        } catch (error) {
          // 失败只落在这一项上，由调用方决定怎么展示/重试。
          output[at] = { error };
        }
      }
    }),
  );
  // 被取消的项从未进入过 fn，不能算成功也不能算失败。
  for (let at = 0; at < items.length; at++)
    if (output[at] === undefined) output[at] = { cancelled: true };
  return output;
}
export function download(name: string, text: string, type = 'application/json') {
  return exportText(name, text, type);
}

export interface ResultMetric {
  name: string;
  site?: string;
  n: number;
  rmse: number | null;
  mae: number | null;
  bias: number | null;
  r2: number | null;
  nse: number | null;
  kge: number | null;
  correlation?: number | null;
  alpha?: number | null;
  beta?: number | null;
  [key: string]: unknown;
}
export interface ResultReport {
  generated_at: string;
  source: string;
  provenance: unknown;
  catalog: unknown;
  evaluation: unknown;
  metrics: ResultMetric[];
  diagnostic: EnergyDiagnostic | null;
  baseline: unknown;
  baselineNote: string;
  conditions: Record<string, unknown>;
  notice: string;
}
export type ReportFormat = 'json' | 'csv' | 'markdown' | 'html' | 'pdf';
const metricColumns = [
  'site',
  'name',
  'n',
  'rmse',
  'mae',
  'bias',
  'r2',
  'correlation',
  'nse',
  'kge',
  'alpha',
  'beta',
] as const;
function csvCell(value: unknown) {
  const text =
    typeof value === 'number'
      ? Number.isFinite(value)
        ? String(value)
        : ''
      : value == null
        ? ''
        : String(value);
  const safe =
    typeof value === 'string' && (/^[\t\r\n]/.test(text) || /^[\s]*[=+\-@]/.test(text))
      ? `'${text}`
      : text;
  return `"${safe.replaceAll('"', '""')}"`;
}
function csvRows(rows: Record<string, unknown>[], columns: readonly string[]) {
  return (
    [
      columns.map(csvCell).join(','),
      ...rows.map((row) => columns.map((column) => csvCell(row[column])).join(',')),
    ].join('\r\n') + '\r\n'
  );
}
export function fullSeriesCsv(data: SeriesData, variable: string) {
  if (
    data.downsampled !== false ||
    !Number.isSafeInteger(data.source_n) ||
    data.n !== data.source_n ||
    data.n !== data.time.length ||
    !Array.isArray(data.vars[variable]) ||
    data.vars[variable].length !== data.n
  )
    throw new Error(
      'The full-series export requires every selected-window record; sampled data cannot be exported as complete',
    );
  // 导出必须如实：只有 utc 基准才写 time_utc/Z；地方标准时写钟面 model_time，
  // 偏移已记录时才额外换算一列真实 UTC；模型钟永远没有 time_utc。
  const basis = timeBasisOf(data);
  const offset =
    basis.kind === 'local_standard_time'
      ? basis.utc_offset_seconds
      : basis.kind === 'utc'
        ? 0
        : null;
  const rows = data.time.map((time, index) => {
    if (!Number.isFinite(time) || !Number.isFinite(new Date(time * 1000).getTime()))
      throw new Error('Invalid full-series record time');
    const value = data.vars[variable][index];
    if (value !== null && (typeof value !== 'number' || !Number.isFinite(value)))
      throw new Error('Invalid full-series record value');
    const row: Record<string, unknown> = {
      unix_seconds: time,
      variable,
      value,
      time_basis: basis.kind,
    };
    // 模型钟面（无 Z）：它只是钟面读数，不是 UTC 瞬间。
    if (basis.kind !== 'utc') row.model_time = modelClockIso(time);
    if (offset !== null) row.time_utc = new Date((time - offset) * 1000).toISOString();
    return row;
  });
  const columns =
    basis.kind === 'utc'
      ? (['time_utc', 'unix_seconds', 'variable', 'value', 'time_basis'] as const)
      : offset !== null
        ? (['model_time', 'time_utc', 'unix_seconds', 'variable', 'value', 'time_basis'] as const)
        : (['model_time', 'unix_seconds', 'variable', 'value', 'time_basis'] as const);
  return csvRows(rows, columns);
}
function html(value: unknown) {
  return String(value ?? '—')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;');
}
function markdownCell(value: unknown) {
  return html(value)
    .replaceAll('\\', '\\\\')
    .replaceAll('|', '\\|')
    .replaceAll('`', '\\`')
    .replaceAll('[', '\\[')
    .replaceAll(']', '\\]')
    .replaceAll('*', '\\*')
    .replaceAll('_', '\\_')
    .replace(/\r\n|[\r\n]/g, '<br>');
}
const title = (en: boolean) =>
  en ? 'CoLM Desktop Results Analysis Report' : 'CoLM Desktop 结果分析报告';
function reportDetails(data: ResultReport) {
  return {
    conditions: data.conditions,
    catalog: data.catalog,
    evaluation: data.evaluation,
    diagnostic: data.diagnostic,
    baseline: data.baseline,
    baselineNote: data.baselineNote,
    provenance: data.provenance,
    notice: data.notice,
  };
}
export function reportMarkdown(data: ResultReport, en: boolean) {
  const lines = [
    `# ${title(en)}`,
    '',
    `${en ? 'Generated' : '生成时间'}: ${markdownCell(data.generated_at)}`,
    `${en ? 'Run copy' : '运行副本'}: ${markdownCell(data.source)}`,
    '',
    `## ${en ? 'Evaluation metrics' : '评估指标'}`,
    '',
    `| ${metricColumns.join(' | ')} |`,
    `| ${metricColumns.map(() => '---').join(' | ')} |`,
  ];
  for (const row of data.metrics)
    lines.push(
      `| ${metricColumns.map((column) => markdownCell(column === 'site' ? row.site || data.source : row[column])).join(' | ')} |`,
    );
  lines.push(
    '',
    `## ${en ? 'Recorded conditions and evidence' : '评估条件与已记录证据'}`,
    '',
    '```json',
    JSON.stringify(reportDetails(data), null, 2).replaceAll('`', '\\u0060'),
    '```',
    '',
  );
  return lines.join('\n');
}
export function reportBodyHtml(data: ResultReport, en: boolean) {
  const rows = data.metrics
    .map(
      (row) =>
        `<tr>${metricColumns.map((column) => `<td>${html(column === 'site' ? row.site || data.source : row[column])}</td>`).join('')}</tr>`,
    )
    .join('');
  return `<h1>${title(en)}</h1><p>${en ? 'Generated' : '生成时间'}: ${html(data.generated_at)}</p><p>${en ? 'Run copy' : '运行副本'}: ${html(data.source)}</p><h2>${en ? 'Evaluation metrics' : '评估指标'}</h2><table><thead><tr>${metricColumns.map((column) => `<th>${html(column)}</th>`).join('')}</tr></thead><tbody>${rows}</tbody></table><h2>${en ? 'Recorded conditions and evidence' : '评估条件与已记录证据'}</h2><pre>${html(JSON.stringify(reportDetails(data), null, 2))}</pre>`;
}
export function reportFile(data: ResultReport, format: Exclude<ReportFormat, 'pdf'>, en: boolean) {
  if (format === 'json')
    return {
      filename: 'colm-results.json',
      mime_type: 'application/json',
      text: JSON.stringify(data, null, 2) + '\n',
    };
  if (format === 'csv')
    return {
      filename: 'colm-results.csv',
      mime_type: 'text/csv;charset=utf-8',
      text: csvRows(
        data.metrics.map((row) => ({ ...row, site: row.site || data.source })),
        metricColumns,
      ),
    };
  if (format === 'markdown')
    return {
      filename: 'colm-results.md',
      mime_type: 'text/markdown;charset=utf-8',
      text: reportMarkdown(data, en),
    };
  return {
    filename: 'colm-results.html',
    mime_type: 'text/html;charset=utf-8',
    text: `<!doctype html><html lang="${en ? 'en' : 'zh-CN'}"><head><meta charset="utf-8"><title>${title(en)}</title><style>body{font:14px system-ui;max-width:1100px;margin:40px auto;padding:0 20px;color:#17251d}table{border-collapse:collapse;width:100%}th,td{border:1px solid #bbb;padding:6px;text-align:left;overflow-wrap:anywhere}pre{white-space:pre-wrap;overflow-wrap:anywhere}</style></head><body>${reportBodyHtml(data, en)}</body></html>`,
  };
}
export async function printReport(
  data: ResultReport,
  en: boolean,
  nativePrint?: () => Promise<unknown>,
) {
  document.querySelector('#print-report')?.remove();
  const article = document.createElement('article');
  article.id = 'print-report';
  article.innerHTML = reportBodyHtml(data, en);
  document.body.appendChild(article);
  let timer: ReturnType<typeof setTimeout> | undefined;
  const cleanup = () => {
    article.remove();
    window.removeEventListener('afterprint', cleanup);
    if (timer !== undefined) clearTimeout(timer);
  };
  window.addEventListener('afterprint', cleanup, { once: true });
  try {
    await new Promise<void>((resolve) =>
      requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
    );
    if (nativePrint) await nativePrint();
    else if (typeof window.print === 'function') window.print();
    else throw new Error(bilingual('这个窗口不能打印', 'Printing is unavailable'));
    timer = setTimeout(cleanup, 300_000);
    return { status: 'printdialog_requested' as const, format: 'pdf' as const };
  } catch (error) {
    cleanup();
    throw error;
  }
}

export interface RecordedRun {
  operation_id?: string;
  run_case?: string;
  version?: { id: string; revision: number };
  provenance?: { ok?: boolean } | null;
}
export function parentRuns<T extends RecordedRun>(
  runs: T[],
  parent: { id: string; revision: number } | null,
): T[] {
  return parent
    ? runs.filter(
        (run) =>
          !!run.operation_id &&
          run.provenance?.ok === true &&
          run.version?.id === parent.id &&
          run.version.revision === parent.revision,
      )
    : [];
}
