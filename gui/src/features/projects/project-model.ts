export const RECENT_PROJECTS_KEY = 'ui-project-roots';
export function recentProjectRoots(value: string | undefined): string[] {
  if (!value) return [];
  const parsed: unknown = JSON.parse(value);
  if (!Array.isArray(parsed)) throw new Error('Invalid recent project list');
  return [
    ...new Set(parsed.filter((item): item is string => typeof item === 'string' && !!item.trim())),
  ].slice(0, 8);
}
export function rememberProject(roots: readonly string[], root: string): string[] {
  return [root, ...roots.filter((item) => item !== root)].slice(0, 8);
}
export function projectName(root: string) {
  return (
    root
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .at(-1) || root
  );
}
