//! 进门向导：这次要跑什么、从哪套配置开始。
//!
//! **不是欢迎页，是分流点。** 完整设计有六页（docs/design-gate.md）——
//! 空间结构、次网格方案、土壤水力、物理过程、调试，选定之后再进六步主
//! 界面。这一步只做前两页 + 状态机骨架：后面几页依赖一场正在进行的
//! CoLM 宏改造（把编译期宏变成运行时开关），那还没完成，现在的内核
//! 只支持 LULC_IGBP + vanGenuchten 这一套。等宏改造完成，把第 2 页的
//! 灰去掉、把状态机往后填三页就行 —— 这正是这一步的价值：机制先立起来。
//!
//! ## 两页
//!
//! 第 1 页：站点 / 区域 / 全球（原来那道一次性弹窗的内容，不变）。
//! 第 2 页：默认 / 碳氮循环 / 城市 / 从零开始 —— 除「默认」外全部置灰，
//! 灰选项写两件事：需要什么、是什么造成的（design-gate.md §2b）。
//!
//! **「默认」这个名字没问题，前提是预设的语义是「填好初值」而不是
//! 「跳过后面几页」。** LULC_IGBP + vanGenuchten 不是科学上更优的选择，
//! 但只要选完预设之后**仍然逐页走过第 3–6 页、每一层都能看能改**，
//! 「默认」就不是「正确答案」而只是「不知道从哪开始时的起点」——
//! 用户走完之后看到的是自己实际的配置，不是一个名字
//! （design-gate.md「预设是填好后面几页，不是跳过后面几页」一节）。
//! 这也是为什么每档下面列出对应的技术选项（`IGBP · van Genuchten`）：
//! 让懂的人一眼看出这一档等于什么。
//!
//! **第 3–6 页现在不存在**，所以选完预设、点下去，实际效果仍然是直接
//! 进主界面 —— 但下面的数据结构已经按「预设展开成一组初值，供第
//! 3–6 页落地后去读」的样子设计（见 `PRESET_VALUES` 与
//! `state.wizard`），不是等页面真的存在了再回来重写。
//!
//! 两页都不预选任何一档 —— 必须显式点一下才能「下一步」。
//!
//! ## 「下一步」与「直接开始」
//!
//! 第 2 页除了逐页前进的「下一步」，还有一个「直接开始」——
//! **接受当前已选的初值，跳过还没走的页，直接进主界面。** 想看一眼
//! 配置的人逐页走，赶时间的人一键进去，**两条路结果完全一样**
//! （design-gate.md「快速路径靠跳到最后」一节）。现在两个按钮做的事
//! 相同（因为第 2 页恰好是最后一页），语义不同：「下一步」是「往后
//! 翻一页」，「直接开始」是「跳到最后」；第 3–6 页落地后两者才会真正
//! 分岔。
//!
//! ## 状态机
//!
//! `pageIdx` + 本模块私有的 `picked` 是整套状态机的全部数据。**`picked`
//! 不进 `state`，也不进 `recent.js` 的 `REMEMBERED` 表** —— 关掉重开要
//! 重新问，design-gate.md §5：「不做记住上次的选择——它是分流点，不是
//! 一次性的欢迎页」。选择只有在按下「下一步」（最后一页）或「直接开始」
//! 时才落到 `state.domain` / `state.profile` / `state.wizard`，半路退出
//! （刷新页面重新起）不留痕迹。
//!
//! 下一步：保存本页选择，进下一页，没选时禁用。
//! 直接开始：接受当前选择，跳过其余页，没选时禁用（跟下一步同一条件）。
//! 上一步：回上一页，`picked` 原样保留，不清空。
//! 取消：`pageIdx` 归零、`picked` 清空，回第 1 页重来。
//!
//! **没实现 / 没接通的选项是 disabled，不是「点了报错」** —— 一个能点
//! 但必然失败的入口比一个灰着的更糟。
//!
//! **每次启动都弹，不记忆。** 它是分流点，不是一次性的欢迎页。
//!
//! 依赖方向只出不进：`main.js` import 它，它不被任何业务模块 import ——
//! `check-gui` 禁止模块成环，而 `sites ↔ results` 有前科。
//!
//! **区域/全球落地时要改的是 `shell.js` 的 `STEPS`。** 它现在是模块级的
//! const，被 `nextOf` / `go` / `renderSteps` 直接闭包引用 —— 三档各自一套
//! 步骤链的话，得把它变成 `STEPS[state.domain]` 或一个函数，那三个调用点
//! 都要跟着动。`state.domain` 现在**零读取点**，别以为它已经接好了 ——
//! `state.profile` / `state.wizard` 同理，落到 case.nml 是宏改造完成后
//! 的事（design-gate.md §3），这一步没做。
//!
//! 已经接好的那半：`renderNextButtons`（shell.js）遍历 `.page`，未知
//! `data-step` 的页 `nextOf` 返回 null、`.foot` 会被移掉，新域的页面
//! 加进来不会炸。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';

const DOMAINS = [
  { id: 'site',   t: '站点', d: 'PLUMBER2 / Urban-PLUMBER 单点模拟', ready: true },
  { id: 'region', t: '区域', d: '有限范围网格', ready: false },
  { id: 'global', t: '全球', d: '全球网格', ready: false },
];

// 第 2 页：从哪套配置开始。除「默认」外全部置灰 —— 不是因为它们不对，
// 是因为现在的内核只编进了 IGBP + vanGenuchten 这一套，别的组合选了会
// 在跑到一半才炸，而不是灰着更糟。`tech` 是每档对应的技术选项，展示在
// 用途描述下面一行；`need` 是灰选项要写的第一件事「需要什么」，第二件
// 事「是什么造成的」三档共用同一句 `PROFILE_BLOCKED_BY`，因为现在挡住
// 它们的是同一场宏改造，不像 design-gate.md 第 5 页那样各自指向前面
// 某一页的某个选择——那要等第 3–5 页真的存在才有「回哪页改」这回事。
const PROFILES = [
  {
    id: 'default', t: '默认', d: '地表能量与水分平衡', tech: 'IGBP · van Genuchten',
    ready: true,
  },
  {
    id: 'carbon_nitrogen', t: '碳氮循环', d: '植被生长与碳收支', tech: 'PFT · BGC · CROP',
    ready: false, need: '需要运行时的 LULC 开关',
  },
  {
    id: 'urban', t: '城市', d: '城市冠层与人为热', tech: 'IGBP · URBAN',
    ready: false, need: '需要运行时的 URBAN 开关',
  },
  {
    // 从零开始没有固定的技术组合（那正是它存在的理由），没有 `tech`
    // 这一行，`PRESET_VALUES` 里也没有它的条目。
    id: 'custom', t: '从零开始', d: '每一项都自己选', ready: false,
    need: '需要运行时开关支持逐项配置（次网格方案 · 土壤水力 · 物理过程）',
  },
];
const PROFILE_BLOCKED_BY = 'CoLM 宏改造尚未完成';

// 预设展开成的具体初值。**这是「预设 = 填好后面几页的初值」这件事的
// 数据形状** —— 第 3–6 页落地后从这里读起始值去预填那几页的控件，
// 用户在那几页看到的是这份值，不是「default」这个名字。字段名按
// design-gate.md §1 的依赖表起：`lulc`（第 3 页，次网格方案）、
// `soil`（第 4 页，土壤水力）、`bgc`/`crop`/`urban`/`tracer`
// （第 5 页，物理过程开关，被 lulc 与 soil 约束）。
//
// 现在只有 `default` 选得到，但表按将来的样子写全，免得第 3–6 页
// 落地时要回头重新设计这份数据结构。`custom`（从零开始）没有条目——
// 它的意思就是没有初值，用户在那几页自己一项项填。
const PRESET_VALUES = {
  default: {
    lulc: 'IGBP', soil: 'vanGenuchten',
    bgc: false, crop: false, urban: false, tracer: false,
  },
  carbon_nitrogen: {
    // PFT 是 BGC 的前提（§1：BGC 要 PFT 或 PC），CROP 要先开 BGC。
    lulc: 'PFT', soil: 'vanGenuchten',
    bgc: true, crop: true, urban: false, tracer: false,
  },
  urban: {
    lulc: 'IGBP', soil: 'vanGenuchten',
    bgc: false, crop: false, urban: true, tracer: false,
  },
};

/** 向导页顺序。加第 3 页时在这里插入，`render()`/`renderFoot()` 不用改
 *  —— 它们只认「当前是不是最后一页」，不认页数。 */
const PAGES = ['domain', 'profile'];

let pageIdx = 0;
/** 本次向导会话的暂存选择，见模块头部注释。 */
let picked = { domain: null, profile: null };

/** 立起门。后台初始化在它后面照常跑 —— 门只是视觉遮挡。 */
export function showDomainGate() {
  pageIdx = 0;
  picked = { domain: null, profile: null };
  render();
  $('domaingate').hidden = false;
}

function render() {
  const page = PAGES[pageIdx];
  $('gatetitle').textContent = page === 'domain' ? '这次要跑什么？' : '从哪套配置开始？';
  $('gatesub').textContent = page === 'domain'
    ? '现在只有站点能跑。区域与全球的步骤链还没有实现。'
    : '现在的内核只编进了「默认」这一套（IGBP · van Genuchten）—— '
      + '其余三档要等 CoLM 宏改造完成才能选。';
  // 第 2 页专有的一句：定义「预设」是什么意思。第 3–6 页现在不存在，
  // 这句话描述的是宏改造完成后的样子，不是这一刻的字面行为——但话要
  // 现在就写上，晚填页的人才不会把预设做成「跳过」。
  $('gateinfo').textContent = page === 'profile'
    ? 'ⓘ 选哪个都会走完后面四页，只是初值不同，随时可以改'
    : '';
  const box = $('gatecards');
  box.textContent = '';
  const items = page === 'domain' ? DOMAINS : PROFILES;
  const sel = page === 'domain' ? picked.domain : picked.profile;
  for (const it of items) box.appendChild(card(it, page, sel === it.id));
  renderFoot();
}

function card(it, page, isSelected) {
  const b = document.createElement('button');
  b.className = 'domain-card';
  b.disabled = !it.ready;
  b.setAttribute('aria-selected', String(isSelected));

  const t = document.createElement('span');
  t.className = 'dt';
  t.textContent = it.t;
  b.appendChild(t);

  const d = document.createElement('span');
  d.className = 'dd';
  d.textContent = it.d;
  b.appendChild(d);

  // 用途描述下面那一行技术选项：让懂的人一眼看出这一档等于哪几个
  // 开关，不懂的人看上面那行用途描述就够。从零开始没有固定组合，
  // 没有这一行（`it.tech` 未定义）。
  if (it.tech) {
    const tech = document.createElement('span');
    tech.className = 'dtech';
    tech.textContent = it.tech;
    b.appendChild(tech);
  }

  if (it.ready) {
    b.onclick = () => {
      if (page === 'domain') picked.domain = it.id; else picked.profile = it.id;
      render();
    };
  } else if (page === 'domain') {
    // 第 1 页原有行为，不动：两档单纯没实现，没有「需要什么/谁造成的」
    // 可写。
    const soon = document.createElement('span');
    soon.className = 'dsoon';
    soon.textContent = '暂不支持';
    b.appendChild(soon);
  } else {
    // 第 2 页灰选项写两件事（design-gate.md §2b）：① 需要什么 ② 是什么
    // 造成的。只写①的话，用户知道「碳氮循环要 PFT」但不知道为什么现在
    // 选不了；只写②的话，知道卡在宏改造但不知道等它做完之后这一档到底
    // 是什么。
    const need = document.createElement('span');
    need.className = 'dneed';
    need.textContent = it.need;
    b.appendChild(need);

    const why = document.createElement('span');
    why.className = 'dwhy';
    why.textContent = PROFILE_BLOCKED_BY;
    b.appendChild(why);
  }
  return b;
}

function renderFoot() {
  const foot = $('gatefoot');
  foot.textContent = '';

  // 第 1 页没有「上一步」「取消」可去 —— 它本身就是重来的终点。
  if (pageIdx > 0) {
    const cancel = document.createElement('button');
    cancel.className = 'btn-ghost';
    cancel.textContent = '取消';
    cancel.onclick = () => {
      pageIdx = 0;
      picked = { domain: null, profile: null };
      render();
    };
    foot.appendChild(cancel);

    const prev = document.createElement('button');
    prev.className = 'btn-ghost';
    prev.textContent = '← 上一步';
    // 只回页，不碰 `picked` —— 保留已选的正是判据②要的行为。
    prev.onclick = () => { pageIdx -= 1; render(); };
    foot.appendChild(prev);
  }

  const page = PAGES[pageIdx];
  const sel = page === 'domain' ? picked.domain : picked.profile;
  const isLast = pageIdx === PAGES.length - 1;

  // 「直接开始」只在第 2 页给 —— 第 1 页（域）还没有「初值」这回事
  // 可接受：预设本身是第 2 页才出现的概念。跟「下一步」同一个启用
  // 条件：没有选中项就没有「当前初值」可接受。
  if (page === 'profile') {
    const skip = document.createElement('button');
    skip.className = 'btn-ghost';
    skip.textContent = '直接开始 →';
    skip.disabled = !sel;
    skip.onclick = () => finish();
    foot.appendChild(skip);
  }

  const next = document.createElement('button');
  next.className = 'btn-next';
  next.textContent = isLast ? '进入主界面 →' : '下一步 →';
  // 没选时禁用，照 shell.js `renderNextButtons` 同一条规矩：
  // 进不去的时候不给一个能点但必然什么都不做的按钮。
  next.disabled = !sel;
  next.onclick = () => { if (isLast) finish(); else { pageIdx += 1; render(); } };
  foot.appendChild(next);
}

function finish() {
  state.domain = picked.domain;
  state.profile = picked.profile;
  // 预设展开成的初值——`custom`（从零开始）没有条目，落到 `null`，
  // 意思是「没有初值，等第 3–6 页落地后自己填」。浅拷贝一份，免得
  // 将来第 3–6 页改动 `state.wizard` 时顺手改到了 `PRESET_VALUES`
  // 这张共享表。
  const preset = PRESET_VALUES[picked.profile];
  state.wizard = preset ? { ...preset } : null;
  $('domaingate').hidden = true;
  go('prep');
}
