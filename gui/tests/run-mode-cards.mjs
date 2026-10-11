import assert from 'node:assert/strict';
import { cp, mkdtemp, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const temp = await mkdtemp(join(tmpdir(), 'colm-run-cards-'));
await cp(new URL('../dist/app/', import.meta.url), join(temp, 'app'), { recursive:true });
await writeFile(join(temp,'package.json'), '{"type":"module"}');
await writeFile(join(temp,'app','i18n.js'), "export const language = () => 'zh'; export const translateZh = value => value;");
class Node {
  constructor(tag) { this.tagName = tag; this.children = []; this.dataset = {}; this.attributes = {}; this.textContent = ''; }
  append(...nodes) { this.children.push(...nodes); }
  replaceChildren(...nodes) { this.children = nodes; }
  contains(node) { return node === this || this.children.some(child=>child.contains(node)); }
  querySelector(selector) {
    const key = selector.match(/data-run-choice="(.*?)"/)?.[1];
    return this.children.find(child=>child.dataset.runChoice === key)
      ?? this.children.map(child=>child.querySelector(selector)).find(Boolean) ?? null;
  }
  setAttribute(name,value) { this.attributes[name] = value; }
  focus() { document.activeElement = this; }
  click() { if (!this.disabled) this.onclick?.(); }
}
globalThis.document = { createElement:tag=>new Node(tag), activeElement:null };
Object.defineProperty(globalThis,'navigator',{value:{hardwareConcurrency:12},configurable:true});
const storage = new Map();
globalThis.localStorage = {getItem:key=>storage.get(key),setItem:(key,value)=>storage.set(key,value)};
globalThis.dispatchEvent = () => {};
globalThis.CustomEvent = class {};
try {
  const {renderRunModeCards} = await import(pathToFileURL(join(temp,'app','run-mode-cards.js')));
  const {recordProbe} = await import(pathToFileURL(join(temp,'app','remote-probe-cache.js')));
  const container = new Node('div'); container.id = 'mode';
  const servers = [{host:'7920ts',root:'/data/colm'}, {host:'unseen',root:'/data/colm'}, {host:'failed',root:'/data/colm'}];
  recordProbe(servers[0],{cpus:64,schedulers:['slurm']});
  recordProbe(servers[2],null,'SSH rejected');
  const versions = [
    {name:'pass',touches:{engine:true},lights:{compile:'pass',tests:'pass'}},
    {name:'fail',lights:{compile:'pass',tests:'pass',parity:'fail'}},
    {name:'unknown',lights:{compile:'pass'}},
  ];
  const selection = {version:'official',target:'local',engine:'rust'}, calls = [];
  const options = {versions,servers,selection,onChange:(key,value)=>{calls.push([key,value]);selection[key]=value;renderRunModeCards(container,options);}};
  const group = key=>container.querySelector(`[data-run-choice="${key}"]`);
  const words = node=>[node.textContent,...node.children.map(words)].join(' ');
  renderRunModeCards(container,options);
  assert.equal(container.children.length,3);
  assert.match(words(group('version').children[1]),/✓ 检查已通过/);
  assert.match(words(group('version').children[1]),/改了 Rust 引擎/);
  assert.match(words(group('version').children[2]),/✕ 检查没通过/,'any failure wins over compile/test passes');
  assert.match(words(group('version').children[3]),/! 还没测试/);
  assert.match(words(group('target').children[0]),/12 个逻辑 CPU/);
  assert.match(words(group('target').children[1]),/64 个逻辑 CPU · slurm.*在线（上次测试）/);
  assert.match(words(group('target').children[2]),/没测过/);
  assert.match(words(group('target').children[3]),/连接失败（上次测试）/);
  for (const key of ['version','target','engine']) {
    const button = group(key).children[1]; button.focus(); const id = button.id;
    button.click();
    assert.equal(calls.at(-1)[0],key);
    if (key === 'target') assert.deepEqual(calls.at(-1), ['target','7920ts']);
    assert.equal(group(key).children[1].attributes['aria-pressed'],'true');
    assert.equal(group(key).children[0].attributes['aria-pressed'],'false');
    assert.equal(document.activeElement.id,id);
    assert.notEqual(document.activeElement,button,'focus must follow replacement button');
  }
  assert.equal(calls.length,3);
  options.lockEngine = 'rust'; selection.engine = 'rust';
  renderRunModeCards(container,options);
  group('engine').children[1].click();
  assert.equal(calls.length,3,'locked engine must not call onChange');
  assert.match(words(group('engine').children[1]),/已锁定引擎/);
  assert.equal(group('engine').children[0].attributes['aria-pressed'],'true');
  assert.equal(container.children.length,3,'rerender must reuse existing groups');
  servers[0].host = 'changed'; renderRunModeCards(container,options);
  assert.match(words(group('target').children[1]),/没测过/,'modified config invalidates probe facts');
  assert.doesNotMatch(words(group('target').children[1]),/64|slurm/);
  console.log('run-mode cards: choices, badges, cached facts, lock and focus passed');
} finally { await rm(temp,{recursive:true,force:true}); }
