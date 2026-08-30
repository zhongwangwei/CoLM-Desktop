//! 进门向导。顺序按约束方向排：空间 → 计算网格 → 次网格 → 土壤 → 物理 → 示踪剂 → 调试。
//!
//! 回退保留无关选择，改上游时只清掉已经失效的下游值。USGS 由向导
//! 自动匹配专用编译产物；CROP 需要 CROP-enabled 内核。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';
import { kernelForSubgrid } from './kernel.js';

const DOMAINS = [
  { id: 'site', t: '站点', d: '单点站点模拟', ready: true },
  { id: 'watershed', t: '流域', d: '按流域边界限定模拟范围' },
  { id: 'region', t: '区域', d: '按经纬度范围限定模拟区域' },
  { id: 'global', t: '全球', d: '覆盖全球陆地区域' },
];

const GRIDS = [
  { id: 'latlon', t: '经纬度网格', d: '规则等经纬度网格（GRIDBASED）' },
  { id: 'unstructured', t: '非结构网格', d: '由 elmindex 描述计算单元（UNSTRUCTURED）' },
  { id: 'catchment', t: '流域网格', d: '集水区与 HRU 水文单元（CATCHMENT）' },
];

const SUBGRIDS = [
  { id: 'USGS', t: 'USGS', d: '24 类地表覆盖' },
  { id: 'IGBP', t: 'IGBP', d: '17 类地表覆盖', ready: true },
  { id: 'PFT', t: 'PFT', d: '植物功能型', ready: true },
  { id: 'PC', t: 'PC', d: '植物群落' },
];

const SOILS = [
  { id: 'vg', t: 'van Genuchten–Mualem（Ippisch 2006）', d: '启用 van Genuchten–Mualem 土壤水力模型' },
  { id: 'campbell', t: 'Campbell（1974）', d: '启用 Campbell 土壤水力模型' },
];

const PHYSICS = [
  { id: 'urban', t: 'URBAN', d: '城市冠层与人为热；不锁定次网格方案' },
  { id: 'lulcc', t: 'LULCC', d: '土地利用变化' },
  { id: 'bgc', t: 'BGC', d: '碳氮循环' },
  { id: 'crop', t: 'CROP', d: '作物模型' },
  { id: 'tracer', t: 'TRACER', d: '同位素 / 溶质 / 气体 / 颗粒示踪；当前仅开放甲烷' },
];

const DEBUG = [
  { id: 'rangecheck', t: 'RangeCheck', d: '逐变量范围检查' },
  { id: 'colmdebug', t: 'CoLMDEBUG', d: '详细诊断输出' },
  {
    id: 'srfdatadiag', t: 'SrfdataDiag', d: '地表数据诊断', ready: false,
    need: '单点站点会自动关闭地表数据诊断',
  },
];

const TRACERS = [
  { id: 'isotope', t: '水同位素', d: 'H₂¹⁸O / HDO 水循环同位素', ready: false, need: '暂未开放' },
  { id: 'methane', t: '甲烷 CH₄', d: '湿地、土壤、湖泊甲烷产生/氧化/排放', ready: true },
  { id: 'solute', t: '溶质', d: '水溶性示踪物', ready: false, need: '暂未开放' },
  { id: 'sediment', t: '泥沙', d: '颗粒泥沙输移', ready: false, need: '单点站点不可用；需要河道/流域输移链路' },
];

const pages = () => [
  'domain',
  ...(picked.domain && picked.domain !== 'site' ? ['grid'] : []),
  'subgrid', 'soil', 'physics', ...(picked.physics.tracer ? ['tracer'] : []), 'debug',
];
const pageIndex = page => pages().indexOf(page);
const pageNumber = page => pageIndex(page) + 1;

const emptyPhysics = () => ({ urban: false, lulcc: false, bgc: false, crop: false, tracer: false });
const emptyDebug = () => ({ rangecheck: false, colmdebug: false, srfdatadiag: false });
const emptyPicked = () => ({
  domain: null,
  grid: null,
  subgrid: null,
  soil: 'vg',
  physics: emptyPhysics(),
  tracer: null,
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
  const list = pages();
  if (pageIdx >= list.length) pageIdx = list.length - 1;
  const page = list[pageIdx];
  const copy = {
    domain: ['这次要跑什么？', '先选择模拟范围，再为流域、区域或全球选择计算网格。'],
    grid: ['计算网格怎么组织？', '三种空间范围都可选择经纬度、非结构或流域网格。'],
    subgrid: ['次网格怎么分？', '次网格方案决定 BGC 是否可用，也决定站点数据要求。'],
    soil: ['土壤水力用哪套？', '选择本次模拟使用的土壤水力方案。'],
    physics: ['还要打开哪些过程？', '可多选；被上游约束挡住的项会说明回哪一页修改。'],
    tracer: ['选择示踪剂类型', '目前只开放甲烷 CH₄；其他类型保留入口但不可选。'],
    debug: ['要打开调试吗？', '可全部不选；这些开关只增加检查与日志，不改变页间约束。'],
  }[page];
  $('gatetitle').textContent = copy[0];
  $('gatesub').textContent = `第 ${pageIdx + 1}/${list.length} 页 · ${copy[1]}`;
  $('gateinfo').textContent = pageInfo(page);

  const box = $('gatecards');
  box.textContent = '';
  if (page === 'domain') renderCards(DOMAINS, picked.domain, chooseDomain);
  if (page === 'grid') renderCards(GRIDS, picked.grid, chooseGrid);
  if (page === 'subgrid') renderCards(SUBGRIDS, picked.subgrid, chooseSubgrid, subgridBlock);
  if (page === 'soil') renderCards(SOILS, picked.soil, chooseSoil);
  if (page === 'physics') renderCards(PHYSICS, picked.physics, togglePhysics, physicsBlock, true);
  if (page === 'tracer') renderCards(TRACERS, picked.tracer, chooseTracer, tracerBlock);
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
  if (page === 'physics') return 'ⓘ 灰项仍然列出；带“← 第 N 页”的卡片可直接返回修改';
  if (page === 'tracer') return 'ⓘ 甲烷需要 PFT 或 PC、BGC、van Genuchten 土壤水力；本页会把运行参数自动写入算例';
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
  if (id === 'site') picked.grid = null;
  render();
}

function chooseGrid(id) {
  picked.grid = id;
  render();
}

function chooseSubgrid(id) {
  picked.subgrid = id;
  if (id !== 'PFT' && id !== 'PC') {
    picked.physics.bgc = false;
    picked.physics.crop = false;
    picked.physics.tracer = false;
    picked.tracer = null;
  }
  render();
}

function chooseSoil(id) {
  picked.soil = id;
  if (id === 'campbell') { picked.physics.tracer = false; picked.tracer = null; }
  render();
}

function togglePhysics(id) {
  picked.physics[id] = !picked.physics[id];
  if (id === 'urban' && picked.domain === 'site' && picked.physics.urban) {
    picked.physics.bgc = false;
    picked.physics.crop = false;
    picked.physics.tracer = false;
    picked.physics.lulcc = false;
    picked.tracer = null;
  }
  if (id === 'bgc') {
    if (!picked.physics.bgc) { picked.physics.crop = false; picked.physics.tracer = false; picked.tracer = null; }
    picked.physics.lulcc = false;
  }
  if (id === 'crop' && picked.physics.crop) picked.physics.lulcc = false;
  if (id === 'tracer' && !picked.physics.tracer) picked.tracer = null;
  render();
}

function chooseTracer(id) {
  picked.tracer = id;
  render();
}

function toggleDebug(id) {
  picked.debug[id] = !picked.debug[id];
  render();
}

function subgridBlock(item) {
  if (kernelForSubgrid(item.id, picked)) return null;
  const classification = item.id === 'USGS' ? 'USGS' : 'IGBP';
  return { need: state.kernels.length ? `当前安装缺少 ${classification} 内核` : '正在检查可用内核' };
}

function physicsBlock(item) {
  if (item.ready === false) return null;
  if (item.id === 'lulcc') {
    if (picked.subgrid === 'USGS') {
      return { need: 'LULCC 不支持 USGS 次网格', cause: `第 ${pageNumber('subgrid')} 页选了 USGS`, page: pageIndex('subgrid') };
    }
    if (picked.physics.bgc) return { need: 'LULCC 不能与 BGC 同时开启' };
    if (picked.domain === 'site') {
      return { need: '单点站点暂不支持 LULCC', cause: '第 1 页选择了站点', page: 0 };
    }
  }
  if (item.id === 'urban') {
    return null;
  }
  if (item.id === 'bgc') {
    if (picked.domain === 'site' && picked.physics.urban) {
      return { need: '纯城市单点不运行 BGC', cause: '本页已开启 URBAN' };
    }
    if (picked.subgrid !== 'PFT' && picked.subgrid !== 'PC') {
      return { need: '需要 PFT 或 PC 次网格', cause: `第 ${pageNumber('subgrid')} 页选了 ${picked.subgrid}`, page: pageIndex('subgrid') };
    }
  }
  if (item.id === 'crop') {
    if (picked.domain === 'site' && picked.physics.urban) return { need: '城市单点暂不支持 CROP', cause: '本页已开启 URBAN' };
    if (picked.subgrid !== 'PFT' && picked.subgrid !== 'PC') return { need: 'CROP 需要 PFT 或 PC 次网格', cause: `第 ${pageNumber('subgrid')} 页选了 ${picked.subgrid}`, page: pageIndex('subgrid') };
    if (!picked.physics.bgc) return { need: 'CROP 需要同时开启 BGC' };
    if (!kernelForSubgrid(picked.subgrid, { ...picked, crop: true })) return { need: '当前安装缺少 CROP-enabled 内核' };
  }
  if (item.id === 'tracer') {
    if (picked.domain === 'site' && picked.physics.urban) {
      return { need: '城市单点暂不支持甲烷示踪', cause: '本页已开启 URBAN' };
    }
    if (picked.subgrid !== 'PFT' && picked.subgrid !== 'PC') {
      return { need: '甲烷示踪需要 PFT 或 PC 次网格', cause: `第 ${pageNumber('subgrid')} 页选了 ${picked.subgrid}`, page: pageIndex('subgrid') };
    }
    if (picked.soil !== 'vg') {
      return { need: '需要 van Genuchten 土壤水力', cause: `第 ${pageNumber('soil')} 页选了 Campbell`, page: pageIndex('soil') };
    }
    if (!picked.physics.bgc) return { need: '甲烷示踪需要同时开启 BGC' };
  }
  return null;
}

function tracerBlock(item) {
  if (item.ready === false) return { need: item.need };
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

  const list = pages();
  const page = list[pageIdx];
  const required = { domain: picked.domain, grid: picked.grid, subgrid: picked.subgrid, soil: picked.soil, tracer: picked.tracer };
  const next = document.createElement('button');
  next.className = 'btn-next';
  next.textContent = '下一步 →';
  next.disabled = Object.hasOwn(required, page) && !required[page];
  next.onclick = () => {
    if (pageIdx === list.length - 1) finish();
    else { pageIdx += 1; render(); }
  };
  foot.appendChild(next);
}

function finish() {
  // 新向导就是一次新任务；站点库可以复用，但上一次的选择和运行批次不能
  // 混进来，否则运行页会出现与本次模型配置无关的旧算例。
  state.picked.clear();
  state.pickedSite = null;
  state.pickedSiteAuto = false;
  state.pickedCases.clear();
  state.batch = [];
  state.createdCases.clear();
  state.createdBySite.clear();
  state.selected = null;
  state.expertCaseDir = null;
  state.resultCaseDir = null;
  state.resultSelection.clear();
  state.resultSelectionTouched = false;
  state.resultObsOverrides.clear();
  state.resultMetrics = [];
  state.resultFailures = [];
  state.text = '';
  state.prepArtifacts = {
    siteStem: null, siteFile: null, siteDir: null, siteReport: null,
    rawdataDir: null, forcingFile: null, forcingDir: null,
    observationFile: null, observationDir: null, batchSites: [],
  };
  state.domain = picked.domain;
  state.grid = picked.grid;
  state.subgrid = picked.subgrid;
  state.wizard = {
    grid: picked.grid,
    subgrid: picked.subgrid,
    soil: picked.soil,
    physics: { ...picked.physics },
    tracer: picked.tracer,
    debug: { ...picked.debug },
  };
  globalThis.dispatchEvent?.(new Event('colm:wizard'));
  $('domaingate').hidden = true;
  // 向导已经决定本次模型结构；通常下一步是选现成站点并建算例，不是重新
  // 制作原始数据。前处理仍在左侧作为按需入口，但不再拦住主路径。
  go('basic-files');
}

const logical = value => value ? '.true.' : '.false.';

/** 新建算例时一次写入的运行时初值。编译期的 USGS / CROP 不在这里伪装成可写。 */
export function wizardFields(wizard = state.wizard) {
  if (!wizard) return [];
  const p = wizard.physics;
  const d = wizard.debug;
  const methane = p.tracer && wizard.tracer === 'methane';
  const urban = p.urban;
  const bgc = p.bgc || p.crop || methane;
  const fields = [
    ['DEF_USE_LCT', wizard.subgrid === 'IGBP' || wizard.subgrid === 'USGS', 'logical'],
    ['DEF_USE_PFT', wizard.subgrid === 'PFT', 'logical'],
    ['DEF_USE_PC', wizard.subgrid === 'PC', 'logical'],
    ['DEF_USE_Campbell_SOIL_MODEL', wizard.soil === 'campbell', 'logical'],
    ['DEF_USE_BGC', bgc, 'logical'],
    ['DEF_USE_NITRIF', bgc, 'logical'],
    ['DEF_USE_FERT', false, 'logical'],
    ['DEF_USE_CNSOYFIXN', false, 'logical'],
    ['DEF_Aerosol_Readin', false, 'logical'],
    ['DEF_URBAN_RUN', urban, 'logical'],
    ['DEF_USE_LULCC', p.lulcc, 'logical'],
    ['DEF_USE_TRACER', methane, 'logical'],
    ['DEF_USE_RangeCheck', d.rangecheck, 'logical'],
    ['DEF_USE_CoLMDEBUG', d.colmdebug, 'logical'],
    ['DEF_USE_SrfdataDiag', d.srfdatadiag, 'logical'],
  ];
  if (p.crop) fields.push(
    ['DEF_USE_LAIFEEDBACK', true, 'logical'],
    ['DEF_USE_IRRIGATION', false, 'logical'],
    ['DEF_TUNING_CROP_PLANTING_DAY', '120'],
  );
  if (methane) fields.push(
    ['DEF_USE_Dynamic_Wetland', false, 'logical'],
    ['DEF_TRACER_NUM', '1'],
    ['DEF_TRACER_NAMES', 'CH4'],
    ['DEF_TRACER_TYPES', 'gas'],
    ['DEF_TRACER_MRAT', '16.04'],
    ['DEF_TRACER_REF_RATIO', '1.0'],
    ['DEF_TRACER_INIT_DELTA', '0.0'],
    ['DEF_TRACER_REACTIVE_DECAY_RATE', '0.0'],
    ['DEF_TRACER_PARAM_FILES', 'CH4:standard_ch4_parameter.nml'],
  );
  return fields.map(([path, value, kind]) => ({ path, value: kind === 'logical' ? logical(value) : String(value) }));
}

/** 向导锁定结构字段；CROP 的施肥、灌溉和种植日只是初值，仍可在过程/专家页调。 */
export function wizardFieldNames(wizard = state.wizard) {
  const empty = {
    subgrid: null, soil: null, tracer: null,
    physics: emptyPhysics(), debug: emptyDebug(),
  };
  const editableInitials = new Set([
    'DEF_USE_NITRIF', 'DEF_USE_FERT', 'DEF_USE_CNSOYFIXN', 'DEF_Aerosol_Readin',
  ]);
  const names = wizardFields(empty).map(x => x.path).filter(path => !editableInitials.has(path)).concat(
    'DEF_USE_USGS', 'DEF_USE_IGBP',
    'DEF_TRACER_NUM', 'DEF_TRACER_NAMES', 'DEF_TRACER_TYPES', 'DEF_TRACER_MRAT',
    'DEF_TRACER_REF_RATIO', 'DEF_TRACER_INIT_DELTA', 'DEF_TRACER_REACTIVE_DECAY_RATE',
    'DEF_TRACER_PARAM_FILES',
  );
  if (wizard?.tracer === 'methane') names.push('DEF_USE_Dynamic_Wetland');
  return names;
}

globalThis.addEventListener?.('colm:kernels', () => {
  if (!$('domaingate').hidden) render();
});

$('homeBtn').onclick = showDomainGate;
