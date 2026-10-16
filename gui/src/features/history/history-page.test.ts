import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as history from './history';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Node = { type: string; props: Record<string, any>; children: unknown[] };
function harness() {
  const source = readFileSync(new URL('./HistoryPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const slots: unknown[] = [],
    effects: { deps: unknown[]; cleanup?: () => void }[] = [];
  let cursor = 0,
    effectCursor = 0;
  const pending: (() => void)[] = [];
  const requests: {
    command: string;
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    args: any;
    resolve: (value: unknown) => void;
    reject: (error: unknown) => void;
  }[] = [];
  const env = {
    ...history,
    React: {
      createElement: (type: string, props: Node['props'], ...children: unknown[]) => ({
        type,
        props: props ?? {},
        children,
      }),
    },
    ...Object.fromEntries(
      [
        'Advanced',
        'Button',
        'Card',
        'ContextMenu',
        'Dialog',
        'Field',
        'Input',
        'Notice',
        'Select',
      ].map((x) => [x, x]),
    ),
    useApp: () => ({
      state: { language: 'en' },
      update: () => {},
      openCase: () => {},
      navigate: () => true,
    }),
    usePageContract: () => {},
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (value: unknown) => {
          slots[i] = typeof value === 'function' ? value(slots[i]) : value;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect: (fn: () => () => void, deps: unknown[]) => {
      const i = effectCursor++;
      if (!effects[i] || deps.some((v, j) => v !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          effects[i].cleanup = fn();
        });
      }
    },
    invokeJson: (command: string, args: unknown) =>
      new Promise((resolve, reject) => requests.push({ command, args, resolve, reject })),
  };
  const Page = new Function(...Object.keys(env), `${code}; return HistoryPage;`)(
    ...Object.values(env),
  );
  let tree: Node;
  const render = () => {
    cursor = effectCursor = 0;
    tree = Page({});
    while (pending.length) pending.shift()!();
  };
  const nodes = (node: unknown): Node[] =>
    Array.isArray(node)
      ? node.flatMap(nodes)
      : node && typeof node === 'object'
        ? [node as Node, ...(node as Node).children.flatMap(nodes)]
        : [];
  const text = (node: unknown): string =>
    Array.isArray(node)
      ? node.map(text).join('')
      : node && typeof node === 'object'
        ? text((node as Node).children)
        : node == null || node === false
          ? ''
          : String(node);
  const menu = (index = 0) => nodes(tree).filter((n) => n.type === 'ContextMenu')[index];
  const click = (label: string) => {
    const button = nodes(tree).find((n) => n.type === 'Button' && text(n) === label);
    assert.ok(button, label);
    assert.ok(!button.props.disabled);
    button.props.onClick();
    render();
  };
  const settle = async () => {
    await new Promise((resolve) => setImmediate(resolve));
    render();
    render();
  };
  render();
  render();
  requests.splice(0, requests.length - 1);
  return {
    requests,
    render,
    settle,
    menu,
    click,
    nodes: () => nodes(tree),
    unmount: () => effects.forEach((e) => e.cleanup?.()),
  };
}
const row = (id: string) => ({
  operation_id: id,
  kind: 'run',
  phase: 'colm',
  status: 'succeeded',
  needs_reconciliation: false,
  time_ms: null,
  case_path: '/case',
  location: 'local',
});

test('record-only deletion confirms without a path, retries errors, and refreshes records', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 1 });
  await h.settle();
  h.menu().props.items[0].onSelect();
  h.render();
  h.click('Confirm deletion');
  assert.deepEqual(h.requests[1].args, { id: 'a', files: false });
  h.requests[1].reject(new Error('busy'));
  await h.settle();
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 1);
  h.click('Confirm deletion');
  h.requests[2].resolve({ removed: true });
  await h.settle();
  assert.equal(h.requests[3].command, 'history_list');
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 0);
});

test('directory plans ignore stale menus and confirmed deletion carries the verified path', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a'), row('b')], total: 2 });
  await h.settle();
  h.menu(0).props.onOpenChange(true);
  h.render();
  h.menu(0).props.onOpenChange(false);
  h.render();
  h.menu(1).props.onOpenChange(true);
  h.render();
  h.requests[1].resolve({ id: 'a', path: '/wrong', host: null, reason: null });
  await h.settle();
  assert.equal(h.menu(1).props.items[1].disabled, true);
  h.requests[2].resolve({ id: 'b', path: '/runs/b', host: 'server', reason: null });
  await h.settle();
  h.menu(1).props.items[1].onSelect();
  h.menu(1).props.onOpenChange(false);
  h.render();
  h.click('Confirm deletion');
  assert.deepEqual(h.requests[3].args, { id: 'b', files: true, expectedPath: '/runs/b' });
  h.unmount();
  h.requests[3].resolve({ removed: true, path: '/runs/b', host: 'server' });
  await h.settle();
  assert.equal(h.requests.length, 4);
});

test('empty final page after deletion falls back to the preceding page', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 31 });
  await h.settle();
  h.click('Next');
  h.requests[1].resolve({ items: [row('last')], total: 31 });
  await h.settle();
  h.menu().props.items[0].onSelect();
  h.render();
  h.click('Confirm deletion');
  h.requests[2].resolve({ removed: true });
  await h.settle();
  h.requests[3].resolve({ items: [], total: 30 });
  await h.settle();
  assert.equal(h.requests[4].args.filter.offset, 0);
});

test('whole-case deletion is a separate third option and requires the directory name', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 1 });
  await h.settle();
  assert.equal(h.menu().props.items.length, 3);
  h.menu().props.onOpenChange(true);
  h.render();
  h.requests[1].resolve({
    id: 'a',
    path: '/cases/Case A/runs/a',
    host: null,
    reason: null,
    casePath: '/cases/Case A',
    caseHost: null,
    caseReason: null,
  });
  await h.settle();
  assert.equal(h.menu().props.items[1].disabled, false);
  assert.equal(h.menu().props.items[2].disabled, false);
  h.menu().props.items[2].onSelect();
  h.render();
  const confirmation = () =>
    h.nodes().find((n) => n.type === 'Button' && n.props.variant === 'danger')!;
  assert.equal(confirmation().props.disabled, true);
  confirmation().props.onClick();
  await h.settle();
  assert.equal(h.requests.length, 2);
  h.click('Cancel');
  assert.equal(h.requests.length, 2);
  h.menu().props.items[2].onSelect();
  h.render();
  const input = () =>
    h.nodes().find((n) => n.type === 'Input' && n.props['aria-label'] === 'Case folder name')!;
  input().props.onChange({ target: { value: 'Case A' } });
  h.render();
  h.click('Confirm deletion');
  assert.equal(h.requests[2].command, 'history_delete');
  assert.deepEqual(h.requests[2].args, {
    id: 'a',
    files: true,
    wholeCase: true,
    expectedPath: '/cases/Case A',
  });
  h.requests[2].resolve({ removed: true, path: '/cases/Case A', host: null });
  await h.settle();
});

test('case ownership rejection disables only whole-case deletion', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 1 });
  await h.settle();
  h.menu().props.onOpenChange(true);
  h.render();
  h.requests[1].resolve({
    id: 'a',
    path: '/cases/a/runs/a',
    host: null,
    reason: null,
    casePath: null,
    caseHost: null,
    caseReason: 'Another run is active',
  });
  await h.settle();
  assert.equal(h.menu().props.items[0].disabled, false);
  assert.equal(h.menu().props.items[1].disabled, false);
  assert.equal(h.menu().props.items[2].disabled, true);
});
