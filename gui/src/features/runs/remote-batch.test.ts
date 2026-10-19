import test from 'node:test';
import assert from 'node:assert/strict';
import { runRemoteBatch } from './remote-batch';

const record = (dir: string) => ({ host: 'server', job: `job-${dir}` });
const finished = (dir: string, exit_code = 0) => ({
  record: record(dir),
  status: { state: 'finished', exit_code },
});

function queue(overrides: Partial<Parameters<typeof runRemoteBatch>[0]> = {}) {
  const submitted: string[] = [];
  const changes: { dir: string; state: string; error?: string }[] = [];
  const options: Parameters<typeof runRemoteBatch>[0] = {
    cases: ['a', 'b', 'c'],
    workers: 2,
    stopped: () => false,
    record: async () => null,
    submit: async (dir) => {
      submitted.push(dir);
      return record(dir);
    },
    status: async (dir) => finished(dir),
    change: (dir, state, _remote, error) => changes.push({ dir, state, error }),
    wait: async () => {},
    ...overrides,
  };
  return { submitted, changes, run: () => runRemoteBatch(options) };
}

test('remote concurrency counts active scheduler jobs, not successful submissions', async () => {
  let round = 0;
  const q = queue({
    wait: async () => {
      round++;
      assert.deepEqual(q.submitted, round <= 2 ? ['a', 'b'] : ['a', 'b', 'c']);
    },
    status: async (dir) =>
      round === 1
        ? { record: record(dir), status: { state: dir === 'a' ? 'running' : 'queued' } }
        : finished(dir),
  });
  await q.run();
  assert.equal(round, 3);
  assert.equal(q.changes.filter((entry) => entry.state === 'succeeded').length, 3);
});

test('unknown, missing exit code and poll failure stop submission without freeing their slots', async () => {
  for (const failure of ['unknown', 'finished', 'network']) {
    const q = queue({
      status: async (dir) => {
        if (failure === 'network') throw new Error('Connection lost');
        return { record: record(dir), status: { state: failure } };
      },
    });
    await assert.rejects(q.run());
    assert.deepEqual(q.submitted, ['a', 'b']);
    // b is still followed after a fails, and fails the same way.
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'unknown');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'unknown');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
  }
});

test('existing unreconciled records block duplicate submission for each selected case', async () => {
  let polls = 0;
  const q = queue({
    record: async (dir) => (dir === 'b' ? record(dir) : null),
    status: async (dir) =>
      dir === 'a' && ++polls > 2
        ? finished(dir)
        : { record: record(dir), status: { state: 'running' } },
  });
  await assert.rejects(q.run(), /previous server submission is not confirmed/);
  assert.deepEqual(q.submitted, ['a']);
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'unknown');
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
  // The job already running is followed to its end instead of being left 运行中.
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'succeeded');
});

test('a previous submission still being reconciled is reported as unconfirmed, not as a changed job', async () => {
  const q = queue({
    cases: ['a'],
    record: async (dir) => ({ ...record(dir), operation_id: 'op-1', pending: true }),
    status: async (dir) => ({
      record: null,
      pending: { host: 'server', job: `job-${dir}`, needs_reconciliation: true },
      status: { state: 'unknown' },
    }),
  });
  await assert.rejects(q.run(), /previous server submission is not confirmed/);
  assert.deepEqual(q.submitted, []);
});

test('terminal previous records allow a new run and terminal failures release one slot', async () => {
  const q = queue({
    workers: 1,
    record: async (dir) => record(dir),
    status: async (dir) => finished(dir, 130),
  });
  await q.run();
  assert.deepEqual(q.submitted, ['a', 'b', 'c']);
  assert.equal(q.changes.filter((entry) => entry.state === 'failed').length, 3);
});

test('uncertain submission and changed identity stop the queue and preserve reconciliation', async () => {
  for (const failure of ['submit', 'identity', 'unconfirmed']) {
    const q = queue({
      workers: 1,
      ...(failure === 'submit'
        ? {
            submit: async () => {
              throw new Error('Lost after submit');
            },
          }
        : {}),
      ...(failure === 'unconfirmed' ? { submit: async () => ({ pending: true }) } : {}),
      ...(failure === 'identity' ? { status: async () => finished('different-job') } : {}),
    });
    await assert.rejects(q.run());
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'unknown');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'cancelled');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
  }
});

test('closing or stopping the queue leaves submitted jobs intact and marks unsent stations stopped', async () => {
  let stopped = false;
  const q = queue({
    workers: 1,
    stopped: () => stopped,
    wait: async () => {
      stopped = true;
    },
  });
  await q.run();
  assert.deepEqual(q.submitted, ['a']);
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'submitted');
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'cancelled');
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
});

test('a changed host, job or operation in an existing terminal record cannot authorize resubmission', async () => {
  for (const mismatch of [{ host: 'other' }, { job: 'other' }, { operation_id: 'other' }]) {
    const q = queue({
      record: async (dir) => ({ ...record(dir), operation_id: 'original' }),
      status: async (dir) => ({
        ...finished(dir),
        record: { ...record(dir), operation_id: 'original', ...mismatch },
      }),
    });
    await assert.rejects(q.run(), /identity changed/);
    assert.deepEqual(q.submitted, []);
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'unknown');
  }
});

test('unreadable prior records stop the queue with unknown state and no submissions', async () => {
  const q = queue({
    record: async () => {
      throw new Error('Cannot read job record');
    },
  });
  await assert.rejects(q.run(), /Cannot read job record/);
  assert.deepEqual(q.submitted, []);
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'unknown');
});

test('a late polling response cannot replace a terminal cancellation or start another poll', async () => {
  let stopped = false;
  let resolvePoll!: (value: ReturnType<typeof finished>) => void;
  let entered!: () => void;
  const polling = new Promise<void>((resolve) => {
    entered = resolve;
  });
  const polled: string[] = [];
  const q = queue({
    stopped: () => stopped,
    status: async (dir) => {
      polled.push(dir);
      entered();
      return new Promise((resolve) => {
        resolvePoll = resolve;
      });
    },
  });
  const running = q.run();
  await polling;
  stopped = true;
  q.changes.push({ dir: 'a', state: 'cancelled' });
  resolvePoll(finished('a'));
  await running;
  assert.deepEqual(polled, ['a']);
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'cancelled');
  assert.deepEqual(q.submitted, ['a', 'b']);
});

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

test('stations are submitted and polled several at a time, not one after another', async () => {
  const cases = ['a', 'b', 'c', 'd', 'e', 'f'];
  let submitting = 0;
  let peakSubmitting = 0;
  let polling = 0;
  let peakPolling = 0;
  const done = new Set<string>();
  const changes: { dir: string; state: string }[] = [];
  await runRemoteBatch({
    cases,
    workers: 6,
    submitConcurrency: 3,
    pollConcurrency: 4,
    stopped: () => false,
    record: async () => null,
    submit: async (dir) => {
      peakSubmitting = Math.max(peakSubmitting, ++submitting);
      await tick();
      await tick();
      submitting--;
      return record(dir);
    },
    status: async (dir) => {
      peakPolling = Math.max(peakPolling, ++polling);
      await tick();
      polling--;
      done.add(dir);
      return finished(dir);
    },
    change: (dir, state) => changes.push({ dir, state }),
    wait: async () => {},
  });
  assert.equal(peakSubmitting, 3);
  assert.equal(peakPolling, 4);
  for (const dir of cases)
    assert.equal([...changes].reverse().find((entry) => entry.dir === dir)?.state, 'succeeded');
});

test('parallel submissions never exceed the workers, counting those still being submitted', async () => {
  let inFlight = 0;
  let peak = 0;
  const sent: string[] = [];
  const q = queue({
    cases: ['a', 'b', 'c', 'd', 'e'],
    workers: 2,
    submitConcurrency: 4,
    submit: async (dir) => {
      sent.push(dir);
      peak = Math.max(peak, ++inFlight);
      await tick();
      return record(dir);
    },
    status: async (dir) => {
      inFlight--;
      return finished(dir);
    },
  });
  await q.run();
  assert.equal(peak, 2);
  assert.deepEqual([...sent].sort(), ['a', 'b', 'c', 'd', 'e']);
});

test('jobs running when a submission fails are followed to the end, and nothing more is sent', async () => {
  // 天河 2026-10-19: five ran, the next five got an sbatch error; the five running ones then
  // stayed 运行中 because polling stopped with the queue.
  const polled: string[] = [];
  let polls = 0;
  const q = queue({
    cases: ['a', 'b', 'c', 'd'],
    workers: 2,
    submit: async (dir) => {
      if (dir === 'b') throw new Error('sbatch failed: Unexpected message received');
      return record(dir);
    },
    status: async (dir) => {
      polled.push(dir);
      return ++polls > 2 ? finished(dir) : { record: record(dir), status: { state: 'running' } };
    },
  });
  await assert.rejects(q.run(), /Unexpected message/);
  const last = (dir: string) => [...q.changes].reverse().find((entry) => entry.dir === dir)?.state;
  assert.equal(last('a'), 'succeeded');
  assert.equal(last('b'), 'unknown');
  assert.equal(last('c'), 'cancelled');
  assert.equal(last('d'), 'cancelled');
  assert.deepEqual(q.submitted, []);
  assert.ok(polled.every((dir) => dir === 'a'));
});

test('one failed submission among parallel ones is recorded, the rest of the queue is stopped', async () => {
  const sent: string[] = [];
  const q = queue({
    cases: ['a', 'b', 'c', 'd', 'e', 'f'],
    workers: 6,
    submitConcurrency: 3,
    submit: async (dir) => {
      sent.push(dir);
      await tick();
      if (dir === 'b') throw new Error('Lost after submit');
      return record(dir);
    },
  });
  await assert.rejects(q.run(), /Lost after submit/);
  const last = (dir: string) => [...q.changes].reverse().find((entry) => entry.dir === dir)?.state;
  assert.equal(last('b'), 'unknown');
  // A station started while another was still failing is not lost: whatever was sent is followed
  // to its end, and nothing is started once the failure is seen.
  assert.ok(sent.length >= 3 && sent.length <= 5, `${sent}`);
  for (const dir of sent.filter((name) => name !== 'b')) assert.equal(last(dir), 'succeeded', dir);
  for (const dir of ['a', 'b', 'c', 'd', 'e', 'f'].filter((name) => !sent.includes(name)))
    assert.equal(last(dir), 'cancelled', dir);
});
