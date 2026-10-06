//! 进门向导。顺序按约束方向排：空间 → 计算网格 → 次网格 → 土壤 → 物理 → 示踪剂 → 调试。
//!
//! 回退保留无关选择，改上游时只清掉已经失效的下游值。USGS 由向导
//! 自动匹配专用编译产物；CROP 需要 CROP-enabled 内核。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';
import { kernelForSubgrid } from './kernel.js';
import { invoke } from './ipc.js';

const DOMAINS = [
  { id: 'site', t: '站点', d: '单点站点模拟', ready: true },
  { id: 'watershed', t: '流域', d: '按流域边界限定模拟范围' },
  { id: 'region', t: '区域', d: '按经纬度范围限定模拟区域' },
  { id: 'global', t: '全球', d: '覆盖全球陆地区域' },
];

const GRIDS = [
  { id: 'latlon', t: '经纬度网格', d: '规则等经纬度网格（GRIDBASED）' },
  { id: 'unstructured', t: '非结构网格', d: '由 elmindex 描述计算单元（UNSTRUCTURED）' },
  { id: 'catchment', t: '流域网格', d: '集水区与 HRU 水文单元（CATCHMENT），含坡面、河湖与地下侧向流' },
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
  { id: 'river', t: '河湖汇流', d: '格点河道、湖泊与水库汇流（15′ 单元流域河网）', note: () => riverNote() },
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

// 甲烷淹没方案（写进 CH4 参数文件的 `DEF_METHANE%inundation_mode`）。动态湿地跟着方案走：
// dynamic_wtd / hybrid 打开，其余关闭——上游与 `colm-rs` 都要求两者一致，否则停机。
const METHANE_MODES = [
  { id: 'hybrid', t: '混合（hybrid）', d: '动态地下水位 + 河网洪泛；上游参数文件的推荐方案', dynamic: true, river: true },
  { id: 'routing', t: '河网洪泛（routing）', d: '淹水比例取河湖汇流的洪泛面积', river: true },
  { id: 'dynamic_wtd', t: '动态地下水位（dynamic_wtd）', d: '按动态湿地的地下水位估算淹水比例', dynamic: true },
  { id: 'satellite', t: '卫星湿地（GIEMS）', d: '读 GIEMS 月均淹水面积；建例后要在「示踪剂」页选择 GIEMS 文件' },
  { id: 'wetwat', t: '湿地蓄水（wetwat）', d: '按湿地 patch 的蓄水判断淹水；任何内核都能跑' },
];

const pages = () => [
  'domain',
  ...(picked.domain && picked.domain !== 'site' ? ['grid', 'spatial'] : []),
  'subgrid', 'soil', 'physics', ...(picked.physics.tracer ? ['tracer'] : []),
  ...(picked.physics.tracer && picked.tracer === 'methane' ? ['methane'] : []), 'debug',
];
const pageIndex = page => pages().indexOf(page);
const pageNumber = page => pageIndex(page) + 1;

const emptyPhysics = () => ({
  urban: false, lulcc: false, bgc: false, river: false, crop: false, tracer: false,
});
const emptyDebug = () => ({ rangecheck: false, colmdebug: false, srfdatadiag: false });
const emptyPicked = () => ({
  domain: null,
  grid: null,
  spatial: {
    shapefile: '', west: '', east: '', south: '', north: '',
    dlon: '0.5', dlat: '0.5', meshFile: '', catchmentFile: '',
  },
  subgrid: null,
  soil: 'vg',
  physics: emptyPhysics(),
  tracer: null,
  methaneMode: null,
  methaneModeChosen: false,
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

/** 前几页改了选择后，过程页里已勾选、现在却被挡住的项清掉；否则会照样写进算例，建例时才失败。 */
function dropBlockedPhysics() {
  for (let pass = 0; pass < PHYSICS.length; pass += 1) {
    const blocked = PHYSICS.find(item => picked.physics[item.id] && physicsBlock(item));
    if (!blocked) break;
    picked.physics[blocked.id] = false;
  }
  if (!picked.physics.bgc) { picked.physics.crop = false; picked.physics.tracer = false; }
  if (!picked.physics.tracer) picked.tracer = null;
  if (picked.tracer !== 'methane') { picked.methaneMode = null; picked.methaneModeChosen = false; }
}

function render() {
  dropBlockedPhysics();
  const list = pages();
  if (pageIdx >= list.length) pageIdx = list.length - 1;
  const page = list[pageIdx];
  const copy = {
    domain: ['这次要跑什么？', '先选择模拟范围，再为流域、区域或全球选择计算网格。'],
    grid: ['计算网格怎么组织？', '三种空间范围都可选择经纬度、非结构或流域网格。'],
    spatial: ['空间输入怎么准备？', '网格类型决定要准备哪些空间文件；RawData、Runtime 与强迫场在下一步选。'],
    subgrid: ['次网格怎么分？', '次网格方案决定 BGC 是否可用，也决定站点数据要求。'],
    soil: ['土壤水力用哪套？', '选择本次模拟使用的土壤水力方案。'],
    physics: ['还要打开哪些过程？', '可多选；被上游约束挡住的项会说明回哪一页修改。'],
    tracer: ['选择示踪剂类型', '目前只开放甲烷 CH₄；其他类型保留入口但不可选。'],
    methane: ['甲烷淹水范围怎么算？', '决定湿地与土壤的淹水比例从哪来；动态湿地会随方案自动打开或关闭。'],
    debug: ['要打开调试吗？', '可全部不选；这些开关只增加检查与日志，不改变页间约束。'],
  }[page];
  $('gatetitle').textContent = copy[0];
  $('gatesub').textContent = `第 ${pageIdx + 1}/${list.length} 页 · ${copy[1]}`;
  $('gateinfo').textContent = pageInfo(page);

  const box = $('gatecards');
  box.textContent = '';
  if (page === 'domain') renderCards(DOMAINS, picked.domain, chooseDomain);
  if (page === 'grid') renderCards(GRIDS, picked.grid, chooseGrid);
  if (page === 'spatial') renderSpatial();
  if (page === 'subgrid') renderCards(SUBGRIDS, picked.subgrid, chooseSubgrid, subgridBlock);
  if (page === 'soil') renderCards(SOILS, picked.soil, chooseSoil);
  if (page === 'physics') renderCards(PHYSICS, picked.physics, togglePhysics, physicsBlock, true);
  if (page === 'tracer') renderCards(TRACERS, picked.tracer, chooseTracer, tracerBlock);
  if (page === 'methane') {
    // 用户没亲手选过时跟着约束走（回去打开河湖汇流后自动换成 hybrid）；亲手选的只在被挡住时才换。
    if (!picked.methaneModeChosen || methaneModeBlock({ id: picked.methaneMode })) {
      picked.methaneMode = defaultMethaneMode();
      picked.methaneModeChosen = false;
    }
    renderCards(METHANE_MODES, picked.methaneMode, chooseMethaneMode, methaneModeBlock);
  }
  if (page === 'debug') renderCards(DEBUG, picked.debug, toggleDebug, null, true);
  renderFoot();
}

function pageInfo(page) {
  if (page === 'spatial') {
    const issue = spatialIssue();
    return issue ? `ⓘ ${issue}` : '✓ 空间输入参数完整；文件内容会在启动长任务前预检';
  }
  if (page === 'subgrid') {
    if (picked.subgrid === 'PFT' || picked.subgrid === 'PC') {
      return 'ⓘ 站点文件最好提供 pfttyp 与 pctpfts；缺少时会回落到 rawdata/plant_15s';
    }
    return picked.subgrid === 'IGBP' ? 'ⓘ 站点数据使用 IGBP_classification' : 'ⓘ 必须选择一种次网格方案';
  }
  if (page === 'physics') return 'ⓘ 灰项仍然列出；带“← 第 N 页”的卡片可直接返回修改';
  if (page === 'tracer') return 'ⓘ 甲烷需要 PFT 或 PC、BGC、van Genuchten 土壤水力；本页会把运行参数自动写入算例';
  if (page === 'methane') return 'ⓘ 建例后仍可在专家模式「示踪剂」页改；改方案时动态湿地会一并改';
  if (page === 'debug') return 'ⓘ 打开调试会让日志明显增多，常规运行可全部关闭';
  return '';
}

function renderSpatial() {
  const panel = document.createElement('div');
  panel.className = 'card spatial-config';
  const title = document.createElement('h3');
  if (picked.grid === 'unstructured') {
    title.textContent = '非结构网格输入';
    panel.append(title, spatialStudyNote());
    panel.appendChild(pathField('已有非结构 mesh NetCDF（必需）', 'meshFile', 'nc,nc4'));
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = '读取已有 mesh 的 elmindex 与空间范围；无需设置边界或分辨率。';
    panel.append(note, nextStepNote());
    $('gatecards').appendChild(panel);
    return;
  }
  if (picked.grid === 'catchment') {
    // 流域网格文件自带单元、HRU、河网与空间范围：不需要边界或 Shapefile。
    title.textContent = '流域网格数据';
    panel.append(title, spatialStudyNote());
    panel.appendChild(pathField('流域网格 NetCDF（必需）', 'catchmentFile', 'nc,nc4'));
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = '包含集水区单元、HRU 与河网的 NetCDF（DEF_CatchmentMesh_data）；模拟范围直接取自这个文件，无需另选流域边界。';
    panel.append(note, nextStepNote());
    $('gatecards').appendChild(panel);
    return;
  }
  title.textContent = picked.domain === 'watershed' ? '流域边界'
    : picked.domain === 'region' ? '区域边界' : '全球范围';
  panel.appendChild(title);
  panel.appendChild(spatialStudyNote());

  if (picked.domain === 'watershed') {
    panel.appendChild(pathField('流域 Shapefile（WGS84）', 'shapefile', 'shp'));
  } else if (picked.domain === 'region') {
    const row1 = document.createElement('div');
    row1.className = 'row';
    row1.append(numberField('西边界', 'west', -180, 180), numberField('东边界', 'east', -180, 180));
    const row2 = document.createElement('div');
    row2.className = 'row';
    row2.append(numberField('南边界', 'south', -90, 90), numberField('北边界', 'north', -90, 90));
    panel.append(row1, row2);
  } else {
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = picked.grid === 'latlon'
      ? '全球经纬度网格自动使用边界：西=-180°，东=180°，南=-90°，北=90°。'
      : '使用全球范围，不再填写边界。';
    panel.appendChild(note);
  }

  const gridTitle = document.createElement('h3');
  gridTitle.textContent = '经纬度网格设置';
  panel.appendChild(gridTitle);
  {
    const row = document.createElement('div');
    row.className = 'row';
    row.append(numberField('经度分辨率（度）', 'dlon', 0, 360), numberField('纬度分辨率（度）', 'dlat', 0, 180));
    panel.appendChild(row);
    const note = document.createElement('p');
    note.className = 'muted mini';
    note.textContent = '按经纬度边界和分辨率生成网格，以 GRIDBASED 模式运行；海洋按 RawData 里的地表覆盖自动剔除。';
    panel.appendChild(note);
  }
  panel.appendChild(nextStepNote());
  $('gatecards').appendChild(panel);
}

/** 空间算例的另外三项输入在「基本设定」页选，这里先说清楚，免得用户在本页找。 */
function nextStepNote() {
  const note = document.createElement('p');
  note.className = 'muted mini';
  note.textContent = '下一步「基本设定」里还要选择：RawData 目录、Runtime 目录，以及强迫场（选标准数据集并指定本机数据目录，或使用已有 forcing namelist）。';
  return note;
}


function spatialStudyNote() {
  const warning = document.createElement('p');
  warning.className = 'warn mini';
  warning.textContent = '空间算例暂不支持参数调优和不确定性分析。';
  return warning;
}

function numberField(label, key, min, max) {
  const field = document.createElement('div');
  field.className = 'field';
  const caption = document.createElement('label');
  caption.textContent = label;
  const input = document.createElement('input');
  input.id = `spatial-${key}`;
  caption.htmlFor = input.id;
  input.className = 'input';
  input.type = 'number';
  input.step = 'any';
  input.min = String(min);
  input.max = String(max);
  input.value = picked.spatial[key];
  input.oninput = () => updateSpatial(key, input.value);
  field.append(caption, input);
  return field;
}

function pathField(label, key, filter) {
  const field = document.createElement('div');
  field.className = 'field';
  const caption = document.createElement('label');
  caption.textContent = label;
  const browse = document.createElement('div');
  browse.className = 'browse';
  const input = document.createElement('input');
  input.id = `spatial-${key}`;
  caption.htmlFor = input.id;
  input.className = 'input';
  input.value = picked.spatial[key];
  input.oninput = () => updateSpatial(key, input.value);
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'btn-ghost';
  button.textContent = '选择…';
  button.setAttribute('aria-label', `选择${label}`);
  button.onclick = async () => {
    let value = null;
    try { value = await invoke?.('pick_file', { key: input.id, filter }); } catch { /* 可继续粘贴路径 */ }
    if (value) { input.value = value; input.oninput(); }
  };
  browse.append(input, button);
  field.append(caption, browse);
  return field;
}

function updateSpatial(key, value) {
  picked.spatial[key] = String(value).trim();
  $('gateinfo').textContent = pageInfo('spatial');
  renderFoot();
}

function spatialIssue() {
  const s = picked.spatial;
  if (picked.grid === 'unstructured') return s.meshFile ? null : '请选择已有非结构 mesh NetCDF';
  if (picked.grid === 'catchment') return s.catchmentFile ? null : '请选择流域网格 NetCDF';
  if (picked.domain === 'watershed' && !s.shapefile) return '请选择流域 Shapefile';
  if (picked.domain === 'region') {
    if ([s.west, s.east, s.south, s.north].some(value => value === '')) return '请填写完整的区域边界';
    const [west, east, south, north] = [s.west, s.east, s.south, s.north].map(Number);
    if (![west, east, south, north].every(Number.isFinite)) return '请填写完整的区域边界';
    if (west < -180 || east > 180 || south < -90 || north > 90 || west >= east || south >= north) {
      return '区域边界必须位于 WGS84 范围，并满足西 < 东、南 < 北';
    }
  }
  const dlon = Number(s.dlon);
  const dlat = Number(s.dlat);
  if (!(dlon > 0) || !(dlat > 0)) return '经纬度分辨率必须大于 0';
  const divides = (span, step) => Math.abs(span / step - Math.round(span / step)) < 1e-9;
  if (!divides(360, dlon) || !divides(180, dlat)) {
    return '分辨率必须整除全球 360°×180° 格架';
  }
  const [nlon, nlat] = [Math.round(360 / dlon), Math.round(180 / dlat)];
  if (!Number.isSafeInteger(nlon) || !Number.isSafeInteger(nlat)
      || !Number.isSafeInteger(nlon * nlat)) return '格点数量超过安全整数范围';
  return null;
}

function renderCards(items, selected, choose, blocker = null, multi = false) {
  const box = $('gatecards');
  for (const item of items) {
    const on = multi ? !!selected[item.id] : selected === item.id;
    box.appendChild(card(item, on, choose, blocker?.(item), multi));
  }
}

// 河网用的是 15′ 单元流域，与模拟范围各自独立：经纬度网格的范围来自第 1 页的流域边界或区域，
// 非结构网格的范围来自它自己的 mesh 文件。全球范围不必提醒。
const RIVER_NOTES = {
  watershed: '注意：Shapefile 的流域边界与河网的单元流域划分可能不一致，边界附近的单元流域只有一部分在模拟范围内，出口流量会偏小。',
  region: '注意：区域须包含所关心河流的全部上游子流域；区域外的上游不产流，下游流量会偏小。',
  unstructured: '注意：非结构 mesh 的边界与河网的单元流域划分可能不一致，mesh 须包含所关心河流的全部上游子流域；mesh 外的上游与边界处只部分覆盖的单元流域都会少算产流，出口流量会偏小。',
};
function riverNote() {
  if (picked.domain === 'global') return null;
  return RIVER_NOTES[picked.grid === 'unstructured' ? 'unstructured' : picked.domain] ?? null;
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
    const note = item.note?.();
    if (note) {
      const n = document.createElement('span');
      n.className = 'dnote';
      n.textContent = note;
      b.appendChild(n);
    }
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

// 流域网格的侧向流与城市、LULCC、示踪物都没有移植（`colm-rs` 的 `check_catchment`）；它也没有格点河湖汇流。
const CATCHMENT_UNSUPPORTED = ['urban', 'lulcc', 'tracer', 'river'];

function chooseGrid(id) {
  picked.grid = id;
  if (id === 'catchment') {
    for (const flag of CATCHMENT_UNSUPPORTED) picked.physics[flag] = false;
    picked.tracer = null;
  }
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
  if (id === 'urban' && picked.physics.urban) {
    picked.physics.tracer = false;
    picked.tracer = null;
  }
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

function chooseMethaneMode(id) {
  picked.methaneMode = id;
  picked.methaneModeChosen = true;
  render();
}

/** 有网格河湖时用上游推荐的 hybrid，否则 wetwat。 */
function defaultMethaneMode() {
  return methaneModeBlock({ id: 'hybrid' }) ? 'wetwat' : 'hybrid';
}

function methaneModeBlock(item) {
  const mode = METHANE_MODES.find(m => m.id === item.id);
  if (mode?.river) {
    if (picked.domain === 'site') return { need: '需要河湖汇流；单点没有河网' };
    if (!picked.physics.river) {
      return { need: '需要河湖汇流', cause: `第 ${pageNumber('physics')} 页没有开启河湖汇流`, page: pageIndex('physics') };
    }
  }
  // 单点只有湿地站点才有动态湿地，建例前还不知道站点类型；湿地站点可在专家页改。
  if (mode?.dynamic && picked.domain === 'site') return { need: '单点请先用 wetwat；湿地站点建例后可在专家页改' };
  return null;
}

function toggleDebug(id) {
  picked.debug[id] = !picked.debug[id];
  render();
}

function subgridBlock(item) {
  if (kernelForSubgrid(item.id, picked)) return null;
  const classification = item.id === 'USGS' ? 'USGS' : 'IGBP';
  const grid = picked.domain === 'site' ? '站点'
    : picked.grid === 'latlon' ? '经纬度'
      : picked.grid === 'unstructured' ? '非结构' : '流域网格';
  return { need: state.kernels.length ? `当前安装缺少 ${grid} + ${classification} 内核` : '正在检查可用内核' };
}

function physicsBlock(item) {
  if (item.ready === false) return null;
  if (picked.grid === 'catchment' && ['urban', 'lulcc', 'tracer'].includes(item.id)) {
    return {
      need: '流域网格的侧向流暂不支持这一项',
      cause: `第 ${pageNumber('grid')} 页选了流域网格`,
      page: pageIndex('grid'),
    };
  }
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
  // `DEF_USE_GridRiverLakeFlow` 只在编进 GridRiverLakeFlow 的经纬度/非结构网格内核里有。
  if (item.id === 'river') {
    if (picked.domain === 'site') {
      return { need: '单点站点没有河湖汇流', cause: '第 1 页选择了站点', page: 0 };
    }
    if (picked.grid === 'catchment') {
      return { need: '流域网格自带坡面与河道侧向流，不用这一项', cause: `第 ${pageNumber('grid')} 页选了流域网格`, page: pageIndex('grid') };
    }
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
    // 城市斑块没有示踪物账（建例时 `validate_runtime_contract` 与运行时装配都会拒绝），站点与空间都一样。
    if (picked.physics.urban) {
      return { need: '城市模式暂不支持甲烷示踪', cause: '本页已开启 URBAN' };
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

  if (pageIdx === 0 && state.wizard) {
    // 「返回首页」点错了也能回去：当前任务原样保留。
    const back = document.createElement('button');
    back.className = 'btn-ghost';
    back.textContent = '← 返回当前任务';
    back.onclick = () => { $('domaingate').hidden = true; };
    foot.appendChild(back);
  }
  if (pageIdx === 0) {
    // 不走向导：直接打开磁盘上已有的算例（`opencase.js` 接这个事件）。
    const open = document.createElement('button');
    open.className = 'btn-ghost';
    open.textContent = '打开已有算例…';
    open.onclick = () => globalThis.dispatchEvent?.(new Event('colm:open-case'));
    foot.appendChild(open);
  }

  const list = pages();
  const page = list[pageIdx];
  const required = { domain: picked.domain, grid: picked.grid, subgrid: picked.subgrid, soil: picked.soil, tracer: picked.tracer, methane: picked.methaneMode };
  const next = document.createElement('button');
  next.className = 'btn-next';
  next.textContent = '下一步 →';
  next.disabled = (Object.hasOwn(required, page) && !required[page]) || (page === 'spatial' && !!spatialIssue());
  next.onclick = () => {
    if (pageIdx === list.length - 1) finish();
    else { pageIdx += 1; render(); }
  };
  foot.appendChild(next);
}

/** 开始一次新会话：清掉上一次的选择与运行批次，装上这份模型配置并通知各页。
 *  `config` 与向导结束时的形状相同：`{ domain, grid, spatial, subgrid, soil, physics, tracer, debug }`。
 *  向导与「打开已有算例」共用这一条路。 */
export function startSession(config) {
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
  // 评估变量按本次模型的可用变量重新勾选（甲烷会话要有 FCH4），不能沿用上一次会话的勾选。
  state.evaluationVariables = new Set();
  state.evaluationSelectionTouched = false;
  state.resultMetricMissing = [];
  state.summaryVar = null;
  state.text = '';
  state.prepArtifacts = {
    siteStem: null, siteFile: null, siteDir: null, siteReport: null,
    rawdataDir: null, forcingFile: null, forcingDir: null,
    observationFile: null, observationDir: null, batchSites: [],
  };
  state.domain = config.domain;
  state.grid = config.grid;
  state.spatial = config.spatial;
  state.subgrid = config.subgrid;
  state.wizard = {
    grid: config.grid,
    spatial: config.spatial,
    subgrid: config.subgrid,
    soil: config.soil,
    physics: { ...emptyPhysics(), ...config.physics },
    tracer: config.tracer,
    methaneMode: config.methaneMode ?? null,
    debug: { ...emptyDebug(), ...config.debug },
  };
  globalThis.dispatchEvent?.(new Event('colm:wizard'));
  $('domaingate').hidden = true;
}

function finish() {
  dropBlockedPhysics();
  if (picked.tracer === 'methane' && (!picked.methaneMode || methaneModeBlock({ id: picked.methaneMode }))) {
    picked.methaneMode = defaultMethaneMode();
  }
  const spatial = picked.domain === 'site' ? null : {
    domain: picked.grid === 'unstructured' || picked.grid === 'catchment'
      ? { kind: picked.domain }
      : picked.domain === 'watershed'
      ? { kind: picked.domain, shapefile: picked.spatial.shapefile }
      : picked.domain === 'region'
        ? {
          kind: picked.domain,
          west: Number(picked.spatial.west), east: Number(picked.spatial.east),
          south: Number(picked.spatial.south), north: Number(picked.spatial.north),
        }
        : picked.domain === 'global' && picked.grid === 'latlon'
          ? { kind: picked.domain, west: -180, east: 180, south: -90, north: 90 }
        : { kind: picked.domain },
    grid: picked.grid === 'unstructured'
      ? { kind: picked.grid, meshFile: picked.spatial.meshFile }
      : picked.grid === 'catchment'
      ? { kind: picked.grid, input: picked.spatial.catchmentFile }
      : {
        kind: picked.grid,
        // 经纬度网格总是按分辨率新生成网格；之前在非结构网格页填过的 mesh 不能带过来。
        meshFile: null,
        dlon: Number(picked.spatial.dlon), dlat: Number(picked.spatial.dlat),
        nlon: Math.round(360 / Number(picked.spatial.dlon)),
        nlat: Math.round(180 / Number(picked.spatial.dlat)),
      },
  };
  startSession({
    domain: picked.domain, grid: picked.grid, spatial, subgrid: picked.subgrid, soil: picked.soil,
    physics: picked.physics, tracer: picked.tracer, methaneMode: picked.methaneMode, debug: picked.debug,
  });
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
  // 区域单元流域汇流（`DEF_UnitCatchment_regional`）：只对落在区域内的河网汇流，2°×2° 的算例
  // 模拟阶段快约 6 倍、结果两侧逐位一致（第 567 轮）。上游要求 GridRiverLakeFlow（流域网格内核没有）
  // 且不能与 LULCC 同开；全球范围裁剪了也是全球，保持关闭。参数页不显示它，之后打开 LULCC 时自动关掉。
  // 河湖汇流总开关（`DEF_USE_GridRiverLakeFlow`）：没选时河道不算，区域单元流域也无从谈起。
  if (wizard.spatial && wizard.grid !== 'catchment') {
    const global = wizard.spatial.domain?.kind === 'global';
    fields.push(['DEF_USE_GridRiverLakeFlow', p.river, 'logical']);
    fields.push(['DEF_UnitCatchment_regional', p.river && !global && !p.lulcc, 'logical']);
  }
  if (p.crop) fields.push(
    ['DEF_USE_LAIFEEDBACK', true, 'logical'],
    ['DEF_USE_IRRIGATION', false, 'logical'],
    // 播种日覆盖只在 SinglePoint 生效（空间算例上游与 `colm-rs` 都会停）；空间算例写 0 = 读运行时数据。
    // 字段本身保留：`sidecar.rs` 按它的存在识别作物算例。
    ['DEF_TUNING_CROP_PLANTING_DAY', wizard.spatial ? '0' : '120'],
  );
  const methaneMode = wizard.methaneMode ?? 'wetwat';
  if (methane) fields.push(
    ['DEF_USE_Dynamic_Wetland', methaneMode === 'dynamic_wtd' || methaneMode === 'hybrid', 'logical'],
    ['DEF_METHANE%inundation_mode', methaneMode],
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
export function wizardFieldNames() {
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
    'DEF_TRACER_PARAM_FILES', 'DEF_USE_GridRiverLakeFlow',
  );
  return names;
}

globalThis.addEventListener?.('colm:kernels', () => {
  if (!$('domaingate').hidden) render();
});

$('homeBtn').onclick = showDomainGate;
