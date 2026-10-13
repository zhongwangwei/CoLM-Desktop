import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { fullSeriesCsv } from './series';

const source = ts.createSourceFile(
  'ResultsPage.tsx',
  readFileSync(new URL('./ResultsPage.tsx', import.meta.url), 'utf8'),
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const definitions: Record<string, string> = {};
function visit(node: ts.Node) {
  if (
    ts.isFunctionDeclaration(node) &&
    node.name &&
    ['task', 'exportMessage'].includes(node.name.text)
  )
    definitions[node.name.text] = node.getText(source);
  if (ts.isVariableDeclaration(node) && node.name.getText(source) === 'exportFullSeries')
    definitions.exportFullSeries = `const exportFullSeries=${node.initializer!.getText(source)};`;
  ts.forEachChild(node, visit);
}
visit(source);
function harness() {
  let busy = false,
    error = '',
    notice = '';
  let finish!: (value: unknown) => void, fail!: (reason: Error) => void;
  const calls: unknown[][] = [];
  const context = {
    lock: { current: false },
    epoch: { current: 1 },
    setBusy: (value: boolean) => {
      busy = value;
    },
    setError: (value: string) => {
      error = value;
    },
    tr: (zh: string) => zh,
    setExportNotice: (value: string) => {
      notice = value;
    },
    source: '/case/run',
    variable: 'GPP',
    fullSeriesCsv,
    readSeries: async () => ({
      time: [1, 2],
      vars: { GPP: [3, 4] },
      n: 2,
      source_n: 2,
      downsampled: false,
    }),
    download: (...args: unknown[]) => {
      calls.push(args);
      return new Promise((resolve, reject) => {
        finish = resolve;
        fail = reject;
      });
    },
  };
  const code = ts.transpileModule(Object.values(definitions).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const run = new Function(...Object.keys(context), `${code};return exportFullSeries;`)(
    ...Object.values(context),
  ) as () => Promise<{ status: string }>;
  return {
    run,
    calls,
    get busy() {
      return busy;
    },
    get error() {
      return error;
    },
    get notice() {
      return notice;
    },
    resolve: (value: unknown) => finish(value),
    reject: (reason: Error) => fail(reason),
  };
}
test('result export stays locked until native completion and distinguishes cancellation from success', async () => {
  for (const status of ['cancelled', 'saved']) {
    const h = harness(),
      pending = h.run();
    await Promise.resolve();
    assert.equal(h.busy, true);
    assert.equal(h.notice, '');
    await assert.rejects(h.run(), /in progress/);
    assert.equal(h.calls.length, 1);
    h.resolve({ status, path: status === 'saved' ? '/chosen/run.csv' : null });
    assert.equal((await pending).status, status);
    assert.equal(h.busy, false);
    assert.match(h.notice, status === 'saved' ? /已保存到：\/chosen\/run.csv/ : /已取消/);
  }
});
test('result export exposes native write failures and unlocks for retry without a success notice', async () => {
  const h = harness(),
    pending = h.run();
  const failed = assert.rejects(pending, /permission denied/);
  await Promise.resolve();
  h.reject(new Error('permission denied'));
  await failed;
  assert.match(h.error, /permission denied/);
  assert.equal(h.notice, '');
  assert.equal(h.busy, false);
});
