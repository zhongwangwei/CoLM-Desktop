/** WebKit can expose POSIX locales such as "C" as navigator.language on Linux.
 * Chart libraries pass that value directly to Intl while their module loads. */
export function normalizeBrowserLocale(
  browser: Pick<Navigator, 'language'>,
  english = false,
): void {
  try {
    if (browser.language) {
      new Intl.NumberFormat(browser.language);
      return;
    }
  } catch (error) {
    if (!(error instanceof RangeError)) throw error;
  }
  Object.defineProperty(browser, 'language', {
    configurable: true,
    value: english ? 'en-US' : 'zh-CN',
  });
}
