import { useCallback, useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { usePageContract, type UiField } from '../../lib/guide';
import { ActionButton } from '../../components/ActionButton';
import {
  Advanced,
  Button,
  Card,
  ContextMenu,
  Dialog,
  Field,
  Input,
  Notice,
  Select,
} from '../../components/ui';
import {
  newServer,
  replaceServer,
  remoteCheckRequest,
  localRunEntries,
  resourceCheckSummary,
  partitionLabel,
  SERVER_PRESETS,
  applyServerPreset,
  recognisedPreset,
  localDefaults,
  localPathSignature,
  localPathValues,
  type LocalPathKey,
  type Partition,
  type Server,
  type ResourceCheck,
  kindPathRows,
} from './resource';
import type { DataDefaults } from '../../lib/types';
type DataPathKey = LocalPathKey;
import NavIcon from '../../components/NavIcon';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import LiveServerLoad from '../runs/LiveServerLoad';
import './resources-page.css';
export default function ResourcesPage() {
  const { state, update, navigate } = useApp(),
    en = state.language === 'en',
    t = (a: string, b: string) => (en ? b : a);
  const [localData, setLocalData] = useState<DataDefaults>(localDefaults());
  // 本机默认路径的"已保存基线"：加载成功和保存成功时才更新，放弃修改回到这里而不是回到上一次输入。
  const [savedLocal, setSavedLocal] = useState<DataDefaults>(localDefaults());
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
    [output, setOutput] = useState<Record<string, unknown>>({});
  const [rootBrowser, setRootBrowser] = useState<Server | null>(null),
    [deletingServer, setDeletingServer] = useState<Server | null>(null);
  const [preset, setPreset] = useState('default'),
    [snapshot, setSnapshot] = useState(''),
    [workspace, setWorkspace] = useState(''),
    [job, setJob] = useState(''),
    [action, setAction] = useState('test'),
    [remoteCase, setRemoteCase] = useState(''),
    [packageName, setPackageName] = useState('colm-core'),
    [changeKind, setChangeKind] = useState('refactor'),
    // 服务器探测报回的 Slurm 分区（按服务器地址），给"分区"一栏做下拉；改设置不清空它。
    [partitionsByHost, setPartitionsByHost] = useState<Record<string, Partition[]>>({}),
    // 套用了哪个集群预设（只在这次编辑里记着，用来显示它的注意事项），以及检查连接后认出的集群。
    [appliedPreset, setAppliedPreset] = useState(''),
    [recognisedByHost, setRecognisedByHost] = useState<Record<string, string>>({});
  const lock = useRef(false),
    epoch = useRef({ active: false }),
    initialized = useRef(false),
    refreshQueue = useRef<Promise<void>>(Promise.resolve());
  const dirty = Boolean(editor && (JSON.stringify(editor) !== baseline || password));
  // 本机默认路径原来只在组件状态里：切页/切资源/刷新会静默丢输入（GUI 审查 GUI-13）。
  const localDirty = localPathSignature(localData) !== localPathSignature(savedLocal);
  // refresh 在异步等待后还要知道"此刻"有没有未保存的本机路径；放进 deps 会让它换身份，
  // 进而让加载 effect 重跑、把正在编辑的值重新覆盖掉，所以只读 ref。
  const localDirtyRef = useRef(false);
  localDirtyRef.current = localDirty;
  function navigationReason() {
    return deletingServer
      ? t('请先关闭删除确认窗口。', 'Close the deletion confirmation first.')
      : rootBrowser
        ? t('请先关闭目录选择窗口。', 'Close the folder browser first.')
        : busy || lock.current
          ? t('操作进行中，请等待完成。', 'Wait for the current operation to finish.')
          : dirty
            ? t(
                '请先保存连接设置或放弃修改，再切换资源或离开。',
                'Save the connection or discard edits before switching resources or leaving.',
              )
            : localDirty
              ? t(
                  '请先保存本机默认路径或放弃修改，再切换资源或离开。',
                  'Save the local default paths or discard edits before switching resources or leaving.',
                )
              : null;
  }
  useNavigationGuard(navigationReason);
  function clearFeedback() {
    setOutput((current) => {
      const next = { ...current };
      delete next.save;
      delete next.credentials;
      delete next[`scheduler:${selected}`];
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
    setAppliedPreset('');
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
    setAppliedPreset('');
    setSelected('new');
    setOriginal(null);
    setEditor(next);
    setBaseline('');
    setPassword('');
    clearFeedback();
    setCheckDetails(false);
    setDirectories(false);
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
    const pending = refreshQueue.current
      .catch(() => {})
      .then(async () => {
        if (!token.active || token !== epoch.current) return;
        setConfigLoaded(false);
        const config = await invokeJson<{ servers: Server[]; local?: DataDefaults }>(
          'remote_config',
        );
        if (!token.active || token !== epoch.current) return;
        setServers(config.servers);
        // 刷新只在没有未保存的本机路径时重设输入与基线：否则一次后台刷新会静默覆盖用户刚改的目录。
        if (!localDirtyRef.current) {
          const loadedLocal = localDefaults(config.local);
          setLocalData(loadedLocal);
          setSavedLocal(loadedLocal);
        }
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
        // The servers are all this page needs: compute resources do not depend on which project is
        // open, so nothing of the project is read here.
        setBusy(false);
        const hosts = await invokeJson<string[]>('remote_ssh_hosts');
        if (!token.active || token !== epoch.current) return;
        setAliases(hosts);
      });
    refreshQueue.current = pending;
    return pending;
  }, [state.simulationReturn?.resourceHost]);

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
    // The backend keeps remembered per-kind paths; show its copy, not the editor's.
    const next = replaceServer(servers, original, editor).map((server) =>
      server.host === editor.host
        ? { ...server, kind_paths: servers.find((old) => old.host === original)?.kind_paths }
        : server,
    );
    await invoke('remote_save_config', { config: { servers: next } });
    setServers(next);
    setOriginal(editor.host);
    setSelected(`server:${editor.host}`);
    setBaseline(JSON.stringify(editor));
  }
  /** 删除连接设置：只改本机保存的服务器清单，服务器上的文件和任务不动。 */
  async function deleteConnection() {
    const deleting = deletingServer;
    if (!deleting) return;
    await execute('delete', async () => {
      if (!configLoaded)
        throw new Error(
          t('无法读取连接配置，请刷新后重试', 'Cannot read connection settings; refresh and retry'),
        );
      if (dirty) throw new Error(t('请先保存或放弃修改', 'Save or discard edits first'));
      const host = deleting.host;
      if (!servers.some((server) => server.host === host))
        throw new Error(t('连接已被删除，请刷新', 'Connection was already removed; refresh first'));
      const next = servers.filter((server) => server.host !== host);
      await invoke('remote_save_config', { config: { servers: next } });
      setServers(next);
      if (original === host) {
        setSelected('local');
        setEditor(null);
        setOriginal(null);
        setBaseline('');
        setPassword('');
      }
      setDeletingServer(null);
    });
  }
  async function checkConnection() {
    if (!editor) return;
    const server = structuredClone(editor),
      configuration = JSON.stringify(server);
    const result: ResourceCheck = {
      configuration,
      checked_at: new Date().toISOString(),
    };
    try {
      result.probe = await invokeJson<ResourceCheck['probe']>('remote_probe', { server });
      rememberPartitions(server.host, result.probe);
    } catch (error) {
      result.probe_error = String(error);
    }
    if (result.probe && !result.probe.configuration_error) {
      try {
        result.engines = await invokeJson<ResourceCheck['engines']>('remote_engines', { server });
      } catch (error) {
        result.engine_error = String(error);
      }
    }
    const summary = resourceCheckSummary(server, result, en);
    return {
      ...result,
      ok:
        [summary.ssh, summary.engine, summary.directory, summary.mapping].every(
          (item) => item.state === 'success',
        ) && !result.probe?.problems?.length,
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
  async function detectScheduler() {
    if (!editor) return;
    const server = structuredClone(editor),
      configuration = JSON.stringify(server),
      token = epoch.current;
    const probe = await invokeJson<ResourceCheck['probe']>('remote_probe', { server });
    rememberPartitions(server.host, probe);
    if (
      !probe?.hostname ||
      !Array.isArray(probe.schedulers) ||
      !probe.schedulers.every((name) => ['slurm', 'pbs', 'lsf'].includes(name))
    )
      throw new Error(
        t('服务器未返回有效的调度器识别结果。', 'Invalid scheduler detection response.'),
      );
    const schedulers = [...new Set(probe.schedulers)];
    const next = schedulers.length === 1 ? { ...server, scheduler: schedulers[0] } : server;
    if (token.active && token === epoch.current && next.scheduler !== server.scheduler) {
      setEditor((current) => (JSON.stringify(current) === configuration ? next : current));
      setOutput((current) => {
        const result = { ...current };
        delete result[`check:${selected}`];
        return result;
      });
    }
    return {
      configuration: JSON.stringify(next),
      schedulers,
      unverifiedQsub: probe.qsub_unverified === true,
    };
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
      delete next[`scheduler:${selected}`];
      return next;
    });
  }
  function rememberPartitions(host: string, probe: ResourceCheck['probe']) {
    const recognised = recognisedPreset(probe);
    if (recognised) setRecognisedByHost((current) => ({ ...current, [host]: recognised.id }));
    const partitions = probe?.partitions;
    if (Array.isArray(partitions) && partitions.length)
      setPartitionsByHost((current) => ({ ...current, [host]: partitions }));
  }
  function applyPreset(id: string) {
    const preset = SERVER_PRESETS.find((item) => item.id === id);
    setAppliedPreset(id);
    if (editor && preset) editServer(applyServerPreset(editor, preset));
  }
  function editField(key: keyof Server, value: string) {
    if (key === 'auth' && value !== 'password') setPassword('');
    if (editor)
      editServer({
        ...editor,
        [key]: typeof editor[key] === 'number' ? Number(value) : value,
        // 密钥文件只在 key 认证下有意义；切走时清掉，否则后端会把它带进 CLI，
        // CLI 的 Auth::validate 会以“非 key 模式不应有 identity_file”拒绝整条命令。
        ...(key === 'auth' && value !== 'key' ? { identity_file: '' } : {}),
      });
  }
  /** 清除本机默认路径上一次保存成功的提示；一改值它就不再成立。 */
  function clearLocalFeedback() {
    setOutput((current) => {
      const next = { ...current };
      delete next['local-data'];
      return next;
    });
  }
  function editLocalData(key: DataPathKey, value: string) {
    setLocalData((current) => ({ ...current, [key]: value }));
    clearLocalFeedback();
  }
  /** 放弃本机路径编辑：回到真实已保存值（不是回到上一次 setLocalData 的中间状态）。 */
  function discardLocalData() {
    if (busy || lock.current) return;
    setLocalData((current) => ({ ...current, ...localPathValues(savedLocal) }));
    setError('');
    clearLocalFeedback();
  }
  async function saveLocalData() {
    if (!configLoaded)
      throw new Error(
        t(
          '服务器配置尚未成功加载，请先刷新。',
          'Load the server configuration successfully before saving.',
        ),
      );
    // Remembered per-kind paths are kept by the backend; only the default folders are saved here.
    const local: DataDefaults = {
      project_dir: localData.project_dir.trim(),
      rawdata_dir: localData.rawdata_dir.trim(),
      runtime_dir: localData.runtime_dir.trim(),
      forcing_dir: localData.forcing_dir.trim(),
      ...(localData.kind_paths ? { kind_paths: localData.kind_paths } : {}),
    };
    await invoke('remote_save_config', { config: { servers, local } });
    // 保存成功才把输入改写成 trim 后的值并推进基线：失败时保留原输入、不显示成功（execute 会清 key）。
    setLocalData(local);
    setSavedLocal(local);
    return t('已保存本机默认数据路径', 'Saved local default data paths');
  }
  async function pickLocalData(key: DataPathKey) {
    const path = await invoke<string | null>('pick_folder', { key: `resource-${key}` });
    if (path) editLocalData(key, path);
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
      delete next[`scheduler:${selected}`];
      return next;
    });
    return t('密码已登记到本次会话', 'Password registered for this session');
  }
  const selectedServer = servers.find((server) => server.host === original);
  const detection = output[`scheduler:${selected}`] as
    { configuration: string; schedulers: string[]; unverifiedQsub: boolean } | undefined;
  const schedulerDetection =
    editor && detection?.configuration === JSON.stringify(editor) ? detection : null;
  const schedulerLabels: Record<string, string> = {
    auto: t('运行时自动识别', 'Detect at run time'),
    bare: t('直接后台运行', 'Direct background execution'),
    slurm: 'Slurm',
    pbs: 'PBS',
    lsf: 'LSF',
  };
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
  const dataFields: [DataPathKey, string][] = [
    ['project_dir', t('默认项目目录', 'Default project folder')],
    ['rawdata_dir', t('默认地表数据目录', 'Default surface data folder')],
    ['runtime_dir', t('默认 Runtime 数据目录', 'Default runtime data folder')],
    ['forcing_dir', t('默认强迫场数据路径', 'Default forcing data path')],
  ];
  const extraFields: [keyof Server, string][] = [
    ['port', t('端口（0 使用 SSH 默认）', 'Port (0 uses SSH default)')],
    ['threads', t('线程', 'Threads')],
    ['partition', t('分区 / 队列', 'Partition / queue')],
    ['account', t('账户', 'Account')],
    ['walltime', t('时限', 'Walltime')],
    [
      'cpus',
      t(
        '每个 MPI 进程申请的 CPU 数（只用于 Fortran 引擎；Rust 引擎按线程数申请）',
        'CPUs per MPI rank (Fortran engine only; the Rust engine requests its thread count)',
      ),
    ],
    ['memory_gb', t('内存 GB', 'Memory GB')],
    [
      'cores_per_node',
      t(
        '每节点核数（如摆渡船 48；填了以后，总核数超过它的 MPI 运行会自动算出占几个节点）',
        'Cores per node (e.g. 48; with it, an MPI run that needs more cores plans its own nodes)',
      ),
    ],
    [
      'nodes',
      t(
        '节点数（留空或 0 = 自动；需要手动指定占几个节点时才填）',
        'Nodes (empty or 0 = automatic; fill only to force how many nodes a run takes)',
      ),
    ],
    ['fetch_vars', t('取回变量', 'Fetch variables')],
    [
      'env_script',
      t(
        '环境脚本（先于服务器上的每条命令执行，如 module load、export PATH）',
        'Environment script (runs before every command on the server, e.g. module load, export PATH)',
      ),
    ],
  ];
  const guideFields: UiField[] = editor
    ? [
        ...basicFields,
        ['root', t('服务器工作目录', 'Server working root')] as [keyof Server, string],
        ...dataFields,
        ...(advanced ? extraFields : []),
      ].map(([key, label]) => ({
        id: `resource-${key}`,
        label,
        value: String(editor[key] ?? ''),
        type: typeof editor[key] === 'number' ? 'number' : 'text',
        set: (v) => editField(key, v),
      }))
    : [];
  const partitions = editor ? (partitionsByHost[editor.host] ?? []) : [];
  const partitionAt = guideFields.findIndex((item) => item.id === 'resource-partition');
  if (editor && partitions.length && partitionAt >= 0) {
    const chosen = editor.partition;
    const fallback = partitions.find((item) => item.default);
    guideFields[partitionAt] = {
      ...guideFields[partitionAt],
      type: 'choice',
      options: [
        {
          value: '',
          label: fallback
            ? t(`服务器默认（${fallback.name}）`, `Server default (${fallback.name})`)
            : t('服务器默认', 'Server default'),
        },
        ...partitions.map((item) => ({ value: item.name, label: partitionLabel(item, en) })),
        ...(chosen && !partitions.some((item) => item.name === chosen)
          ? [{ value: chosen, label: chosen }]
          : []),
      ],
      // 选了分区，"每节点核数"还没填时顺手填上（取分区里最小的节点），MPI 运行据此算占几个节点。
      set: (value) => {
        const picked =
          partitions.find((item) => item.name === value) ?? (value ? undefined : fallback);
        editServer({
          ...editor,
          partition: value,
          ...(picked && !editor.cores_per_node ? { cores_per_node: picked.cores_min } : {}),
        });
      },
    };
  }
  if (editor) {
    guideFields.push({
      id: 'resource-preset',
      label: t('集群预设', 'Cluster preset'),
      value: appliedPreset,
      type: 'choice',
      options: [
        { value: '', label: t('不使用预设', 'No preset') },
        ...SERVER_PRESETS.map((preset) => ({
          value: preset.id,
          label: en ? preset.en : preset.zh,
        })),
      ],
      set: (value) => applyPreset(value),
    });
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
    if (advanced && directories)
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
            label: schedulerLabels[value],
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
          // 与上面的"集群预设"原来同为 resource-preset：维护区渲染出的是集群预设下拉，编译配置永远
          // 停在 default，助手按 id 设置也报"不唯一"（GUI 审查 2026-10-22）。
          id: 'resource-build-preset',
          label: t('编译配置', 'Build configuration'),
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
  const kindLabels: Record<string, string> = {
    site: t('站点', 'Site'),
    watershed: t('流域', 'Watershed'),
    region: t('区域', 'Region'),
    global: t('全球', 'Global'),
  };
  const kindFieldLabels: Record<string, string> = {
    sitedir: t('站点数据目录', 'Site data folder'),
    forcingdir: t('站点强迫场目录', 'Site forcing folder'),
    shapefile: t('流域边界文件', 'Watershed boundary'),
    catchment: t('Catchment 文件', 'Catchment file'),
    mesh: t('网格文件', 'Mesh file'),
    spatial_forcingdir: t('强迫场数据路径', 'Forcing data path'),
    forcing_dataset: t('强迫场数据集', 'Forcing dataset'),
    forcing: t('强迫场配置文件', 'Forcing configuration'),
  };
  const kindPathHost = selected === 'local' ? null : (selectedServer?.host ?? null);
  const kindPaths = kindPathRows(
    selected === 'local' ? localData.kind_paths : selectedServer?.kind_paths,
  );
  /** 清除某类型记住的路径：只动本机保存的"上次用过的路径"，不删文件。 */
  async function forgetKindPath(kind: string, key: string) {
    await invoke('remote_forget_kind_path', { host: kindPathHost, kind, key });
    const forget = <T extends { kind_paths?: Record<string, Record<string, string>> }>(
      old: T,
    ): T => {
      const fields = { ...old.kind_paths?.[kind] };
      delete fields[key];
      const paths = { ...old.kind_paths, [kind]: fields };
      if (!Object.keys(fields).length) delete paths[kind];
      return { ...old, kind_paths: paths };
    };
    if (kindPathHost)
      setServers((old) =>
        old.map((server) => (server.host === kindPathHost ? forget(server) : server)),
      );
    else setLocalData((old) => forget(old));
    return t(
      `已清除 ${kindLabels[kind] ?? kind} · ${kindFieldLabels[key] ?? key}`,
      `Forgot ${kindLabels[kind] ?? kind} · ${kindFieldLabels[key] ?? key}`,
    );
  }
  const kindPathSection = (
    <section className="resource-kind-paths">
      <h3>{t('各类型上次使用的路径', 'Last paths by simulation type')}</h3>
      <p className="muted">
        {t(
          '新建模拟在这里选择相应类型时，自动填入空着的框。创建成功后更新；站点在读到站点时就更新。',
          'Filled into empty fields when a new simulation of that type runs here. Updated after a successful creation, and for sites as soon as sites are read.',
        )}
      </p>
      {kindPaths.length ? (
        <div className="kind-path-list">
          {kindPaths.map(({ kind, key, value }) => (
            <div className="kind-path-row" key={`${kind}:${key}`}>
              <span className="kind-path-kind">{kindLabels[kind] ?? kind}</span>
              <span className="kind-path-field">{kindFieldLabels[key] ?? key}</span>
              <code className="kind-path-value" title={value}>
                {value}
              </code>
              {/* 清除记住的路径属于"忘记连接设置"，只在界面上按，不登记给助手（D 级）。 */}
              <Button
                disabled={busy}
                onClick={() => run('kind-paths', () => forgetKindPath(kind, key))}
              >
                {t('清除', 'Forget')}
              </Button>
            </div>
          ))}
        </div>
      ) : (
        <p className="muted">{t('还没有记录。', 'Nothing remembered yet.')}</p>
      )}
      {output['kind-paths'] != null && <Notice>{String(output['kind-paths'])}</Notice>}
    </section>
  );
  if (selected === 'local') {
    guideFields.push(
      ...dataFields.map(([key, label]) => ({
        id: `resource-local-${key}`,
        label,
        value: localData[key],
        type: 'text' as const,
        set: (value: string) => editLocalData(key, value),
      })),
    );
  }
  usePageContract({
    title: t('计算资源', 'Resources'),
    step: 'resources',
    status: busy ? 'working' : error || 'ready',
    details: {
      selected,
      // 页面级的"有未保存修改"同时覆盖连接设置草稿与本机默认路径，截图/助手看到的是同一事实。
      dirty: dirty || localDirty,
      localDirty,
      check,
      schedulerDetection: advanced ? schedulerDetection : null,
      ...(checkDetails && checkEvidence ? { checkEvidence } : {}),
      local: selected === 'local' ? output.local : undefined,
    },
    fields: guideFields.map((field) => ({ ...field, disabled: busy })),
    // 动作由界面上的按钮自己登记（第 737 轮）：助手只看此刻真正显示着的按钮。
    actions: [],
  });
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
        {state.simulationReturn && (
          <ActionButton
            action="resource-back-simulation"
            label={t('返回模拟设置', 'Back to simulation setup')}
            disabled={busy || dirty || localDirty}
            run={() =>
              navigate(
                state.simulationReturn!.route === 'new-case' ? 'new-case' : 'case',
                'settings',
              )
            }
          />
        )}
        <ActionButton
          action="resource-new"
          label={t('添加服务器', 'Add server')}
          variant="primary"
          disabled={!configLoaded || busy || dirty || localDirty}
          run={addServer}
        />
      </header>
      {busy && <Notice>{t('操作进行中…', 'Operation in progress…')}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
      <Dialog
        open={deletingServer !== null}
        title={t('删除连接设置', 'Delete connection settings')}
        // 删除是 D 级，删除按钮本身不登记；只把"关闭"登记给助手，免得对话框开着时无路可走。
        closeAction="resource-delete-close"
        onOpenChange={(open) => {
          if (!open && !busy) setDeletingServer(null);
        }}
      >
        <p>{deletingServer?.host}</p>
        <p>
          {t(
            '仅从软件中删除此连接配置。服务器上的文件和任务会保留。',
            'Remove this connection from the app. Files and jobs on the server are retained.',
          )}
        </p>
        {error && <Notice tone="error">{error}</Notice>}
        <div className="actions">
          <Button disabled={busy} onClick={() => setDeletingServer(null)}>
            {t('取消', 'Cancel')}
          </Button>
          <Button
            variant="danger"
            disabled={!configLoaded || busy || dirty}
            onClick={() => {
              void deleteConnection().catch(() => {});
            }}
          >
            {t('删除连接设置', 'Delete connection settings')}
          </Button>
        </div>
      </Dialog>
      <div className="resources-selector">
        <ActionButton
          action="resource-select-local"
          label={t('本机', 'Local')}
          className={`resource-choice ${selected === 'local' ? 'selected' : ''}`}
          aria-pressed={selected === 'local'}
          disabled={busy || dirty || localDirty}
          run={() => selectResource(null)}
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
        </ActionButton>
        {servers.map((server) => (
          <ContextMenu
            key={server.host}
            items={[
              {
                label: t('删除连接设置', 'Delete connection settings'),
                danger: true,
                disabled: !configLoaded || busy || dirty,
                onSelect: () => {
                  if (configLoaded && !busy && !dirty) {
                    setError('');
                    setDeletingServer(server);
                  }
                },
              },
            ]}
          >
            <ActionButton
              action={`resource-select-server-${server.host}`}
              label={server.host}
              className={`resource-choice ${selected === `server:${server.host}` ? 'selected' : ''}`}
              aria-pressed={selected === `server:${server.host}`}
              disabled={busy || dirty || localDirty}
              run={() => selectResource(server)}
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
            </ActionButton>
          </ContextMenu>
        ))}
      </div>
      {selected === 'local' && (
        <Card title={t('本机计算环境', 'Local environment')}>
          <p>
            {navigator.hardwareConcurrency || 1} CPU · {t('编译配置：', 'Build configurations: ')}
            {state.kernels.map((kernel) => kernel.preset).join(', ') ||
              t('尚无 Fortran 编译配置', 'No Fortran build configuration')}
          </p>
          <ActionButton
            action="resource-local"
            label={t('检查本机后端与模型内核', 'Check local backend and model kernel')}
            disabled={busy}
            run={() => execute('local', checkLocal)}
          />
          {output.local != null && <Notice>{String(output.local)}</Notice>}
          <section className="resource-data-defaults">
            <h3>{t('默认数据路径', 'Default data paths')}</h3>
            <p className="muted">
              {t(
                '新建模拟选择本机运行时自动填入项目目录和数据路径；新建时改用的路径会同步回这里。',
                'Filled in when a new simulation runs locally; paths changed while creating one are synced back here.',
              )}
            </p>
            {dataFields.map(([key, label]) => (
              <div className="resource-identity" key={key}>
                {field(`resource-local-${key}`)}
                <ActionButton
                  action={`resource-local-pick-${key}`}
                  label={`${t('选择', 'Choose')} ${label}`}
                  disabled={busy}
                  run={() =>
                    pickLocalData(key).catch((e) => {
                      setError(String(e));
                      throw e;
                    })
                  }
                />
              </div>
            ))}
            <div className="actions resource-local-data-actions">
              <ActionButton
                action="resource-local-data-save"
                label={t('保存默认数据路径', 'Save default data paths')}
                kind="commit"
                variant="primary"
                disabled={busy || !configLoaded}
                run={() => execute('local-data', saveLocalData)}
              />
              {localDirty && (
                <>
                  <ActionButton
                    action="resource-local-data-discard"
                    label={t('放弃修改', 'Discard edits')}
                    disabled={busy}
                    run={discardLocalData}
                  />
                  <small>{t('有未保存的修改', 'Unsaved changes')}</small>
                </>
              )}
            </div>
            {output['local-data'] != null && <Notice>{String(output['local-data'])}</Notice>}
          </section>
          {kindPathSection}
          {localRunEntries(state.running).map(([dir, task]) => (
            <div className="resource-task-row" key={dir}>
              <span>
                {dir} · {task.status}
              </span>
              <ActionButton
                action={`resource-local-cancel-${dir}`}
                label={t('取消本机任务', 'Cancel local task')}
                kind="commit"
                disabled={busy}
                run={() => execute(dir, () => invoke('cancel_runs', { cases: [dir] }))}
              />
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
            {field('resource-preset')}
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
          {(() => {
            const applied = SERVER_PRESETS.find((preset) => preset.id === appliedPreset);
            const recognised = SERVER_PRESETS.find(
              (preset) => preset.id === recognisedByHost[editor.host],
            );
            return (
              <>
                {recognised && recognised.id !== appliedPreset && (
                  <Notice>
                    {t(
                      `这台服务器看起来是${recognised.zh}（按调度系统和分区认出，不看地址）。要套用这个预设吗？账号、地址和工作目录不会改。`,
                      `This server looks like ${recognised.en} (recognised by its scheduler and partitions, not its address). Apply that preset? Account, address and work root stay as they are.`,
                    )}{' '}
                    <ActionButton
                      action="resource-apply-preset"
                      label={t('套用预设', 'Apply preset')}
                      disabled={busy}
                      run={() => applyPreset(recognised.id)}
                    />
                  </Notice>
                )}
                {applied && (
                  <Notice>
                    <strong>{en ? applied.en : applied.zh}</strong>
                    <ul className="resource-preset-notes">
                      {applied.notes.map(([zh, english]) => (
                        <li key={zh}>{en ? english : zh}</li>
                      ))}
                    </ul>
                  </Notice>
                )}
              </>
            );
          })()}
          <div className="resource-wide-field">{field('resource-username')}</div>
          {editor.auth === 'key' && (
            <div className="resource-identity">
              {field('resource-identity')}
              <ActionButton
                action="resource-pick-identity"
                label={t('选择密钥文件', 'Choose identity file')}
                disabled={busy}
                run={() => execute('pick-identity', pickIdentity)}
              />
            </div>
          )}
          {editor.auth === 'key' && (
            <Notice>
              {t(
                '带口令的私钥请先用 ssh-add 加入 ssh-agent；应用不接收、也不保存私钥口令。',
                'For passphrase-protected keys, load the key into ssh-agent with ssh-add first; the app never asks for or stores the passphrase.',
              )}
            </Notice>
          )}
          {editor.auth === 'password' && (
            <>
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
              <Notice>
                {t(
                  '密码模式仅支持 SSH password 认证，不支持键盘交互或一次性动态口令；需要动态口令时请用 SSH config，并先在终端里建立连接。',
                  'Password mode supports SSH password authentication only; it cannot answer keyboard-interactive or one-time-code prompts. For OTP, use SSH config with an existing terminal connection.',
                )}
              </Notice>
            </>
          )}
          <div className="resource-identity">
            {field('resource-root')}
            <ActionButton
              action="resource-browse-root"
              label={t('浏览服务器', 'Browse server')}
              disabled={busy || !editor.host}
              run={() => setRootBrowser(structuredClone(editor))}
            />
          </div>
          <section className="resource-data-defaults">
            <h3>{t('默认数据路径', 'Default data paths')}</h3>
            <p className="muted">
              {t(
                '新建模拟选择这台服务器时自动填入；新建时改用的路径会同步回这里。填服务器上的绝对路径。',
                'Filled in when a new simulation runs on this server; paths changed while creating one are synced back here. Use absolute server paths.',
              )}
            </p>
            <div className="form-grid">{dataFields.map(([key]) => field(`resource-${key}`))}</div>
          </section>
          {selectedServer && kindPathSection}
          {/* 已保存的服务器才读：新填的地址还没连通过，读负载只会报错。 */}
          {selectedServer && (
            <LiveServerLoad
              host={selectedServer.host}
              partition={selectedServer.partition}
              en={en}
            />
          )}
          <section className="resource-checks">
            <h3>{t('连接与环境检查', 'Connection and environment')}</h3>
            <div className="resource-status-grid">
              {check &&
                (
                  [
                    ['ssh', t('SSH 连接', 'SSH connection')],
                    ['engine', t('Rust 引擎', 'Rust engine')],
                    ['directory', t('工作目录', 'Working directory')],
                    ['mapping', t('输入目录映射（可选）', 'Input directory mapping (optional)')],
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
            <div className="actions resource-connection-actions">
              <ActionButton
                action="resource-check"
                label={t('检查连接', 'Check connection')}
                variant={dirty ? 'secondary' : 'primary'}
                disabled={busy || !editor.host}
                run={() => execute(`check:${selected}`, checkConnection)}
              />
              {dirty && (
                <>
                  <ActionButton
                    action="resource-save"
                    label={t('保存连接设置', 'Save connection')}
                    kind="commit"
                    variant="primary"
                    disabled={!configLoaded || busy || !!password}
                    run={() => execute('save', save)}
                  />
                  <ActionButton
                    action="resource-discard"
                    label={t('放弃修改', 'Discard edits')}
                    disabled={busy}
                    run={discard}
                  />
                  <small>{t('有未保存的修改', 'Unsaved changes')}</small>
                </>
              )}
              {['save', 'credentials']
                .filter((key) => output[key] != null)
                .map((key) => (
                  <small key={key}>{String(output[key])}</small>
                ))}
            </div>
            {check?.checkedAt &&
              [
                checkResult?.probe_error,
                checkResult?.probe?.configuration_error,
                checkResult?.engine_error,
                ...(checkResult?.probe?.problems ?? []),
              ]
                .filter(Boolean)
                .map((message, index) => (
                  <Notice key={index} tone="warning">
                    {message}
                  </Notice>
                ))}
            {checkEvidence && (
              <Advanced
                action="resource-check-details"
                actionLabel={t('检查详情', 'Check details')}
                title={t('检查详情', 'Check details')}
                open={checkDetails}
                onOpenChange={setCheckDetails}
              >
                {checkDetails && <pre>{JSON.stringify(checkEvidence, null, 2)}</pre>}
              </Advanced>
            )}
          </section>
          <Advanced
            action="resource-advanced-toggle"
            actionLabel={t('高级设置', 'Advanced')}
            open={advanced}
            onOpenChange={setAdvanced}
          >
            <ActionButton
              action="resource-directories-toggle"
              label={t('本地路径映射（可选）', 'Local path mapping (optional)')}
              run={() => setDirectories(!directories)}
            />
            {advanced && directories && (
              <section className="resource-directories">
                <h3>{t('本地路径映射（可选）', 'Local path mapping (optional)')}</h3>
                <p className="muted">
                  {t(
                    '用于把已有本地算例拿到服务器运行。例如，算例里写的是 /Users/.../forcing，服务器上的同一份数据位于 /tera11/.../forcing：将这两个目录配成一组映射，运行时就会把本地路径替换为对应的服务器路径。数据需要已存在于服务器上，映射不会复制或上传数据。',
                    'Use this to run an existing local case on a server. For example, if the case uses /Users/.../forcing and the same data is already at /tera11/.../forcing on the server, pair these directories to replace the local path at run time. Data must already exist on the server; mapping does not copy or upload it.',
                  )}
                </p>
                <p className="muted">
                  {t(
                    '如果直接在服务器创建算例，可在“数据与时段”中浏览选择服务器输入数据，无需设置映射。',
                    'When creating a case directly on the server, browse server inputs in Data and period; no mapping is needed.',
                  )}
                </p>
                {editor.maps.map((_, i) => (
                  <div className="resource-map" key={i}>
                    {field(`resource-map-${i}-local`)}
                    {field(`resource-map-${i}-remote`)}
                    <ActionButton
                      action={`resource-remove-map-${i}`}
                      label={t('移除映射', 'Remove mapping')}
                      disabled={busy}
                      run={() =>
                        editServer({ ...editor, maps: editor.maps.filter((_, n) => n !== i) })
                      }
                    />
                  </div>
                ))}
                <ActionButton
                  action="resource-add-map"
                  label={t('添加映射', 'Add mapping')}
                  disabled={busy}
                  run={() =>
                    editServer({ ...editor, maps: [...editor.maps, { local: '', remote: '' }] })
                  }
                />
                <p className="muted">
                  {t(
                    '路径存在不代表模型输入已完整；运行前仍需输入检查。',
                    'Existing paths do not prove complete model inputs; check inputs before running.',
                  )}
                </p>
              </section>
            )}
            <div className="form-grid">
              {extraFields.map(([key]) => field(`resource-${key}`))}
              <div>
                <div className="resource-scheduler">
                  {field('resource-scheduler')}
                  <ActionButton
                    action="resource-detect-scheduler"
                    label={t('自动识别', 'Detect automatically')}
                    disabled={busy || !editor.host.trim() || !!password}
                    run={() => execute(`scheduler:${selected}`, detectScheduler)}
                  />
                </div>
                <p className="muted">
                  {t(
                    '通过当前 SSH 连接识别调度器，不会提交任务；识别结果保存后生效。',
                    'Detect through this SSH connection without submitting a job; save to apply.',
                  )}
                </p>
                {schedulerDetection && (
                  <p role="status">
                    {schedulerDetection.schedulers.length === 1
                      ? t(
                          `已识别为 ${schedulerLabels[schedulerDetection.schedulers[0]]}。${dirty ? '请保存连接设置。' : '与当前设置一致。'}`,
                          `Detected ${schedulerLabels[schedulerDetection.schedulers[0]]}. ${dirty ? 'Save the connection settings.' : 'Matches the current setting.'}`,
                        )
                      : schedulerDetection.schedulers.length > 1
                        ? t(
                            `检测到多个调度器：${schedulerDetection.schedulers.map((name) => schedulerLabels[name]).join('、')}。已保留当前选择，请手动确认。`,
                            `Multiple schedulers detected: ${schedulerDetection.schedulers.map((name) => schedulerLabels[name]).join(', ')}. Current selection retained; choose one manually.`,
                          )
                        : schedulerDetection.unverifiedQsub
                          ? t(
                              '发现 qsub，但无法确认所属调度系统。已保留当前选择，请确认服务器使用的系统后手动选择。',
                              'Found qsub, but could not verify its scheduler. Current selection retained; confirm the server scheduler and choose it manually.',
                            )
                          : t(
                              '未检测到 Slurm、PBS 或 LSF，已保留当前选择。集群可能需要先加载调度工具；普通服务器可选择直接后台运行。',
                              'No Slurm, PBS or LSF detected; current selection retained. Cluster tools may need loading; ordinary servers can use direct background execution.',
                            )}
                  </p>
                )}
              </div>
              {field('resource-directives')}
            </div>
          </Advanced>
          {selectedServer && (
            <Advanced
              action="resource-maintenance-toggle"
              actionLabel={t('编译、远程检查与维护', 'Build, remote checks and maintenance')}
              title={t('编译、远程检查与维护', 'Build, remote checks and maintenance')}
              open={maintenance}
              onOpenChange={setMaintenance}
            >
              <div className="form-grid">
                {[
                  'build-preset',
                  'workspace',
                  'workspace-action',
                  'remote-case',
                  'test-package',
                  'change-kind',
                  'job',
                ].map((key) => field(`resource-${key}`))}
              </div>
              <div className="actions">
                <ActionButton
                  action="resource-kernels"
                  label={t('Fortran 编译配置', 'Fortran build configurations')}
                  disabled={busy || dirty}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('remote_kernels', { host: selectedServer.host }),
                    )
                  }
                />
                <ActionButton
                  action="resource-build"
                  label={t('编译 Fortran 引擎', 'Build Fortran engine')}
                  kind="commit"
                  disabled={busy || dirty || !preset}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('remote_build_kernel', {
                        host: selectedServer.host,
                        preset,
                        modelVersion: state.version?.code.workspace ?? null,
                      }),
                    )
                  }
                />
                <ActionButton
                  action="resource-workspace-submit"
                  label={t('提交远程检查', 'Submit remote check')}
                  kind="commit"
                  disabled={busy || dirty || !workspace}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('workspace_remote', {
                        name: workspace,
                        operation: 'submit',
                        host: selectedServer.host,
                        request: remoteCheckRequest(
                          action,
                          preset,
                          remoteCase,
                          packageName,
                          changeKind,
                        ),
                      }),
                    )
                  }
                />
                <ActionButton
                  action="resource-workspace-list"
                  label="list"
                  disabled={busy || dirty || !workspace}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('workspace_remote', {
                        name: workspace,
                        operation: 'list',
                        job: job || null,
                      }),
                    )
                  }
                />
                <ActionButton
                  action="resource-workspace-status"
                  label="status"
                  disabled={busy || dirty || !workspace}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('workspace_remote', {
                        name: workspace,
                        operation: 'status',
                        job: job || null,
                      }),
                    )
                  }
                />
                <ActionButton
                  action="resource-workspace-fetch"
                  label="fetch"
                  kind="commit"
                  disabled={busy || dirty || !workspace || !job}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('workspace_remote', {
                        name: workspace,
                        operation: 'fetch',
                        job: job || null,
                      }),
                    )
                  }
                />
                <ActionButton
                  action="resource-workspace-cancel"
                  label="cancel"
                  kind="commit"
                  disabled={busy || dirty || !workspace || !job}
                  run={() =>
                    execute(`maintenance:${selected}`, () =>
                      invokeJson('workspace_remote', {
                        name: workspace,
                        operation: 'cancel',
                        job: job || null,
                      }),
                    )
                  }
                />
              </div>
              {/* 核对租约、清理快照会动服务器上的租约/快照：D 级，只在界面上按，不登记。 */}
              <Button
                disabled={busy || dirty}
                onClick={async () => {
                  if (
                    await askConfirm(
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
                  onClick={async () => {
                    if (
                      await askConfirm(
                        t(
                          '清理该快照？后端会保护活动租约。',
                          'Clean this snapshot? Active leases are protected by the backend.',
                        ),
                        { danger: true },
                      )
                    )
                      run(`maintenance:${selected}`, () =>
                        invokeJson('remote_engine_clean', { server: selectedServer, snapshot }),
                      );
                  }}
                >
                  {t('清理快照', 'Clean snapshot')}
                </Button>
              </div>
              {output[`maintenance:${selected}`] != null && (
                <pre>{JSON.stringify(output[`maintenance:${selected}`], null, 2)}</pre>
              )}
            </Advanced>
          )}
        </Card>
      )}
      {rootBrowser && (
        <RemotePathBrowser
          host={rootBrowser.host}
          server={rootBrowser}
          initialPath={rootBrowser.root.startsWith('/') ? rootBrowser.root : undefined}
          directory
          en={en}
          onSelect={(path) => {
            if (JSON.stringify(editor) === JSON.stringify(rootBrowser)) editField('root', path);
            else
              setError(
                t(
                  '连接设置已改变，请重新选择目录。',
                  'Connection settings changed; choose the folder again.',
                ),
              );
            setRootBrowser(null);
          }}
          onClose={() => setRootBrowser(null)}
        />
      )}
      <footer className="resources-footer">
        <ActionButton
          action="resource-refresh"
          label={t('刷新服务器配置', 'Refresh server settings')}
          disabled={busy || dirty || localDirty}
          run={() => execute('refresh', refresh)}
        />
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
