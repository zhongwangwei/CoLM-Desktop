//! 不值得单独成模块的小东西。

export const $ = id => document.getElementById(id);

/** 状态栏。出错与「已保存」都走这里，免得两种消息各写各的。
 *
 *  直接写 DOM 而不是 import shell.js —— shell 要 import state，
 *  而几乎每个模块都 import ui，绕一圈就成了循环依赖。 */
export function status(msg) {
  $('status').textContent = String(msg);
}


/** 路径的最后一段。
 *
 *  **两种分隔符都要认。** Windows 上算例目录是 `C:\Users\…\CN-Cng`，
 *  只按 `/` 切会原样返回整条路径 —— 于是横幅上写的不是「CN-Cng」而是
 *  一长串绝对路径，把那一行挤没。
 */
export function baseName(p) {
  return String(p).replace(/[\\/]+$/, '').split(/[\\/]/).pop();
}

/** 拼一段路径。分隔符跟着**已有那部分**走，两边都不认识平台。
 *
 *  Windows 的 API 认正斜杠，所以拼错了多半也能跑；但写进 namelist 的
 *  路径会被交给 `cmd`，而它在未加引号的参数里把 `/` 当开关前缀 ——
 *  这个坑刚在内核那边踩过一次。
 */
export function joinPath(dir, name) {
  const d = String(dir).replace(/[\\/]+$/, '');
  return d + (d.includes('\\') && !d.includes('/') ? '\\' : '/') + name;
}
