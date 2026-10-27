import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { copyText, parseMarkdown, safeHttpUrl } from './markdown';

const t = (_zh: string, en: string) => en;
const { Markdown } = await import('./MarkdownMessage');

function render(text: string) {
  return renderToStaticMarkup(createElement(Markdown, { text, t }));
}

test('safe subset parses code, emphasis, headings, quotes, lists and tables', () => {
  const blocks = parseMarkdown(
    [
      '## Result',
      '',
      'A **bold** and *italic* with `inline code` and a [source](https://example.com/a?b=1).',
      '',
      '- first',
      '- second',
      '',
      '1. one',
      '2. two',
      '',
      '> quoted line',
      '',
      '| metric | value |',
      '| --- | --- |',
      '| RMSE | 1.2 |',
      '',
      '```ts',
      'const x = 1;',
      '```',
    ].join('\n'),
  );
  assert.deepEqual(
    blocks.map((block) => block.kind),
    ['heading', 'paragraph', 'list', 'list', 'quote', 'table', 'code'],
  );
  const table = blocks.find((block) => block.kind === 'table');
  assert.ok(table && table.kind === 'table');
  assert.equal(table.header.length, 2);
  assert.equal(table.rows[0].length, 2);
  const code = blocks.find((block) => block.kind === 'code');
  assert.ok(code && code.kind === 'code');
  assert.equal(code.lang, 'ts');
  assert.equal(code.code, 'const x = 1;');
  const paragraph = blocks[1];
  assert.ok(paragraph.kind === 'paragraph');
  assert.deepEqual(
    paragraph.children.map((node) => node.kind),
    ['text', 'strong', 'text', 'em', 'text', 'code', 'text', 'link', 'text'],
  );
});

test('only absolute http and https links are accepted', () => {
  assert.equal(safeHttpUrl('https://example.com/a'), 'https://example.com/a');
  assert.equal(safeHttpUrl('http://example.com'), 'http://example.com/');
  for (const unsafe of [
    'javascript:alert(1)',
    'JAVASCRIPT:alert(1)',
    'data:text/html;base64,PHNjcmlwdD4=',
    'file:///etc/passwd',
    '/relative/path',
    '#anchor',
    '',
  ])
    assert.equal(safeHttpUrl(unsafe), null, unsafe);
});

test('raw HTML is rendered as text and never becomes markup', () => {
  const markup = render('<script>alert(1)</script>\n\n<b onclick="bad()">bold?</b>');
  assert.doesNotMatch(markup, /<script/i);
  assert.doesNotMatch(markup, /<b onclick/i);
  assert.match(markup, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.match(markup, /&lt;b onclick=&quot;bad\(\)&quot;&gt;bold\?&lt;\/b&gt;/);
});

test('dangerous links stay unclickable literal text', () => {
  for (const unsafe of [
    '[click](javascript:alert(1))',
    '[click](data:text/html,<script>alert(1)</script>)',
    '[click](file:///etc/passwd)',
  ]) {
    const markup = render(unsafe);
    assert.doesNotMatch(markup, /<a\b/, unsafe);
    assert.doesNotMatch(markup, /href=/i, unsafe);
    assert.match(markup, /\[click\]/, unsafe);
  }
});

test('source links open in a new window without an opener', () => {
  const wikipedia = render('[Foo](https://en.wikipedia.org/wiki/Foo_(bar))');
  assert.match(wikipedia, /href="https:\/\/en\.wikipedia\.org\/wiki\/Foo_\(bar\)"/);
  const markup = render('See [the paper](https://example.com/paper).');
  assert.match(
    markup,
    /<a href="https:\/\/example\.com\/paper" target="_blank" rel="noopener noreferrer"/,
  );
  assert.match(markup, />the paper<\/a>/);
});

test('code blocks render safely with a copy button and escaped content', () => {
  const markup = render('```html\n<img src=x onerror="alert(1)">\n```');
  assert.match(
    markup,
    /<pre[^>]*><code>&lt;img src=x onerror=&quot;alert\(1\)&quot;&gt;<\/code><\/pre>/,
  );
  assert.match(markup, /aria-label="Copy code"/);
  assert.doesNotMatch(markup, /<img/);
});

test('tables, lists, headings and quotes render their semantic elements', () => {
  const markup = render(
    [
      '### Model',
      '',
      '| metric | value |',
      '| --- | --- |',
      '| RMSE | **1.2** |',
      '',
      '- one',
      '- two',
      '',
      '> careful',
    ].join('\n'),
  );
  assert.match(markup, /<h3[^>]*>Model<\/h3>/);
  assert.match(markup, /<table[^>]*><thead><tr><th[^>]*>metric<\/th><th[^>]*>value<\/th>/);
  assert.match(markup, /<td[^>]*>RMSE<\/td><td[^>]*><strong>1\.2<\/strong><\/td>/);
  assert.match(markup, /<ul[^>]*><li[^>]*>one<\/li><li[^>]*>two<\/li><\/ul>/);
  assert.match(markup, /<blockquote[^>]*>.*careful.*<\/blockquote>/);
});

test('streaming partial markup never throws and stays text until complete', () => {
  const partial = render('An answer **still');
  assert.match(partial, /An answer \*\*still/);
  assert.deepEqual(parseMarkdown('```js\nconst x ='), [
    { kind: 'code', lang: 'js', code: 'const x =' },
  ]);
});

test('copyText writes the exact code block and reports a missing clipboard', async () => {
  const written: string[] = [];
  await copyText('const x = 1;', {
    writeText: async (text: string) => {
      written.push(text);
    },
  });
  assert.deepEqual(written, ['const x = 1;']);
  await assert.rejects(copyText('x', null), /clipboard/i);
});
