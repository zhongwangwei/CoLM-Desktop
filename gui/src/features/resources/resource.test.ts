import test from 'node:test';
import assert from 'node:assert/strict';
import {
  newServer,
  replaceServer,
  remoteCheckRequest,
  localRunEntries,
  studyResourcePermissions,
  resourceCheckSummary,
  resourceTaskInfo,
  partitionLabel,
  SERVER_PRESETS,
  applyServerPreset,
  recognisedPreset,
  type ResourceCheck,
} from './resource';
test('server edits preserve others and reject duplicate host', () => {
  const a = { ...newServer(), host: 'a' },
    b = { ...newServer(), host: 'b' };
  assert.equal(replaceServer([a, b], 'a', { ...a, root: '/data' })[1], b);
  assert.throws(() => replaceServer([a, b], 'a', { ...a, host: 'b' }));
  assert.equal('password' in newServer(), false);
});

test('remote verification requires actual case and uses explicit test kind', () => {
  assert.throws(() => remoteCheckRequest('verify', 'default', '', 'colm-core', 'refactor'));
  assert.deepEqual(remoteCheckRequest('test', 'default', '', 'colm-core', 'refactor'), {
    action: 'test',
    preset: 'default',
    kind: 'cargo',
    package: 'colm-core',
  });
});

test('saving a removed server does not report a silent success', () => {
  assert.throws(() => replaceServer([], 'a', { ...newServer(), host: 'a' }));
});

test('local cancellation excludes server and unverified task locations', () => {
  const run = { case: '/case', status: 'submitted', running: true, logs: [], stages: {} };
  assert.deepEqual(
    localRunEntries({
      local: { ...run, location: 'local' },
      remote: { ...run, location: 'remote' },
      unknown: run,
    }).map(([dir]) => dir),
    ['local'],
  );
});

test('pending server studies remain visible with only read-only reconciliation', () => {
  const pending = { remote: null, remote_pending: { host: 'server', job: 'j1' } };
  const unresolved = studyResourcePermissions(pending);
  assert.equal(unresolved.exists, true);
  assert.equal(unresolved.pending, true);
  assert.equal(unresolved.cancel, false);
  assert.equal(unresolved.fetch, false);
  const recovered = studyResourcePermissions(pending, {
    record: { host: 'server', job: 'j1' },
    status: { state: 'running' },
  });
  assert.equal(recovered.pending, false);
  assert.equal(recovered.cancel, true);
  assert.equal(recovered.fetch, false);
  assert.equal(
    studyResourcePermissions(pending, {
      record: { host: 'server', job: 'j1' },
      status: { state: 'finished', exit_code: 0 },
    }).fetch,
    true,
  );
  assert.equal(studyResourcePermissions({ remote: null, remote_pending: null }).exists, false);
});

test('resource checks require current configuration and separate SSH, engine snapshots and path evidence', () => {
  const server = { ...newServer(), host: 'a', maps: [{ local: '/local', remote: '/remote' }] };
  assert.deepEqual(
    Object.values(resourceCheckSummary(server))
      .slice(0, 3)
      .map((item) => (item as { state: string }).state),
    ['pending', 'pending', 'pending'],
  );
  const check: ResourceCheck = {
    configuration: JSON.stringify(server),
    checked_at: '2026-10-12T05:00:00Z',
    probe: {
      hostname: 'a',
      maps: [{ local: '/local', remote: '/remote', local_exists: true, remote_exists: true }],
    },
    engines: {
      engines: [
        { id: 'one', ready: true },
        { id: 'two', ready: false },
      ],
    },
  };
  const summary = resourceCheckSummary(server, check);
  assert.equal(summary.ssh.state, 'success');
  assert.equal(summary.engine.label, '1 个可用快照');
  assert.equal(summary.mapping.label, '对应路径存在');
  assert.equal(summary.checkedAt, check.checked_at);
  assert.equal(
    resourceCheckSummary({ ...server, username: 'another' }, check).checkedAt,
    null,
    'edited credentials invalidate earlier evidence',
  );
  assert.equal(
    resourceCheckSummary({ ...server, maps: [] }, check).ssh.state,
    'pending',
    'edited mappings invalidate earlier evidence',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: undefined }).engine.state,
    'pending',
    'probe supply strategy is never a ready engine',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: {} }).engine.state,
    'pending',
    'a missing inventory is not proof of no available engine',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: { engines: [] } }).engine.state,
    'warning',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe: { hostname: 'a', maps: [] } }).mapping.state,
    'warning',
  );
  assert.equal(
    resourceCheckSummary(server, {
      ...check,
      probe: {
        hostname: 'a',
        maps: [{ local: '/different', remote: '/remote', local_exists: true, remote_exists: true }],
      },
    }).mapping.state,
    'warning',
    'counts cannot substitute for matching path identities',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe_error: 'ssh failed' }).ssh.state,
    'error',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engine_error: 'inventory failed' }).engine.state,
    'error',
  );
  assert.equal(resourceCheckSummary(server, { ...check, checked_at: 'invalid' }).checkedAt, null);
});

test('connection failures and incomplete run configuration leave dependent checks pending', () => {
  const server = { ...newServer(), host: 'a' };
  const check: ResourceCheck = {
    configuration: JSON.stringify(server),
    checked_at: '2026-10-16T05:00:00Z',
    probe: { hostname: 'a', configuration_error: 'Work directory required' },
    engine_error: 'Work directory required',
  };
  const incomplete = resourceCheckSummary(server, check);
  assert.equal(incomplete.ssh.state, 'success');
  for (const item of [incomplete.engine, incomplete.mapping, incomplete.directory]) {
    assert.deepEqual(item, { state: 'pending', label: '待完善运行配置' });
  }
  const disconnected = resourceCheckSummary(server, {
    ...check,
    probe_error: 'Connection refused',
  });
  assert.equal(disconnected.ssh.state, 'error');
  for (const item of [disconnected.engine, disconnected.mapping, disconnected.directory]) {
    assert.deepEqual(item, { state: 'pending', label: '等待连接成功' });
  }
});

test('server-native paths and first-run deployment are ready without existing engine snapshots', () => {
  const server = { ...newServer(), host: 'a' };
  const check: ResourceCheck = {
    configuration: JSON.stringify(server),
    checked_at: '2026-10-16T05:00:00Z',
    probe: { hostname: 'a', engine: 'source', root_exists: false, root_writable: true },
    engines: { engines: [] },
  };
  const summary = resourceCheckSummary(server, check);
  assert.deepEqual(summary.engine, { state: 'success', label: '首次运行自动构建引擎' });
  const installation = {
    ...check,
    probe: { ...check.probe, toolchain_installable: true, build_problems: ['cargo missing'] },
  };
  assert.deepEqual(resourceCheckSummary(server, installation).engine, {
    state: 'success',
    label: '首次运行自动安装工具并构建引擎',
  });
  assert.match(
    resourceCheckSummary(server, installation, true).engine.label,
    /installs build tools/,
  );
  assert.deepEqual(summary.mapping, { state: 'success', label: '直接使用服务器路径，无需映射' });
  assert.deepEqual(summary.directory, { state: 'success', label: '工作目录可创建' });
  assert.deepEqual(
    resourceCheckSummary(server, { ...check, probe: { ...check.probe, engine: 'prebuilt' } })
      .engine,
    { state: 'success', label: '首次运行自动部署引擎' },
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, engines: undefined }).engine.state,
    'pending',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe: { ...check.probe, root_writable: false } })
      .directory.state,
    'error',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe: { hostname: 'a' } }).directory.state,
    'pending',
  );
  assert.equal(
    resourceCheckSummary(server, { ...check, probe: { ...check.probe, root_exists: true } })
      .directory.label,
    '工作目录可写',
  );
});

test('resource task summaries retain pending host identity and require actual remote completion evidence', () => {
  const pending = {
    remote: null,
    remote_pending: { host: 'a', job: 'unconfirmed' },
    state: 'completed',
  };
  assert.deepEqual(resourceTaskInfo(pending), {
    host: 'a',
    job: 'unconfirmed',
    status: 'unknown',
    pending: true,
  });
  assert.deepEqual(resourceTaskInfo({ host: 'a', job: 'one' }), {
    host: 'a',
    job: 'one',
    status: 'unknown',
    pending: false,
  });
  assert.equal(
    resourceTaskInfo({ remote: { host: 'a', job: 'one' }, state: 'completed' }).status,
    'unknown',
    'local study state is not remote success',
  );
  const running = { record: { host: 'a', job: 'one' }, status: { state: 'running' } };
  assert.deepEqual(resourceTaskInfo(pending, running), {
    host: 'a',
    job: 'one',
    status: 'running',
    pending: false,
  });
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished' } }).status,
    'unknown',
  );
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished', exit_code: 0 } }).status,
    'succeeded',
  );
  assert.equal(
    resourceTaskInfo(pending, { ...running, status: { state: 'finished', exit_code: 2 } }).status,
    'failed',
  );
  assert.deepEqual(
    resourceTaskInfo(pending, {
      record: null,
      pending: { host: 'b', job: 'new' },
      status: { state: 'finished', exit_code: 0 },
    }),
    { host: 'b', job: 'new', status: 'unknown', pending: true },
  );
  assert.equal(
    resourceTaskInfo(pending, {
      cancellation: { request_accepted: true },
      reconciled_status: running,
    }).status,
    'running',
    'cancellation acceptance is not a stopped job',
  );
  assert.equal(resourceTaskInfo({ error: 'record read failed' }).host, null);
  assert.equal(
    resourceTaskInfo({ record: null, status: { state: 'finished', exit_code: 0 } }).status,
    'unknown',
    'completion also requires a recorded host and job',
  );
});

test('a partition reads as its node sizes, limits and whether it takes whole nodes', () => {
  const deimos = {
    name: 'deimos',
    default: false,
    exclusive: true,
    nodes: 1514,
    cores_min: 64,
    cores_max: 64,
    memory_gb_min: 500,
    memory_gb_max: 500,
    max_nodes: null,
    max_time: '7-00:00:00',
    whole_node_memory: true,
  };
  assert.equal(
    partitionLabel(deimos),
    'deimos · 1514 个节点 · 每节点 64 核 / 500 GB · 7 天 · 整节点独占（站点批量会按节点打包）',
  );
  assert.equal(
    partitionLabel({ ...deimos, name: 'debug', exclusive: false, max_time: '00:30:00' }, true),
    'debug · 1514 nodes · 64 cores / 500 GB per node · 30 min',
  );
});

test('presets carry only what a cluster shares, never an address, account or personal folder', () => {
  for (const preset of SERVER_PRESETS) {
    for (const key of ['host', 'username', 'identity_file', 'root', 'auth', 'port', 'maps'])
      assert.ok(!(key in preset.fields), `${preset.id} sets ${key}`);
    const text = JSON.stringify(preset);
    assert.ok(!/\d+\.\d+\.\d+\.\d+/.test(text), `${preset.id} names an IP address`);
    assert.ok(!/sysu_|dq013|zhwei|\/home\//i.test(text), `${preset.id} names a personal folder`);
  }
});

test('a preset fills scheduling settings and only the data folders still empty', () => {
  const land = SERVER_PRESETS.find((preset) => preset.id === 'land')!;
  const mine = {
    ...newServer(),
    host: 'land02',
    username: 'me',
    root: '/tera11/me/colm',
    scheduler: 'auto',
    rawdata_dir: '/my/rawdata',
  };
  const next = applyServerPreset(mine, land);
  assert.deepEqual(
    [next.host, next.username, next.root, next.auth],
    ['land02', 'me', '/tera11/me/colm', mine.auth],
  );
  assert.deepEqual([next.scheduler, next.partition, next.cores_per_node], ['slurm', 'first', 48]);
  assert.equal(next.rawdata_dir, '/my/rawdata', 'a folder the user set is kept');
  assert.equal(next.runtime_dir, '/shr03/CoLMruntime');
});

test('a checked server is recognised by its scheduler and partitions, not its address', () => {
  const partitions = (names: string[]) =>
    names.map(
      (name) =>
        ({ name }) as NonNullable<ResourceCheck['probe']>['partitions'] extends
          (infer P)[] | undefined
          ? P
          : never,
    );
  assert.equal(
    recognisedPreset({
      schedulers: ['slurm'],
      partitions: partitions(['first', 'largemem', 'second']),
    })?.id,
    'land',
  );
  assert.equal(
    recognisedPreset({ schedulers: ['slurm'], queues: ['deimos', 'phobos', 'debug'] })?.id,
    'tianhe-xingyi',
  );
  assert.equal(
    recognisedPreset({ schedulers: ['lsf'], queues: ['normal', 'gpu'] })?.id,
    'baiduchuan',
  );
  assert.equal(recognisedPreset({ schedulers: ['slurm'], queues: ['normal', 'gpu'] }), null);
  assert.equal(recognisedPreset({ schedulers: ['slurm'], queues: ['first', 'second'] }), null);
  assert.equal(recognisedPreset(undefined), null);
});
