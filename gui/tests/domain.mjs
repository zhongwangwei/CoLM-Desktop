import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

class El {
  constructor(tag = 'div') {
    this.tagName = tag.toUpperCase();
    this.children = [];
    this.attributes = {};
    this.dataset = {};
    this.style = {};
    this.hidden = false;
    this.disabled = false;
    this.selectedIndex = -1;
    this.options = [];
    this._text = '';
    this.id = '';
    this.htmlFor = '';
  }
  get textContent() { return this._text; }
  set textContent(value) {
    this._text = String(value);
    if (value === '') this.children = [];
  }
  appendChild(child) { this.children.push(child); return child; }
  append(...children) { this.children.push(...children); }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  getAttribute(name) { return this.attributes[name] ?? null; }
}

const ids = Object.fromEntries([
  'gatetitle', 'gatesub', 'gateinfo', 'gatecards', 'gatefoot', 'domaingate',
  'loadinggate', 'launchgate', 'localRunCard', 'steps', 'status', 'estSite', 'casename', 'kernel', 'homeBtn',
].map(id => [id, new El()]));

globalThis.document = {
  getElementById: id => ids[id] ?? new El(),
  createElement: tag => new El(tag),
  querySelectorAll: () => [],
};
globalThis.window = globalThis;
globalThis.addEventListener = () => {};
globalThis.requestAnimationFrame = () => 0;

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-gate-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');

const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;
const { showLaunchGate } = await import(moduleUrl('gate-boot.js'));
if (ids.loadinggate.hidden || !ids.launchgate.hidden || !ids.domaingate.hidden) {
  throw new Error('loading page must cover the launcher until the application is ready');
}
showLaunchGate();
if (!ids.loadinggate.hidden || ids.launchgate.hidden) {
  throw new Error('ready application must replace the loading page with the launcher');
}
ids.localRunCard.onclick();
if (!ids.launchgate.hidden || ids.domaingate.hidden || ids.gatetitle.textContent !== '这次要跑什么？') {
  throw new Error('local run did not enter the existing model wizard');
}
const { showDomainGate, wizardFields, wizardFieldNames } = await import(moduleUrl('domain.js'));
const { state } = await import(moduleUrl('state.js'));
const { kernelForSubgrid } = await import(moduleUrl('kernel.js'));
const { withoutWizardFields } = await import(moduleUrl('params.js'));

state.kernels = [
  { preset: 'default', dir: '/igbp', generator_args: 'SinglePoint LULC_IGBP CaMaOFF CROPOFF', macros: ['SinglePoint', 'LULC_IGBP', 'CaMaOFF', 'CROPOFF'] },
  { preset: 'usgs', dir: '/usgs', generator_args: 'SinglePoint LULC_USGS CaMaOFF CROPOFF', macros: ['SinglePoint', 'LULC_USGS', 'CaMaOFF', 'CROPOFF'] },
  { preset: 'crop', dir: '/crop', generator_args: 'SinglePoint LULC_IGBP CaMaOFF CROPON', macros: ['SinglePoint', 'LULC_IGBP', 'CaMaOFF', 'CROP'] },
  { preset: 'latlon', dir: '/latlon', generator_args: 'GRID LULC_IGBP CaMaOFF CROPOFF', macros: ['GRIDBASED', 'GridRiverLakeFlow', 'LULC_IGBP'] },
  { preset: 'unstructured', dir: '/unstructured', generator_args: 'UNSTRUCTURED LULC_IGBP CaMaOFF CROPOFF', macros: ['UNSTRUCTURED', 'GridRiverLakeFlow', 'LULC_IGBP'] },
  { preset: 'catchment', dir: '/catchment', generator_args: 'CATCHMENT LULC_IGBP CaMaOFF CROPOFF', macros: ['CATCHMENT', 'CatchLateralFlow', 'LULC_IGBP'] },
];
if (kernelForSubgrid('PC')?.dir !== '/igbp' || kernelForSubgrid('USGS')?.dir !== '/usgs') {
  throw new Error('subgrid did not resolve to its compiled land classification');
}
if (kernelForSubgrid('PC', { crop: true })?.dir !== '/crop' || kernelForSubgrid('PC', { crop: false })?.dir !== '/igbp') {
  throw new Error('kernel matching must separate CROP and non-CROP builds');
}
state.kernels = [
  { preset: 'default', dir: '/igbp-real', generator_args: 'SinglePoint LULC_USGS CaMaOFF CROPOFF', macros: ['SinglePoint', 'LULC_IGBP', 'CaMaOFF', 'CROPOFF'] },
  { preset: 'usgs', dir: '/usgs-real', generator_args: 'SinglePoint LULC_IGBP CaMaOFF CROPOFF', macros: ['SinglePoint', 'LULC_USGS', 'CaMaOFF', 'CROPOFF'] },
  { preset: 'crop', dir: '/crop-real', generator_args: 'SinglePoint LULC_IGBP CaMaOFF CROPON', macros: ['SinglePoint', 'LULC_IGBP', 'CaMaOFF', 'CROP'] },
  { preset: 'latlon', dir: '/latlon-real', generator_args: 'GRID LULC_IGBP CaMaOFF CROPOFF', macros: ['GRIDBASED', 'GridRiverLakeFlow', 'LULC_IGBP'] },
  { preset: 'unstructured', dir: '/unstructured-real', generator_args: 'UNSTRUCTURED LULC_IGBP CaMaOFF CROPOFF', macros: ['UNSTRUCTURED', 'GridRiverLakeFlow', 'LULC_IGBP'] },
  { preset: 'catchment', dir: '/catchment-real', generator_args: 'CATCHMENT LULC_IGBP CaMaOFF CROPOFF', macros: ['CATCHMENT', 'CatchLateralFlow', 'LULC_IGBP'] },
];
if (kernelForSubgrid('PC')?.dir !== '/igbp-real' || kernelForSubgrid('USGS')?.dir !== '/usgs-real') {
  throw new Error('kernel matching must prefer effective macros over requested generator_args');
}
if (kernelForSubgrid('PC', { physics: { crop: true } })?.dir !== '/crop-real') {
  throw new Error('crop wizard must resolve to the CROP-enabled kernel');
}

const cards = () => ids.gatecards.children;
const card = label => cards().find(c => c.children[0]?.textContent === label);
const nodeText = node => [node.textContent, ...node.children.flatMap(nodeText)].join(' ');
const foot = label => ids.gatefoot.children.find(b => b.textContent.includes(label));
const choose = label => {
  const c = card(label);
  if (!c) throw new Error(`missing card ${label} on ${ids.gatetitle.textContent}`);
  if (c.disabled) throw new Error(`${label} unexpectedly disabled`);
  c.onclick();
};
const next = () => foot('下一步').onclick();
const previous = () => foot('上一步').onclick();
const findNode = (root, predicate) => predicate(root) ? root
  : root.children.map(child => findNode(child, predicate)).find(Boolean);

showDomainGate();
if (ids.gatetitle.textContent !== '这次要跑什么？') throw new Error('page 1 missing');
if (cards().map(c => c.children[0].textContent).join('|') !== '站点|流域|区域|全球') {
  throw new Error('page 1 must list site, watershed, regional, and global in order');
}
for (const domain of ['流域', '区域', '全球']) {
  showDomainGate();
  choose(domain);
  next();
  if (ids.gatetitle.textContent !== '计算网格怎么组织？') throw new Error(`${domain} did not open the grid page`);
  if (cards().map(c => c.children[0].textContent).join('|') !== '经纬度网格|非结构网格|流域网格') {
    throw new Error(`${domain} must offer all three spatial grids`);
  }
  if (cards().some(c => c.disabled)) throw new Error(`${domain} unexpectedly disabled a grid choice`);
}

showDomainGate();
choose('区域'); next(); choose('非结构网格'); next();
if (ids.gatetitle.textContent !== '空间输入怎么准备？') throw new Error('spatial selections must collect domain and grid inputs');
if (nodeText(ids.gatecards).includes('early state') || !nodeText(ids.gatecards).includes('空间算例暂不支持参数率定和不确定性分析')) {
  throw new Error('spatial setup must state that Study is unavailable, without an early-state label');
}
if (findNode(ids.gatecards, node => node.id === 'spatial-west')
    || findNode(ids.gatecards, node => node.id === 'spatial-nonOceanMask')) {
  throw new Error('an unstructured mesh must not ask for lat-lon bounds or a landmask');
}
const mesh = findNode(ids.gatecards, node => node.id === 'spatial-meshFile');
if (!mesh || !foot('下一步').disabled || !ids.gateinfo.textContent.includes('已有非结构 mesh')) {
  throw new Error('an unstructured mesh must be required');
}
mesh.value = '/data/PearlRiver.nc';
mesh.oninput();
if (foot('下一步').disabled) throw new Error('an existing unstructured mesh must pass the wizard gate');
next(); choose('IGBP'); next(); next(); next(); next();
if (state.spatial?.domain?.kind !== 'region' || state.spatial?.grid?.kind !== 'unstructured'
    || state.spatial?.grid?.meshFile !== '/data/PearlRiver.nc' || 'dlon' in state.spatial?.grid
    || state.wizard?.spatial !== state.spatial) {
  throw new Error(`spatial contract was not preserved: ${JSON.stringify(state.spatial)}`);
}

showDomainGate();
choose('区域'); next(); choose('经纬度网格'); next();
for (const [id, value] of Object.entries({
  'spatial-west': '100', 'spatial-east': '110', 'spatial-south': '20', 'spatial-north': '30',
})) {
  const input = findNode(ids.gatecards, node => node.id === id);
  if (!input) throw new Error(`missing ${id}`);
  input.value = value;
  input.oninput();
}
// 不再有非海洋 mask：海洋按 RawData 的地表覆盖剔除（DEF_LANDONLY）。
if (findNode(ids.gatecards, node => node.id === 'spatial-nonOceanMask')) {
  throw new Error('the lat-lon wizard must not ask for a non-ocean mask');
}
const westLabel = findNode(ids.gatecards, node => node.htmlFor === 'spatial-west');
if (!westLabel) throw new Error('spatial input labels must be associated with their controls');
if (foot('下一步').disabled) throw new Error('valid regional bounds and resolution must pass without a mask');
if (!nodeText(ids.gatecards).includes('RawData 目录、Runtime 目录，以及强迫场')) {
  throw new Error('the spatial page must say where RawData, Runtime and forcing are chosen');
}
next(); choose('IGBP'); next(); next(); next(); next();
if (state.spatial?.domain?.west !== 100 || state.spatial?.grid?.kind !== 'latlon'
    || state.spatial?.grid?.dlon !== 0.5 || state.spatial?.grid?.nlon !== 720
    || state.wizard?.spatial !== state.spatial) {
  throw new Error(`lat-lon spatial contract was not preserved: ${JSON.stringify(state.spatial)}`);
}

showDomainGate();
choose('全球'); next(); choose('经纬度网格'); next();
if (!nodeText(ids.gatecards).includes('西=-180°，东=180°，南=-90°，北=90°')
    || findNode(ids.gatecards, node => node.id === 'spatial-west')) {
  throw new Error('global lat-lon bounds must be fixed and visible rather than editable');
}
if (findNode(ids.gatecards, node => node.id === 'spatial-nonOceanMask') || foot('下一步').disabled) {
  throw new Error('global lat-lon must pass without a non-ocean mask');
}
next(); choose('IGBP'); next(); next(); next(); next();
if (JSON.stringify(state.spatial?.domain) !== JSON.stringify({ kind: 'global', west: -180, east: 180, south: -90, north: 90 })) {
  throw new Error(`global lat-lon bounds were not persisted: ${JSON.stringify(state.spatial?.domain)}`);
}
// 流域 + 流域网格：只要流域网格文件，不要 Shapefile、边界或 mask；范围取自网格文件。
showDomainGate();
choose('流域'); next(); choose('流域网格'); next();
if (findNode(ids.gatecards, node => ['spatial-shapefile', 'spatial-west', 'spatial-nonOceanMask'].includes(node.id))) {
  throw new Error('a catchment grid must not ask for a shapefile, bounds or a mask');
}
const catchmentInput = findNode(ids.gatecards, node => node.id === 'spatial-catchmentFile');
if (!catchmentInput || !foot('下一步').disabled || !ids.gateinfo.textContent.includes('流域网格 NetCDF')) {
  throw new Error('a catchment grid must require only the catchment NetCDF');
}
catchmentInput.value = '/data/PearlRiver_250km2.nc';
catchmentInput.oninput();
if (foot('下一步').disabled) throw new Error('a catchment NetCDF alone must pass the wizard gate');
next(); choose('IGBP'); next(); next(); next(); next();
if (JSON.stringify(state.spatial?.domain) !== JSON.stringify({ kind: 'watershed' })
    || state.spatial?.grid?.kind !== 'catchment' || state.spatial?.grid?.input !== '/data/PearlRiver_250km2.nc') {
  throw new Error(`catchment spatial contract was not preserved: ${JSON.stringify(state.spatial)}`);
}

showDomainGate();
choose('站点');
next();
if (ids.gatetitle.textContent !== '次网格怎么分？') throw new Error('page 2 missing');
if (cards().map(c => c.children[0].textContent).join('|') !== 'USGS|IGBP|PFT|PC') {
  throw new Error('page 2 must list USGS, IGBP, PFT, and PC in order');
}
if (['USGS', 'IGBP', 'PFT', 'PC'].some(label => card(label).disabled)) {
  throw new Error('page 2 readiness does not match runtime support');
}
if (cards().some(c => /旧方案|patch|一个地类/.test(nodeText(c)))) {
  throw new Error('page 2 must not expose legacy or patch implementation wording');
}
choose('IGBP');
next();
if (ids.gatetitle.textContent !== '土壤水力用哪套？') throw new Error('page 3 missing');
if (/支持 TRACER|不需要 alpha_vgm|TRACER 是否可用/.test(
  [ids.gatesub.textContent, ids.gateinfo.textContent, ...cards().map(nodeText)].join(' '))) {
  throw new Error('page 3 must not expose tracer or parameter-requirement explanations');
}
if (!nodeText(card('van Genuchten–Mualem（Ippisch 2006）')).includes('启用 van Genuchten–Mualem 土壤水力模型')
    || !nodeText(card('Campbell（1974）')).includes('启用 Campbell 土壤水力模型')) {
  throw new Error('soil cards must use the same readable scheme names as the parameter selector');
}
if (cards().some(c => /\.true\.|\.false\.|默认：/.test(nodeText(c)))) {
  throw new Error('soil cards must not expose booleans or implementation-default wording');
}
if (card('van Genuchten–Mualem（Ippisch 2006）')?.getAttribute('aria-selected') !== 'true') {
  throw new Error('default soil card must be the code-default van Genuchten–Mualem scheme');
}
choose('Campbell（1974）');
next();
if (ids.gatetitle.textContent !== '还要打开哪些过程？') throw new Error('page 4 missing');
if (card('BGC').getAttribute('aria-disabled') !== 'true') throw new Error('BGC must require PFT or PC');
if (card('TRACER').getAttribute('aria-disabled') !== 'true') throw new Error('TRACER must require van Genuchten');
card('BGC').onclick();
if (ids.gatetitle.textContent !== '次网格怎么分？') throw new Error('blocked option did not link to page 2');
next(); next();
choose('URBAN');
if (card('LULCC').getAttribute('aria-disabled') !== 'true') {
  throw new Error('LULCC must be blocked for SinglePoint/site runs');
}
next();
if (ids.gatetitle.textContent !== '要打开调试吗？') throw new Error('page 5 missing');
if (!card('SrfdataDiag') || card('SrfdataDiag').getAttribute('aria-disabled') !== 'true') {
  throw new Error('SinglePoint surface-data diagnostics must stay visible but disabled');
}
choose('RangeCheck');
next();

if (!ids.domaingate.hidden) throw new Error('wizard did not finish');
if (state.step !== 'basic-files') throw new Error(`wizard finished at ${state.step}, not basic-files`);
ids.homeBtn.onclick();
if (ids.domaingate.hidden || ids.gatetitle.textContent !== '这次要跑什么？') {
  throw new Error('home button did not reopen the wizard at page 1');
}
if (state.wizard.soil !== 'campbell' || 'profile' in state.wizard) {
  throw new Error(`wrong wizard state: ${JSON.stringify(state.wizard)}`);
}
const fields = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_LCT: '.true.',
  DEF_USE_PFT: '.false.',
  DEF_USE_Campbell_SOIL_MODEL: '.true.',
  DEF_URBAN_RUN: '.true.',
  DEF_USE_LULCC: '.false.',
  DEF_USE_RangeCheck: '.true.',
})) {
  if (fields[path] !== value) throw new Error(`${path}: expected ${value}, got ${fields[path]}`);
}
// 区域单元流域汇流：选了河湖汇流的空间算例默认打开；没选河湖汇流、全球、LULCC、流域网格、站点保持不写或关闭。
{
  const regional = (wizard) => Object.fromEntries(wizardFields(wizard).map(x => [x.path, x.value]))
    .DEF_UnitCatchment_regional;
  const region = { kind: 'region', west: 113, east: 115, south: 23, north: 25 };
  const spatial = {
    ...state.wizard, grid: 'latlon', spatial: { domain: region, grid: { kind: 'latlon' } },
    physics: { ...state.wizard.physics, river: true },
  };
  const cases = [
    ['region', spatial, '.true.'],
    ['no routing', { ...spatial, physics: { ...spatial.physics, river: false } }, '.false.'],
    ['lulcc', { ...spatial, physics: { ...spatial.physics, lulcc: true } }, '.false.'],
    ['global', { ...spatial, spatial: { ...spatial.spatial, domain: { kind: 'global' } } }, '.false.'],
    ['catchment', { ...spatial, grid: 'catchment' }, undefined],
    ['site', { ...state.wizard, spatial: null }, undefined],
  ];
  for (const [name, wizard, want] of cases) {
    const got = regional(wizard);
    if (got !== want) throw new Error(`regional routing for ${name}: expected ${want}, got ${got}`);
  }
}
const usgs = Object.fromEntries(wizardFields({ ...state.wizard, subgrid: 'USGS' }).map(x => [x.path, x.value]));
if (usgs.DEF_USE_LCT !== '.true.' || usgs.DEF_USE_PFT !== '.false.' || usgs.DEF_USE_PC !== '.false.') {
  throw new Error(`wrong USGS structure fields: ${JSON.stringify(usgs)}`);
}
const pc = Object.fromEntries(wizardFields({ ...state.wizard, subgrid: 'PC' }).map(x => [x.path, x.value]));
if (pc.DEF_USE_LCT !== '.false.' || pc.DEF_USE_PFT !== '.false.' || pc.DEF_USE_PC !== '.true.') {
  throw new Error(`wrong PC structure fields: ${JSON.stringify(pc)}`);
}
for (const [subgrid, expected] of Object.entries({
  USGS: ['.true.', '.false.', '.false.'],
  IGBP: ['.true.', '.false.', '.false.'],
  PFT: ['.false.', '.true.', '.false.'],
  PC: ['.false.', '.false.', '.true.'],
})) {
  const urban = Object.fromEntries(wizardFields({
    ...state.wizard,
    subgrid,
    physics: { urban: true, bgc: false, tracer: false, lulcc: false },
  }).map(x => [x.path, x.value]));
  const actual = [urban.DEF_USE_LCT, urban.DEF_USE_PFT, urban.DEF_USE_PC];
  if (actual.join('|') !== expected.join('|') || urban.DEF_URBAN_RUN !== '.true.') {
    throw new Error(`${subgrid} + URBAN was rewritten: ${JSON.stringify(urban)}`);
  }
}
const owned = wizardFieldNames();
for (const path of ['DEF_USE_NITRIF', 'DEF_USE_FERT', 'DEF_USE_CNSOYFIXN', 'DEF_Aerosol_Readin']) {
  if (owned.includes(path)) throw new Error(`${path} is an editable process initial value, not a locked wizard field`);
}
const mainFields = withoutWizardFields([
  ...owned.map(path => ({ path })),
  { path: 'DEF_HIST_FREQ' },
]);
if (mainFields.map(x => x.path).join('|') !== 'DEF_HIST_FREQ') {
  throw new Error(`main page repeated wizard fields: ${JSON.stringify(mainFields)}`);
}


// Urban is a runtime process and must preserve all four runtime subgrid layouts.
showDomainGate();
choose('站点'); next(); choose('PC'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
if (card('URBAN').getAttribute('aria-disabled') === 'true') throw new Error('SinglePoint PC urban was incorrectly blocked');
choose('URBAN');
if (card('BGC').getAttribute('aria-disabled') !== 'true') throw new Error('pure urban SinglePoint must not allow BGC');
if (card('TRACER').getAttribute('aria-disabled') !== 'true') throw new Error('urban SinglePoint must not allow tracer/methane');
showDomainGate();
choose('站点'); next(); choose('IGBP'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
choose('URBAN');
if (card('BGC').getAttribute('aria-disabled') !== 'true') {
  throw new Error('pure urban SinglePoint must not allow BGC');
}
if (card('TRACER').getAttribute('aria-disabled') !== 'true') {
  throw new Error('urban SinglePoint must not allow tracer/methane');
}

// LULCC is not a SinglePoint/USGS/BGC option in the runnable desktop path.
showDomainGate();
choose('站点'); next(); choose('USGS'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
if (card('LULCC').getAttribute('aria-disabled') !== 'true' || !/USGS/.test(nodeText(card('LULCC')))) {
  throw new Error('LULCC must be blocked for USGS');
}
showDomainGate();
choose('站点'); next(); choose('PC'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
choose('BGC');
if (card('LULCC').getAttribute('aria-disabled') !== 'true' || !/BGC/.test(nodeText(card('LULCC')))) {
  throw new Error('LULCC must be blocked when BGC is enabled');
}
const bgcFields = Object.fromEntries(wizardFields({
  subgrid: 'PFT', soil: 'vg', tracer: null,
  physics: { urban: false, lulcc: false, bgc: true, crop: false, tracer: false },
  debug: { rangecheck: false, colmdebug: false, srfdatadiag: false },
}).map(x => [x.path, x.value]));
if (bgcFields.DEF_USE_NITRIF !== '.true.'
  || bgcFields.DEF_USE_FERT !== '.false.'
  || bgcFields.DEF_USE_CNSOYFIXN !== '.false.'
  || bgcFields.DEF_Aerosol_Readin !== '.false.') {
  throw new Error('natural BGC cases must disable crop-only nitrogen inputs');
}
const waterFields = Object.fromEntries(wizardFields({
  subgrid: 'IGBP', soil: 'vg', tracer: null,
  physics: { urban: false, lulcc: false, bgc: false, crop: false, tracer: false },
  debug: { rangecheck: false, colmdebug: false, srfdatadiag: false },
}).map(x => [x.path, x.value]));
for (const name of ['DEF_USE_NITRIF', 'DEF_USE_FERT', 'DEF_USE_CNSOYFIXN', 'DEF_Aerosol_Readin']) {
  if (waterFields[name] !== '.false.') throw new Error(`ordinary natural cases must disable ${name}`);
}

showDomainGate();
choose('站点'); next(); choose('PFT'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
if (card('CROP').getAttribute('aria-disabled') !== 'true') throw new Error('CROP must require BGC');
choose('BGC');
if (card('CROP').getAttribute('aria-disabled') === 'true') throw new Error('CROP did not unlock with PFT+BGC+CROP kernel');
choose('CROP');
next();
next();
const cropFields = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_BGC: '.true.',
  DEF_USE_TRACER: '.false.',
  DEF_USE_LAIFEEDBACK: '.true.',
  DEF_USE_FERT: '.false.',
  DEF_USE_CNSOYFIXN: '.false.',
  DEF_USE_IRRIGATION: '.false.',
  DEF_TUNING_CROP_PLANTING_DAY: '120',
})) {
  if (cropFields[path] !== value) throw new Error(`${path}: expected ${value}, got ${cropFields[path]}`);
}
if (kernelForSubgrid('PFT')?.dir !== '/crop-real') throw new Error('selected CROP wizard must resolve to the CROP kernel');

showDomainGate();
choose('站点'); next(); choose('PFT'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
// 示踪剂本身只要求 van Genuchten、非城市；BGC 与 PFT/PC 是甲烷自己的要求。
if (card('TRACER').getAttribute('aria-disabled') === 'true') throw new Error('TRACER must not require BGC any more');
choose('TRACER');
next();
if (ids.gatetitle.textContent !== '选择示踪剂类型') throw new Error('tracer type page missing');
if (cards().map(c => c.children[0].textContent).join('|') !== '水同位素|甲烷 CH₄|溶质|泥沙') {
  throw new Error('tracer page must list isotope, methane, solute, and sediment');
}
for (const label of ['甲烷 CH₄', '泥沙']) {
  if (card(label).getAttribute('aria-disabled') !== 'true') throw new Error(`${label} must be disabled on a site without BGC`);
}
for (const label of ['水同位素', '溶质']) {
  if (card(label).getAttribute('aria-disabled') === 'true') throw new Error(`${label} must be open on a site`);
}
if (!/只能用于空间算例/.test(nodeText(card('泥沙')))) throw new Error('sediment must explain it is spatial only');
if (!/BGC/.test(nodeText(card('甲烷 CH₄')))) throw new Error('methane must explain it needs BGC');
previous();
choose('BGC');
next();
choose('甲烷 CH₄');
choose('水同位素');
choose('溶质');
if (card('甲烷 CH₄').getAttribute('aria-pressed') !== 'true' || card('水同位素').getAttribute('aria-pressed') !== 'true') {
  throw new Error('tracer page must be multi-select');
}
next();
if (ids.gatetitle.textContent !== '水同位素怎么算？') throw new Error('isotope tracer must ask for its mode');
if (card('开分馏（IsoGSM 驱动）').getAttribute('aria-disabled') !== 'true') throw new Error('site isotope cannot fractionate');
if (card('不分馏').getAttribute('aria-selected') !== 'true') throw new Error('site isotope must default to no fractionation');
const mixingBox = card('含水层混合水量（mm）');
const mixingInput = mixingBox?.children.find(c => c.id === 'isotope-mixing');
if (mixingInput?.value !== '1000') throw new Error('aquifer mixing water must be prefilled with 1000 mm');
if (!/测试值，正式模拟请用实测或率定/.test(nodeText(mixingBox))) throw new Error('prefilled mixing water must be labelled a test value');
mixingInput.value = '0';
mixingInput.oninput();
if (!foot('下一步').disabled) throw new Error('non-positive mixing water must block the isotope page');
mixingInput.value = '1500';
mixingInput.oninput();
next();
if (ids.gatetitle.textContent !== '溶质浓度怎么设？') throw new Error('solute tracer must ask for its concentrations');
const soluteBox = card('初始浓度（kg Cl / kg 水）');
const soluteInit = soluteBox?.children.find(c => c.id === 'solute-init');
const solutePrecip = soluteBox?.children.find(c => c.id === 'solute-precip');
if (soluteInit?.value !== '1.0e-5' || solutePrecip?.value !== '2.0e-6') throw new Error('solute concentrations must be prefilled');
if (!/只是测试值，正式模拟请用实测值/.test(nodeText(soluteBox))) throw new Error('prefilled solute concentrations must be labelled test values');
soluteInit.value = '';
soluteInit.oninput();
if (!foot('下一步').disabled) throw new Error('an empty solute concentration must block the page');
soluteInit.value = '-1';
soluteInit.oninput();
if (!foot('下一步').disabled) throw new Error('a negative solute concentration must block the page');
soluteInit.value = '3e-5';
soluteInit.oninput();
next();
if (ids.gatetitle.textContent !== '甲烷淹水范围怎么算？') throw new Error('methane tracer must ask for the inundation mode');
// 单点没有河网，也还不知道是不是湿地站点：只留 satellite 与 wetwat，缺省 wetwat。
for (const label of ['混合（hybrid）', '河网洪泛（routing）', '动态地下水位（dynamic_wtd）']) {
  if (card(label).getAttribute('aria-disabled') !== 'true') throw new Error(`${label} must be blocked on a site`);
}
if (card('湿地蓄水（wetwat）').getAttribute('aria-selected') !== 'true') throw new Error('site methane must default to wetwat');
previous(); previous(); previous(); previous();
if (card('LULCC').getAttribute('aria-disabled') !== 'true') throw new Error('LULCC must be blocked while isotopes are selected');
next(); next(); next(); next();
next();
if (ids.gatetitle.textContent !== '要打开调试吗？') throw new Error('methane page did not continue to debug');
next();
if (state.wizard.tracer !== 'methane,isotope,solute') throw new Error(`wrong tracer state: ${JSON.stringify(state.wizard)}`);
const methaneFields = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_TRACER: '.true.',
  DEF_USE_BGC: '.true.',
  DEF_TRACER_NUM: '4',
  DEF_TRACER_NAMES: 'CH4,H2_18O,HDO,Cl',
  DEF_TRACER_TYPES: 'gas,isotope,isotope,solute',
  DEF_TRACER_MRAT: '16.04,20.0,19.0,35.453',
  DEF_TRACER_REF_RATIO: '1.0,2.0052e-3,1.5576e-4,1.0',
  DEF_TRACER_INIT_DELTA: '0.0,-10.0,-70.0,0.0',
  DEF_TRACER_REACTIVE_DECAY_RATE: '0.0,0.0,0.0,0.0',
  DEF_TRACER_PARAM_FILES: 'CH4:standard_ch4_parameter.nml,H2_18O:standard_O18_parameter.nml,HDO:standard_HDO_parameter.nml,Cl:standard_chloride_parameter.nml',
  'DEF_TRACER%init_conc': '3e-5',
  'DEF_TRACER%precip_default_conc': '2.0e-6',
  DEF_TRACER_USE_FRACTIONATION: '.false.',
  DEF_TRACER_AQUIFER_MIXING_WATER_MM: '1500.0',
  DEF_USE_Dynamic_Wetland: '.false.',
  'DEF_METHANE%inundation_mode': 'wetwat',
})) {
  if (methaneFields[path] !== value) throw new Error(`${path}: expected ${value}, got ${methaneFields[path]}`);
}
// 空间算例缺省开分馏；只选同位素时不写甲烷，也不强开 BGC。
const isoOnly = Object.fromEntries(wizardFields({
  ...state.wizard, domain: 'global', tracer: 'isotope', isotopeMode: 'fractionation', isotopeMixing: 1000,
  physics: { ...state.wizard.physics, bgc: false },
}).map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_TRACER: '.true.', DEF_USE_BGC: '.false.', DEF_TRACER_NUM: '2', DEF_TRACER_NAMES: 'H2_18O,HDO',
  DEF_TRACER_USE_FRACTIONATION: '.true.', DEF_TRACER_AQUIFER_MIXING_WATER_MM: '1000.0',
})) {
  if (isoOnly[path] !== value) throw new Error(`isotope-only ${path}: expected ${value}, got ${isoOnly[path]}`);
}
if ('DEF_METHANE%inundation_mode' in isoOnly) throw new Error('isotope-only must not write methane settings');
// 空间算例：泥沙要先开河湖汇流；关掉河湖汇流时已选的泥沙被去掉。
showDomainGate();
choose('区域'); next(); choose('经纬度网格'); next();
for (const [id, value] of Object.entries({
  'spatial-west': '100', 'spatial-east': '110', 'spatial-south': '20', 'spatial-north': '30',
})) {
  const input = findNode(ids.gatecards, node => node.id === id);
  input.value = value;
  input.oninput();
}
next(); choose('IGBP'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
if (card('河湖汇流').getAttribute('aria-pressed') === 'true') choose('河湖汇流');
choose('TRACER');
next();
if (card('泥沙').getAttribute('aria-disabled') !== 'true' || !/河湖汇流/.test(nodeText(card('泥沙')))) {
  throw new Error('spatial sediment must ask for river-lake routing');
}
previous();
choose('河湖汇流');
next();
choose('泥沙');
if (card('泥沙').getAttribute('aria-pressed') !== 'true') throw new Error('sediment must be selectable with river-lake routing');
previous();
choose('河湖汇流');
next();
if (card('泥沙').getAttribute('aria-pressed') === 'true') throw new Error('turning routing off must drop sediment');

// 泥沙：particle 类型的 SEDIMENT，参数文件用上游标准文件；不写溶质浓度。
const sedOnly = Object.fromEntries(wizardFields({
  ...state.wizard, domain: 'global', tracer: 'sediment',
  physics: { ...state.wizard.physics, bgc: false, river: true },
}).map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_TRACER: '.true.', DEF_TRACER_NUM: '1', DEF_TRACER_NAMES: 'SEDIMENT', DEF_TRACER_TYPES: 'particle',
  DEF_TRACER_PARAM_FILES: 'SEDIMENT:standard_sediment_parameter.nml',
})) {
  if (sedOnly[path] !== value) throw new Error(`sediment-only ${path}: expected ${value}, got ${sedOnly[path]}`);
}
if ('DEF_TRACER%init_conc' in sedOnly) throw new Error('sediment-only must not write solute concentrations');
// 动态湿地留在参数页，由后端按淹没方案置灰并说明，不再被向导整个藏起来。
if (wizardFieldNames().includes('DEF_USE_Dynamic_Wetland')) {
  throw new Error('dynamic wetland must stay visible (read-only) on the parameters page');
}


// 改第 2 页时，第 3/4 页的无关选择保留并重算约束。
showDomainGate();
choose('站点'); next(); choose('IGBP'); next(); choose('van Genuchten–Mualem（Ippisch 2006）'); next();
choose('URBAN');
previous(); previous();
choose('PC');
next();
if (card('van Genuchten–Mualem（Ippisch 2006）').getAttribute('aria-selected') !== 'true') {
  throw new Error('soil choice was not preserved after changing subgrid');
}
next();
if (card('URBAN').getAttribute('aria-pressed') !== 'true') throw new Error('urban choice was lost across a runtime subgrid change');
if (card('URBAN').getAttribute('aria-disabled') === 'true') throw new Error('SinglePoint PC urban must remain selectable');
if (card('BGC').getAttribute('aria-disabled') !== 'true') throw new Error('pure urban SinglePoint must keep BGC blocked');
next();
next();
const pcFieldsAfterUrban = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
if (pcFieldsAfterUrban.DEF_USE_PC !== '.true.' || pcFieldsAfterUrban.DEF_USE_LCT !== '.false.'
  || pcFieldsAfterUrban.DEF_URBAN_RUN !== '.true.') {
  throw new Error(`PC retained invalid urban state: ${JSON.stringify(pcFieldsAfterUrban)}`);
}

// Spatial scope and computation grid are independent and survive the full wizard.
showDomainGate();
choose('流域'); next(); choose('经纬度网格'); next();
const shapefile = findNode(ids.gatecards, node => node.id === 'spatial-shapefile');
shapefile.value = '/data/basin.shp'; shapefile.oninput(); next(); choose('IGBP'); next();
choose('van Genuchten–Mualem（Ippisch 2006）'); next(); next(); next();
if (state.domain !== 'watershed' || state.grid !== 'latlon' || state.wizard.grid !== 'latlon') {
  throw new Error(`spatial domain/grid state was lost: ${JSON.stringify(state.wizard)}`);
}
if (kernelForSubgrid()?.dir !== '/latlon-real') throw new Error('latlon wizard did not select the GRIDBASED kernel');
if (kernelForSubgrid('IGBP', { grid: 'unstructured' })?.dir !== '/unstructured-real') {
  throw new Error('unstructured grid did not select the UNSTRUCTURED kernel');
}
if (kernelForSubgrid('IGBP', { grid: 'catchment' })?.dir !== '/catchment-real') {
  throw new Error('watershed grid did not select the CATCHMENT kernel');
}

console.log('gate: domain/grid cards, five site pages, constraints, finish state, and namelist fields resolve');

const { go } = await import(moduleUrl('shell.js'));
state.domain = 'region';
state.selected = { name: 'spatial', dir: '/cases/spatial' };
state.cases = [state.selected];
state.createdCases.add('/cases/spatial');
state.step = 'basic-files';
go('result-uncertainty');
if (state.step === 'result-uncertainty' || !ids.status.textContent.includes('空间算例暂不支持参数率定和不确定性分析')) {
  throw new Error('spatial workflow must disable uncertainty-analysis navigation');
}
state.domain = 'site';
state.cases.push({ name: 'old-spatial', dir: '/cases/old-spatial', spatial: true });
state.text = '&nl_colm\n DEF_domain%edgew = -180.0 ! default site-template bound\n DEF_file_mesh = \"\"\n DEF_CatchmentMesh_data = \"\"\n/';
go('result-uncertainty');
if (state.step !== 'result-uncertainty') {
  throw new Error('switching back to site mode must re-enable Study navigation');
}
state.step = 'basic-files';
state.selected = { name: 'imported-spatial', dir: '/cases/imported-spatial', spatial: true };
state.cases = [state.selected];
state.createdCases = new Set(['/cases/imported-spatial']);
go('result-tuning');
if (state.step === 'result-tuning' || !ids.status.textContent.includes('空间算例暂不支持参数率定和不确定性分析')) {
  throw new Error('imported spatial case metadata must disable tuning navigation');
}

// 河湖汇流总开关由向导写（与 BGC 并列），并归向导所有，不在参数页重复出现。
{
  const region = { kind: 'region', west: 113, east: 115, south: 23, north: 25 };
  const fields = (river, grid = 'latlon') => Object.fromEntries(wizardFields({
    ...state.wizard, grid, spatial: { domain: region, grid: { kind: grid } },
    physics: { ...state.wizard.physics, river },
  }).map(x => [x.path, x.value]));
  if (fields(true).DEF_USE_GridRiverLakeFlow !== '.true.') throw new Error('river card on writes .true.');
  if (fields(false).DEF_USE_GridRiverLakeFlow !== '.false.') throw new Error('river card off writes .false.');
  if (fields(true, 'catchment').DEF_USE_GridRiverLakeFlow !== undefined) {
    throw new Error('catchment meshes do not get the grid routing switch');
  }
  if (!wizardFieldNames().includes('DEF_USE_GridRiverLakeFlow')) {
    throw new Error('the wizard owns the routing switch');
  }
}
// 甲烷淹没方案写进字段表；动态湿地跟着方案走（dynamic_wtd / hybrid 打开）。
{
  const fields = methaneMode => Object.fromEntries(wizardFields({
    ...state.wizard, tracer: 'methane', methaneMode,
    physics: { ...state.wizard.physics, tracer: true, bgc: true, river: true },
  }).map(x => [x.path, x.value]));
  for (const [mode, dynamic] of [['hybrid', '.true.'], ['dynamic_wtd', '.true.'], ['routing', '.false.'], ['satellite', '.false.']]) {
    const f = fields(mode);
    if (f['DEF_METHANE%inundation_mode'] !== mode) throw new Error(`${mode} must reach the CH4 file`);
    if (f.DEF_USE_Dynamic_Wetland !== dynamic) throw new Error(`${mode} needs DEF_USE_Dynamic_Wetland = ${dynamic}`);
  }
}
// 河湖汇流卡片按空间类型提醒边界与河网的关系，不再写「不选则……」。
{
  const source = await import('node:fs').then(fs =>
    fs.readFileSync(new URL('../dist/app/domain.js', import.meta.url), 'utf8'));
  if (/不选则不算河道/.test(source)) throw new Error('the river card no longer explains the unchecked state');
  for (const key of ['watershed:', 'region:', 'unstructured:']) {
    if (!source.includes(key)) throw new Error(`the river card needs a ${key} note`);
  }
  if (!/n\.className = 'dnote'/.test(source)) throw new Error('river notes render as a card note');
}
if (!/picked\.grid === 'unstructured' \? 'unstructured' : picked\.domain/.test(
  await import('node:fs').then(fs => fs.readFileSync(new URL('../dist/app/domain.js', import.meta.url), 'utf8')),
)) throw new Error('unstructured meshes get their own routing note');
// 打开已有算例：首页与「文件与目录」都有入口，复用向导的 startSession，并换回上次用过的内核。
{
  const read = name => import('node:fs').then(fs =>
    fs.readFileSync(new URL(`../dist/${name}`, import.meta.url), 'utf8'));
  const [open, domainSrc, html] = await Promise.all([read('app/opencase.js'), read('app/domain.js'), read('index.html')]);
  if (!/id="open-case"/.test(html)) throw new Error('the files page offers to open an existing case');
  if (!/new Event\('colm:open-case'\)/.test(domainSrc)) throw new Error('the start gate offers to open an existing case');
  if (!/export function startSession\(config\)/.test(domainSrc)) throw new Error('the wizard and opened cases share startSession');
  if (!/startSession\(sessionFromProfile\(opened\.profile\)\)/.test(open)) throw new Error('an opened case starts a session from its profile');
  if (!/k\.preset === opened\.profile\.kernel_preset/.test(open)) throw new Error('an opened case returns to the kernel it last ran with');
}
{
  const open = await import('node:fs').then(fs => fs.readFileSync(new URL('../dist/app/opencase.js', import.meta.url), 'utf8'));
  const spatialSrc = await import('node:fs').then(fs => fs.readFileSync(new URL('../dist/app/spatial.js', import.meta.url), 'utf8'));
  if (!/await fillSpatialForm\(inputs, opened\.root\)/.test(open)) throw new Error('an opened spatial case fills the setup form');
  if (!/export async function fillSpatialForm\(inputs, root\) \{\n  await loadForcingDatasets\(\);/.test(spatialSrc)) {
    throw new Error('the forcing list loads before the opened values are filled in');
  }
}
// 全链路排查（向导组合 × colm-rs 预检）发现的四类问题。
{
  const crop = (spatial) => Object.fromEntries(wizardFields({
    ...state.wizard, subgrid: 'PFT', spatial,
    physics: { ...state.wizard.physics, bgc: true, crop: true },
  }).map(x => [x.path, x.value])).DEF_TUNING_CROP_PLANTING_DAY;
  if (crop(null) !== '120') throw new Error('site crop cases keep the planting-day override');
  if (crop({ domain: { kind: 'region' }, grid: { kind: 'latlon' } }) !== '0') {
    throw new Error('spatial crop cases must not set a planting-day override (SinglePoint only)');
  }
  const src = await import('node:fs').then(fs => fs.readFileSync(new URL('../dist/app/domain.js', import.meta.url), 'utf8'));
  if (!/picked\.grid === 'catchment' && \['urban', 'lulcc', 'tracer'\]\.includes\(item\.id\)/.test(src)) {
    throw new Error('catchment meshes block urban, LULCC and tracers');
  }
  if (!/if \(picked\.physics\.urban\) \{\n\s+return \{ need: '城市模式暂不支持示踪剂'/.test(src)) {
    throw new Error('urban blocks tracers for every domain, not only sites');
  }
}
