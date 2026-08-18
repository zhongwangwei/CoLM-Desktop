//! 记住上次用过的目录。
//!
//! 界面上有五个要填绝对路径的框。第一次打开全空还情有可原，
//! **每次打开都要重打一遍就没人会用了** —— 而这是第一次真用最容易卡住的地方。

import { invoke } from './ipc.js';
import { $ } from './ui.js';

/** 会被记住的框。**只记目录与内核，不记算例名** ——
 *  那个每次都不一样，填回上次的反而误导。 */
const REMEMBERED = ['sitedir', 'root', 'kernel', 'obs'];

export async function restoreRecent() {
  let all = {};
  try { all = await invoke('load_recent'); } catch { /* 没有就算了，不值得打扰用户 */ }
  for (const id of REMEMBERED) {
    const el = $(id);
    if (!el || !all[id]) continue;
    // 下拉框（内核）要那一项确实还在才恢复 —— 内核目录可能已经被删了，
    // 硬塞一个不存在的值会让「运行」失败在一个用户看不懂的地方。
    if (el.tagName === 'SELECT') {
      if ([...el.options].some(o => o.value === all[id])) el.value = all[id];
    } else if (!el.value) {
      el.value = all[id];
    }
  }
  // 变了就记。用 change 而不是 input：每敲一个字符写一次文件太浪费，
  // 而这份东西丢了也不要紧。
  for (const id of REMEMBERED) {
    const el = $(id);
    if (!el) continue;
    el.addEventListener('change', () => {
      invoke('save_recent', { key: id, value: el.value }).catch(() => {});
    });
  }
}
