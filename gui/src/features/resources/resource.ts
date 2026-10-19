import type { KindPaths, RunState, Server } from '../../lib/types';
import { remoteStudyReply, remoteStudyPermissions } from '../studies/remote-study';
export type { Server } from '../../lib/types';
export const newServer = (): Server => ({
  host: '',
  username: '',
  port: 0,
  auth: 'config',
  identity_file: '',
  root: '',
  maps: [],
  threads: 8,
  scheduler: 'auto',
  partition: '',
  account: '',
  walltime: '',
  cpus: 0,
  memory_gb: 0,
  nodes: 0,
  cores_per_node: 0,
  fetch_vars: '',
  env_script: '',
  directives: [],
  project_dir: '',
  rawdata_dir: '',
  runtime_dir: '',
  forcing_dir: '',
});
export function replaceServer(servers: Server[], original: string | null, next: Server) {
  if (!next.host.trim() || next.host.startsWith('-') || /\s/.test(next.host))
    throw new Error('Invalid SSH host');
  if (servers.some((s) => s.host === next.host && s.host !== original))
    throw new Error('Duplicate SSH host');
  if (original && !servers.some((s) => s.host === original))
    throw new Error('Server was removed; reload before saving');
  return original ? servers.map((s) => (s.host === original ? next : s)) : [...servers, next];
}

export function remoteCheckRequest(
  action: string,
  preset: string,
  casePath: string,
  packageName: string,
  kind: string,
) {
  const request: Record<string, string> = { action, preset: preset.trim() || 'default' };
  if (action === 'verify' || action === 'test') {
    if (!/^[A-Za-z0-9_-]+$/.test(packageName)) throw new Error('Invalid test package');
    request.kind = action === 'verify' ? kind : 'cargo';
    request.package = packageName;
  }
  if (action === 'verify') {
    if (!casePath.startsWith('/') || /[\0\r\n]/.test(casePath))
      throw new Error('Absolute remote case path required');
    request.case = casePath;
  }
  return request;
}

export function localRunEntries(running: Record<string, RunState>) {
  return Object.entries(running).filter(([, run]) => run.location === 'local');
}

export function studyResourcePermissions(value: unknown, result?: unknown) {
  const data = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const initial = remoteStudyReply({ record: data.remote });
  const reply = result == null ? null : remoteStudyReply(result);
  const candidate = reply?.record ?? initial.record;
  const record = candidate?.job ? candidate : null;
  const pending = !record && Boolean(data.remote_pending || data.remote);
  return { ...remoteStudyPermissions(record, pending || null, reply), pending };
}

/** A Slurm partition as the server probe reports it (`colm_remote::probe::Partition`). */
export type Partition = {
  name: string;
  default: boolean;
  exclusive: boolean;
  nodes: number;
  cores_min: number;
  cores_max: number;
  memory_gb_min: number;
  memory_gb_max: number;
  max_nodes: number | null;
  max_time: string | null;
  whole_node_memory: boolean;
};

function slurmDuration(text: string, en: boolean) {
  const match = /^(?:(\d+)-)?(\d+):(\d+)(?::(\d+))?$/.exec(text);
  if (!match) return text;
  const [days, hours, minutes] = [Number(match[1] ?? 0), Number(match[2]), Number(match[3])];
  if (days && !hours && !minutes) return en ? `${days} d` : `${days} 天`;
  if (!days && hours && !minutes) return en ? `${hours} h` : `${hours} 小时`;
  if (!days && !hours) return en ? `${minutes} min` : `${minutes} 分钟`;
  return text;
}

/** One line telling partitions apart: name, size of its nodes and what a single job may take. */
export function partitionLabel(partition: Partition, en = false): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  const range = (low: number, high: number) => (low === high ? `${low}` : `${low}–${high}`);
  return [
    `${partition.name}${partition.default ? t('（默认）', ' (default)') : ''}`,
    partition.nodes ? t(`${partition.nodes} 个节点`, `${partition.nodes} nodes`) : '',
    partition.nodes
      ? t(
          `每节点 ${range(partition.cores_min, partition.cores_max)} 核 / ${range(partition.memory_gb_min, partition.memory_gb_max)} GB`,
          `${range(partition.cores_min, partition.cores_max)} cores / ${range(partition.memory_gb_min, partition.memory_gb_max)} GB per node`,
        )
      : '',
    partition.max_nodes
      ? t(`单作业最多 ${partition.max_nodes} 节点`, `up to ${partition.max_nodes} nodes per job`)
      : '',
    partition.max_time ? slurmDuration(partition.max_time, en) : '',
    partition.exclusive
      ? t('整节点独占（站点批量会按节点打包）', 'whole nodes (station batches are packed per node)')
      : '',
  ]
    .filter(Boolean)
    .join(' · ');
}

export type ResourceCheck = {
  configuration: string;
  checked_at: string;
  probe?: {
    hostname?: string;
    schedulers?: string[];
    qsub_unverified?: boolean;
    configuration_error?: string;
    engine?: 'source' | 'prebuilt';
    toolchain_installable?: boolean;
    build_problems?: string[];
    problems?: string[];
    root_exists?: boolean;
    root_writable?: boolean;
    maps?: { local: string; remote: string; local_exists: boolean; remote_exists: boolean }[];
    partitions?: Partition[];
  };
  engines?: { engines?: { id?: string; ready?: boolean }[] };
  probe_error?: string;
  engine_error?: string;
};
type CheckState = { state: 'pending' | 'success' | 'warning' | 'error'; label: string };
export function resourceCheckSummary(server: Server, check?: ResourceCheck, en = false) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const pending: CheckState = { state: 'pending', label: t('尚未检查', 'Not checked') };
  if (!check || check.configuration !== JSON.stringify(server))
    return { ssh: pending, engine: pending, mapping: pending, directory: pending, checkedAt: null };
  const ssh: CheckState = check.probe_error
    ? { state: 'error', label: t('连接检查失败', 'Connection check failed') }
    : check.probe?.hostname
      ? { state: 'success', label: t('此次连接成功', 'Connected at last check') }
      : pending;
  const checkedAt = Number.isFinite(Date.parse(check.checked_at)) ? check.checked_at : null;
  if (check.probe_error || check.probe?.configuration_error) {
    const blocked: CheckState = {
      state: 'pending',
      label: check.probe_error
        ? t('等待连接成功', 'Waiting for connection')
        : t('待完善运行配置', 'Complete run configuration'),
    };
    return { ssh, engine: blocked, mapping: blocked, directory: blocked, checkedAt };
  }
  const inventory = check.engines?.engines;
  const ready = Array.isArray(inventory)
    ? inventory.filter((engine) => engine?.ready === true).length
    : 0;
  const engine: CheckState = check.engine_error
    ? { state: 'error', label: t('引擎检查失败', 'Engine check failed') }
    : inventory?.length === 0 &&
        (check.probe?.engine === 'source' || check.probe?.engine === 'prebuilt')
      ? {
          state: 'success',
          label:
            check.probe.engine === 'source'
              ? check.probe.toolchain_installable && check.probe.build_problems?.length
                ? t(
                    '首次运行自动安装工具并构建引擎',
                    'First run installs build tools and builds the engine',
                  )
                : t('首次运行自动构建引擎', 'Engine builds automatically on first run')
              : t('首次运行自动部署引擎', 'Engine deploys automatically on first run'),
        }
      : Array.isArray(inventory)
        ? {
            state: ready ? 'success' : 'warning',
            label: ready
              ? t(`${ready} 个可用快照`, `${ready} available snapshots`)
              : t('未找到可用快照', 'No available snapshot'),
          }
        : pending;
  const maps = Array.isArray(check.probe?.maps) ? check.probe.maps : [];
  const matched = server.maps.filter((map) =>
    maps.some(
      (checked) =>
        checked &&
        checked.local === map.local &&
        checked.remote === map.remote &&
        checked.local_exists === true &&
        checked.remote_exists === true,
    ),
  ).length;
  const mapping: CheckState = !check.probe
    ? pending
    : !server.maps.length
      ? {
          state: 'success',
          label: t('直接使用服务器路径，无需映射', 'Server paths used directly; no mapping needed'),
        }
      : matched === server.maps.length
        ? { state: 'success', label: t('对应路径存在', 'Mapped paths exist') }
        : {
            state: 'warning',
            label: t(
              `${server.maps.length - matched} 项待确认`,
              `${server.maps.length - matched} mappings need review`,
            ),
          };
  const directory: CheckState =
    check.probe?.root_writable === true
      ? {
          state: 'success',
          label:
            check.probe.root_exists === false
              ? t('工作目录可创建', 'Work directory can be created')
              : t('工作目录可写', 'Work directory is writable'),
        }
      : check.probe?.root_writable === false
        ? { state: 'error', label: t('工作目录不可写', 'Work directory is not writable') }
        : pending;
  return {
    ssh,
    engine,
    mapping,
    directory,
    checkedAt,
  };
}

export function resourceTaskInfo(value: unknown, result?: unknown) {
  const object = (value: unknown): Record<string, unknown> =>
    value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const base = object(value),
    raw = object(result),
    reply = object(raw.reconciled_status ?? raw);
  const initial = object(
    base.remote_pending ??
      base.remote ??
      base.record ??
      (base.pending && typeof base.pending === 'object' ? base.pending : base),
  );
  const identity = object(
    (typeof reply.pending === 'object' ? reply.pending : null) ?? reply.record ?? initial,
  );
  const status = object(reply.status ?? base.status);
  const pending = Boolean(
    reply.pending ||
    reply.needs_reconciliation ||
    (!reply.record && (base.remote_pending || base.pending || base.needs_reconciliation)),
  );
  const confirmed =
    typeof identity.host === 'string' &&
    !!identity.host &&
    typeof identity.job === 'string' &&
    !!identity.job;
  let state = 'unknown';
  if (
    confirmed &&
    !pending &&
    ['queued', 'running', 'lost', 'unknown'].includes(String(status.state))
  )
    state = String(status.state);
  if (confirmed && !pending && status.state === 'finished' && Number.isInteger(status.exit_code))
    state = status.exit_code === 0 ? 'succeeded' : 'failed';
  return {
    host: typeof identity.host === 'string' ? identity.host : null,
    job: typeof identity.job === 'string' ? identity.job : null,
    status: state,
    pending,
  };
}

const pathKindOrder = ['site', 'watershed', 'region', 'global'];
const pathFieldOrder = [
  'sitedir',
  'forcingdir',
  'shapefile',
  'catchment',
  'mesh',
  'spatial_forcingdir',
  'forcing_dataset',
  'forcing',
];
/** Remembered per-kind paths as rows, in wizard order (kind, then field). */
export function kindPathRows(paths: KindPaths | undefined) {
  const rank = (order: string[], value: string) =>
    order.includes(value) ? order.indexOf(value) : order.length;
  return Object.entries(paths ?? {})
    .flatMap(([kind, fields]) =>
      Object.entries(fields)
        .filter(([, value]) => value)
        .map(([key, value]) => ({ kind, key, value })),
    )
    .sort(
      (a, b) =>
        rank(pathKindOrder, a.kind) - rank(pathKindOrder, b.kind) ||
        rank(pathFieldOrder, a.key) - rank(pathFieldOrder, b.key),
    );
}
