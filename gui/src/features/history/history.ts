import { download as exportText } from '../../lib/utils';
import { bilingual } from '../../lib/bilingual';
import { utcSeconds } from '../../lib/dates';
import type { AppState, CaseEntry, HistoryOperation } from '../../lib/types';
export type Filter = {
  query: string;
  kind: string;
  casePath: string;
  versionId: string;
  host: string;
  status: string;
  from: string;
  to: string;
  includeRoutine?: boolean;
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
  includeRoutine: false,
};
export const appLocale = (en = false) => (en ? 'en-US' : 'zh-CN');
export function historyFilter(filter: Filter, offset = 0, limit = 30) {
  const result: Record<string, string | number | boolean> = { offset, limit };
  for (const [key, value] of Object.entries(filter)) {
    if (value === undefined || value === '' || value === null) continue;
    if (key === 'from' || key === 'to') {
      const time = new Date(value as string).getTime();
      if (!Number.isFinite(time)) throw new Error(bilingual('日期格式不对', 'Invalid date'));
      result[key] = time;
    } else if (key === 'includeRoutine') {
      if (value) result.includeRoutine = true;
    } else result[key] = value;
  }
  if (typeof result.from === 'number' && typeof result.to === 'number' && result.from > result.to)
    throw new Error(bilingual('开始日期晚于结束日期', 'Start is after end'));
  return result;
}
export function dayGroups(items: HistoryOperation[], en = false) {
  const groups = new Map<string, HistoryOperation[]>();
  for (const item of items) {
    const day =
      item.time_ms == null
        ? en
          ? 'Unknown date'
          : '未知日期'
        : new Date(item.time_ms).toLocaleDateString(appLocale(en));
    groups.set(day, [...(groups.get(day) ?? []), item]);
  }
  return [...groups];
}
export function download(content: string, name: string, type: string) {
  return exportText(name, content, type);
}

export function canReplay(operation: HistoryOperation) {
  return (
    !operation.run_group &&
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
  if (!a || !b || a === b)
    throw new Error(bilingual('请选两次不同的运行', 'Select two different recorded runs'));
  if (!input.obs.trim() || !input.variable.trim())
    throw new Error(bilingual('真实观测与变量必填', 'Observations and variable required'));
  const time = (value: string) => {
    if (!value) return null;
    const seconds = utcSeconds(value);
    if (!Number.isSafeInteger(seconds))
      throw new Error(bilingual('日期格式不对', 'Invalid UTC date'));
    return seconds;
  };
  const from = time(input.from),
    to = time(input.to);
  if (from != null && to != null && from >= to)
    throw new Error(bilingual('开始时间要早于结束时间', 'Start must precede end'));
  const spinup = Number(input.spinup);
  if (!input.spinup.trim() || !Number.isSafeInteger(spinup) || spinup < 0)
    throw new Error(
      bilingual('跳过的模型记录数要是非负整数', 'Invalid skipped model record count'),
    );
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
export function historySections(detail: HistoryDetail, en = false): HistorySection[] {
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
  const row = (zh: string, enText: string, value: unknown): HistoryRow => ({
      zh,
      en: enText,
      value: text(value),
    }),
    time = (value: number | null) =>
      value == null ? null : new Date(value).toLocaleString(appLocale(en));
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
        row('阶段', 'Phase', historyPhaseLabel(operation.phase, en)),
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
export function historyEvents(detail: HistoryDetail, en = false): HistoryRow[][] {
  return detail.events.map((event) => {
    const e = object(event),
      evidence = object(e.detail);
    return [
      {
        zh: '时间',
        en: 'Time',
        value:
          typeof e.time_ms === 'number' ? new Date(e.time_ms).toLocaleString(appLocale(en)) : null,
      },
      { zh: '阶段', en: 'Phase', value: historyPhaseLabel(text(e.phase) ?? '', en) },
      { zh: '状态', en: 'Status', value: historyStatusLabel(text(e.status) ?? '', en) },
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
    'remote-run': ['服务器运行', 'Server run'],
    remote_run: ['服务器运行', 'Server run'],
    'remote-status': ['查看服务器作业', 'Server job status'],
    'remote-study-status': ['查看服务器研究', 'Server study status'],
    'remote-cancel': ['取消服务器作业', 'Cancel server job'],
    server: ['服务器', 'Server'],
    study: ['研究', 'Research'],
    'study-create': ['新建研究', 'New study'],
    'study-run': ['运行研究', 'Run study'],
    'study-clone': ['复制研究', 'Copy study'],
    'study-pause': ['暂停研究', 'Pause study'],
    'study-resume': ['继续研究', 'Resume study'],
    'study-cancel': ['取消研究', 'Cancel study'],
    'study-retry': ['重试研究', 'Retry study'],
    'study-export': ['导出研究', 'Export study'],
    'study-apply': ['采用研究结果', 'Apply study result'],
    'study-save-version': ['研究结果存为版本', 'Save study as version'],
    version: ['版本修改', 'Version change'],
    'model-create': ['新建版本', 'New version'],
    'model-save-draft': ['保存版本草稿', 'Save version draft'],
    'model-freeze': ['存为版本', 'Save version'],
    'model-migrate-case': ['绑定算例版本', 'Bind case to version'],
    'model-attach-network': ['添加 AI 网络', 'Attach AI network'],
    'model-import-process': ['导入过程参数文件', 'Import process file'],
    case: ['算例修改', 'Case change'],
    'case-config-save': ['保存算例设置', 'Save case settings'],
    new: ['新建算例', 'New case'],
    'remote-case-create': ['在服务器上新建算例', 'New case on server'],
    'site-new': ['新建站点算例', 'New site case'],
    'spatial-new': ['新建区域算例', 'New regional case'],
    'mesh-new': ['新建网格算例', 'New mesh case'],
    'new-simulation': ['新建模拟（未完成）', 'Unfinished new simulation'],
    'hybrid-fit': ['训练 AI 网络', 'Train AI network'],
    'hybrid-install': ['安装 AI 网络', 'Install AI network'],
    'hybrid-remove': ['移除 AI 网络', 'Remove AI network'],
    'hybrid-climate': ['AI 网络气候统计', 'AI network climatology'],
    'forcing-convert': ['转换强迫场', 'Convert forcing'],
    'forcing-table-convert': ['转换强迫场表格', 'Convert forcing table'],
    'forcing-repair': ['修补强迫场', 'Repair forcing'],
    'era5land-download': ['下载 ERA5-Land', 'Download ERA5-Land'],
    'observation-table-convert': ['转换观测表格', 'Convert observation table'],
    'history-replay': ['按记录重跑', 'Rerun from record'],
    'history-compare': ['对比两次运行', 'Compare runs'],
    'ws-create': ['新建代码工作区', 'New code workspace'],
    'ws-note': ['修改版本说明', 'Edit version note'],
    'ws-build-engine': ['编译引擎', 'Build engine'],
    'ws-build-kernel': ['编译内核', 'Build kernel'],
    'ws-test': ['运行测试', 'Run tests'],
    'ws-parity': ['新旧对照', 'Parity check'],
    'ws-patch': ['应用补丁', 'Apply patch'],
    'ws-revert-file': ['撤销文件改动', 'Revert file'],
    'ws-export': ['导出改动', 'Export changes'],
    'study-preflight': ['研究预检', 'Study preflight'],
    'remote-study-run': ['服务器运行研究', 'Run study on server'],
    'remote-study-fetch': ['取回研究结果', 'Fetch study results'],
    'remote-fetch': ['取回结果', 'Fetch results'],
    'remote-probe': ['检查服务器', 'Check server'],
    'engine-pack': ['打包引擎', 'Package engine'],
    // 台账会记下的命令（colm-cli history_cmd::action_kind）都要有名字，否则历史里显示成命令名。
    all: ['一键运行', 'Run all stages'],
    'run-replay': ['按记录重跑', 'Rerun from record'],
    readiness: ['检查运行条件', 'Check readiness'],
    'version-check': ['版本自检', 'Version check'],
    'hybrid-check': ['检查 AI 网络', 'Check AI network'],
    'model-hybrid-check': ['检查版本的 AI 网络', 'Check version AI network'],
    'kernel-build': ['编译内核', 'Build kernel'],
    'remote-kernel': ['在服务器上编译内核', 'Build kernel on server'],
    'remote-dist': ['向服务器分发引擎', 'Send engine to server'],
    'study-finalize-cancel': ['确认研究已取消', 'Confirm study cancellation'],
    'study-finalize-idle-cancel': ['确认研究已取消', 'Confirm study cancellation'],
    'model-revise': ['修改版本', 'Revise version'],
    'model-import-study': ['研究结果写进草稿', 'Write study results into draft'],
    'model-branch-code': ['复制版本代码', 'Copy version code'],
    'model-rollback': ['回退版本', 'Roll back version'],
    'model-make-run': ['准备版本运行目录', 'Prepare version run'],
    'new-spatial': ['新建区域算例', 'New regional case'],
    'crop-new': ['新建作物算例', 'New crop case'],
    clone: ['复制算例', 'Copy case'],
    set: ['修改算例设置', 'Change case setting'],
    'set-many': ['修改算例设置', 'Change case settings'],
    fetch: ['取回结果', 'Fetch results'],
    'remote-test': ['测试服务器连接', 'Test server connection'],
    'remote-clean': ['清理服务器文件', 'Clean server files'],
    'remote-engine-clean': ['清理服务器引擎', 'Clean server engine'],
    'remote-engine-recover': ['恢复服务器引擎', 'Recover server engine'],
    'remote-engines': ['查看服务器引擎', 'List server engines'],
    'remote-stop': ['停止服务器作业', 'Stop server job'],
    'remote-study-cancel': ['取消服务器研究', 'Cancel server study'],
    'remote-unlock': ['解除服务器锁', 'Unlock server'],
    'ws-run': ['用工作区代码运行', 'Run with workspace code'],
    'ws-build': ['编译工作区', 'Build workspace'],
    'ws-readiness': ['检查工作区', 'Check workspace'],
    'ws-regress': ['工作区回归检查', 'Workspace regression check'],
    'ws-write': ['修改代码文件', 'Edit code file'],
    'ws-put': ['写入代码文件', 'Write code file'],
    'ws-put-dir': ['写入代码目录', 'Write code folder'],
    'ws-rm': ['删除代码文件', 'Delete code file'],
    'ws-revert': ['撤销代码改动', 'Revert code changes'],
    'ws-adopt': ['采用工作区改动', 'Adopt workspace changes'],
    'ws-delete': ['删除代码工作区', 'Delete code workspace'],
    project: ['项目', 'Project'],
    check: ['检查', 'Check'],
    build: ['编译', 'Build'],
    assistant: ['助手', 'Assistant'],
    assistant_ui: ['助手操作', 'Assistant action'],
    assistant_key: ['助手密钥设置', 'Assistant key setting'],
  };
  const label = labels[kind];
  if (label) return label[en ? 1 : 0];
  return en ? kind.replace(/[-_]/g, ' ') : kind;
}
/** 同一算例在这条记录之后又开始过运行：软件只核对每个算例最新的那次。 */
export function hasNewerRun(operation: HistoryOperation, members: HistoryOperation[]): boolean {
  const start = operation.started_ms ?? operation.time_ms ?? 0;
  return members.some(
    (member) =>
      member.operation_id !== operation.operation_id &&
      !!operation.case_path &&
      member.case_path === operation.case_path &&
      ['run', 'remote-run', 'remote_run'].includes(member.kind) &&
      (member.started_ms ?? member.time_ms ?? 0) > start,
  );
}
/**
 * 一条"待核对"记录为什么还不算结束、怎样才会变成已完成。台账只在有窗口盯着时才记下结束：
 * 服务器作业在没人看的时候跑完、请求取消后没人再查、软件中途被关，记录都会停在待核对。
 */
export function pendingReason(operation: HistoryOperation, en = false, newer = false): string {
  const at =
    operation.time_ms == null ? '' : new Date(operation.time_ms).toLocaleString(appLocale(en));
  const since = at ? (en ? ` (${at})` : `（${at}）`) : '';
  const remote = operation.location === 'remote' || operation.kind.startsWith('remote');
  if (newer)
    return en
      ? 'This case ran again later. The app only checks a case’s latest run, so this older record will not update by itself.'
      : '这个算例之后又运行过，软件只核对最新的那次，这条旧记录不会再自动更新。';
  if (operation.phase === 'imported')
    return en
      ? 'A job record imported from the case folder; the history has no record of how it ended.'
      : '从算例文件夹导入的作业记录，历史里没有它怎么结束的信息。';
  if (operation.status === 'cancel_requested')
    return en
      ? `Cancellation was requested${since}, but the app has not confirmed that the server job stopped.`
      : `已请求取消${since}，软件还没确认服务器上的作业已经停下。`;
  const seen: Record<string, [string, string]> = {
    running: ['运行中', 'running'],
    queued: ['排队中', 'queued'],
    submitted: ['已提交', 'submitted'],
  };
  if (remote && seen[operation.status])
    return en
      ? `Last seen on the server as ${seen[operation.status][1]}${since}. The app has not checked since, so it does not know whether the job ended. Open the project’s run page and the app checks the server by itself.`
      : `上次向服务器查到：${seen[operation.status][0]}${since}。之后软件没再查，所以不知道它是否已经结束。打开这个项目的运行页，软件会自动向服务器核对。`;
  if (operation.status === 'running')
    return en
      ? `The app recorded this run starting on this computer${since} but never its end, most likely because the app was closed during the run.`
      : `软件记下它在本机开始运行${since}，但没有记下结束，多半是运行途中软件被关闭了。`;
  return remote
    ? en
      ? `The app could not confirm that the server received this job${since} (for example, the connection dropped). Use “Check server status” on the run page.`
      : `提交时没能确认服务器收到了这个作业${since}（例如连接中断）。可在运行页点“核对服务器状态”确认。`
    : en
      ? 'The app did not record how this operation ended.'
      : '软件没有记下这项操作怎么结束。';
}
export function historyStatusLabel(status: string, en = false): string {
  const labels: Record<string, [string, string]> = {
    running: ['运行中', 'Running'],
    succeeded: ['已完成', 'Completed'],
    completed_with_failures: ['部分失败', 'Partially failed'],
    failed: ['失败', 'Failed'],
    cancelled: ['已取消', 'Cancelled'],
    completed: ['已完成', 'Completed'],
    paused: ['已暂停', 'Paused'],
    needs_review: ['待复核', 'Needs review'],
    interrupted: ['已中断', 'Interrupted'],
    unknown: ['未知', 'Unknown'],
  };
  return labels[status]?.[en ? 1 : 0] ?? status;
}
export function historyPhaseLabel(phase?: string | null, en = false): string {
  if (!phase) return '';
  const labels: Record<string, [string, string]> = {
    started: ['已启动', 'Started'],
    status: ['状态更新', 'Status update'],
    finished: ['已结束', 'Finished'],
    imported: ['已导入', 'Imported'],
    mksrfdata: ['地表数据', 'Surface data'],
    mkinidata: ['初始条件', 'Initial conditions'],
    colm: ['主模型计算', 'Simulation'],
    submit: ['已提交', 'Submitted'],
  };
  return labels[phase.toLowerCase()]?.[en ? 1 : 0] ?? phase;
}
/** The command an operation ran (`study-run`, `new`, …); older records only carry their kind. */
export function historyAction(operation: HistoryOperation): string {
  const action = operation.detail?.action;
  return typeof action === 'string' && action ? action : operation.kind;
}
/** What the operation did, e.g. 运行研究 rather than the generic kind 研究. */
export function historyActionLabel(operation: HistoryOperation, en = false): string {
  const action = historyAction(operation);
  const label = historyKindLabel(action, en);
  return label === action || label === action.replace(/[-_]/g, ' ')
    ? historyKindLabel(operation.kind, en)
    : label;
}
/** Bookkeeping the app does by itself (binding versions, polling servers, autosaving settings). */
export function historyInternal(operation: HistoryOperation): boolean {
  if (operation.run_group) return false;
  return (
    operation.kind === 'case-config-save' ||
    ['model-migrate-case', 'remote-status', 'remote-study-status', 'remote-probe'].includes(
      historyAction(operation),
    )
  );
}
const lastName = (path: string) =>
  path
    .replace(/[\\/]+$/, '')
    .split(/[\\/]/)
    .pop() || '';
const parentName = (path: string) =>
  lastName(path.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]*$/, ''));
const nonEmpty = (value: unknown) => (typeof value === 'string' && value ? value : undefined);
/**
 * The record's subject as a name and its project: recorded since 2026-10-18 as
 * `detail.subject`, and derived from the case path, the command's first argument or its
 * `--out`/`--name` for older records. Unknown subjects show a short record number.
 */
export function historySubject(operation: HistoryOperation): {
  name: string;
  project?: string;
  kind?: string;
} {
  const subject = operation.detail?.subject as
    { kind?: unknown; name?: unknown; project?: unknown } | undefined;
  if (nonEmpty(subject?.name))
    return {
      name: nonEmpty(subject?.name)!,
      project: nonEmpty(subject?.project),
      kind: nonEmpty(subject?.kind),
    };
  if (operation.case_path)
    return {
      name: lastName(operation.case_path),
      project: parentName(operation.case_path) || undefined,
    };
  if (operation.case_id) return { name: operation.case_id };
  const args = (operation.detail?.args ?? {}) as Record<string, unknown>;
  const path =
    nonEmpty(args['--out']) ??
    nonEmpty(operation.detail?.object) ??
    nonEmpty(operation.detail?.workspace_name) ??
    nonEmpty(operation.command?.argv?.[1])?.replace(/^-.*/, '');
  const name = nonEmpty(args['--name']) ?? (path ? lastName(path) : undefined);
  if (name) return { name, project: path ? parentName(path) || undefined : undefined };
  if (operation.version_id) return { name: operation.version_id, kind: 'version' };
  if (operation.host) return { name: operation.host, kind: 'server' };
  return { name: `#${operation.operation_id.slice(0, 8)}` };
}
export type ProjectTimelineRow = {
  key: string;
  action: string;
  members: HistoryOperation[];
  time_ms: number | null;
  counts: { succeeded: number; failed: number; running: number; pending: number };
};
const when = (operation: HistoryOperation) => operation.started_ms ?? operation.time_ms ?? 0;
function countStatuses(members: HistoryOperation[]) {
  const counts = { succeeded: 0, failed: 0, running: 0, pending: 0 };
  for (const member of members) {
    if (member.needs_reconciliation) counts.pending++;
    else if (['succeeded', 'completed'].includes(member.status)) counts.succeeded++;
    else if (['failed', 'completed_with_failures', 'cancelled'].includes(member.status))
      counts.failed++;
    else counts.running++;
  }
  return counts;
}
/**
 * A project's operations as a timeline, newest first. Consecutive operations of the same
 * action within a quarter of an hour (a batch of stations) share one row. Imported
 * placeholders (a case import repeating a job its run already records) are left out when the
 * project has anything else.
 */
export function projectTimeline(members: HistoryOperation[]): ProjectTimelineRow[] {
  const real = members.filter((member) => member.phase !== 'imported');
  const shown = [...(real.length ? real : members)].sort((a, b) => when(b) - when(a));
  const rows: ProjectTimelineRow[] = [];
  for (const member of shown) {
    const action = historyAction(member);
    const last = rows.at(-1);
    if (
      last &&
      last.action === action &&
      Math.abs((last.time_ms ?? 0) - when(member)) <= 15 * 60_000
    ) {
      last.members.push(member);
      continue;
    }
    rows.push({
      key: member.operation_id,
      action,
      members: [member],
      time_ms: when(member) || null,
      counts: { succeeded: 0, failed: 0, running: 0, pending: 0 },
    });
  }
  for (const row of rows) row.counts = countStatuses(row.members);
  return rows;
}
/** How many cases a project touched and how its latest operations ended. */
export function projectSummary(group: NonNullable<HistoryOperation['project_group']>) {
  const real = group.members.filter((member) => member.phase !== 'imported');
  const latestPerCase = new Map<string, HistoryOperation>();
  for (const member of [...(real.length ? real : group.members)].sort((a, b) => when(b) - when(a)))
    if (member.case_path && !latestPerCase.has(member.case_path))
      latestPerCase.set(member.case_path, member);
  return { cases: latestPerCase.size, counts: countStatuses([...latestPerCase.values()]) };
}
export function statusCountsLabel(counts: ProjectTimelineRow['counts'], en = false): string {
  const parts = [
    counts.succeeded && (en ? `${counts.succeeded} succeeded` : `成功 ${counts.succeeded}`),
    counts.failed && (en ? `${counts.failed} failed` : `失败 ${counts.failed}`),
    counts.running && (en ? `${counts.running} running` : `进行中 ${counts.running}`),
    counts.pending && (en ? `${counts.pending} to reconcile` : `待核对 ${counts.pending}`),
  ].filter(Boolean);
  return parts.join(' · ');
}
/** `official` r64 → 官方版 r64; other versions by their registry name when known. */
export function historyVersionLabel(
  operation: Pick<HistoryOperation, 'version_id' | 'revision'>,
  names: Record<string, string> = {},
  en = false,
): string | null {
  if (!operation.version_id) return null;
  const name =
    operation.version_id === 'official'
      ? en
        ? 'Official'
        : '官方版'
      : (names[operation.version_id] ?? operation.version_id);
  return operation.revision ? `${name} r${operation.revision}` : name;
}
export function historyDurationLabel(ms: number | null | undefined, en = false): string | null {
  if (ms == null || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < 1000) return en ? '< 1 s' : '不到 1 秒';
  const seconds = Math.round(ms / 1000);
  if (seconds < 60) return en ? `${seconds} s` : `${seconds} 秒`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return en ? `${minutes} min` : `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  return en ? `${hours} h ${minutes % 60} min` : `${hours} 小时 ${minutes % 60} 分`;
}
export function historyObject(operation: HistoryOperation, en = false): string {
  if (operation.run_group) {
    const parents = operation.run_group.members.map((member) =>
      member.case_path?.replace(/[\\/]+$/, '').replace(/[\\/][^\\/]+$/, ''),
    );
    const parent = parents[0];
    return parent && parents.every((value) => value === parent)
      ? parent.split(/[\\/]/).pop() || (en ? 'Multi-site run' : '多站点运行')
      : en
        ? 'Multi-site run'
        : '多站点运行';
  }
  return historySubject(operation).name;
}

export type HistoryDeletePlan = {
  casePath: string | null;
  caseHost: string | null;
  caseReason: string | null;
  id: string;
  path: string | null;
  host: string | null;
  reason: string | null;
};
export function canDeleteHistory(operation: HistoryOperation): boolean {
  if (simulationDraft(operation)) return true;
  return (
    !operation.needs_reconciliation &&
    (!operation.run_group || operation.run_group.members.every(canDeleteHistory)) &&
    ['succeeded', 'failed', 'completed', 'completed_with_failures', 'cancelled'].includes(
      operation.status,
    )
  );
}

export function historyDirectoryContains(directory: string, candidate: string): boolean {
  const windows = /^(?:[a-z]:[\\/]|\\\\|\/\/)/i.test(directory);
  const normalize = (value: string) =>
    (windows
      ? value
          .replace(/\\/g, '/')
          .toLowerCase()
          .replace(/^\/\/\?\/unc\//, '//')
          .replace(/^\/\/\?\//, '')
      : value
    ).replace(/\/+$/, '');
  const root = normalize(directory);
  const path = normalize(candidate);
  return path === root || path.startsWith(`${root}/`);
}

export function historyStateAfterDirectoryDeletion(
  state: Pick<AppState, 'selectedCase' | 'cases' | 'running'>,
  path: string,
  host?: string | null,
): Pick<AppState, 'selectedCase' | 'cases' | 'running'> {
  const deletedEntry = (entry: CaseEntry) =>
    host
      ? entry.run_host === host &&
        !!entry.remote_directory &&
        historyDirectoryContains(path, entry.remote_directory)
      : historyDirectoryContains(path, entry.dir);
  const selectedDeleted = !!state.selectedCase && deletedEntry(state.selectedCase.entry);
  const removedDirs = state.cases.filter(deletedEntry).map((entry) => entry.dir);
  if (selectedDeleted) removedDirs.push(state.selectedCase!.dir);
  const removedPath = (value: string) =>
    host
      ? removedDirs.some((dir) => historyDirectoryContains(dir, value))
      : historyDirectoryContains(path, value);
  return {
    selectedCase: selectedDeleted ? null : state.selectedCase,
    cases: state.cases.filter((entry) => !deletedEntry(entry)),
    running: Object.fromEntries(
      Object.entries(state.running).filter(
        ([, value]) => !removedPath(value.case) && !(value.run_case && removedPath(value.run_case)),
      ),
    ),
  };
}

/** What a replay could confirm: forcing contents are never verified (they are not fingerprinted). */
export function replayVerificationText(result: unknown, en = false) {
  const verification =
    result && typeof result === 'object'
      ? (result as { verification?: { other_inputs?: string } }).verification
      : undefined;
  const others = verification?.other_inputs;
  const tail =
    others === 'verified'
      ? en
        ? 'other recorded inputs were confirmed unchanged.'
        : '其他已记录的输入已核对一致。'
      : others === 'server'
        ? en
          ? 'other inputs are checked on the server.'
          : '其他输入在服务器上核对。'
        : en
          ? 'the original run recorded no comparable input fingerprints, so other inputs were not verified either.'
          : '原运行没有记录可比对的输入指纹，其他输入也未核对。';
  return en
    ? `Rerun with the recorded model version and settings. Forcing contents were not verified; ${tail}`
    : `已按记录的模型版本和设置重跑。强迫场内容未核对；${tail}`;
}

/** The filled-in wizard an unfinished 新建模拟 record keeps (`simulation_draft.rs`), with its step. */
export function simulationDraft(
  operation: HistoryOperation,
): (NonNullable<AppState['newCaseSeed']> & { step: 1 | 2 | 3 | 4 | 5 }) | null {
  if (operation.kind !== 'new-simulation') return null;
  const seed = operation.detail?.seed as AppState['newCaseSeed'] | undefined,
    recorded = operation.detail?.step,
    step = typeof recorded === 'number' ? recorded : (seed?.step ?? 1);
  return seed && typeof seed === 'object' && seed.form && typeof seed.form === 'object'
    ? { ...seed, step: Math.min(5, Math.max(1, Math.round(step))) as 1 | 2 | 3 | 4 | 5 }
    : null;
}
