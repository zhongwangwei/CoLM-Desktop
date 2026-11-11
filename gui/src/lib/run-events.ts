import type { RunState } from './types';

export function applyRunEvent(
  previous: RunState | undefined,
  event: string,
  payload: Record<string, unknown>,
): RunState | null {
  const caseDir =
    typeof payload.source_case === 'string'
      ? payload.source_case
      : typeof payload.case === 'string'
        ? payload.case
        : '';
  if (!caseDir) return null;
  const runId = typeof payload.run_id === 'string' ? payload.run_id : '';
  if (
    previous?.run_id &&
    runId &&
    previous.run_id !== runId &&
    (previous.running || event !== 'run://started')
  )
    return null;
  // A delayed start cannot reopen a run whose completion has already arrived.
  if (event === 'run://started' && previous && !previous.running && previous.run_id === runId)
    return null;
  const base =
    !previous || (runId && runId !== previous.run_id)
      ? { case: caseDir, status: 'running', running: true, logs: [], stages: {} }
      : previous;
  const next: RunState = {
    ...base,
    location:
      payload.location === 'remote'
        ? 'remote'
        : payload.location === 'local'
          ? 'local'
          : (base.location ?? 'local'),
    run_id: runId || previous?.run_id,
    operation_id:
      typeof payload.operation_id === 'string' ? payload.operation_id : base.operation_id,
    run_case: typeof payload.run_case === 'string' ? payload.run_case : base.run_case,
  };
  if (
    payload.execution_environment &&
    typeof payload.execution_environment === 'object' &&
    !Array.isArray(payload.execution_environment)
  )
    next.execution_environment = payload.execution_environment as Record<string, unknown>;
  if (
    next.running &&
    next.location === 'local' &&
    ['run://started', 'run://stage', 'run://progress'].includes(event)
  )
    next.status = 'running';
  if (event === 'run://lines' && Array.isArray(payload.lines))
    next.logs = [...next.logs, ...payload.lines.map(String)].slice(-3000);
  if (event === 'run://stage' && typeof payload.stage === 'string') {
    next.stages = { ...next.stages, [payload.stage]: String(payload.state) };
    next.progress = { ...next.progress, stage: payload.stage };
  }
  if (event === 'run://progress') next.progress = payload;
  if (event === 'run://done') {
    next.requested_stage =
      typeof payload.requested_stage === 'string' ? payload.requested_stage : null;
    next.running = false;
    next.status = payload.cancelled
      ? 'cancelled'
      : Number(payload.code) === 0
        ? 'succeeded'
        : 'failed';
    next.error =
      next.status === 'failed' ? (payload.reason ? String(payload.reason) : next.error) : undefined;
    // 本机运行结束时后端不再发进度事件：最后一个 `run://progress` 往往落在总数之前
    // （2026-10-31 实测 1536 步的短算例停在 1304/1536 = 84%），界面上的进度条就一直是
    // 未满格。成功结束把进度补到计划总数；失败/取消保留停下的位置，那是判断"在哪一步断掉"
    // 的唯一线索。远程进度的形状不同（嵌在 `progress.remote` 里），这里不动。
    const progress = next.progress;
    if (
      !payload.cancelled &&
      Number(payload.code) === 0 &&
      next.location !== 'remote' &&
      progress &&
      typeof progress.step === 'number' &&
      typeof progress.total_steps === 'number' &&
      progress.step < progress.total_steps
    )
      next.progress = { ...progress, step: progress.total_steps };
  }
  return next;
}
