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
  constructor(props = {}) { Object.assign(this, { children: [], options: [], value: '', listeners: {} }, props); }
  append(...children) { this.children.push(...children); this.options = this.children.filter(c => c instanceof Option); }
  appendChild(child) { this.append(child); }
  replaceChildren(...children) { this.children = []; this.append(...children); }
  focus() { document.activeElement = this; }
  setAttribute(key, value) { this[key] = value; }
  addEventListener(type, fn) { (this.listeners[type] ??= []).push(fn); }
  dispatchEvent(event) { this['on' + event.type]?.(event); for (const fn of this.listeners[event.type] ?? []) fn(event); }
}
globalThis.Option = class extends Element {
  constructor(text, value, _default, selected = false) { super({ textContent: text, value, selected }); }
};
const elements = Object.fromEntries(['context-bar', 'model-version', 'run-target', 'model-engine', 'casename'].map(id => [id, new Element()]));
elements['run-target'].append(new Option('本机', 'local'), new Option('srv', 'srv'));
elements['run-target'].value = 'local';
elements['model-engine'].append(new Option('Rust', 'rust'), new Option('Fortran', 'fortran'));
elements['model-engine'].value = 'rust';
globalThis.document = { createElement: () => new Element() };
globalThis.addEventListener = () => {};
globalThis.localStorage = { getItem: () => null, setItem() {} };
for (const [name, body] of Object.entries({
  'ui.js': 'export const $ = id => globalThis.contextElements[id] ?? globalThis.contextElements["context-bar"].children.flatMap(c => c.children).find(c => c.id === id); export const status = () => {}; export const baseName = p => p;',
  'ipc.js': 'export const hasBackend = false; export const invoke = async () => ({});',
  'i18n.js': 'export const language = () => "zh"; export const translateZh = s => s;',
  'engine.js': 'export const modelEngine = () => globalThis.contextElements["model-engine"].value;',
  'kernel.js': 'export const kernelForSubgrid = () => null;',
  'shell.js': 'export const go = () => {};',
  'workspace.js': 'export const openWorkspaces = () => {};',
})) await writeFile(join(temp, 'app', name), body);
globalThis.contextElements = elements;
const view = await import(pathToFileURL(join(temp, 'app/run-mode-view.js')));
const { state } = await import(pathToFileURL(join(temp, 'app/state.js')));
elements['model-version'].append(new Option('snow', 'snow'));
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
const html = await readFile(new URL('dist/index.html', root), 'utf8');
assert.ok(html.slice(html.indexOf('<header'), html.indexOf('</header>')).includes('id="context-bar"'));
assert.equal((html.match(/id="context-bar"/g) ?? []).length, 1);
console.log('context-bar: shared selects and two-way version changes');
