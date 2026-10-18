import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { newServer } from '../features/resources/resource';
import type { Server } from '../lib/types';

type Node = {
  type: string;
  props: {
    disabled?: boolean;
    placeholder?: string;
    onClick: () => void;
    onChange: (event: { target: { value: string } }) => void;
    onSubmit: (event: { preventDefault: () => void }) => void;
  };
  children: unknown[];
};
function harness(directory = false, server?: Server) {
  const source = readFileSync(new URL('./RemotePathBrowser.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const slots: unknown[] = [];
  const effects: { deps: unknown[]; cleanup?: () => void }[] = [];
  let cursor = 0,
    effectCursor = 0;
  const pending: (() => void)[] = [];
  const requests: {
    command: string;
    args: Record<string, unknown>;
    resolve: (value: unknown) => void;
    reject: (error: unknown) => void;
  }[] = [];
  const selected: string[] = [];
  const env = {
    React: {
      createElement: (type: string, props: Node['props'], ...children: unknown[]) => ({
        type,
        props: props ?? {},
        children,
      }),
      Fragment: 'fragment',
    },
    Button: 'button',
    Dialog: 'dialog',
    Field: 'field',
    Input: 'input',
    Notice: 'notice',
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (value: unknown) => {
          slots[i] = value;
        },
      ];
    },
    useEffect: (fn: () => () => void, deps: unknown[]) => {
      const i = effectCursor++;
      if (!effects[i] || deps.some((value, j) => value !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          effects[i].cleanup = fn();
        });
      }
    },
    invoke: (command: string, args: Record<string, unknown>) =>
      new Promise((resolve, reject) => requests.push({ command, args, resolve, reject })),
  };
  const Browser = new Function(...Object.keys(env), `${code}; return Browser;`)(
    ...Object.values(env),
  );
  let tree: Node;
  const props = {
    host: 'server',
    server,
    directory,
    en: true,
    onSelect: (path: string) => selected.push(path),
    onClose: () => {},
  };
  function render() {
    cursor = effectCursor = 0;
    tree = Browser(props);
    while (pending.length) pending.shift()!();
    return tree;
  }
  function nodes(node: unknown): Node[] {
    return Array.isArray(node)
      ? node.flatMap((value) => nodes(value))
      : node && typeof node === 'object'
        ? [node as Node, ...(node as Node).children.flatMap((value: unknown) => nodes(value))]
        : [];
  }
  const text = (node: unknown): string =>
    Array.isArray(node)
      ? node.map(text).join('')
      : node && typeof node === 'object'
        ? text((node as Node).children)
        : node == null || node === false
          ? ''
          : String(node);
  function click(label: string) {
    const node = nodes(tree).find((value) => value.type === 'button' && text(value) === label);
    assert.ok(node, label);
    assert.ok(!node.props.disabled);
    node.props.onClick();
    render();
  }
  async function settle() {
    await new Promise((resolve) => setImmediate(resolve));
    render();
  }
  function go(path: string) {
    nodes(tree)
      .find((value) => value.type === 'input')!
      .props.onChange({ target: { value: path } });
    render();
    nodes(tree)
      .find((value) => value.type === 'form')!
      .props.onSubmit({ preventDefault() {} });
    render();
  }
  function createFolder(name: string) {
    if (
      !nodes(tree).some((node) => node.type === 'input' && node.props.placeholder === 'Folder name')
    )
      click('New folder');
    nodes(tree)
      .find((node) => node.type === 'input' && node.props.placeholder === 'Folder name')!
      .props.onChange({ target: { value: name } });
    render();
    nodes(tree)
      .filter((node) => node.type === 'form')[1]
      .props.onSubmit({ preventDefault() {} });
    render();
  }
  render();
  return {
    requests,
    selected,
    click,
    settle,
    go,
    createFolder,
    text: () => text(tree),
    render,
    props,
    unmount: () => effects.forEach((effect) => effect.cleanup?.()),
  };
}

test('folder browsing carries an unsaved connection without requiring its work directory', () => {
  const server = { ...newServer(), host: 'server', username: 'researcher' };
  const view = harness(true, server);
  assert.deepEqual(view.requests[0].args, { host: 'server', path: null, server });
  assert.equal(server.root, '');
});
const listing = {
  path: '/data',
  parent: '/',
  entries: [
    { name: 'sites', path: '/data/sites', directory: true },
    { name: 'site.nc', path: '/data/site.nc', directory: false },
  ],
};

test('new folders use the displayed server directory and open the created folder', async () => {
  const server = { ...newServer(), host: 'server' };
  const view = harness(true, server);
  view.requests[0].resolve(listing);
  await view.settle();
  view.createFolder('project');
  assert.equal(view.requests[1].command, 'remote_mkdir');
  assert.deepEqual(view.requests[1].args, {
    host: 'server',
    path: '/data',
    name: 'project',
    server,
  });
  view.requests[1].resolve({ path: '/data/project' });
  await view.settle();
  assert.equal(view.requests[2].command, 'remote_browse');
  assert.equal(view.requests[2].args.path, '/data/project');
  view.requests[2].resolve({ path: '/data/project', parent: '/data', entries: [] });
  await view.settle();
  view.click('Select this folder');
  assert.deepEqual(view.selected, ['/data/project']);
});

test('folder creation errors retain the parent and allow a corrected name', async () => {
  const view = harness(true);
  view.requests[0].resolve(listing);
  await view.settle();
  view.createFolder('exists');
  view.requests[1].reject(new Error('Folder already exists'));
  await view.settle();
  assert.match(view.text(), /Folder already exists/);
  view.createFolder('new-name');
  assert.equal(view.requests[2].args.path, '/data');
  assert.equal(view.requests[2].args.name, 'new-name');
  view.unmount();
  view.requests[2].resolve({ path: '/data/new-name' });
  await view.settle();
  assert.equal(view.requests.length, 3, 'closed dialogs cannot navigate after creation');
});

test('remote file browser loads, navigates folders and selects files', async () => {
  const view = harness();
  assert.deepEqual(view.requests[0].args, { host: 'server', path: null });
  assert.match(view.text(), /Loading/);
  view.requests[0].resolve(listing);
  await view.settle();
  view.click('site.nc');
  assert.deepEqual(view.selected, ['/data/site.nc']);
  view.click('sites');
  assert.equal(view.requests[1].args.path, '/data/sites');
  assert.ok(!view.text().includes('site.nc'));
});

test('folder browser hides files and selects the resolved current directory', async () => {
  const view = harness(true);
  view.requests[0].resolve(listing);
  await view.settle();
  assert.ok(!view.text().includes('site.nc'));
  view.click('Select this folder');
  assert.deepEqual(view.selected, ['/data']);
  view.click('Up');
  assert.equal(view.requests[1].args.path, '/');
});

test('obsolete responses, host changes and unmount cannot replace the active listing', async () => {
  const view = harness();
  view.go('/next');
  view.requests[1].resolve({ ...listing, path: '/next' });
  await view.settle();
  view.requests[0].resolve({ ...listing, path: '/old' });
  await view.settle();
  assert.ok(!view.text().includes('/old'));
  view.go('/pending');
  view.props.host = 'another';
  view.render();
  view.requests[2].reject('old host error');
  await view.settle();
  assert.ok(!view.text().includes('old host error'));
  view.unmount();
  view.requests[3].reject('unmounted error');
  await view.settle();
  assert.ok(!view.text().includes('unmounted error'));
});

test('browse errors clear obsolete selections and allow retry', async () => {
  const view = harness(true);
  view.requests[0].resolve(listing);
  await view.settle();
  view.go('/missing');
  view.requests[1].reject('Permission denied');
  await view.settle();
  assert.match(view.text(), /Permission denied/);
  assert.ok(!view.text().includes('Select this folder'));
  view.go('/missing');
  view.requests[2].resolve({ ...listing, truncated: true });
  await view.settle();
  assert.ok(!view.text().includes('Permission denied'));
  assert.match(view.text(), /more entries/);
});
