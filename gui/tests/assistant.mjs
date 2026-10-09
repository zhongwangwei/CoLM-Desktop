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
  (await readFile(join(temp, 'app', 'assistant.js'), 'utf8')) + '\nexport { ensureStarted, ui, send };\n');
await writeFile(join(temp, 'app', 'ipc.js'), `
  export const invoke = (...args) => globalThis.assistantInvoke(...args);
  export const listen = async () => {};
  export const hasBackend = () => true;
`);

globalThis.window = {};
globalThis.document = { getElementById: () => null, querySelectorAll: () => [], documentElement: { lang: 'zh' } };
globalThis.addEventListener = () => {};
const assistant = await import(pathToFileURL(join(temp, 'app', 'assistant.js')).href);

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
  // 启用过的实验内核要告诉助手（不然它会以为结果来自正式内核）。
  assert.ok(assistant.pageDetails('run', {
    adopted: [{ preset: 'default', experimental: { workspace: 'vcmax-grass', head: 'd32488535882' } }],
  })[0].includes('workspace vcmax-grass (commit d3248853)'));
  assert.deepEqual(assistant.pageDetails('run', { adopted: [{ preset: 'default' }] }), []);
}
// 提问建议跟着步骤走；没有建议的步骤返回空。
assert.ok(assistant.pagePrompts('result-evaluation').length >= 2);
assert.ok(assistant.pagePrompts('params-water')[0].prompt.includes('过程参数'));
assert.ok(assistant.pagePrompts('basic-files').length === 1);
assert.deepEqual(assistant.pagePrompts('prep-site'), []);
assert.deepEqual(assistant.pagePrompts(undefined), []);
for (const step of ['run', 'research', 'result-tuning', 'result-uncertainty', 'hybrid-learn', 'hybrid-process', 'result-diagnostics']) {
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
  assert.ok(html.includes(`id="${id}"`), `index.html has no #${id}`);
}
assert.ok(!/\.(innerHTML|outerHTML)\s*=|insertAdjacentHTML/.test(source), 'assistant.js must not use innerHTML');
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
assert.equal(assistant.thinkValue({ thinking: true, reasoning_effort: null }), '');
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
const values = options => options.map(o => o[0]);
assert.deepEqual(values(assistant.thinkOptions('builtin')), ['', 'low', 'high', 'max', 'off']);
assert.deepEqual(values(assistant.thinkOptions('claude_code', 'opus')), ['', 'low', 'medium', 'high', 'xhigh', 'max']);
// Codex：强度随模型；没选模型用默认模型的；取不到清单给四档。
assert.deepEqual(values(assistant.thinkOptions('codex', '', codex)), ['', 'low', 'high', 'max']);
assert.equal(assistant.thinkOptions('codex', '', codex)[0][1], '思考：默认（low）');
assert.deepEqual(values(assistant.thinkOptions('codex', 'gpt-6-astra', codex)), ['', 'medium', 'xhigh']);
assert.deepEqual(values(assistant.thinkOptions('codex', '', [])), ['', 'low', 'medium', 'high', 'xhigh']);
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
    if (command !== 'assistant_start') throw new Error(`Unexpected IPC: ${command}`);
  };
  assistant.ui.conversation = 'existing-session';
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
  assert.equal((source.match(/授权目录中的文本文件片段/g) || []).length, 3);
}
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
