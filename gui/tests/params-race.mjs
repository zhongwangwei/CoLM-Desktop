import { cp, mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

class El {
  constructor(id = '') {
    this.id = id;
    this.children = [];
    this.dataset = {};
    this.style = {};
    this.className = '';
    this.value = '';
    this.disabled = false;
    this.hidden = false;
    this.checked = false;
    this._text = '';
    this._html = '';
  }
  get textContent() { return this._text; }
  set textContent(value) { this._text = String(value); if (value === '') this.children = []; }
  get innerHTML() { return this._html; }
  set innerHTML(value) { this._html = String(value); this.children = []; }
  appendChild(child) { this.children.push(child); return child; }
  append(...items) { this.children.push(...items); }
  replaceChildren(...items) { this.children = items; }
  remove() { this.removed = true; }
  setAttribute(name, value) { this[name] = String(value); }
  getAttribute(name) { return this[name] ?? null; }
  addEventListener() {}
  querySelector() { return null; }
  querySelectorAll() { return []; }
}

const elements = new Map();
const el = id => {
  if (!elements.has(id)) elements.set(id, new El(id));
  return elements.get(id);
};
globalThis.window = globalThis;
globalThis.document = {
  getElementById: el,
  createElement: tag => new El(tag),
  createTextNode: text => ({ textContent: String(text) }),
  querySelectorAll: () => [],
};
globalThis.addEventListener = () => {};
globalThis.requestAnimationFrame = fn => { setTimeout(fn, 0); return 1; };

let stale = false;
let readCaseCalled = false;
let invokeMode = 'case-text';
let firstTimingResolve;
let timingCalls = 0;
let firstHistResolve;
let histCalls = 0;
globalThis.__TAURI__ = { core: { invoke: async (command, args) => {
  if (invokeMode === 'timing-race' && command === 'read_timing') {
    timingCalls += 1;
    if (timingCalls === 1) {
      return await new Promise(resolve => { firstTimingResolve = resolve; });
    }
    return {
      count: 1, window_varies: false, start: '2008-01-01', end: '2009-01-01',
      spinup_years: 2, spinup_repeat: 1, spinup_varies: false,
      output_start: '2010-01-01', total_steps: 1,
    };
  }
  if (invokeMode === 'hist-race') {
    if (command === 'read_timing') return {
      count: 1, window_varies: false, start: '2008-01-01', end: '2009-01-01',
      spinup_years: 0, spinup_repeat: 0, spinup_varies: false,
      output_start: '2008-01-01', total_steps: 1,
    };
    if (command === 'read_case') return [];
    if (command === 'land_cover_contexts') return [];
    if (command === 'varying_fields') return [];
    if (command === 'field_states_batch') return [];
    if (command === 'hist_vars') {
      histCalls += 1;
      if (histCalls === 1) {
        return await new Promise(resolve => { firstHistResolve = resolve; });
      }
      return [{ name: 'NEW', on: true, writable: true, settable: true }];
    }
    return [];
  }
  if (command === 'read_timing') return {
    count: 1, window_varies: false, start: '2008-01-01', end: '2009-01-01',
    spinup_years: 0, spinup_repeat: 0, spinup_varies: false,
    output_start: '2008-01-01', total_steps: 1,
  };
  if (command === 'read_text') {
    stale = true;
    return '&nl_colm\n DEF_CASE_NAME=\'A\'\n/\n';
  }
  if (command === 'read_case') {
    readCaseCalled = true;
    throw new Error('stale renderFields reached read_case');
  }
  return [];
} }, event: { listen: () => () => {} } };

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-params-race-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const moduleUrl = name => pathToFileURL(join(temp, 'app', name)).href;
const { state } = await import(moduleUrl('state.js'));
const { renderFields } = await import(moduleUrl('params.js'));

state.selected = { name: 'A', dir: '/case/A' };
state.batch = ['/case/A'];
state.createdCases.add('/case/A');
state.cases = [state.selected];
state.text = '&nl_colm\n DEF_CASE_NAME=\'old\'\n/\n';
state.fields = [];
el('kernel').value = '/kernel';

await renderFields(() => !stale);
if (readCaseCalled) throw new Error('stale renderFields continued after read_text');
if (state.text.includes("DEF_CASE_NAME='A'")) throw new Error('stale renderFields overwrote state.text');

console.log('params: stale renderFields stops after async case text load');


invokeMode = 'timing-race';
stale = false;
readCaseCalled = false;
timingCalls = 0;
state.text = '';
el('timing').children = [];
const first = renderFields();
await Promise.resolve();
const second = renderFields();
await second;
firstTimingResolve({
  count: 1, window_varies: false, start: '2008-01-01', end: '2009-01-01',
  spinup_years: 9, spinup_repeat: 1, spinup_varies: false,
  output_start: '2017-01-01', total_steps: 1,
});
await first;
const timingHtml = el('timing').children[0]?.innerHTML ?? '';
if (!timingHtml.includes('value="2"') || timingHtml.includes('value="9"')) {
  throw new Error('stale default renderFields timing response overwrote the newer render');
}

console.log('params: default renderFields ignores stale timing responses');

invokeMode = 'hist-race';
stale = false;
readCaseCalled = false;
histCalls = 0;
state.selected = null;
state.batch = ['/case/B'];
state.cases = [];
state.createdCases.clear();
state.text = '&nl_colm\n/\n';
state.fields = [];
el('kernel').value = '/kernel';
el('hist-fields').children = [];
const staleHist = renderFields();
for (let i = 0; histCalls < 1 && i < 20; i++) await new Promise(resolve => setTimeout(resolve, 0));
if (histCalls !== 1) throw new Error('test did not reach first hist_vars call');
const currentHist = renderFields();
await currentHist;
firstHistResolve([{ name: 'OLD', on: true, writable: true, settable: true }]);
await staleHist;
const textTree = node => [node?.textContent ?? '', ...(node?.children ?? []).flatMap(textTree)].join(' ');
const histText = textTree(el('hist-fields'));
if (!histText.includes('NEW') || histText.includes('OLD')) {
  throw new Error('stale hist_vars response overwrote the newer render');
}

console.log('params: hist vars renderer ignores stale responses');
