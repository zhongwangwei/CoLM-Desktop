import type { Route } from './types';
export type NavigationTarget = { route: Route; caseDir?: string; reload?: boolean };
export type NavigationGuard = () => string | null;
export function navigationBlock(
  current: NavigationTarget,
  target: NavigationTarget,
  guards: Iterable<NavigationGuard>,
): string | null {
  if (
    !target.reload &&
    current.route === target.route &&
    (target.caseDir === undefined || current.caseDir === target.caseDir)
  )
    return null;
  for (const guard of guards) {
    const reason = guard();
    if (reason) return reason;
  }
  return null;
}

/**
 * 关窗或退出前要提醒用户的事：各页登记的未保存编辑（同一条原因只说一次），以及正在跑的本机任务
 * （关窗会把它们一起停掉；远程作业在服务器上继续，不算）。空数组表示可以直接关（GUI 审查 G-01）。
 */
export function closeReasons(
  guards: Iterable<NavigationGuard>,
  running: Record<string, { running?: boolean; location?: string }>,
  localRunning: (count: number) => string,
): string[] {
  const reasons: string[] = [];
  for (const guard of guards) {
    const reason = guard();
    if (reason && !reasons.includes(reason)) reasons.push(reason);
  }
  const local = Object.values(running).filter(
    (run) => run.running && run.location !== 'remote',
  ).length;
  if (local) reasons.push(localRunning(local));
  return reasons;
}
