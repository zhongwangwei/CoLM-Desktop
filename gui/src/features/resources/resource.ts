import { bilingual } from '../../lib/bilingual';
import type { DataDefaults, KindPaths, RunState, Server } from '../../lib/types';
import { remoteStudyReply, remoteStudyPermissions } from '../studies/remote-study';
export type { Server } from '../../lib/types';

/** 本机默认数据路径里可以编辑的四个目录；`kind_paths` 由后端即时维护，不参与未保存判断。 */
export const localPathKeys = ['project_dir', 'rawdata_dir', 'runtime_dir', 'forcing_dir'] as const;
export type LocalPathKey = (typeof localPathKeys)[number];
/** 后端缺省时本机默认为空；加载与刷新都先用它补齐再套用配置里的值。 */
export function localDefaults(local?: Partial<DataDefaults>): DataDefaults {
  return {
    project_dir: '',
    rawdata_dir: '',
    runtime_dir: '',
    forcing_dir: '',
    ...local,
  };
}
/**
 * 本机默认路径的未保存比较签名：保存时会 trim，所以比较也按 trim 后的值，只多敲空格不算改动；
 * 按类型记住的 `kind_paths` 是后端即时写盘的，不参与签名。
 */
export function localPathSignature(local: Partial<DataDefaults>): string {
  return JSON.stringify(localPathKeys.map((key) => (local[key] ?? '').trim()));
}
/** 放弃本机路径编辑时只还原这四个目录，保留已经即时保存的 `kind_paths` 等其它字段。 */
export function localPathValues(local: Partial<DataDefaults>): Pick<DataDefaults, LocalPathKey> {
  return {
    project_dir: local.project_dir ?? '',
    rawdata_dir: local.rawdata_dir ?? '',
    runtime_dir: local.runtime_dir ?? '',
    forcing_dir: local.forcing_dir ?? '',
  };
}
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
/**
 * Clusters this app has been run on, as starting points for a new server. Only what every user of
 * the cluster shares: scheduler, partitions, node size, limits and shared data folders. Never an
 * address, account, key or personal folder (the user's own work root contains their account name
 * on 天河 and 摆渡船, so it stays empty). Facts measured 2026-10-18/19.
 */
export type ServerPreset = {
  id: string;
  zh: string;
  en: string;
  notes: [string, string][];
  fields: Partial<Server>;
  /**
   * How a checked server is recognised as this cluster: its scheduler and the partitions or
   * queues it lists, never its address (login nodes, jump hosts and VPN addresses change).
   */
  match: { scheduler: string; queues: string[] };
};
export const SERVER_PRESETS: ServerPreset[] = [
  {
    id: 'tianhe-xingyi',
    zh: '天河·星逸（Slurm）',
    en: 'Tianhe Xingyi (Slurm)',
    fields: { scheduler: 'slurm', partition: 'deimos', walltime: '12:00:00', cores_per_node: 64 },
    match: { scheduler: 'slurm', queues: ['deimos', 'phobos'] },
    notes: [
      [
        '分区 deimos：每节点 64 核，整节点独占计费；多站点批量会自动按节点打包。另有 phobos（88 核）与 debug（最长 30 分钟）。',
        'Partition deimos: 64 cores per node, billed per whole node; station batches are packed per node. Also phobos (88 cores) and debug (30 minutes at most).',
      ],
      [
        '没有外网：编引擎所需的依赖、Rust 与 CMake 由本机下载后经 ssh 送过去。',
        'No internet: build dependencies, Rust and CMake are downloaded here and sent over SSH.',
      ],
      [
        '计算节点没有 C 链接器：引擎先在登录节点编好再提交作业。',
        'Compute nodes have no C linker: the engine is built on the login node before jobs are submitted.',
      ],
      [
        '工作目录请填自己 HDD_POOL 下的一个文件夹。',
        'Use a folder under your own HDD_POOL as the work root.',
      ],
    ],
  },
  {
    id: 'baiduchuan',
    zh: '摆渡船（LSF）',
    en: 'Baiduchuan (LSF)',
    fields: { scheduler: 'lsf', partition: 'normal', cores_per_node: 48 },
    match: { scheduler: 'lsf', queues: ['normal', 'gpu'] },
    notes: [
      [
        '队列 normal，每节点 48 核；作业按申请的核数分配，不占整个节点。',
        'Queue normal, 48 cores per node; jobs get the cores they ask for, not whole nodes.',
      ],
      [
        '只有登录节点能上网，计算节点不能；系统自带的 CMake 太旧，首次编引擎时会在用户目录装新版。',
        'Only the login node has internet; the system CMake is too old, so a newer one is installed in your home folder on the first build.',
      ],
    ],
  },
  {
    id: 'land',
    zh: 'land 集群 land02（Slurm）',
    en: 'land cluster, land02 (Slurm)',
    fields: {
      scheduler: 'slurm',
      partition: 'first',
      cores_per_node: 48,
      rawdata_dir: '/shr03/CoLMrawdata',
      runtime_dir: '/shr03/CoLMruntime',
      forcing_dir: '/shr03/CoLM_Forcing/JRA3Q',
    },
    match: { scheduler: 'slurm', queues: ['first', 'second', 'largemem'] },
    notes: [
      [
        '分区 first：7 个节点，每节点 48 核 / 320 GB，单作业最多 3 节点；second：64–80 核 / 512 GB–1 TB，单作业 1 节点；largemem：48 核 / 1.5 TB。',
        'Partition first: 7 nodes of 48 cores / 320 GB, up to 3 nodes per job; second: 64–80 cores / 512 GB–1 TB, one node per job; largemem: 48 cores / 1.5 TB.',
      ],
      [
        '不写内存的作业会占满整个节点的内存，应用提交时自动按核分配内存；second 常被别人的作业占满内存，排队时可改用 first。',
        'A job without a memory request takes the whole node’s memory; the app requests memory per core. second is often full, so try first when queued.',
      ],
      [
        '默认数据目录是课题组共享的 /shr03 下的 CoLMrawdata、CoLMruntime 与 CoLM_Forcing。',
        'Default data folders are the group’s shared CoLMrawdata, CoLMruntime and CoLM_Forcing under /shr03.',
      ],
    ],
  },
];

/** The preset a checked server looks like, by its scheduler and every partition the preset names. */
export function recognisedPreset(probe: ResourceCheck['probe'] | undefined): ServerPreset | null {
  const schedulers = probe?.schedulers ?? [];
  const queues = new Set([
    ...(probe?.queues ?? []),
    ...(probe?.partitions ?? []).map((partition) => partition.name),
  ]);
  return (
    SERVER_PRESETS.find(
      (preset) =>
        schedulers.includes(preset.match.scheduler) &&
        preset.match.queues.every((queue) => queues.has(queue)),
    ) ?? null
  );
}

const PRESET_DATA_KEYS = ['project_dir', 'rawdata_dir', 'runtime_dir', 'forcing_dir'] as const;

/**
 * A server with a preset applied: scheduling settings follow the preset; data folders are only
 * filled where still empty; address, account, authentication and work root are never touched.
 */
export function applyServerPreset(server: Server, preset: ServerPreset): Server {
  const next: Server = { ...server };
  for (const [key, value] of Object.entries(preset.fields) as [keyof Server, unknown][]) {
    if ((PRESET_DATA_KEYS as readonly string[]).includes(key) && next[key]) continue;
    (next as Record<string, unknown>)[key] = value;
  }
  return next;
}

export function replaceServer(servers: Server[], original: string | null, next: Server) {
  if (!next.host.trim() || next.host.startsWith('-') || /\s/.test(next.host))
    throw new Error(
      bilingual('SSH 主机名不对：只能用字母、数字、点、下划线和连字符', 'Invalid SSH host'),
    );
  if (servers.some((s) => s.host === next.host && s.host !== original))
    throw new Error(bilingual('这个 SSH 主机已经添加过了', 'Duplicate SSH host'));
  if (original && !servers.some((s) => s.host === original))
    throw new Error(
      bilingual('这台服务器已被删除，请刷新后再保存', 'Server was removed; reload before saving'),
    );
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
    if (!/^[A-Za-z0-9_-]+$/.test(packageName))
      throw new Error(bilingual('测试包名只能用字母、数字、- 和 _', 'Invalid test package'));
    request.kind = action === 'verify' ? kind : 'cargo';
    request.package = packageName;
  }
  if (action === 'verify') {
    if (!casePath.startsWith('/') || /[\0\r\n]/.test(casePath))
      throw new Error(
        bilingual('服务器上的算例路径要写绝对路径', 'Absolute remote case path required'),
      );
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
    queues?: string[];
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
