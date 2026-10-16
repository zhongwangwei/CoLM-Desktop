import { useCallback, useEffect, useRef, useState } from 'react';
import { remoteRunPermissions, reconcileRemoteRun } from '../runs/run-model';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import {
  newServer,
  replaceServer,
  remoteCheckRequest,
  localRunEntries,
  studyResourcePermissions,
  resourceCheckSummary,
  resourceTaskInfo,
  type Server,
  type ResourceCheck,
} from './resource';
import NavIcon from '../../components/NavIcon';
import './resources-page.css';
export default function ResourcesPage() {
  const { state, update, navigate } = useApp(),
    en = state.language === 'en',
    t = (a: string, b: string) => (en ? b : a);
  const [servers, setServers] = useState<Server[]>([]),
    [aliases, setAliases] = useState<string[]>([]),
    [editor, setEditor] = useState<Server | null>(null),
    [original, setOriginal] = useState<string | null>(null),
    [baseline, setBaseline] = useState(''),
    [selected, setSelected] = useState('local'),
    [password, setPassword] = useState('');
  const [advanced, setAdvanced] = useState(false),
    [directories, setDirectories] = useState(false),
    [checkDetails, setCheckDetails] = useState(false),
    [maintenance, setMaintenance] = useState(false),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(''),
    [configLoaded, setConfigLoaded] = useState(false),
    [historyErrors, setHistoryErrors] = useState<string[]>([]),
    [output, setOutput] = useState<Record<string, unknown>>({}),
    [records, setRecords] = useState<{ dir: string; record: unknown; kind: 'case' | 'study' }[]>(
      [],
    ),
    [taskDetails, setTaskDetails] = useState<Record<string, boolean>>({});
  const [preset, setPreset] = useState('default'),
    [snapshot, setSnapshot] = useState(''),
    [workspace, setWorkspace] = useState(''),
    [job, setJob] = useState(''),
    [action, setAction] = useState('test'),
    [remoteCase, setRemoteCase] = useState(''),
    [packageName, setPackageName] = useState('colm-core'),
    [changeKind, setChangeKind] = useState('refactor');
  const lock = useRef(false),
    epoch = useRef({ active: false }),
    initialized = useRef(false);
  const dirty = Boolean(editor && (JSON.stringify(editor) !== baseline || password));
  function navigationReason() {
    return busy || lock.current
      ? t('操作进行中，请等待完成。', 'Wait for the current operation to finish.')
      : dirty
        ? t(
            '请先保存连接设置或放弃修改，再切换资源或离开。',
            'Save the connection or discard edits before switching resources or leaving.',
          )
        : null;
  }
  useNavigationGuard(navigationReason);
  function clearFeedback() {
    setOutput((current) => {
      const next = { ...current };
      delete next.save;
      delete next.credentials;
      return next;
    });
  }
  function selectResource(server: Server | null) {
    const reason = navigationReason();
    if (reason) {
      setError(reason);
      return;
    }
    setSelected(server ? `server:${server.host}` : 'local');
    setOriginal(server?.host ?? null);
    setEditor(server ? structuredClone(server) : null);
    setBaseline(server ? JSON.stringify(server) : '');
    setPassword('');
    setError('');
    clearFeedback();
    setCheckDetails(false);
    setDirectories(false);
    setAdvanced(false);
    setMaintenance(false);
  }
  function addServer() {
    if (!configLoaded) return;
    if (navigationReason()) {
      setError(navigationReason()!);
      return;
    }
    const next = newServer();
    setSelected('new');
    setOriginal(null);
    setEditor(next);
    setBaseline('');
    setPassword('');
    clearFeedback();
    setCheckDetails(false);
    setDirectories(true);
    setAdvanced(false);
    setMaintenance(false);
  }
  function discard() {
    if (busy || lock.current) return;
    const saved = servers.find((server) => server.host === original);
    setEditor(saved ? structuredClone(saved) : null);
    setBaseline(saved ? JSON.stringify(saved) : '');
    setSelected(saved ? `server:${saved.host}` : 'local');
    setPassword('');
    setError('');
    clearFeedback();
  }
  const refresh = useCallback(async () => {
    const token = epoch.current;
    setConfigLoaded(false);
    const config = await invokeJson<{ servers: Server[] }>('remote_config');
    if (!token.active || token !== epoch.current) return;
    setServers(config.servers);
    setConfigLoaded(true);
    if (!initialized.current) {
      initialized.current = true;
      const requestedHost = state.simulationReturn?.resourceHost;
      const first =
        requestedHost === undefined
          ? config.servers[0]
          : config.servers.find((server) => server.host === requestedHost);
      if (first) {
        setSelected(`server:${first.host}`);
        setOriginal(first.host);
        setEditor(structuredClone(first));
        setBaseline(JSON.stringify(first));
      }
    }
    const errors: string[] = [];
    const [hosts, rows, histories] = await Promise.all([
      invokeJson<string[]>('remote_ssh_hosts'),
      Promise.all(
        state.cases.map(async (c) => ({
          dir: c.dir,
          kind: 'case' as const,
          record: await invokeJson('remote_job_record', { case: c.dir }).catch((e) => ({
            error: String(e),
          })),
        })),
      ),
      Promise.all(
        state.cases.map((c) =>
          invokeJson<{ studies?: { dir: string }[] }>('case_runs', { case: c.dir }).catch((e) => {
            errors.push(`${c.dir}: ${String(e)}`);
            return { studies: [] };
          }),
        ),
      ),
    ]);
    const studyDirs = [
      ...new Set(histories.flatMap((h) => (h.studies ?? []).map((s) => s.dir)).filter(Boolean)),
    ];
    const studies = await Promise.all(
      studyDirs.map(async (dir) => {
        const record = await invokeJson<{ remote?: unknown }>('study_status', {
          studyDir: dir,
        }).catch((e) => ({ remote: true, error: String(e) }));
        return {
          dir,
          kind: 'study' as const,
          record: studyResourcePermissions(record).exists ? record : null,
        };
      }),
    );
    if (token.active && token === epoch.current) {
      setAliases(hosts);
      setHistoryErrors(errors);
      setRecords([...rows, ...studies].filter((r) => r.record));
      setOutput((current) => {
        const next = { ...current };
        for (const { dir } of [...rows, ...studies]) delete next[dir];
        return next;
      });
    }
  }, [state.cases, state.simulationReturn?.resourceHost]);

  useEffect(() => {
    const token = { active: true };
    epoch.current = token;
    setBusy(true);
    void refresh()
      .catch((e) => {
        if (token.active) setError(String(e));
      })
      .finally(() => {
        if (token.active) setBusy(false);
      });
    return () => {
      token.active = false;
    };
  }, [refresh]);
  async function execute(key: string, fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    setOutput((o) => {
      const next = { ...o };
      delete next[key];
      return next;
    });
    const token = epoch.current;
    try {
      const result = await fn();
      if (token.active && token === epoch.current)
        setOutput((o) => ({ ...o, [key]: result ?? t('完成', 'Done') }));
      return result;
    } catch (e) {
      if (token.active && token === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (token.active && token === epoch.current) setBusy(false);
    }
  }
  const run = (key: string, fn: () => Promise<unknown>) => void execute(key, fn).catch(() => {});
  async function caseAction(
    command: 'remote_status' | 'remote_fetch' | 'remote_cancel',
    dir: string,
  ) {
    const result = await invokeJson(command, { case: dir });
    return command === 'remote_status'
      ? result
      : {
          ...(await invokeJson<Record<string, unknown>>('remote_status', { case: dir })),
          action_result: result,
        };
  }
  async function cancelStudy(dir: string) {
    const cancellation = await invokeJson('remote_study_cancel', { studyDir: dir });
    try {
      return {
        cancellation,
        reconciled_status: await invokeJson('remote_study_status', { studyDir: dir }),
      };
    } catch (error) {
      return { cancellation, status_error: String(error) };
    }
  }
  async function save() {
    if (!editor) return;
    if (!configLoaded)
      throw new Error(
        t(
          '服务器配置尚未成功加载，请先刷新。',
          'Load the server configuration successfully before saving.',
        ),
      );
    if (password)
      throw new Error(
        t('请先登记密码，或清空密码后保存。', 'Register or clear the password before saving.'),
      );
    const next = replaceServer(servers, original, editor);
    await invoke('remote_save_config', { config: { servers: next } });
    setServers(next);
    setOriginal(editor.host);
    setSelected(`server:${editor.host}`);
    setBaseline(JSON.stringify(editor));
  }
  async function checkConnection() {
    if (!editor) return;
    const server = structuredClone(editor),
      configuration = JSON.stringify(server);
    const [probe, engines] = await Promise.allSettled([
      invokeJson<ResourceCheck['probe']>('remote_probe', { server }),
      invokeJson<ResourceCheck['engines']>('remote_engines', { server }),
    ]);
    const result: ResourceCheck = {
      configuration,
      checked_at: new Date().toISOString(),
      ...(probe.status === 'fulfilled'
        ? { probe: probe.value }
        : { probe_error: String(probe.reason) }),
      ...(engines.status === 'fulfilled'
        ? { engines: engines.value }
        : { engine_error: String(engines.reason) }),
    };
    const summary = resourceCheckSummary(server, result, en);
    return {
      ...result,
      ok:
        [summary.ssh, summary.engine, summary.mapping].every((item) => item.state === 'success') &&
        !result.probe?.problems?.length,
    };
  }
  async function pickIdentity() {
    const path = await invoke<string | null>('pick_file', {
      key: 'resource-identity',
      filter: null,
    });
    if (path) editField('identity_file', path);
    return path;
  }
  async function checkLocal() {
    const kernels = await invokeJson<typeof state.kernels>('list_kernels');
    update({ kernels });
    await invoke('backend_ready');
    return t('后端已连接', 'Backend connected');
  }
  function editServer(server: Server) {
    setEditor(server);
    setOutput((current) => {
      const next = { ...current };
      delete next[`check:${selected}`];
      return next;
    });
  }
  function editField(key: keyof Server, value: string) {
    if (key === 'auth' && value !== 'password') setPassword('');
    if (editor)
      editServer({ ...editor, [key]: typeof editor[key] === 'number' ? Number(value) : value });
  }
  async function registerPassword() {
    if (!editor) return;
    await invoke('remote_set_password', {
      host: editor.host,
      username: editor.username,
      port: editor.port,
      password,
    });
    setPassword('');
    setOutput((current) => {
      const next = { ...current };
      delete next[`check:${selected}`];
      delete next[`check:server:${editor.host}`];
      return next;
    });
    return t('密码已登记到本次会话', 'Password registered for this session');
  }
  const selectedServer = servers.find((server) => server.host === original);
  const checkResult = output[`check:${selected}`] as ResourceCheck | undefined;
  const check = editor ? resourceCheckSummary(editor, checkResult, en) : null;
  const checkEvidence =
    check?.checkedAt && checkResult
      ? {
          checked_at: checkResult.checked_at,
          probe_error: checkResult.probe_error,
          engine_error: checkResult.engine_error,
          probe: checkResult.probe,
          engines: checkResult.engines,
        }
      : null;
  const basicFields: [keyof Server, string][] = [
    ['host', t('服务器地址 / SSH 别名', 'Server address / SSH alias')],
    ['username', t('登录账号', 'Username')],
  ];
  const extraFields: [keyof Server, string][] = [
    ['port', t('端口（0 使用 SSH 默认）', 'Port (0 uses SSH default)')],
    ['threads', t('线程', 'Threads')],
    ['partition', t('分区 / 队列', 'Partition / queue')],
    ['account', t('账户', 'Account')],
    ['walltime', t('时限', 'Walltime')],
    ['cpus', 'CPUs'],
    ['memory_gb', t('内存 GB', 'Memory GB')],
    ['nodes', t('节点', 'Nodes')],
    ['fetch_vars', t('取回变量', 'Fetch variables')],
    ['env_script', t('环境脚本', 'Environment script')],
  ];
  const guideFields: UiField[] = editor
    ? [
        ...basicFields,
        ...(directories
          ? [['root', t('远程工作目录', 'Remote working root')] as [keyof Server, string]]
          : []),
        ...(advanced ? extraFields : []),
      ].map(([key, label]) => ({
        id: `resource-${key}`,
        label,
        value: String(editor[key]),
        type: typeof editor[key] === 'number' ? 'number' : 'text',
        set: (v) => editField(key, v),
      }))
    : [];
  if (editor) {
    guideFields.push({
      id: 'resource-auth',
      label: t('认证方式', 'Authentication'),
      value: editor.auth,
      type: 'choice',
      options: ['config', 'key', 'password'].map((value) => ({ value, label: value })),
      set: (value) => editField('auth', value),
    });
    if (editor.auth === 'key')
      guideFields.push({
        id: 'resource-identity',
        label: t('密钥文件', 'Identity file'),
        value: editor.identity_file,
        type: 'text',
        set: (value) => editField('identity_file', value),
      });
    if (directories)
      for (const [i, map] of editor.maps.entries())
        for (const key of ['local', 'remote'] as const)
          guideFields.push({
            id: `resource-map-${i}-${key}`,
            label: `${key} ${i + 1}`,
            value: map[key],
            type: 'text',
            set: (value) =>
              editServer({
                ...editor,
                maps: editor.maps.map((m, n) => (n === i ? { ...m, [key]: value } : m)),
              }),
          });
    if (advanced)
      guideFields.push(
        {
          id: 'resource-scheduler',
          label: t('调度器', 'Scheduler'),
          value: editor.scheduler,
          type: 'choice',
          options: ['auto', 'bare', 'slurm', 'pbs', 'lsf'].map((value) => ({
            value,
            label: value,
          })),
          set: (value) => editField('scheduler', value),
        },
        {
          id: 'resource-directives',
          label: t('调度指令（每行一条）', 'Scheduler directives (one per line)'),
          value: editor.directives.join('\n'),
          type: 'text',
          set: (value) => editServer({ ...editor, directives: value.split('\n').filter(Boolean) }),
        },
      );
  }
  if (editor && selectedServer && maintenance)
    guideFields.push(
      ...([
        {
          id: 'resource-preset',
          label: t('内核预设', 'Kernel preset'),
          value: preset,
          type: 'text',
          set: setPreset,
        },
        {
          id: 'resource-workspace',
          label: t('工作区', 'Workspace'),
          value: workspace,
          type: 'text',
          set: setWorkspace,
        },
        {
          id: 'resource-workspace-action',
          label: t('检查任务', 'Check action'),
          value: action,
          type: 'choice',
          options: ['test', 'build-engine', 'build-kernel', 'verify'].map((value) => ({
            value,
            label: value,
          })),
          set: setAction,
        },
        {
          id: 'resource-remote-case',
          label: t('服务器算例绝对路径', 'Absolute remote case path'),
          value: remoteCase,
          type: 'text',
          set: setRemoteCase,
        },
        {
          id: 'resource-test-package',
          label: t('测试包', 'Test package'),
          value: packageName,
          type: 'text',
          set: setPackageName,
        },
        {
          id: 'resource-change-kind',
          label: t('改动类型', 'Change kind'),
          value: changeKind,
          type: 'choice',
          options: ['refactor', 'physics'].map((value) => ({ value, label: value })),
          set: setChangeKind,
        },
        {
          id: 'resource-job',
          label: t('工作区任务 ID', 'Workspace job ID'),
          value: job,
          type: 'text',
          set: setJob,
        },
        {
          id: 'resource-snapshot',
          label: t('可清理快照 ID', 'Snapshot ID to clean'),
          value: snapshot,
          type: 'text',
          set: setSnapshot,
        },
      ] as UiField[]),
    );
  const visibleRecords = records.filter(({ record, dir }) => {
    const info = resourceTaskInfo(record, output[dir]);
    return (
      info.host === original || !info.host || !servers.some((server) => server.host === info.host)
    );
  });
  const guideActions: UiAction[] = [
    {
      id: 'resource-refresh',
      label: t('刷新资源记录', 'Refresh records'),
      kind: 'click',
      enabled: !busy && !dirty,
      run: () => execute('refresh', refresh),
    },
    {
      id: 'resource-new',
      label: t('添加服务器', 'Add server'),
      kind: 'click',
      enabled: configLoaded && !busy && !dirty,
      run: addServer,
    },
    {
      id: 'resource-select-local',
      label: t('本机', 'Local'),
      kind: 'click',
      enabled: !busy && !dirty,
      run: () => selectResource(null),
    },
    ...servers.map((server) => ({
      id: `resource-select-server-${server.host}`,
      label: server.host,
      kind: 'click' as const,
      enabled: !busy && !dirty,
      run: () => selectResource(server),
    })),
  ];
  if (selected === 'local') {
    guideActions.push({
      id: 'resource-local',
      label: t('检查本机后端与内核', 'Check local backend and kernels'),
      kind: 'click',
      enabled: !busy,
      run: () => execute('local', checkLocal),
    });
    for (const [dir, task] of localRunEntries(state.running))
      if (task.running)
        guideActions.push({
          id: `resource-local-cancel-${dir}`,
          label: t('取消本机任务', 'Cancel local task'),
          kind: 'commit',
          enabled: !busy,
          run: () => execute(dir, () => invoke('cancel_runs', { cases: [dir] })),
        });
  }
  if (editor) {
    if (checkEvidence)
      guideActions.push({
        id: 'resource-check-details',
        label: t('检查详情', 'Check details'),
        kind: 'click',
        run: () => setCheckDetails(!checkDetails),
      });
    guideActions.push(
      {
        id: 'resource-advanced-toggle',
        label: t('高级设置', 'Advanced'),
        kind: 'click',
        run: () => setAdvanced(!advanced),
      },
      {
        id: 'resource-directories-toggle',
        label: t('配置数据目录', 'Configure data directories'),
        kind: 'click',
        run: () => setDirectories(!directories),
      },
      {
        id: 'resource-check',
        label: t('检查连接', 'Check connection'),
        kind: 'click',
        enabled: !busy && !!editor.host,
        run: () => execute(`check:${selected}`, checkConnection),
      },
    );
    if (dirty)
      guideActions.push(
        {
          id: 'resource-save',
          label: t('保存连接设置', 'Save connection'),
          kind: 'commit',
          enabled: configLoaded && !busy && !password,
          run: () => execute('save', save),
        },
        {
          id: 'resource-discard',
          label: t('放弃修改', 'Discard edits'),
          kind: 'click',
          enabled: !busy,
          run: discard,
        },
      );
    if (editor.auth === 'key')
      guideActions.push({
        id: 'resource-pick-identity',
        label: t('选择密钥文件', 'Choose identity file'),
        kind: 'click',
        enabled: !busy,
        run: () => execute('pick-identity', pickIdentity),
      });
    if (directories) {
      guideActions.push({
        id: 'resource-add-map',
        label: t('添加映射', 'Add mapping'),
        kind: 'click',
        enabled: !busy,
        run: () => editServer({ ...editor, maps: [...editor.maps, { local: '', remote: '' }] }),
      });
      editor.maps.forEach((_, i) =>
        guideActions.push({
          id: `resource-remove-map-${i}`,
          label: t('移除映射', 'Remove mapping'),
          kind: 'click',
          enabled: !busy,
          run: () => editServer({ ...editor, maps: editor.maps.filter((_, n) => n !== i) }),
        }),
      );
    }
    if (selectedServer) {
      const server = selectedServer;
      guideActions.push({
        id: 'resource-maintenance-toggle',
        label: t('编译、远程检查与维护', 'Build, remote checks and maintenance'),
        kind: 'click',
        run: () => setMaintenance(!maintenance),
      });
      if (maintenance)
        guideActions.push(
          {
            id: 'resource-kernels',
            label: t('Fortran 内核', 'Fortran kernels'),
            kind: 'click',
            enabled: !busy && !dirty,
            run: () =>
              execute(`maintenance:${selected}`, () =>
                invokeJson('remote_kernels', { host: server.host }),
              ),
          },
          {
            id: 'resource-build',
            label: t('编译 Fortran 内核', 'Build Fortran kernel'),
            kind: 'commit',
            enabled: !busy && !dirty && !!preset,
            run: () =>
              execute(`maintenance:${selected}`, () =>
                invokeJson('remote_build_kernel', {
                  host: server.host,
                  preset,
                  modelVersion: state.version?.code.workspace ?? null,
                }),
              ),
          },
          {
            id: 'resource-workspace-submit',
            label: t('提交远程检查', 'Submit remote check'),
            kind: 'commit',
            enabled: !busy && !dirty && !!workspace,
            run: () =>
              execute(`maintenance:${selected}`, () =>
                invokeJson('workspace_remote', {
                  name: workspace,
                  operation: 'submit',
                  host: server.host,
                  request: remoteCheckRequest(action, preset, remoteCase, packageName, changeKind),
                }),
              ),
          },
          ...(['list', 'status', 'fetch', 'cancel'] as const).map((operation) => ({
            id: `resource-workspace-${operation}`,
            label: operation,
            kind:
              operation === 'list' || operation === 'status'
                ? ('click' as const)
                : ('commit' as const),
            enabled: !busy && !dirty && !!workspace && (operation === 'list' || !!job),
            run: () =>
              execute(`maintenance:${selected}`, () =>
                invokeJson('workspace_remote', { name: workspace, operation, job: job || null }),
              ),
          })),
        );
    }
  }
  for (const { dir, kind, record } of visibleRecords) {
    guideActions.push({
      id: `resource-task-${dir}`,
      label: taskDetails[dir] ? t('收起任务', 'Hide task') : t('查看任务', 'View task'),
      kind: 'click',
      run: () => setTaskDetails((details) => ({ ...details, [dir]: !details[dir] })),
    });
    if (taskDetails[dir])
      for (const operation of ['status', 'fetch', 'cancel'] as const) {
        const permissions =
          kind === 'study'
            ? studyResourcePermissions(record, output[dir])
            : remoteRunPermissions(
                output[dir] == null
                  ? record
                  : reconcileRemoteRun(record, output[dir] as Record<string, unknown>),
              );
        guideActions.push({
          id: `resource-${kind}-${operation}-${dir}`,
          label:
            operation === 'status'
              ? t('核对状态', 'Reconcile')
              : operation === 'fetch'
                ? t('取回结果', 'Fetch results')
                : t('取消任务', 'Cancel job'),
          kind: operation === 'status' ? 'click' : 'commit',
          enabled: !busy && (!permissions || operation === 'status' || permissions[operation]),
          run: () =>
            execute(dir, () =>
              kind === 'study'
                ? operation === 'cancel'
                  ? cancelStudy(dir)
                  : invokeJson(
                      operation === 'fetch' ? 'remote_study_fetch' : 'remote_study_status',
                      { studyDir: dir },
                    )
                : caseAction(
                    operation === 'fetch'
                      ? 'remote_fetch'
                      : operation === 'cancel'
                        ? 'remote_cancel'
                        : 'remote_status',
                    dir,
                  ),
            ),
        });
      }
  }
  if (state.simulationReturn)
    guideActions.push({
      id: 'resource-back-simulation',
      label: t('返回模拟设置', 'Back to simulation setup'),
      kind: 'click',
      enabled: !busy && !dirty,
      run: () =>
        navigate(state.simulationReturn!.route === 'new-case' ? 'new-case' : 'case', 'settings'),
    });
  usePageContract({
    title: t('计算资源', 'Resources'),
    step: 'resources',
    status: busy ? 'working' : error || 'ready',
    details: {
      selected,
      dirty,
      historyErrors,
      check,
      ...(checkDetails && checkEvidence ? { checkEvidence } : {}),
      local: selected === 'local' ? output.local : undefined,
      records: visibleRecords.map(({ dir, record }) => ({
        dir,
        ...resourceTaskInfo(record, output[dir]),
        ...(taskDetails[dir] ? { record, result: output[dir] } : {}),
      })),
    },
    fields: guideFields.map((field) => ({ ...field, disabled: busy })),
    actions: guideActions,
  });
  function actionButton(id: string, primary = false) {
    const control = guideActions.find((item) => item.id === id);
    return control ? (
      <Button
        key={id}
        variant={primary ? 'primary' : 'secondary'}
        disabled={control.enabled === false}
        onClick={() => {
          void Promise.resolve(control.run()).catch((e) => setError(String(e)));
        }}
      >
        {control.label}
      </Button>
    ) : null;
  }
  function field(id: string) {
    const control = guideFields.find((item) => item.id === id);
    return control ? (
      <Field key={id} label={control.label}>
        {control.type === 'choice' ? (
          <Select
            disabled={busy}
            value={control.value}
            onChange={(e) => control.set(e.target.value)}
          >
            {control.options?.map((option) => (
              <option key={option.value} value={option.value}>
                {option.label}
              </option>
            ))}
          </Select>
        ) : id === 'resource-directives' ? (
          <textarea
            disabled={busy}
            value={control.value}
            onChange={(e) => control.set(e.target.value)}
          />
        ) : (
          <Input
            disabled={busy}
            type={control.type === 'number' ? 'number' : 'text'}
            list={id === 'resource-host' ? 'ssh-aliases' : undefined}
            value={control.value}
            onChange={(e) => control.set(e.target.value)}
          />
        )}
      </Field>
    ) : null;
  }
  return (
    <div className="page-stack resources-page">
      <header className="resources-heading">
        <div>
          <h1>{t('计算资源', 'Resources')}</h1>
          <p>
            {t(
              '管理可用的计算服务器，配置连接并检查运行环境。',
              'Manage compute servers, configure connections and check the environment.',
            )}
          </p>
        </div>
        {actionButton('resource-back-simulation')}
        {actionButton('resource-new', true)}
      </header>
      {busy && <Notice>{t('操作进行中…', 'Operation in progress…')}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
      {historyErrors.map((message) => (
        <Notice key={message} tone="error">
          {message}
        </Notice>
      ))}
      <div className="resources-selector">
        <Button
          className={`resource-choice ${selected === 'local' ? 'selected' : ''}`}
          aria-pressed={selected === 'local'}
          disabled={busy || dirty}
          onClick={() => selectResource(null)}
        >
          <span className="resource-device" aria-hidden="true">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7">
              <rect x="3" y="3" width="18" height="13" rx="1.5" />
              <path d="M8 21h8M12 16v5M2 18h20" />
            </svg>
          </span>
          <span>
            <strong>{t('本机', 'Local')}</strong>
            <small>{output.local ? String(output.local) : t('尚未检查', 'Not checked')}</small>
          </span>
          {selected === 'local' && <span className="badge">{t('当前选择', 'Selected')}</span>}
        </Button>
        {servers.map((server) => (
          <Button
            key={server.host}
            className={`resource-choice ${selected === `server:${server.host}` ? 'selected' : ''}`}
            aria-pressed={selected === `server:${server.host}`}
            disabled={busy || dirty}
            onClick={() => selectResource(server)}
          >
            <span className="resource-device" aria-hidden="true">
              <NavIcon name="resources" />
            </span>
            <span>
              <strong>{server.host}</strong>
              <small>
                {
                  resourceCheckSummary(
                    server,
                    output[`check:server:${server.host}`] as ResourceCheck | undefined,
                    en,
                  ).ssh.label
                }
              </small>
            </span>
            {selected === `server:${server.host}` && (
              <span className="badge">{t('当前选择', 'Selected')}</span>
            )}
          </Button>
        ))}
      </div>
      {selected === 'local' && (
        <Card title={t('本机计算环境', 'Local environment')}>
          <p>
            {navigator.hardwareConcurrency || 1} CPU ·{' '}
            {state.kernels.map((kernel) => kernel.preset).join(', ') ||
              t('尚无 Fortran 内核', 'No Fortran kernel')}
          </p>
          {actionButton('resource-local')}
          {output.local != null && <Notice>{String(output.local)}</Notice>}
          {localRunEntries(state.running).map(([dir, task]) => (
            <div className="resource-task-row" key={dir}>
              <span>
                {dir} · {task.status}
              </span>
              {actionButton(`resource-local-cancel-${dir}`)}
            </div>
          ))}
        </Card>
      )}
      {editor && (
        <Card
          className="resource-configuration"
          title={`${original ?? t('新服务器', 'New server')} ${t('服务器配置', 'configuration')}`}
        >
          <div className="resource-connection-grid">
            {field('resource-host')}
            <div className="field">
              <span className="field-label">{t('认证方式', 'Authentication')}</span>
              <div
                className="resource-auth"
                role="group"
                aria-label={t('认证方式', 'Authentication')}
              >
                {[
                  ['config', 'SSH config'],
                  ['key', t('密钥', 'Key')],
                  ['password', t('密码', 'Password')],
                ].map(([value, label]) => (
                  <Button
                    key={value}
                    aria-pressed={editor.auth === value}
                    variant={editor.auth === value ? 'primary' : 'secondary'}
                    disabled={busy}
                    onClick={() => editField('auth', value)}
                  >
                    {label}
                  </Button>
                ))}
              </div>
            </div>
          </div>
          <datalist id="ssh-aliases">
            {aliases.map((alias) => (
              <option key={alias} value={alias} />
            ))}
          </datalist>
          <div className="resource-wide-field">{field('resource-username')}</div>
          {editor.auth === 'key' && (
            <div className="resource-identity">
              {field('resource-identity')}
              {actionButton('resource-pick-identity')}
            </div>
          )}
          {editor.auth === 'password' && (
            <Field label={t('密码（仅本次原生会话）', 'Password (native session only)')}>
              <div className="browse-input">
                <Input
                  type="password"
                  autoComplete="new-password"
                  disabled={busy}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                />
                <Button
                  disabled={busy || !password || !editor.host}
                  onClick={() => run('credentials', registerPassword)}
                >
                  {t('登记密码', 'Register password')}
                </Button>
              </div>
            </Field>
          )}
          {(dirty || output.save != null || output.credentials != null) && (
            <div className="resource-save-row">
              {actionButton('resource-save')}
              {actionButton('resource-discard')}
              {dirty && <small>{t('有未保存的修改', 'Unsaved changes')}</small>}
              {['save', 'credentials']
                .filter((key) => output[key] != null)
                .map((key) => (
                  <small key={key}>{String(output[key])}</small>
                ))}
            </div>
          )}
          <section className="resource-checks">
            <h3>{t('连接与环境检查', 'Connection and environment')}</h3>
            <div className="resource-status-grid">
              {check &&
                (
                  [
                    ['ssh', t('SSH 连接', 'SSH connection')],
                    ['engine', t('Rust 引擎', 'Rust engine')],
                    ['mapping', t('输入目录映射', 'Input directory mapping')],
                  ] as const
                ).map(([key, label]) => (
                  <div className="resource-status" key={key}>
                    <span className={`resource-status-icon ${check[key].state}`} aria-hidden="true">
                      {check[key].state === 'success'
                        ? '✓'
                        : check[key].state === 'error'
                          ? '×'
                          : check[key].state === 'warning'
                            ? '!'
                            : '○'}
                    </span>
                    <span>
                      <strong>{label}</strong>
                      <small>{check[key].label}</small>
                    </span>
                  </div>
                ))}
            </div>
            {check?.checkedAt && (
              <p className="resource-check-time">
                {t('上次检查', 'Last checked')}:{' '}
                {new Date(check.checkedAt).toLocaleString(en ? 'en-US' : 'zh-CN')}
              </p>
            )}
            <div className="actions">
              {actionButton('resource-check', true)}
              {actionButton('resource-directories-toggle')}
            </div>
            {checkEvidence && (
              <Advanced
                title={t('检查详情', 'Check details')}
                open={checkDetails}
                onOpenChange={setCheckDetails}
              >
                {checkDetails && <pre>{JSON.stringify(checkEvidence, null, 2)}</pre>}
              </Advanced>
            )}
          </section>
          {directories && (
            <section className="resource-directories">
              <h3>{t('数据目录对应', 'Data directory mapping')}</h3>
              {field('resource-root')}
              {editor.maps.map((_, i) => (
                <div className="resource-map" key={i}>
                  {field(`resource-map-${i}-local`)}
                  {field(`resource-map-${i}-remote`)}
                  {actionButton(`resource-remove-map-${i}`)}
                </div>
              ))}
              {actionButton('resource-add-map')}
              <p className="muted">
                {t(
                  '路径存在不代表模型输入已完整；运行前仍需输入检查。',
                  'Existing paths do not prove complete model inputs; check inputs before running.',
                )}
              </p>
            </section>
          )}
          <Advanced open={advanced} onOpenChange={setAdvanced}>
            <div className="form-grid">
              {extraFields.map(([key]) => field(`resource-${key}`))}
              {field('resource-scheduler')}
              {field('resource-directives')}
            </div>
          </Advanced>
          {selectedServer && (
            <Advanced
              title={t('编译、远程检查与维护', 'Build, remote checks and maintenance')}
              open={maintenance}
              onOpenChange={setMaintenance}
            >
              <div className="form-grid">
                {[
                  'preset',
                  'workspace',
                  'workspace-action',
                  'remote-case',
                  'test-package',
                  'change-kind',
                  'job',
                ].map((key) => field(`resource-${key}`))}
              </div>
              <div className="actions">
                {[
                  'kernels',
                  'build',
                  'workspace-submit',
                  'workspace-list',
                  'workspace-status',
                  'workspace-fetch',
                  'workspace-cancel',
                ].map((key) => actionButton(`resource-${key}`))}
              </div>
              <Button
                disabled={busy || dirty}
                onClick={() => {
                  if (
                    window.confirm(
                      t(
                        '核对并恢复租约？仅在确认任务已结束时移除失效租约。',
                        'Reconcile leases? Remove stale leases only after confirming their jobs have stopped.',
                      ),
                    )
                  )
                    run(`maintenance:${selected}`, () =>
                      invokeJson('remote_engine_recover', { server: selectedServer }),
                    );
                }}
              >
                {t('核对并恢复租约', 'Reconcile leases')}
              </Button>
              {field('resource-snapshot')}
              <div className="actions">
                <Button
                  variant="danger"
                  disabled={busy || dirty || !snapshot}
                  onClick={() => {
                    if (
                      window.confirm(
                        t(
                          '清理该快照？后端会保护活动租约。',
                          'Clean this snapshot? Active leases are protected by the backend.',
                        ),
                      )
                    )
                      run(`maintenance:${selected}`, () =>
                        invokeJson('remote_engine_clean', { server: selectedServer, snapshot }),
                      );
                  }}
                >
                  {t('清理快照', 'Clean snapshot')}
                </Button>
                <Button
                  variant="danger"
                  disabled={busy || dirty}
                  onClick={() => {
                    if (
                      window.confirm(
                        t(
                          `移除服务器设置 ${selectedServer.host}？`,
                          `Remove server settings ${selectedServer.host}?`,
                        ),
                      )
                    )
                      run('delete', async () => {
                        const next = servers.filter(
                          (server) => server.host !== selectedServer.host,
                        );
                        await invoke('remote_save_config', { config: { servers: next } });
                        setServers(next);
                        setSelected('local');
                        setEditor(null);
                        setOriginal(null);
                        setBaseline('');
                      });
                  }}
                >
                  {t('删除设置', 'Delete settings')}
                </Button>
              </div>
              {output[`maintenance:${selected}`] != null && (
                <pre>{JSON.stringify(output[`maintenance:${selected}`], null, 2)}</pre>
              )}
            </Advanced>
          )}
        </Card>
      )}
      <Card
        className="resource-tasks"
        title={
          selected === 'local'
            ? t('其他远程任务记录', 'Other remote task records')
            : t('服务器上的任务', 'Server tasks')
        }
      >
        {!visibleRecords.length && (
          <p className="muted">{t('暂无已登记的远程任务。', 'No recorded remote tasks.')}</p>
        )}
        {visibleRecords.map(({ dir, record, kind }) => {
          const info = resourceTaskInfo(record, output[dir]),
            orphan = !info.host || !servers.some((server) => server.host === info.host);
          return (
            <div className="resource-task" key={dir}>
              <div className="resource-task-row">
                <span className="resource-task-name">
                  <strong>{dir.split(/[\\/]/).filter(Boolean).at(-1) ?? dir}</strong>
                  <small>
                    {info.host ?? t('主机未记录', 'Host not recorded')} ·{' '}
                    {info.job ?? t('身份未确认', 'Unconfirmed identity')} ·{' '}
                    {info.pending
                      ? t('提交待确认', 'Submission unconfirmed')
                      : ({
                          unknown: t('状态未确认', 'Status unknown'),
                          queued: t('排队中', 'Queued'),
                          running: t('运行中', 'Running'),
                          succeeded: t('已完成', 'Succeeded'),
                          failed: t('失败', 'Failed'),
                          lost: t('未找到', 'Not found'),
                        }[info.status] ?? info.status)}
                  </small>
                </span>
                {actionButton(`resource-task-${dir}`)}
              </div>
              {orphan && (
                <p className="resource-orphan">
                  {t(
                    '记录缺少主机，或对应服务器配置已移除；展开查看原始记录并核对。',
                    'The record has no host or its server configuration was removed. Open the record to reconcile.',
                  )}
                </p>
              )}
              {taskDetails[dir] && (
                <div className="resource-task-detail">
                  <p className="path">{dir}</p>
                  <Notice>
                    {t(
                      '登记记录不是实时状态；核对不会重提任务，取消请求不代表任务已停止。',
                      'Records are not live state. Reconcile does not resubmit; a cancellation request does not prove the job stopped.',
                    )}
                  </Notice>
                  <div className="actions">
                    {['status', 'fetch', 'cancel'].map((operation) =>
                      actionButton(`resource-${kind}-${operation}-${dir}`),
                    )}
                  </div>
                  <pre>{JSON.stringify(output[dir] ?? record, null, 2)}</pre>
                </div>
              )}
            </div>
          );
        })}
      </Card>
      <footer className="resources-footer">
        {actionButton('resource-refresh')}
        <span>
          {t(
            '选择仅用于查看资源，不改变模拟运行位置。',
            'Resource selection does not change the simulation location.',
          )}
        </span>
      </footer>
    </div>
  );
}
