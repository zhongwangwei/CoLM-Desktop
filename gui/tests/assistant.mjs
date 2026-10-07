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
globalThis.window = {};
globalThis.document = { getElementById: () => null, querySelectorAll: () => [], documentElement: { lang: 'zh' } };
globalThis.addEventListener = () => {};
const assistant = await import(pathToFileURL(join(temp, 'app', 'assistant.js')).href);

assert.equal(
  assistant.viewContext({ step: 'result', caseDir: '/p/A', kernel: '/k', root: '' }),
  'page: result\nselected case: /p/A\nkernel: /k',
);
assert.equal(assistant.viewContext({}), '');

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
console.log('assistant: context, events, answer blocks and page wiring ok');
