/**
 * 项目级「批量运行」交接设置（`projectRunSettings`）的本地持久化。
 *
 * 原来只活在本次会话内存里：应用重启后从项目页进批量运行会回到默认值，而单算例设置已经
 * 跨重启保存（GUI 审查 F-R-07）。`ProjectBatchRun` 只用 `root === state.root` 的交接设置，
 * 所以多项目之间不会串用。
 */
export const PROJECT_RUN_SETTINGS_KEY = 'colm-project-run-settings';
/** cases 列表上限：一个项目最多 1000 个算例，超出视为损坏。 */
export const PROJECT_RUN_SETTINGS_CASE_LIMIT = 1000;

export function parseProjectRunSettings(raw: string | null): Record<string, unknown> | undefined {
  if (!raw) return undefined;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return undefined;
    const value = parsed as Record<string, unknown>;
    if (typeof value.root !== 'string' || !value.root) return undefined;
    if (!Array.isArray(value.cases) || value.cases.length > PROJECT_RUN_SETTINGS_CASE_LIMIT)
      return undefined;
    if (!value.cases.every((entry) => typeof entry === 'string' && entry)) return undefined;
    return value;
  } catch {
    return undefined;
  }
}

export function serializeProjectRunSettings(settings: unknown): string {
  if (!settings || typeof settings !== 'object' || Array.isArray(settings)) return '{}';
  return JSON.stringify(settings);
}
