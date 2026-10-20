import test from 'node:test';
import assert from 'node:assert/strict';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { removable, usageLabel } = await import('./RevisionTree');

const official = (revision: number) => ({ id: 'official', revision });
const usage = (
  revision: number,
  cases: number,
  history: number,
  blocker: string | null = null,
) => ({
  version: official(revision),
  cases,
  history,
  blocker,
});

test('a revision only history names can be removed; bound, selected and protected ones cannot', () => {
  const selected = official(92);
  assert.ok(removable(usage(83, 0, 95), selected));
  assert.equal(usageLabel(usage(83, 0, 95), selected, false), '只有 95 条历史记录');
  assert.ok(removable(usage(6, 0, 0), selected));
  assert.equal(usageLabel(usage(6, 0, 0), selected, true), 'Unused');
  assert.ok(!removable(usage(70, 90, 270), selected));
  assert.equal(usageLabel(usage(70, 90, 270), selected, false), '90 个算例在用');
  assert.ok(!removable(usage(92, 0, 0), selected), 'the selected version stays');
  assert.equal(usageLabel(usage(92, 0, 0), selected, false), '当前选中');
  const latest = usage(
    100,
    0,
    0,
    'the latest official revision is the bundled baseline and cannot be deleted',
  );
  assert.ok(!removable(latest, selected));
  assert.equal(usageLabel(latest, selected, false), '当前官方版本');
  const parent = usage(95, 0, 1, 'draft model-1 is based on it; delete that draft first');
  assert.ok(!removable(parent, selected));
  assert.equal(usageLabel(parent, selected, true), 'Has derived versions or drafts');
  assert.ok(!removable(undefined, selected), 'nothing is offered before usage is known');
});
