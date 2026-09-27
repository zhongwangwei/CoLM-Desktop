//! 运行页“模拟引擎”下拉框的取值。单算例、批量与 Study 三条运行路径共用。
//!
//! 独立成模块而不放进 runner.js：results.js 也要读它，而 results.js 已被
//! runner.js 间接导入，放在 runner.js 会成环（xtask check-gui 拦下过）。

import { $ } from './ui.js';

/** 未知或缺失的取值一律当作 rust —— 与 `colm-cli --engine` 的默认一致。 */
export function modelEngine() {
  return $('model-engine')?.value === 'fortran' ? 'fortran' : 'rust';
}
