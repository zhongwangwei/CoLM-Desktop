import test from 'node:test';
import assert from 'node:assert/strict';
import { myJobsText, serverLoadSummary, type ServerLoad } from './server-load';

const land02: ServerLoad = {
  hostname: 'land02',
  cpus: 40,
  load: [3.9, 3.94, 4.21],
  memory_total_gb: 94.3,
  memory_available_gb: 76.2,
  scheduler: 'slurm',
  partitions: [
    { name: 'first', nodes: [5, 2, 0, 7], cpus: [296, 376, 0, 672] },
    { name: 'second', nodes: [7, 0, 0, 7], cpus: [700, 388, 0, 1088] },
  ],
  hosts: null,
  queues: [{ name: 'first', pending: 1, running: 5 }],
  mine: [
    { partition: 'first', state: 'RUNNING', reason: 'None', count: 3 },
    { partition: 'first', state: 'PENDING', reason: 'QOSMaxJobsPerUserLimit', count: 2 },
    { partition: 'first', state: 'PENDING', reason: 'QOSMaxJobsPerUserLimit', count: 1 },
  ],
};

test('a status check says how busy the chosen partition is and why own jobs wait', () => {
  assert.equal(
    serverLoadSummary(land02, false, 'first'),
    'first 分区 CPU 占用 44%，空闲节点 2 个，空闲核 376；全体排队 1、运行 5；运行 3，排队 3（超过了每人可同时运行的作业数）',
  );
  assert.equal(
    serverLoadSummary({ ...land02, mine: [] }, true, 'first'),
    'first: 44% of CPUs in use, 2 idle nodes, 376 idle cores; all users: 1 queued, 5 running; You have no jobs on this server',
  );
});

test('LSF hosts and machines without a scheduler are summarised by slots or load', () => {
  const bd: ServerLoad = {
    ...land02,
    scheduler: 'lsf',
    partitions: [],
    queues: [{ name: 'normal', pending: 5137, running: 2257 }],
    hosts: { ok: 66, full: 47, closed: 7, total: 120, slots_used: 2280, slots_total: 5728 },
    mine: [{ partition: 'normal', state: 'PENDING', reason: '', count: 4 }],
  };
  assert.equal(serverLoadSummary(bd), '作业槽占用 40%，可用主机 66 台；排队 4');
  const bare = { ...land02, scheduler: null, partitions: [], queues: [], mine: [] };
  assert.equal(serverLoadSummary(bare), '负载 3.9 / 40 核；你在这台服务器上没有作业');
  assert.equal(
    myJobsText({ ...land02, mine: [{ partition: 'q', state: 'HELD', reason: '', count: 2 }] }),
    '其他状态 2',
  );
});
