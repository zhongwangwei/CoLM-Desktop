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

test('unknown, missing exit code and poll failure stop submission without freeing active jobs', async () => {
  for (const failure of ['unknown', 'finished', 'network']) {
    const q = queue({
      status: async (dir) => {
        if (failure === 'network') throw new Error('Connection lost');
        return { record: record(dir), status: { state: failure } };
      },
    });
    await assert.rejects(q.run());
    assert.deepEqual(q.submitted, ['a', 'b']);
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'a')?.state, 'unknown');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'submitted');
    assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
  }
});

test('existing unreconciled records block duplicate submission for each selected case', async () => {
  const q = queue({
    record: async (dir) => (dir === 'b' ? record(dir) : null),
    status: async (dir) => ({ record: record(dir), status: { state: 'running' } }),
  });
  await assert.rejects(q.run(), /existing server job/);
  assert.deepEqual(q.submitted, ['a']);
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'b')?.state, 'unknown');
  assert.equal([...q.changes].reverse().find((entry) => entry.dir === 'c')?.state, 'cancelled');
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
