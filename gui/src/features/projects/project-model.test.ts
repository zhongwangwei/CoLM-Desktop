import test from 'node:test';
import assert from 'node:assert/strict';
import {
  namesFor,
  recentProjectNames,
  recentProjectRoots,
  rememberProject,
  projectName,
} from './project-model';
test('recent projects keep actual distinct roots in opening order without invented metadata', () => {
  assert.deepEqual(recentProjectRoots(undefined), []);
  assert.deepEqual(recentProjectRoots('["/a",null,"/a","/b",""]'), ['/a', '/b']);
  assert.deepEqual(rememberProject(['/a', '/b'], '/b'), ['/b', '/a']);
  assert.equal(
    rememberProject(
      Array.from({ length: 10 }, (_, i) => `/${i}`),
      '/new',
    ).length,
    8,
  );
  // 坏值不抛错（A-02）：捞出还认得的路径，其余丢掉，调用方回写成合法 JSON。
  assert.deepEqual(recentProjectRoots('{}'), []);
  assert.deepEqual(recentProjectRoots('["/a","/b'), ['/a']);
  assert.deepEqual(recentProjectRoots('not json'), []);
  assert.equal(projectName('/work/grass/'), 'grass');
  assert.equal(projectName('C:\\work\\grass\\'), 'grass');
});

test('recent project names survive bad records and only keep listed projects', () => {
  assert.deepEqual(recentProjectNames(undefined), {});
  assert.deepEqual(recentProjectNames('not json'), {});
  assert.deepEqual(recentProjectNames('["/a"]'), {});
  assert.deepEqual(recentProjectNames('{"/m/1f37":"tianhe: uuu","/b":3,"/c":""}'), {
    '/m/1f37': 'tianhe: uuu',
  });
  assert.deepEqual(namesFor(['/m/1f37', '/d'], { '/m/1f37': 'tianhe: uuu', '/gone': 'old' }), {
    '/m/1f37': 'tianhe: uuu',
  });
});
