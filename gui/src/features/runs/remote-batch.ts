import { reconcileRemoteRun, remoteRunPermissions } from './run-model';

type Remote = Record<string, unknown>;

// The window owns pending work; submitted jobs retain their durable backend records.
export async function runRemoteBatch(options: {
  cases: string[];
  workers: number;
  stopped: () => boolean;
  record: (dir: string) => Promise<Remote | null>;
  submit: (dir: string) => Promise<Remote>;
  status: (dir: string) => Promise<Remote>;
  change: (dir: string, state: string, remote?: Remote, error?: string) => void;
  wait?: () => Promise<void>;
}) {
  const active = new Map<string, Remote>();
  let next = 0;
  const wait = options.wait ?? (() => new Promise<void>((resolve) => setTimeout(resolve, 3000)));
  function checkIdentity(dir: string, previous: Remote, result: Remote) {
    const before = (previous.record ?? previous) as Remote;
    const after = (result.record ?? result) as Remote;
    const operation = (value: Remote) =>
      value.operation_id ?? (value.provenance as Remote | undefined)?.operation_id;
    const expected = operation(before);
    if (
      typeof before.job !== 'string' ||
      typeof before.host !== 'string' ||
      before.job !== after.job ||
      before.host !== after.host ||
      (typeof expected === 'string' && expected !== operation(after))
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
  try {
    while (next < options.cases.length || active.size) {
      if (options.stopped()) break;
      while (next < options.cases.length && active.size < options.workers && !options.stopped()) {
        const dir = options.cases[next++];
        let known: Remote | undefined;
        try {
          const record = await options.record(dir);
          if (record) {
            known = record;
            const status = await options.status(dir);
            checkIdentity(dir, record, status);
            known = reconcileRemoteRun(record, status);
            if (!remoteRunPermissions(known).terminal)
              throw new Error(`${dir}: reconcile the existing server job before resubmitting`);
          }
          if (options.stopped()) {
            options.change(dir, 'cancelled');
            break;
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
          throw error;
        }
      }
      if (!active.size || options.stopped()) break;
      await wait();
      if (options.stopped()) break;
      for (const [dir, previous] of active) {
        if (options.stopped()) break;
        try {
          const result = await options.status(dir);
          if (options.stopped()) break;
          checkIdentity(dir, previous, result);
          const merged = reconcileRemoteRun(previous, result);
          active.set(dir, merged);
          publish(dir, merged);
        } catch (error) {
          if (options.stopped()) break;
          options.change(dir, 'unknown', active.get(dir), String(error));
          throw error;
        }
      }
    }
  } finally {
    for (const dir of options.cases.slice(next)) options.change(dir, 'cancelled');
  }
}
