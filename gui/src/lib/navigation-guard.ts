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
