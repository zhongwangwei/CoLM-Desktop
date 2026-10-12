// 首页复用真实导航/恢复接口；这里隔离 I/O，检查等待、配置和会话边界。
import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const temp = await mkdtemp(join(tmpdir(), 'colm-homepage-'));
const source = await readFile(new URL('../dist/app/homepage.js', import.meta.url), 'utf8');
const deferred = () => {
  let resolve, reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
};

async function harness(hasBackend = true) {
  const nodes = new Map();
  class Element {
    constructor() { this.children = []; this.hidden = false; this.disabled = false; this.value = ''; this.attrs = {}; this.textContent = ''; }
    set id(value) { this._id = value; nodes.set(value, this); }
    get id() { return this._id; }
    setAttribute(key, value) { this.attrs[key] = value; }
    append(...children) { this.children.push(...children); }
    replaceChildren(...children) { this.children = children; }
  }
  const $ = id => nodes.get(id);
  for (const id of ['launchgate', 'loadinggate', 'domaingate', 'home-status', 'home-new-case', 'home-show-examples', 'home-resume', 'home-open-case', 'home-examples', 'home-example-list', 'home-recent-list', 'home-recent-root', 'home-recent-empty', 'root', 'homeBtn']) {
    const element = new Element(); element.id = id;
  }
  $('home-examples').hidden = true;
  $('root').value = ' /existing/project ';
  const events = new Map();
  const calls = [];
  const state = { wizard: null, cases: [], step: 'launch' };
  const h = {
    $, state, calls, hasBackend,
    invoke: async (...args) => { calls.push(['invoke', ...args]); return null; },
    openExistingCase: async (...args) => { calls.push(['open', ...args]); return { dir: args[0] }; },
    showDomainGate: () => calls.push(['domain']),
    startSession: config => { calls.push(['start', config]); state.wizard = { config }; },
    loadExample: async () => { calls.push(['load']); },
    sitesForWizard: () => ['CN-Cng', 'AT-Neu', 'AU-Preston', 'US-Ne3'].map(name => ({ name })),
    pickSite: site => calls.push(['pick', site.name]),
    kernelForSubgrid: () => 'installed',
    go: step => { calls.push(['go', step]); state.step = step; },
    status: message => calls.push(['status', message]),
  };
  globalThis.homeTest = h;
  globalThis.document = { createElement: () => new Element() };
  globalThis.addEventListener = (event, listener) => events.set(event, listener);
  const dir = await mkdtemp(join(temp, 'case-'));
  const modules = {
    'package.json': '{"type":"module"}',
    'homepage.js': source,
    'ipc.js': 'export const hasBackend = globalThis.homeTest.hasBackend; export const invoke = (...args) => globalThis.homeTest.invoke(...args);',
    'state.js': 'export const state = globalThis.homeTest.state;',
    'ui.js': 'export const $ = id => globalThis.homeTest.$(id); export const status = message => globalThis.homeTest.status(message);',
    'i18n.js': 'export const language = () => "zh"; export const translateZh = text => text;',
    'domain.js': 'export const showDomainGate = () => globalThis.homeTest.showDomainGate(); export const startSession = config => globalThis.homeTest.startSession(config);',
    'opencase.js': 'export const openExistingCase = (...args) => globalThis.homeTest.openExistingCase(...args);',
    'sites.js': 'export const loadExample = () => globalThis.homeTest.loadExample(); export const sitesForWizard = () => globalThis.homeTest.sitesForWizard(); export const pickSite = site => globalThis.homeTest.pickSite(site);',
    'kernel.js': 'export const kernelForSubgrid = (...args) => globalThis.homeTest.kernelForSubgrid(...args);',
    'shell.js': 'export const go = step => globalThis.homeTest.go(step);',
  };
  await Promise.all(Object.entries(modules).map(([name, body]) => writeFile(join(dir, name), body)));
  const home = await import(pathToFileURL(join(dir, 'homepage.js')).href);
  return { ...home, h, $, events, run: id => home.homeGuide.actions()[id].run() };
}

try {
  const { h, $, showHome, homeGuide, run, events } = await harness();
  showHome();
  assert.equal($('launchgate').hidden, false);
  assert.equal($('loadinggate').hidden, true);
  assert.equal($('domaingate').hidden, true);
  assert.equal($('home-resume').hidden, true);
  assert.equal($('home-recent-empty').hidden, false);
  assert.equal(h.calls.length, 0, 'opening home must not start or load anything');
  for (const id of ['new-case', 'open-existing-case', 'show-examples']) assert.ok(homeGuide.actions()[id]);
  run('show-examples');
  assert.equal(homeGuide.state().examples_visible, true);
  assert.equal($('home-show-examples').attrs['aria-expanded'], 'true');
  run('show-examples');
  assert.equal(homeGuide.state().examples_visible, false);
  run('new-case');
  assert.equal($('launchgate').hidden, true);
  assert.deepEqual(h.calls, [['domain']]);

  const wizard = { unsaved: true };
  h.state.wizard = wizard;
  h.state.selected = { dir: '/current/case' };
  h.state.step = 'parameters';
  h.state.cases = Array.from({ length: 8 }, (_, i) => ({ name: `case-${i}`, dir: `/project/case-${i}` }));
  events.get('colm:home')();
  assert.equal(h.state.wizard, wizard);
  assert.equal(h.state.selected.dir, '/current/case');
  assert.equal(h.state.step, 'parameters');
  assert.equal($('home-resume').hidden, false);
  assert.equal($('home-recent-root').textContent, '/existing/project');
  assert.deepEqual($('home-recent-list').children.map(button => button.title), h.state.cases.slice(0, 6).map(item => item.dir));
  assert.equal($('home-recent-empty').hidden, true);
  run('resume-task');
  assert.equal($('launchgate').hidden, true);
  assert.equal(h.state.wizard, wizard);
  showHome();
  await run('recent:/project/case-2');
  assert.deepEqual(h.calls.at(-1), ['open', '/project/case-2', 'basic-files', { strict: true }]);

  h.calls.length = 0;
  assert.equal(await run('open-existing-case'), null, 'cancel does not restore/create a case');
  assert.deepEqual(h.calls, [['invoke', 'pick_folder', { key: 'open-case' }]]);
  assert.equal($('launchgate').hidden, false);
  assert.equal(h.state.wizard, wizard);
  const picker = deferred(), opening = deferred();
  h.invoke = () => picker.promise;
  h.openExistingCase = (...args) => { h.calls.push(['open', ...args]); return opening.promise; };
  let finished = false;
  const operation = run('open-existing-case').then(value => { finished = true; return value; });
  assert.equal(homeGuide.state().busy, true);
  assert.equal($('home-new-case').disabled, true);
  assert.equal($('home-open-case').disabled, true);
  assert.equal($('home-recent-list').children[0].disabled, true);
  await assert.rejects(run('open-existing-case'), /等待/);
  assert.throws(() => run('new-case'), /等待/);
  picker.resolve('/chosen');
  await Promise.resolve(); await Promise.resolve();
  assert.equal(finished, false, 'folder choice is not completion of restoring a case');
  assert.deepEqual(h.calls.at(-1), ['open', '/chosen', 'basic-files', { strict: true }]);
  opening.resolve({ dir: '/chosen' });
  assert.deepEqual(await operation, { dir: '/chosen' });
  assert.equal(homeGuide.state().busy, false);
  assert.equal($('home-open-case').disabled, false);
  h.invoke = async () => '/broken';
  h.openExistingCase = async () => { throw new Error('restore failed'); };
  await assert.rejects(run('open-existing-case'), /restore failed/);
  assert.equal($('home-status').textContent, 'restore failed');
  assert.equal(homeGuide.state().busy, false);

  const expected = {
    'CN-Cng': { subgrid: 'IGBP', physics: {} },
    'AT-Neu': { subgrid: 'PFT', physics: { bgc: true, tracer: true }, tracer: 'methane', methaneMode: 'wetwat' },
    'AU-Preston': { subgrid: 'IGBP', physics: { urban: true } },
    'US-Ne3': { subgrid: 'PFT', physics: { bgc: true, crop: true } },
  };
  for (const [name, config] of Object.entries(expected)) {
    showHome(); h.calls.length = 0;
    const loading = deferred();
    h.loadExample = () => { h.calls.push(['load']); return loading.promise; };
    let done = false;
    const preparing = run(`example:${name}`).then(result => { done = true; return result; });
    assert.deepEqual(h.calls[0], ['start', { domain: 'site', grid: null, spatial: null, soil: 'vg', ...config }]);
    assert.equal(homeGuide.actions()[`example:${name}`].kind, 'commit');
    assert.equal(homeGuide.state().busy, true);
    assert.equal(done, false);
    assert.equal($('launchgate').hidden, false);
    assert.equal(h.calls.some(call => call[0] === 'pick'), false);
    loading.resolve();
    assert.deepEqual(await preparing, { site: name, step: 'basic-files', started: false });
    assert.equal($('launchgate').hidden, true);
    assert.deepEqual(h.calls.map(call => call[0]), ['start', 'go', 'load', 'pick', 'status'], 'preparing only configures and selects input; it never creates or runs');
    assert.deepEqual(h.calls[3], ['pick', name]);
  }
  showHome(); h.calls.length = 0;
  h.kernelForSubgrid = () => null;
  events.get('colm:kernels')();
  assert.equal($('home-example-AT-Neu').disabled, true);
  await assert.rejects(run('example:AT-Neu'), /缺少.*内核/);
  assert.equal(h.calls.some(call => call[0] === 'start'), false);
  h.kernelForSubgrid = () => 'installed';
  h.loadExample = async () => { h.state.wizard = { replaced: true }; };
  await assert.rejects(run('example:CN-Cng'), /工作流已变化/);
  assert.equal($('launchgate').hidden, false);
  h.calls.length = 0;
  h.loadExample = async () => {};
  h.sitesForWizard = () => [];
  await assert.rejects(run('example:CN-Cng'), /没有找到所选站点/);
  assert.equal(h.calls.some(call => call[0] === 'pick'), false);
  assert.equal(homeGuide.state().busy, false);
  assert.equal($('launchgate').hidden, false);

  const offline = await harness(false);
  offline.h.state.cases = [{ name: 'saved', dir: '/saved' }];
  offline.showHome();
  assert.equal(offline.$('home-open-case').disabled, true);
  assert.equal(offline.$('home-recent-list').children[0].disabled, true);
  for (const name of Object.keys(expected)) assert.equal(offline.$(`home-example-${name}`).disabled, true);
  assert.equal(offline.$('home-new-case').disabled, false);
  assert.equal(offline.$('home-show-examples').disabled, false);
  assert.match(offline.$('home-status').textContent, /桌面软件/);
  console.log('homepage: session preservation, entry points, real recents, strict async operations and four example configurations passed');
} finally {
  await rm(temp, { recursive: true, force: true });
  delete globalThis.homeTest;
  delete globalThis.document;
  delete globalThis.addEventListener;
}
