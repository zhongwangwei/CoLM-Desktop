/**
 * One message in the language the app is showing. Messages built outside React (validators,
 * IPC errors, setup rules) cannot see the language state, so this reads the `lang` that
 * `AppProvider` keeps on `<html>`. Without a document (tests, workers) it answers in Chinese.
 */
export function bilingual(zh: string, en: string): string {
  return globalThis.document?.documentElement?.lang === 'en' ? en : zh;
}
