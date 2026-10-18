import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as history from './history';

// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Node = { type: string; props: Record<string, any>; children: unknown[] };
function harness(running: Record<string, { running: boolean; run_id: string }> = {}) {
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
  let contract: { actions: { id: string; run: () => unknown }[] };
  const downloads: { content: string; name: string; type: string }[] = [];
  const env = {
    ...history,
    download: async (content: string, name: string, type: string) => {
      downloads.push({ content, name, type });
    },
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
      state: { language: 'en', running },
      update: () => {},
      openCase: () => {},
      navigate: () => true,
    }),
    usePageContract: (value: typeof contract) => {
      contract = value;
    },
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
  const openCard = (index = 0) => {
    const card = nodes(tree).filter(
      (n) => n.type === 'Button' && String(n.props.className).includes('history-card'),
    )[index];
    assert.ok(card, 'history card');
    assert.ok(!card.props.disabled);
    card.props.onClick();
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
    downloads,
    guide: () => contract,
    render,
    settle,
    menu,
    click,
    openCard,
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

const groupRow = () => ({
  ...row('a'),
  case_path: null,
  run_group: {
    run_id: 'batch-1',
    members: [
      { ...row('a'), case_path: '/project/AT-Neu', detail: {} },
      { ...row('b'), case_path: '/project/CN-Cng', detail: {} },
    ],
  },
});

test('one run row opens all stations and nonfirst station actions use its real operation', async () => {
  const h = harness();
  const group = groupRow();
  h.requests[0].resolve({ items: [group], total: 1 });
  await h.settle();
  assert.equal(h.nodes().filter((n) => n.type === 'ContextMenu').length, 1);
  h.openCard();
  await h.settle();
  assert.equal(h.requests.length, 1, 'group view must not load representative station');
  assert.equal(h.nodes().filter((n) => n.type === 'ContextMenu').length, 3);
  assert.equal(
    h.nodes().some((n) => n.type === 'Button' && n.children.includes('Replay recorded settings')),
    false,
  );
  h.click('CN-Cng · Completed');
  assert.equal(h.requests[1].command, 'history_detail');
  assert.deepEqual(h.requests[1].args, { id: 'b' });
  h.requests[1].resolve({ operation: group.run_group.members[1], events: [], related: [] });
  await h.settle();
  assert.equal(
    h.nodes().filter((n) => n.type === 'ContextMenu').length,
    3,
    'overview stays available',
  );
  h.menu(2).props.onOpenChange(true);
  h.render();
  assert.equal(h.requests[2].command, 'history_delete_plan');
  assert.deepEqual(h.requests[2].args, { id: 'b' });
});

test('group deletion is record only, carries group scope, and refreshes overview', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [groupRow()], total: 1 });
  await h.settle();
  h.openCard();
  await h.settle();
  h.menu().props.onOpenChange(true);
  h.render();
  assert.equal(h.requests.length, 1, 'never request representative filesystem plan');
  assert.equal(h.menu().props.items.length, 1);
  h.menu().props.items[0].onSelect();
  h.render();
  h.click('Confirm deletion');
  assert.deepEqual(h.requests[1].args, { id: 'a', files: false, group: true });
  h.requests[1].resolve({ removed: true });
  await h.settle();
  assert.equal(h.requests[2].command, 'history_list');
  assert.equal(h.nodes().filter((n) => n.type === 'ContextMenu').length, 0);
});

test('current GUI run prevents deleting an apparently completed partial group', async () => {
  const h = harness({ active: { run_id: 'batch-1', running: true } });
  h.requests[0].resolve({ items: [groupRow()], total: 1 });
  await h.settle();
  assert.equal(h.menu().props.items[0].disabled, true);
  h.menu().props.items[0].onSelect();
  h.render();
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 0);
});

test('run export retains all station evidence and filtering clears group actions', async () => {
  const h = harness();
  const group = groupRow();
  h.requests[0].resolve({ items: [group], total: 1 });
  await h.settle();
  h.openCard();
  await h.settle();
  h.click('Export this run JSON');
  assert.deepEqual(
    h.requests.slice(1).map((r) => r.args.id),
    ['a', 'b'],
  );
  group.run_group.members.forEach((member, i) =>
    h.requests[i + 1].resolve({
      operation: member,
      events: [{ evidence: member.operation_id }],
      related: [],
    }),
  );
  await h.settle();
  const exported = JSON.parse(h.downloads[0].content);
  assert.equal(exported.operation.run_group.run_id, 'batch-1');
  assert.deepEqual(
    exported.members.map((m: { events: unknown[] }) => m.events),
    [[{ evidence: 'a' }], [{ evidence: 'b' }]],
  );
  h.nodes()
    .find((n) => n.type === 'form')!
    .props.onSubmit({ preventDefault() {} });
  await h.settle();
  assert.equal(h.requests[3].command, 'history_list');
  assert.equal(h.nodes().filter((n) => n.type === 'ContextMenu').length, 0);
});

test('assistant details action opens the same group overview without selecting a representative', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [groupRow()], total: 1 });
  await h.settle();
  await h
    .guide()
    .actions.find((action) => action.id === 'history-detail-a')!
    .run();
  await h.settle();
  assert.equal(h.requests.length, 1);
  assert.equal(h.nodes().filter((n) => n.type === 'ContextMenu').length, 3);
  const selecting = h
    .guide()
    .actions.find((action) => action.id === 'history-member-b')!
    .run();
  assert.deepEqual(h.requests[1].args, { id: 'b' });
  h.requests[1].resolve({ operation: groupRow().run_group.members[1], events: [], related: [] });
  await selecting;
  await h.settle();
  assert.ok(h.guide().actions.some((action) => action.id === 'history-export-run'));
});

test('clicking a card opens its details in a dialog that closes back to the cards', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 1 });
  await h.settle();
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 0);
  h.openCard();
  assert.deepEqual(h.requests[1], {
    ...h.requests[1],
    command: 'history_detail',
    args: { id: 'a' },
  });
  h.requests[1].resolve({ operation: row('a'), events: [], related: [] });
  await h.settle();
  const dialog = h.nodes().find((n) => n.type === 'Dialog')!;
  assert.ok(dialog);
  dialog.props.onOpenChange(false);
  h.render();
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 0);
});

test('clearing history deletes case folders only when the user ticks them', async () => {
  const h = harness();
  h.requests[0].resolve({ items: [row('a')], total: 1 });
  await h.settle();
  h.click('Clear history');
  assert.equal(h.requests[1].command, 'history_clear_plan');
  h.requests[1].resolve({
    records: 1,
    unconfirmed: 2,
    unconfirmedRemote: 1,
    cases: [{ path: '/case', bytes: 2048 }],
    remoteCases: 0,
    skipped: [],
  });
  await h.settle();
  h.click('Confirm clearing records');
  assert.deepEqual(h.requests[2].args, { cases: [], unconfirmed: false });
  h.requests[2].reject(new Error('busy'));
  await h.settle();
  const tick = (label: string) => {
    const text = (node: unknown): string =>
      Array.isArray(node)
        ? node.map(text).join('')
        : node && typeof node === 'object'
          ? text((node as { children: unknown[] }).children)
          : node == null || node === false
            ? ''
            : String(node);
    const owner = h.nodes().find((n) => n.type === 'label' && text(n).includes(label))!;
    const box = owner.children.find((child) => (child as { type?: string })?.type === 'input') as {
      props: { onChange: (event: unknown) => void };
    };
    box.props.onChange({ target: { checked: true } });
    h.render();
  };
  tick('local case folders');
  h.render();
  tick('never confirmed');
  h.click('Clear records and delete folders');
  assert.deepEqual(h.requests[3].args, { cases: ['/case'], unconfirmed: true });
  h.requests[3].resolve({ removed: 1, kept: 0, deleted: ['/case'], failed: [] });
  await h.settle();
  assert.equal(h.requests[4].command, 'history_list');
  assert.equal(h.nodes().filter((n) => n.type === 'Dialog').length, 0);
});

test('a project card opens its timeline, batching stations, and a case opens its record', async () => {
  const h = harness();
  const member = (id: string, site: string, action: string, status: string, time: number) => ({
    ...row(id),
    case_path: `/cache/222/${site}`,
    status,
    time_ms: time,
    started_ms: time,
    detail: { action },
  });
  const members = [
    member('r1', 'AT-Neu', 'remote-run', 'succeeded', 5_000),
    member('r2', 'AU-Cpr', 'remote-run', 'failed', 4_000),
    member('n1', 'AT-Neu', 'new', 'succeeded', 1_000),
    {
      ...member('i1', 'AU-Cpr', 'remote-run', 'needs_reconciliation', 4_500),
      phase: 'imported',
      needs_reconciliation: true,
    },
  ];
  const project = {
    ...row('r1'),
    kind: 'project',
    case_path: null,
    status: 'completed_with_failures',
    detail: { project_path: '/cache/222', subject: { kind: 'project', name: '222' } },
    project_group: { path: '/cache/222', name: '222', members },
  };
  h.requests[0].resolve({ items: [project], total: 1 });
  await h.settle();
  const text = (n: unknown): string =>
    Array.isArray(n)
      ? n.map(text).join('')
      : n && typeof n === 'object'
        ? text((n as { children: unknown[] }).children)
        : n == null || n === false
          ? ''
          : String(n);
  const page = text(h.nodes());
  assert.ok(page.includes('222'));
  assert.ok(page.includes('2 cases'));
  assert.ok(page.includes('1 succeeded · 1 failed'), 'latest per case, imported left out');
  h.openCard();
  await h.settle();
  assert.equal(h.requests.length, 1, 'opening a project loads no single record');
  const rows = h.nodes().filter((n) => n.type === 'details');
  assert.equal(rows.length, 2, 'two remote runs share a row; creation has its own');
  assert.ok(text(rows[0]).includes('2 cases'));
  h.click('AU-Cpr · Failed');
  assert.equal(h.requests[1].command, 'history_detail');
  assert.deepEqual(h.requests[1].args, { id: 'r2' });
});
