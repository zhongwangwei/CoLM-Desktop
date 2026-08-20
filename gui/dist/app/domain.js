//! 进门向导：这次要跑什么、这次研究什么过程。
//!
//! **不是欢迎页，是分流点。** 完整设计有六页（docs/design-gate.md）——
//! 空间结构、研究什么过程、次网格方案、土壤水力、其余物理开关、调试，
//! 选定之后再进六步主界面。这一步只做前两页 + 状态机骨架：后面几页
//! 依赖一场正在进行的 CoLM 宏改造（把编译期宏变成运行时开关），那还
//! 没完成，PC 与 USGS 还选不了（各自的理由见 SUBGRID 表里那两条
//! 注释 —— 一条是「还没跑过」，一条是「另一个工程」，不是同一回事）。
//! 价值：机制先立起来。
//!
//! ## 两页
//!
//! 第 1 页：站点 / 区域 / 全球（原来那道一次性弹窗的内容，不变）。
//! 第 2 页：默认 / 碳氮循环 / 城市 / 自定义 —— 除「默认」外全部置灰，
//! 灰选项写两件事：需要什么、是什么造成的（design-gate.md §2b）。
//!
//! **「默认」只管物理过程，不绑定次网格方案。** 它现在的意思是「不开
//! BGC/CROP/URBAN 这些附加过程」，不再声称自己用 IGBP 还是哪种土壤
//! 水力方案 —— 次网格方案（第 3 页）与土壤水力（第 4 页）是独立选的。
//!
//! **这条收窄背后的道理**（design-gate.md「为什么『研究什么过程』在
//! 『次网格方案』之前」一节）：约束是双向可用的——选了 IGBP 则 BGC
//! 不可用，选了 BGC 则 IGBP 不可用，同一条约束从哪头问都能表达。
//! 顺序该由「用户先想清楚哪个」决定，而用户先想清楚的是**要研究什么**
//! （水热、碳收支、城市热岛），不是**用哪种次网格**——研究碳循环的人
//! 知道自己要碳循环，未必立刻知道那需要 PFT 方案。所以「研究什么过程」
//! 排在「次网格方案」之前，「默认」也就只能管物理过程这一层，不能替
//! 用户预先决定次网格方案。
//!
//! **「默认」这个名字没问题，前提是预设的语义是「填好初值」而不是
//! 「跳过后面几页」。** 选完预设之后**仍然逐页走过第 3–6 页、每一层
//! 都能看能改**，「默认」就不是「正确答案」而只是「不知道从哪开始时
//! 的起点」——用户走完之后看到的是自己实际的配置，不是一个名字
//! （design-gate.md「预设是填好后面几页，不是跳过后面几页」一节）。
//! 这也是为什么每档下面列出它对应的那句说明：默认写「不开碳循环等
//! 附加过程」，碳氮循环/城市写它们各自打开的开关（`BGC · CROP` /
//! `URBAN`）。
//!
//! **第 3–6 页现在不存在**，所以选完预设、点下去，实际效果仍然是直接
//! 进主界面 —— 但下面的数据结构已经按「预设展开成一组初值，供第
//! 3–6 页落地后去读」的样子设计（见 `PRESET_VALUES` 与
//! `state.wizard`），不是等页面真的存在了再回来重写。**这份初值只覆盖
//! 物理过程开关**（`bgc`/`crop`/`urban`/`tracer`），不包含次网格方案
//! 或土壤水力 —— 那两项是第 3、4 页各自的字段，预设不替它们做主。
//!
//! 两页都不预选任何一档 —— 必须显式点一下才能「下一步」。
//!
//! ## 只有「上一步」与「下一步」
//!
//! **不给「直接开始」那种跳过剩余页的按钮。** 曾经加过，又去掉了：
//! 两个按钮做同一件事（只有两页时它们完全等价）只会让人停下来想
//! 「这两个有什么区别」，而**想清楚之后发现没区别** —— 纯粹的认知负担。
//!
//! 更要紧的是走完流程本身有价值：每一页都是一次「你要跑的是这个吗」
//! 的确认。跳过去的人省下几次点击，代价是**不知道自己在跑什么配置** ——
//! 而这个项目一路在防的正是这类「跑得完但不知道跑了什么」。
//!
//! `pageIdx` + 本模块私有的 `picked` 是整套状态机的全部数据。**`picked`
//! 不进 `state`，也不进 `recent.js` 的 `REMEMBERED` 表** —— 关掉重开要
//! 重新问，design-gate.md §5：「不做记住上次的选择——它是分流点，不是
//! 一次性的欢迎页」。选择只有在最后一页按下「进入主界面」
//! 时才落到 `state.domain` / `state.subgrid`，半路退出
//! （刷新页面重新起）不留痕迹。
//!
//! 下一步：保存本页选择，进下一页，没选时禁用。
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
//! `state.subgrid` 同理，落到 case.nml 是宏改造完成后
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

// 第 2 页：次网格方案。`tech` 说明方案，`need` 说明该选项为何暂不可用。
const SUBGRID = [
  {
    id: 'IGBP', t: 'IGBP', d: '17 类地表覆盖',
    tech: '一个 patch 一个地类', ready: true,
  },
  {
    id: 'USGS', t: 'USGS', d: '24 类地表覆盖（旧方案）',
    tech: '一个 patch 一个地类', ready: false,
    // 与 PC 那条的「等等就有」不同，**这是另一个工程**：
    // `N_land_classification` 是 Fortran `parameter`，24 类与 17 类的数组
    // 尺寸编译期就定死，不是把 `#ifdef` 换成 `IF` 能解决的。
    need: '数组尺寸由 N_land_classification 定死，要单独一轮数据结构改造',
  },
  {
    // 端到端验证过：CN-Cng 只改 case.nml 的 `DEF_USE_PFT=.true.`、不动
    // site.nc，mksrfdata→mkinidata→colm 整条跑完 11 天 528 步，LCT 与 PFT
    // 两份 restart 都写出来了。原以为还要一份 plant_15s 栅格，实际不用 ——
    // 那份 site.nc 里本来就带着逐 PFT 的数据。
    id: 'PFT', t: 'PFT', d: '植物功能型',
    tech: '一个 patch 拆成多个功能型', ready: true,
  },
  {
    // 宏已经改成运行时开关 `DEF_USE_PC`，与 PFT 同一批（`06543f8`）。
    // **但没有端到端跑通过的算例** —— 判据③只验了 PFT 那一路。
    // 「代码改完了」和「能用」是两回事，没跑过就不给选。
    id: 'PC', t: 'PC', d: '植物群落',
    tech: '同 PFT，次网格组织方式不同', ready: false,
    need: '已是运行时开关，但还没有端到端跑通的算例',
  },
];

const PAGES = ['domain', 'subgrid'];

let pageIdx = 0;
/** 本次向导会话的暂存选择，见模块头部注释。 */
let picked = { domain: null, subgrid: null };

/** 立起门。后台初始化在它后面照常跑 —— 门只是视觉遮挡。 */
export function showDomainGate() {
  pageIdx = 0;
  picked = { domain: null, subgrid: null };
  render();
  $('domaingate').hidden = false;
}

function render() {
  const page = PAGES[pageIdx];
  $('gatetitle').textContent = page === 'domain' ? '这次要跑什么？' : '次网格怎么分？';
  $('gatesub').textContent = page === 'domain'
    ? '现在只有站点能跑。区域与全球的步骤链还没有实现。'
    : 'IGBP 与 PFT 都跑通过；PC 的开关有了但还没有跑通的算例，'
      + 'USGS 要等数组尺寸那一轮改造。';
  // 第 2 页专有的一句：说清楚次网格方案、土壤水力不在这一页问。
  // 第 3–6 页现在不存在，这句话是给将来占位的，但要现在就写上。
  $('gateinfo').textContent = page === 'subgrid'
    ? 'ⓘ 次网格方案、土壤水力在后面几页选'
    : '';
  const box = $('gatecards');
  box.textContent = '';
  const items = page === 'domain' ? DOMAINS : SUBGRID;
  const sel = page === 'domain' ? picked.domain : picked.subgrid;
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

  // 用途描述下面那一行说明（design-gate.md §2b「①需要什么」）：
  // 「默认」说不开什么，「碳氮循环」「城市」说打开哪些开关，「自定义」
  // 说需要什么机制。所有卡片（不论是否置灰）都有这一行。
  if (it.tech) {
    const tech = document.createElement('span');
    tech.className = 'dtech';
    tech.textContent = it.tech;
    b.appendChild(tech);
  }

  if (it.ready) {
    b.onclick = () => {
      if (page === 'domain') picked.domain = it.id; else picked.subgrid = it.id;
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
    // 第 2 页灰选项还要说清楚是什么挡住了它。
    const why = document.createElement('span');
    why.className = 'dwhy';
    why.textContent = it.need;
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
      picked = { domain: null, subgrid: null };
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
  const sel = page === 'domain' ? picked.domain : picked.subgrid;
  const isLast = pageIdx === PAGES.length - 1;

  const next = document.createElement('button');
  next.className = 'btn-next';
  // **不分是不是最后一页，一律叫「下一步」。** 最后一页叫「进入主界面」
  // 看着更"贴心"，实际是多一种说法要人分辨 —— 而它做的事和前面每一页
  // 完全一样：把当前选择存下，往前走。名字变了会让人以为行为也变了。
  next.textContent = '下一步 →';
  // 没选时禁用，照 shell.js `renderNextButtons` 同一条规矩：
  // 进不去的时候不给一个能点但必然什么都不做的按钮。
  next.disabled = !sel;
  next.onclick = () => { if (isLast) finish(); else { pageIdx += 1; render(); } };
  foot.appendChild(next);
}

function finish() {
  state.domain = picked.domain;
  state.subgrid = picked.subgrid;
  $('domaingate').hidden = true;
  go('prep');
}
