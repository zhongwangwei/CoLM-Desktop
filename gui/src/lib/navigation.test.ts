import test from 'node:test';
import assert from 'node:assert/strict';
import { activeNavigation, navigation, navigationTarget } from './navigation';

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

test('Simulation returns to the current batch after visiting results or another sidebar page', () => {
  const simulation = navigation.find((item) => item.id === 'simulation')!;
  const results = navigation.find((item) => item.id === 'results')!;
  const state = {
    root: '/multi',
    projectView: { root: '/multi', page: 'batch' as const },
  };
  assert.deepEqual(navigationTarget(simulation, state), { route: 'project' });
  assert.deepEqual(navigationTarget(results, state), { route: 'case', tab: 'results' });
  assert.equal(activeNavigation({ ...state, route: 'project', caseTab: 'results' }), 'simulation');
  assert.deepEqual(navigationTarget(simulation, { ...state, root: '/other' }), {
    route: 'case',
    tab: 'settings',
  });
  assert.deepEqual(navigationTarget(simulation, { root: '/multi' }), {
    route: 'case',
    tab: 'settings',
  });
});
