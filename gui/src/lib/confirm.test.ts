import test from 'node:test';
import assert from 'node:assert/strict';
import { askConfirm, setConfirmHost, type ConfirmRequest } from './confirm';

test('confirmations go through the in-app dialog, and without one nothing is confirmed', async () => {
  // No dialog mounted (unit tests, a page rendered alone): treat it as "no", never delete by default.
  setConfirmHost(null);
  assert.equal(await askConfirm('放弃尚未保存的修改？'), false);
  const asked: ConfirmRequest[] = [];
  setConfirmHost((request) => asked.push(request));
  const yes = askConfirm('删除草稿？', { confirmLabel: '删除草稿', danger: true });
  assert.equal(asked[0]!.message, '删除草稿？');
  assert.equal(asked[0]!.confirmLabel, '删除草稿');
  asked[0]!.resolve(true);
  assert.equal(await yes, true);
  const no = askConfirm('同时删除代码工作区？', { cancelLabel: '保留代码' });
  asked[1]!.resolve(false);
  assert.equal(await no, false);
  setConfirmHost(null);
});
