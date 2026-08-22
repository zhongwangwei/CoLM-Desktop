//! 批量操作的作用对象，以及运行/评估按钮的可用状态与文字。
//!
//! **单独一个模块，不是为了整齐。** 这几个函数被 `sites`（渲染列表时）、
//! `runner`（批量运行）与 `results`（批量评估）三处用到，
//! 放进其中任何一个都会立刻形成环：`sites` 已经 import 了 `results`，
//! 而 `results` 又要来问「批量作用于谁」。
//!
//! ES module 的循环依赖**不报错**，只让某个 import 在运行时变成 `undefined`
//! —— 那种故障比编译错误难查得多，前面拆模块时就为此单独立过 `state.js`。

import { state } from './state.js';
import { $ } from './ui.js';

/** root 中可以有很多历史算例；主界面只展示本次向导创建的这一批。 */
export function currentCases() {
  return state.cases.filter(c => state.createdCases.has(c.dir));
}

/** 不覆盖 root 里同名的旧算例；给新算例找一个稳定、可读的新名字。 */
export function freshCaseName(base, cases = state.cases) {
  const names = new Set(cases.map(c => c.name));
  if (!names.has(base)) return base;
  let n = 2;
  while (names.has(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

/** 参数改动作用于哪些算例目录。**默认是整批** —— 用户勾了 20 个站点是要配
 *  "这一次运行"，不是配其中第一个。
 *
 *  放在这里而不是 `params.js`：`timing.js` 也要问同一个问题，而 `params.js`
 *  已经 import 了 `timing.js` —— 反过来 import 就是一个环。
 *  ES module 的环不报错，只让某个 import 在运行时变成 `undefined`。
 *  实测：`check-gui` 当场抓到了这个环。 */
export function editTarget() {
  if (state.batch.length) return state.batch;
  return state.selected ? [state.selected.dir] : [];
}

/** 批量操作作用于谁：勾了就是勾中的；否则只用这次建/选出来的批次。
 *  不再默认扫整个 root —— 那里面常有上一次残留的自然站/旧算例。 */
export function batchTarget() {
  const current = currentCases();
  const picked = current.filter(c => state.pickedCases.has(c.dir));
  if (picked.length) return picked;
  if (state.batch.length) {
    const want = new Set(state.batch);
    return current.filter(c => want.has(c.dir));
  }
  return state.selected && state.createdCases.has(state.selected.dir) ? [state.selected] : [];
}

/** 找回一个新算例来自哪个站点。算例为避开旧目录可能改名成 `site-2`，
 * 因而不能拿算例名反查观测文件；建例时保存的 site_file -> case dir 才是主键。 */
export function sourceSite(c) {
  const siteFile = [...state.createdBySite].find(([, dir]) => dir === c.dir)?.[0];
  return state.sites.find(s => s.site_file === siteFile)
    ?? state.sites.find(s => s.name === c.name);
}

/** 运行按钮共用同一批目标；评估按钮把实际数量写在按钮上。 */
export function updateCaseBatchButtons() {
  const picked = currentCases().filter(c => state.pickedCases.has(c.dir));
  const target = batchTarget();
  for (const id of ['run-mksrfdata', 'run-mkinidata', 'run-colm', 'runall']) {
    const run = $(id);
    if (run) run.disabled = !target.length || state.runningCases.size > 0;
  }
  const ev = $('eval-all');
  if (ev) {
    const done = target.filter(c => c.has_history).length;
    ev.textContent = picked.length ? `评估选中的 ${done} 个已跑算例` : `评估本次 ${done} 个已跑算例`;
    ev.disabled = !done;
  }
}
