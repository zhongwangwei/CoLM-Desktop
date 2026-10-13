import test from 'node:test';
import assert from 'node:assert/strict';
import { recentProjectRoots, rememberProject, projectName } from './project-model';
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
  assert.throws(() => recentProjectRoots('{}'));
  assert.equal(projectName('/work/grass/'), 'grass');
  assert.equal(projectName('C:\\work\\grass\\'), 'grass');
});
