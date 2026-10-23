/**
 * One message in the language the app is showing. Messages built outside React (validators,
 * IPC errors, setup rules) cannot see the language state, so this reads the `lang` that
 * `AppProvider` keeps on `<html>`. Without a document (tests, workers) it answers in Chinese.
 */
export function bilingual(zh: string, en: string): string {
  return globalThis.document?.documentElement?.lang === 'en' ? en : zh;
}

/**
 * 报错文字去掉 `String(error)` 带出的 "Error: " 前缀（可能叠了好几层）。页面里有上百处 `String(e)`，
 * 逐处改要给每个抽源码的页面测试补上下文；所以在显示出口（`Notice`、`notify`）统一去掉。
 */
export function plainError(text: string): string {
  return text.replace(/^(?:Error: )+/, '');
}
