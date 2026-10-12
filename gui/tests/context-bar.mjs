import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
const root = new URL('../', import.meta.url);
const temp = await mkdtemp(join(tmpdir(), 'colm-context-'));
await cp(new URL('dist/app', root), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}');
class Element {
  constructor(props = {}) { Object.assign(this, { children: [], dataset: {}, options: [], value: '', listeners: {} }, props); }
  append(...children) { this.children.push(...children); this.options = this.children.filter(c => c instanceof Option); }
  appendChild(child) { this.append(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  contains(node) { return this === node || this.children.some(child => child.contains?.(node)); }
  querySelector(selector) {
    const key = selector.match(/data-run-choice="([^"]+)"/)?.[1];
    return this.children.find(child => key && child.dataset?.runChoice === key)
      ?? this.children.map(child => child.querySelector?.(selector)).find(Boolean) ?? null;
  }
  focus() { document.activeElement = this; }
  setAttribute(key, value) { this[key] = value; }
  addEventListener(type, fn) { (this.listeners[type] ??= []).push(fn); }
  dispatchEvent(event) { this['on' + event.type]?.(event); for (const fn of this.listeners[event.type] ?? []) fn(event); }
}
globalThis.Option = class extends Element {
  constructor(text, value, _default, selected = false) { super({ textContent: text, value, selected }); }
};
const elements = Object.fromEntries(['context-bar', 'model-version', 'run-target', 'model-engine', 'casename', 'run-mode-choices', 'run-mode-next'].map(id => [id, new Element()]));
elements['run-target'].append(new Option('本机', 'local'), new Option('srv', 'srv'));
elements['run-target'].value = 'local';
elements['model-engine'].append(new Option('Rust', 'rust'), new Option('Fortran', 'fortran'));
elements['model-engine'].value = 'rust';
globalThis.document = { createElement: () => new Element() };
globalThis.addEventListener = () => {};
globalThis.localStorage = { getItem: () => null, setItem() {} };
for (const [name, body] of Object.entries({
  'ui.js': 'export const $ = id => globalThis.contextElements[id] ?? globalThis.contextElements["context-bar"].children.flatMap(c => c.children).find(c => c.id === id); export const status = () => {}; export const baseName = p => p;',
  'ipc.js': 'export const hasBackend = true; export const invoke = async cmd => cmd === "workspace_list" ? { workspaces: [{name:"snow",touches:{engine:true},lights:{compile:"pass",tests:"pass"}}] } : {name:"snow",ready:true,items:[]};',
  'i18n.js': 'export const language = () => "zh"; export const translateZh = s => s;',
  'engine.js': 'export const modelEngine = () => globalThis.contextElements["model-engine"].value;',
  'kernel.js': 'export const kernelForSubgrid = () => ({dir:"/kernel"});',
  'shell.js': 'export const go = step => { globalThis.contextNavigation = step; }; export const renderSteps = () => {};',
  'remote.js': 'export const configuredServers = () => [{host:"srv"}];',
  'workspace.js': 'export const openWorkspaces = () => {};',
})) await writeFile(join(temp, 'app', name), body);
globalThis.contextElements = elements;
const view = await import(pathToFileURL(join(temp, 'app/run-mode-view.js')));
const { state } = await import(pathToFileURL(join(temp, 'app/state.js')));
await view.refreshVersions();
state.selected = { name: 'CN-Cng', dir: '/case/CN-Cng' };
state.batch = [{}, {}, {}];
view.renderContextBar();
assert.equal(elements['context-bar'].children.length, 1, 'header only shows the case');
const caseButton = elements['context-bar'].children[0];
assert.equal(caseButton.children[1].textContent, 'CN-Cng 等 3 个');
assert.equal(caseButton.title, '/case/CN-Cng');
caseButton.onclick(); assert.equal(globalThis.contextNavigation, 'basic-files');
assert.equal(elements['model-version'].value, 'official', 'rendering the case must not change run choices');
let bubbled = false;
elements['model-engine'].addEventListener('change', event => { bubbled = event.bubbles; });
const cards = elements['run-mode-choices'];
const group = key => cards.querySelector(`[data-run-choice="${key}"]`);
group('version').children[1].onclick();
assert.equal(elements['model-version'].value, 'snow');
assert.equal(state.modelVersion, 'snow');
await new Promise(resolve => setImmediate(resolve));
assert.equal(elements['context-bar'].children.length, 1);
assert.equal(group('version').children[1]['aria-pressed'], 'true');
group('target').children[0].onclick();
assert.equal(elements['run-target'].value, 'local');
assert.equal(group('target').children[0]['aria-pressed'], 'true');
group('engine').children[0].onclick();
assert.equal(elements['model-engine'].value, 'rust');
await new Promise(resolve => setImmediate(resolve));
assert.equal(group('engine').children[0]['aria-pressed'], 'true');
assert.equal(elements['run-mode-next'].disabled, false);
group('target').children[1].onclick();
assert.equal(elements['run-target'].value, 'srv');
group('engine').children[1].onclick();
assert.equal(elements['model-engine'].value, 'fortran');
assert.equal(bubbled, true, 'card selection retains delegated engine warnings');
elements['run-target'].disabled = true;
group('target').children[0].onclick();
assert.equal(elements['run-target'].value, 'srv', 'disabled canonical choice remains protected');
view.renderContextBar();
assert.equal(elements['context-bar'].children[0].children[1].textContent, 'CN-Cng 等 3 个');
assert.equal(elements['model-engine'].value, 'fortran');
const html = await readFile(new URL('dist/index.html', root), 'utf8');
assert.ok(html.slice(html.indexOf('<header'), html.indexOf('</header>')).includes('id="context-bar"'));
assert.equal((html.match(/id="context-bar"/g) ?? []).length, 1);
const header = html.slice(html.indexOf('<header'), html.indexOf('</header>'));
assert.ok(!header.includes('context-select-'));
const settings = header.slice(header.indexOf('id="view-settings"'));
for (const id of ['langSeg', 'modeSeg', 'liveToggle', 'themeBtn']) assert.ok(settings.includes(`id="${id}"`));
assert.ok(!settings.includes(' open'), 'view settings start folded');
console.log('context-bar: case-only header and preserved run-mode cards');
