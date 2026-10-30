/**
 * 跨路由可见的比较任务（UX ③）：页面仍然自己提交/等待/取报告，这里只镜像"正在跑什么、到哪一步"，
 * 让用户切到别的页面后还能看到进度并回到比较页。纯数据操作，localStorage 持久化在 provider 里做。
 */
export type BenchTaskState = 'running' | 'done' | 'failed' | 'interrupted';
/** 阶段文案按双语存：任务可能跨语言切换后仍要能显示。 */
export type BenchTaskPhase = [string, string];

export interface BenchTask {
  id: string;
  /** '' = 本机；否则为服务器名。 */
  host: string;
  /** OpenBench 比较项目名（comparisonName），用于返回比较页时对上。 */
  project: string;
  /** 用户看到的比较名。 */
  label: string;
  kind: 'run' | 'fetch';
  startedAt: number;
  phase: BenchTaskPhase;
  /** 远程作业号；本机任务为空。 */
  job?: string | null;
  state: BenchTaskState;
  finishedAt?: number;
  report?: string | null;
  error?: string | null;
}

/** 只保留最近若干条；完成的留在状态条上几分钟让用户看见。 */
export const BENCH_TASKS_KEY = 'colm.benchTasks';
const DONE_LINGER_MS = 5 * 60 * 1000;
const MAX_TASKS = 6;

export interface BenchTaskInput {
  host: string;
  project: string;
  label: string;
  kind: 'run' | 'fetch';
  phase: BenchTaskPhase;
  job?: string | null;
}

export function benchTaskId(now = Date.now(), random = Math.random): string {
  return `bench-${now.toString(36)}-${Math.floor(random() * 1e6).toString(36)}`;
}

/**
 * 开始一个任务。同一台机器、同一个比较项目只保留最新一次：旧 running 标成 interrupted，
 * 避免状态条出现两条同样项目的"正在跑"（重复提交是被页面锁挡住的，这里只是镜像去重）。
 */
export function beginBenchTask(
  tasks: BenchTask[],
  input: BenchTaskInput & { id?: string },
  now = Date.now(),
): { tasks: BenchTask[]; id: string } {
  const id = input.id ?? benchTaskId(now);
  const superseded = tasks.map((task) =>
    task.state === 'running' && task.host === input.host && task.project === input.project
      ? { ...task, state: 'interrupted' as const, finishedAt: now }
      : task,
  );
  const next: BenchTask = {
    id,
    host: input.host,
    project: input.project,
    label: input.label,
    kind: input.kind,
    startedAt: now,
    phase: input.phase,
    job: input.job ?? null,
    state: 'running',
    report: null,
    error: null,
  };
  return { tasks: [next, ...superseded].slice(0, MAX_TASKS), id };
}

export function updateBenchTask(
  tasks: BenchTask[],
  id: string,
  patch: Partial<Pick<BenchTask, 'phase' | 'job' | 'report' | 'error'>>,
): BenchTask[] {
  return tasks.map((task) => (task.id === id ? { ...task, ...patch } : task));
}

export function finishBenchTask(
  tasks: BenchTask[],
  id: string,
  outcome: {
    state: 'done' | 'failed' | 'interrupted';
    report?: string | null;
    error?: string | null;
  },
  now = Date.now(),
): BenchTask[] {
  return tasks.map((task) =>
    task.id === id
      ? {
          ...task,
          state: outcome.state,
          finishedAt: now,
          report: outcome.report ?? task.report ?? null,
          error: outcome.error ?? null,
        }
      : task,
  );
}

export function clearBenchTask(tasks: BenchTask[], id: string): BenchTask[] {
  return tasks.filter((task) => task.id !== id);
}

/** 应用重启后不可能还有本机运行：把上次留下的 running 标成 interrupted（远程作业可能还在服务器上跑，另行核对）。 */
export function markInterruptedBenchTasks(tasks: BenchTask[], now = Date.now()): BenchTask[] {
  return tasks.map((task) =>
    task.state === 'running' ? { ...task, state: 'interrupted' as const, finishedAt: now } : task,
  );
}

/** 状态条要显示的项：正在跑的一直显示；结束的留 DONE_LINGER_MS 后自动收起。 */
export function visibleBenchTasks(
  tasks: BenchTask[],
  now = Date.now(),
  linger = DONE_LINGER_MS,
): BenchTask[] {
  return tasks.filter(
    (task) => task.state === 'running' || task.finishedAt == null || now - task.finishedAt < linger,
  );
}

const STATES: BenchTaskState[] = ['running', 'done', 'failed', 'interrupted'];

/** 读 localStorage：坏记录逐条丢，不能因为一条烂数据让整条状态条消失。 */
export function parseBenchTasks(text: string): BenchTask[] {
  if (!text.trim()) return [];
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return [];
  }
  if (!Array.isArray(raw)) return [];
  const tasks: BenchTask[] = [];
  for (const item of raw) {
    if (!item || typeof item !== 'object') continue;
    const task = item as Record<string, unknown>;
    const phase = task.phase;
    if (
      typeof task.id !== 'string' ||
      typeof task.host !== 'string' ||
      typeof task.project !== 'string' ||
      typeof task.label !== 'string' ||
      typeof task.startedAt !== 'number' ||
      !Array.isArray(phase) ||
      phase.length !== 2 ||
      typeof phase[0] !== 'string' ||
      typeof phase[1] !== 'string' ||
      (task.kind !== 'run' && task.kind !== 'fetch') ||
      !STATES.includes(task.state as BenchTaskState)
    )
      continue;
    tasks.push({
      id: task.id,
      host: task.host,
      project: task.project,
      label: task.label,
      kind: task.kind,
      startedAt: task.startedAt,
      phase: [phase[0], phase[1]],
      job: typeof task.job === 'string' ? task.job : null,
      state: task.state as BenchTaskState,
      finishedAt: typeof task.finishedAt === 'number' ? task.finishedAt : undefined,
      report: typeof task.report === 'string' ? task.report : null,
      error: typeof task.error === 'string' ? task.error : null,
    });
  }
  return tasks.slice(0, MAX_TASKS);
}

export function serializeBenchTasks(tasks: BenchTask[]): string {
  return JSON.stringify(tasks.slice(0, MAX_TASKS));
}
