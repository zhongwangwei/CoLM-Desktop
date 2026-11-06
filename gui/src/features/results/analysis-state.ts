/**
 * 单站点评估与多站点比较的**结果快照**：把「算出来的结果」和「算它时用的条件」绑在同一个对象里。
 *
 * 页面上原来两处共用一个 `metrics` 数组，切页签、改条件、旧请求晚返回都会把不同口径的结果
 * 混进同一张表；导出的 CSV 还会拿「现在的条件」去描述「上一次计算的结果」。这里把契约钉死：
 *
 * - 请求条件是不可变的 `request` 快照，导出的条件列只读它，绝不读当前界面输入；
 * - 结果的 `key`（单站）/ `id`（多站）就是对应请求条件序列化后的值，`fresh()` 用它判断
 *   「这份结果是不是当前条件下的结果」；
 * - `exportGate()` 只在结果存在且条件仍然匹配时放行，原因分为「没有结果」与「条件已改」两种，
 *   页面据此给出中英文提示而不是导出一份条件对不上的文件。
 *
 * 纯函数、无 React 依赖：页面只负责发布快照与按 gate 渲染，规则集中在这里可单独测试。
 */

import type { TimeBasis } from './time-basis';

/** 单站点评估的一次请求条件（全部是计算时真正用到的字段）。 */
export interface SingleRequest {
  caseDir: string;
  versionId: string;
  revision: number;
  source: string;
  obs: string;
  from: string;
  to: string;
  spinup: string;
  corrected: boolean;
}

/** 多站点比较里的一行：一个运行副本 + 它的观测文件。顺序参与 key（决定 CSV 行序）。 */
export interface ComparisonEntryRequest {
  runCase: string;
  obs: string;
}

/** 多站点比较的一次请求条件。 */
export interface ComparisonRequest {
  entries: ComparisonEntryRequest[];
  from: string;
  to: string;
  spinup: string;
  corrected: boolean;
  /** 比较窗口的时间基准；导出 CSV 必须写清楚，否则 from/to 的时钟口径无从判断（GUI 审查 H-09）。 */
  time_basis?: TimeBasis;
}

/** 指标行里快照/导出真正要读的字段；后端新增字段允许缺省，旧行为不变。 */
export interface AnalysisRow {
  site?: string;
  comparison_obs?: string;
  /** 后端行里的出处对象形状随运行台账变化；导出只读 name/status/version/case_*，读不到就留空。 */
  provenance?: Record<string, unknown> | null;
  /**
   * 这一行指标实际使用的时间基准（`metrics` 每行都回传）。多站比较可能混用不同时钟的算例：
   * 导出必须逐行写，不能拿页面请求的基准冒充每一行（GUI 审查 H-09）。
   */
  time_basis?: TimeBasis;
  name: string;
  units?: string;
  n: number;
  rmse?: number | null;
  mae?: number | null;
  bias?: number | null;
  r2?: number | null;
  correlation?: number | null;
  nse?: number | null;
  kge?: number | null;
}

/** 评估变量可用性一行（结果页用来解释「为什么没有这个变量」）。 */
export interface EvaluationAvailability {
  name: string;
  available: boolean;
  reason?: string;
  missing_model?: string[];
  missing_observation?: string[];
  [key: string]: unknown;
}

/** 单站点评估结果：条件 + 行 + 变量可用性。 */
export interface SingleResult<R extends AnalysisRow = AnalysisRow> {
  scope: 'single';
  key: string;
  request: SingleRequest;
  rows: R[];
  evaluation: EvaluationAvailability[];
  completedAt: number;
}

/** 多站点比较里某个运行失败的原因（只记录，不阻断其他行）。 */
export interface ComparisonFailure {
  runCase: string;
  obs: string;
  error: string;
  /**
   * 条件已改/页面已切换时，并发池把**还没开始**的条目直接标成取消：它们没有真正的后端错误，
   * 也不该被当成「算过但失败」，重试时要重算。`error` 仍带一句可显示的说明。
   */
  cancelled?: boolean;
}

/** 多站点比较结果：条件 + 行 + 逐行失败。 */
export interface ComparisonResult<R extends AnalysisRow = AnalysisRow> {
  scope: 'multi';
  id: string;
  request: ComparisonRequest;
  rows: R[];
  failures: ComparisonFailure[];
  completedAt: number;
}

/** 两类快照的公共结构：`fresh`/`exportGate` 只依赖这些字段。 */
export type AnalysisSnapshot =
  | Pick<SingleResult<AnalysisRow>, 'scope' | 'key' | 'rows'>
  | Pick<ComparisonResult<AnalysisRow>, 'scope' | 'id' | 'rows'>;

export function singleKey(request: SingleRequest): string {
  return JSON.stringify(request);
}

/** 多站点比较的 key 对条目顺序敏感：重排会改变 CSV 行序，也算条件变化。 */
export function comparisonKey(request: ComparisonRequest): string {
  // time_basis 只决定结果怎么标注，不改变配对；它来自异步到达的 catalog/series，放进 key 会让
  // 已完成的比较在 catalog 到位瞬间被标成"条件已更改"并禁用导出（GUI 审查 F-H-11）。
  const conditions = { ...request };
  delete conditions.time_basis;
  return JSON.stringify(conditions);
}

/** 这份快照是不是当前条件下的结果。 */
export function fresh(result: AnalysisSnapshot | null | undefined, currentKey: string): boolean {
  if (!result) return false;
  return (result.scope === 'single' ? result.key : result.id) === currentKey;
}

export type ExportGate = { ok: true } | { ok: false; reason: 'none' | 'conditions-changed' };

/**
 * 导出闸门：没有结果（或结果没有可导出的行）→ `none`；结果存在但条件已改 → `conditions-changed`。
 * 页面必须按 gate 禁用导出并显示原因，不能拿旧快照按新条件导出。
 */
export function exportGate(
  result: AnalysisSnapshot | null | undefined,
  currentKey: string,
): ExportGate {
  if (!result || result.rows.length === 0) return { ok: false, reason: 'none' };
  return fresh(result, currentKey) ? { ok: true } : { ok: false, reason: 'conditions-changed' };
}

function csvCell(value: unknown): string {
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
  return /[",\n]/.test(safe) ? `"${safe.replaceAll('"', '""')}"` : safe;
}

/** 比较 CSV 的列头（测试与导出共用一份，避免表头与行错位）。 */
export const comparisonCsvHeader = [
  'site',
  'run_case',
  'status',
  'version',
  'observation',
  'from',
  'to',
  'spinup',
  'corrected',
  'time_basis',
  'variable',
  'units',
  'n',
  'rmse',
  'mae',
  'bias',
  'r2',
  'r',
  'nse',
  'kge',
] as const;

/**
 * CSV 里的时间基准文本：kind + 已知偏移；同一个 kind 下不同 offset 必须能区分。
 * 入参来自后端原文（旧服务器/未来形状可能不是 `TimeBasis`）：不认识的一律写 `unknown`，
 * 不能静默写空单元格（复核 C F4）。
 */
export function comparisonTimeBasisText(basis: unknown): string {
  if (!basis || typeof basis !== 'object') return 'unknown';
  const record = basis as Record<string, unknown>;
  if (record.kind === 'utc') return 'utc';
  if (record.kind === 'model_clock') return 'model_clock';
  if (record.kind === 'local_standard_time') {
    const offset = record.utc_offset_seconds;
    return typeof offset === 'number' && Number.isFinite(offset)
      ? `local_standard_time:offset=${offset}s`
      : 'local_standard_time:offset=unknown';
  }
  return 'unknown';
}

/**
 * 用比较**当时的条件快照**生成 CSV。条件列（from/to/spinup/corrected）只读 `result.request`；
 * 观测列优先取每行实际用的观测，其次按运行路径回查请求条目。
 * `time_basis` 逐行取该指标实际使用的基准（缺省才回退请求快照，最后回退 model_clock）：
 * 同一张比较表里可能混着 UTC 格点算例与地方时单点算例，不能用一个基准覆盖整列。
 */
export function comparisonCsv<R extends AnalysisRow>(result: ComparisonResult<R>): string {
  const { entries, from, to, spinup, corrected } = result.request;
  // 只要有一行带自己的基准，就不能再拿“当前打开算例”的基准去补缺行：
  // 多站比较可能混用不同时钟，补错比写 unknown 更糟（GUI 审查 H-09 复核）。
  const anyRowBasis = result.rows.some((metric) => !!metric.time_basis);
  const lines = [comparisonCsvHeader.join(',')];
  for (const metric of result.rows) {
    const site = metric.site ?? '';
    const run = metric.provenance ?? null;
    const runText = (key: string) => {
      const value = run?.[key];
      return typeof value === 'string' ? value : '';
    };
    const version = run?.version;
    const versionText =
      version && typeof version === 'object'
        ? `${String((version as Record<string, unknown>).id ?? '')}@${String((version as Record<string, unknown>).revision ?? '')}`
        : '';
    const entryObs = entries.find((entry) => entry.runCase === site)?.obs;
    const label = run
      ? `${runText('case_name') || runText('case_dir')} · ${site.split(/[\\/]/).at(-1) ?? ''}`
      : site;
    lines.push(
      [
        label,
        site,
        runText('status'),
        versionText,
        metric.comparison_obs ?? entryObs ?? '',
        from,
        to,
        spinup,
        corrected ? '1' : '0',
        metric.time_basis
          ? comparisonTimeBasisText(metric.time_basis)
          : anyRowBasis
            ? 'unknown'
            : result.request.time_basis
              ? comparisonTimeBasisText(result.request.time_basis)
              : 'model_clock',
        metric.name,
        metric.units,
        metric.n,
        metric.rmse,
        metric.mae,
        metric.bias,
        metric.r2,
        metric.correlation,
        metric.nse,
        metric.kge,
      ]
        .map(csvCell)
        .join(','),
    );
  }
  return lines.join('\n');
}
