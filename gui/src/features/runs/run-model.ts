/** How often an unfinished server job or study is checked automatically. */
export const REMOTE_CHECK_MS = 10_000;
import { cropEnabled } from '../cases/case-model';
import type { LastRun, RunState } from '../../lib/types';
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
        status?: { log_tail?: unknown; phase?: unknown };
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
  const phase = remote?.status?.phase;
  const buildStage =
    typeof phase === 'string' &&
    [
      'preparing the engine',
      'installing Rust/Cargo',
      'installing CMake',
      'building the Rust engine',
    ].includes(phase)
      ? phase
      : undefined;
  return {
    runId: record.run_id ?? record.operation_id ?? '',
    casePath,
    status: record.status,
    lines: typeof logTail === 'string' && logTail ? logTail.split('\n') : record.logs,
    stage: remote
      ? (remoteStage ?? buildStage)
      : typeof progress?.stage === 'string'
        ? progress.stage
        : undefined,
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

const ENDED = ['succeeded', 'completed', 'failed', 'cancelled', 'completed_with_failures'];
/** A last run the ledger has settled: finished, and nothing left to re-check on a server. */
export function endedRun(run: LastRun | undefined): run is LastRun {
  return (
    !!run && !run.needs_reconciliation && run.finished_ms != null && ENDED.includes(run.status)
  );
}
/** Run states for cases that ended before this session and have none yet. */
export function restoredRuns(
  last: Record<string, LastRun>,
  running: Record<string, RunState>,
): Record<string, RunState> {
  const restored: Record<string, RunState> = {};
  for (const [dir, run] of Object.entries(last)) {
    if (running[dir] || !endedRun(run)) continue;
    restored[dir] = {
      case: dir,
      location: run.location === 'local' ? 'local' : 'remote',
      run_id: run.operation_id,
      operation_id: run.operation_id,
      status: ['succeeded', 'completed'].includes(run.status) ? 'succeeded' : 'failed',
      running: false,
      logs: [],
      stages: {},
      ...(run.host ? { progress: { host: run.host } } : {}),
    };
  }
  return restored;
}

/** Cases whose last run the ledger leaves open: they are watched until their server says how it ended. */
export function openRuns(
  last: Record<string, LastRun>,
  running: Record<string, RunState>,
): Record<string, RunState> {
  const open: Record<string, RunState> = {};
  for (const [dir, run] of Object.entries(last)) {
    // `endedRun` narrows its argument, so read it as a plain boolean here.
    const ended: boolean = endedRun(run);
    if (running[dir] || ended || run.location === 'local') continue;
    open[dir] = {
      case: dir,
      location: 'remote',
      run_id: run.operation_id,
      operation_id: run.operation_id,
      status: 'unknown',
      running: true,
      logs: [],
      stages: {},
      ...(run.host ? { progress: { host: run.host } } : {}),
    };
  }
  return open;
}

/** One line for the result of checking server jobs: how many finished, failed, run, wait. */
export function remoteStatusSummary(results: unknown[], en = false): string {
  const counts = { finished: 0, failed: 0, running: 0, queued: 0, unknown: 0 };
  for (const result of results) {
    const status = ((result as { status?: { state?: string; exit_code?: number } } | null)
      ?.status ?? {}) as { state?: string; exit_code?: number };
    if (remoteRunPermissions(result).terminal)
      counts[status.exit_code === 0 ? 'finished' : 'failed']++;
    else if (status.state === 'running') counts.running++;
    else if (status.state === 'queued') counts.queued++;
    else counts.unknown++;
  }
  const parts = [
    counts.finished && (en ? `${counts.finished} finished` : `完成 ${counts.finished}`),
    counts.failed && (en ? `${counts.failed} failed` : `失败 ${counts.failed}`),
    counts.running && (en ? `${counts.running} running` : `运行中 ${counts.running}`),
    counts.queued && (en ? `${counts.queued} queued` : `排队中 ${counts.queued}`),
    counts.unknown && (en ? `${counts.unknown} unconfirmed` : `未能确认 ${counts.unknown}`),
  ].filter(Boolean);
  const head = en
    ? `Checked ${results.length} server job(s)`
    : `已核对 ${results.length} 个服务器作业`;
  return parts.length ? `${head}：${parts.join(' · ')}` : head;
}

const text = (value: unknown): string => (typeof value === 'string' ? value : '');
const asRecord = (value: unknown): Record<string, unknown> =>
  value && typeof value === 'object' ? (value as Record<string, unknown>) : {};

/** What a server job record says, in words: the facts a person wants, not the raw JSON. */
export function remoteJobFacts(
  value: unknown,
  en = false,
  now = Date.now(),
): { label: string; value: string }[] {
  const data = asRecord(value);
  const record = asRecord(data.record ?? value);
  const status = asRecord(data.status);
  const provenance = asRecord(record.provenance);
  const options = new Map(
    (Array.isArray(provenance.remote_options) ? provenance.remote_options : [])
      .filter((item): item is [string, string] => Array.isArray(item) && item.length === 2)
      .map(([flag, option]) => [String(flag), String(option)]),
  );
  const t = (zh: string, english: string) => (en ? english : zh);
  const facts: { label: string; value: string }[] = [];
  const add = (label: string, content: unknown) => {
    const shown = typeof content === 'number' ? String(content) : text(content);
    if (shown) facts.push({ label, value: shown });
  };

  const permissions = remoteRunPermissions(value);
  const state = text(status.state);
  const detail = text(status.detail);
  let stateText = t('未能确认（请核对服务器状态）', 'Unconfirmed (check the server status)');
  if (permissions.terminal)
    stateText =
      status.exit_code === 0
        ? t('已结束，成功（退出码 0）', 'Finished, succeeded (exit code 0)')
        : t(
            `已结束，失败（退出码 ${status.exit_code}）`,
            `Finished, failed (exit code ${status.exit_code})`,
          );
  else if (state === 'running') stateText = t('运行中', 'Running');
  else if (state === 'queued') stateText = t('排队中，等待调度器分配资源', 'Queued for resources');
  else if (state === 'lost')
    stateText = t('作业已不在调度器里，也没有留下退出码', 'The job is gone without an exit code');
  add(t('状态', 'State'), stateText);
  if (detail && state !== 'finished') add(t('调度器状态', 'Scheduler state'), detail);

  add(t('服务器', 'Server'), record.host);
  const scheduler = text(record.scheduler);
  const schedulerId = text(record.scheduler_id);
  if (scheduler)
    add(
      t('调度系统', 'Scheduler'),
      `${scheduler === 'bare' ? t('直接运行', 'Direct execution') : scheduler.toUpperCase()}${
        schedulerId ? ` · ${t('作业号', 'job')} ${schedulerId}` : ''
      }`,
    );
  const requested = [
    options.get('--partition') && `${t('队列', 'queue')} ${options.get('--partition')}`,
    options.get('--cpus') && `${options.get('--cpus')} CPU`,
    options.get('--threads') && `${options.get('--threads')} ${t('线程', 'threads')}`,
    Number(provenance.ranks) > 1 && `${provenance.ranks} ${t('个 MPI 进程', 'MPI ranks')}`,
  ].filter(Boolean);
  if (requested.length) add(t('申请资源', 'Requested'), requested.join(' · '));

  const submitted = Number(record.submitted_at);
  if (Number.isFinite(submitted) && submitted > 0) {
    const when = new Date(submitted * 1000);
    const minutes = Math.max(0, Math.round((now - when.getTime()) / 60000));
    add(
      t('提交时间', 'Submitted'),
      `${when.toLocaleString(en ? 'en-US' : 'zh-CN')}${
        minutes < 1 ? '' : minutes < 60 ? t(`（${minutes} 分钟前）`, ` (${minutes} min ago)`) : ''
      }`,
    );
  }
  const revision = asRecord(provenance.model_revision);
  if (text(revision.id))
    add(
      t('模型版本', 'Model'),
      `${revision.id === 'official' ? t('官方版', 'Official') : revision.id} r${revision.revision ?? ''}`.trim(),
    );
  const engine = text(provenance.engine);
  if (engine)
    add(
      t('计算引擎', 'Engine'),
      engine === 'rust' ? 'Rust' : engine === 'fortran' ? 'Fortran' : engine,
    );
  add(t('服务器上的算例目录', 'Server case folder'), record.remote_case);

  const lines = text(status.log_tail)
    .split('\n')
    .map((line) => line.trim())
    .filter(Boolean);
  const progress = [...lines].reverse().find((line) => /^TIMESTEP\s*=/.test(line));
  const match = progress?.match(/TIMESTEP\s*=\s*(\d+)\s*\|\s*DATE\s*=\s*(\d{4}-\d{2}-\d{2})/);
  if (match)
    add(
      t('模型计算进度', 'Model progress'),
      t(`第 ${match[1]} 步（${match[2]}）`, `step ${match[1]} (${match[2]})`),
    );
  const phase = text(status.phase);
  if (phase && !permissions.terminal) add(t('当前阶段', 'Stage'), phase);
  const last = lines.filter((line) => !/^TIMESTEP\s*=/.test(line)).slice(-3);
  if (last.length) add(t('最近输出', 'Recent output'), last.join(' / '));
  return facts;
}
