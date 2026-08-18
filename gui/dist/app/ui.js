//! 不值得单独成模块的小东西。

export const $ = id => document.getElementById(id);

/** 状态栏。出错与「已保存」都走这里，免得两种消息各写各的。 */
export function status(msg) {
  $('status').textContent = String(msg);
}
