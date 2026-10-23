import { it } from 'node:test';
import assert from 'node:assert/strict';
import { workspaceErrorText } from './workspace-errors';

it('external-editor errors read in both languages', () => {
  assert.deepEqual(workspaceErrorText('Error: 找不到程序 /Applications/Code.app'), [
    '找不到程序 /Applications/Code.app',
    'Cannot find the app /Applications/Code.app',
  ]);
  assert.equal(
    workspaceErrorText('/Applications/X.app 没能打开 /w/src')[1],
    '/Applications/X.app could not open /w/src',
  );
  const [zh, en] = workspaceErrorText(
    'oracle/golden/a.nc was changed outside the app: golden references are protected',
  );
  assert.match(zh, /oracle\/golden\/a\.nc 在软件外被改了.*golden references are protected/);
  assert.match(en, /Undo that change/);
  // colm-cli 实际给的是 anyhow 的多行形式（2026-10-23 窗口实测）。
  assert.match(
    workspaceErrorText(
      'Error: oracle/golden/a.nc was changed outside the app\n\nCaused by:\n    oracle/golden/a.nc is protected (reference answers and git internals cannot be patched)',
    )[0],
    /^oracle\/golden\/a\.nc 在软件外被改了，但这个位置不能改：oracle\/golden\/a\.nc is protected/,
  );
  assert.deepEqual(workspaceErrorText('boom'), ['boom', 'boom']);
});
