import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

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
  'steps', 'status', 'estSite', 'estKernel', 'casename', 'kernel',
].map(id => [id, new El()]));

globalThis.document = {
  getElementById: id => ids[id] ?? new El(),
  createElement: tag => new El(tag),
  querySelectorAll: () => [],
};

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-gate-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');

const { showDomainGate, wizardFields } = await import(join(temp, 'app', 'domain.js'));
const { state } = await import(join(temp, 'app', 'state.js'));

const cards = () => ids.gatecards.children;
const card = label => cards().find(c => c.children[0]?.textContent === label);
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
choose('站点');
next();
if (ids.gatetitle.textContent !== '这次研究什么过程？') throw new Error('page 2 missing');
choose('自定义');
next();
if (ids.gatetitle.textContent !== '次网格怎么分？') throw new Error('page 3 missing');
choose('IGBP');
next();
if (ids.gatetitle.textContent !== '土壤水力用哪套？') throw new Error('page 4 missing');
choose('Campbell');
next();
if (ids.gatetitle.textContent !== '还要打开哪些过程？') throw new Error('page 5 missing');
if (card('BGC').getAttribute('aria-disabled') !== 'true') throw new Error('BGC must require PFT or PC');
if (card('TRACER').getAttribute('aria-disabled') !== 'true') throw new Error('TRACER must require van Genuchten');
card('BGC').onclick();
if (ids.gatetitle.textContent !== '次网格怎么分？') throw new Error('blocked option did not link to page 3');
next(); next();
choose('URBAN');
choose('LULCC');
next();
if (ids.gatetitle.textContent !== '要打开调试吗？') throw new Error('page 6 missing');
choose('RangeCheck');
next();

if (!ids.domaingate.hidden) throw new Error('wizard did not finish');
if (state.wizard.profile !== 'custom' || state.wizard.soil !== 'campbell') {
  throw new Error(`wrong wizard state: ${JSON.stringify(state.wizard)}`);
}
const fields = Object.fromEntries(wizardFields().map(x => [x.path, x.value]));
for (const [path, value] of Object.entries({
  DEF_USE_LCT: '.true.',
  DEF_USE_PFT: '.false.',
  DEF_USE_Campbell_SOIL_MODEL: '.true.',
  DEF_URBAN_RUN: '.true.',
  DEF_USE_LULCC: '.true.',
  DEF_USE_RangeCheck: '.true.',
})) {
  if (fields[path] !== value) throw new Error(`${path}: expected ${value}, got ${fields[path]}`);
}

// 预设只填初值，仍逐页经过 3–6；改第 3 页时第 4/5 页的无关选择保留并重算约束。
showDomainGate();
choose('站点'); next(); choose('自定义'); next(); choose('IGBP'); next(); choose('van Genuchten–Mualem'); next();
choose('LULCC');
previous(); previous();
choose('PFT');
next();
if (card('van Genuchten–Mualem').getAttribute('aria-selected') !== 'true') {
  throw new Error('soil choice was not preserved after changing subgrid');
}
next();
if (card('LULCC').getAttribute('aria-pressed') !== 'true') throw new Error('unrelated physics choice was lost');
if (card('BGC').getAttribute('aria-disabled') === 'true') throw new Error('BGC did not become available for PFT');

showDomainGate();
choose('站点'); next(); choose('碳氮循环'); next();
if (card('PFT').getAttribute('aria-selected') !== 'true') throw new Error('carbon preset did not select PFT');
next(); next();
if (card('BGC').getAttribute('aria-pressed') !== 'true') throw new Error('carbon preset did not enable BGC');

console.log('gate: six pages, constraints, finish state, and namelist fields resolve');
