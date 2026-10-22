import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  coreHistoryVariables,
  historyKey,
  historyPreset,
  historyBlockedReason,
  historySelection,
} from './history-variables';

test('core variables are a short research set distinct from the full history catalog', () => {
  const generated = readFileSync(
    new URL('../../../../crates/colm-schema/src/generated.rs', import.meta.url),
    'utf8',
  );
  const all = new Set(
    [...generated.matchAll(/DEF_hist_vars%([A-Za-z0-9_]+)/g)].map((match) => match[1]),
  );
  for (const name of coreHistoryVariables) assert.ok(all.has(name), name);
  assert.ok(coreHistoryVariables.length < all.size / 10);
  const variables = [...all].map((name) => ({ name, settable: name !== 'gpp' }));
  const selection = historySelection(variables, {});
  assert.equal(selection.available.length, all.size - 1);
  assert.equal(selection.core.length, coreHistoryVariables.length - 1);
  assert.notEqual(selection.core.length, selection.available.length);
});

test('presets switch only writable variables and clear-all leaves nothing selected', () => {
  const variables = [
    { name: 'gpp', settable: true },
    { name: 'fsena', settable: true },
    { name: 'leafc', settable: true },
    { name: 'cropprod1c', settable: false },
  ];
  const core = historyPreset(variables, 'core');
  assert.deepEqual(core, {
    [historyKey('gpp')]: '.true.',
    [historyKey('fsena')]: '.true.',
    [historyKey('leafc')]: '.false.',
  });
  assert.equal(historySelection(variables, core).selected, 2);
  const all = historyPreset(variables, 'all');
  assert.equal(historySelection(variables, all).selected, 3);
  assert.ok(!(historyKey('cropprod1c') in all), 'unwritable variables are left untouched');
  assert.equal(historySelection(variables, historyPreset(variables, 'none')).selected, 0);
});

test('blocked history variables explain themselves in English too', () => {
  assert.equal(historyBlockedReason('需要 DEF_USE_BGC', true), 'Requires DEF_USE_BGC');
  assert.equal(
    historyBlockedReason('本内核未编入：需要 CROP', true),
    'Not compiled into this kernel: requires CROP',
  );
  assert.equal(historyBlockedReason('需要 DEF_USE_BGC', false), '需要 DEF_USE_BGC');
});
