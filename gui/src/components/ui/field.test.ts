import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement as h, Fragment, type ReactNode } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// App state reads saved preferences when the shared UI module is imported.
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { Field, Input, DateInput, Select, Button } = await import('./index');

function field(label: string, children: ReactNode) {
  return renderToStaticMarkup(h(Field, { label, children }));
}
function labelledControl(markup: string, tag: string) {
  const target = markup.match(/<label class="field-label" for="([^"]+)">/)?.[1];
  assert.ok(target, 'the visible caption must be a native label');
  const control = markup.match(new RegExp(`<${tag}\\b[^>]*>`))?.[0];
  assert.ok(control?.includes(`id="${target}"`), 'label activation targets the actual control');
  return target;
}

test('Field labels inputs, selects and textareas with unique native for/id associations', () => {
  labelledControl(field('Name', h(Input)), 'input');
  labelledControl(field('Mode', h(Select, null, h('option', null, 'Site'))), 'select');
  labelledControl(field('Notes', h('textarea')), 'textarea');
  assert.equal(
    labelledControl(field('Name', h(Input, { id: 'existing-name' })), 'input'),
    'existing-name',
  );
  const twoFields = renderToStaticMarkup(
    h(
      Fragment,
      null,
      h(Field, { label: 'First', children: h(Input) }),
      h(Field, { label: 'Second', children: h(Input) }),
    ),
  );
  const targets = [...twoFields.matchAll(/for="([^"]+)"/g)].map((match) => match[1]);
  assert.equal(new Set(targets).size, 2, 'separate fields must not target the same input');
});

test('nested browse-input labels the input without wrapping or targeting the browse button', () => {
  const markup = field(
    'Case directory',
    h(
      'div',
      { className: 'browse-input' },
      h(Button, { id: 'browse', 'aria-label': 'Browse' }, '…'),
      h(Fragment, null, h(Input, { id: 'case-root', 'aria-label': 'Root' })),
    ),
  );
  assert.equal(labelledControl(markup, 'input'), 'case-root');
  assert.match(markup, /<label[^>]*>Case directory<\/label><div class="browse-input">/);
  assert.match(markup, /aria-label="Root"/);
  assert.doesNotMatch(markup, /for="browse"/);
});

test('button-only groups remain inert captions and never become label descendants', () => {
  const markup = field(
    'Scheme',
    h(
      'div',
      null,
      h(Button, { id: 'first' }, 'First'),
      h(Button, null, 'Second'),
      h('input', { type: 'hidden' }),
      h('input', { type: 'submit', value: 'Apply' }),
    ),
  );
  assert.match(markup, /<span class="field-label">Scheme<\/span>/);
  assert.doesNotMatch(markup, /<label|for=/);
});

test('multiple controls keep independent IDs and labels while Field targets only the first', () => {
  const markup = field(
    'Range',
    h(Fragment, null, h(Input, { id: 'from' }), h(Input, { id: 'to', 'aria-label': 'End' })),
  );
  assert.equal(labelledControl(markup, 'input'), 'from');
  assert.match(markup, /id="to" aria-label="End"/);
  const ownLabel = field('Options', h('label', null, 'Enabled', h(Input, { type: 'checkbox' })));
  assert.match(ownLabel, /<span class="field-label">Options<\/span>/);
  assert.match(ownLabel, /<label>Enabled<input/);
});

test('date fields label the editable text and keep an independently named calendar action', () => {
  const markup = field(
    'Start date',
    h(DateInput, { value: '2001-', calendarLabel: 'Choose start date', onChange: () => {} }),
  );
  labelledControl(markup, 'input');
  assert.match(markup, /type="text"[^>]*value="2001-"/);
  assert.match(markup, /aria-label="Choose start date"/);
  assert.match(markup, /type="date"/);
});
