/** What `remote_engine_progress` reports (`colm_remote::engine::BuildProgress`). */
export type BuildProgress = {
  engine?: string | null;
  stage?: string | null;
  done?: number | null;
  total?: number | null;
  current?: string | null;
};

/**
 * Which step the engine preparation is at. The last four compilation units are the four engine
 * programs: with LTO and one codegen unit they take most of the time (several minutes on land02 and 天河), so the
 * bar would otherwise sit at 99% without saying why.
 */
export function engineStep(phase: 'checking' | 'building', progress?: BuildProgress | null) {
  if (phase === 'checking') return 0;
  if (!progress?.engine) return 1;
  if (progress.stage?.startsWith('installing')) return 2;
  if (progress.done != null && progress.total) return progress.total - progress.done <= 4 ? 4 : 3;
  return 3;
}
