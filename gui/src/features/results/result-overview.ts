import { variableTitle } from './variable-labels';
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
