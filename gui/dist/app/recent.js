//! 记住上次用过的目录。
//!
//! 界面上有五个要填绝对路径的框。第一次打开全空还情有可原，
//! **每次打开都要重打一遍就没人会用了** —— 而这是第一次真用最容易卡住的地方。

import { invoke } from './ipc.js';
import { $ } from './ui.js';

/** 会被记住的框。**只记目录，不记算例名** ——
 *  那个每次都不一样，填回上次的反而误导。 */
// **加新字段时要同时改这里。** `wirePickers()` 会把任何带 `data-for`
// 的选择结果写进 `recent.json`，但**恢复只认这张表** —— 漏了的字段
// 表现是「选过的东西下次打开没了」，而旁边的字段都还在，看着像随机失灵。
// `fsrc`（前处理页的强迫场文件）就漏过一次，真机验收才发现。
const REMEMBERED = [
  'sitedir', 'root', 'obs',
  'fsrc', 'forcingdir',  // 前处理源文件，以及建算例时用于匹配的强迫场目录
  'cpu-workers',         // 批量运行同时占用的 CPU 核数
  'soutdir', 'srawdata', // 前处理：站点属性的产物目录与 rawdata
  'rawdata', 'runtime',  // 建例：自然/城市外部数据与城市运行时数据
  // **经纬度不记。** 那是每个站点各不相同的东西，恢复上一个站点的坐标
  // 只会让人在没注意的情况下建出一份位置错的站点文件 —— 而位置错的
  // 站点文件跑得完，结果全错。目录不同：它是「我把东西放哪」，
  // 换个站点也不会变。
];

export async function restoreRecent() {
  let all = {};
  try { all = await invoke('load_recent'); } catch { /* 没有就算了，不值得打扰用户 */ }
  for (const id of REMEMBERED) {
    const el = $(id);
    if (!el || !all[id]) continue;
    if (!el.value) el.value = all[id];
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

/** 「选择…」按钮。**路径不该让人手打** —— 一个绝对路径打错一个字符，
 *  报错会出现在完全无关的地方（CoLM 读不到某个 `.nc`），而那时人已经
 *  等了一次运行。
 *
 *  输入框保留：命名不合约定的数据、或者从别处复制来的路径仍要能贴进去。
 *  选择器是**更容易的那条路**，不是唯一的路。 */
export function wirePickers() {
  for (const b of document.querySelectorAll('button.pick')) {
    const id = b.dataset.for;
    b.onclick = async () => {
      const el = $(id);
      let picked = null;
      try {
        // **写成字面量，不用变量拼命令名。** check-gui 靠扫 invoke 后面那个
        // 字符串字面量来核对前后端接口，拼出来的名字它看不见 —— 那时这两个
        // 命令就成了守卫的盲区（实测：26 注册 / 23 调用）。
        picked = b.dataset.file
          ? await invoke('pick_file', { key: id, filter: b.dataset.file })
          : await invoke('pick_folder', { key: id });
      } catch { /* 选择器起不来就让人手打 */ }
      // null 是**取消**，不是失败 —— 不该清空已有的值，也不该报错。
      if (!picked) return;
      el.value = picked;
      el.dispatchEvent(new Event('change'));
    };
  }
}
