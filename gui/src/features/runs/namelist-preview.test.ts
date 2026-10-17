import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function preview() {
  const states: unknown[] = [];
  let index = 0;
  let effect: () => (() => void) | undefined;
  let cleanup: (() => void) | undefined;
  const elements: Element[] = [];
  const calls: {
    command: string;
    args: unknown;
    resolve: (data: unknown) => void;
    reject: (error: unknown) => void;
  }[] = [];
  const react = {
    useState: (initial: unknown) => {
      const key = index++;
      if (!(key in states)) states[key] = initial;
      return [
        states[key],
        (value: unknown) => {
          states[key] = value;
        },
      ];
    },
    useEffect: (next: typeof effect) => {
      effect = next;
    },
    createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => {
      const el = { type, props: props ?? {}, children };
      elements.push(el);
      return el;
    },
  };
  const exports: { default?: (props: unknown) => void } = {};
  const js = ts.transpileModule(
    readFileSync(new URL('./NamelistPreview.tsx', import.meta.url), 'utf8'),
    {
      compilerOptions: {
        module: ts.ModuleKind.CommonJS,
        jsx: ts.JsxEmit.React,
        target: ts.ScriptTarget.ES2022,
      },
    },
  ).outputText;
  new Function('require', 'exports', 'React', js)(
    (name: string) =>
      name === 'react'
        ? react
        : name === '../../lib/ipc'
          ? {
              invoke: (command: string, args: unknown) =>
                new Promise((resolve, reject) => calls.push({ command, args, resolve, reject })),
            }
          : Object.fromEntries(
              ['Advanced', 'Field', 'Notice', 'Select'].map((name) => [name, name]),
            ),
    exports,
    react,
  );
  const render = (disabled = false) => {
    index = 0;
    elements.length = 0;
    exports.default!({ casePaths: ['/case/first', '/case/second'], disabled, en: false });
  };
  const runEffect = () => {
    cleanup?.();
    cleanup = effect?.();
  };
  render();
  return { render, runEffect, elements, calls };
}
const settle = () => new Promise<void>((resolve) => setImmediate(resolve));

test('NML preview is read-only, lazy and switches batch stations without stale responses', async () => {
  const page = preview();
  page.runEffect();
  assert.equal(page.calls.length, 0);
  (
    page.elements.find((el) => el.type === 'Advanced')!.props.onOpenChange as (
      open: boolean,
    ) => void
  )(true);
  page.render();
  page.runEffect();
  assert.equal(page.calls[0].command, 'case_namelist_preview');
  assert.deepEqual(page.calls[0].args, { dir: '/case/first' });
  (page.elements.find((el) => el.type === 'Select')!.props.onChange as (event: unknown) => void)({
    target: { value: '/case/second' },
  });
  page.render();
  page.runEffect();
  page.calls[1].resolve({
    files: [
      { name: 'case.nml', text: 'second case' },
      { name: 'forcing.nml', text: 'forcing content' },
    ],
    warnings: ['forcing warning'],
  });
  await settle();
  page.calls[0].resolve({ files: [{ name: 'case.nml', text: 'stale first case' }], warnings: [] });
  await settle();
  page.render();
  assert.deepEqual(
    page.elements.filter((el) => el.type === 'pre').map((el) => el.children[0]),
    ['second case', 'forcing content'],
  );
  assert.ok(
    page.elements.some((el) => el.type === 'Notice' && el.children[0] === 'forcing warning'),
  );
  assert.ok(page.calls.every((call) => call.command === 'case_namelist_preview'));
});

test('unsaved settings hide previous preview and load failures are explicit', async () => {
  const page = preview();
  (
    page.elements.find((el) => el.type === 'Advanced')!.props.onOpenChange as (
      open: boolean,
    ) => void
  )(true);
  page.render();
  page.runEffect();
  page.calls[0].resolve({ files: [{ name: 'case.nml', text: 'old saved case' }], warnings: [] });
  await settle();
  page.render(true);
  page.runEffect();
  assert.equal(page.elements.filter((el) => el.type === 'pre').length, 0);
  assert.ok(
    page.elements.some((el) => el.type === 'Notice' && String(el.children[0]).includes('请先保存')),
  );
  page.render();
  page.runEffect();
  page.calls[1].reject(new Error('cannot read case'));
  await settle();
  page.render();
  assert.ok(
    page.elements.some(
      (el) => el.type === 'Notice' && String(el.children[0]).includes('cannot read case'),
    ),
  );
});
