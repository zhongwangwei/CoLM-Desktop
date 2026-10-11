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
  'shell.js': 'export const go = () => {}; export const renderSteps = () => {};',
  'remote.js': 'export const configuredServers = () => [{host:"srv"}];',
  'workspace.js': 'export const openWorkspaces = () => {};',
})) await writeFile(join(temp, 'app', name), body);
globalThis.contextElements = elements;
const view = await import(pathToFileURL(join(temp, 'app/run-mode-view.js')));
const { state } = await import(pathToFileURL(join(temp, 'app/state.js')));
await view.refreshVersions();
view.renderContextBar();
assert.equal(elements['context-bar'].children.length, 4);
const headerVersion = elements['context-bar'].children[1].children[1];
headerVersion.focus();
headerVersion.value = 'snow'; headerVersion.dispatchEvent(new Event('change'));
assert.equal(elements['model-version'].value, 'snow');
assert.equal(state.modelVersion, 'snow');
assert.equal(document.activeElement, elements['context-bar'].children[1].children[1]);
// A second keyboard-style change remains on the replacement control.
document.activeElement.value = 'official'; document.activeElement.dispatchEvent(new Event('change'));
assert.equal(state.modelVersion, 'official');
assert.equal(document.activeElement.id, 'context-select-version');
elements['model-version'].value = 'official'; elements['model-version'].dispatchEvent(new Event('change'));
assert.equal(elements['context-bar'].children[1].children[1].value, 'official');
let bubbled = false;
elements['model-engine'].addEventListener('change', event => { bubbled = event.bubbles; });
const headerEngine = elements['context-bar'].children[3].children[1];
headerEngine.value = 'fortran'; headerEngine.dispatchEvent(new Event('change'));
assert.equal(elements['model-engine'].value, 'fortran');
assert.equal(bubbled, true, 'delegated engine warnings must see the underlying select change');
const headerTarget = elements['context-bar'].children[2].children[1];
headerTarget.value = 'srv'; headerTarget.dispatchEvent(new Event('change'));
assert.equal(elements['run-target'].value, 'srv');
const cards = elements['run-mode-choices'];
const group = key => cards.querySelector(`[data-run-choice="${key}"]`);
group('version').children[1].onclick();
assert.equal(elements['model-version'].value, 'snow');
assert.equal(state.modelVersion, 'snow');
await new Promise(resolve => setImmediate(resolve));
assert.equal(elements['context-bar'].children[1].children[1].value, 'snow');
assert.equal(group('version').children[1]['aria-pressed'], 'true');
group('target').children[0].onclick();
assert.equal(elements['run-target'].value, 'local');
assert.equal(elements['context-bar'].children[2].children[1].value, 'local');
group('engine').children[0].onclick();
assert.equal(elements['model-engine'].value, 'rust');
await new Promise(resolve => setImmediate(resolve));
assert.equal(elements['context-bar'].children[3].children[1].value, 'rust');
assert.equal(elements['run-mode-next'].disabled, false);
const html = await readFile(new URL('dist/index.html', root), 'utf8');
assert.ok(html.slice(html.indexOf('<header'), html.indexOf('</header>')).includes('id="context-bar"'));
assert.equal((html.match(/id="context-bar"/g) ?? []).length, 1);
console.log('context-bar: shared selects and two-way version changes');
