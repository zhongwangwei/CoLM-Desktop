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

/** 批量操作作用于谁：勾了就是勾中的，一个没勾就是全部。 */
export function batchTarget() {
  const picked = state.cases.filter(c => state.pickedCases.has(c.dir));
  return picked.length ? picked : state.cases;
}

/** 两个批量按钮上的字**就是它们会做的事**。
 *
 *  「勾了就作用于勾中的、没勾就作用于全部」这条规则本身没问题，
 *  但按钮上只写「全部运行」的话，那条规则就是**隐藏的** —— 用户勾了三个、
 *  按下去跑了九个，事后才知道。所以文字跟着勾选变。 */
export function updateCaseBatchButtons() {
  const picked = state.cases.filter(c => state.pickedCases.has(c.dir));
  const target = picked.length ? picked : state.cases;
  const suffix = picked.length ? `选中的 ${picked.length} 个` : `全部 ${state.cases.length} 个`;
  const run = $('runall');
  if (run) {
    run.textContent = `运行${suffix}`;
    run.disabled = !target.length;
  }
  const ev = $('eval-all');
  if (ev) {
    const done = target.filter(c => c.has_history).length;
    ev.textContent = picked.length ? `评估选中的 ${done} 个已跑算例` : `评估全部 ${done} 个已跑算例`;
    ev.disabled = !done;
  }
}
