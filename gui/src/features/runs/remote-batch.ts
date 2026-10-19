import { reconcileRemoteRun, remoteRunPermissions } from './run-model';

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
    const status = result.status as { state?: string; exit_code?: number } | undefined;
    options.change(
      dir,
      permissions.terminal ? (status?.exit_code === 0 ? 'succeeded' : 'failed') : 'submitted',
      result,
    );
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
