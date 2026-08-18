//! 各模块共享的那点状态。
//!
//! 单独一个模块而不是挂在 `main.js` 上：`main.js` 要 import 其余模块，
//! 其余模块又要读状态 —— 放在 main 里就成了循环依赖。
//! ES module 的循环依赖不报错，只是让某个 import 在运行时变成 `undefined`，
//! 那种故障比编译错误难查得多。

export const state = {
  cases: [],
  kernels: [],
  selected: null,
  /** 当前算例 case.nml 的全文。改字段走后端往返，前端不自己拼。 */
  text: '',
  /** schema 全表，启动时取一次 */
  fields: [],
  group: 'nl_colm',
};
