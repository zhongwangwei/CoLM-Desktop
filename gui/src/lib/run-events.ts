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
    location: payload.location === 'remote' ? 'remote' : 'local',
    run_id: runId || previous?.run_id,
    operation_id:
      typeof payload.operation_id === 'string' ? payload.operation_id : base.operation_id,
    run_case: typeof payload.run_case === 'string' ? payload.run_case : base.run_case,
  };
  if (event === 'run://lines' && Array.isArray(payload.lines))
    next.logs = [...next.logs, ...payload.lines.map(String)].slice(-3000);
  if (event === 'run://stage' && typeof payload.stage === 'string')
    next.stages = { ...next.stages, [payload.stage]: String(payload.state) };
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
    if (payload.reason) next.error = String(payload.reason);
  }
  return next;
}
