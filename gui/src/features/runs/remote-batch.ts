import { reconcileRemoteRun, remoteEndStatus, remoteRunPermissions } from './run-model';

type Remote = Record<string, unknown>;

// The window owns pending work; submitted jobs retain their durable backend records.
//
// Submitting a station takes several seconds of SSH round trips, and so does asking for a job's
// state. Done one by one, ninety stations spend minutes only on submissions and a round of status
// checks takes longer than the jobs run. `submitConcurrency` submissions and `pollConcurrency`
// status checks run at once (each is its own SSH session; the two phases alternate, so the larger
// one must stay under sshd's usual ten sessions per connection); both default to one, the original
// strictly sequential behaviour.
export async function runRemoteBatch(options: {
  cases: string[];
  workers: number;
  stopped: () => boolean;
  record: (dir: string) => Promise<Remote | null>;
  submit: (dir: string) => Promise<Remote>;
  status: (dir: string) => Promise<Remote>;
  change: (dir: string, state: string, remote?: Remote, error?: string) => void;
  wait?: () => Promise<void>;
  submitConcurrency?: number;
  pollConcurrency?: number;
}) {
  const active = new Map<string, Remote>();
  const submitting = new Set<Promise<void>>();
  const submitLimit = Math.max(1, Math.floor(options.submitConcurrency ?? 1));
  const pollLimit = Math.max(1, Math.floor(options.pollConcurrency ?? 1));
  let next = 0;
  let failed = false;
  let failure: unknown;
  const fail = (error: unknown) => {
    if (!failed) {
      failed = true;
      failure = error;
    }
  };
  const wait = options.wait ?? (() => new Promise<void>((resolve) => setTimeout(resolve, 3000)));
  function checkIdentity(dir: string, previous: Remote, result: Remote) {
    const before = (previous.record ?? previous) as Remote;
    // A submission still being reconciled answers with `pending` (no record yet); its host and job
    // are what identify it, so it is not mistaken for a changed job (天河 2026-10-19, AU-How).
    const after = (result.record ?? result.pending ?? result) as Remote;
    const operation = (value: Remote) =>
      value.operation_id ?? (value.provenance as Remote | undefined)?.operation_id;
    const expected = operation(before);
    if (
      typeof before.job !== 'string' ||
      typeof before.host !== 'string' ||
      before.job !== after.job ||
      before.host !== after.host ||
      (typeof expected === 'string' &&
        operation(after) !== undefined &&
        expected !== operation(after))
    )
      throw new Error(`${dir}: server job identity changed`);
  }
  function publish(dir: string, result: Remote) {
    const permissions = remoteRunPermissions(result);
    options.change(dir, permissions.terminal ? remoteEndStatus(result) : 'submitted', result);
    if (permissions.terminal) active.delete(dir);
    else if (!permissions.cancel) throw new Error(`${dir}: server job state is unconfirmed`);
  }
  // One station: check what it already has, submit it, and take it into `active`. A failure is
  // recorded on the station and stops further submissions; jobs already running are still
  // followed to the end, or they would stay 运行中 forever (天河, 2026-10-19).
  async function submitOne(dir: string) {
    let known: Remote | undefined;
    try {
      const record = await options.record(dir);
      if (record) {
        known = record;
        const status = await options.status(dir);
        checkIdentity(dir, record, status);
        known = reconcileRemoteRun(record, status);
        if (!remoteRunPermissions(known).terminal)
          throw new Error(
            `${dir}: the previous server submission is not confirmed yet; check its status before resubmitting`,
          );
      }
      if (options.stopped()) {
        options.change(dir, 'cancelled');
        return;
      }
      options.change(dir, 'submitting');
      known = await options.submit(dir);
      const identity = (known.record ?? known) as Remote;
      if (
        typeof identity.job !== 'string' ||
        typeof identity.host !== 'string' ||
        known.pending ||
        known.needs_reconciliation
      )
        throw new Error(`${dir}: submission identity is unconfirmed`);
      active.set(dir, known);
      options.change(dir, 'submitted', known);
    } catch (error) {
      options.change(dir, 'unknown', known, String(error));
      fail(error);
    }
  }
  async function pollOne(dir: string, previous: Remote) {
    if (options.stopped()) return;
    try {
      const result = await options.status(dir);
      if (options.stopped()) return;
      checkIdentity(dir, previous, result);
      const merged = reconcileRemoteRun(previous, result);
      active.set(dir, merged);
      publish(dir, merged);
    } catch (error) {
      if (options.stopped()) return;
      options.change(dir, 'unknown', active.get(dir), String(error));
      // Not polled again: an identity that changed would fail the same way every few seconds.
      active.delete(dir);
      fail(error);
    }
  }
  try {
    while ((!failed && next < options.cases.length) || active.size || submitting.size) {
      if (options.stopped()) break;
      while (
        !failed &&
        next < options.cases.length &&
        active.size + submitting.size < options.workers &&
        submitting.size < submitLimit &&
        !options.stopped()
      ) {
        const task: Promise<void> = submitOne(options.cases[next++]).finally(() =>
          submitting.delete(task),
        );
        submitting.add(task);
      }
      if (options.stopped()) break;
      if (!active.size && !submitting.size) break;
      // Submissions under way come first: as soon as one finishes there may be room for the next.
      // Only when nothing can be started are the running jobs looked at (every few seconds).
      if (submitting.size) {
        await Promise.race(submitting);
        continue;
      }
      await wait();
      if (options.stopped()) break;
      const entries = [...active];
      for (let index = 0; index < entries.length; index += pollLimit) {
        if (options.stopped()) break;
        await Promise.all(
          entries.slice(index, index + pollLimit).map(([dir, previous]) => pollOne(dir, previous)),
        );
      }
    }
  } finally {
    // Submissions already under way finish (and are recorded) before the rest are given up.
    await Promise.allSettled([...submitting]);
    for (const dir of options.cases.slice(next)) options.change(dir, 'cancelled');
  }
  if (failed) throw failure;
}

/**
 * 取这一轮要核对的站点：从上一轮最后核对的那个站点之后接着取，到末尾绕回开头，最多 `limit` 个。
 *
 * 为什么必须轮转：自动核对原来每 10 秒固定看列表前 8 个。前 8 个一直运行（或查询一直失败）时，
 * 后面 12 个站点永远轮不到，服务器上早已结束的任务一直显示"运行中"（GUI-12）。每轮从上一轮
 * 停下的位置往后走，⌈待核对数 / limit⌉ 轮内就能覆盖全部任务，且每轮并发不超过 limit。
 *
 * `lastChecked` 已经不在列表里（那个站点结束了）时从头开始，不会平白空转一轮。
 */
export function nextCheckBatch(
  pending: string[],
  lastChecked: string | null,
  limit: number,
): { batch: string[]; next: string | null } {
  if (!pending.length) return { batch: [], next: null };
  const size = Math.max(1, Math.floor(limit));
  const after = lastChecked ? pending.indexOf(lastChecked) + 1 : 0;
  const from = after >= pending.length ? 0 : after;
  const batch = Array.from(
    { length: Math.min(size, pending.length) },
    (_, index) => pending[(from + index) % pending.length],
  );
  return { batch, next: batch.at(-1) ?? null };
}

/**
 * 核对一批服务器任务：最多 `limit` 个并发（SSH 会话数有上限），单个任务失败只记录下来，
 * 不影响其他任务继续核对。自动轮询和手动核对共用；手动核对用 `progress` 显示已核对数与总数。
 */
export async function checkInBatches<T>(options: {
  targets: string[];
  limit: number;
  check: (dir: string) => Promise<T | null | undefined>;
  progress?: (done: number, total: number) => void;
}): Promise<{ results: T[]; errors: { dir: string; error: unknown }[] }> {
  const results: T[] = [];
  const errors: { dir: string; error: unknown }[] = [];
  let done = 0;
  await eachLimited(options.targets, Math.max(1, Math.floor(options.limit)), async (dir) => {
    try {
      const result = await options.check(dir);
      if (result !== null && result !== undefined) results.push(result);
    } catch (error) {
      errors.push({ dir, error });
    }
    options.progress?.(++done, options.targets.length);
  });
  return { results, errors };
}

/**
 * Run `work` over `items`, at most `limit` at a time. Stopping a batch went station by station:
 * three SSH round trips each (identity check, cancel, status), so 40 queued land02 jobs took
 * minutes (2026-10-19). Five at a time stays under the ten sessions sshd allows, as submission does.
 */
export async function eachLimited<T>(
  items: T[],
  limit: number,
  work: (item: T) => Promise<void>,
): Promise<void> {
  let next = 0;
  const lanes = Array.from({ length: Math.max(1, Math.min(limit, items.length)) }, async () => {
    while (next < items.length) await work(items[next++]);
  });
  await Promise.all(lanes);
}
