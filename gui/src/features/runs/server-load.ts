import { waitingReason } from './run-model';

/** What `remote_load` reports (`colm_remote::load::Load`). */
export type ServerLoad = {
  hostname: string;
  cpus: number | null;
  load: [number, number, number] | null;
  memory_total_gb: number | null;
  memory_available_gb: number | null;
  scheduler: string | null;
  /** Slurm partitions: allocated, idle, other, total. */
  partitions: {
    name: string;
    nodes: [number, number, number, number];
    cpus: [number, number, number, number];
  }[];
  hosts: {
    ok: number;
    full: number;
    closed: number;
    total: number;
    slots_used: number;
    slots_total: number;
  } | null;
  queues: { name: string; pending: number; running: number }[];
  mine: { partition: string; state: string; reason: string; count: number }[];
};

const percent = (used: number, total: number) => (total ? Math.round((used / total) * 100) : 0);

/** Own jobs as "running 3, queued 2 (over the per-user job limit)". */
export function myJobsText(load: ServerLoad, en = false): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  const count = (state: string) =>
    load.mine.filter((job) => job.state === state).reduce((sum, job) => sum + job.count, 0);
  const running = count('RUNNING'),
    pending = count('PENDING');
  const other = load.mine.reduce((sum, job) => sum + job.count, 0) - running - pending;
  if (!running && !pending && !other)
    return t('你在这台服务器上没有作业', 'You have no jobs on this server');
  // 同一种原因合并：90 个站点排队时只说一次"在等空闲的核与内存"。
  const reasons = [
    ...new Set(
      load.mine
        .filter((job) => job.state === 'PENDING' && job.reason && job.reason !== 'None')
        .map((job) => waitingReason(job.reason, en)),
    ),
  ];
  return [
    running && t(`运行 ${running}`, `${running} running`),
    pending &&
      t(`排队 ${pending}`, `${pending} queued`) +
        (reasons.length ? (en ? ` (${reasons.join('; ')})` : `（${reasons.join('；')}）`) : ''),
    other && t(`其他状态 ${other}`, `${other} in other states`),
  ]
    .filter(Boolean)
    .join(t('，', ', '));
}

/** One line for the "Check server status" answer: how busy the server is and where the own jobs stand. */
export function serverLoadSummary(load: ServerLoad, en = false, partition = ''): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  const parts: string[] = [];
  const chosen =
    load.partitions.find((entry) => entry.name === partition) ??
    (load.partitions.length === 1 ? load.partitions[0] : undefined);
  if (chosen) {
    const [allocated, idle, , total] = chosen.cpus;
    parts.push(
      t(
        `${chosen.name} 分区 CPU 占用 ${percent(allocated, total)}%，空闲节点 ${chosen.nodes[1]} 个，空闲核 ${idle}`,
        `${chosen.name}: ${percent(allocated, total)}% of CPUs in use, ${chosen.nodes[1]} idle nodes, ${idle} idle cores`,
      ),
    );
  } else if (load.hosts) {
    parts.push(
      t(
        `作业槽占用 ${percent(load.hosts.slots_used, load.hosts.slots_total)}%，可用主机 ${load.hosts.ok} 台`,
        `${percent(load.hosts.slots_used, load.hosts.slots_total)}% of job slots in use, ${load.hosts.ok} hosts available`,
      ),
    );
  } else if (load.load && load.cpus) {
    parts.push(
      t(
        `负载 ${load.load[0].toFixed(1)} / ${load.cpus} 核`,
        `load ${load.load[0].toFixed(1)} on ${load.cpus} cores`,
      ),
    );
  }
  const queue = load.queues.find((entry) => entry.name === chosen?.name);
  if (queue)
    parts.push(
      t(
        `全体排队 ${queue.pending}、运行 ${queue.running}`,
        `all users: ${queue.pending} queued, ${queue.running} running`,
      ),
    );
  parts.push(myJobsText(load, en));
  return parts.join(t('；', '; '));
}

export const loadPercent = percent;
