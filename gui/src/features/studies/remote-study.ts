import type { VersionRef } from '../../lib/types';

export type RemoteStudyRecord = {
  host?: string;
  job?: string;
  fetched_at?: number | null;
  scientific_verified?: boolean;
  verification_error?: string;
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
  warnings: string[];
};

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

export function remoteStudyPermissions(
  record: RemoteStudyRecord | null | undefined,
  pending: unknown,
  reply: RemoteStudyReply | null,
) {
  const exists = Boolean(record || pending || reply);
  const state = reply?.status?.state;
  const terminal = state === 'finished' && Number.isInteger(reply?.status?.exit_code);
  const fetched = Boolean(record?.fetched_at || reply?.record?.fetched_at);
  const scientificVerified =
    record?.scientific_verified ??
    reply?.record?.scientific_verified ??
    reply?.scientific_verified ??
    null;
  return {
    exists,
    terminal,
    fetched,
    scientificVerified,
    submit: !exists,
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
