/** How often an unfinished server job or study is checked automatically. */
export const REMOTE_CHECK_MS = 10_000;
import { cropEnabled } from '../cases/case-model';
import type { LastRun, RunState } from '../../lib/types';

/** Logical cores on this machine; WebKit exposes the count as `hardwareConcurrency`. */
export function localCoreCount(): number {
  const cores = typeof navigator === 'undefined' ? undefined : navigator.hardwareConcurrency;
  return Number.isInteger(cores) && (cores ?? 0) > 0 ? Math.min(256, cores as number) : 1;
}

/**
 * 单点算例是一个垂直柱、时间步串行推进：主模型没有可以拆给多线程的循环
 * （rayon 只用在空间格点、河网汇流与历史汇总）。所以单点每站固定 1 线程，
 * 并行度只能来自"同时跑几个站点"（GUI 审查 2026-10-22）。
 */
export const SITE_THREADS = 1;

/** 本机单点批量的默认并发：填满本机逻辑核，且不超过站点数。 */
export function siteBatchConcurrency(count: number, cores = localCoreCount()): number {
  const stations = Math.max(1, Math.floor(count));
  return Math.min(stations, Math.max(1, Math.floor(cores)));
}

/** 本机单个空间算例的默认线程数：一个算例可以用满本机逻辑核。 */
export function spatialThreads(cores = localCoreCount()): number {
  return Math.max(1, Math.min(256, Math.floor(cores)));
}

/** 本机运行的默认线程数：单点固定 1；空间单算例用满核；空间批量保持 1（由用户显式调）。 */
export function defaultLocalThreads(
  spatial: boolean,
  batch = false,
  cores = localCoreCount(),
): number {
  return spatial && !batch ? spatialThreads(cores) : SITE_THREADS;
}

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
  const finished = confirmed && status.state === 'finished' && Number.isInteger(status.exit_code);
  // No exit code, but the scheduler recorded the end (`colm_remote::job::ended`): a job cancelled
  // while queued never writes one (land02, 2026-10-19). Over, but there is nothing to fetch.
  const ended =
    confirmed && status.state === 'lost' && (data.ended === 'cancelled' || data.ended === 'failed');
  return {
    pending,
    terminal: finished || ended,
    cancel: confirmed && ['queued', 'running'].includes(String(status.state)),
    fetch: finished,
  };
}

/** How a terminal server job ended: by its exit code, or as the scheduler recorded it. */
export function remoteEndStatus(value: unknown): 'succeeded' | 'failed' | 'cancelled' {
  const data = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  if (data.ended === 'cancelled') return 'cancelled';
  const status =
    data.status && typeof data.status === 'object' ? (data.status as Record<string, unknown>) : {};
  return status.exit_code === 0 ? 'succeeded' : 'failed';
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
    // 台账记下的最终进度按本机运行的形状放进去：没有 `remote`，就不会被当成还要核对的服务器作业。
    const final = run.progress;
    const progress = {
      ...(run.host ? { host: run.host } : {}),
      ...(final?.stages?.length ? { stage: final.stages.at(-1)![0] } : {}),
      ...(typeof final?.step === 'number' ? { step: final.step } : {}),
      ...(typeof final?.total_steps === 'number' ? { total_steps: final.total_steps } : {}),
      ...(final?.date ? { date: final.date } : {}),
    };
    restored[dir] = {
      case: dir,
      location: run.location === 'local' ? 'local' : 'remote',
      run_id: run.operation_id,
      operation_id: run.operation_id,
      // 停止的就是停止，不算失败（以前重开项目后，取消的站点都显示成"运行失败"）。
      status: ['succeeded', 'completed'].includes(run.status)
        ? 'succeeded'
        : run.status === 'cancelled'
          ? 'cancelled'
          : 'failed',
      running: false,
      logs: [],
      stages: {},
      ...(Object.keys(progress).length ? { progress } : {}),
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
    // No operation id here: an imported record carries a placeholder (`legacy-…`) that the server's
    // job never has, and the first status check claims the job by its host instead.
    open[dir] = {
      case: dir,
      location: 'remote',
      run_id: run.operation_id,
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
export type RemoteActivityKind =
  'finished' | 'failed' | 'stopped' | 'building' | 'running' | 'queued' | 'unconfirmed';

/** Why Slurm keeps a job waiting (`squeue %r`), as a person would say it. */
export function waitingReason(reason: string, en: boolean): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  const known: Record<string, string> = {
    Priority: t('前面有优先级更高的作业', 'jobs with higher priority are ahead'),
    Resources: t('在等空闲的核与内存', 'waiting for free cores and memory'),
    Dependency: t('在等它依赖的作业', 'waiting for a job it depends on'),
    QOSMaxJobsPerUserLimit: t('超过了每人可同时运行的作业数', 'over the per-user job limit'),
    AssocMaxJobsLimit: t('超过了账户可同时运行的作业数', 'over the account job limit'),
    ReqNodeNotAvail: t('需要的节点暂时不可用', 'the requested nodes are unavailable'),
    PartitionTimeLimit: t('申请的时限超过了分区上限', 'the time limit exceeds the partition limit'),
  };
  return known[reason] ?? reason;
}

/**
 * What the server is doing with a job, in one sentence: compiling the engine, making surface data,
 * running the model (and how far), waiting in the queue (and why), or how it ended. "运行中 /
 * 未能确认" alone did not say whether the server was compiling or computing (land02, 2026-10-19).
 */
export function remoteActivity(
  value: unknown,
  en = false,
): { kind: RemoteActivityKind; text: string } {
  const t = (zh: string, english: string) => (en ? english : zh);
  const data = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const status =
    data.status && typeof data.status === 'object' ? (data.status as Record<string, unknown>) : {};
  const state = typeof status.state === 'string' ? status.state : '';
  const detail = typeof status.detail === 'string' ? status.detail : '';
  const phase = typeof status.phase === 'string' ? status.phase : '';
  if (remoteRunPermissions(value).terminal) {
    const ended = remoteEndStatus(value);
    if (ended === 'succeeded')
      return {
        kind: 'finished',
        text: t('已结束，成功（退出码 0）', 'Finished, succeeded (exit code 0)'),
      };
    if (ended === 'cancelled')
      return {
        kind: 'stopped',
        text: t('已停止（作业被取消）', 'Stopped (the job was cancelled)'),
      };
    return {
      kind: 'failed',
      text: Number.isInteger(status.exit_code)
        ? t(
            `已结束，失败（退出码 ${status.exit_code}）`,
            `Finished, failed (exit code ${status.exit_code})`,
          )
        : t(
            `已结束，失败（调度器：${detail.split('|')[0]}）`,
            `Failed (scheduler: ${detail.split('|')[0]})`,
          ),
    };
  }
  if (state === 'queued') {
    const reason = detail.split('|')[1]?.trim();
    return {
      kind: 'queued',
      text: reason
        ? t(`排队中：${waitingReason(reason, en)}`, `Queued: ${waitingReason(reason, en)}`)
        : t('排队中，等待调度器分配资源', 'Queued for resources'),
    };
  }
  if (state === 'running') {
    const stages: Record<string, [RemoteActivityKind, string, string]> = {
      'preparing the engine': ['building', '正在准备计算引擎', 'Preparing the engine'],
      'installing Rust/Cargo': ['building', '正在安装 Rust 编译器', 'Installing the Rust compiler'],
      'installing CMake': ['building', '正在安装 CMake', 'Installing CMake'],
      'building the Rust engine': [
        'building',
        '正在服务器上编译计算引擎',
        'Compiling the engine on the server',
      ],
      mksrfdata: ['running', '正在制作地表数据', 'Making surface data'],
      mkinidata: ['running', '正在制作初始条件', 'Making initial conditions'],
      colm: ['running', '正在进行主模型计算', 'Running the model'],
    };
    const [kind, zh, english] = stages[phase] ?? ['running', '运行中', 'Running'];
    const lines = typeof status.log_tail === 'string' ? status.log_tail.split('\n') : [];
    const date = [...lines]
      .reverse()
      .map((line) => line.match(/TIMESTEP\s*=\s*\d+\s*\|\s*DATE\s*=\s*(\d{4}-\d{2}-\d{2})/)?.[1])
      .find(Boolean);
    return {
      kind,
      text:
        date && kind === 'running'
          ? t(`${zh}，已算到 ${date}`, `${english}, at ${date}`)
          : t(zh, english),
    };
  }
  if (state === 'lost')
    return {
      kind: 'unconfirmed',
      text: t(
        '作业已不在调度器里，也没有留下退出码，需要核对',
        'The job is gone without an exit code; check it',
      ),
    };
  if (data.pending || data.needs_reconciliation)
    return {
      kind: 'unconfirmed',
      text: t('提交结果还没有确认', 'The submission is not confirmed yet'),
    };
  return { kind: 'unconfirmed', text: t('未能确认', 'Unconfirmed') };
}

export function remoteStatusSummary(results: unknown[], en = false): string {
  const groups = new Map<string, number>();
  for (const result of results) {
    const { kind, text } = remoteActivity(result, en);
    const reason = kind === 'queued' ? text.split(/[：:]/)[1]?.trim() : '';
    const names: Record<RemoteActivityKind, [string, string]> = {
      finished: ['已完成', 'finished'],
      failed: ['失败', 'failed'],
      stopped: ['已停止', 'stopped'],
      building: ['编译计算引擎', 'compiling the engine'],
      running: ['运行中', 'running'],
      queued: ['排队中', 'queued'],
      unconfirmed: ['未能确认', 'unconfirmed'],
    };
    const name = en ? names[kind][1] : names[kind][0];
    const key = reason ? `${name}（${reason}）` : name;
    groups.set(key, (groups.get(key) ?? 0) + 1);
  }
  const parts = [...groups].map(([name, count]) => (en ? `${count} ${name}` : `${name} ${count}`));
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
  const stateText = remoteActivity(value, en).text;
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

/** How the stations of a batch stand right now, from the run states the page already holds. */
export function runCountsText(runs: (RunState | undefined)[], en = false): string {
  const counts = { running: 0, waiting: 0, succeeded: 0, failed: 0, stopped: 0, unconfirmed: 0 };
  for (const run of runs) {
    if (!run) continue;
    if (run.status === 'succeeded') counts.succeeded++;
    else if (run.status === 'failed') counts.failed++;
    else if (run.status === 'cancelled') counts.stopped++;
    else if (run.status === 'unknown') counts.unconfirmed++;
    else if (['submitted', 'running', 'queued'].includes(run.status)) counts.running++;
    else counts.waiting++;
  }
  const parts = [
    counts.running && (en ? `${counts.running} running` : `运行中 ${counts.running}`),
    counts.waiting && (en ? `${counts.waiting} waiting` : `等待 ${counts.waiting}`),
    counts.succeeded && (en ? `${counts.succeeded} finished` : `完成 ${counts.succeeded}`),
    counts.failed && (en ? `${counts.failed} failed` : `失败 ${counts.failed}`),
    counts.stopped && (en ? `${counts.stopped} stopped` : `已停止 ${counts.stopped}`),
    counts.unconfirmed &&
      (en ? `${counts.unconfirmed} unconfirmed` : `待核对 ${counts.unconfirmed}`),
  ].filter(Boolean);
  return parts.join(' · ');
}

/**
 * The server whose login node may be building the engine for this submission: a server run builds
 * its engine there before submitting (several minutes after an update), while its stations show
 * 提交中. Empty when nothing is being submitted to a server.
 */
export function submittingHost(
  host: string,
  targets: string[],
  running: Record<string, Pick<RunState, 'status'> | undefined>,
): string {
  return host && targets.some((dir) => running[dir]?.status === 'submitting') ? host : '';
}

/**
 * `remote_status` 说这个算例本机既没有提交记录也没有提交意图（colm-cli 的 "has not been run remotely"）：
 * 提交根本没到服务器，可以放开重新提交（GUI 审查 G-06）。
 */
export function neverSubmitted(error: string) {
  return error.includes('has not been run remotely');
}
