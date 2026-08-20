//! 当前选中的内核，以及向导是否打开城市过程。
//!
//! **单独一个模块，不是为了整齐。** `runner.js`（切内核时更新界面）与
//! `sites.js`（标出站点配不配得上内核）都要问同一个问题，而
//! `runner.js` 已经 import 了 `sites.js` —— 反过来 import 就是一个环。
//! ES module 的环**不报错**，只让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。`batch.js` 当初正是为同样的理由立的。
//!
//! 判据取 `generator_args` 而不是目录名或 preset 名：**目录名不是身份**，
//! 「这个内核到底编没编 URBAN」只有那一行宏组合说了算。

import { state } from './state.js';
import { $ } from './ui.js';

/** 下拉框现在选中的那个内核条目，没选中就是 `null`。 */
export function currentKernel() {
  return state.kernels.find(k => k.dir === $('kernel')?.value) ?? null;
}

/** URBAN 已是运行时开关，站点匹配必须看向导配置而不是旧内核名。 */
export function urbanEnabled() {
  return !!state.wizard?.physics.urban;
}
