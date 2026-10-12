// AI 助手面板（assistant.js）的纯函数：页面上下文、事件解析、回答分块。
import assert from 'node:assert/strict';
import { cp, mkdtemp, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-assistant-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
// Expose startup only in this copied test module; exercise configuration through a stub IPC.
await writeFile(join(temp, 'app', 'assistant.js'),
  (await readFile(join(temp, 'app', 'assistant.js'), 'utf8')) + '\nexport { ensureStarted, ui, send, refreshApiModels, startNewConversation, refreshInputHistory, renderChoices, refreshBackendStatus, externalModels, currentView, refreshVisibleContext, watchVisibleContext, restorePanel, resizePanel, togglePanel, renderAnswer, setRunning };\n');
await writeFile(join(temp, 'app', 'ipc.js'), `
  export const invoke = (...args) => globalThis.assistantInvoke(...args);
  export const listen = async () => {};
  export const hasBackend = () => true;
`);

globalThis.window = {};
globalThis.document = { getElementById: () => null, querySelectorAll: () => [], documentElement: { lang: 'zh' } };
globalThis.addEventListener = () => {};
const assistant = await import(pathToFileURL(join(temp, 'app', 'assistant.js')).href);
assert.equal(assistant.ui.conversation, null, 'restart starts with a new conversation');
assert.equal(assistant.ui.pendingNewConversation, true);

assert.equal(
  assistant.viewContext({ step: 'result', caseDir: '/p/A', kernel: '/k', root: '' }),
  'page: result\nselected case: /p/A\nkernel: /k',
);
assert.equal(assistant.viewContext({}), '');
assert.equal(assistant.viewContext({ root: '/p' }), 'file operations directory: /p');
assert.equal(assistant.viewContext({ variable: 'f_lfevpa' }), 'selected history variable: f_lfevpa');
for (const [page, workflow] of [['run', 'startup'], ['result-diagnostics', 'closure'], ['result-evaluation', 'flux'], ['result-tuning', 'calibration'], ['research', 'parity']]) {
  assert.ok(assistant.pagePrompts(page).some(p => p.prompt.includes(`workflow=${workflow}`)), page);
}
assert.equal(assistant.taskPhase('run_status'), 'checkdata');
assert.equal(assistant.taskPhase('series_stats'), 'localise');
assert.equal(assistant.taskPhase('parity_check'), 'validate');
assert.equal(assistant.taskPhase('write_text_file'), 'report');
assert.match(assistant.taskProgressText({ state: 'interrupted', phase: 'validate', requires_reconciliation: true,
  actions: [{ state: 'unconfirmed' }, { state: 'success' }] }), /已中断.*1\/2.*不自动重放/);
assert.match(assistant.taskProgressText({ state: 'answered', phase: 'report', actions: [] }), /验证以工具证据为准/);
for (const [view, expected] of [
  [{}, ''],
  [{ root: '  /explicit  ', caseDir: '/p/A' }, '/explicit'],
  [{ root: '   ', caseDir: '/p/A' }, '/p'],
  [{ root: '/tmp/project\\archive' }, '/tmp/project\\archive'],
  [{ caseDir: '/tmp/project\\archive/A' }, '/tmp/project\\archive'],
  [{ caseDir: '/tmp/project\\archive' }, '/tmp'],
  [{ root: 'C:\\projects\\archive' }, 'C:/projects/archive'],
  [{ root: '\\\\server\\share\\archive' }, '//server/share/archive'],
  [{ caseDir: '/p/A/' }, '/p'],
  [{ caseDir: '/A' }, '/'],
  [{ caseDir: '/' }, '/'],
  [{ caseDir: 'C:\\projects\\A' }, 'C:/projects'],
  [{ caseDir: 'C:\\A' }, 'C:/'],
  [{ caseDir: 'C:\\' }, 'C:/'],
  [{ caseDir: '\\\\server\\share\\A' }, '//server/share'],
  [{ caseDir: '\\\\server\\share\\' }, '//server/share'],
]) assert.equal(assistant.projectRoot(view), expected, JSON.stringify(view));

// 具体步骤与这一页的关键信息也附上；步骤与页面同名时不重复。
assert.equal(
  assistant.viewContext({ step: 'result', flow: 'result-evaluation', caseDir: '/p/A', details: ['x: 1'] }),
  'page: result\nworkflow step: result-evaluation\nselected case: /p/A\nx: 1',
);
assert.equal(assistant.viewContext({ step: 'run', flow: 'run' }), 'page: run');
{
  const lines = assistant.pageDetails('result-evaluation', {
    metrics: [{ name: 'Qle', n: 100, nse: 0.41, kge: 0.5, rmse: 30.2, bias: -12 }],
    batch: 3,
  });
  assert.equal(lines[0], 'cases in this batch: 3');
  assert.ok(lines.some(line => line.includes('Qle') && line.includes('NSE=0.410')), lines.join('\n'));
  assert.deepEqual(assistant.pageDetails('run', { metrics: [{ name: 'Qle', nse: 0.4 }] }), []);
  assert.ok(assistant.pageDetails('result-tuning', { badges: { tuning: '运行中 3/9' } })
    .includes('calibration Study status: 运行中 3/9'));
  assert.ok(assistant.pageDetails('hybrid-process')[0].includes('process replacement'));
  // 运行方式快照来自可见运行方式页；检查清单只在运行相关的页上附上。
  const runMode = { version: 'snow', target: 'local', engine: 'rust', ready: false, readiness: [{ kind: 'block', text: 'engine not built' }] };
  const onRunMode = assistant.pageDetails('run-mode', { runMode });
  assert.equal(onRunMode[0], 'run mode: model version modified version (development workspace) snow; location local; engine rust; NOT ready to run');
  assert.equal(onRunMode[1], '- version check [block] engine not built');
  assert.equal(assistant.pageDetails('result-series', { runMode }).length, 1);
  assert.match(assistant.pageDetails('basic-files', { runMode: { version: 'official', target: 'srv', engine: 'fortran', ready: true } })[0], /official release; location srv; engine fortran$/);
  // 结果页说出这批结果实际由哪一版跑出。
  const provenance = { ok: true, engine: 'rust', version: { kind: 'workspace', name: 'snow', head: '0123456789' } };
  assert.ok(assistant.pageDetails('result-overview', { provenance })
    .includes('these results were produced by modified version snow (commit 01234567) with the rust engine (run completed)'));
  assert.deepEqual(assistant.pageDetails('run', { provenance }), []);
}
// 提问建议跟着步骤走；没有建议的步骤返回空。
assert.ok(assistant.pagePrompts('result-evaluation').length >= 2);
assert.ok(assistant.pagePrompts('params-water')[0].prompt.includes('过程参数'));
assert.ok(assistant.pagePrompts('basic-files').length === 1);
assert.deepEqual(assistant.pagePrompts('prep-site'), []);
assert.deepEqual(assistant.pagePrompts(undefined), []);
for (const step of ['run-mode', 'run', 'research', 'result-tuning', 'result-uncertainty', 'hybrid-learn', 'hybrid-process', 'result-diagnostics']) {
  for (const { label, prompt } of assistant.pagePrompts(step)) assert.ok(label && prompt, step);
}

assert.deepEqual(assistant.parseEvent('{"type":"turn_done","content":"x","steps":1}'), { type: 'turn_done', content: 'x', steps: 1 });
assert.equal(assistant.parseEvent('not json'), null);
assert.equal(assistant.parseEvent('{"no":"type"}'), null);

const blocks = assistant.answerBlocks([
  'GPP is low at two sites.',
  'See the table:',
  '',
  '| site | KGE |',
  '|---|---:|',
  '| A | 0.6 |',
  '| B | -0.2 |',
  '',
  '```',
  'colm-cli metrics A',
  '```',
  'Done.',
].join('\n'));
assert.deepEqual(blocks.map(b => b.kind), ['text', 'table', 'code', 'text']);
assert.equal(blocks[0].text, 'GPP is low at two sites.\nSee the table:');
assert.deepEqual(blocks[1].rows, [['site', 'KGE'], ['A', '0.6'], ['B', '-0.2']]);
assert.equal(blocks[2].text, 'colm-cli metrics A');
// 模型给的文本里的 HTML 只是文字：块里原样保留，渲染时走 textContent。
assert.deepEqual(assistant.answerBlocks('<img src=x onerror=alert(1)>'), [{ kind: 'text', text: '<img src=x onerror=alert(1)>' }]);

// 页面接线：用到的元素都在 index.html 里；渲染不用 innerHTML。
const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
const source = await readFile(join(root, 'dist', 'app', 'assistant.js'), 'utf8');
for (const [, id] of source.matchAll(/\$\('([\w-]+)'\)/g)) {
  if (['kernel', 'root'].includes(id)) continue;
  if (id === 'workspace-detail') {
    assert.match(await readFile(join(root, 'dist', 'app', 'workspace.js'), 'utf8'), /id: 'workspace-detail'/);
    continue;
  }
  assert.ok(html.includes(`id="${id}"`), `index.html has no #${id}`);
}
assert.ok(!/\.(innerHTML|outerHTML)\s*=|insertAdjacentHTML/.test(source), 'assistant.js must not use innerHTML');
assert.ok(!source.includes('restoreLatest'), 'opening the panel must not restore a previous session');
assert.match(html, /aria-describedby="assistant-input-help"/);
for (const site of ['CN-Cng', 'AT-Neu', 'AU-Preston', 'US-Ne3']) {
  assert.ok(html.includes(`data-prompt="我想试试自带的 ${site}`), site);
}
assert.ok(html.includes('点击只会填入问题'));
// 助手栏宽度：至少 320，并给主页面留至少 480。
assert.equal(assistant.clampAssistantWidth(200, 1600, 250), 320);
assert.equal(assistant.clampAssistantWidth(600, 1600, 250), 600);
assert.equal(assistant.clampAssistantWidth(1200, 1600, 250), 870);
assert.equal(assistant.clampAssistantWidth(500, 900, 250), 320);
assert.equal(assistant.caseFromResult('create_case', '{"case":"/p/A","created":true}'), '/p/A');
assert.equal(assistant.caseFromResult('create_case', '{"case":"/p/A","created":false}'), null);
assert.equal(assistant.caseFromResult('metrics', '{"case":"/p/A"}'), null);
assert.equal(assistant.caseFromResult('create_case', 'error: x'), null);
// 桌面窗口里的 WebView 不弹系统对话框（直接当作取消）：前端一律用 ui.js 的 appConfirm / appPrompt。
import { readdir } from 'node:fs/promises';
for (const file of await readdir(join(root, 'dist', 'app'))) {
  if (!file.endsWith('.js')) continue;
  const code = (await readFile(join(root, 'dist', 'app', file), 'utf8')).replace(/\/\/.*$/gm, '').replace(/\/\*[\s\S]*?\*\//g, '');
  assert.ok(!/\b(window|globalThis)\.(confirm|prompt|alert)\b|(^|[^.\w])(confirm|prompt|alert)\(/m.test(code), `${file} uses a native dialog`);
}
console.log('assistant: context, events, answer blocks and page wiring ok');

// 输入框下的“思考”选框与设置之间的换算：不思考关闭思考模式，其余只设强度。
assert.equal(assistant.thinkValue({ thinking: false, reasoning_effort: 'max' }), 'off');
assert.equal(assistant.thinkValue({ thinking: null, reasoning_effort: 'low' }), 'low');
assert.equal(assistant.thinkValue({ thinking: true, reasoning_effort: null }), 'on');
assert.deepEqual(assistant.thinkSettings('off'), { thinking: false, reasoning_effort: null });
assert.deepEqual(assistant.thinkSettings('max'), { thinking: null, reasoning_effort: 'max' });
assert.deepEqual(assistant.thinkSettings(''), { thinking: null, reasoning_effort: null });

// 外部后端的模型与思考强度（Codex 的清单来自 model/list，形状同 colm-agent --codex-models）。
const codex = [
  { id: 'gpt-6.1-sol', name: 'GPT-6.1-Sol', efforts: ['low', 'high', 'max'], default_effort: 'low', default: true },
  { id: 'gpt-6-astra', name: 'GPT-6-Astra', efforts: ['medium', 'xhigh'], default_effort: 'medium', default: false },
];
assert.deepEqual(assistant.modelOptions('codex', codex).map(o => o[0]), ['', 'gpt-6.1-sol', 'gpt-6-astra']);
assert.equal(assistant.modelOptions('codex', codex)[0][1], '默认（GPT-6.1-Sol）');
assert.deepEqual(assistant.modelOptions('codex', []), [['', '默认']]);
assert.deepEqual(assistant.modelOptions('claude_code').map(o => o[0]), ['', 'fable', 'opus', 'sonnet', 'haiku']);
const openCode = [
  { id: 'deepseek/deepseek-flash', name: 'Flash', efforts: ['low', 'high', 'max'] },
  { id: 'local/custom', name: 'Local', efforts: [] },
];
assert.deepEqual(assistant.modelOptions('opencode', openCode).map(o => o[0]), ['', 'deepseek/deepseek-flash', 'local/custom']);
const values = options => options.map(o => o[0]);
assert.deepEqual(values(assistant.thinkOptions('builtin')), ['', 'low', 'high', 'max', 'off']);
assert.deepEqual(values(assistant.thinkOptions('claude_code', 'opus')), ['', 'low', 'medium', 'high', 'xhigh', 'max']);
// Codex：强度随模型；没选模型用默认模型的；取不到清单给四档。
assert.deepEqual(values(assistant.thinkOptions('codex', '', codex)), ['', 'low', 'high', 'max']);
assert.equal(assistant.thinkOptions('codex', '', codex)[0][1], '思考：默认（low）');
assert.deepEqual(values(assistant.thinkOptions('codex', 'gpt-6-astra', codex)), ['', 'medium', 'xhigh']);
assert.deepEqual(values(assistant.thinkOptions('codex', '', [])), ['', 'low', 'medium', 'high', 'xhigh']);
assert.deepEqual(values(assistant.thinkOptions('opencode', 'deepseek/deepseek-flash', openCode)), ['', 'low', 'high', 'max']);
assert.deepEqual(values(assistant.thinkOptions('opencode', 'local/custom', openCode)), ['']);
assert.deepEqual(values(assistant.thinkOptions('opencode', 'unknown/model', openCode)), ['']);
const saved = { thinking: null, reasoning_effort: 'max', external: { codex: { model: 'gpt-6-astra', effort: 'xhigh' } } };
assert.equal(assistant.currentThink(saved, 'builtin'), 'max');
assert.equal(assistant.currentThink(saved, 'codex'), 'xhigh');
assert.equal(assistant.currentThink(saved, 'claude_code'), '');
const changed = assistant.withChoice(saved, 'claude_code', { effort: 'high' });
assert.deepEqual(changed.external.claude_code, { model: null, effort: 'high' });
assert.deepEqual(changed.external.codex, saved.external.codex);
assert.equal(changed.reasoning_effort, 'max');
assert.deepEqual(assistant.withChoice(saved, 'codex', { effort: '' }).external.codex, { model: 'gpt-6-astra', effort: null });
for (const value of ['', 'low', 'high', 'max', 'off']) {
  assert.equal(assistant.thinkValue(assistant.thinkSettings(value)), value);
}

// 历史列表的时间：今天只写时分，今年写月日，往年带年份。
{
  const now = new Date(2026, 9, 8, 15, 0).getTime();
  assert.equal(assistant.sessionTime(new Date(2026, 9, 8, 9, 5).getTime(), now), '09:05');
  assert.equal(assistant.sessionTime(new Date(2026, 9, 7, 23, 40).getTime(), now), '10-07 23:40');
  assert.equal(assistant.sessionTime(new Date(2025, 0, 3, 8, 0).getTime(), now), '2025-01-03');
}

// 只自动打开当场建成的算例；回放历史、已经选中、或别的工具都不开。
assert.equal(assistant.shouldOpenCreatedCase({ name: 'create_case' }, '/c/a', '/c/b'), true);
assert.equal(assistant.shouldOpenCreatedCase({ name: 'create_case', replay: true }, '/c/a', '/c/b'), false);
assert.equal(assistant.shouldOpenCreatedCase({ name: 'create_case' }, '/c/a', '/c/a'), false);
assert.equal(assistant.shouldOpenCreatedCase({ name: 'set_case_fields' }, '/c/a', null), false);
assert.equal(assistant.shouldOpenCreatedCase({ name: 'create_case' }, null, null), false);

// 外部后端缺什么：没装、没登录、齐了。
assert.match(assistant.backendProblem('codex', { installed: false }), /没有找到 Codex/);
assert.match(assistant.backendProblem('claude_code', { installed: true, logged_in: false }), /Claude Code 还没有登录/);
assert.equal(assistant.backendProblem('claude_code', { installed: true, logged_in: true }), null);
assert.equal(assistant.backendProblem('opencode', { installed: true, configured: true }), null);
assert.match(assistant.backendProblem('opencode', { installed: true, logged_in: true, configured: false }), /首次连接模型/);
assert.match(assistant.backendProblem('opencode', { installed: false }), /没有找到 OpenCode/);
assert.equal(assistant.egressTarget({ base_url: 'https://api.example/v1' }), 'https://api.example/v1');
assert.equal(assistant.egressTarget({ backend: 'codex' }), 'codex');
assert.equal(assistant.egressTarget({ backend: 'opencode', external: { opencode: { model: 'deepseek/deepseek-flash' } } }), 'opencode:deepseek/deepseek-flash');
assert.notEqual(assistant.egressTarget({ backend: 'opencode', external: { opencode: { model: 'anthropic/claude' } } }), 'opencode:deepseek/deepseek-flash');

// Delayed discovery cannot overwrite another backend's draft or status; failed discovery is retried.
{
  const select = () => ({ value: '', disabled: false, replaceChildren(...items) { this.items = items; }, appendChild(item) { this.items.push(item); } });
  const nodes = new Map([
    ['assistant-backend', { value: 'opencode' }], ['assistant-ext-model', select()],
    ['assistant-think', select()], ['assistant-settings-save', { disabled: false }],
    ['assistant-backend-status', { textContent: '' }],
    ['assistant-opencode-setup', { hidden: true }], ['assistant-opencode-command', { textContent: '' }],
  ]);
  globalThis.document = { getElementById: id => nodes.get(id), querySelectorAll: () => [],
    createElement: () => ({}), documentElement: { lang: 'zh' } };
  assistant.ui.opencodeModels = null;
  let releaseModels;
  globalThis.assistantInvoke = async command => {
    assert.equal(command, 'assistant_opencode_models');
    return new Promise(resolve => { releaseModels = resolve; });
  };
  const settings = { external: { opencode: { model: 'deepseek/deepseek-flash', effort: 'high' }, claude_code: { model: 'sonnet', effort: 'low' } } };
  const stale = assistant.renderChoices(settings, 'opencode');
  assert.equal(nodes.get('assistant-ext-model').value, 'deepseek/deepseek-flash');
  assert.equal(nodes.get('assistant-settings-save').disabled, true);
  nodes.get('assistant-backend').value = 'claude_code';
  await assistant.renderChoices(settings, 'claude_code');
  releaseModels(openCode);
  await stale;
  assert.equal(nodes.get('assistant-ext-model').value, 'sonnet');
  assert.equal(nodes.get('assistant-think').value, 'low');
  assert.equal(nodes.get('assistant-settings-save').disabled, false);
  assert.deepEqual(assistant.withChoice(settings, 'claude_code', { model: nodes.get('assistant-ext-model').value, effort: nodes.get('assistant-think').value }).external.claude_code,
    { model: 'sonnet', effort: 'low' });

  let releaseStatus;
  globalThis.assistantInvoke = async () => new Promise(resolve => { releaseStatus = resolve; });
  nodes.get('assistant-backend').value = 'opencode';
  const staleStatus = assistant.refreshBackendStatus();
  nodes.get('assistant-backend').value = 'builtin';
  await assistant.refreshBackendStatus();
  releaseStatus({ opencode: { installed: true, configured: true, version: '2.0.0' } });
  await staleStatus;
  assert.equal(nodes.get('assistant-backend-status').textContent, '');

  nodes.get('assistant-backend').value = 'opencode';
  globalThis.assistantInvoke = async () => ({ opencode: { installed: true, configured: false, setup_command: "env HOME='/profile' opencode --standalone" } });
  await assistant.refreshBackendStatus();
  assert.equal(nodes.get('assistant-opencode-setup').hidden, false);
  assert.equal(nodes.get('assistant-opencode-command').textContent, "env HOME='/profile' opencode --standalone");
  globalThis.assistantInvoke = async () => ({ opencode: { installed: true, configured: true, version: '2.0.6' } });
  await assistant.refreshBackendStatus();
  assert.equal(nodes.get('assistant-opencode-setup').hidden, true);
  assert.equal(nodes.get('assistant-opencode-command').textContent, '');

  assistant.ui.opencodeModels = null;
  let attempts = 0;
  globalThis.assistantInvoke = async () => {
    if (++attempts === 1) throw new Error('unsupported OpenCode contract');
    return openCode;
  };
  await assert.rejects(assistant.externalModels('opencode'), /unsupported OpenCode contract/);
  assert.equal(assistant.ui.opencodeModels, null);
  assert.deepEqual(await assistant.externalModels('opencode'), openCode);
  assert.equal(attempts, 2);
}
assert.equal(assistant.backendProblem('builtin', null), null);

// Every send refreshes a changed directory/kernel grant, while retaining the conversation.
{
  const elements = new Map(['root', 'kernel', 'assistant-key-status', 'assistant-key', 'assistant-key-save']
    .map(id => [id, { value: '' }]));
  document.getElementById = id => elements.get(id) ?? null;
  document.querySelector = () => null;
  const calls = [];
  globalThis.assistantInvoke = async (command, args) => {
    calls.push([command, args]);
    if (command === 'assistant_settings') return { base_url: 'local', egress_acknowledged: 'local' };
    if (command === 'assistant_has_key') return true;
    if (!['assistant_start', 'assistant_new_session'].includes(command)) throw new Error(`Unexpected IPC: ${command}`);
  };
  assistant.ui.conversation = 'existing-session';
  assistant.ui.pendingNewConversation = false;
  elements.get('root').value = '/p';
  elements.get('kernel').value = '/k';
  const first = await assistant.ensureStarted();
  assert.equal(first.root, '/p');
  await assistant.ensureStarted();
  assert.equal(calls.filter(([cmd]) => cmd === 'assistant_start').length, 1);
  elements.get('root').value = '/different/missing';
  assert.equal((await assistant.ensureStarted()).root, '/different/missing');
  elements.get('kernel').value = '/new-kernel';
  await assistant.ensureStarted();
  elements.get('root').value = '';
  assert.equal((await assistant.ensureStarted()).root, '');
  assert.deepEqual(calls.filter(([cmd]) => cmd === 'assistant_start').map(([, args]) => args), [
    { projectRoot: '/p', kernelDir: '/k', resume: 'existing-session' },
    { projectRoot: '/different/missing', kernelDir: '/k', resume: 'existing-session' },
    { projectRoot: '/different/missing', kernelDir: '/new-kernel', resume: 'existing-session' },
    { projectRoot: '', kernelDir: '/new-kernel', resume: 'existing-session' },
  ]);
  // A root change while assistant_start is pending must refresh again before sending.
  const originalInvoke = globalThis.assistantInvoke;
  globalThis.assistantInvoke = async (command, args) => {
    const result = await originalInvoke(command, args);
    if (command === 'assistant_start' && args.projectRoot === '/during-start') {
      elements.get('root').value = '/latest';
    }
    return result;
  };
  elements.get('root').value = '/during-start';
  assert.equal((await assistant.ensureStarted()).root, '/latest');
  assert.equal(calls.filter(([cmd]) => cmd === 'assistant_start').at(-1)[1].projectRoot, '/latest');
  assistant.ui.started = false;
  assistant.ui.conversation = null;
  assistant.ui.pendingNewConversation = true;
  await assistant.ensureStarted();
  assert.equal(calls.filter(([cmd]) => cmd === 'assistant_start').at(-1)[1].resume, null);
  assert.equal(calls.at(-1)[0], 'assistant_new_session');
  assert.equal(assistant.ui.pendingNewConversation, false);
  // Settings can require configuration while the process retains the previous native session.
  assistant.ui.conversation = 'live-old-session';
  assistant.ui.started = false;
  assistant.ui.pendingNewConversation = true;
  await assistant.ensureStarted();
  assert.equal(calls.filter(([cmd]) => cmd === 'assistant_start').at(-1)[1].resume, null);
  assert.equal(calls.at(-1)[0], 'assistant_new_session');
  assistant.ui.pendingNewConversation = true;
  globalThis.assistantInvoke = async (command, args) => {
    if (command === 'assistant_new_session') throw new Error('reset failed');
    return originalInvoke(command, args);
  };
  await assert.rejects(assistant.ensureStarted(), /reset failed/);
  assert.equal(assistant.ui.pendingNewConversation, true, 'retry resets before sending');
  assert.equal((source.match(/授权目录中的文本文件片段/g) || []).length, 4);
}

// History navigation restores the unsent draft and leaves multiline/IME editing alone.
{
  assert.deepEqual(assistant.inputHistory(['old', '', null, 'old', 'new']), ['old', 'new']);
  assert.equal(assistant.inputHistory(Array.from({ length: 110 }, (_, i) => `${i}`))[0], '10');
  assert.deepEqual(assistant.inputHistory(['x'.repeat(32001)]), []);
  const cursor = { historyIndex: null, historyDraft: '' };
  const textarea = { value: 'unsent draft', selectionStart: 12, selectionEnd: 12,
    setSelectionRange(start, end) { this.selectionStart = start; this.selectionEnd = end; } };
  const press = (key, extras = {}) => {
    let prevented = false;
    const handled = assistant.recallInput({ key, preventDefault() { prevented = true; }, ...extras }, textarea, ['older\nquestion', 'newest'], cursor);
    assert.equal(prevented, handled);
    return handled;
  };
  assert.equal(press('ArrowDown'), false);
  assert.equal(press('ArrowUp'), true); assert.equal(textarea.value, 'newest');
  assert.equal(press('ArrowUp'), true); assert.equal(textarea.value, 'older\nquestion');
  assert.equal(press('ArrowUp'), true); assert.equal(textarea.value, 'older\nquestion');
  assert.equal(press('ArrowDown'), true); assert.equal(textarea.value, 'newest');
  assert.equal(press('ArrowDown'), true); assert.equal(textarea.value, 'unsent draft');
  assert.equal(cursor.historyIndex, null);
  for (const extra of [{ isComposing: true }, { keyCode: 229 }, { shiftKey: true }, { ctrlKey: true }, { altKey: true }, { metaKey: true }]) assert.equal(press('ArrowUp', extra), false);
  textarea.value = 'first\nsecond'; textarea.setSelectionRange(12, 12);
  assert.equal(press('ArrowUp'), false);
  textarea.setSelectionRange(0, 5); assert.equal(press('ArrowUp'), false);
  textarea.setSelectionRange(3, 3); assert.equal(press('ArrowUp'), true);
}

// New conversation never removes stored sessions or carries over an unsent draft.
{
  const empty = {};
  const nodes = new Map([
    ['assistant-log', { replaceChildren(...children) { this.children = children; } }],
    ['assistant-usage', {}], ['assistant-notice', {}],
    ['assistant-text', { value: 'old draft', focus() {} }],
  ]);
  document.getElementById = id => nodes.get(id) ?? null;
  assistant.ui.emptyState = empty;
  assistant.ui.conversation = 'previous'; assistant.ui.started = true; assistant.ui.running = false;
  assistant.ui.historyIndex = 0; assistant.ui.historyDraft = 'saved draft';
  const commands = [];
  globalThis.assistantInvoke = async command => { commands.push(command); };
  await assistant.startNewConversation();
  assert.equal(assistant.ui.conversation, null);
  assert.equal(nodes.get('assistant-text').value, '');
  assert.equal(assistant.ui.historyIndex, null);
  assert.deepEqual(nodes.get('assistant-log').children, [empty]);
  assert.equal(nodes.get('assistant-log').scrollTop, 0);
  assert.deepEqual(commands, [], 'the native reset happens before the next send');
  assert.equal(assistant.ui.pendingNewConversation, true);
  assistant.ui.running = true;
  nodes.get('assistant-text').value = 'keep during run';
  await assistant.startNewConversation();
  assert.equal(nodes.get('assistant-text').value, 'keep during run');
  assistant.ui.running = false;
}

// Read only stored prompts, merge new sends during loading and honor session deletion.
{
  let resolveHistory;
  globalThis.assistantInvoke = async command => {
    assert.equal(command, 'assistant_input_history');
    return new Promise(resolve => { resolveHistory = resolve; });
  };
  assistant.ui.inputHistoryLoading = null;
  const loading = assistant.refreshInputHistory();
  assistant.ui.pendingInputs.push('sent while loading');
  resolveHistory(['previous user input']); await loading;
  assert.deepEqual(assistant.ui.inputHistory, ['previous user input', 'sent while loading']);
  assistant.ui.historyIndex = 0; assistant.ui.historyDraft = 'unsent draft';
  globalThis.assistantInvoke = async () => [];
  await assistant.refreshInputHistory();
  assert.deepEqual(assistant.ui.inputHistory, []);
  assert.equal(document.getElementById('assistant-text').value, 'unsent draft');
  assert.equal(assistant.ui.historyIndex, null);
  let historyReads = 0;
  globalThis.assistantInvoke = async () => {
    historyReads += 1;
    if (historyReads === 1) return new Promise(resolve => { resolveHistory = resolve; });
    return ['still-saved'];
  };
  const oldRead = assistant.refreshInputHistory();
  const deletionRefresh = assistant.refreshInputHistory(true);
  resolveHistory(['deleted-session-input']);
  await Promise.all([oldRead, deletionRefresh]);
  assert.equal(historyReads, 2);
  assert.deepEqual(assistant.ui.inputHistory, ['still-saved']);
}
console.log('assistant: new conversation, example entry points and persisted input navigation ok');
console.log('assistant: project paths and per-message configuration refresh ok');

// Two clicks during startup must not send the same operation twice; failed startup unlocks the panel.
{
  const nodes = new Map([
    ['assistant-text', { value: 'run current case' }], ['assistant-send', {}], ['assistant-stop', {}],
  ]);
  document.getElementById = id => nodes.get(id) ?? null;
  let rejectStartup;
  let configurations = 0;
  globalThis.assistantInvoke = async command => {
    assert.equal(command, 'assistant_settings');
    configurations += 1;
    return new Promise((_, reject) => { rejectStartup = reject; });
  };
  assistant.ui.running = false;
  const first = assistant.send();
  await assistant.send();
  assert.equal(configurations, 1);
  assert.equal(assistant.ui.running, true);
  rejectStartup(new Error('startup fixture failed'));
  await assert.rejects(first, /startup fixture failed/);
  assert.equal(assistant.ui.running, false);
  assert.equal(nodes.get('assistant-send').disabled, false);
}

// Model discovery offers suggestions without changing a typed model, including failed requests.
{
  const nodes = new Map([
    ['assistant-model-refresh', {}], ['assistant-base', { value: 'https://models.example/v1' }],
    ['assistant-api-format', { value: 'responses' }], ['assistant-model-name', { value: 'my-future-model' }],
    ['assistant-model-status', {}], ['assistant-models', { replaceChildren(...items) { this.items = items; } }],
  ]);
  document.getElementById = id => nodes.get(id) ?? null;
  document.createElement = () => ({});
  globalThis.assistantInvoke = async (command, args) => {
    assert.equal(command, 'assistant_api_models');
    assert.deepEqual(args, { baseUrl: 'https://models.example/v1', apiFormat: 'responses' });
    return ['model-one', 'model-two'];
  };
  await assistant.refreshApiModels();
  assert.deepEqual(nodes.get('assistant-models').items.map(o => o.value), ['model-one', 'model-two']);
  assert.equal(nodes.get('assistant-model-name').value, 'my-future-model');
  globalThis.assistantInvoke = async () => { throw new Error('discovery unavailable'); };
  await assistant.refreshApiModels();
  assert.equal(nodes.get('assistant-model-name').value, 'my-future-model');
  assert.equal(nodes.get('assistant-model-refresh').disabled, false);
  assert.match(nodes.get('assistant-model-status').textContent, /已保留/);
}

// T7: restore loads the saved backend without focusing or starting a conversation.
{
  for (const [width, preference, open] of [[1399, null, false], [1400, null, true], [1800, 'false', false], [900, 'true', true]]) {
    assert.equal(assistant.defaultPanelOpen(width, preference), open);
  }
  const stored = new Map(); let focuses = 0; const calls = [];
  globalThis.localStorage = { getItem: key => stored.get(key) ?? null, setItem: (key, value) => stored.set(key, value) };
  globalThis.innerWidth = 1600;
  const settings = { backend: 'codex', base_url: 'local', external: { codex: { model: 'gpt-6.1-sol', effort: 'high' } } };
  globalThis.assistantInvoke = async command => {
    calls.push(command);
    if (command === 'assistant_settings') return settings;
    if (command === 'assistant_input_history') return ['saved input'];
    if (command === 'assistant_codex_models') return codex;
    if (command === 'assistant_backend_status') return { codex: { installed: true, logged_in: true, version: 'test' } };
    if (command === 'assistant_has_key') return false;
    throw new Error(`unexpected startup IPC: ${command}`);
  };
  const node = () => ({ value: '', dataset: {}, children: [], replaceChildren(...children) { this.children = children; }, appendChild(child) { this.children.push(child); } });
  const nodes = new Map([...html.matchAll(/id="([^"]+)"/g)].map(([, id]) => [id, node()]));
  const panel = nodes.get('assistant-panel'); panel.hidden = true; panel.getBoundingClientRect = () => ({ width: 440 });
  nodes.get('assistant-text').focus = () => { focuses++; };
  document.getElementById = id => nodes.get(id) ?? null;
  document.createElement = () => node();
  document.querySelector = () => null;
  document.querySelectorAll = () => [];
  document.body = { classList: { toggle() {} } };
  document.documentElement.style = { setProperty() {} };
  assistant.ui.codexModels = null; assistant.ui.inputHistoryLoaded = false; assistant.ui.inputHistoryLoading = null;
  assistant.restorePanel(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(panel.hidden, false); assert.equal(focuses, 0); assert.equal(stored.size, 0);
  assert.equal(assistant.ui.settingsDraft, settings);
  assert.equal(nodes.get('assistant-backend').value, 'codex');
  assert.equal(nodes.get('assistant-ext-model').value, 'gpt-6.1-sol');
  assert.equal(nodes.get('assistant-think').value, 'high');
  assert.ok(calls.includes('assistant_settings')); assert.ok(calls.includes('assistant_input_history'));
  assert.ok(calls.includes('assistant_codex_models'));
  const beforeResize = calls.length;
  assistant.resizePanel(); assistant.resizePanel();
  assert.equal(calls.length, beforeResize, 'same expansion state does not reload settings');
  globalThis.innerWidth = 1000; assistant.resizePanel(); assert.equal(panel.hidden, true);
  globalThis.innerWidth = 1600; assistant.resizePanel(); await new Promise(resolve => setImmediate(resolve));
  assert.equal(panel.hidden, false); assert.equal(focuses, 0);
  assert.equal(calls.filter(command => command === 'assistant_settings').length, 2);
  assistant.togglePanel(false); assistant.restorePanel();
  assert.equal(panel.hidden, true); assert.equal(stored.get('colm.assistant.open'), 'false');
  assistant.resizePanel(); assert.equal(panel.hidden, true);
  globalThis.innerWidth = 900;
  assistant.togglePanel(true); await new Promise(resolve => setImmediate(resolve));
  assistant.restorePanel(); await new Promise(resolve => setImmediate(resolve));
  assistant.resizePanel();
  assert.equal(panel.hidden, false); assert.equal(focuses, 1);
  assert.equal(calls.some(command => ['assistant_start', 'assistant_send', 'assistant_new_session'].includes(command)), false);
}

// The context label and sent context derive from the same visible-page snapshot.
{
  const { state } = await import(pathToFileURL(join(temp, 'app', 'state.js')).href);
  const shown = (text = '') => ({ textContent: text, closest() { return this.hidden ? {} : null; }, getClientRects() { return this.hidden ? [] : [{}]; } });
  const hiddenHeading = Object.assign(shown('结果总览'), { hidden: true });
  const page = { dataset: { step: 'result' }, querySelectorAll: () => [hiddenHeading, shown('参数率定')] };
  const cards = [shown('研究版本 snow'), shown('7920ts'), shown('Rust')];
  const mode = Object.assign(shown(), { querySelectorAll: () => cards });
  const nodes = new Map([
    ['context-bar', shown()], ['model-version', { value: 'official' }], ['model-engine', { value: 'fortran' }], ['run-target', { value: 'local' }],
    ['tune-run-mode', mode], ['tune-run-mode-status', shown('研究检查通过')], ['tune-step-progress', shown('4 / 8')],
    ['assistant-visible-context', shown()], ['work', shown()],
  ]);
  document.getElementById = id => nodes.get(id) ?? null;
  document.querySelector = () => page;
  state.step = 'result-tuning'; state.selected = { dir: '/p/old' }; state.resultCaseDir = '/p/current';
  let view = assistant.currentView();
  assert.equal(view.caseDir, '/p/current');
  assert.equal(view.snapshot.pageName, '参数率定', 'skip the hidden overview heading on result subpages');
  assert.doesNotMatch(view.details.join('\n'), /official release; location local; engine fortran/, 'case-only header does not expose hidden run choices');
  assert.equal(view.snapshot.runMode, false);
  nodes.set('run-mode-choices', shown()); state.step = 'run-mode';
  view = assistant.currentView(); assert.equal(view.snapshot.runMode, true);
  assert.match(view.details.join('\n'), /official release; location local; engine fortran/);
  nodes.get('run-mode-choices').hidden = true;
  nodes.set('run-mode-line', shown()); state.step = 'run';
  assert.equal(assistant.currentView().snapshot.runMode, true);
  nodes.get('run-mode-line').hidden = true; state.step = 'result-tuning';
  view = assistant.currentView();
  assert.match(view.details.join('\n'), /研究版本 snow/);
  assert.match(view.details.join('\n'), /7920ts/);
  assistant.refreshVisibleContext(view);
  assert.equal(nodes.get('assistant-visible-context').textContent, assistant.visibleContextText(view.snapshot));
  mode.hidden = true; nodes.get('tune-run-mode-status').hidden = true;
  assert.doesNotMatch(assistant.currentView().details.join('\n'), /研究版本 snow|7920ts|研究检查通过/);
  state.step = 'result-evaluation';
  nodes.set('metrics', shown('Qle: NSE 0.4'));
  state.resultMetrics = [{ case_dir: '/p/current', name: 'Qle', nse: 0.4 }, { case_dir: '/p/old', name: 'Old', nse: 0.1 }];
  view = assistant.currentView(); assert.equal(view.snapshot.metrics, 1);
  assert.match(view.details.join('\n'), /Qle/); assert.doesNotMatch(view.details.join('\n'), /Old/);
  nodes.get('metrics').hidden = true;
  assert.equal(assistant.currentView().snapshot.metrics, 0);
  state.step = 'code';
  const checks = shown('编译 通过 · 测试 不通过');
  const detail = Object.assign(shown(), { querySelector: selector => selector === '.ws-detail-header' ? shown('snow') : checks });
  nodes.set('workspace-detail', detail);
  view = assistant.currentView(); assert.match(view.details.join('\n'), /测试 不通过/);
  detail.hidden = true; nodes.get('context-bar').hidden = true;
  view = assistant.currentView(); assert.equal(view.snapshot.versionDetail.length, 0); assert.equal(view.snapshot.header, false);
  // Observer never watches assistant output, and async page mutations refresh the same label.
  const watched = []; const callbacks = []; let changed;
  globalThis.MutationObserver = class { constructor(callback) { changed = callback; } observe(root) { watched.push(root); } };
  globalThis.addEventListener = (name, callback) => callbacks.push([name, callback]);
  for (const id of ['work', 'context-bar']) nodes.get(id).addEventListener = () => {};
  assistant.watchVisibleContext();
  assert.deepEqual(watched, [nodes.get('work'), nodes.get('context-bar')]);
  detail.hidden = false; changed(); await Promise.resolve();
  assert.match(nodes.get('assistant-visible-context').textContent, /版本详情/);
  assert.ok(callbacks.some(([name]) => name === 'colm:step'));
  delete globalThis.MutationObserver;
}

// Next actions are plain text requests, excluded from quotes/code and never executed on click.
{
  assert.deepEqual(assistant.nextRequests('下一步：检查 snow\n\nNext action: Check server 7920ts'), ['检查 snow', 'Check server 7920ts']);
  assert.deepEqual(assistant.nextRequests('> 下一步：quoted\n```js\n下一步：code\n```\n~~~\nNext action: code\n~~~'), []);
  assert.equal(assistant.nextRequests(Array.from({ length: 5 }, (_, i) => `下一步：${i}`).join('\n')).length, 3);
  assert.deepEqual(assistant.nextRequests(`下一步：${'x'.repeat(241)}`), []);
  class Node {
    constructor(tag) { this.tagName = tag; this.children = []; this.textContent = ''; }
    append(...children) { this.children.push(...children); }
    appendChild(child) { this.append(child); return child; }
    replaceChildren(...children) { this.children = children; }
  }
  let focuses = 0; let calls = 0;
  const input = { value: '', focus() { focuses++; } };
  const host = new Node('div'); const buttons = [];
  document.createElement = tag => { const node = new Node(tag); if (tag === 'button') buttons.push(node); return node; };
  document.getElementById = id => id === 'assistant-text' ? input : {};
  document.querySelectorAll = selector => selector === '.assistant-next-actions button' ? buttons : [];
  globalThis.assistantInvoke = async () => { calls++; };
  assistant.ui.running = false;
  const literal = '<img src=x onerror=alert(1)>';
  assistant.renderAnswer(host, `下一步：${literal}`);
  assert.equal(buttons.length, 1); assert.equal(buttons[0].textContent, literal);
  buttons[0].onclick(); assert.equal(input.value, literal); assert.equal(focuses, 1); assert.equal(calls, 0);
  assistant.setRunning(true); assert.equal(buttons[0].disabled, true);
  input.value = 'keep'; buttons[0].onclick(); assert.equal(input.value, 'keep');
  assistant.setRunning(false); assert.equal(buttons[0].disabled, false);
  assert.match(host.children.at(-1).children[0].textContent, /不会发送/);
}
console.log('assistant: default docking, visible page snapshots and fill-only next actions ok');
