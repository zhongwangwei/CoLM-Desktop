import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { visibleBenchTasks } from '../lib/bench-task';
import { benchErrorText, remoteJobFailureText } from '../features/compare/compare';

test('completed task rows expire without another task update', () => {
  const source = readFileSync(new URL('./BenchTaskBar.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  let now = 1000;
  let state = now;
  let effect: () => void | (() => void) = () => {};
  let timer: (() => void) | undefined;
  const context = {
    React: { createElement: (...args: unknown[]) => args },
    Button: 'Button',
    visibleBenchTasks,
    benchErrorText,
    Date: { now: () => now },
    useState: () => [
      state,
      (value: number) => {
        state = value;
      },
    ],
    useEffect: (fn: typeof effect) => {
      effect = fn;
    },
    setInterval: (fn: () => void) => {
      timer = fn;
      return 1;
    },
    clearInterval: () => {
      timer = undefined;
    },
  };
  const Component = new Function(...Object.keys(context), `${js}; return BenchTaskBar;`)(
    ...Object.values(context),
  );
  const props = {
    tasks: [{ id: 'done', state: 'done', label: 'study', finishedAt: now, startedAt: 0 }],
    t: (_zh: string, en: string) => en,
  };
  assert.ok(Component(props));
  const cleanup = effect();
  assert.ok(timer, 'the last completed row still has an expiry clock');
  now += 5 * 60 * 1000;
  timer();
  assert.equal(Component(props), null);
  cleanup?.();
  effect();
  assert.equal(timer, undefined, 'the timer stops once all rows expire');
});

test('an already failed server task changes language without changing its stored error', () => {
  const source = readFileSync(new URL('./BenchTaskBar.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const now = Date.now();
  const context = {
    React: { createElement: (...args: unknown[]) => args },
    Button: 'Button',
    visibleBenchTasks,
    benchErrorText,
    useState: () => [now, () => {}],
    useEffect: () => {},
  };
  const Component = new Function(...Object.keys(context), `${js}; return BenchTaskBar;`)(
    ...Object.values(context),
  );
  for (const status of [{ exit_code: 9 }, {}, { detail: 'TIMEOUT' }]) {
    const error = remoteJobFailureText(status);
    const tasks = [
      {
        id: 'failed',
        state: 'failed',
        label: 'study',
        finishedAt: now,
        startedAt: now - 1000,
        error,
      },
    ];
    let en = false;
    const props = { tasks, t: (zh: string, english: string) => (en ? english : zh) };
    assert.ok(JSON.stringify(Component(props)).includes('服务器上的作业失败'));
    en = true;
    assert.ok(JSON.stringify(Component(props)).includes('The server job failed'));
    assert.equal(tasks[0].error, error);
    assert.deepEqual(benchErrorText(benchErrorText(error)[0]), benchErrorText(error));
  }
});
