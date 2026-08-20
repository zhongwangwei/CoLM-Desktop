//! 进门向导。顺序按约束方向排：空间 → 次网格 → 土壤 → 物理 → 调试。
//!
//! 回退保留无关选择，改上游时只清掉已经失效的下游值。USGS、CROP
//! 仍是编译期方案，PC 尚未端到端跑通，因此都
//! 列出来但不可选，不能给一个点了之后必然失败的入口。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';

const DOMAINS = [
  { id: 'site', t: '站点', d: 'PLUMBER2 / Urban-PLUMBER 单点模拟', ready: true },
  { id: 'region', t: '区域', d: '有限范围网格', ready: false, need: '区域步骤链尚未实现' },
  { id: 'global', t: '全球', d: '全球网格', ready: false, need: '全球步骤链尚未实现' },
];

const SUBGRIDS = [
  {
    id: 'USGS', t: 'USGS', d: '24 类地表覆盖（旧方案）', tech: '一个 patch 一个地类',
    ready: false, need: '数组尺寸由 N_land_classification 编译期定死，需要 USGS 内核',
  },
  { id: 'IGBP', t: 'IGBP', d: '17 类地表覆盖', tech: '一个 patch 一个地类', ready: true },
  { id: 'PFT', t: 'PFT', d: '植物功能型', tech: '一个 patch 拆成多个功能型', ready: true },
  {
    id: 'PC', t: 'PC', d: '植物群落', tech: '同 PFT，次网格组织方式不同',
    ready: false, need: '运行时开关已有，但还没有端到端跑通的算例',
  },
];

const SOILS = [
  { id: 'vg', t: 'van Genuchten–Mualem', d: '默认土壤水力方案', tech: '支持 TRACER' },
  { id: 'campbell', t: 'Campbell', d: 'Campbell 土壤水力', tech: '不需要 alpha_vgm / n_vgm / theta_r' },
];

const PHYSICS = [
  { id: 'urban', t: 'URBAN', d: '城市冠层与人为热' },
  { id: 'lulcc', t: 'LULCC', d: '土地利用变化' },
  { id: 'bgc', t: 'BGC', d: '碳氮循环' },
  {
    id: 'crop', t: 'CROP', d: '作物模型',
    ready: false, need: 'CROP 仍决定数组尺寸，需要 CROP-enabled 内核；同时需要 BGC',
  },
  { id: 'tracer', t: 'TRACER', d: '同位素 / 溶质示踪' },
];

const DEBUG = [
  { id: 'rangecheck', t: 'RangeCheck', d: '逐变量范围检查' },
  { id: 'colmdebug', t: 'CoLMDEBUG', d: '详细诊断输出' },
  { id: 'srfdatadiag', t: 'SrfdataDiag', d: '地表数据诊断' },
];

const PAGES = ['domain', 'subgrid', 'soil', 'physics', 'debug'];

const emptyPhysics = () => ({ urban: false, lulcc: false, bgc: false, crop: false, tracer: false });
const emptyDebug = () => ({ rangecheck: false, colmdebug: false, srfdatadiag: false });
const emptyPicked = () => ({
  domain: null,
  subgrid: null,
  soil: null,
  physics: emptyPhysics(),
  debug: emptyDebug(),
});

let pageIdx = 0;
let picked = emptyPicked();

export function showDomainGate() {
  pageIdx = 0;
  picked = emptyPicked();
  render();
  $('domaingate').hidden = false;
}

function render() {
  const page = PAGES[pageIdx];
  const copy = {
    domain: ['这次要跑什么？', '空间结构先定；现在只有站点步骤链能跑。'],
    subgrid: ['次网格怎么分？', '次网格方案决定 BGC 是否可用，也决定站点数据要求。'],
    soil: ['土壤水力用哪套？', '土壤方案决定 TRACER 是否可用，也改变站点数据要求。'],
    physics: ['还要打开哪些过程？', '可多选；被上游约束挡住的项会说明回哪一页修改。'],
    debug: ['要打开调试吗？', '可全部不选；这些开关只增加检查与日志，不改变页间约束。'],
  }[page];
  $('gatetitle').textContent = copy[0];
  $('gatesub').textContent = `第 ${pageIdx + 1}/${PAGES.length} 页 · ${copy[1]}`;
  $('gateinfo').textContent = pageInfo(page);

  const box = $('gatecards');
  box.textContent = '';
  if (page === 'domain') renderCards(DOMAINS, picked.domain, chooseDomain);
  if (page === 'subgrid') renderCards(SUBGRIDS, picked.subgrid, chooseSubgrid);
  if (page === 'soil') renderCards(SOILS, picked.soil, chooseSoil);
  if (page === 'physics') renderCards(PHYSICS, picked.physics, togglePhysics, physicsBlock, true);
  if (page === 'debug') renderCards(DEBUG, picked.debug, toggleDebug, null, true);
  renderFoot();
}

function pageInfo(page) {
  if (page === 'subgrid') {
    if (picked.subgrid === 'PFT' || picked.subgrid === 'PC') {
      return 'ⓘ 站点文件最好提供 pfttyp 与 pctpfts；缺少时会回落到 rawdata/plant_15s';
    }
    return picked.subgrid === 'IGBP' ? 'ⓘ 站点数据使用 IGBP_classification' : 'ⓘ 必须选择一种次网格方案';
  }
  if (page === 'soil') {
    return picked.soil === 'campbell'
      ? 'ⓘ Campbell 不需要 alpha_vgm / n_vgm / theta_r，但不能使用 TRACER'
      : 'ⓘ van Genuchten 需要 alpha_vgm / n_vgm / theta_r，并支持 TRACER';
  }
  if (page === 'physics') return 'ⓘ 灰项仍然列出；带“← 第 N 页”的卡片可直接返回修改';
  if (page === 'debug') return 'ⓘ 打开调试会让日志明显增多，常规运行可全部关闭';
  return '';
}

function renderCards(items, selected, choose, blocker = null, multi = false) {
  const box = $('gatecards');
  for (const item of items) {
    const on = multi ? !!selected[item.id] : selected === item.id;
    box.appendChild(card(item, on, choose, blocker?.(item), multi));
  }
}

function card(item, selected, choose, blocked, multi) {
  const b = document.createElement('button');
  b.type = 'button';
  b.className = 'domain-card';
  b.setAttribute(multi ? 'aria-pressed' : 'aria-selected', String(selected));

  const title = document.createElement('span');
  title.className = 'dt';
  title.textContent = item.t;
  b.appendChild(title);

  const desc = document.createElement('span');
  desc.className = 'dd';
  desc.textContent = item.d;
  b.appendChild(desc);

  if (item.tech) {
    const tech = document.createElement('span');
    tech.className = 'dtech';
    tech.textContent = item.tech;
    b.appendChild(tech);
  }

  blocked ??= item.ready === false ? { need: item.need } : null;
  if (!blocked) {
    b.onclick = () => choose(item.id);
    return b;
  }

  b.className += ' disabled';
  b.setAttribute('aria-disabled', 'true');
  const why = document.createElement('span');
  why.className = 'dwhy';
  why.textContent = blocked.need + (blocked.cause ? `  ← ${blocked.cause}` : '');
  b.appendChild(why);
  if (blocked.page == null) b.disabled = true;
  else b.onclick = () => { pageIdx = blocked.page; render(); };
  return b;
}

function chooseDomain(id) {
  picked.domain = id;
  render();
}

function chooseSubgrid(id) {
  picked.subgrid = id;
  if (id !== 'PFT' && id !== 'PC') {
    picked.physics.bgc = false;
    picked.physics.crop = false;
  }
  render();
}

function chooseSoil(id) {
  picked.soil = id;
  if (id === 'campbell') picked.physics.tracer = false;
  render();
}

function togglePhysics(id) {
  picked.physics[id] = !picked.physics[id];
  if (id === 'bgc' && !picked.physics.bgc) picked.physics.crop = false;
  render();
}

function toggleDebug(id) {
  picked.debug[id] = !picked.debug[id];
  render();
}

function physicsBlock(item) {
  if (item.ready === false) return null;
  if (item.id === 'bgc' && picked.subgrid !== 'PFT' && picked.subgrid !== 'PC') {
    return { need: '需要 PFT 或 PC 次网格', cause: `第 2 页选了 ${picked.subgrid}`, page: 1 };
  }
  if (item.id === 'tracer' && picked.soil !== 'vg') {
    return { need: '需要 van Genuchten 土壤水力', cause: '第 3 页选了 Campbell', page: 2 };
  }
  return null;
}

function renderFoot() {
  const foot = $('gatefoot');
  foot.textContent = '';
  if (pageIdx > 0) {
    const cancel = document.createElement('button');
    cancel.className = 'btn-ghost';
    cancel.textContent = '取消';
    cancel.onclick = () => { pageIdx = 0; picked = emptyPicked(); render(); };
    foot.appendChild(cancel);

    const prev = document.createElement('button');
    prev.className = 'btn-ghost';
    prev.textContent = '← 上一步';
    prev.onclick = () => { pageIdx -= 1; render(); };
    foot.appendChild(prev);
  }

  const page = PAGES[pageIdx];
  const required = { domain: picked.domain, subgrid: picked.subgrid, soil: picked.soil };
  const next = document.createElement('button');
  next.className = 'btn-next';
  next.textContent = '下一步 →';
  next.disabled = Object.hasOwn(required, page) && !required[page];
  next.onclick = () => {
    if (pageIdx === PAGES.length - 1) finish();
    else { pageIdx += 1; render(); }
  };
  foot.appendChild(next);
}

function finish() {
  state.domain = picked.domain;
  state.subgrid = picked.subgrid;
  state.wizard = {
    subgrid: picked.subgrid,
    soil: picked.soil,
    physics: { ...picked.physics },
    debug: { ...picked.debug },
  };
  globalThis.dispatchEvent?.(new Event('colm:wizard'));
  $('domaingate').hidden = true;
  go('prep');
}

const logical = value => value ? '.true.' : '.false.';

/** 新建算例时一次写入的运行时初值。编译期的 USGS / CROP 不在这里伪装成可写。 */
export function wizardFields(wizard = state.wizard) {
  if (!wizard) return [];
  const p = wizard.physics;
  const d = wizard.debug;
  return [
    ['DEF_USE_LCT', wizard.subgrid === 'IGBP'],
    ['DEF_USE_PFT', wizard.subgrid === 'PFT'],
    ['DEF_USE_PC', wizard.subgrid === 'PC'],
    ['DEF_USE_Campbell_SOIL_MODEL', wizard.soil === 'campbell'],
    ['DEF_USE_BGC', p.bgc],
    ['DEF_URBAN_RUN', p.urban],
    ['DEF_USE_LULCC', p.lulcc],
    ['DEF_USE_TRACER', p.tracer],
    ['DEF_USE_RangeCheck', d.rangecheck],
    ['DEF_USE_CoLMDEBUG', d.colmdebug],
    ['DEF_USE_SrfdataDiag', d.srfdatadiag],
  ].map(([path, value]) => ({ path, value: logical(value) }));
}
