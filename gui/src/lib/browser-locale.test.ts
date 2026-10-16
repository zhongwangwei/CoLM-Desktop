import test from 'node:test';
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { normalizeBrowserLocale } from './browser-locale';

test('POSIX and empty browser locales receive the selected application fallback', () => {
  for (const language of ['C', 'C.UTF-8', '']) {
    for (const english of [false, true]) {
      const browser = { language };
      normalizeBrowserLocale(browser, english);
      assert.equal(browser.language, english ? 'en-US' : 'zh-CN');
      assert.doesNotThrow(() => new Intl.NumberFormat(browser.language));
    }
  }
});

test('valid browser locales are retained without redefining the property', () => {
  for (const language of ['en-GB', 'zh-CN', 'de-DE', 'POSIX']) {
    const browser = Object.freeze({ language });
    normalizeBrowserLocale(browser);
    assert.equal(browser.language, language);
  }
});

test('the real chart dependency imports after normalizing WebKit C locale', () => {
  execFileSync(process.execPath, ['--import', 'tsx', '--input-type=module', '-e', `
    import assert from 'node:assert/strict';
    import { normalizeBrowserLocale } from './src/lib/browser-locale.ts';
    globalThis.window = new EventTarget();
    globalThis.document = {};
    globalThis.devicePixelRatio = 1;
    globalThis.matchMedia = () => new EventTarget();
    Object.defineProperty(globalThis, 'navigator', {value: {language: 'C'}, configurable: true});
    assert.throws(() => new Intl.NumberFormat(navigator.language), RangeError);
    normalizeBrowserLocale(navigator);
    const {default: uPlot} = await import('uplot');
    assert.equal(typeof uPlot, 'function');
  `], { cwd: new URL('../..', import.meta.url), stdio: 'pipe' });
});
