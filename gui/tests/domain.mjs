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
  }
  get textContent() { return this._text; }
  set textContent(value) {
    this._text = String(value);
    if (value === '') this.children = [];
  }
  appendChild(child) { this.children.push(child); return child; }
  setAttribute(name, value) { this.attributes[name] = String(value); }
  getAttribute(name) { return this.attributes[name] ?? null; }
}

const ids = Object.fromEntries([
  'gatetitle', 'gatesub', 'gateinfo', 'gatecards', 'gatefoot', 'domaingate',
  'launchgate', 'localRunCard', 'steps', 'status', 'estSite', 'casename', 'kernel', 'homeBtn',
].map(id => [id, new El()]));

globalThis.document = {
  getElementById: id => ids[id] ?? new El(),
  createElement: tag => new El(tag),
  querySelectorAll: () => [],
};
globalThis.window = globalThis;
globalThis.requestAnimationFrame = () => 0;

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-gate-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');

const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;
await import(moduleUrl('gate-boot.js'));
if (ids.launchgate.hidden || !ids.domaingate.hidden) throw new Error('launch page must appear before the model wizard');
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

showDomainGate();
if (ids.gatetitle.textContent !== '这次要跑什么？') throw new Error('page 1 missing');
if (cards().map(c => c.children[0].textContent).join('|') !== '站点|流域|区域|全球') {
  throw new Error('page 1 must list site, watershed, regional, and global in order');
}
if (card('流域').getAttribute('aria-disabled') !== 'true' || !card('流域').disabled) {
  throw new Error('watershed must be visible but temporarily unavailable');
}
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
if (card('TRACER').getAttribute('aria-disabled') !== 'true') throw new Error('TRACER must require BGC');
choose('BGC');
if (card('TRACER').getAttribute('aria-disabled') === 'true') throw new Error('TRACER did not unlock after BGC');
choose('TRACER');
next();
if (ids.gatetitle.textContent !== '选择示踪剂类型') throw new Error('tracer type page missing');
if (cards().map(c => c.children[0].textContent).join('|') !== '水同位素|甲烷 CH₄|溶质|泥沙') {
  throw new Error('tracer page must list isotope, methane, solute, and sediment');
}
for (const label of ['水同位素', '溶质', '泥沙']) {
  if (card(label).getAttribute('aria-disabled') !== 'true') throw new Error(`${label} must be disabled for now`);
}
if (!/单点站点不可用/.test(nodeText(card('泥沙')))) throw new Error('sediment must explain SinglePoint is unavailable');
choose('甲烷 CH₄');
next();
if (ids.gatetitle.textContent !== '要打开调试吗？') throw new Error('tracer page did not continue to debug');
next();
if (state.wizard.tracer !== 'methane') throw new Error(`wrong tracer state: ${JSON.stringify(state.wizard)}`);
const methaneFields = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_TRACER: '.true.',
  DEF_USE_BGC: '.true.',
  DEF_TRACER_NUM: '1',
  DEF_TRACER_NAMES: 'CH4',
  DEF_TRACER_TYPES: 'gas',
  DEF_TRACER_MRAT: '16.04',
  DEF_TRACER_REF_RATIO: '1.0',
  DEF_TRACER_INIT_DELTA: '0.0',
  DEF_TRACER_REACTIVE_DECAY_RATE: '0.0',
  DEF_TRACER_PARAM_FILES: 'CH4:standard_ch4_parameter.nml',
  DEF_USE_Dynamic_Wetland: '.false.',
})) {
  if (methaneFields[path] !== value) throw new Error(`${path}: expected ${value}, got ${methaneFields[path]}`);
}
if (!wizardFieldNames().includes('DEF_USE_Dynamic_Wetland')) {
  throw new Error('methane hydrology constraint was repeated on the main parameters page');
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

console.log('gate: five base pages plus conditional tracer page, constraints, finish state, and namelist fields resolve');
