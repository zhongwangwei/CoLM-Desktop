import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import {
  dialogOpenerTarget,
  fallbackFocusTarget,
  restoreDialogFocus,
  usableFocusTarget,
} from './dialog-focus';

const DIALOG = '[role="dialog"], [role="alertdialog"]';
const CLOSED = '[data-state="closed"]';

type FakeTarget = {
  focusOptions: { preventScroll?: boolean }[];
  focus: (options?: { preventScroll?: boolean }) => void;
  isConnected: boolean;
  disabled?: boolean;
  matches: (selectors: string) => boolean;
  closest: (selectors: string) => Element | null;
};

/** 手写的焦点元素替身：node:test 里没有真实 DOM，策略分支只能靠这种鸭子类型覆盖。 */
function target(
  options: {
    body?: boolean;
    connected?: boolean;
    disabled?: boolean;
    dialog?: FakeTarget | null;
    hidden?: boolean;
  } = {},
): FakeTarget {
  const focusOptions: { preventScroll?: boolean }[] = [];
  const node: FakeTarget = {
    focusOptions,
    focus: (value) => focusOptions.push(value ?? {}),
    isConnected: options.connected ?? true,
    matches: (selectors) => Boolean(options.body) && selectors.includes('body'),
    // 真实 Element.closest 返回 Element；测试替身只需要能被策略判定为"有/无"。
    closest: () => (options.dialog ?? (options.hidden ? {} : null)) as unknown as Element | null,
  };
  if (options.disabled) node.disabled = true;
  return node;
}

function dialogElement(options: { closed?: boolean; children?: FakeTarget[] } = {}) {
  return {
    ...target(),
    matches: (selectors: string) => selectors === CLOSED && Boolean(options.closed),
    querySelectorAll: (selectors: string) =>
      selectors.includes('button') ? (options.children ?? []) : [],
  };
}

function mainRegion(children: FakeTarget[]) {
  return { querySelectorAll: () => children } as unknown as ParentNode;
}

function fakeDocument(
  options: {
    active?: FakeTarget | null;
    dialogs?: ReturnType<typeof dialogElement>[];
    main?: ParentNode | null;
  } = {},
) {
  return {
    activeElement: options.active ?? null,
    querySelector: (selectors: string) => (selectors === 'main' ? (options.main ?? null) : null),
    querySelectorAll: (selectors: string) => (selectors === DIALOG ? (options.dialogs ?? []) : []),
  } as unknown as Document;
}

test('the opener is the element focused before opening, never BODY or a detached entry', () => {
  const button = target();
  assert.equal(dialogOpenerTarget(fakeDocument({ active: button })), button);
  assert.equal(dialogOpenerTarget(fakeDocument({ active: target({ body: true }) })), null);
  assert.equal(dialogOpenerTarget(fakeDocument({ active: target({ connected: false }) })), null);
  assert.equal(dialogOpenerTarget(fakeDocument({ active: target({ disabled: true }) })), null);
  assert.equal(usableFocusTarget(null), false);
});

test('without a document nothing is recorded or restored', () => {
  const opener = target();
  let prevented = 0;
  assert.equal(dialogOpenerTarget(null), null);
  assert.equal(
    restoreDialogFocus(
      opener,
      {
        preventDefault: () => {
          prevented += 1;
        },
      },
      null,
    ),
    false,
  );
  assert.equal(prevented, 0);
  assert.deepEqual(opener.focusOptions, []);
});

test('closing restores focus to the opener with preventScroll and claims the close event', () => {
  const opener = target();
  let prevented = 0;
  const handled = restoreDialogFocus(
    opener,
    {
      preventDefault: () => {
        prevented += 1;
      },
    },
    fakeDocument({ active: target({ body: true }) }),
  );
  assert.equal(handled, true);
  assert.deepEqual(opener.focusOptions, [{ preventScroll: true }]);
  assert.equal(prevented, 1);
});

test('an unmounted or BODY opener falls back to the first focusable control of the page main region', () => {
  for (const opener of [target({ connected: false }), null]) {
    const first = target(),
      second = target();
    let prevented = 0;
    const handled = restoreDialogFocus(
      opener,
      {
        preventDefault: () => {
          prevented += 1;
        },
      },
      fakeDocument({
        active: target({ body: true }),
        main: mainRegion([first, second]),
      }),
    );
    assert.equal(handled, true);
    assert.deepEqual(first.focusOptions, [{ preventScroll: true }]);
    assert.deepEqual(second.focusOptions, []);
    assert.equal(prevented, 1);
  }
});

test('hidden, disabled and BODY controls are skipped when choosing the page fallback', () => {
  const hidden = target({ hidden: true }),
    disabled = target({ disabled: true }),
    body = target({ body: true }),
    usable = target();
  const fallback = fallbackFocusTarget(
    fakeDocument({ main: mainRegion([hidden, disabled, body, usable]) }),
  );
  assert.equal(fallback, usable);
});

test('while another dialog is still open, focus falls back inside it instead of the aria-hidden page', () => {
  const inside = target(),
    pageButton = target(),
    openDialog = dialogElement({ children: [inside] }),
    closingDialog = dialogElement({ closed: true, children: [target()] });
  const handled = restoreDialogFocus(
    null,
    undefined,
    fakeDocument({
      active: target({ body: true }),
      dialogs: [closingDialog, openDialog],
      main: mainRegion([pageButton]),
    }),
  );
  assert.equal(handled, true);
  assert.deepEqual(inside.focusOptions, [{ preventScroll: true }]);
  assert.deepEqual(
    pageButton.focusOptions,
    [],
    'the page is aria-hidden while another dialog is open',
  );
});

test('a consecutive dialog keeps its own focus instead of being dragged back to the old page', () => {
  const other = dialogElement(),
    active = target({ dialog: other as unknown as FakeTarget }),
    opener = target();
  let prevented = 0;
  const handled = restoreDialogFocus(
    opener,
    {
      preventDefault: () => {
        prevented += 1;
      },
    },
    fakeDocument({ active, dialogs: [other] }),
  );
  assert.equal(handled, false);
  assert.equal(prevented, 0);
  assert.deepEqual(opener.focusOptions, []);
});

test('a focus that only remains inside the closing dialog does not block the restore', () => {
  const closing = dialogElement({ closed: true }),
    containing = target({ dialog: closing as unknown as FakeTarget }),
    opener = target();
  assert.equal(restoreDialogFocus(opener, undefined, fakeDocument({ active: containing })), true);
  assert.deepEqual(opener.focusOptions, [{ preventScroll: true }]);
});

test('without any usable target the close event is left to Radix', () => {
  let prevented = 0;
  const handled = restoreDialogFocus(
    null,
    {
      preventDefault: () => {
        prevented += 1;
      },
    },
    fakeDocument({ active: target({ body: true }) }),
  );
  assert.equal(handled, false);
  assert.equal(prevented, 0);
});

/**
 * 从 index.tsx 里抽出真正的共享 Dialog，用假 React/Radix 渲染，验证接线（记录入口 → onCloseAutoFocus）。
 * 复现的是当前源码的组件逻辑，不是真实浏览器键盘事件。
 */
function dialogHarness() {
  const path = new URL('./index.tsx', import.meta.url);
  const source = readFileSync(path, 'utf8');
  const file = ts.createSourceFile(
    path.pathname,
    source,
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  let body = '';
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === 'Dialog') body = node.getText(file);
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.ok(body.includes('function Dialog'), 'the shared Dialog implementation is found');
  const code = ts.transpileModule(`const fn=(${body.replace('export ', '')});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;

  const refs: { current: unknown }[] = [];
  const effects: { deps?: unknown[]; run: () => void }[] = [];
  const pending: (() => void)[] = [];
  let refCursor = 0,
    effectCursor = 0,
    opener: FakeTarget | null = null;
  const recordCalls: unknown[] = [];
  const restoreCalls: { opener: unknown; event: unknown }[] = [];
  const recordStub = () => {
    recordCalls.push(opener);
    return opener;
  };
  const restoreStub = (next: unknown, event: unknown) => {
    restoreCalls.push({ opener: next, event });
    return true;
  };
  const useRef = (initial: unknown) => {
    const index = refCursor++;
    refs[index] ??= { current: initial };
    return refs[index];
  };
  const depsChanged = (previous: unknown[] | undefined, next?: unknown[]) => {
    if (!previous || !next || previous.length !== next.length) return true;
    return next.some((value, index) => !Object.is(value, previous[index]));
  };
  const useLayoutEffect = (run: () => void, deps?: unknown[]) => {
    const index = effectCursor++;
    const previous = effects[index];
    if (depsChanged(previous?.deps, deps)) pending.push(run);
    effects[index] = { deps: deps ? [...deps] : undefined, run };
  };
  const React = {
    createElement: (
      type: unknown,
      props: Record<string, unknown> | null,
      ...children: unknown[]
    ) => ({ type, props: { ...(props ?? {}), children } }),
    Fragment: 'Fragment',
  };
  const RadixDialog = {
    Root: 'DialogRoot',
    Portal: 'DialogPortal',
    Overlay: 'DialogOverlay',
    Content: 'DialogContent',
    Title: 'DialogTitle',
    Description: 'DialogDescription',
    Close: 'DialogClose',
  };
  const Dialog = new Function(
    'React',
    'RadixDialog',
    'cn',
    'Button',
    'useId',
    'useApp',
    'useRenderedAction',
    'useRef',
    'useLayoutEffect',
    'dialogOpenerTarget',
    'restoreDialogFocus',
    `${code}; return fn;`,
  )(
    React,
    RadixDialog,
    (value: string) => value,
    'Button',
    () => 'dialog-id',
    () => ({ state: { language: 'zh' } }),
    () => {},
    useRef,
    useLayoutEffect,
    recordStub,
    restoreStub,
  ) as (props: Record<string, unknown>) => unknown;
  let tree: unknown,
    currentOpen = false;
  function walk(node: unknown): { type: unknown; props: Record<string, unknown> }[] {
    if (!node || typeof node !== 'object') return [];
    if (Array.isArray(node)) return node.flatMap(walk);
    const element = node as { type: unknown; props: Record<string, unknown> };
    return [element, ...walk(element.props?.children)];
  }
  function collect() {
    refCursor = 0;
    effectCursor = 0;
    tree = Dialog({ title: 'Dialog title', open: currentOpen, onOpenChange: () => {} });
    for (const run of pending.splice(0)) run();
  }
  return {
    render(open: boolean) {
      currentOpen = open;
      collect();
      return tree;
    },
    runEffectsAgain() {
      for (const effect of effects) effect.run();
    },
    content: () => walk(tree).find((element) => element.type === RadixDialog.Content),
    tree: () => tree,
    recordCalls,
    restoreCalls,
    setOpener: (next: FakeTarget | null) => {
      opener = next;
    },
  };
}

test('the shared Dialog records the pre-open focus and restores it through its close handler', () => {
  const harness = dialogHarness(),
    opener = target();
  harness.setOpener(opener);
  harness.render(false);
  assert.equal(harness.recordCalls.length, 0, 'closed dialogs do not record');
  harness.render(true);
  assert.deepEqual(harness.recordCalls, [opener]);
  const content = harness.content();
  assert.ok(content, 'RadixDialog.Content still renders the modal content');
  // Escape、遮罩和关闭按钮都先落到 Radix 的 onOpenChange，再卸载 content 触发 onCloseAutoFocus。
  const root = walkRoot(harness.tree());
  assert.equal(typeof root?.props.onOpenChange, 'function');
  const closeHandler = content.props.onCloseAutoFocus as (event: unknown) => void;
  assert.equal(typeof closeHandler, 'function');
  const event = { preventDefault: () => {} };
  closeHandler(event);
  assert.deepEqual(harness.restoreCalls, [{ opener, event }]);
});

test('re-renders and StrictMode effect replays never overwrite the recorded opener', () => {
  const harness = dialogHarness(),
    first = target(),
    second = target();
  harness.setOpener(first);
  harness.render(false);
  harness.render(true);
  harness.render(true);
  harness.runEffectsAgain();
  assert.deepEqual(harness.recordCalls, [first], 'the opener is recorded only on false → true');
  // 关掉再打开才记录新的入口。
  harness.setOpener(second);
  harness.render(false);
  harness.render(true);
  assert.deepEqual(harness.recordCalls, [first, second]);
  const content = harness.content();
  assert.ok(content);
  (content.props.onCloseAutoFocus as (event: unknown) => void)({});
  assert.deepEqual(harness.restoreCalls, [{ opener: second, event: {} }]);
});

test('the modal content still sits under the Radix root and portal that provide the focus trap', () => {
  const harness = dialogHarness();
  harness.render(true);
  const types = walkTypes(harness.tree());
  assert.deepEqual(types.slice(0, 3), ['DialogRoot', 'DialogPortal', 'DialogOverlay']);
  assert.ok(types.includes('DialogContent'), 'the trapped Radix content is unchanged');
  assert.ok(types.includes('DialogClose'), 'the close button still closes through Radix');
});

function walkRoot(node: unknown): { type: unknown; props: Record<string, unknown> } | null {
  if (!node || typeof node !== 'object') return null;
  if (Array.isArray(node)) return walkRoot(node[0]);
  return node as { type: unknown; props: Record<string, unknown> };
}

function walkTypes(node: unknown): string[] {
  if (!node || typeof node !== 'object') return [];
  if (Array.isArray(node)) return node.flatMap(walkTypes);
  const element = node as { type: unknown; props: Record<string, unknown> };
  return [String(element.type), ...walkTypes(element.props?.children)];
}
