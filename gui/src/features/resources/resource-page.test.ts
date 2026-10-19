import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as resource from './resource';
import { remoteRunPermissions, reconcileRemoteRun } from '../runs/run-model';
import type { UiAction, UiField } from '../../lib/guide';

type Contract = { fields: UiField[]; actions: UiAction[]; details: unknown };
function harness(
  records: Record<string, unknown> = {},
  hosts = ['7920land', '7920mp'],
  failures: { config?: boolean; history?: string; save?: boolean } = {},
  simulationReturn?: { route: 'new-case' | 'case'; step: 1; resourceHost?: string },
  scan?: {
    before: (command: string, args?: Record<string, unknown>) => Promise<void>;
    studies?: Record<string, { dir: string }[]>;
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
  let unverifiedQsub = false;
  let pendingProbe: Promise<void> | undefined;
  const menus: { items: { disabled?: boolean; onSelect: () => void }[] }[] = [];
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
        return { servers };
      case 'remote_save_config':
        if (failures.save) throw new Error('Cannot save connection settings');
        return null;
      case 'remote_forget_kind_path':
        return true;
      case 'remote_ssh_hosts':
        return ['7920land', '7920mp'];
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
  const environment = {
    ...resource,
    remoteRunPermissions,
    reconcileRemoteRun,
    React: {
      createElement: (type: string, props: Record<string, unknown>) => {
        if (type === 'remote-browser') browser = props as typeof browser;
        if (type === 'context-menu') menus.push(props as (typeof menus)[number]);
        return null;
      },
    },
    RemotePathBrowser: 'remote-browser',
    ContextMenu: 'context-menu',
    Dialog: () => {},
    Advanced: () => {},
    Button: () => {},
    Card: () => {},
    Field: () => {},
    Input: () => {},
    Notice: () => {},
    Select: () => {},
    NavIcon: () => {},
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
    usePageContract: (value: Contract) => {
      contract = value;
    },
    invoke: ipc,
    invokeJson: ipc,
  };
  const Page = new Function(...Object.keys(environment), `${code};return ResourcesPage;`)(
    ...Object.values(environment),
  );
  function render() {
    cursor = 0;
    menus.length = 0;
    Page();
    return contract;
  }
  async function ready() {
    render();
    await new Promise((resolve) => setImmediate(resolve));
    return render();
  }
  async function action(id: string) {
    const item = contract.actions.find((action) => action.id === id);
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
    render,
    calls,
    routes,
    servers,
    guard: () => guard(),
    contract: () => contract,
    failProbe: () => {
      failProbe = true;
    },
    schedulers: (value: unknown) => {
      schedulers = value;
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
  const forget = () =>
    page
      .contract()
      .actions.filter((action) => action.id.startsWith('resource-forget-'))
      .map((action) => action.id);
  assert.deepEqual(forget(), [
    'resource-forget-site-sitedir',
    'resource-forget-site-forcingdir',
    'resource-forget-watershed-shapefile',
  ]);
  page.fill('resource-username', 'changed');
  await page.action('resource-save');
  assert.equal(forget().length, 3, 'saving connection settings keeps the remembered paths');
  await page.action('resource-forget-site-sitedir');
  assert.deepEqual(page.calls.find((call) => call.command === 'remote_forget_kind_path')?.args, {
    host: '7920land',
    kind: 'site',
    key: 'sitedir',
  });
  assert.deepEqual(forget(), [
    'resource-forget-site-forcingdir',
    'resource-forget-watershed-shapefile',
  ]);
  await page.action('resource-select-server-7920mp');
  assert.deepEqual(forget(), []);
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
  const contract = await page.action('resource-delete-confirm');
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
  await assert.rejects(page.action('resource-delete-confirm'), /Cannot save/);
  page.render();
  assert.equal(page.menus().length, 2);
  assert.ok(page.contract().actions.some((action) => action.id === 'resource-delete-confirm'));
  page.fill('resource-username', 'edited');
  assert.ok(page.menus().every((menu) => menu.items[0].disabled));
  assert.equal(
    page.contract().actions.find((action) => action.id === 'resource-delete-confirm')?.enabled,
    false,
  );
});

test('failed configuration refresh prevents deletion from overwriting with a stale server list', async () => {
  const failures = { config: false };
  const page = harness({}, undefined, failures);
  await page.ready();
  page.menus()[0].items[0].onSelect();
  page.render();
  failures.config = true;
  await assert.rejects(page.action('resource-refresh'), /Unreadable/);
  const contract = page.render();
  const deletion = contract.actions.find((action) => action.id === 'resource-delete-confirm');
  assert.ok(deletion);
  assert.equal(deletion.enabled, false);
  assert.ok(page.menus().every((menu) => menu.items[0].disabled));
  await assert.rejects(async () => await deletion.run(), /无法读取连接配置/);
  assert.ok(!page.calls.some((call) => call.command === 'remote_save_config'));
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
      'resource-host',
      'resource-username',
      'resource-root',
      'resource-project_dir',
      'resource-rawdata_dir',
      'resource-runtime_dir',
      'resource-forcing_dir',
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

test('remote tasks filter by actual host, preserve orphan records, and expose operations only after expansion', async () => {
  const page = harness({
    '/case/land': { host: '7920land', job: 'job1' },
    '/case/mp': { host: '7920mp', job: 'job2' },
    '/case/orphan': { host: 'removed', job: 'job3' },
    '/case/missing': { error: 'Unreadable record' },
  });
  let contract = await page.ready();
  assert.ok(contract.actions.some((action) => action.id === 'resource-task-/case/land'));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-task-/case/mp'));
  assert.ok(contract.actions.some((action) => action.id === 'resource-task-/case/orphan'));
  assert.ok(contract.actions.some((action) => action.id === 'resource-task-/case/missing'));
  assert.ok(!contract.actions.some((action) => action.id === 'resource-case-status-/case/land'));
  contract = await page.action('resource-task-/case/land');
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-case-fetch-/case/land')?.kind,
    'commit',
  );
  assert.equal(
    contract.actions.find((action) => action.id === 'resource-case-status-/case/land')?.kind,
    'click',
  );
  await page.action('resource-case-status-/case/land');
  assert.deepEqual(page.calls.at(-1), { command: 'remote_status', args: { case: '/case/land' } });
  contract = await page.action('resource-task-/case/land');
  assert.ok(!contract.actions.some((action) => action.id === 'resource-case-status-/case/land'));
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
test('refresh replaces job identities without retaining a previous job status for the same directory', async () => {
  const records: Record<string, unknown> = { '/case/land': { host: '7920land', job: 'job1' } };
  const page = harness(records);
  await page.ready();
  await page.action('resource-task-/case/land');
  await page.action('resource-case-status-/case/land');
  records['/case/land'] = { host: '7920land', job: 'new-job' };
  const contract = await page.action('resource-refresh');
  const rows = (contract.details as { records: { job: string; status: string }[] }).records;
  assert.equal(rows[0].job, 'new-job');
  assert.equal(rows[0].status, 'unknown');
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

test('a deleted case reports its history error without losing saved servers on the next save', async () => {
  const page = harness({ '/deleted': null }, undefined, { history: '/deleted' });
  const contract = await page.ready();
  assert.deepEqual((contract.details as { historyErrors: string[] }).historyErrors, [
    '/deleted: Error: Case directory was deleted',
  ]);
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

test('90-station resource refresh reads every job record, searches studies on a dozen cases, and serializes overlapping refreshes', async () => {
  const records = Object.fromEntries(
    Array.from({ length: 90 }, (_, i) => [
      `/batch/site-${i}`,
      { host: '7920land', job: `job-${i}` },
    ]),
  );
  let active = 0;
  let peak = 0;
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  let block = true;
  const page = harness(records, undefined, {}, undefined, {
    before: async (command) => {
      if (!['remote_job_record', 'case_runs', 'study_status'].includes(command)) return;
      active++;
      peak = Math.max(peak, active);
      if (block) await blocked;
      await new Promise((resolve) => setImmediate(resolve));
      active--;
    },
    studies: { '/batch/site-3': Array.from({ length: 90 }, (_, i) => ({ dir: `/study/${i}` })) },
  });
  await page.ready();
  assert.equal(active, 8);
  // A changed case list restarts the effect while the previous CLI calls still exist.
  page.restartEffect();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(active, 8);
  block = false;
  release();
  // The page is usable as soon as the servers are shown; the job records finish behind it.
  const recordsRead = () =>
    (page.render().details as { records: { dir: string }[] }).records.some(
      (r) => r.dir === '/batch/site-89',
    );
  for (let i = 0; i < 5000 && !(recordsRead() && (active as number) === 0); i++) {
    await new Promise((resolve) => setImmediate(resolve));
  }
  assert.equal(page.contract().actions.find((a) => a.id === 'resource-refresh')?.enabled, true);
  assert.equal(peak, 8);
  assert.equal(active, 0);
  const details = page.contract().details as { records: { dir: string }[] };
  assert.ok(details.records.some((r) => r.dir === '/batch/site-89'));
  // Ninety job records are read, but only a dozen cases are searched for studies.
  // The eight in flight when the case list changed, plus one full pass of ninety.
  assert.equal(page.calls.filter((c) => c.command === 'remote_job_record').length, 98);
  assert.equal(page.calls.filter((c) => c.command === 'case_runs').length, 12);
  assert.equal(page.calls.filter((c) => c.command === 'study_status').length, 90);
  assert.equal(
    page.calls.filter((c) => c.command === 'study_status').at(-1)?.args?.studyDir,
    '/study/89',
  );
});

test('unmount stops scheduling remaining resource CLI calls', async () => {
  let release!: () => void;
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  const page = harness(
    Object.fromEntries(Array.from({ length: 90 }, (_, i) => [`/batch/${i}`, null])),
    undefined,
    {},
    undefined,
    {
      before: async (command) => {
        if (command === 'remote_job_record') await blocked;
      },
    },
  );
  await page.ready();
  page.unmount();
  release();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(page.calls.filter((c) => c.command === 'remote_job_record').length, 8);
  assert.equal(page.calls.filter((c) => c.command === 'case_runs').length, 0);
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
