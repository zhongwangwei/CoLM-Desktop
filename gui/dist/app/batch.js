//! 批量操作的作用对象，以及两个批量按钮的文字。
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
  const picked = state.cases.filter(c => state.pickedCases.has(c.dir));
  if (picked.length) return picked;
  if (state.batch.length) {
    const want = new Set(state.batch);
    return state.cases.filter(c => want.has(c.dir));
  }
  return state.selected ? [state.selected] : [];
}

/** 两个批量按钮上的字**就是它们会做的事**。
 *
 *  「勾了就作用于勾中的、没勾就作用于全部」这条规则本身没问题，
 *  但按钮上只写「全部运行」的话，那条规则就是**隐藏的** —— 用户勾了三个、
 *  按下去跑了九个，事后才知道。所以文字跟着勾选变。 */
export function updateCaseBatchButtons() {
  const picked = state.cases.filter(c => state.pickedCases.has(c.dir));
  const target = batchTarget();
  const suffix = picked.length ? `选中的 ${picked.length} 个` : `本次 ${target.length} 个`;
  const run = $('runall');
  if (run) {
    run.textContent = `运行${suffix}`;
    run.disabled = !target.length;
  }
  const ev = $('eval-all');
  if (ev) {
    const done = target.filter(c => c.has_history).length;
    ev.textContent = picked.length ? `评估选中的 ${done} 个已跑算例` : `评估本次 ${done} 个已跑算例`;
    ev.disabled = !done;
  }
}
