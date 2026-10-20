import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import type { FrozenRevision } from '../../lib/types';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { ChangeReport } = await import('./VersionChanges');
type Report = import('./VersionChanges').VersionChangeReport;

const parent = {
  version: { id: 'official', revision: 99 },
  name: 'CoLM202X',
} as FrozenRevision;
const report = (patch: Partial<Report> = {}): Report => ({
  version: { id: 'model-1', revision: 1 },
  against: { id: 'official', revision: 99 },
  against_is_parent: true,
  code: {
    kind: 'workspace',
    workspace: 'ver-1',
    from_official: [{ path: 'vendor/CoLM202X/main/MOD_Root.F90', added: 12, removed: 3 }],
    touches: { engine: false, kernel: true },
    differs: ['vendor/CoLM202X/main/MOD_Root.F90'],
    note: null,
  },
  parameters: [{ key: 'DEF_TUNING_CNFAC', before: null, after: '0.26' }],
  physics: [],
  process_files: [],
  network: null,
  studies: [
    {
      study: '/p/.colm/studies/s-1',
      purpose: 'parameters',
      sites: ['cng'],
      best_member: 'm000001',
      best_objective: 1.6581566,
      targets: ['Qle nrmse'],
    },
  ],
  origin: null,
  ...patch,
});
const render = (value: Report, en: boolean) =>
  renderToStaticMarkup(createElement(ChangeReport, { report: value, versions: [parent], en }));

test('a version lists its changed code, its overrides against the parent and its study', () => {
  const html = render(report(), false);
  assert.ok(html.includes('相对官方源码改了 1 个文件（Fortran 内核）'));
  assert.ok(html.includes('+12 −3'));
  assert.ok(
    html.includes('与父版本 CoLM202X · r99内容不同的文件：vendor/CoLM202X/main/MOD_Root.F90'),
  );
  assert.ok(html.includes('DEF_TUNING_CNFAC'));
  assert.ok(html.includes('— → 0.26'));
  assert.ok(html.includes('导入率定参数 · cng · Qle nrmse · 最优 m000001 (1.658)'));
  const en = render(report(), true);
  assert.ok(en.includes('1 files changed from the official source (Fortran kernel)'));
  assert.ok(en.includes('Calibrated parameters · cng · Qle nrmse · best m000001 (1.658)'));
});

test('identical versions say so and a comparison limit is explained in both languages', () => {
  const same = report({
    code: {
      ...report().code,
      differs: [],
      note: 'different_official_source',
    },
    parameters: [],
    studies: [],
    origin: 'case:abc',
  });
  const html = render(same, false);
  assert.ok(html.includes('代码与父版本 CoLM202X · r99相同。'));
  assert.ok(html.includes('参数、物理方案、过程文件与 AI 网络都与父版本 CoLM202X · r99相同。'));
  assert.ok(html.includes('两个版本基于不同的官方源码'));
  assert.ok(html.includes('case:abc'));
  assert.ok(render(same, true).includes('based on different official sources'));
  const official = render(
    report({
      against: null,
      code: { ...report().code, workspace: null, from_official: [], differs: [] },
      parameters: [],
      studies: [],
    }),
    true,
  );
  assert.ok(official.includes('The official source, unchanged.'));
});
