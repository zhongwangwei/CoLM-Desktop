import { chartVariables, variableTitle, type CatalogVariable } from './variable-labels';
import type { ResultMetric } from './series';
export function overviewMetric<T extends ResultMetric>(rows: T[], variable: string): T | undefined {
  return rows.find((row) => !row.site && (row.model_var === variable || row.name === variable));
}
export function overviewRunStatus(run: Record<string, unknown> | undefined, en: boolean): string {
  if (!run) return en ? 'No selected run' : '未选择运行';
  if (run.needs_reconciliation) return en ? 'Awaiting reconciliation' : '待核对运行状态';
  const status = run.status;
  if (status === 'succeeded' || status === 'completed') return en ? 'Run completed' : '运行完成';
  if (status === 'failed') return en ? 'Run failed' : '运行失败';
  if (status === 'running') return en ? 'Running' : '运行中';
  return en ? 'Recorded result · status unconfirmed' : '已记录结果 · 状态未确认';
}

export function overviewVariable(variables: { name: string; kind: string }[]): string {
  // 单点是 (time) 序列，区域模拟是 (time, lat, lon) 格点：两种都能画。
  const series = variables.filter(
    (variable) =>
      (variable.kind === 'series' || variable.kind === 'grid') && variable.name !== 'time',
  );
  return series.find((variable) => variable.name === 'f_lfevpa')?.name || series[0]?.name || '';
}
export function overviewRunLabel(
  run: {
    operation_id?: string;
    run_id?: string;
    id?: string;
    run_case?: string;
    version?: { id: string; revision: number };
  },
  en: boolean,
): string {
  const id = run.operation_id || run.run_id || run.id || run.run_case?.split(/[\\/]/).at(-1) || '—';
  return `${en ? 'Run' : '运行'} ${id.slice(0, 12)} · ${run.version ? `${run.version.id}@${run.version.revision}` : en ? 'Unknown version' : '版本身份未知'}`;
}

export function overviewVariableLabel(
  variable: string,
  en: boolean,
  longName?: string | null,
): string {
  return variableTitle(variable, en, longName);
}
export function overviewFinding(
  metric: ResultMetric | undefined,
  variable: string,
  en: boolean,
): string {
  if (!metric || metric.n <= 0)
    return en
      ? 'Simulation only; no valid observation evaluation yet.'
      : '目前仅展示模拟，尚无有效观测评估。';
  if (typeof metric.bias !== 'number' || !Number.isFinite(metric.bias))
    return en
      ? 'Valid pairs are available; mean bias is unavailable.'
      : '已有有效观测配对，平均偏差暂不可用。';
  const label =
    typeof metric[en ? 'label_en' : 'label_zh'] === 'string'
      ? String(metric[en ? 'label_en' : 'label_zh'])
      : overviewVariableLabel(variable, en);
  const units = typeof metric.units === 'string' ? metric.units : '';
  return en
    ? `${label}: mean simulation minus observation is ${metric.bias.toFixed(1)} ${units}.`
    : `${label}的模拟减观测平均偏差为 ${metric.bias.toFixed(1)} ${units}。`;
}
export function overviewRunLocation(
  run: { provenance?: unknown } | undefined,
  en: boolean,
): string {
  const data = run?.provenance;
  if (!data || typeof data !== 'object') return '';
  const p = data as { engine?: unknown; location?: { kind?: unknown; host?: unknown } | null };
  const engine = p.engine === 'rust' ? 'Rust' : p.engine === 'fortran' ? 'Fortran' : '';
  const location =
    p.location?.kind === 'local'
      ? en
        ? 'Local'
        : '本机'
      : p.location?.kind === 'remote'
        ? typeof p.location.host === 'string'
          ? p.location.host
          : en
            ? 'Server'
            : '服务器'
        : '';
  return [location, engine].filter(Boolean).join(' · ');
}

/** Composite evaluations are selectable as whole quantities, never as one of their components. */
export function overviewVariables(
  catalog: CatalogVariable[],
  rows: ResultMetric[],
  en: boolean,
): CatalogVariable[] {
  const choices = chartVariables(catalog);
  for (const row of rows) {
    if (
      row.site ||
      typeof row.model_var !== 'string' ||
      !/[+-]/.test(row.model_var) ||
      choices.some((choice) => choice.name === row.name)
    )
      continue;
    choices.push({
      name: row.name,
      kind: 'paired',
      dimensions: [],
      units: typeof row.units === 'string' ? row.units : null,
      long_name: String(row[en ? 'label_en' : 'label_zh'] || row.name),
    });
  }
  return choices;
}

export function metricWarningText(value: unknown, en: boolean): string {
  const text = String(value);
  if (!en) return text;
  const translated = text
    .replace(
      /^读不到 (.+)（(.+)）；history 区间起点按名义边界推断$/,
      'Cannot read $1 ($2); history interval starts use nominal boundaries',
    )
    .replace(
      /^DEF_HIST_FREQ 无法识别（(.+)）；输出频率改由 history 标签判断$/,
      'Unrecognized DEF_HIST_FREQ ($1); output frequency is inferred from history labels',
    )
    .replace(
      /^DEF_simulation_time%timestep = (.+) 不能当作 history 输出步长；TIMESTEP 频率将用标签间隔推断$/,
      'DEF_simulation_time%timestep = $1 is not a valid history step; TIMESTEP frequency is inferred from label spacing',
    )
    .replace(
      /^case.nml 的模拟起点不可用（(.+)）；history 区间起点按名义边界推断$/,
      'The simulation start in case.nml is unavailable ($1); history interval starts use nominal boundaries',
    )
    .replace(
      /^case.nml 的预热截止时刻不可用（(.+)）；history 区间起点按模拟起点处理$/,
      'The spin-up end in case.nml is unavailable ($1); history interval starts use the simulation start',
    )
    .replace(
      /^case.nml 声明 DEF_HIST_FREQ = (.+)，history 标签显示是 (.+)；按 history 为准$/,
      'case.nml declares DEF_HIST_FREQ = $1, but history labels indicate $2; history takes precedence',
    )
    .replace(
      /^case.nml 的模拟起点晚于首条 history 记录（配置被改过？）；区间起点改按名义边界推断$/,
      'The simulation start in case.nml is later than the first history record (configuration changed?); interval starts use nominal boundaries',
    )
    .replace(
      /^KGE 不可信：观测均值 (.+) 接近零$/,
      'KGE is unreliable: observed mean $1 is near zero',
    )
    .replace(
      /^KGE 不可信：模型与观测的均值反号$/,
      'KGE is unreliable: model and observed means have opposite signs',
    )
    .replace(
      /^(.+) 个输出区间因观测覆盖不足未参与配对（阈值 (.+)%）$/,
      '$1 output intervals were excluded for insufficient observation coverage (threshold $2%)',
    )
    .replace(
      /^观测覆盖率均值 (.+)% 偏低（<(.+)%），整段均值代表的是部分时段$/,
      'Mean observation coverage is low ($1%, below $2%); interval means represent only part of each period',
    );
  return translated === text && /[\u3400-\u9fff]/.test(text)
    ? `Backend warning (Chinese original): ${translated}`
    : translated;
}
