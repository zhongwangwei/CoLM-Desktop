/**
 * 单算例运行设置（`caseRunSettings`）的本地持久化辅助。
 *
 * 这些设置原来只活在 AppState 内存里：页面内重开能恢复，应用重启就丢（复核 C F5b）。
 * 存到 localStorage 时只接受对象形状、并限制条目数，避免路径失效/无限增长。
 * 做成纯函数便于单测；storage 读写由 app-state 负责。
 */
export const CASE_RUN_SETTINGS_KEY = 'colm-case-run-settings';
/** 最多保留多少个算例目录；旧的按插入顺序淘汰。 */
export const CASE_RUN_SETTINGS_LIMIT = 100;

export function parseCaseRunSettings(
  raw: string | null,
): Record<string, Record<string, unknown>> | undefined {
  if (!raw) return undefined;
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return undefined;
    const result: Record<string, Record<string, unknown>> = {};
    for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
      if (!key || !value || typeof value !== 'object' || Array.isArray(value)) continue;
      result[key] = value as Record<string, unknown>;
    }
    return Object.keys(result).length ? result : undefined;
  } catch {
    return undefined;
  }
}

export function serializeCaseRunSettings(
  settings: Record<string, Record<string, unknown>> | undefined,
): string {
  if (!settings) return '{}';
  const entries = Object.entries(settings).filter(
    ([key, value]) => key && value && typeof value === 'object' && !Array.isArray(value),
  );
  const kept = entries.slice(Math.max(0, entries.length - CASE_RUN_SETTINGS_LIMIT));
  return JSON.stringify(Object.fromEntries(kept));
}
