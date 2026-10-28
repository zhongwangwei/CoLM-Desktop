import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as resource from './resource';
import { remoteRunPermissions, reconcileRemoteRun } from '../runs/run-model';
import { ActionButtonStub } from '../../lib/rendered-tree';
import type { UiAction, UiField } from '../../lib/guide';

type Contract = { fields: UiField[]; actions: UiAction[]; details: unknown };
type Rendered = {
  id: string;
  label: string;
  kind: 'click' | 'commit';
  enabled: boolean;
  run: () => unknown;
};
// eslint-disable-next-line @typescript-eslint/no-explicit-any
type Node = { type: unknown; props: Record<string, any>; children?: unknown[] };
function harness(
  records: Record<string, unknown> = {},
  hosts = ['7920land', '7920mp'],
  failures: { config?: boolean; history?: string; save?: boolean } = {},
  simulationReturn?: { route: 'new-case' | 'case'; step: 1; resourceHost?: string },
  scan?: {
    before: (command: string, args?: Record<string, unknown>) => Promise<void>;
    studies?: Record<string, { dir: string }[]>;
  },
  // 模拟 remote_config 里已经保存的本机默认路径（GUI-13 的基线来源）。
  local?: {
    project_dir?: string;
    rawdata_dir?: string;
    runtime_dir?: string;
    forcing_dir?: string;
    kind_paths?: Record<string, Record<string, string>>;
  },
) {
  const source = readFileSync(new URL('./ResourcesPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const slots: unknown[] = [];
  let cursor = 0,
    effected = false,
    contract: Contract,
    guard: () => string | null;
  let cleanup: (() => void) | undefined;
  const routes: string[] = [];
  const calls: { command: string; args: Record<string, unknown> | undefined }[] = [];
  const servers = hosts.map((host) => ({ ...resource.newServer(), host, root: '/data/colm' }));
  let failProbe = false;
  let schedulers: unknown = ['slurm'];
  let probePartitions: resource.Partition[] = [
    {
      name: 'first',
      default: true,
      exclusive: false,
      nodes: 7,
      cores_min: 48,
      cores_max: 48,
      memory_gb_min: 312,
      memory_gb_max: 375,
      max_nodes: 3,
      max_time: '7-00:00:00',
      whole_node_memory: true,
    },
    {
      name: 'second',
      default: false,
      exclusive: false,
      nodes: 7,
      cores_min: 64,
      cores_max: 80,
      memory_gb_min: 502,
      memory_gb_max: 1006,
      max_nodes: 1,
      max_time: '7-00:00:00',
      whole_node_memory: true,
    },
  ];
  let unverifiedQsub = false;
  let pendingProbe: Promise<void> | undefined;
  const menus: { items: { disabled?: boolean; onSelect: () => void }[] }[] = [];
  const liveLoads: { host: string; partition: string }[] = [];
  let browser: {
    server: resource.Server;
    directory: boolean;
    onSelect: (path: string) => void;
    onClose: () => void;
  } | null = null;
  const ipc = async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    await scan?.before(command, args);
    switch (command) {
      case 'remote_config':
        if (failures.config) throw new Error('Unreadable server configuration');
        return { servers, ...(local ? { local } : {}) };
      case 'remote_save_config':
        if (failures.save) throw new Error('Cannot save connection settings');
        return null;
      case 'remote_forget_kind_path':
        return true;
      case 'remote_ssh_hosts':
        return ['7920land', '7920mp'];
      case 'remote_cases':
        return [];
      case 'remote_case_save':
        return {
          id: 'case-1',
          name: String(args?.name),
          server: args?.server,
          saved_at: 1,
        };
      case 'remote_case_delete':
        return true;
      case 'remote_job_record':
        return records[String(args?.case)] ?? null;
      case 'case_runs':
        if (failures.history === args?.case) throw new Error('Case directory was deleted');
        return { studies: scan?.studies?.[String(args?.case)] ?? [] };
      case 'study_status':
        return { remote: { host: hosts[0], job: 'study-job' } };
      case 'remote_probe':
        await pendingProbe;
        if (failProbe) throw new Error('SSH unavailable');
        return {
          hostname: 'remote',
          schedulers,
          qsub_unverified: unverifiedQsub,
          root_writable: true,
          maps: [],
          partitions: probePartitions,
          ...(!(args?.server as resource.Server)?.root
            ? { configuration_error: '请填写服务器工作目录' }
            : {}),
        };
      case 'remote_engines':
        return { engines: [{ id: 'snapshot', ready: true }] };
      case 'pick_file':
        return '/private/id_ed25519';
      case 'remote_status':
        return { record: { host: '7920land', job: 'job1' }, status: { state: 'running' } };
      default:
        return null;
    }
  };
  let tree: unknown;
  const environment = {
    ...resource,
    remoteRunPermissions,
    reconcileRemoteRun,
    React: {
      createElement: (
        type: unknown,
        props: Record<string, unknown> | null,
        ...children: unknown[]
      ) => {
        if (type === 'remote-browser') browser = props as typeof browser;
        if (type === 'context-menu') menus.push(props as (typeof menus)[number]);
        if (type === 'live-server-load') liveLoads.push(props as (typeof liveLoads)[number]);
        return { type, props: { ...(props ?? {}), children } };
      },
      Fragment: 'Fragment',
    },
    RemotePathBrowser: 'remote-browser',
    LiveServerLoad: 'live-server-load',
    ContextMenu: 'context-menu',
    Dialog: 'Dialog',
    Advanced: 'Advanced',
    Button: 'Button',
    Card: 'Card',
    Field: 'Field',
    Input: 'Input',
    Notice: 'Notice',
    Select: 'Select',
    NavIcon: 'NavIcon',
    ActionButton: ActionButtonStub,
    useApp: () => ({
      state: {
        language: 'zh',
        simulationReturn,
        cases: Object.keys(records).map((dir) => ({ dir })),
        running: {},
        kernels: [],
        version: null,
      },
      update: () => {},
      navigate: (route: string) => {
        routes.push(route);
        return true;
      },
    }),
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (next: unknown) => {
          slots[key] = typeof next === 'function' ? next(slots[key]) : next;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = { current: initial };
      return slots[key];
    },
    useCallback: (fn: unknown) => fn,
    useEffect: (fn: () => (() => void) | void) => {
      if (!effected) {
        effected = true;
        cleanup = fn() || undefined;
      }
    },
    useNavigationGuard: (fn: () => string | null) => {
      guard = fn;
    },
    // 动作由按钮自己登记（第 737 轮）：从渲染出的元素树里收；打开的对话框挡住页面，只算对话框里的。
    usePageContract: (value: Contract) => {
      contract = {
        ...value,
        get actions() {
          return visibleActions();
        },
      };
    },
    invoke: ipc,
    invokeJson: ipc,
  };
  const Page = new Function(...Object.keys(environment), `${code};return ResourcesPage;`)(
    ...Object.values(environment),
  );
  const walk = (node: unknown): Node[] =>
    !node || typeof node !== 'object'
      ? []
      : Array.isArray(node)
        ? node.flatMap(walk)
        : [node as Node, ...walk((node as Node).props?.children)];
  const text = (node: unknown): string =>
    Array.isArray(node)
      ? node.map(text).join('')
      : node == null || node === false
        ? ''
        : typeof node === 'object'
          ? text((node as Node).props?.children)
          : String(node);
  function collect(node: unknown): Rendered[] {
    if (!node || typeof node !== 'object') return [];
    if (Array.isArray(node)) return node.flatMap(collect);
    const element = node as Node,
      props = element.props ?? {};
    if (element.type === 'Dialog') return props.open ? collect(props.children) : [];
    if (element.type === 'Advanced') {
      const own: Rendered[] =
        typeof props.action === 'string'
          ? [
              {
                id: String(props.action),
                label: String(props.actionLabel ?? ''),
                kind: 'click',
                enabled: true,
                run: () => (props.onOpenChange as (open: boolean) => void)(!props.open),
              },
            ]
          : [];
      // 折叠着的可折叠区不渲染内容，里面的按钮也就不存在。
      return [...own, ...(props.open ? collect(props.children) : [])];
    }
    if (element.type === ActionButtonStub)
      return [
        {
          id: String(props.action),
          label: String(props.label),
          kind: (props.kind as Rendered['kind']) ?? 'click',
          enabled: !props.disabled,
          run: props.run as () => unknown,
        },
      ];
    return collect(props.children);
  }
  function findOpenDialog(node: unknown): Node | null {
    if (!node || typeof node !== 'object') return null;
    if (Array.isArray(node)) {
      for (const item of node) {
        const found = findOpenDialog(item);
        if (found) return found;
      }
      return null;
    }
    const element = node as Node;
    if (element.type === 'Dialog' && element.props?.open) return element;
    return findOpenDialog(element.props?.children);
  }
  function visibleActions(): UiAction[] {
    const dialog = findOpenDialog(tree),
      found = collect(dialog ? dialog.props.children : tree);
    if (dialog && typeof dialog.props.closeAction === 'string')
      found.unshift({
        id: dialog.props.closeAction,
        label: '关闭',
        kind: 'click',
        enabled: true,
        run: () => (dialog.props.onOpenChange as (open: boolean) => void)(false),
      });
    return found;
  }
  function render() {
    cursor = 0;
    menus.length = 0;
    tree = Page();
    return contract;
  }
  async function ready() {
    render();
    await new Promise((resolve) => setImmediate(resolve));
    return render();
  }
  async function action(id: string) {
    const item = visibleActions().find((action) => action.id === id);
    assert.ok(item, `visible action ${id}`);
    assert.notEqual(item.enabled, false, `${id} is enabled`);
    await item.run();
    return render();
  }
  function fill(id: string, value: string) {
    const item = contract.fields.find((field) => field.id === id);
    assert.ok(item, `visible field ${id}`);
    item.set(value);
    return render();
  }
  /** 普通按钮（删除、清除这类 D 级动作）：不登记给助手，但界面上能按。 */
  function button(label: string) {
    const item = walk(tree).find((node) => node.type === 'Button' && text(node) === label);
    assert.ok(item, `button ${label}`);
    return item;
  }
  async function press(label: string) {
    const item = button(label);
    assert.notEqual(item.props.disabled, true, `${label} is enabled`);
    (item.props.onClick as () => void)();
    await new Promise((resolve) => setImmediate(resolve));
    return render();
  }
  return {
    ready,
    unmount: () => cleanup?.(),
    restartEffect: () => {
      cleanup?.();
      effected = false;
      return render();
    },
    action,
    fill,
    press,
    button,
    render,
    calls,
    routes,
    servers,
    guard: () => guard(),
    contract: () => contract,
    visible: visibleActions,
    nodes: () => walk(tree),
    notices: () =>
      walk(tree)
        .filter((node) => node.type === 'Notice')
        .map((node) => text(node)),
    text,
    failProbe: () => {
      failProbe = true;
    },
    schedulers: (value: unknown) => {
      schedulers = value;
    },
    partitions: (value: resource.Partition[]) => {
      probePartitions = value;
    },
    unverifiedQsub: () => {
      unverifiedQsub = true;
    },
    pauseProbe: () => {
      let release!: () => void;
      pendingProbe = new Promise<void>((resolve) => {
        release = resolve;
      });
      return release;
    },
    browser: () => browser,
    menus: () => menus,
    liveLoads: () => liveLoads,
  };
}

test('resource selection is page-local and dirty drafts block switching and navigation until discarded', async () => {
  const page = harness();
  let contract = await page.ready();
  assert.equal((contract.details as { selected: string }).selected, 'server:7920land');
  assert.equal(page.guard(), null);
  assert.ok(
    !contract.actions.some(
      (action) => action.id === 'resource-save' || action.id === 'resource-discard',
    ),
    'unchanged settings do not expose hidden save controls',
  );
  assert.deepEqual(
    contract.actions
      .filter((action) => action.id.startsWith('resource-select-'))
      .map((action) => action.id),
    ['resource-select-local', 'resource-select-server-7920land', 'resource-select-server-7920mp'],
  );
  page.fill('resource-username', 'changed');
  assert.match(page.guard()!, /保存/);
  assert.ok(page.contract().actions.some((action) => action.id === 'resource-save'));
  assert.ok(page.contract().actions.some((action) => action.id === 'resource-discard'));
  contract = page.contract();
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-select-server-7920mp')?.enabled,
    false,
  );
  assert.equal(contract.actions.find((action) => action.id === 'resource-new')?.enabled, false);
  assert.equal((contract.details as { selected: string }).selected, 'server:7920land');
  assert.equal(contract.fields.find((field) => field.id === 'resource-username')?.value, 'changed');
  await page.action('resource-discard');
  assert.equal(page.guard(), null);
  contract = await page.action('resource-select-server-7920mp');
  assert.equal((contract.details as { selected: string }).selected, 'server:7920mp');
  assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
});

test('remembered paths by simulation type are listed per server, survive saving and can be forgotten', async () => {
  const page = harness();
  Object.assign(page.servers[0], {
    kind_paths: {
      watershed: { shapefile: '/basins/heihe.shp' },
      site: { forcingdir: '/server/Forcing', sitedir: '/server/Sitedata' },
    },
  });
  await page.ready();
  // "清除"忘记记住的路径属于 D 级：界面上有按钮，助手看不到（第 737 轮）。
  const forget = () =>
    page.nodes().filter((node) => node.type === 'Button' && page.text(node) === '清除');
  assert.equal(forget().length, 3);
  assert.ok(!page.contract().actions.some((action) => action.id.includes('forget')));
  page.fill('resource-username', 'changed');
  await page.action('resource-save');
  assert.equal(forget().length, 3, 'saving connection settings keeps the remembered paths');
  await page.press('清除');
  assert.deepEqual(page.calls.find((call) => call.command === 'remote_forget_kind_path')?.args, {
    host: '7920land',
    kind: 'site',
    key: 'sitedir',
  });
  assert.equal(forget().length, 2);
  await page.action('resource-select-server-7920mp');
  assert.equal(forget().length, 0);
});

test('scheduler detection is visible beside advanced settings and applies one candidate only to the draft', async () => {
  for (const scheduler of ['slurm', 'pbs', 'lsf']) {
    const page = harness();
    let contract = await page.ready();
    assert.ok(!contract.actions.some((action) => action.id === 'resource-detect-scheduler'));
    await page.action('resource-check');
    await page.action('resource-advanced-toggle');
    page.schedulers([scheduler]);
    contract = await page.action('resource-detect-scheduler');
    assert.equal(
      contract.fields.find((field) => field.id === 'resource-scheduler')?.value,
      scheduler,
    );
    assert.deepEqual((contract.details as { schedulerDetection: unknown }).schedulerDetection, {
      configuration: JSON.stringify({ ...page.servers[0], scheduler }),
      schedulers: [scheduler],
      unverifiedQsub: false,
    });
    assert.equal(
      (contract.details as { check: { ssh: { state: string } } }).check.ssh.state,
      'pending',
    );
    assert.ok(contract.actions.some((action) => action.id === 'resource-save'));
    assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
    await page.action('resource-discard');
    contract = page.contract();
    assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'auto');
    assert.equal((contract.details as { schedulerDetection: unknown }).schedulerDetection, null);
  }
});

test('a checked Slurm server offers its partitions with node sizes and fills cores per node', async () => {
  const page = harness();
  await page.ready();
  let contract = await page.action('resource-advanced-toggle');
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-partition')?.type,
    'text',
    'typed by hand until the server is checked',
  );
  await page.action('resource-check');
  contract = page.contract();
  const partition = contract.fields.find((field) => field.id === 'resource-partition')!;
  assert.equal(partition.type, 'choice');
  assert.deepEqual(
    partition.options?.map((option) => option.label),
    [
      '服务器默认（first）',
      'first（默认） · 7 个节点 · 每节点 48 核 / 312–375 GB · 单作业最多 3 节点 · 7 天',
      'second · 7 个节点 · 每节点 64–80 核 / 502–1006 GB · 单作业最多 1 节点 · 7 天',
    ],
  );
  page.fill('resource-partition', 'second');
  contract = page.contract();
  assert.equal(contract.fields.find((field) => field.id === 'resource-partition')?.value, 'second');
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-cores_per_node')?.value,
    '64',
  );
  // A value the user already set is kept.
  page.fill('resource-partition', 'first');
  contract = page.contract();
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-cores_per_node')?.value,
    '64',
  );
});

test('a cluster preset fills scheduling settings without touching address, account or work root', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-advanced-toggle');
  page.fill('resource-preset', 'land');
  const contract = page.contract();
  const value = (id: string) => contract.fields.find((field) => field.id === id)?.value;
  assert.deepEqual(
    ['resource-scheduler', 'resource-partition', 'resource-cores_per_node'].map(value),
    ['slurm', 'first', '48'],
  );
  assert.equal(value('resource-host'), '7920land', 'the address the user gave stays');
  assert.equal(value('resource-root'), '/data/colm', 'so does the work root');
  assert.equal(value('resource-rawdata_dir'), '/shr03/CoLMrawdata');
  assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
});

test('a recognised cluster preset is applied from its notice and only edits the draft', async () => {
  const page = harness();
  await page.ready();
  assert.ok(!page.contract().actions.some((action) => action.id === 'resource-apply-preset'));
  page.partitions([
    {
      name: 'first',
      default: true,
      exclusive: false,
      nodes: 7,
      cores_min: 48,
      cores_max: 48,
      memory_gb_min: 312,
      memory_gb_max: 375,
      max_nodes: 3,
      max_time: '7-00:00:00',
      whole_node_memory: true,
    },
    {
      name: 'second',
      default: false,
      exclusive: false,
      nodes: 7,
      cores_min: 64,
      cores_max: 80,
      memory_gb_min: 502,
      memory_gb_max: 1006,
      max_nodes: 1,
      max_time: '7-00:00:00',
      whole_node_memory: true,
    },
    {
      name: 'largemem',
      default: false,
      exclusive: false,
      nodes: 1,
      cores_min: 48,
      cores_max: 48,
      memory_gb_min: 1500,
      memory_gb_max: 1500,
      max_nodes: 1,
      max_time: '7-00:00:00',
      whole_node_memory: true,
    },
  ]);
  await page.action('resource-advanced-toggle');
  await page.action('resource-check');
  const preset = page.contract().actions.find((action) => action.id === 'resource-apply-preset');
  assert.ok(preset, 'the recognised cluster notice registers its apply button');
  assert.equal(preset.label, '套用预设');
  const contract = await page.action('resource-apply-preset');
  const value = (id: string) => contract.fields.find((field) => field.id === id)?.value;
  assert.deepEqual(
    ['resource-scheduler', 'resource-partition', 'resource-cores_per_node'].map(value),
    ['slurm', 'first', '48'],
  );
  assert.equal(value('resource-host'), '7920land');
  assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
});

test('destructive resource actions stay on the interface, while removing a draft mapping row is registered', async () => {
  const page = harness();
  Object.assign(page.servers[0], {
    kind_paths: { site: { sitedir: '/server/Sitedata' } },
  });
  await page.ready();
  // 界面上真正删除/清理/忘记的动作不登记给助手（D 级，第 737 轮）。
  const forbidden = /^resource-(delete|forget|clean|recover|register-password)/;
  assert.ok(!page.contract().actions.some((action) => forbidden.test(action.id)));
  assert.ok(page.button('清除'), 'the forget button is still on the interface');
  // 删除连接对话框：助手只看得到"关闭"。
  page.menus()[0].items[0].onSelect();
  page.render();
  assert.deepEqual(
    page.contract().actions.map((action) => action.id),
    ['resource-delete-close'],
  );
  await page.action('resource-delete-close');
  // 移除映射行只改内存草稿，保留原 id 与 kind=click。
  await page.action('resource-advanced-toggle');
  await page.action('resource-directories-toggle');
  let contract = await page.action('resource-add-map');
  assert.ok(contract.actions.some((action) => action.id === 'resource-remove-map-0'));
  assert.ok(!contract.actions.some((action) => forbidden.test(action.id)));
  contract = await page.action('resource-remove-map-0');
  assert.ok(!contract.actions.some((action) => action.id === 'resource-remove-map-0'));
  // 维护区里清理快照、核对租约只在界面上。
  contract = await page.action('resource-maintenance-toggle');
  assert.ok(!contract.actions.some((action) => forbidden.test(action.id)));
  assert.ok(page.button('清理快照'));
  assert.ok(page.button('核对并恢复租约'));
  // 同时渲染的按钮 id 唯一（列表项都带各自的 key）。
  const ids = contract.actions.map((action) => action.id);
  assert.equal(new Set(ids).size, ids.length, `duplicate action ids: ${ids}`);
  // 密码是本次会话的密钥，登记密码也不登记给助手。
  page.fill('resource-auth', 'password');
  assert.ok(!page.render().actions.some((action) => forbidden.test(action.id)));
  assert.ok(page.button('登记密码'));
});

test('ambiguous or absent schedulers retain the manual selection, without falling back to bare', async () => {
  for (const schedulers of [[], ['slurm', 'pbs']]) {
    const page = harness();
    await page.ready();
    await page.action('resource-advanced-toggle');
    page.fill('resource-scheduler', 'lsf');
    page.schedulers(schedulers);
    const contract = await page.action('resource-detect-scheduler');
    assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'lsf');
    assert.deepEqual(
      (contract.details as { schedulerDetection: { schedulers: string[] } }).schedulerDetection
        .schedulers,
      schedulers,
    );
    assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
  }
});

test('an unverified qsub is identified separately from a server without scheduler tools', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-advanced-toggle');
  page.schedulers([]);
  page.unverifiedQsub();
  const contract = await page.action('resource-detect-scheduler');
  assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'auto');
  assert.equal(
    (contract.details as { schedulerDetection: { unverifiedQsub: boolean } }).schedulerDetection
      .unverifiedQsub,
    true,
  );
});

test('detection works for an unsaved connection without a work root and rejects failed or malformed replies', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-new');
  await page.action('resource-advanced-toggle');
  assert.equal(
    page.contract().actions.find((action) => action.id === 'resource-detect-scheduler')?.enabled,
    false,
  );
  page.fill('resource-host', 'tianhe');
  let contract = await page.action('resource-detect-scheduler');
  assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'slurm');
  assert.equal((page.calls.at(-1)?.args?.server as resource.Server).root, '');
  for (const invalid of [undefined, null, 'pbs', ['unknown']]) {
    page.schedulers(invalid);
    await assert.rejects(page.action('resource-detect-scheduler'), /有效的调度器/);
    contract = page.render();
    assert.equal(
      contract.fields.find((field) => field.id === 'resource-scheduler')?.value,
      'slurm',
    );
    assert.equal((contract.details as { schedulerDetection: unknown }).schedulerDetection, null);
  }
  page.failProbe();
  await assert.rejects(page.action('resource-detect-scheduler'), /SSH unavailable/);
  contract = page.render();
  assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'slurm');
  assert.equal(page.calls.filter((call) => call.command === 'remote_save_config').length, 0);
});

test('late scheduler replies cannot overwrite another edited connection or bypass the busy guard', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-advanced-toggle');
  const release = page.pauseProbe();
  const pending = page.action('resource-detect-scheduler');
  let contract = page.render();
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-detect-scheduler')?.enabled,
    false,
  );
  assert.match(page.guard()!, /操作进行中/);
  page.fill('resource-host', 'changed-host');
  release();
  contract = await pending;
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-host')?.value,
    'changed-host',
  );
  assert.equal(contract.fields.find((field) => field.id === 'resource-scheduler')?.value, 'auto');
  assert.equal((contract.details as { schedulerDetection: unknown }).schedulerDetection, null);
});

test('resource context menu deletes only the confirmed connection and preserves another selection', async () => {
  const page = harness();
  await page.ready();
  assert.ok(!page.contract().actions.some((action) => action.id === 'resource-delete-confirm'));
  page.menus()[1].items[0].onSelect();
  page.render();
  assert.match(page.guard()!, /删除确认/);
  // 删除连接设置是 D 级：确认按钮不登记，助手只看到对话框的"关闭"。
  assert.deepEqual(
    page.contract().actions.map((action) => action.id),
    ['resource-delete-close'],
  );
  assert.equal(page.button('删除连接设置').props.disabled, false);
  const contract = await page.press('删除连接设置');
  assert.equal((contract.details as { selected: string }).selected, 'server:7920land');
  assert.equal(page.menus().length, 1);
  assert.deepEqual(page.calls.at(-1), {
    command: 'remote_save_config',
    args: { config: { servers: [page.servers[0]] } },
  });
  assert.equal(page.guard(), null);
});

test('connection delete failure preserves settings and dirty drafts disable its context menu', async () => {
  const page = harness({}, undefined, { save: true });
  await page.ready();
  page.menus()[0].items[0].onSelect();
  page.render();
  await page.press('删除连接设置');
  page.render();
  assert.equal(page.menus().length, 2);
  // 删除失败后对话框仍开着，助手依旧只看到"关闭"。
  assert.deepEqual(
    page.contract().actions.map((action) => action.id),
    ['resource-delete-close'],
  );
  page.fill('resource-username', 'edited');
  assert.ok(page.menus().every((menu) => menu.items[0].disabled));
  assert.equal(page.button('删除连接设置').props.disabled, true);
});

test('the open delete dialog blocks page actions and only offers its close button', async () => {
  const page = harness();
  await page.ready();
  page.menus()[0].items[0].onSelect();
  page.render();
  // 对话框打开时页面上的按钮被挡住，助手连"刷新"也按不到。
  await assert.rejects(page.action('resource-refresh'), /visible action resource-refresh/);
  assert.deepEqual(
    page.contract().actions.map((action) => action.id),
    ['resource-delete-close'],
  );
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
  const contract = await page.action('resource-delete-close');
  assert.equal(page.guard(), null);
  // 关闭后页面动作回来，删除仍然看不到。
  assert.ok(contract.actions.some((action) => action.id === 'resource-refresh'));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-delete-confirm'));
});

test('server work directory picker uses the unsaved host and only fills its draft', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-new');
  page.fill('resource-host', 'tms17l');
  await page.action('resource-browse-root');
  const browser = page.browser();
  assert.ok(browser);
  assert.equal(browser.server.host, 'tms17l');
  assert.equal(browser.server.root, '');
  assert.equal(browser.directory, true);
  assert.ok(page.guard());
  browser.onSelect('/server/projects');
  const contract = page.render();
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-root')?.value,
    '/server/projects',
  );
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
});

test('new server keeps optional local path mappings collapsed and empty', async () => {
  const page = harness();
  await page.ready();
  let contract = await page.action('resource-new');
  assert.ok(!contract.actions.some((action) => action.id === 'resource-directories-toggle'));
  contract = await page.action('resource-advanced-toggle');
  assert.ok(!contract.fields.some((field) => field.id.includes('map-')));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-add-map'));
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-directories-toggle')?.label,
    '本地路径映射（可选）',
  );
  page.fill('resource-host', 'tms17l');
  await page.action('resource-browse-root');
  assert.deepEqual(page.browser()?.server.maps, []);
  page.browser()?.onClose();
  contract = await page.action('resource-directories-toggle');
  assert.ok(contract.actions.some((action) => action.id === 'resource-add-map'));
  assert.ok(!contract.fields.some((field) => field.id.includes('map-')));
});

test('only visible configuration fields and selected maintenance actions enter the contract', async () => {
  const page = harness();
  let contract = await page.ready();
  assert.deepEqual(
    contract.fields.map((field) => field.id),
    [
      'resource-name',
      'resource-host',
      'resource-username',
      'resource-root',
      'resource-project_dir',
      'resource-rawdata_dir',
      'resource-runtime_dir',
      'resource-forcing_dir',
      'resource-preset',
      'resource-auth',
    ],
  );
  assert.ok(!contract.actions.some((action) => action.id === 'resource-build'));
  await page.action('resource-advanced-toggle');
  contract = await page.action('resource-directories-toggle');
  assert.ok(contract.fields.some((field) => field.id === 'resource-root'));
  await page.action('resource-add-map');
  page.fill('resource-map-0-local', '/data');
  contract = await page.action('resource-directories-toggle');
  assert.ok(!contract.fields.some((field) => field.id.includes('map-')));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-add-map'));
  await page.action('resource-discard');
  await page.action('resource-select-server-7920mp');
  contract = await page.action('resource-maintenance-toggle');
  assert.ok(contract.actions.some((action) => action.id === 'resource-build'));
  // 编译配置与集群预设各有自己的 id：字段 id 不重复，编译用的是这里填的配置。
  const ids = contract.fields.map((field) => field.id);
  assert.equal(new Set(ids).size, ids.length, `duplicate field ids: ${ids}`);
  page.fill('resource-build-preset', 'urban');
  await page.action('resource-build');
  assert.equal((page.calls.at(-1)!.args as { preset?: string }).preset, 'urban');
  await page.action('resource-kernels');
  assert.deepEqual(page.calls.at(-1), { command: 'remote_kernels', args: { host: '7920mp' } });
  assert.ok(!contract.actions.some((action) => /clean|delete|recover/.test(action.id)));
});

test('checks aggregate both readonly replies, edits revoke evidence, and native key selection edits only the draft', async () => {
  const page = harness();
  await page.ready();
  let contract = await page.action('resource-check');
  let check = (contract.details as { check: ReturnType<typeof resource.resourceCheckSummary> })
    .check;
  assert.equal(check.ssh.state, 'success');
  assert.equal(check.engine.label, '1 个可用快照');
  assert.ok(check.checkedAt);
  assert.deepEqual(
    page.calls
      .filter((call) => ['remote_probe', 'remote_engines'].includes(call.command))
      .map((call) => call.command),
    ['remote_probe', 'remote_engines'],
  );
  page.fill('resource-auth', 'key');
  await page.action('resource-pick-identity');
  contract = page.contract();
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-identity')?.value,
    '/private/id_ed25519',
  );
  check = (contract.details as { check: ReturnType<typeof resource.resourceCheckSummary> }).check;
  assert.equal(check.ssh.state, 'pending');
  await page.action('resource-discard');
  contract = page.contract();
  assert.equal(
    (contract.details as { check: { ssh: { state: string } } }).check.ssh.state,
    'pending',
    'discard does not resurrect revoked evidence',
  );
  page.failProbe();
  contract = await page.action('resource-check');
  check = (contract.details as { check: ReturnType<typeof resource.resourceCheckSummary> }).check;
  assert.equal(check.ssh.state, 'error');
  assert.equal(check.engine.state, 'pending');
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
});

test('an incomplete server draft still checks SSH without claiming the run settings failed to connect', async () => {
  const page = harness();
  await page.ready();
  await page.action('resource-new');
  page.fill('resource-host', 'tms17l');
  const contract = await page.action('resource-check');
  const check = (contract.details as { check: ReturnType<typeof resource.resourceCheckSummary> })
    .check;
  assert.equal(check.ssh.state, 'success');
  assert.equal(check.engine.state, 'pending');
  assert.equal(check.directory.state, 'pending');
  assert.equal(check.mapping.state, 'pending');
  assert.equal(page.calls.filter((call) => call.command === 'remote_engines').length, 0);
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
});

function callback(name: string, context: Record<string, unknown>) {
  const path = new URL('./ResourcesPage.tsx', import.meta.url),
    source = ts.createSourceFile(
      path.pathname,
      readFileSync(path, 'utf8'),
      ts.ScriptTarget.Latest,
      true,
      ts.ScriptKind.TSX,
    );
  let body = '';
  function visit(node: ts.Node) {
    if (ts.isFunctionDeclaration(node) && node.name?.text === name) body = node.getText(source);
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(body);
  const code = ts.transpileModule(`const fn=(${body});`, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  return new Function(...Object.keys(context), `${code};return fn;`)(...Object.values(context));
}
test('password registration stays native-session-only and revokes connection evidence', async () => {
  const editor = {
      ...resource.newServer(),
      host: '7920land',
      username: 'researcher',
      port: 22,
      auth: 'password',
    },
    calls: unknown[] = [];
  let password = 'session-secret',
    output: Record<string, unknown> = {
      'check:server:7920land': { old: true },
      'check:server:7920mp': { other: true },
    };
  const register = callback('registerPassword', {
    editor,
    password,
    selected: 'server:7920land',
    invoke: async (command: string, args: unknown) => calls.push({ command, args }),
    setPassword: (next: string) => {
      password = next;
    },
    setOutput: (update: (old: Record<string, unknown>) => Record<string, unknown>) => {
      output = update(output);
    },
    t: (zh: string) => zh,
  });
  await register();
  assert.equal(password, '');
  assert.deepEqual(output, { 'check:server:7920mp': { other: true } });
  assert.deepEqual(calls, [
    {
      command: 'remote_set_password',
      args: { host: '7920land', username: 'researcher', port: 22, password: 'session-secret' },
    },
  ]);
  assert.ok(!JSON.stringify(editor).includes('session-secret'));
  const page = harness();
  await page.ready();
  const contract = page.fill('resource-auth', 'password');
  assert.ok(!contract.fields.some((field) => field.id === 'resource-password'));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-register-password'));
});
test('busy operations block navigation even before React rerenders the busy state', () => {
  const lock = { current: true };
  const reason = callback('navigationReason', {
    deletingServer: null,
    rootBrowser: null,
    busy: false,
    lock,
    dirty: false,
    localDirty: false,
    t: (zh: string) => zh,
  });
  assert.match(reason(), /操作进行中/);
  lock.current = false;
  assert.equal(reason(), null);
});
test('SSH aliases local and new remain server selections, and new servers expose their required root', async () => {
  const page = harness({}, ['local', 'new']);
  let contract = await page.ready();
  assert.equal((contract.details as { selected: string }).selected, 'server:local');
  assert.ok(!contract.actions.some((action) => action.id === 'resource-local'));
  assert.equal(new Set(contract.actions.map((action) => action.id)).size, contract.actions.length);
  await page.action('resource-select-local');
  contract = await page.action('resource-select-server-local');
  assert.equal((contract.details as { selected: string }).selected, 'server:local');
  contract = await page.action('resource-select-server-new');
  assert.equal((contract.details as { selected: string }).selected, 'server:new');
  assert.ok(contract.fields.some((field) => field.id === 'resource-host' && field.value === 'new'));
  contract = await page.action('resource-new');
  assert.equal((contract.details as { selected: string }).selected, 'new');
  assert.ok(contract.fields.some((field) => field.id === 'resource-root'));
});
test('key and password modes explain agent and password-only constraints', async () => {
  const page = harness();
  await page.ready();
  page.fill('resource-auth', 'key');
  assert.match(page.text(page.nodes()), /ssh-add/);
  page.fill('resource-auth', 'password');
  assert.match(page.text(page.nodes()), /一次性动态口令/);
});

test('leaving key authentication clears the hidden identity path', () => {
  const editor = {
    ...resource.newServer(),
    auth: 'key',
    identity_file: '/private/id_ed25519',
  };
  let password = 'secret',
    next: unknown;
  callback('editField', {
    editor,
    setPassword: (value: string) => {
      password = value;
    },
    editServer: (server: unknown) => {
      next = server;
    },
  })('auth', 'config');
  assert.equal(password, '');
  assert.deepEqual(next, { ...editor, auth: 'config', identity_file: '' });
});

test('leaving password authentication clears the hidden password draft', () => {
  const editor = { ...resource.newServer(), auth: 'password' };
  let password = 'secret',
    next: unknown;
  callback('editField', {
    editor,
    setPassword: (value: string) => {
      password = value;
    },
    editServer: (server: unknown) => {
      next = server;
    },
  })('auth', 'config');
  assert.equal(password, '');
  assert.deepEqual(next, { ...editor, auth: 'config' });
});
test('failed checks retain readable error evidence only when their details are visible', async () => {
  const page = harness();
  await page.ready();
  page.failProbe();
  let contract = await page.action('resource-check');
  assert.ok(!('checkEvidence' in (contract.details as object)));
  contract = await page.action('resource-check-details');
  assert.match(
    (contract.details as { checkEvidence: { probe_error: string } }).checkEvidence.probe_error,
    /SSH unavailable/,
  );
  contract = await page.action('resource-check-details');
  assert.ok(!('checkEvidence' in (contract.details as object)));
});

test('adding a server keeps the saved ones', async () => {
  const page = harness({ '/some/case': null });
  const contract = await page.ready();
  assert.ok(contract.actions.some((action) => action.id === 'resource-select-server-7920land'));
  assert.ok(contract.actions.some((action) => action.id === 'resource-select-server-7920mp'));
  await page.action('resource-new');
  page.fill('resource-host', 'new-host');
  await page.action('resource-save');
  const saved = page.calls.find((call) => call.command === 'remote_save_config');
  assert.deepEqual(
    (saved?.args?.config as { servers: resource.Server[] }).servers.map((server) => server.host),
    ['7920land', '7920mp', 'new-host'],
  );
});
test('compute resources do not read the open project: a ninety-station project costs nothing', async () => {
  const page = harness(
    Object.fromEntries(
      Array.from({ length: 90 }, (_, i) => [
        `/batch/site-${i}`,
        { host: '7920land', job: `j${i}` },
      ]),
    ),
  );
  const contract = await page.ready();
  assert.ok(contract.actions.some((action) => action.id === 'resource-select-server-7920land'));
  assert.deepEqual(
    page.calls
      .map((call) => call.command)
      .filter((command) =>
        ['remote_job_record', 'case_runs', 'study_status', 'remote_status'].includes(command),
      ),
    [],
  );
  // The server tasks of a project belong to the run and history pages, not to this one.
  assert.ok(!contract.actions.some((action) => /^resource-(task|case|study)-/.test(action.id)));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-load-tasks'));
});
test('failed configuration loads disable adding and saving, including after a successful load', async () => {
  const failures = { config: true };
  const page = harness({}, undefined, failures);
  let contract = await page.ready();
  const add = contract.actions.find((action) => action.id === 'resource-new')!;
  assert.equal(add.enabled, false);
  await add.run();
  assert.ok(!page.render().actions.some((action) => action.id === 'resource-save'));
  failures.config = false;
  await page.action('resource-refresh');
  failures.config = true;
  await assert.rejects(page.action('resource-refresh'), /Unreadable server configuration/);
  page.render();
  contract = page.fill('resource-username', 'changed');
  const save = contract.actions.find((action) => action.id === 'resource-save')!;
  assert.equal(save.enabled, false);
  await assert.rejects(async () => await save.run(), /服务器配置尚未成功加载/);
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
});

test('resource configuration returns to the pending simulation after saving or discarding edits', async () => {
  const page = harness({}, ['compute'], {}, { route: 'new-case', step: 1 });
  await page.ready();
  await page.action('resource-back-simulation');
  assert.equal(page.routes.at(-1), 'new-case');
  page.fill('resource-host', 'changed');
  assert.equal(
    page.contract().actions.find((action) => action.id === 'resource-back-simulation')?.enabled,
    false,
  );
  await page.action('resource-discard');
  assert.equal(
    page.contract().actions.find((action) => action.id === 'resource-back-simulation')?.enabled,
    true,
  );
});

test('configuring a simulation opens its selected resource instead of another server', async () => {
  const page = harness(
    {},
    ['other', 'selected'],
    {},
    { route: 'new-case', step: 1, resourceHost: 'selected' },
  );
  await page.ready();
  assert.equal((page.contract().details as { selected: string }).selected, 'server:selected');
});

test('local default data paths save with the server list intact', async () => {
  const page = harness();
  await page.ready();
  const contract = await page.action('resource-select-local');
  assert.deepEqual(
    contract.fields.filter((field) => field.id.startsWith('resource-local-')).map((f) => f.id),
    [
      'resource-local-project_dir',
      'resource-local-rawdata_dir',
      'resource-local-runtime_dir',
      'resource-local-forcing_dir',
    ],
  );
  page.fill('resource-local-rawdata_dir', ' /Volumes/Data/CoLMrawdata ');
  page.fill('resource-local-forcing_dir', '/Volumes/Data/CoLM_Forcing/JRA3Q');
  await page.action('resource-local-data-save');
  const saved = page.calls.filter((call) => call.command === 'remote_save_config').at(-1)!;
  const config = saved.args!.config as {
    servers: { host: string }[];
    local: Record<string, string>;
  };
  assert.deepEqual(
    config.servers.map((server) => server.host),
    ['7920land', '7920mp'],
  );
  assert.deepEqual(config.local, {
    project_dir: '',
    rawdata_dir: '/Volumes/Data/CoLMrawdata',
    runtime_dir: '',
    forcing_dir: '/Volumes/Data/CoLM_Forcing/JRA3Q',
  });
});

test('a saved server shows its live status panel with its partition', async () => {
  const page = harness();
  await page.ready();
  const panel = page.liveLoads().at(-1);
  assert.equal(panel?.host, '7920land');
  assert.equal(panel?.partition, '');
});

const SAVED_LOCAL = {
  project_dir: '/saved/projects',
  rawdata_dir: '/saved/raw',
  runtime_dir: '/saved/runtime',
  forcing_dir: '/saved/forcing',
};

test('local default path edits join the unsaved guard and block switching, refreshing and adding servers', async () => {
  const page = harness({}, undefined, {}, undefined, undefined, SAVED_LOCAL);
  await page.ready();
  assert.equal(page.guard(), null, 'loaded local defaults are not dirty');
  let contract = await page.action('resource-select-local');
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-project_dir')?.value,
    '/saved/projects',
    'the saved baseline is shown',
  );
  page.fill('resource-local-project_dir', '/edited/projects');
  assert.match(page.guard()!, /本机默认路径/);
  contract = page.contract();
  assert.equal((contract.details as { localDirty: boolean }).localDirty, true);
  assert.equal((contract.details as { dirty: boolean }).dirty, true);
  for (const id of ['resource-select-server-7920land', 'resource-new', 'resource-refresh'])
    assert.equal(contract.actions.find((action) => action.id === id)?.enabled, false, id);
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-local-data-discard')?.enabled,
    true,
  );
  // 放弃回到加载配置时的真实已保存值，而不是中间输入。
  contract = await page.action('resource-local-data-discard');
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-project_dir')?.value,
    '/saved/projects',
  );
  assert.equal((contract.details as { localDirty: boolean }).localDirty, false);
  assert.equal(page.guard(), null);
  assert.ok(!contract.actions.some((action) => action.id === 'resource-local-data-discard'));
});

test('unsaved local default paths block resource switching with their own reason', () => {
  const reason = callback('navigationReason', {
    deletingServer: null,
    rootBrowser: null,
    busy: false,
    lock: { current: false },
    dirty: false,
    localDirty: true,
    t: (zh: string) => zh,
  });
  assert.match(reason()!, /本机默认路径/);
});

test('saving local default paths writes the local block, advances the baseline and clears the old success on edit', async () => {
  const page = harness({}, undefined, {}, undefined, undefined, SAVED_LOCAL);
  await page.ready();
  await page.action('resource-select-local');
  page.fill('resource-local-rawdata_dir', ' /edited/raw ');
  page.fill('resource-local-forcing_dir', '/edited/forcing');
  let contract = await page.action('resource-local-data-save');
  const saved = page.calls.filter((call) => call.command === 'remote_save_config').at(-1)!;
  const config = saved.args!.config as {
    servers: { host: string }[];
    local: Record<string, string>;
  };
  assert.deepEqual(
    config.servers.map((server) => server.host),
    ['7920land', '7920mp'],
  );
  // NewCasePage 新建模拟时读取的就是 remote_config.local 这一块（simulation-page.test.ts 覆盖读取一侧）。
  assert.deepEqual(config.local, {
    project_dir: '/saved/projects',
    rawdata_dir: '/edited/raw',
    runtime_dir: '/saved/runtime',
    forcing_dir: '/edited/forcing',
  });
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-rawdata_dir')?.value,
    '/edited/raw',
    'saved paths are shown trimmed',
  );
  assert.equal(page.guard(), null);
  assert.ok(page.notices().some((text) => text.includes('已保存本机默认数据路径')));
  // 改值后上一次的保存成功提示必须消失。
  page.fill('resource-local-runtime_dir', '/edited/runtime');
  assert.ok(!page.notices().some((text) => text.includes('已保存本机默认数据路径')));
  assert.match(page.guard()!, /本机默认路径/);
  // 放弃回到最近一次真实保存的值，而不是加载时的 /saved/raw。
  contract = await page.action('resource-local-data-discard');
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-rawdata_dir')?.value,
    '/edited/raw',
  );
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-runtime_dir')?.value,
    '/saved/runtime',
  );
  assert.equal(page.guard(), null);
});

test('a failed local default save keeps the edited input and does not report success', async () => {
  const page = harness({}, undefined, { save: true }, undefined, undefined, SAVED_LOCAL);
  await page.ready();
  await page.action('resource-select-local');
  page.fill('resource-local-project_dir', '/edited/projects');
  await assert.rejects(page.action('resource-local-data-save'), /Cannot save connection settings/);
  page.render();
  const contract = page.contract();
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-project_dir')?.value,
    '/edited/projects',
    'failed saves never roll the input back',
  );
  assert.ok(!page.notices().some((text) => text.includes('已保存本机默认数据路径')));
  assert.match(page.guard()!, /本机默认路径/);
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-local-data-save')?.enabled,
    true,
    'the busy lock is released so saving can be retried',
  );
});

test('a configuration refresh cannot overwrite unsaved local default paths', async () => {
  const page = harness({}, undefined, {}, undefined, undefined, SAVED_LOCAL);
  await page.ready();
  await page.action('resource-select-local');
  page.fill('resource-local-project_dir', '/edited/projects');
  // 模拟返回模拟设置等场景让加载 effect 重跑（刷新按钮本身在脏状态下已禁用，这里守住内部路径）。
  page.restartEffect();
  for (let i = 0; i < 4; i++) await new Promise((resolve) => setImmediate(resolve));
  page.render();
  const contract = page.contract();
  assert.equal(
    contract.fields.find((field) => field.id === 'resource-local-project_dir')?.value,
    '/edited/projects',
  );
  assert.match(page.guard()!, /本机默认路径/);
});

test('a server can be stored inside the app as a reusable case', async () => {
  const page = harness();
  await page.ready();
  page.fill('resource-name', '摆渡船');
  const contract = await page.action('resource-save-case');
  const saved = page.calls.find((call) => call.command === 'remote_case_save');
  assert.equal(saved?.args?.name, '摆渡船');
  assert.equal((saved?.args?.server as resource.Server).host, '7920land');
  const preset = contract.fields.find((field) => field.id === 'resource-preset');
  assert.ok(
    preset?.options?.some((option) => String(option.label).includes('案例：摆渡船')),
    'the saved case appears in the cluster preset list',
  );
  // 保存结果与案例下拉项都已出现；案例不带本机凭据（后端负责清掉，这里核对请求没带密码字段）。
  assert.ok(!JSON.stringify(saved?.args).includes('password'));
});
