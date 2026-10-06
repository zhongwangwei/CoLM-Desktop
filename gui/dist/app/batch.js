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
import { $, baseName } from './ui.js';

/** root 中可以有很多历史算例；主界面只展示本次向导创建的这一批。 */
export function currentCases() {
  return state.cases.filter(c => state.createdCases.has(c.dir));
}

/** 用一次 `list_cases` 的结果更新算例表。本次会话已建或已打开、但不在这个根目录下的算例
 *  （中途换了根目录）要留着：否则它们从运行列表消失，而批量编辑仍会写到它们身上。 */
export function adoptCaseList(listed) {
  const seen = new Set(listed.map(c => c.dir));
  const kept = state.cases.filter(c => state.createdCases.has(c.dir) && !seen.has(c.dir));
  state.cases = [...listed, ...kept];
}

/** 不覆盖 root 里同名的旧算例；给新算例找一个稳定、可读的新名字。 */
export function freshCaseName(base, cases = state.cases) {
  // 名字与目录都要避开：算例目录名不一定等于 case.nml 里的算例名。
  const names = new Set(cases.flatMap(c => [c.name, String(c.dir ?? '').split(/[\\/]/).pop()]));
  if (!names.has(base)) return base;
  let n = 2;
  while (names.has(`${base}-${n}`)) n += 1;
  return `${base}-${n}`;
}

/** 批次级编辑项作用于哪些算例目录。预热、输出和网格设置默认写整批；
 *  逐站点基本设定与过程参数在 params.js 中另选站点或全部站点。
 *
 *  放在这里而不是 `params.js`：`timing.js` 也要问同一个问题，而 `params.js`
 *  已经 import 了 `timing.js` —— 反过来 import 就是一个环。
 *  ES module 的环不报错，只让某个 import 在运行时变成 `undefined`。
 *  实测：`check-gui` 当场抓到了这个环。 */
export function editTarget() {
  if (state.batch.length) return state.batch;
  return state.selected ? [state.selected.dir] : [];
}

/** 批量操作作用于谁：勾了就是勾中的；一个没勾就是本次会话的全部算例（运行页卡片上就是这么写的）。
 *  不扫整个 root —— 那里面常有上一次残留的自然站/旧算例。 */
export function batchTarget() {
  const current = currentCases();
  const picked = current.filter(c => state.pickedCases.has(c.dir));
  return picked.length ? picked : current;
}

/** 找回一个新算例来自哪个站点。算例为避开旧目录可能改名成 `site-2`，
 * 因而不能拿算例名反查观测文件；建例时保存的 site_file -> case dir 才是主键。 */
export function sourceSite(c) {
  const siteFile = [...state.createdBySite].find(([, dir]) => dir === c.dir)?.[0];
  return state.sites.find(s => s.site_file === siteFile)
    ?? state.sites.find(s => s.name === c.name);
}

/** 运行按钮共用同一批目标。结果工作台有独立分析范围，不在这里复用运行勾选。 */
export function updateCaseBatchButtons() {
  const target = batchTarget();
  for (const id of ['run-mksrfdata', 'run-mkinidata', 'run-colm', 'runall']) {
    const run = $(id);
    if (run) run.disabled = !target.length || state.runningCases.size > 0;
  }
}

/** 批量编辑时在卡片顶上写明这次改动会落到哪些算例；单个算例不写。
 *  预热、输出变量这些卡片只显示代表算例的值，不写明范围就会在事后才发现整批都被改了。 */
export function renderScope(box, dirs = editTarget()) {
  if (dirs.length < 2) return;
  const bar = document.createElement('div');
  bar.className = 'expert-note';
  bar.style.marginBottom = '10px';
  const names = dirs.map(baseName);
  bar.append('除逐站点数据文件外，下面的改动会写进 ');
  const count = document.createElement('b');
  count.textContent = `${dirs.length} 个算例`;
  bar.append(count, '：', names.slice(0, 6).join('、'));
  if (names.length > 6) bar.append(` 等 ${names.length} 个`);
  box.appendChild(bar);
}
