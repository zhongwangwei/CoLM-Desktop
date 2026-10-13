import test from 'node:test';
import assert from 'node:assert/strict';
import { navigationBlock } from './navigation-guard';
test('unsaved pages block routes and case reloads without changing same-page tabs', () => {
  let dirty = true;
  const guards = [() => (dirty ? 'Save or discard edits' : null)],
    current = { route: 'case' as const, caseDir: '/a' };
  assert.equal(navigationBlock(current, { route: 'research' }, guards), 'Save or discard edits');
  assert.equal(
    navigationBlock(current, { route: 'case', caseDir: '/b', reload: true }, guards),
    'Save or discard edits',
  );
  assert.equal(
    navigationBlock(current, { route: 'case', caseDir: '/a', reload: true }, guards),
    'Save or discard edits',
  );
  assert.equal(navigationBlock(current, { route: 'case' }, guards), null);
  dirty = false;
  assert.equal(navigationBlock(current, { route: 'research' }, guards), null);
});
