import type { HistoryOperation } from '../../lib/types';
export type Filter = {
  query: string;
  kind: string;
  casePath: string;
  versionId: string;
  host: string;
  status: string;
  from: string;
  to: string;
};
export const emptyFilter: Filter = {
  query: '',
  kind: '',
  casePath: '',
  versionId: '',
  host: '',
  status: '',
  from: '',
  to: '',
};
export function historyFilter(filter: Filter, offset = 0, limit = 30) {
  const result: Record<string, string | number> = { offset, limit };
  for (const [key, value] of Object.entries(filter)) {
    if (!value) continue;
    if (key === 'from' || key === 'to') {
      const time = new Date(value).getTime();
      if (!Number.isFinite(time)) throw new Error('Invalid date');
      result[key] = time;
    } else result[key] = value;
  }
  if (typeof result.from === 'number' && typeof result.to === 'number' && result.from > result.to)
    throw new Error('开始时间晚于结束时间 / Start is after end');
  return result;
}
export function dayGroups(items: HistoryOperation[]) {
  const groups = new Map<string, HistoryOperation[]>();
  for (const item of items) {
    const day =
      item.time_ms == null
        ? '未知日期 / Unknown date'
        : new Date(item.time_ms).toLocaleDateString();
    groups.set(day, [...(groups.get(day) ?? []), item]);
  }
  return [...groups];
}
export function download(content: string, name: string, type: string) {
  const url = URL.createObjectURL(new Blob([content], { type }));
  const link = document.createElement('a');
  link.href = url;
  link.download = name;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export function canReplay(operation: HistoryOperation) {
  return (
    ['run', 'remote-run'].includes(operation.kind) &&
    !!operation.version_id &&
    !!operation.revision &&
    !operation.needs_reconciliation
  );
}
export function canCompare(operation: HistoryOperation) {
  return canReplay(operation) && operation.status === 'succeeded';
}
export type HistoryDetail = {
  operation: HistoryOperation;
  events: unknown[];
  related: HistoryOperation[];
};
function object(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}
function absolutePath(value: unknown): value is string {
  return (
    typeof value === 'string' &&
    !/[\0\r\n]/.test(value) &&
    /^(?:\/(?!\/)|[A-Za-z]:[\\/]|\\\\[^\\]+\\)/.test(value)
  );
}
export function recordedRunPath(detail: HistoryDetail): string | null {
  const records = [...detail.events].reverse().map((e) => object(object(e).detail));
  records.push(object(detail.operation.detail));
  for (const row of records) {
    const path =
      object(row.model_run).run_case ??
      object(object(row.provenance).model_run).run_case ??
      row.run_case;
    if (absolutePath(path)) return path;
  }
  return null;
}
export function historyPath(detail: HistoryDetail): string | null {
  const run = recordedRunPath(detail);
  if (run) return run;
  for (const row of [...detail.events]
    .reverse()
    .map((e) => object(object(e).detail))
    .concat(object(detail.operation.detail))) {
    for (const value of [row.output_path, row.output_dir]) if (absolutePath(value)) return value;
  }
  return absolutePath(detail.operation.case_path) ? detail.operation.case_path : null;
}
export type ComparisonInput = {
  obs: string;
  variable: string;
  from: string;
  to: string;
  spinup: string;
  corrected: boolean;
};
export function comparisonArgs(a: string, b: string, input: ComparisonInput) {
  if (!a || !b || a === b) throw new Error('Select two different recorded runs');
  if (!input.obs.trim() || !input.variable.trim())
    throw new Error('真实观测与变量必填 / Observations and variable required');
  const time = (value: string) => {
    if (!value) return null;
    if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(?::\d{2})?$/.test(value))
      throw new Error('Invalid UTC date');
    const seconds = Date.parse(`${value}Z`) / 1000;
    if (
      !Number.isSafeInteger(seconds) ||
      new Date(seconds * 1000).toISOString().slice(0, value.length) !== value
    )
      throw new Error('Invalid UTC date');
    return seconds;
  };
  const from = time(input.from),
    to = time(input.to);
  if (from != null && to != null && from >= to) throw new Error('Start must precede end');
  const spinup = Number(input.spinup);
  if (!input.spinup.trim() || !Number.isSafeInteger(spinup) || spinup < 0)
    throw new Error('Invalid skipped model record count');
  return {
    a,
    b,
    obs: input.obs.trim(),
    variable: input.variable.trim(),
    from,
    to,
    spinup,
    corrected: input.corrected,
  };
}
export function comparisonWinner(value: unknown): 'a' | 'b' | 'tie' | null {
  const result = object(value),
    evaluation = object(result.evaluation),
    rows = result.variables;
  if (
    result.same_design !== true ||
    !Array.isArray(rows) ||
    rows.length !== 1 ||
    !/^[a-f0-9]{64}$/.test(String(evaluation.obs_sha256 ?? ''))
  )
    return null;
  const row = object(rows[0]);
  return row.same_pairs === true &&
    row.better === result.better &&
    ['a', 'b', 'tie'].includes(String(result.better))
    ? (result.better as 'a' | 'b' | 'tie')
    : null;
}

export type HistoryRow = { zh: string; en: string; value: string | null };
export type HistorySection = { zh: string; en: string; rows: HistoryRow[] };
function text(value: unknown): string | null {
  if (value == null || value === '') return null;
  if (typeof value === 'string') return value.length > 1000 ? `${value.slice(0, 1000)}…` : value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  if (Array.isArray(value))
    return (
      value
        .slice(0, 24)
        .map((v) => text(v) ?? '—')
        .join(', ') + (value.length > 24 ? ` … (${value.length})` : '')
    );
  return null;
}
function historyEvidence(detail: HistoryDetail) {
  return Object.assign(
    {},
    ...detail.events.map((e) => object(object(e).detail)),
    detail.operation.detail,
  ) as Record<string, unknown>;
}
export function historySections(detail: HistoryDetail): HistorySection[] {
  const operation = detail.operation,
    evidence = historyEvidence(detail),
    provenance = object(evidence.provenance),
    summary = provenance.kind === 'provenance-reference' ? object(provenance.summary) : provenance,
    run = object(evidence.model_run ?? summary.model_run),
    code = object(run.code),
    remote = object(evidence.remote_record),
    status = object(evidence.status),
    identity = object(summary.full_input_identity),
    kernel = object(summary.kernel),
    command =
      operation.command ??
      [...detail.events]
        .reverse()
        .map((e) => object(e).command)
        .find((c) => c && typeof c === 'object');
  const row = (zh: string, en: string, value: unknown): HistoryRow => ({
      zh,
      en,
      value: text(value),
    }),
    time = (value: number | null) => (value == null ? null : new Date(value).toLocaleString());
  const inputRows = Object.entries(object(run.inputs))
    .slice(0, 24)
    .map(([name, hash]) => row(name, name, hash));
  return [
    {
      zh: '执行结果',
      en: 'Outcome',
      rows: [
        row('类型', 'Kind', operation.kind),
        row('状态', 'Status', operation.status),
        row('阶段', 'Phase', operation.phase),
        row('退出码', 'Exit code', status.exit_code ?? evidence.exit_code),
        row('开始时间', 'Started', time(operation.started_ms)),
        row('结束时间', 'Finished', time(operation.finished_ms)),
        row(
          '耗时（秒）',
          'Duration (seconds)',
          operation.duration_ms == null ? null : operation.duration_ms / 1000,
        ),
        row('发起者', 'Initiator', operation.initiator),
        row(
          '错误',
          'Error',
          evidence.error ?? evidence.provenance_error ?? evidence.provenance_capture_failed,
        ),
      ],
    },
    {
      zh: '算例与版本',
      en: 'Case and revision',
      rows: [
        row('原始算例', 'Source case', run.source_case ?? operation.case_path),
        row('冻结运行副本', 'Frozen run copy', run.run_case),
        row('版本', 'Version', operation.version_id ?? object(run.version).id),
        row('修订', 'Revision', operation.revision ?? object(run.version).revision),
        row('参数与过程覆盖 SHA-256', 'Overlay SHA-256', run.overlay_sha256),
        row('修订指纹', 'Revision fingerprint', run.revision_fingerprint),
        row('评估变量', 'Evaluation variable', evidence.variable),
        row('评估开始', 'Evaluation start', evidence.from),
        row('评估结束', 'Evaluation end', evidence.to),
        row('预热跳过记录数', 'Skipped model records', evidence.spinup),
      ],
    },
    {
      zh: '计算资源',
      en: 'Computing resources',
      rows: [
        row('位置', 'Location', operation.location ?? object(summary.location).kind),
        row('主机', 'Host', operation.host ?? remote.host),
        row('系统', 'System', evidence.os),
        row('CPU', 'CPU', evidence.cpu),
        row('登录方式', 'Login method', evidence.login_method),
        row('远程根目录', 'Remote root', remote.root),
        row('调度器', 'Scheduler', remote.scheduler ?? evidence.scheduler),
        row('作业 ID', 'Job ID', remote.scheduler_id ?? remote.job ?? evidence.job),
        row('引擎', 'Engine', evidence.engine ?? summary.engine ?? remote.engine),
        row('引擎快照', 'Engine snapshot', evidence.engine_snapshot ?? summary.execution_snapshot),
        row('核配置', 'Kernel', evidence.kernel ?? kernel.dir ?? summary.kernel),
        row('核身份', 'Kernel identity', kernel.identity),
        row('并行进程数', 'Parallel ranks', evidence.ranks ?? summary.ranks),
        row('线程数', 'Threads', evidence.threads),
      ],
    },
    {
      zh: '源码出处',
      en: 'Source provenance',
      rows: [
        row('源码类型', 'Code source', code.kind),
        row('工作区', 'Workspace', code.workspace ?? evidence.workspace_name),
        row('工作区根目录', 'Workspace root', evidence.workspace_root),
        row('提交', 'Commit', code.commit),
        row('源码内容 SHA-256', 'Source content SHA-256', code.source_sha256),
        row('记录来源', 'Record source', operation.source_reference),
      ],
    },
    {
      zh: '输入与输出依据',
      en: 'Input and output evidence',
      rows: [
        row('算例 SHA-256', 'Case SHA-256', run.case_sha256),
        row('生效配置 SHA-256', 'Effective namelist SHA-256', run.effective_nml_sha256),
        row('完整输入标识算法', 'Input identity algorithm', identity.algorithm),
        row('完整输入标识是否完整', 'Input identity complete', identity.complete),
        row('数据 SHA-256', 'Data SHA-256', identity.data_sha256),
        ...inputRows,
        row('输入哈希条目数', 'Input hash entries', Object.keys(object(run.inputs)).length || null),
        row('产物路径', 'Artifact paths', run.artifact_paths),
        row('输出路径', 'Output path', evidence.output_path ?? evidence.output_dir),
        row(
          '出处文件',
          'Provenance file',
          provenance.kind === 'provenance-reference' ? provenance.path : null,
        ),
        row(
          '出处文件 SHA-256',
          'Provenance file SHA-256',
          provenance.kind === 'provenance-reference' ? provenance.sha256 : null,
        ),
        row(
          '出处文件大小（字节）',
          'Provenance bytes',
          provenance.kind === 'provenance-reference' ? provenance.bytes : null,
        ),
      ],
    },
    {
      zh: '命令',
      en: 'Command',
      rows: [
        row('程序', 'Program', object(command).program),
        {
          zh: '完整参数（按顺序，每行一个）',
          en: 'Complete arguments (in order, one per line)',
          value: Array.isArray(object(command).argv)
            ? (object(command).argv as unknown[]).map((v) => JSON.stringify(v)).join('\n')
            : null,
        },
        row('执行目录', 'Working directory', object(command).cwd),
      ],
    },
  ];
}
export function historyReferences(
  detail: HistoryDetail,
): { zh: string; en: string; path: string }[] {
  const evidence = historyEvidence(detail),
    provenance = object(evidence.provenance),
    run = object(evidence.model_run);
  const references: { zh: string; en: string; path: string }[] = [];
  const add = (zh: string, en: string, path: unknown) => {
    if (absolutePath(path) && !references.some((r) => r.path === path))
      references.push({ zh, en, path });
  };
  add('记录出处', 'Recorded source', detail.operation.source_reference);
  if (provenance.kind === 'provenance-reference')
    add('出处文件', 'Provenance file', provenance.path);
  add('日志', 'Log', evidence.log_path);
  add('输出', 'Output', evidence.output_path ?? evidence.output_dir);
  if (Array.isArray(run.artifact_paths))
    for (const path of run.artifact_paths.slice(0, 24)) add('产物', 'Artifact', path);
  return references;
}
export function historyEvents(detail: HistoryDetail): HistoryRow[][] {
  return detail.events.map((event) => {
    const e = object(event),
      evidence = object(e.detail);
    return [
      {
        zh: '时间',
        en: 'Time',
        value: typeof e.time_ms === 'number' ? new Date(e.time_ms).toLocaleString() : null,
      },
      { zh: '阶段', en: 'Phase', value: text(e.phase) },
      { zh: '状态', en: 'Status', value: text(e.status) },
      { zh: '操作', en: 'Action', value: text(evidence.action ?? e.kind) },
      { zh: '错误', en: 'Error', value: text(evidence.error) },
    ];
  });
}
export function historyRecordExport(detail: HistoryDetail) {
  return {
    content: JSON.stringify(detail, null, 2),
    name: `colm-history-${detail.operation.operation_id.replace(/[^a-zA-Z0-9_-]/g, '_')}.json`,
    type: 'application/json',
  };
}

export function scopedHistoryFilter(filter: Filter, casePath?: string): Filter {
  return { ...filter, ...(casePath ? { casePath } : {}) };
}
export function historyKindLabel(kind: string, en = false): string {
  const labels: Record<string, [string, string]> = {
    run: ['运行', 'Run'],
    'remote-run': ['远程运行', 'Remote run'],
    study: ['研究', 'Research'],
    version: ['版本修改', 'Version change'],
    case: ['算例修改', 'Case change'],
    check: ['检查', 'Check'],
    build: ['编译', 'Build'],
    assistant: ['助手', 'Assistant'],
  };
  return labels[kind]?.[en ? 1 : 0] ?? kind;
}
export function historyStatusLabel(status: string, en = false): string {
  const labels: Record<string, [string, string]> = {
    running: ['运行中', 'Running'],
    succeeded: ['已完成', 'Completed'],
    failed: ['失败', 'Failed'],
    cancelled: ['已取消', 'Cancelled'],
    interrupted: ['已中断', 'Interrupted'],
    unknown: ['未知', 'Unknown'],
  };
  return labels[status]?.[en ? 1 : 0] ?? status;
}
export function historyObject(operation: HistoryOperation): string {
  return (
    operation.case_id ||
    operation.case_path
      ?.replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .pop() ||
    operation.version_id ||
    operation.operation_id
  );
}
