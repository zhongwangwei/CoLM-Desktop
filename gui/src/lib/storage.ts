/**
 * 本机偏好的读写（语言、主题、面板宽度……）。存储被禁用、配额满或 WKWebView 拒绝时只当没存过：
 * 原来 `main.tsx` 与 `app-state` 在挂载前直读，抛一次错整个窗口就停在"正在加载…"（GUI 审查 A-09）。
 */
export function readStored(key: string): string {
  try {
    return globalThis.localStorage?.getItem(key) ?? '';
  } catch {
    return '';
  }
}

/** 空字符串表示删掉这一项。存不下就只在这次会话里用。 */
export function writeStored(key: string, value: string) {
  try {
    if (value) globalThis.localStorage?.setItem(key, value);
    else globalThis.localStorage?.removeItem(key);
  } catch {
    /* 存不下就只在这次会话里用 */
  }
}
