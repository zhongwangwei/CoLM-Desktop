import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement as h } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { saveStateText } from './save-status';
import { SaveBadge } from '../components/SaveBadge';

const zh = (a: string) => a;
const en = (_a: string, b: string) => b;

test('save states use the same three phrases in both languages', () => {
  assert.equal(saveStateText('dirty', zh), '有未保存修改');
  assert.equal(saveStateText('saving', zh), '正在保存');
  assert.equal(saveStateText('saved', zh), '已保存');
  assert.equal(saveStateText('dirty', en), 'Unsaved changes');
  assert.equal(saveStateText('saving', en), 'Saving…');
  assert.equal(saveStateText('saved', en), 'Saved');
});

test('the shared badge renders every save state with its stable data attribute', () => {
  for (const state of ['saved', 'dirty', 'saving'] as const) {
    const html = renderToStaticMarkup(h(SaveBadge, { state, t: en }));
    assert.match(html, new RegExp(`data-save-state="${state}"`));
    assert.ok(html.includes(saveStateText(state, en)), state);
  }
  const saving = renderToStaticMarkup(h(SaveBadge, { state: 'saving', t: zh }));
  assert.match(saving, /正在保存/);
});
