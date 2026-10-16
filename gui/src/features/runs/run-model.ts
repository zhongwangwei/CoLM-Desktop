import { cropEnabled } from '../cases/case-model';
import type { RunState } from '../../lib/types';
export interface RunView {
  runId: string;
  casePath: string;
  status: string;
  lines: string[];
  stage?: string;
  step?: number;
  total?: number;
}
export interface KernelChoice {
  dir: string;
  preset: string;
  macros: string[];
}
export function compatibleKernel<T extends KernelChoice>(
  kernels: T[],
  profile: {
    grid?: string | null;
    classification?: 'IGBP' | 'USGS';
    subgrid?: string;
    crop?: boolean;
  },
  physics: Record<string, string> = {},
): T | undefined {
  const grid =
    (
      { latlon: 'GRIDBASED', unstructured: 'UNSTRUCTURED', catchment: 'CATCHMENT' } as Record<
        string,
        string
      >
    )[profile.grid ?? ''] ?? 'SinglePoint';
  const land =
    (profile.classification ?? (profile.subgrid === 'USGS' ? 'USGS' : 'IGBP')) === 'USGS'
      ? 'LULC_USGS'
      : 'LULC_IGBP';
  const crop = profile.crop ?? cropEnabled(physics);
  const matching = kernels.filter(
    (k) => k.macros.includes(grid) && k.macros.includes(land) && k.macros.includes('CROP') === crop,
  );
  return (
    matching.find(
      (k) => k.preset === (crop ? 'crop' : land === 'LULC_USGS' ? 'usgs' : 'default'),
    ) ?? matching[0]
  );
}
export function remoteSubmissionBlocked(record: unknown): boolean {
  if (record == null) return false;
  return !remoteRunPermissions(record).terminal;
}

export function remoteRunPermissions(value: unknown) {
  const data = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const record =
    data.record && typeof data.record === 'object'
      ? (data.record as Record<string, unknown>)
      : data;
  const status =
    data.status && typeof data.status === 'object' ? (data.status as Record<string, unknown>) : {};
  const pending = Boolean(data.pending || data.needs_reconciliation);
  const confirmed =
    !pending &&
    data.record !== null &&
    typeof record.host === 'string' &&
    typeof record.job === 'string';
  const terminal = confirmed && status.state === 'finished' && Number.isInteger(status.exit_code);
  return {
    pending,
    terminal,
    cancel: confirmed && ['queued', 'running'].includes(String(status.state)),
    fetch: terminal,
  };
}

export function reconcileRemoteRun(previous: unknown, result: Record<string, unknown>) {
  const confirmed = Boolean(result.record && typeof result.record === 'object');
  return {
    ...(previous && typeof previous === 'object' ? previous : {}),
    ...result,
    ...(confirmed ? { pending: result.pending ?? false, needs_reconciliation: false } : {}),
  };
}

export function matchesRemoteRun(run: RunState | undefined, result: unknown): boolean {
  if (!run || run.location !== 'remote') return false;
  const record = (value: unknown): Record<string, unknown> => {
    if (!value || typeof value !== 'object') return {};
    const data = value as Record<string, unknown>;
    return data.record && typeof data.record === 'object'
      ? (data.record as Record<string, unknown>)
      : data;
  };
  const previous = record(run.progress?.remote);
  const next = record(result);
  const operation =
    next.operation_id ?? (next.provenance as { operation_id?: string } | undefined)?.operation_id;
  const expectedOperation =
    run.operation_id ??
    previous.operation_id ??
    (previous.provenance as { operation_id?: string } | undefined)?.operation_id;
  if (operation && expectedOperation && operation !== expectedOperation) return false;
  if (typeof previous.job !== 'string') {
    return (
      typeof run.operation_id === 'string' &&
      operation === run.operation_id &&
      typeof next.job === 'string' &&
      typeof next.host === 'string' &&
      next.host === run.progress?.host
    );
  }
  return (
    typeof previous.host === 'string' &&
    typeof previous.job === 'string' &&
    previous.host === next.host &&
    previous.job === next.job
  );
}

export function restoreRunView(record: RunState | undefined, casePath: string): RunView | null {
  if (!record) return null;
  const remote = record.progress?.remote as
    | {
        progress?: Record<string, unknown> | null;
        stages?: unknown;
        status?: { log_tail?: unknown };
      }
    | undefined;
  const progress = remote ? remote.progress : record.progress;
  const remoteStage = Array.isArray(remote?.stages)
    ? remote.stages
        .filter(
          (stage): stage is [string, string] =>
            Array.isArray(stage) &&
            stage.length === 2 &&
            typeof stage[0] === 'string' &&
            !!stage[0] &&
            typeof stage[1] === 'string',
        )
        .at(-1)?.[0]
    : undefined;
  const logTail = remote?.status?.log_tail;
  return {
    runId: record.run_id ?? record.operation_id ?? '',
    casePath,
    status: record.status,
    lines: typeof logTail === 'string' && logTail ? logTail.split('\n') : record.logs,
    stage: remote ? remoteStage : typeof progress?.stage === 'string' ? progress.stage : undefined,
    step: typeof progress?.step === 'number' ? progress.step : undefined,
    total: typeof progress?.total_steps === 'number' ? progress.total_steps : undefined,
  };
}
export function settleRuns(
  records: Record<string, RunState>,
  targets: string[],
  runId: string,
  success: boolean,
): Record<string, RunState> {
  return {
    ...records,
    ...Object.fromEntries(
      targets
        .filter((dir) => records[dir]?.run_id === runId && records[dir].running)
        .map((dir) => [
          dir,
          { ...records[dir], running: false, status: success ? 'succeeded' : 'failed' },
        ]),
    ),
  };
}
