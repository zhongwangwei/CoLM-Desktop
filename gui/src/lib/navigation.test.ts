import test from 'node:test';
import assert from 'node:assert/strict';
import { activeNavigation, navigation } from './navigation';

test('sidebar groups model/data work under Project and opens real case tabs', () => {
  assert.deepEqual(
    navigation.map((item) => item.zh),
    ['项目', '模拟', '结果', '研究', '历史记录', '计算资源', '设置'],
  );
  for (const route of ['project', 'preparation', 'versions'] as const)
    assert.equal(activeNavigation({ route, caseTab: 'results' }), 'project');
  assert.equal(activeNavigation({ route: 'new-case', caseTab: 'settings' }), 'simulation');
  for (const tab of ['settings', 'runs'] as const)
    assert.equal(activeNavigation({ route: 'case', caseTab: tab }), 'simulation');
  assert.equal(activeNavigation({ route: 'case', caseTab: 'results' }), 'results');
  assert.deepEqual(
    navigation.find((item) => item.id === 'results'),
    { id: 'results', zh: '结果', en: 'Results', route: 'case', tab: 'results' },
  );
  assert.equal(activeNavigation({ route: 'resources', caseTab: 'results' }), 'resources');
});
