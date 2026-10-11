import assert from 'node:assert/strict';
import { mkdtemp, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

const temp = await mkdtemp(join(tmpdir(), 'colm-hybrid-guide-'));
try {
  await writeFile(join(temp, 'package.json'), '{"type":"module"}');
  await writeFile(join(temp, 'hybrid.js'), await readFile(new URL('../dist/app/hybrid.js', import.meta.url)));
  const stubs = {
    'ipc.js': 'export const hasBackend=true; export const invoke=(...args)=>globalThis.backend(...args);',
    'state.js': 'export const state=globalThis.testState;',
    'ui.js': 'export const $=id=>document.getElementById(id); export const appConfirm=async()=>true; export const status=()=>{}; export const baseName=p=>p.split("/").at(-1);',
    'batch.js': 'export const batchTarget=()=>globalThis.cases;',
    'shell.js': 'export const go=step=>globalThis.navigate(step);',
    'engine.js': 'export const modelEngine=()=>"rust";',
    'i18n.js': 'export const language=()=>"zh"; export const translateZh=s=>s;',
  };
  for (const [file, source] of Object.entries(stubs)) await writeFile(join(temp, file), source);
  const nodes = new Map();
  const selectors = new Map();
  function node(id = '', value = '') {
    const n = { id, value, dataset: {}, children: [], disabled: false, textContent: '',
      append(...children) { this.children.push(...children); }, appendChild(child) { this.children.push(child); },
      replaceChildren(...children) { this.children = children; this.textContent = children.map(x => x.textContent).join(''); },
      querySelectorAll() { return []; }, querySelector() { return null; },
      setAttribute() {}, addEventListener() {}, classList: { toggle() {}, contains() { return false; } },
    };
    if (id) nodes.set(id, n);
    return n;
  }
  globalThis.document = { getElementById: id => nodes.get(id), querySelectorAll: selector => selectors.get(selector) ?? [], querySelector: () => null, createElement: () => node() };
  globalThis.addEventListener = () => {};
  globalThis.testState = { step: 'basic-files' };
  globalThis.cases = [{ dir: '/case', name: 'case' }];
  globalThis.navigate = () => {};
  const { hybridGuide, refreshHybridCard } = await import(pathToFileURL(join(temp, 'hybrid.js')));
  assert.deepEqual(hybridGuide.actions(), {});
  testState.step = 'hybrid-learn';
  for (const id of ['hybrid-fit-refresh', 'hybrid-fit-run', 'hybrid-fit-install', 'hybrid-install', 'hybrid-fit-result', 'hybrid-fit-studies', 'hybrid-go-tuning']) node(id);
  for (const [id, value] of Object.entries({ 'hybrid-fit-root': '/project', 'hybrid-fit-name': 'network', 'hybrid-fit-ridge': '0.1', kernel: '/kernel', 'tune-hybrid-slot': 'pft', 'tune-hybrid-features': 'pftclass', 'tune-hybrid-size': 'linear', 'hybrid-model-path': '/model.onnx', 'hybrid-slot': 'pft', 'hybrid-features': 'pftclass', 'hybrid-normalize-path': '' })) node(id, value);
  node('hybrid-force'); node('hybrid-outside');
  const outputs = [{ read: () => ({ name: 'DEF_PFT_VMAX25', lo: 10, hi: 100, transform: 'sigmoid' }) }];
  node('tune-hybrid-outputs').children = outputs;
  node('hybrid-outputs').children = outputs;
  nodes.get('hybrid-fit-studies').querySelectorAll = () => [{ dataset: { study: '/study' } }];
  let called;
  globalThis.backend = async (command, args) => { called = { command, args }; throw new Error('backend failed'); };
  const actions = hybridGuide.actions();
  assert.equal(actions['hybrid-fit-refresh'].kind, 'click');
  assert.equal(actions['hybrid-fit-run'].kind, 'commit');
  assert.equal(actions['hybrid-install'].kind, 'commit');
  await assert.rejects(actions['hybrid-fit-refresh'].run(), /backend failed/);
  await assert.rejects(actions['hybrid-fit-run'].run(), /backend failed/);
  assert.equal(called.command, 'hybrid_fit');
  assert.deepEqual(called.args.request.studies, ['/study']);
  assert.equal(nodes.get('hybrid-fit-run').disabled, false);
  assert.match(nodes.get('hybrid-fit-result').textContent, /backend failed/);
  assert.equal(hybridGuide.state().fit, null);
  await assert.rejects(actions['hybrid-fit-install'].run(), /先完成拟合/);
  await assert.rejects(actions['hybrid-install'].run(), /backend failed/);
  assert.equal(called.command, 'hybrid_install');
  globalThis.backend = async command => {
    assert.equal(command, 'hybrid_fit');
    return { model: '/project/ai-models/network.mlp.json', normalize: '/norm.json', report: { rows: 4, validation: { passed: false, network_rmse: [2], mean_predictor_rmse: [1] } } };
  };
  await actions['hybrid-fit-run'].run();
  assert.equal(hybridGuide.state().fit.validation.passed, false);
  assert.equal(hybridGuide.state().fit.model, '/project/ai-models/network.mlp.json');
  await assert.rejects(actions['hybrid-fit-install'].run(), /交叉验证未通过/);
  globalThis.backend = async () => { throw new Error('retry failed'); };
  await assert.rejects(actions['hybrid-fit-run'].run(), /retry failed/);
  assert.equal(hybridGuide.state().fit, null, 'failed retries must not expose the earlier model as a fresh success');
  assert.throws(() => actions['hybrid-go-tuning'].run(), /尚不可用/);
  globalThis.navigate = step => { testState.step = step; };
  let toggled = false;
  node('tune-hybrid-on').click = () => { toggled = true; };
  actions['hybrid-go-tuning'].run();
  assert.equal(testState.step, 'result-tuning');
  assert.equal(toggled, true);
  assert.deepEqual(hybridGuide.actions(), {});
  testState.step = 'hybrid-process';
  assert.ok(!hybridGuide.actions()['hybrid-fit-run']);
  assert.ok(hybridGuide.actions()['hybrid-install']);
  assert.equal(hybridGuide.state().mode, 'process');
  // Complete the background refresh first while strict page preparation is still awaiting IPC.
  const host = node('hybrid-cases');
  const pending = [];
  globalThis.backend = (command, args) => {
    assert.equal(command, 'hybrid_info');
    assert.equal(args.case, '/case');
    return new Promise(resolve => pending.push(resolve));
  };
  const preparation = hybridGuide.prepare();
  const rejected = assert.rejects(preparation, /任务上下文已变化/);
  const background = refreshHybridCard();
  assert.equal(pending.length, 2, 'both requests must overlap before either resolves');
  pending[1](JSON.stringify({ land_mode: 'lct', installed: false, slots: [] }));
  await background;
  const newestTable = host.children[0];
  assert.ok(newestTable, 'the newer refresh must actually render');
  pending[0](JSON.stringify({ land_mode: 'pft', installed: false, slots: [] }));
  await rejected;
  assert.equal(host.children[0], newestTable, 'superseded preparation must not overwrite the latest page');
  console.log('hybrid-guide: built-in actions, failure propagation, mode restrictions, navigation and superseded strict preparation verified');
} finally {
  await rm(temp, { recursive: true, force: true });
}
