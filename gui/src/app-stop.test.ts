import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

test('shell stop cancels a remote submission without a job id and retains failed stops', async () => {
  const source = readFileSync(new URL('./App.tsx', import.meta.url), 'utf8');
  const start = source.indexOf('  const stopBenchTask =');
  const end = source.indexOf('\n  // 助手面板', start);
  const code = ts.transpileModule(source.slice(start, end), {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const calls: unknown[] = [];
  const cleared: string[] = [];
  const notices: unknown[] = [];
  let reject = false;
  const stop = new Function('invoke', 'benchTasks', 'notify', `${code}; return stopBenchTask;`)(
    (name: string, args: unknown) => {
      calls.push([name, args]);
      return reject ? Promise.reject(new Error('stop failed')) : Promise.resolve();
    },
    { stopTask: () => false, clear: (id: string) => cleared.push(id) },
    (message: string, error: boolean) => notices.push({ message, error }),
  );
  stop({ host: 'server-a', job: null, id: 'a', state: 'running' });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, [['bench_remote_cancel', { host: 'server-a', job: null }]]);
  assert.deepEqual(cleared, ['a']);
  reject = true;
  stop({ host: 'server-a', job: null, id: 'b', state: 'running' });
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(cleared, ['a']);
  assert.deepEqual(notices, [{ message: 'Error: stop failed', error: true }]);
});
