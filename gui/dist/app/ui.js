//! 不值得单独成模块的小东西。

export const $ = id => document.getElementById(id);

/** 状态栏。出错与「已保存」都走这里，免得两种消息各写各的。
 *
 *  直接写 DOM 而不是 import shell.js —— shell 要 import state，
 *  而几乎每个模块都 import ui，绕一圈就成了循环依赖。 */
export function status(msg) {
  $('status').textContent = String(msg);
}
