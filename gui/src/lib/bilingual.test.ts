import test from 'node:test';
import assert from 'node:assert/strict';
import { knownBackendError, plainError } from './bilingual';

/**
 * 后端固定英文原文的双语化：2026-10-31 实测改过源码但没重跑 `stage-sidecar` 时，
 * 中文界面点"开始运行"只弹一句英文，用户看不出要重暂存。这里守住"换成当前语言 +
 * 带修复指引"和"不认识的错误原样保留"两条。
 */
function withLanguage(lang: 'en' | 'zh', run: () => void) {
  const previous = globalThis.document;
  Object.defineProperty(globalThis, 'document', {
    value: { documentElement: { lang } },
    configurable: true,
    writable: true,
  });
  try {
    run();
  } finally {
    Object.defineProperty(globalThis, 'document', {
      value: previous,
      configurable: true,
      writable: true,
    });
  }
}

test('known build-stamp errors get the current language and an actionable hint', () => {
  const raw =
    'official programs were not built from the current source; rebuild before freezing this model';
  withLanguage('zh', () => {
    const message = knownBackendError(raw);
    assert.match(message, /官方程序与当前源码不一致/);
    assert.match(message, /stage-sidecar/);
    // 去掉 "Error: " 前缀后同样命中，否则 notify 里的原文会漏掉。
    assert.match(plainError(`Error: ${raw}`), /重新暂存 sidecar/);
  });
  withLanguage('en', () => {
    const message = knownBackendError(raw);
    assert.match(message, /not built from the current source/);
    assert.match(message, /stage-sidecar/);
  });
  assert.match(
    knownBackendError('official program build/source stamp is missing; rebuild this package'),
    /源码构建戳|build\/source stamp/,
  );
  // 不认识的错误原样返回：不能把后端原文吞掉。
  assert.equal(knownBackendError('some other backend failure'), 'some other backend failure');
  assert.equal(plainError('Error: plain failure'), 'plain failure');
});
