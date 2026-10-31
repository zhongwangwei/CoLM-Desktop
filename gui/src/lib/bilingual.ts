/**
 * One message in the language the app is showing. Messages built outside React (validators,
 * IPC errors, setup rules) cannot see the language state, so this reads the `lang` that
 * `AppProvider` keeps on `<html>`. Without a document (tests, workers) it answers in Chinese.
 */
export function bilingual(zh: string, en: string): string {
  return globalThis.document?.documentElement?.lang === 'en' ? en : zh;
}

/**
 * 报错文字在显示出口（`Notice`、`notify`）统一处理：去掉 `String(error)` 带出的 "Error: " 前缀
 * （可能叠了好几层），再把已知的后端固定原文换成双语。页面里有上百处 `String(e)`，逐处改要给每个
 * 抽源码的页面测试补上下文，所以只在这一处做。
 */
export function plainError(text: string): string {
  return knownBackendError(text.replace(/^(?:Error: )+/, ''));
}

/**
 * 后端（`colm-cli`、校验器）按固定英文原文报的错，换成当前语言并给一条可执行的修复指引。
 *
 * 只认**逐字固定**的句子（来自 Rust 侧的 `ensure!`/`bail!`，改 Rust 文案时这里要同步），
 * 不认识的错误原样返回 —— 不能吞掉后端原文，否则诊断时少一半信息。为什么需要这一层：
 * 2026-10-31 实测，改过源码但没重跑 `stage-sidecar` 时，点"创建算例/开始运行"只会弹出
 * "official programs were not built from the current source"，中文界面里混着一句英文，
 * 且用户看不出该做什么。
 */
export function knownBackendError(text: string): string {
  if (text.includes('official programs were not built from the current source'))
    return bilingual(
      '官方程序与当前源码不一致：源码改动后要重新暂存 sidecar（cargo run -p xtask -- stage-sidecar），再重启应用。',
      'The official programs were not built from the current source: rerun `cargo run -p xtask -- stage-sidecar` and restart the app.',
    );
  if (text.includes('official program build/source stamp is missing'))
    return bilingual(
      '找不到官方程序的源码构建戳：安装包不完整，需要重新打包；从源码运行时先执行 cargo run -p xtask -- stage-sidecar。',
      'The official program build/source stamp is missing: rebuild this package, or rerun `cargo run -p xtask -- stage-sidecar` when running from source.',
    );
  if (text.includes('official program contents differ from their source build stamp'))
    return bilingual(
      '官方程序的内容与源码构建戳不一致：程序被替换过，需要重新暂存 sidecar 并重启应用。',
      'The official program contents differ from their source build stamp: restage the sidecars and restart the app.',
    );
  return text;
}
