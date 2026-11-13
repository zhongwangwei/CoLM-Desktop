import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';

type Node = { type: unknown; props: Record<string, any>; children: Node[] }; // eslint-disable-line @typescript-eslint/no-explicit-any
const source = readFileSync(new URL('./SshCredentialDialog.tsx', import.meta.url), 'utf8');
const code = ts.transpileModule(
  source.replace(/^import\b[\s\S]*?;\n/gm, '').replace('export default function', 'function'),
  {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  },
).outputText;
const request = (id: string) => ({
  request_id: id,
  operation_host: 'chosen-host',
  prompt: `Password for ${id}`,
  kind: 'password',
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
function harness() {
  const slots: unknown[] = [];
  let cursor = 0;
  let effects: (() => void)[] = [];
  let cleanups: (() => void)[] = [];
  let contract: Record<string, unknown> = {};
  let enabled = false;
  let changed!: (snapshot: unknown) => void;
  let stopped = false;
  let fail = false;
  const initial = deferred<unknown>();
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const deps = {
    React: {
      createElement: (type: unknown, props: Record<string, unknown>, ...children: Node[]) => ({
        type,
        props: props ?? {},
        children,
      }),
    },
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [
        slots[index],
        (value: unknown) => {
          slots[index] = typeof value === 'function' ? value(slots[index]) : value;
        },
      ];
    },
    useRef: (value: unknown) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { current: value };
      return slots[index];
    },
    useEffect: (effect: () => void, dependencies: unknown[]) => {
      const index = cursor++;
      if (!slots[index]) {
        slots[index] = dependencies;
        effects.push(effect);
      }
    },
    useApp: () => ({ state: { language: 'en' }, notify: () => {} }),
    usePageContract: (next: Record<string, unknown>, active: boolean) => {
      contract = next;
      enabled = active;
    },
    hasBackend: () => true,
    listen: async (_event: string, callback: typeof changed) => {
      changed = callback;
      return () => {
        stopped = true;
      };
    },
    invokeJson: async (command: string) => {
      calls.push({ command });
      return initial.promise;
    },
    invoke: async (command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      if (fail) throw new Error('private backend error');
    },
    Button: 'Button',
    Dialog: 'Dialog',
    Input: 'Input',
    Notice: 'Notice',
  };
  const components = new Function(
    ...Object.keys(deps),
    `${code}\nreturn { SshCredentialDialog, CredentialPrompt };`,
  )(...Object.values(deps));
  const render = (name = 'SshCredentialDialog', props?: unknown): Node | null => {
    cursor = 0;
    const tree = components[name](props);
    for (const effect of effects) {
      const cleanup = effect();
      if (typeof cleanup === 'function') cleanups.push(cleanup);
    }
    effects = [];
    return tree;
  };
  return {
    render,
    initial,
    calls,
    emit: (revision: number, ids: string[]) => changed({ revision, requests: ids.map(request) }),
    contract: () => ({ contract, enabled }),
    fail: (value: boolean) => {
      fail = value;
    },
    unmount: () => {
      cleanups.forEach((fn) => fn());
      cleanups = [];
      slots.length = 0;
    },
    stopped: () => stopped,
  };
}
function find(node: Node | null, type: string): Node {
  if (node?.type === type) return node;
  for (const child of node?.children.flat(Infinity) ?? []) {
    if (child && typeof child === 'object') {
      try {
        return find(child, type);
      } catch {
        /* try sibling */
      }
    }
  }
  throw new Error(`Missing ${type}`);
}

test('subscribes before snapshot and ignores late initial snapshots and older revisions', async () => {
  const h = harness();
  h.render();
  h.emit(2, ['jump', 'target']);
  await tick();
  h.initial.resolve({ revision: 0, requests: [] });
  await tick();
  assert.equal(h.render()?.props.request.request_id, 'jump');
  h.emit(1, []);
  assert.equal(h.render()?.props.request.request_id, 'jump');
  h.render()!.props.onAnswered();
  assert.equal(h.render()?.props.request.request_id, 'target');
  h.emit(3, []);
  assert.equal(h.render(), null);
  h.unmount();
  assert.equal(h.stopped(), true);
});

test('revision zero initializes and assistant contract contains no sensitive fields or prompt', async () => {
  const h = harness();
  h.render();
  await tick();
  h.initial.resolve({ revision: 0, requests: [request('private-prompt')] });
  await tick();
  assert.equal(h.render()?.props.request.request_id, 'private-prompt');
  assert.deepEqual(h.contract(), {
    enabled: true,
    contract: {
      title: 'SSH authentication',
      step: 'ssh-auth',
      modal: true,
      fields: [],
      actions: [],
    },
  });
  assert.doesNotMatch(source, /ActionButton|closeAction|textInput/);
});

test('submit clears secret; failed submit keeps request retryable; cancel sends null', async () => {
  const h = harness();
  let answered = 0;
  const props = {
    request: request('jump'),
    en: true,
    onAnswered: () => {
      answered++;
    },
  };
  const render = () => h.render('CredentialPrompt', props);
  find(render(), 'Input').props.onChange({ target: { value: 'secret' } });
  h.fail(true);
  find(render(), 'form').props.onSubmit({ preventDefault() {} });
  await tick();
  assert.equal(answered, 0);
  assert.equal(find(render(), 'Input').props.value, '');
  assert.equal(find(render(), 'Input').props.disabled, false);
  assert.ok(find(render(), 'Notice'));
  assert.deepEqual(h.calls[0].args, { requestId: 'jump', value: 'secret' });
  h.fail(false);
  find(render(), 'Button').props.onClick();
  await tick();
  assert.deepEqual(h.calls[1].args, { requestId: 'jump', value: null });
  assert.equal(answered, 1);
});

test('new keyed requests and unmount discard secret drafts', () => {
  const h = harness();
  const props = { request: request('jump'), en: true, onAnswered() {} };
  find(h.render('CredentialPrompt', props), 'Input').props.onChange({
    target: { value: 'secret' },
  });
  h.unmount();
  assert.equal(
    find(h.render('CredentialPrompt', { ...props, request: request('target') }), 'Input').props
      .value,
    '',
  );
  assert.match(source, /key=\{request.request_id\}/);
});
