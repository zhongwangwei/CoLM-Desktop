import type { VersionRef } from '../../lib/types';

export type RemoteStudyRecord = {
  host?: string;
  job?: string;
  fetched_at?: number | null;
  scientific_verified?: boolean;
  verification_error?: string;
};
/**
 * 服务器作业停在哪一步。后端（`remote.rs::RemoteStudyOutcome`）按 CLI 已公开的字段分类：
 * 记录/状态/退出码/阶段标记/服务器上有没有研究目录。前端不再自己解析英文阶段名。
 */
export type RemoteStudyOutcome =
  | 'not_submitted'
  | 'start_failed'
  | 'build_failed'
  | 'study_create_failed'
  | 'member_run_failed'
  | 'succeeded'
  | 'cancelled'
  | 'lost'
  | 'running'
  | 'unknown';

export type RemoteStudyRecovery = {
  outcome: RemoteStudyOutcome;
  terminal: boolean;
  finished: boolean;
  study_created: boolean;
  fetched: boolean;
  exit_code?: number;
  phase?: string;
  job?: string;
  permissions: {
    submit: boolean;
    resubmit: boolean;
    cancel: boolean;
    fetch: boolean;
    close: boolean;
    poll: boolean;
  };
  next_zh: string;
  next_en: string;
  fetch_error_zh?: string;
  fetch_error_en?: string;
};

export type RemoteStudyReply = {
  host?: string;
  job?: string;
  outcome?: string;
  request_accepted?: boolean;
  terminal?: boolean;
  scientific_verified?: boolean;
  record?: RemoteStudyRecord;
  status?: { state?: string; exit_code?: number; phase?: string | null; log_tail?: string };
  study?: {
    status?: string;
    completed_candidates?: number;
    generation?: number;
    best_member?: string | null;
  };
  recovery?: RemoteStudyRecovery;
  warnings: string[];
};

const REMOTE_STUDY_OUTCOMES: readonly RemoteStudyOutcome[] = [
  'not_submitted',
  'start_failed',
  'build_failed',
  'study_create_failed',
  'member_run_failed',
  'succeeded',
  'cancelled',
  'lost',
  'running',
  'unknown',
];

/** 只接受后端公开的形状：坏字段按“没有分类”处理，页面退回旧的权限推断而不是崩掉。 */
export function remoteStudyRecovery(value: unknown): RemoteStudyRecovery | undefined {
  if (!value || typeof value !== 'object') return undefined;
  const data = value as Record<string, unknown>;
  const outcome = REMOTE_STUDY_OUTCOMES.find((item) => item === data.outcome);
  if (!outcome) return undefined;
  const text = (v: unknown) => (typeof v === 'string' ? v : undefined);
  const flag = (v: unknown) => (typeof v === 'boolean' ? v : undefined);
  const permissions =
    data.permissions && typeof data.permissions === 'object'
      ? (data.permissions as Record<string, unknown>)
      : {};
  const permission = (key: string) => flag(permissions[key]) === true;
  return {
    outcome,
    terminal: flag(data.terminal) === true,
    finished: flag(data.finished) === true,
    study_created: flag(data.study_created) === true,
    fetched: flag(data.fetched) === true,
    exit_code:
      typeof data.exit_code === 'number' && Number.isInteger(data.exit_code)
        ? data.exit_code
        : undefined,
    phase: text(data.phase),
    job: text(data.job),
    permissions: {
      submit: permission('submit'),
      resubmit: permission('resubmit'),
      cancel: permission('cancel'),
      fetch: permission('fetch'),
      close: permission('close'),
      poll: permission('poll'),
    },
    next_zh: text(data.next_zh) ?? '',
    next_en: text(data.next_en) ?? '',
    fetch_error_zh: text(data.fetch_error_zh),
    fetch_error_en: text(data.fetch_error_en),
  };
}

// Only public job metadata belongs in the page or the assistant's UI snapshot.
export function remoteStudyReply(value: unknown): RemoteStudyReply {
  const data = value && typeof value === 'object' ? (value as Record<string, unknown>) : {};
  const record =
    data.record && typeof data.record === 'object' ? (data.record as Record<string, unknown>) : {};
  const status =
    data.status && typeof data.status === 'object' ? (data.status as Record<string, unknown>) : {};
  const study =
    data.study && typeof data.study === 'object' ? (data.study as Record<string, unknown>) : {};
  const provenance =
    record.provenance && typeof record.provenance === 'object'
      ? (record.provenance as Record<string, unknown>)
      : {};
  const recovery = remoteStudyRecovery(data.recovery);
  const text = (v: unknown) => (typeof v === 'string' ? v : undefined);
  const number = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);
  return {
    host: text(data.host),
    job: text(data.job),
    outcome: text(data.outcome),
    request_accepted:
      typeof data.request_accepted === 'boolean' ? data.request_accepted : undefined,
    terminal: typeof data.terminal === 'boolean' ? data.terminal : undefined,
    scientific_verified:
      typeof data.scientific_verified === 'boolean' ? data.scientific_verified : undefined,
    record: Object.keys(record).length
      ? {
          host: text(record.host),
          job: text(record.job),
          fetched_at: number(record.fetched_at),
          ...(typeof provenance.scientific_verified === 'boolean'
            ? { scientific_verified: provenance.scientific_verified }
            : {}),
          ...(typeof provenance.verification_error === 'string'
            ? { verification_error: provenance.verification_error }
            : {}),
        }
      : undefined,
    status: Object.keys(status).length
      ? {
          state: text(status.state),
          exit_code: number(status.exit_code),
          phase: text(status.phase),
          log_tail: text(status.log_tail),
        }
      : undefined,
    ...(recovery ? { recovery } : {}),
    study: Object.keys(study).length
      ? {
          status: text(study.status),
          completed_candidates: number(study.completed_candidates),
          generation: number(study.generation),
          best_member: text(study.best_member),
        }
      : undefined,
    warnings: [
      'ledger_warning',
      'record_warning',
      'lock_warning',
      'cancel_error',
      'status_error',
      'verification_error',
    ].flatMap((key) => (typeof data[key] === 'string' ? [data[key] as string] : [])),
  };
}

export type RemoteStudyPermissions = {
  exists: boolean;
  terminal: boolean;
  finished?: boolean;
  fetched: boolean;
  scientificVerified: boolean | null;
  submit: boolean;
  resubmit: boolean;
  cancel: boolean;
  fetch: boolean;
  close: boolean;
  poll: boolean;
  outcome?: RemoteStudyOutcome;
  studyCreated?: boolean;
  exitCode?: number;
  /** 每种状态一条可执行的下一步（看日志/取回诊断/归档关闭/重提）。 */
  nextStep?: { zh: string; en: string };
  /** 取回按钮可点、但服务器上没有可取的结果时的原因说明。 */
  fetchNote?: { zh: string; en: string };
  closeLabel?: { zh: string; en: string };
};

/** 关闭按钮的文案：失败终态叫“归档”，丢失仍叫“确认丢失”（动作不同，不要混）。 */
function remoteStudyCloseLabel(recovery: RemoteStudyRecovery): { zh: string; en: string } {
  if (recovery.fetched) {
    return { zh: '归档已取回的服务器记录…', en: 'Archive the fetched server record…' };
  }
  switch (recovery.outcome) {
    case 'start_failed':
    case 'build_failed':
    case 'study_create_failed':
    case 'member_run_failed':
      return { zh: '归档失败的服务器记录…', en: 'Archive the failed server record…' };
    case 'cancelled':
      return { zh: '归档已取消的服务器记录…', en: 'Archive the cancelled server record…' };
    case 'succeeded':
      return { zh: '归档成功的服务器记录…', en: 'Archive the finished server record…' };
    default:
      return { zh: '确认服务器作业已丢失…', en: 'Mark the server job as lost…' };
  }
}

export function remoteStudyPermissions(
  record: RemoteStudyRecord | null | undefined,
  pending: unknown,
  reply: RemoteStudyReply | null,
): RemoteStudyPermissions {
  const exists = Boolean(record || pending || reply);
  const state = reply?.status?.state;
  const terminal = state === 'finished' && Number.isInteger(reply?.status?.exit_code);
  const fetched = Boolean(record?.fetched_at || reply?.record?.fetched_at);
  const scientificVerified =
    record?.scientific_verified ??
    reply?.record?.scientific_verified ??
    reply?.scientific_verified ??
    null;
  const recovery = reply?.recovery;
  if (recovery) {
    // 有后端的分类就用它：失败阶段、可重提、可否归档、下一步文案都由同一处决定，
    // 前端不再自己猜 phase 字符串，避免两套逻辑分叉。
    return {
      exists,
      terminal: recovery.terminal,
      finished: recovery.finished,
      fetched: fetched || recovery.fetched,
      scientificVerified,
      submit: recovery.permissions.submit,
      resubmit: recovery.permissions.resubmit,
      cancel: recovery.permissions.cancel,
      fetch: recovery.permissions.fetch,
      close: recovery.permissions.close,
      poll: recovery.permissions.poll,
      outcome: recovery.outcome,
      studyCreated: recovery.study_created,
      exitCode: recovery.exit_code,
      nextStep:
        recovery.next_zh && recovery.next_en
          ? { zh: recovery.next_zh, en: recovery.next_en }
          : undefined,
      // 取回按钮可点但取不回来（构建/创建失败等）：点之前就把原因和下一步摆出来。
      fetchNote:
        recovery.fetch_error_zh && recovery.fetch_error_en
          ? { zh: recovery.fetch_error_zh, en: recovery.fetch_error_en }
          : undefined,
      closeLabel: recovery.permissions.close ? remoteStudyCloseLabel(recovery) : undefined,
    };
  }
  return {
    exists,
    terminal,
    finished: terminal,
    fetched,
    scientificVerified,
    submit: !exists,
    resubmit: false,
    cancel: Boolean(record) && !fetched && ['queued', 'running'].includes(state ?? ''),
    fetch: Boolean(record) && terminal && !fetched,
    // 服务器作业已丢失/无法核对：用户确认后可以关闭本机记录（R25）。排队/运行中不能关，正常结束的应取回。
    close:
      Boolean(record || pending) &&
      !fetched &&
      !terminal &&
      ['lost', 'unknown'].includes(state ?? ''),
    poll: exists && !fetched && !terminal && state !== 'lost',
  };
}

export function studyModelRevision(
  selected: VersionRef,
  pinned?: { model_revision?: { version?: VersionRef } | null } | null,
): VersionRef {
  const version = pinned?.model_revision?.version ?? selected;
  if (!version.id || !Number.isSafeInteger(version.revision) || version.revision < 1)
    throw new Error('Study requires an immutable model revision');
  return { ...version };
}
