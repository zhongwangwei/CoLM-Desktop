import test from 'node:test';
import assert from 'node:assert/strict';
import { applySourceChoice, sourceGuardReason, type SourceEditState } from './source-edits';

function makeState(options: { dirty?: boolean; canSave?: boolean; saveResult?: boolean } = {}) {
  const calls: string[] = [];
  const state: SourceEditState = {
    dirty: options.dirty ?? true,
    busy: false,
    path: 'vendor/CoLM202X/main/MOD_Demo.F90',
    canSave: options.canSave ?? true,
    save: async () => {
      calls.push('save');
      return options.saveResult ?? true;
    },
    discard: () => calls.push('discard'),
  };
  return { state, calls };
}

test('a cancelled unsaved-source prompt keeps the text and the page', async () => {
  const { state, calls } = makeState();
  assert.equal(await applySourceChoice(state, 'cancel'), false);
  assert.deepEqual(calls, [], 'cancelling neither saves nor discards');
});

test('discarding is explicit and allows the pending action to continue', async () => {
  const { state, calls } = makeState();
  assert.equal(await applySourceChoice(state, 'discard'), true);
  assert.deepEqual(calls, ['discard']);
});

test('saving continues only when the write succeeded; a failure keeps the user text', async () => {
  const ok = makeState({ saveResult: true });
  assert.equal(await applySourceChoice(ok.state, 'save'), true);
  assert.deepEqual(ok.calls, ['save'], 'a successful save does not discard the buffer');
  const failed = makeState({ saveResult: false });
  assert.equal(await applySourceChoice(failed.state, 'save'), false);
  assert.deepEqual(failed.calls, ['save'], 'a failed save must not silently discard the text');
});

test('without an unsaved buffer there is nothing to ask', async () => {
  assert.equal(await applySourceChoice(null, 'save'), true);
  // 取消在任何情况下都返回 false（不继续原操作）；没有缓冲区时保存直接放行。
  assert.equal(await applySourceChoice(null, 'cancel'), false);
});

test('the navigation guard names unsaved source edits in both languages', () => {
  const { state } = makeState();
  assert.equal(
    sourceGuardReason(state, (zh, en) => en),
    'Source files have unsaved edits; save or discard them first',
  );
  assert.equal(
    sourceGuardReason(state, (zh) => zh),
    '源码文件有未保存的修改，请先保存或放弃',
  );
  assert.equal(
    sourceGuardReason({ ...state, dirty: false }, (zh) => zh),
    null,
  );
  assert.equal(
    sourceGuardReason(null, (zh) => zh),
    null,
  );
});
