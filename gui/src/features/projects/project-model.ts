export const RECENT_PROJECTS_KEY = 'ui-project-roots';
/**
 * 读最近项目列表。坏了不抛错：从里面捞出还认得的路径，调用方照常回写成合法 JSON。原来抛错，回写也跟着失败，
 * 这个坏值就一直留着：最近项目永远是空的，后台整理模型版本也因读不懂这份列表而一直跳过（GUI 审查 A-02）。
 */
export function recentProjectRoots(value: string | undefined): string[] {
  if (!value) return [];
  let items: unknown[];
  try {
    const parsed: unknown = JSON.parse(value);
    items = Array.isArray(parsed) ? parsed : [];
  } catch {
    items = [...value.matchAll(/"([^"\n]+)"/g)].map((match) => match[1]);
  }
  return [
    ...new Set(
      items.filter(
        (item): item is string =>
          typeof item === 'string' && !!item.trim() && /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(item),
      ),
    ),
  ].slice(0, 8);
}
export function rememberProject(roots: readonly string[], root: string): string[] {
  return [root, ...roots.filter((item) => item !== root)].slice(0, 8);
}
/**
 * 最近项目的显示名。服务器项目在本机的目录是 `remote-projects/<哈希>`，名字（"tianhe: uuu"）只能在打开时
 * 从算例里取，所以记在这里；坏了就当没有，不影响打开。
 */
export const RECENT_PROJECT_NAMES_KEY = 'ui-project-names';
export function recentProjectNames(value: string | undefined): Record<string, string> {
  if (!value) return {};
  try {
    const parsed: unknown = JSON.parse(value);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    return Object.fromEntries(
      Object.entries(parsed).filter(
        (entry): entry is [string, string] => typeof entry[1] === 'string' && !!entry[1],
      ),
    );
  } catch {
    return {};
  }
}
/** 只留列表里还有的项目的名字，免得越记越多。 */
export function namesFor(roots: readonly string[], names: Record<string, string>) {
  return Object.fromEntries(
    roots.filter((root) => names[root]).map((root) => [root, names[root]!]),
  );
}
export function projectName(root: string) {
  return (
    root
      .replace(/[\\/]+$/, '')
      .split(/[\\/]/)
      .at(-1) || root
  );
}
