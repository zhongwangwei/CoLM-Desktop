import { it } from 'node:test';
import assert from 'node:assert/strict';
import {
  ALREADY_EXISTS,
  CONTENT_CONFLICT,
  isAlreadyExists,
  isContentConflict,
  workspaceErrorText,
} from './workspace-errors';

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

it('write conflicts and name clashes are recognisable by stable markers', () => {
  // 写入端用稳定标记区分"可恢复的冲突"和其它错误；整句会随路径变，不能靠整句匹配。
  const changed =
    'Error: content-conflict: vendor/a.F90 changed on disk since it was opened; reload it or overwrite explicitly';
  assert.equal(isContentConflict(changed), true);
  assert.equal(isContentConflict('Error: boom'), false);
  assert.equal(isAlreadyExists(changed), false);
  const exists =
    'Error: already-exists: vendor/a.F90 already exists in the workspace; open it instead, or replace it explicitly';
  assert.equal(isAlreadyExists(exists), true);
  assert.equal(isContentConflict(exists), false);
  // 标记本身进双语映射，不把英文整句漏给中文界面。
  const [zh] = workspaceErrorText(changed);
  assert.match(zh, /^vendor\/a\.F90 在软件外被改了，保存已取消/);
  const [zhExists, enExists] = workspaceErrorText(exists);
  assert.match(zhExists, /^vendor\/a\.F90 已经存在，原文件没有被改动/);
  assert.match(enExists, /already exists; the existing file was not changed/);
  const [, enDeleted] = workspaceErrorText(
    'Error: content-conflict: vendor/a.F90 was deleted or replaced outside the app since it was opened; reload it or save it again explicitly',
  );
  assert.match(enDeleted, /deleted or replaced outside the app since it was opened/);
  assert.equal(CONTENT_CONFLICT, 'content-conflict:');
  assert.equal(ALREADY_EXISTS, 'already-exists:');
});
