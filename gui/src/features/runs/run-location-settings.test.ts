import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { default: RunLocationSettings } = await import('./RunLocationSettings');
import { newServer } from '../resources/resource';

const server = {
  ...newServer(),
  host: 'compute',
  root: '/remote/jobs',
  maps: [{ local: '/local/input', remote: '/data/input' }],
};
const render = (host: string) =>
  renderToStaticMarkup(
    createElement(RunLocationSettings, {
      host,
      servers: [server],
      disabled: false,
      error: '',
      en: true,
      onChange: () => {},
      onConfigure: () => {},
    }),
  );

test('remote setup distinguishes local file browsing from configured server paths', () => {
  const html = render('compute');
  assert.ok(html.includes('File pickers browse local files'));
  assert.ok(html.includes('/remote/jobs'));
  assert.ok(html.includes('/local/input') && html.includes('/data/input'));
  assert.ok(html.includes('Configure resources and path mappings'));
});

test('local setup omits remote paths and removed servers retain the saved selection', () => {
  assert.ok(!render('').includes('/remote/jobs'));
  const html = render('missing');
  assert.ok(html.includes('missing') && html.includes('Not configured'));
  assert.ok(!html.includes('/remote/jobs'));
});
