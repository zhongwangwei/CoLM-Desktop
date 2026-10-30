import { useEffect, useMemo, useRef, useState } from 'react';
import { useApp } from '../../lib/app-state';
import { useBenchTasks } from '../../lib/bench-task-context';
import { usePageContract } from '../../lib/guide';
import { invoke, invokeJson, listen } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { Advanced, Button, Card, Dialog, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { SaveBadge } from '../../components/SaveBadge';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import {
  configFingerprint,
  effectiveSummary,
  freezeCompareSnapshot,
  parseOpenbenchConfig,
  snapshotStale,
  structuralProblems,
  type CompareSnapshot,
} from './config';
import {
  benchErrorText,
  benchLogText,
  buildOpenbenchConfig,
  COMPARISON_ITEM_CHOICES,
  DEFAULT_COMPARISON_ITEMS,
  STATISTICS_ITEMS,
  statisticsMethodWarnings,
  statisticsReferenceWarning,
  statisticsStationWarning,
  comparableVariables,
  commonYears,
  comparisonDir,
  compareDraftHasContent,
  compareDraftKey,
  comparisonName,
  configInvalidation,
  configSaveState,
  defaultGridRes,
  designFingerprint,
  evaluationSummary,
  inspectBlockerText,
  inspectFamilyNote,
  inspectWarningText,
  jobContacted,
  layerVariableWarnings,
  parseCompareDraft,
  type CompareDraft,
  type CompareDraftPick,
  jobEnded,
  jobUncertain,
  parseReferences,
  parseRemoteJob,
  remoteJobTerminal,
  simulationFormForPick,
  simulationInfoForSave,
  validateSimulation,
  type BenchReference,
  type ComparisonDesign,
  type EvaluationDesign,
  type EvaluationSummaryItem,
  type HistoryInfo,
  type RemoteJobIdentity,
  type SimulationField,
  type SimulationFormValues,
  type SimulationIssue,
} from './compare';

type ProjectRun = {
  run_id: string;
  state: 'running' | 'succeeded' | 'failed';
  report: string | null;
  finished_at?: string | null;
  created_at?: string | null;
  config_sha?: string | null;
  error?: string | null;
};
type Project = {
  name: string;
  dir?: string;
  config?: string | null;
  report: string | null;
  /** 最近一次运行的真实身份与结果；失败时 error 是本次运行的失败原因。 */
  latest_run?: ProjectRun | null;
  /** 最近一次成功运行（只有成功才更新）：报告取回/打开都以它为准。 */
  latest_success?: {
    run_id: string;
    config_sha: string;
    report: string | null;
    finished_at: string | null;
  } | null;
  /** 最近若干次运行（新→旧）：历史运行列表/查看旧成功报告用；旧项目没有 runs/ 时为空。 */
  runs?: ProjectRun[];
};
/** 本机（`bench_status`）与服务器（`bench_remote_status`）的状态；服务器那份没有 uv 的路径，多了平台与下载工具。 */
type Status = {
  root: string;
  uv?: string | null;
  uv_own?: boolean;
  platform?: string | null;
  downloader?: string | null;
  checksum?: string | null;
  uv_version: string | null;
  uv_pinned: string;
  openbench_pinned: string;
  python: string | null;
  openbench: string | null;
  projects: Project[];
};
type Pick = {
  dir: string;
  label: string;
  info: HistoryInfo | null;
  error?: string;
  /** 探测警告（时间分辨率是推断的、缺坐标、文件族不参与）：不阻断，但要在列表里看得到。 */
  note?: string;
  /** 新模块的模拟变量覆盖（per-simulation variables）。 */
  variables?: Record<string, { varname: string; varunit: string }>;
  dataGroupby?: string | null;
  gridRes?: number | null;
  /** OpenBench 模型 profile（缺省 CoLM2024）。 */
  profile?: string | null;
  /** 单点模拟的站点 ID 覆盖（空 = 从 history 前缀推导）。 */
  stationId?: string | null;
};
/**
 * 登记/编辑模拟的表单。保存相关字段就是 `SimulationFormValues`；`probe` 保存完整探测结果，
 * 保存时只覆盖用户明确编辑的字段（GUI-10），`probeDir` 记住它属于哪个目录。
 */
type SimForm = SimulationFormValues & {
  label: string;
  suffix: string;
  probe: HistoryInfo | null;
  probeDir: string;
};
type RegisterDialog = 'variable' | 'reference' | 'simulation' | null;
type MetricOption = { name: string; label: string };
type MetricOptions = { metrics: MetricOption[]; scores: MetricOption[] };
// OpenBench 未装好或读取失败时的常用兜底；装好后以已安装 OpenBench 的 registry 为准。
const FALLBACK_METRICS: MetricOption[] = [
  { name: 'bias', label: 'Bias (BIAS)' },
  { name: 'RMSE', label: 'Root Mean Squared Error (RMSE)' },
  { name: 'correlation', label: 'Correlation Coefficient (r)' },
  { name: 'NSE', label: 'Nash–Sutcliffe Efficiency (NSE)' },
  { name: 'KGE', label: 'Kling–Gupta Efficiency (KGE)' },
  { name: 'ubRMSE', label: 'Unbiased Root Mean Squared Error (ubRMSE)' },
  { name: 'NRMSE', label: 'Normalized Root Mean Squared Error (NRMSE)' },
  { name: 'rSpearman', label: 'Spearman Rank Correlation (rSpearman)' },
];
const FALLBACK_SCORES: MetricOption[] = [
  { name: 'Overall_Score', label: 'Overall Score (OS)' },
  { name: 'nBiasScore', label: 'Normalized Bias Score (nBiasScore)' },
  { name: 'nRMSEScore', label: 'Normalized RMSE Score (nRMSEScore)' },
  { name: 'nSpatialScore', label: 'Normalized Spatial Score (nSpatialScore)' },
];
type RemoteJob = {
  job: string;
  status: { state: string; exit_code?: number; log_tail: string; detail?: string | null };
  ended: string | null;
  /** 调度系统明确确认作业不会再写（退出码/结果可能还没经 NFS 可见）。 */
  terminal?: boolean;
  /** 已向调度器发出取消且它收下了，但作业可能仍 queued/running。 */
  cancel_requested?: boolean;
  /** 进程/队列已不在且没有退出码：确认终止但结果未知。 */
  result_unknown?: boolean;
};
/**
 * 正在服务器上跑的那个作业：完整身份记在本机（主机、比较项目、提交时间、最后确认状态），
 * 关掉页面、关掉应用再回来都能接着看；一次查询失败也不删它（GUI-07）。
 */
type ActiveJob = RemoteJobIdentity;
/**
 * 一次远程作业跟踪的结果。只有 `succeeded/failed/cancelled` 是确认过的终态；
 * `uncertain` 是暂时联系不上（身份保留、等待核对），`detached` 表示用户换看了别的机器（身份仍在，
 * 回到那台机器时再接着核对）。
 */
type WatchOutcome =
  | { state: 'succeeded' | 'cancelled'; job: ActiveJob }
  | { state: 'failed'; job: ActiveJob; error: string }
  /** 确认已终止，但没有退出码/结果：不能继续等，也不能当成功/失败；允许关闭并重新提交。 */
  | { state: 'unknown'; job: ActiveJob; reason: string }
  | { state: 'uncertain'; job: ActiveJob; reason: string }
  | { state: 'detached' };

/** 常用的 PyPI 镜像。国内服务器直连 PyPI 常常很慢（7920 实测十分钟装不完，清华镜像 85 秒）。 */
const INDEXES = [
  { value: '', zh: '官方 PyPI', en: 'Official PyPI' },
  { value: 'https://pypi.tuna.tsinghua.edu.cn/simple', zh: '清华镜像', en: 'Tsinghua mirror' },
  { value: 'https://mirrors.aliyun.com/pypi/simple', zh: '阿里云镜像', en: 'Aliyun mirror' },
];

/** 探测结果里只属于那个目录的字段：目录一改就得清空，不能把旧目录的前缀/年份/类型/范围写进新目录（GUI-10）。 */
const CLEAR_PROBED_SIM_FIELDS = {
  dataType: 'auto' as const,
  prefix: '',
  timRes: 'Day',
  from: '',
  to: '',
  dataGroupby: '',
  gridRes: '',
  lon: '',
  lat: '',
  extent: '',
};

/** 新登记模拟的空表单（与编辑已有模拟共用同一套字段）。 */
const emptySimForm = (): SimForm => ({
  dir: '',
  label: '',
  dataType: 'auto',
  prefix: '',
  suffix: '',
  timRes: 'Day',
  from: '',
  to: '',
  dataGroupby: '',
  gridRes: '',
  profile: 'CoLM2024',
  stationId: '',
  lon: '',
  lat: '',
  extent: '',
  variables: [],
  probe: null,
  probeDir: '',
});

const stored = (key: string) => {
  try {
    return globalThis.localStorage?.getItem(key) ?? '';
  } catch {
    return '';
  }
};
const store = (key: string, value: string) => {
  try {
    if (value) globalThis.localStorage?.setItem(key, value);
    else globalThis.localStorage?.removeItem(key);
  } catch {
    /* 存不下就只在这次会话里用 */
  }
};
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

/**
 * "比较"：用 OpenBench 对几个模拟（不同项目、不同模型版本）做评估，看报告。
 * 环境按需逐级装在应用数据目录里（uv → 专用 Python → OpenBench 与全部依赖），不碰系统。
 * 也可以选一台服务器：环境、登记表、history 与评估都在那边，只把报告和指标取回来（格点算例的 history
 * 往往就在服务器上，拉回本机评估不现实）。
 */
export default function ComparePage() {
  const { state } = useApp();
  // 跨路由可见的比较任务（UX ③）：页面负责真实提交/等待，任务条只镜像阶段与结果。
  const benchTasks = useBenchTasks();
  const benchTaskId = useRef('');
  const en = state.language === 'en';
  const t = (zh: string, english: string) => (en ? english : zh);
  const shown = (e: unknown) => {
    const [zh, english] = benchErrorText(String(e));
    return t(zh, english);
  };
  // 在哪里评估：空是本机，否则是"计算资源"里配置的一台服务器。按机器记住。
  const [host, setHostState] = useState(() => stored('colm.benchHost'));
  const [servers, setServers] = useState<string[]>([]);
  // 服务器上挑目录：给哪一项挑（模拟的 history，或参考数据根目录）。
  const [browsing, setBrowsing] = useState<
    'sim' | 'ref' | 'simRegister' | 'refDataDir' | 'refList' | null
  >(null);
  const [index, setIndexState] = useState(() => stored('colm.benchIndex'));
  const setIndex = (value: string) => {
    setIndexState(value);
    store('colm.benchIndex', value);
  };
  const [status, setStatus] = useState<Status | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  // U-05：运行/生成配置后的成功提示记着"针对哪一版设计"，设置一改就清掉，避免旧的"评估完成"继续挂着。
  const [runNotice, setRunNotice] = useState('');
  // U-05：评估配置正在提交（后端写 openbench.yaml）时徽标显示"正在写入"。
  const [configSaving, setConfigSaving] = useState(false);
  // 配置真正写进比较项目目录的那一版指纹：只有提交成功才记。徽标据此区分"页内已生成"与"已写盘"。
  const [writtenFingerprint, setWrittenFingerprint] = useState('');
  const [log, setLog] = useState<string[]>([]);
  const [source, setSource] = useState<'pypi' | 'local'>('pypi');
  const [localSource, setLocalSource] = useState('');
  const [references, setReferences] = useState<BenchReference[]>([]);
  const [picks, setPicks] = useState<Pick[]>([]);
  const [name, setName] = useState('comparison');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [timRes, setTimRes] = useState('Day');
  // 格点比较的网格分辨率（度）：加了格点模拟时按 defaultGridRes 预填，可以改。
  const [gridRes, setGridRes] = useState('');
  const [chosen, setChosen] = useState<Record<string, string[]>>({});
  const [variableQuery, setVariableQuery] = useState('');
  const [yaml, setYaml] = useState('');
  // OpenBench 项目级选项；空串表示用 OpenBench 默认值。
  const [weight, setWeight] = useState('');
  const [minYear, setMinYear] = useState('1');
  const [timeAlignment, setTimeAlignment] = useState('');
  const [numCores, setNumCores] = useState('');
  const [metrics, setMetrics] = useState<string[]>(['bias', 'RMSE', 'correlation']);
  const [scores, setScores] = useState<string[]>(['Overall_Score']);
  // OpenBench 的跨模拟比较阶段（comparison.enabled）：默认用 OpenBench 自己的两个默认图表；
  // 一个都不勾表示不启用。写进配置后由 OpenBench 生成 comparisons/ 下的图。
  const [comparisonItems, setComparisonItems] = useState<string[]>([...DEFAULT_COMPARISON_ITEMS]);
  // OpenBench 独立统计检验阶段（statistics）：默认不启用，勾选方法后才写进配置。
  const [statisticsItems, setStatisticsItems] = useState<string[]>([]);
  // 上次离开页面时留下的草稿：只在打开页面时提示恢复（显式按钮），绝不自动覆盖当前页面。
  // 草稿只含页面输入，不含运行状态/报告；按机器分开存。
  const [draftOffer, setDraftOffer] = useState<CompareDraft | null>(() => {
    const draft = parseCompareDraft(stored(compareDraftKey(stored('colm.benchHost'))));
    return draft && compareDraftHasContent(draft) ? draft : null;
  });
  const [metricOptions, setMetricOptions] = useState<MetricOption[]>(FALLBACK_METRICS);
  const [scoreOptions, setScoreOptions] = useState<MetricOption[]>(FALLBACK_SCORES);
  const [dialog, setDialog] = useState<RegisterDialog>(null);
  const [modelVar, setModelVar] = useState({
    model: 'CoLM2024',
    variable: '',
    varname: '',
    unit: '',
    description: '',
  });
  const [simForm, setSimForm] = useState<SimForm>(emptySimForm);
  const [simEditing, setSimEditing] = useState<string | null>(null);
  // 模拟登记弹窗里的字段错误：同步抛出的错误原来只进控制台（GUI-11），现在弹窗里看得到并聚焦字段。
  const [simError, setSimError] = useState<{ field: SimulationField; message: string } | null>(
    null,
  );
  // 本机最近一次运行的身份（后端 run 快照）：完成提示与报告显示都绑定它，而不是"目录里有一份报告"。
  const [lastRun, setLastRun] = useState<{
    runId: string;
    configSha: string;
    fingerprint: string;
    at: string;
    report: string | null;
  } | null>(null);
  // 生成配置时读不出区域范围的格点模拟：只写目标分辨率，页面说明原因（GUI-10）。
  const [missingExtent, setMissingExtent] = useState<string[]>([]);
  const [report, setReport] = useState<{ path: string; html: string } | null>(null);
  // 参考数据根目录：扫描用它，运行时作为 OPENBENCH_REF_ROOT 交给 OpenBench（内置登记表的路径都相对它）。
  // 按机器记住。
  // 服务器上的参考数据在服务器的路径下：每台机器各记一份。
  const refKey = (machine: string) =>
    machine ? `colm.openbenchRefRoot@${machine}` : 'colm.openbenchRefRoot';
  const [scanDir, setScanDirState] = useState(() => stored(refKey(host)));
  const setScanDir = (value: string) => {
    setScanDirState(value);
    store(refKey(host), value);
  };
  const [scanText, setScanText] = useState('');
  const [reg, setReg] = useState({
    name: '',
    rootDir: '',
    dataType: 'stn',
    timRes: 'Day',
    from: '',
    to: '',
    fulllist: '',
    variables: '',
  });
  const lock = useRef(false);
  const job = useRef<ActiveJob | null>(null);
  // 点了"停止"：服务器上先结束进程、后写下"已取消"，中间那一下查到的是"找不到结果"，不能报成失败。
  const stopping = useRef(false);
  // 页面关掉时停止轮询：作业记录留着，回到页面时由新的页面接着等（否则两个轮询同时跑、结果取两遍）。
  const alive = useRef(true);
  // GUI-05：当前浏览的机器与请求代次。异步响应回来先核对，旧机器/旧代次的只丢弃，不写状态、登记表、
  // 错误或忙碌标记。机器 ref 在渲染时同步，关闭页面/切换机器的窗口期里到达的响应也能被判为过期。
  const hostRef = useRef(host);
  hostRef.current = host;
  const hostEpoch = useRef(0);
  const stillCurrent = (machine: string, epoch: number) =>
    hostRef.current === machine && hostEpoch.current === epoch;
  // GUI-07：正在等待的作业、暂时失联/待核对的作业、已确认终态的历史作业。三者分开：
  // 浏览别的机器不会打断正在跑的跟踪，失联也不会把作业身份从本机删掉。
  const [activeJob, setActiveJob] = useState<ActiveJob | null>(null);
  const [unsettled, setUnsettled] = useState<{ job: ActiveJob; reason: string } | null>(null);
  const [lastJob, setLastJob] = useState<ActiveJob | null>(null);
  // GUI-08：正在取回的报告（机器 + 比较名）；同一份取回期间不重复触发。
  const fetching = useRef<Set<string>>(new Set());
  // 生成配置时冻结"设计 + 站点清单 + 文本 + 指纹"成同一份快照；摘要/指纹/运行都读它。
  // 配置文本本身不冻结（手改后运行时重新解析），但站点清单与设计不能再读当前表单（审查报告 R01）。
  const generated = useRef<CompareSnapshot | null>(null);
  // 运行完成提示对应的有效配置指纹：设置或配置文本一改提示就过期（U-05）。
  const runNoticeFor = useRef('');
  // 上一次自动预填的网格分辨率：只有输入框还是这个值（或空着）时才允许被新的默认值覆盖，
  // 用户手填的值不被"新增模拟"悄悄改掉（审查报告：手工网格设置被再次应用默认值）。
  const autoGridRes = useRef('');
  const LEFT = 'compare page left';
  const remote = !!host;
  const jobKey = (machine: string) => `colm.benchJob@${machine}`;
  const jobHistoryKey = (machine: string) => `colm.benchLastJob@${machine}`;

  // 读当前机器的状态与登记表。`quiet` 用于任务结束后的后台刷新：失败不覆盖用户正在看的错误提示（GUI-05/08）。
  const refresh = async (
    machine = host,
    epoch = hostEpoch.current,
    quiet = false,
  ): Promise<Status | null> => {
    if (!stillCurrent(machine, epoch)) return null;
    let next: Status;
    try {
      next = await invokeJson<Status>(
        machine ? 'bench_remote_status' : 'bench_status',
        machine ? { host: machine } : undefined,
      );
    } catch (e) {
      if (!quiet && stillCurrent(machine, epoch)) setError(shown(e));
      return null;
    }
    // 旧机器的响应回来时，页面已经换成别的机器了：状态、登记表、错误都不写。
    if (!stillCurrent(machine, epoch)) return null;
    setStatus(next);
    if (!next.openbench) {
      setReferences([]);
      return next;
    }
    try {
      const list = await invokeJson(
        machine ? 'bench_remote_references' : 'bench_references',
        machine ? { host: machine } : undefined,
      );
      if (stillCurrent(machine, epoch)) setReferences(parseReferences(list));
    } catch (e) {
      // 读不到登记表要说出来：原来吞掉错误、显示"0 个"，看不出是哪里坏了（2026-10-23 窗口实测）。
      if (stillCurrent(machine, epoch)) {
        setReferences([]);
        if (!quiet) setError(shown(e));
      }
    }
    return next;
  };
  // 草稿自动保存：只存页面输入（不含运行状态/报告）。有"待恢复"提示时先不写，避免把用户还没决定
  // 恢复的旧草稿覆盖掉；提交运行期间也不写，避免把正在写的配置当成草稿。
  useEffect(() => {
    if (busy || draftOffer) return;
    const draft: CompareDraft = {
      version: 1,
      savedAt: new Date().toISOString(),
      host,
      name,
      from,
      to,
      timRes,
      gridRes,
      chosen,
      weight,
      minYear,
      timeAlignment,
      numCores,
      metrics,
      scores,
      comparisonItems,
      statisticsItems,
      yaml,
      picks: picks as CompareDraftPick[],
    };
    store(compareDraftKey(host), compareDraftHasContent(draft) ? JSON.stringify(draft) : '');
  }, [
    busy,
    draftOffer,
    host,
    picks,
    name,
    from,
    to,
    timRes,
    gridRes,
    chosen,
    weight,
    minYear,
    timeAlignment,
    numCores,
    metrics,
    scores,
    comparisonItems,
    statisticsItems,
    yaml,
  ]);
  useEffect(() => {
    void invokeJson<{ servers: { host: string }[] }>('remote_config')
      .then((config) => setServers(config.servers.map((s) => s.host)))
      .catch(() => setServers([]));
    let stop: (() => void) | undefined;
    void listen<{ stage?: string; line?: string }>('bench://log', (payload) =>
      setLog((old) => [...old.slice(-299), `[${payload.stage ?? ''}] ${payload.line ?? ''}`]),
    ).then((unlisten) => (stop = unlisten));
    return () => stop?.();
    // 只在打开页面时订阅一次日志、读一次服务器列表；状态随所选机器读（下面的 effect）。
  }, []);

  // 一次只做一件事。后端的固定句子换成当前语言；点"停止"结束的不是错误，作为提示显示（复验 C2-06）。
  const task = async (fn: () => Promise<unknown>, done?: string) => {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      await fn();
      if (done) setNotice(done);
    } catch (e) {
      if (String(e).includes(LEFT)) return;
      if (benchErrorText(String(e))[0] === '已停止') setNotice(t('已停止', 'Stopped'));
      else setError(shown(e));
    } finally {
      lock.current = false;
      setBusy(false);
      // 后台刷新用 quiet：本次操作已经给出了错误/提示，不能被刷新失败盖掉（GUI-05/08）。
      void refresh(host, hostEpoch.current, true);
    }
  };
  // ---- 远程作业身份与恢复（GUI-07）：一次查询失败不等于作业失败 ----
  // 写入本机的作业身份：主机、比较项目、提交时间、最后确认状态都在里面，重开页面/应用据此恢复核对。
  const rememberJob = (machine: string, identity: ActiveJob) =>
    store(jobKey(machine), JSON.stringify(identity));
  // 确认终态后才结束活动跟踪；作业身份与结果留在历史记录里（历史仍保留身份，重开应用能看到上次结果）。
  const settleJob = (
    machine: string,
    identity: ActiveJob,
    end: 'succeeded' | 'failed' | 'cancelled' | 'unknown',
    reason = '',
  ) => {
    const history = jobEnded(identity, end, new Date().toISOString(), reason);
    store(jobKey(machine), '');
    store(jobHistoryKey(machine), JSON.stringify(history));
    if (hostRef.current !== machine) return;
    setUnsettled((old) => (old && old.job.job === identity.job ? null : old));
    setLastJob(history);
    setActiveJob(null);
  };
  // 通信失败：保留全部身份，只标记"终态未确认"，交给页面显示"暂时失联/待核对"（GUI-07 的核心）。
  const markUncertain = (machine: string, identity: ActiveJob, error: string) => {
    const reason = String(error);
    const kept = jobUncertain(identity, reason);
    rememberJob(machine, kept);
    if (hostRef.current === machine) setUnsettled({ job: kept, reason });
    return kept;
  };
  const jobFailureText = (reply: RemoteJob) => {
    const exit = reply.status.exit_code;
    if (exit != null)
      return t(
        `服务器上的作业失败（退出码 ${exit}），原因见下面的日志。`,
        `The server job failed (exit code ${exit}); see the log below.`,
      );
    return t(
      `服务器上的作业失败了（${reply.status.detail ?? '调度系统记录为失败'}）。`,
      `The server job failed (${reply.status.detail ?? 'recorded as failed by the scheduler'}).`,
    );
  };
  const uncertainText = (machine: string, identity: ActiveJob) =>
    t(
      `服务器 ${machine} 暂时联系不上：作业 ${identity.job}（${identity.name ?? '安装环境'}）的身份已经保留，远端可能仍在运行。可以重连继续等、核对终态，或停止它。`,
      `Server ${machine} cannot be reached right now. Job ${identity.job} (${identity.name ?? 'environment install'}) is still tracked and may still be running; reconnect, check its final state, or stop it.`,
    );
  const jobEndText = (job: ActiveJob) =>
    job.end === 'succeeded'
      ? t('已成功', 'succeeded')
      : job.end === 'failed'
        ? t('失败', 'failed')
        : job.end === 'cancelled'
          ? t('已取消', 'cancelled')
          : job.end === 'unknown'
            ? t('已终止，结果未知', 'ended, result unknown')
            : t('未知', 'unknown');
  // 等服务器上的作业结束：每 3 秒取一次日志末尾显示出来。作业号记在本机，页面关掉、应用关掉都不影响
  // 服务器上的作业，回来接着等。停止按钮结束的是服务器上的作业。
  // GUI-07：只有服务器明确给出终态才清掉持久化身份；一次查询失败只标记"暂时无法确认"并保留身份。
  const watch = async (machine: string, active: ActiveJob): Promise<WatchOutcome> => {
    const epoch = hostEpoch.current;
    let identity: ActiveJob = {
      ...active,
      host: machine,
      submittedAt: active.submittedAt ?? new Date().toISOString(),
      lastState: active.lastState ?? 'queued',
      uncertain: false,
      lastError: undefined,
    };
    job.current = identity;
    setActiveJob(identity);
    rememberJob(machine, identity);
    try {
      for (;;) {
        if (!alive.current) throw new Error(LEFT);
        let reply: RemoteJob;
        try {
          reply = await invokeJson<RemoteJob>('bench_remote_job', {
            host: machine,
            job: identity.job,
            lines: 80,
          });
        } catch (error) {
          // SSH/网络暂时不通：不能当成作业失败，更不能删掉它的身份 —— 远端可能还在跑（旧行为就是这样丢的）。
          const kept = markUncertain(machine, identity, String(error));
          if (!stillCurrent(machine, epoch)) return { state: 'detached' };
          return { state: 'uncertain', job: kept, reason: String(error) };
        }
        // 用户换去看别的机器了：日志不写进新页面；身份还持久化着，回来时接着核对。
        if (!stillCurrent(machine, epoch)) return { state: 'detached' };
        setLog(reply.status.log_tail.split('\n').filter(Boolean));
        if (benchTaskId.current)
          benchTasks.update(benchTaskId.current, {
            phase:
              reply.status.state === 'running'
                ? ['服务器评估中', 'Running on the server']
                : ['排队中', 'Queued'],
          });
        const end = remoteJobTerminal(reply);
        if (end === 'succeeded') {
          settleJob(machine, identity, 'succeeded');
          return { state: 'succeeded', job: identity };
        }
        if (end === 'failed') {
          const reason = jobFailureText(reply);
          settleJob(machine, identity, 'failed', reason);
          return { state: 'failed', job: identity, error: reason };
        }
        if (end === 'cancelled') {
          settleJob(machine, identity, 'cancelled');
          return { state: 'cancelled', job: identity };
        }
        if (end === 'unknown') {
          const reason = t(
            '服务器确认这个作业已经终止，但没有留下退出码/结果（结果未知）。可以关闭这条记录后重新提交。',
            'The server confirms this job has terminated but left no exit code or result (result unknown). Close this record and submit again.',
          );
          settleJob(machine, identity, 'unknown', reason);
          return { state: 'unknown', job: identity, reason };
        }
        // 调度系统已确认作业不会再写，但退出码/结果还没经 NFS 可见（terminal=true、state 不是 finished）：
        // 继续轮询等结果，不能凭"已经结束"或轮询次数判成功/失败（审查报告第 6 条契约）。
        if (reply.terminal === true) {
          identity = jobContacted(identity, reply.status.state, new Date().toISOString());
          rememberJob(machine, identity);
          job.current = identity;
          setActiveJob(identity);
          await sleep(3000);
          continue;
        }
        // 点了停止、调度器也收下了取消请求：显示"正在取消"，等它确认终态；绝不在这里写 cancelled。
        if (reply.cancel_requested || stopping.current)
          setNotice(
            t(
              '已请求取消，等待调度系统确认终态…',
              'Cancellation requested; waiting for the scheduler to confirm the final state…',
            ),
          );
        // 查不到记录（lost/unknown）不是查询失败，也不是确认终态：保留身份继续核对。
        // 不能用"连续几次"制造终态（审查报告：3 次后写 failed 并删身份是错的）。
        identity = jobContacted(identity, reply.status.state, new Date().toISOString());
        rememberJob(machine, identity);
        job.current = identity;
        setActiveJob(identity);
        await sleep(3000);
      }
    } finally {
      job.current = null;
      stopping.current = false;
      setActiveJob(null);
    }
  };
  // watch 的结果 → task 的错误/提示语义：失败与取消抛错（task 统一显示），失联只提示并保留身份（GUI-07）。
  const afterWatch = async (
    machine: string,
    outcome: WatchOutcome,
    onSuccess?: () => Promise<void>,
  ) => {
    if (outcome.state === 'detached') return outcome;
    if (outcome.state === 'uncertain') {
      setNotice(uncertainText(machine, outcome.job));
      return outcome;
    }
    if (outcome.state === 'unknown') {
      setNotice(outcome.reason);
      return outcome;
    }
    if (outcome.state === 'failed') throw new Error(outcome.error);
    if (outcome.state === 'cancelled') throw new Error('已停止');
    if (onSuccess) await onSuccess();
    return outcome;
  };
  // 活动任务保护（GUI-07）：本机还记着一份没被确认终态的作业时，不提交新的服务器作业 ——
  // 同名比较重复提交会写同一个服务器成果目录，终态不确定时不能赌上一个已经结束。
  const assertNoUnresolvedJob = (machine: string) => {
    const pending = parseRemoteJob(stored(jobKey(machine)));
    if (!pending) return;
    const what = pending.kind === 'run' && pending.name ? `比较 ${pending.name}` : '安装环境';
    throw new Error(
      t(
        `服务器 ${machine} 上还有未确认终态的作业（${pending.job}，${what}）。先核对它的终态（或停止它），确认后才会提交新作业，避免重复写同一成果目录。`,
        `Server ${machine} still has a job whose final state is unconfirmed (${pending.job}, ${what}). Check its final state (or stop it) first; nothing new is submitted until then, so the same output folder is never written twice.`,
      ),
    );
  };
  const install = (steps: ('uv' | 'python' | 'openbench')[]) =>
    task(async () => {
      for (const step of steps) {
        const from = step === 'openbench' && source === 'local' ? localSource : null;
        const mirror = step === 'openbench' ? index || null : null;
        if (!remote) {
          await invoke('bench_install', { step, source: from, index: mirror });
          continue;
        }
        const machine = host;
        // 已有未核对终态的作业时先不提交第二次安装（GUI-07 的活动任务保护）。
        assertNoUnresolvedJob(machine);
        const submitted = await invokeJson<{ job: string }>('bench_remote_install', {
          host: machine,
          step,
          source: from,
          index: mirror,
        });
        const outcome = await afterWatch(
          machine,
          await watch(machine, { job: submitted.job, kind: 'install' }),
        );
        // 只是"暂时查不到/失联"时身份还在，不能继续装下一步，也不能显示装好了
        // （审查报告：最后一步返回 uncertain 后仍继续显示"装好了"）。
        if (outcome.state !== 'succeeded') return;
      }
      setNotice(t('装好了', 'Installed'));
    });
  const stop = () => {
    if (remote) {
      // 服务器上没有正在等的作业时（例如正在取回报告）没有可取消的对象：不能误调本机 bench_cancel。
      if (!job.current) return;
      const machine = host;
      stopping.current = true;
      // 取消失败（比如同样断网）要说出来并让停止按钮恢复可用，不能悄悄停在一个永远等下去的状态。
      void invoke<{ job: string; request_accepted?: boolean }>('bench_remote_cancel', {
        host: machine,
        job: job.current.job,
      })
        .then((reply) => {
          if (!stillCurrent(machine, hostEpoch.current)) return;
          // request_accepted 只说明调度器收下取消请求，不是终态：继续等它确认（审查报告第 6 条）。
          if (reply.request_accepted)
            setNotice(
              t(
                '已请求取消，等待调度系统确认终态…',
                'Cancellation requested; waiting for the scheduler to confirm the final state…',
              ),
            );
        })
        .catch((e) => {
          stopping.current = false;
          if (stillCurrent(machine, hostEpoch.current)) setError(shown(e));
        });
    } else void invoke('bench_cancel');
  };
  const uninstall = async (what: 'env' | 'all') => {
    const where = remote ? t(`服务器 ${host} 上的`, `on server ${host} `) : '';
    const ok = await askConfirm(
      what === 'all'
        ? t(
            `删掉${where} OpenBench、专用 Python、uv 和下载缓存？参考数据登记表、比较项目和报告不删。`,
            `Remove OpenBench, the private Python, uv and the download cache ${where}? The reference registry, comparisons and reports stay.`,
          )
        : t(
            `删掉${where} OpenBench 和它的依赖（虚拟环境）？参考数据登记表、比较项目和报告不删。`,
            `Remove OpenBench and its dependencies (the virtual environment) ${where}? The reference registry, comparisons and reports stay.`,
          ),
      { confirmLabel: t('删除', 'Remove') },
    );
    if (ok)
      await task(
        () =>
          remote
            ? invoke('bench_remote_uninstall', { host, what })
            : invoke('bench_uninstall', { what }),
        t('已删除', 'Removed'),
      );
  };

  const ready = !!status?.openbench;
  // 指标/评分清单从当前机器的 OpenBench registry 读取；读失败时保留常用兜底，不挡住生成配置。
  useEffect(() => {
    if (!ready) return;
    let alive = true;
    void invokeJson<MetricOptions>(
      remote ? 'bench_remote_metric_options' : 'bench_metric_options',
      remote ? { host } : undefined,
    )
      .then((options) => {
        if (!alive) return;
        const metricList = options.metrics?.length ? options.metrics : FALLBACK_METRICS;
        const scoreList = options.scores?.length ? options.scores : FALLBACK_SCORES;
        setMetricOptions(metricList);
        setScoreOptions(scoreList);
        setMetrics((old) => {
          const kept = old.filter((name) => metricList.some((item) => item.name === name));
          return kept.length ? kept : metricList.slice(0, 3).map((item) => item.name);
        });
        setScores((old) => {
          const kept = old.filter((name) => scoreList.some((item) => item.name === name));
          return kept.length ? kept : scoreList.slice(0, 1).map((item) => item.name);
        });
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
    // 只在切换机器/OpenBench 就绪时读一次；远端状态变化不重新拉清单。
  }, [ready, remote, host]);
  const steps: ('uv' | 'python' | 'openbench')[] = [
    ...(status?.uv || status?.uv_version ? [] : (['uv'] as const)),
    ...(status?.python ? [] : (['python'] as const)),
    ...(status?.openbench ? [] : (['openbench'] as const)),
  ];

  // 报告里的默认名字：目录名；选的是 `out/<名字>/history` 时用它上一层的名字。
  const pickLabel = (dir: string) => {
    const parts = dir.split(/[/\\]/).filter(Boolean);
    const last = parts.pop() ?? 'sim';
    return last === 'history' ? (parts.pop() ?? last) : last;
  };
  // 远程文件浏览器要目录作为起点；登记表单里的 CSV 很可能已经填了完整文件路径。
  const parentPath = (path: string) => {
    const clean = path.replace(/[/\\]+$/, '');
    const cut = Math.max(clean.lastIndexOf('/'), clean.lastIndexOf('\\'));
    return cut > 0 ? clean.slice(0, cut) : undefined;
  };
  // 参考数据登记的两个路径：本机用系统选择器，服务器用远程浏览器（和挑选模拟目录同一套）。
  const pickReferenceDir = () => {
    if (remote) {
      setBrowsing('refDataDir');
      return;
    }
    void invoke<string | null>('pick_folder', { key: 'openbench-ref-data' }).then(
      (path) => {
        if (path) setReg((old) => ({ ...old, rootDir: path }));
      },
      (e) => setError(shown(e)),
    );
  };
  const pickReferenceList = () => {
    if (remote) {
      setBrowsing('refList');
      return;
    }
    void invoke<string | null>('pick_file', { key: 'openbench-ref-list', filter: 'csv' }).then(
      (path) => {
        if (path) setReg((old) => ({ ...old, fulllist: path }));
      },
      (e) => setError(shown(e)),
    );
  };
  // 选模拟：勾一个算例（或加一个别的目录）就读它的 history 概况，填年份与种类。
  const addPick = async (dir: string, label: string) => {
    if (picks.some((p) => p.dir === dir)) return;
    setPicks((old) => [...old, { dir, label, info: null }]);
    const machine = host;
    const epoch = hostEpoch.current;
    try {
      const info = await invokeJson<HistoryInfo>(
        remote ? 'bench_remote_inspect' : 'bench_inspect',
        remote ? { host: machine, dir } : { dir },
      );
      // 探测期间换了机器：结果属于旧机器，不能写进当前页面的模拟表（GUI-05）。
      if (!stillCurrent(machine, epoch)) return;
      const blocked = info.comparable === false ? inspectBlockerText(info) : null;
      const warning = inspectWarningText(info);
      const family = inspectFamilyNote(info);
      const note = [warning ? t(warning[0], warning[1]) : '', family ? t(family[0], family[1]) : '']
        .filter(Boolean)
        .join(' ');
      setPicks((old) =>
        old.map((p) =>
          p.dir === dir
            ? {
                ...p,
                info,
                error: blocked ? t(blocked[0], blocked[1]) : undefined,
                note: note || undefined,
              }
            : p,
        ),
      );
    } catch (e) {
      if (!stillCurrent(machine, epoch)) return;
      setPicks((old) => old.map((p) => (p.dir === dir ? { ...p, error: shown(e) } : p)));
    }
  };
  const registerModelVariable = () =>
    task(
      async () => {
        if (!modelVar.variable.trim() || !modelVar.varname.trim() || !modelVar.unit.trim())
          throw new Error(
            t('变量、变量名和单位都要填。', 'Variable, variable name and unit are required.'),
          );
        // OpenBench 把 `标准名:变量名:单位` 按冒号拆段，字段里的冒号会被静默当成前缀/后缀；后端也拒绝，
        // 但服务器那条路径的拒绝是英文原文，这里先用两种语言说清楚。
        if (
          [modelVar.variable, modelVar.varname, modelVar.unit].some((value) => value.includes(':'))
        )
          throw new Error(
            t(
              '变量、变量名和单位里不能有冒号（:）。',
              'Variable, variable name and unit cannot contain a colon (:).',
            ),
          );
        await invoke(
          remote ? 'bench_remote_register_model_variable' : 'bench_register_model_variable',
          {
            ...(remote ? { host } : {}),
            model: modelVar.model.trim(),
            variable: modelVar.variable.trim(),
            varname: modelVar.varname.trim(),
            unit: modelVar.unit.trim(),
          },
        );
        setDialog(null);
        setModelVar({ model: 'CoLM2024', variable: '', varname: '', unit: '', description: '' });
      },
      t('模型变量已注册', 'Model variable registered'),
    );
  const registerReference = () =>
    task(
      async () => {
        await invoke(remote ? 'bench_remote_register' : 'bench_register', {
          ...(remote ? { host } : {}),
          name: reg.name.trim(),
          rootDir: reg.rootDir.trim(),
          dataType: reg.dataType,
          timRes: reg.timRes,
          years: [Number(reg.from), Number(reg.to)],
          category: null,
          fulllist: reg.fulllist.trim() || null,
          variables: reg.variables
            .split('\n')
            .map((v) => v.trim())
            .filter(Boolean),
        });
        setDialog(null);
      },
      t('参考数据已登记', 'Reference dataset registered'),
    );
  // ---- 模拟登记表单（GUI-09/10/11）：保存时把表单合并回完整探测对象，字段错误进弹窗并聚焦 ----
  const changeSimulationDir = (dir: string) =>
    setSimForm((old) => {
      // 目录一改，上一处探测到的前缀/年份/分组/范围都属于旧目录，不能带进新目录（GUI-10）。
      if (!old.probe || old.probeDir === dir.trim()) return { ...old, dir };
      return { ...old, ...CLEAR_PROBED_SIM_FIELDS, dir, probe: null, probeDir: '' };
    });
  const showSimulationIssue = (issue: SimulationIssue) => {
    setSimError({ field: issue.field, message: t(issue.message[0], issue.message[1]) });
    // 焦点放到第一个出错的字段：弹窗里字段多，键盘用户不用自己找（GUI-11）。
    globalThis.document?.querySelector<HTMLElement>(`[data-sim-field="${issue.field}"]`)?.focus();
  };
  const simHint = (field: SimulationField, hint?: string) =>
    simError?.field === field ? simError.message : hint;
  const editSimulation = (pick?: Pick) => {
    setSimEditing(pick?.dir ?? null);
    setSimError(null);
    if (pick) {
      const fields = simulationFormForPick(pick);
      setSimForm({ ...emptySimForm(), ...fields, probe: pick.info, probeDir: pick.dir });
    } else setSimForm(emptySimForm());
    setError('');
    setDialog('simulation');
  };
  const inspectSimulation = () => {
    const dir = simForm.dir.trim();
    if (!dir) {
      showSimulationIssue({
        field: 'dir',
        message: ['请先填模拟目录。', 'Enter the simulation folder first.'],
      });
      return;
    }
    const machine = host;
    const epoch = hostEpoch.current;
    return task(
      async () => {
        const info = await invokeJson<HistoryInfo>(
          remote ? 'bench_remote_inspect' : 'bench_inspect',
          remote ? { host: machine, dir } : { dir },
        );
        // 探测期间换了机器：这份概况属于旧机器，不能写进当前页面的表单（GUI-05）。
        if (!stillCurrent(machine, epoch)) return;
        setSimForm((old) => {
          // 读取期间用户改了目录：这次探测属于旧目录，不能写进新目录的元数据（GUI-10）。
          if (old.dir.trim() !== dir) return old;
          const fields = simulationFormForPick({ dir, label: old.label || pickLabel(dir), info });
          return {
            ...old,
            ...fields,
            label: old.label || fields.label,
            variables: old.variables.length ? old.variables : fields.variables,
            probe: info,
            probeDir: dir,
          };
        });
        const blocked = info.comparable === false ? inspectBlockerText(info) : null;
        if (blocked) showSimulationIssue({ field: 'dir', message: blocked });
        else setSimError(null);
      },
      t('已读取目录概况', 'Folder inspected'),
    );
  };
  const saveSimulation = () => {
    try {
      const dir = simForm.dir.trim();
      // 只沿用同一目录的完整探测结果；换了目录就不能拿旧目录的 extent/lon/lat/files 冒充新目录的。
      const probe = simForm.probe && simForm.probeDir === dir ? simForm.probe : null;
      const issue = validateSimulation(probe, simForm);
      if (issue) {
        showSimulationIssue(issue);
        return;
      }
      const info = simulationInfoForSave(probe, simForm);
      const variables = Object.fromEntries(
        simForm.variables
          .map((row) => ({
            variable: row.variable.trim(),
            varname: row.varname.trim(),
            unit: row.unit.trim(),
          }))
          .filter((row) => row.variable && row.varname)
          .map((row) => [row.variable, { varname: row.varname, varunit: row.unit }]),
      );
      // 编辑已有模拟时以打开弹窗的那条为身份：即使把目录改到别处也替换原条目，不能留下旧条目
      // 再新增一条（审查报告：编辑已有模拟目录后旧条目仍在；同名时后续生成失败）。
      const editingAt = simEditing === null ? -1 : picks.findIndex((p) => p.dir === simEditing);
      const sameDir = picks.findIndex((p) => p.dir === dir);
      if (sameDir >= 0 && sameDir !== editingAt) {
        showSimulationIssue({
          field: 'dir',
          message: [
            '这个目录已经是一个模拟了；请选别的目录，或直接编辑那一条。',
            'This folder is already one of the simulations; pick another folder or edit that entry.',
          ],
        });
        return;
      }
      setPicks((old) => {
        // 分组/原生分辨率已经并进 info；清掉旧的手动覆盖，免得它继续盖住用户在表单里改的值（GUI-09/10）。
        const next = {
          dir,
          label: simForm.label.trim() || pickLabel(dir),
          info,
          variables,
          dataGroupby: null,
          gridRes: null,
          profile: simForm.profile.trim() || 'CoLM2024',
          stationId: simForm.stationId.trim() || null,
        };
        if (editingAt >= 0)
          return old.map((p, i) => (i === editingAt ? { ...p, ...next, error: undefined } : p));
        const at = old.findIndex((p) => p.dir === dir);
        return at >= 0
          ? old.map((p, i) => (i === at ? { ...p, ...next, error: undefined } : p))
          : [...old, next];
      });
      setDialog(null);
      setSimEditing(null);
      setSimError(null);
      setError('');
    } catch (e) {
      // 兜底：程序异常也要在界面里说清楚，不能只留在控制台（GUI-11）。
      setSimError(null);
      setError(shown(e));
    }
  };
  // 探测明确判定不可比较（多 patch/非结构/未知空间类型/跨日界线）的目录不参与生成与运行：
  // 运行前就阻断，而不是先显示"可比较"再在制备阶段 KeyError（审查报告第 2 条）。
  const ok = picks.filter(
    (p): p is Pick & { info: HistoryInfo } => !!p.info && p.info.comparable !== false,
  );
  useEffect(() => {
    const years = commonYears(ok.map((p) => ({ label: p.label, info: p.info })));
    if (years && !from && !to) {
      setFrom(String(years[0]));
      setTo(String(years[1]));
    }
    const res = defaultGridRes(ok.map((p) => p.info));
    const next = res ? String(res) : '';
    setGridRes((old) => (old === '' || old === autoGridRes.current ? next : old));
    autoGridRes.current = next;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ok.length]);
  // 换了评估的机器：状态、登记表、参考数据根目录都换成那台的；所选模拟的路径属于原来那台，清掉。
  // GUI-05：代次 +1，让切换前发出的状态/登记表/探测请求全部作废。
  // GUI-07：那台机器上没等完的作业按"终态是否已确认"分开处理 —— 正常在跑就接着等（上次关了页面或
  // 应用），上次是"暂时失联"就交给用户重连/核对/停止，不自动反复重试。
  useEffect(() => {
    store('colm.benchHost', host);
    hostEpoch.current += 1;
    const epoch = hostEpoch.current;
    setStatus(null);
    setReferences([]);
    setPicks([]);
    setChosen({});
    setYaml('');
    setMissingExtent([]);
    generated.current = null;
    setSimError(null);
    setLog([]);
    setScanDirState(stored(refKey(host)));
    void refresh(host, epoch);
    const history = parseRemoteJob(stored(jobHistoryKey(host)));
    setLastJob(history?.end ? history : null);
    const pending = host ? parseRemoteJob(stored(jobKey(host))) : null;
    if (!pending) {
      setUnsettled(null);
      return;
    }
    if (pending.uncertain) {
      setUnsettled({ job: pending, reason: pending.lastError ?? '' });
      return;
    }
    setUnsettled(null);
    void task(async () => {
      const outcome = await watch(host, pending);
      await afterWatch(host, outcome, async () => {
        if (pending.kind === 'run' && pending.name)
          await fetchReport(host, pending.name, pending.job);
      });
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host]);
  useEffect(() => {
    alive.current = true;
    return () => {
      alive.current = false;
    };
  }, []);
  // ---- 评估设计绑定（GUI-04）：同一份设计既生成配置，也是运行前校验和配置失效的依据 ----
  // GUI-04：生成配置与运行共用同一份完整评估设计（含机器/OpenBench/参考数据路径身份）。
  const comparisonDesign = (): ComparisonDesign => ({
    name,
    years: [Number(from), Number(to)],
    timRes,
    variables: chosen,
    simulations: ok.map((p) => ({
      label: p.label,
      info: p.info,
      variables: p.variables,
      dataGroupby: p.dataGroupby,
      gridRes: p.gridRes,
      profile: p.profile,
      stationId: p.stationId,
    })),
    projectDir: comparisonDir(status?.root ?? '', name),
    gridRes: Number(gridRes) || null,
    weight: weight || null,
    minYear: minYear.trim() === '' ? null : Number(minYear),
    timeAlignment: timeAlignment || null,
    numCores: numCores.trim() === '' ? null : Number(numCores),
    metrics,
    scores,
    comparisonItems,
    statisticsItems,
  });
  const machine = { host, openbenchRoot: status?.root ?? '', refRoot: scanDir };
  // 表单设计：`生成配置` 生成配置与站点清单时用它（GUI-04）。
  const formDesign: EvaluationDesign = { machine, ...comparisonDesign() };
  // 层维度变量不再在生成前拒绝：打开文件时保留层维度，OpenBench CoLM2024 profile 用 compute 消费；
  // 但用户显式映射的 varname 若 profile 不认识，OpenBench check 才会报错，所以这里只提示不阻断。
  const layerWarnings = layerVariableWarnings(formDesign);
  // statistics 阶段只在格点场上产图：纯单点比较选了方法时提前说明，别让用户以为漏跑了。
  const stationStatisticsWarning = statisticsStationWarning(formDesign.simulations);
  // 已知跑不了的方法/格点模拟不足/参考全是单点：都在统计多选下方提前说清楚（生成时同样会拦）。
  const referenceStatisticsWarning = statisticsReferenceWarning(formDesign, references);
  const statisticsWarnings: [string, string][] = [
    ...(stationStatisticsWarning ? [stationStatisticsWarning] : []),
    ...statisticsMethodWarnings(formDesign.simulations, statisticsItems),
    ...(referenceStatisticsWarning ? [referenceStatisticsWarning] : []),
  ];
  // 设计指纹覆盖生成配置与运行所依赖的全部输入（含 pick.info 里的前缀/类型/年份/分组/范围/分辨率），
  // 只改目录名和显示名之外的东西也会变（GUI-04）。
  const design = designFingerprint(formDesign);
  const generatedConfig = generated.current;
  // 有效评估配置：用户可以在文本框里直接改 YAML，提交给后端的也是这段文本。配置部分（年份/时间尺度/
  // 数据源/变量/指标）以解析后的文本为准（审查报告 P1）；站点部分读生成时冻结的站点清单（R01）。
  const parsedConfig = useMemo(() => (yaml ? parseOpenbenchConfig(yaml) : null), [yaml]);
  const effectiveConfig = parsedConfig?.config ?? null;
  const configProblems = parsedConfig?.problems ?? [];
  // 生成配置之后页面表单（站点目录/前缀/坐标、模拟条目、年份等）又改过：快照过期，不能运行。
  // 手改文本不算过期：文本没有被冻结，运行时重新解析；只有快照里的设计与站点清单是冻结的。
  const staleSnapshot = generatedConfig ? snapshotStale(generatedConfig, formDesign) : false;
  // 手改不能增删模拟或改单点站点清单路径：那些文件由后端按（冻结的）设计准备。
  const structureProblems = effectiveConfig
    ? structuralProblems(generatedConfig?.design ?? formDesign, effectiveConfig)
    : [];
  // 有效输入指纹 = 快照冻结的站点清单 + 当前文本解析出的完整配置。手改配置换指纹；改站点输入因快照
  // 过期而退回表单指纹，让完成提示/保存徽标也随改动失效。
  const effectiveFingerprint = staleSnapshot
    ? design
    : generatedConfig && effectiveConfig
      ? configFingerprint(generatedConfig.design, effectiveConfig, generatedConfig.sites)
      : design;
  // 运行期间用户还能改字段/改文本，完成提示回来时据此判断它是不是已经过时（U-05）。
  const latestEffective = useRef(effectiveFingerprint);
  latestEffective.current = effectiveFingerprint;
  const yamlHandEdited = !!yaml && !!generatedConfig && yaml !== generatedConfig.generatedText;
  // 生成配置之后表单又改过（但用户手写的内容还留着）：提示表单改动没有进入这份快照。
  const formChanged = staleSnapshot;
  // U-04：运行前条件摘要 = 冻结站点清单（站点输入）+ 当前解析出的有效配置（其余条件）；
  // 没有生成配置时仍按表单设计显示（还没提交过任何文本）。过期时首行明确说"已过期、需重新生成"。
  const conditionSummary = (): EvaluationSummaryItem[] => {
    const items =
      generatedConfig && effectiveConfig
        ? effectiveSummary(generatedConfig.design, effectiveConfig, generatedConfig.sites)
        : effectiveConfig
          ? effectiveSummary(formDesign, effectiveConfig)
          : evaluationSummary(formDesign);
    if (!staleSnapshot) return items;
    return [
      {
        label: ['输入快照', 'Input snapshot'],
        value: t(
          '已过期：生成配置后页面设置（站点目录/坐标、模拟条目、年份等）又改过。这份摘要里的站点输入是生成时冻结的那份，不能按现在的页面运行；重新生成评估配置后才会用上这些改动。',
          'Expired: the page settings changed after the configuration was generated (station folders/coordinates, simulation entries, years, ...). The station inputs in this summary are the frozen ones, so the current page cannot run; regenerate the evaluation configuration to use the changes.',
        ),
      },
      ...items,
    ];
  };
  // U-05：评估配置三态（有未保存修改 / 正在保存 / 已保存）。手改后的文本还没有写进比较项目目录，
  // 因此显示"未保存"而不是"saved"（审查报告已复现的误导）。
  const configState = configSaveState({
    yaml,
    currentFingerprint: design,
    generated: generatedConfig
      ? { fingerprint: generatedConfig.designFingerprint, config: generatedConfig.generatedText }
      : null,
    submitting: configSaving,
  });
  // 运行/保存成功提示只在同一份有效配置上有效；设置或文本一改就清掉（U-05）。
  const showRunNotice = (message: string, fingerprint: string) => {
    // 运行期间配置已经改过：这次完成的是旧配置，不能给当前设置挂"评估完成"。
    if (latestEffective.current !== fingerprint) return;
    runNoticeFor.current = fingerprint;
    setRunNotice(message);
  };
  const clearStaleEvaluationNotice = () => {
    if (!runNotice || runNoticeFor.current === effectiveFingerprint) return;
    runNoticeFor.current = '';
    setRunNotice('');
  };
  const invalidateGeneratedConfig = () => {
    if (
      configInvalidation({
        currentFingerprint: design,
        generated: generatedConfig
          ? {
              fingerprint: generatedConfig.designFingerprint,
              config: generatedConfig.generatedText,
            }
          : null,
        yaml,
      }) !== 'clear'
    )
      return;
    generated.current = null;
    setYaml('');
    setMissingExtent([]);
  };
  useEffect(() => {
    invalidateGeneratedConfig();
    clearStaleEvaluationNotice();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [design, effectiveFingerprint]);
  const allVariables = comparableVariables(
    references,
    ok.map((p) => p.info),
  );
  const variables = comparableVariables(
    references,
    ok.map((p) => p.info),
    variableQuery,
  );
  const simSpatial =
    simForm.dataType === 'grid' ||
    (simForm.dataType === 'auto' && (simForm.probe?.spatial ?? false));
  const restoreDraft = () => {
    const draft = draftOffer;
    if (!draft) return;
    setPicks(draft.picks as unknown as Pick[]);
    setName(draft.name);
    setFrom(draft.from);
    setTo(draft.to);
    setTimRes(draft.timRes);
    setGridRes(draft.gridRes);
    setChosen(draft.chosen);
    setWeight(draft.weight);
    setMinYear(draft.minYear);
    setTimeAlignment(draft.timeAlignment);
    setNumCores(draft.numCores);
    setMetrics(draft.metrics);
    setScores(draft.scores);
    setComparisonItems(draft.comparisonItems);
    setStatisticsItems(draft.statisticsItems);
    setYaml(draft.yaml);
    setDraftOffer(null);
    setError('');
    setRunNotice('');
  };
  const discardDraft = () => {
    setDraftOffer(null);
    store(compareDraftKey(host), '');
  };
  const generate = () => {
    try {
      const inputs = comparisonDesign();
      const built = buildOpenbenchConfig(inputs);
      // 设计、站点清单、配置文本和一个有效指纹绑成同一份快照；摘要的站点部分、指纹和运行都读它。
      generated.current = freezeCompareSnapshot({ machine, ...inputs }, built);
      setYaml(built.config);
      setMissingExtent(built.missingExtent);
      setError('');
    } catch (e) {
      setError(shown(e));
    }
  };
  // 手改过的配置在覆盖前要问一声：这是用户改的内容，不能因为点了"重新生成"就静默丢掉（GUI-04）。
  const regenerate = async () => {
    if (yamlHandEdited) {
      const proceed = await askConfirm(
        t(
          '重新生成会覆盖你在配置里改过的内容，继续？',
          'Regenerating replaces the edits you made to the configuration. Continue?',
        ),
        { confirmLabel: t('重新生成评估配置', 'Regenerate evaluation configuration') },
      );
      if (!proceed) return;
    }
    generate();
  };
  const run = () =>
    task(async () => {
      // 运行前只认生成时冻结的那份输入：没有快照就没有可信的站点清单/模拟集合；表单在生成后又改过时
      // 快照过期，这里明确阻断并让用户重新生成，而不是"摘要读表单、提交读旧站点"。
      const snapshot = generated.current;
      if (!snapshot)
        throw new Error(
          t(
            '还没有生成评估配置：先点「生成配置」，摘要、指纹和提交都绑定那一份快照。',
            'No evaluation configuration yet: generate one first; the summary, fingerprint and submission all bind to that snapshot.',
          ),
        );
      if (snapshotStale(snapshot, formDesign))
        throw new Error(
          t(
            '生成配置之后页面设置又改过（站点目录/坐标、模拟条目、年份等）：冻结的站点清单和这份配置已经不是同一份输入，不能运行。请重新生成评估配置后再运行。',
            'The page settings changed after the configuration was generated (station folders/coordinates, simulation entries, years, ...): the frozen station list and this configuration are no longer the same inputs, so it cannot run. Regenerate the evaluation configuration first.',
          ),
        );
      // 文本没有被冻结：按当前文本重新解析；解析失败或结构校验不过都明确阻断（审查报告 P1）。
      const parsedNow = parseOpenbenchConfig(yaml);
      if (!parsedNow.config)
        throw new Error(
          t(
            `配置里有不能运行的内容：${parsedNow.problems.map(([zh]) => zh).join('；')}`,
            `The configuration cannot run: ${parsedNow.problems.map(([, en]) => en).join('; ')}`,
          ),
        );
      const frozenConfig = parsedNow.config;
      const mismatched = structuralProblems(snapshot.design, frozenConfig);
      if (mismatched.length)
        throw new Error(
          t(mismatched.map(([zh]) => zh).join('；'), mismatched.map(([, en]) => en).join('; ')),
        );
      // 冻结这次运行的实际条件（当前配置文本、快照里的站点清单、两者一起算的指纹）：完成提示与报告
      // 身份都绑定这份输入，运行期间再改页面也不会把"上一次的结果"说成当前配置的结果。
      const frozen = {
        config: yaml,
        sites: snapshot.sites,
        fingerprint: configFingerprint(snapshot.design, frozenConfig, snapshot.sites),
        at: new Date().toISOString(),
      };
      const project = comparisonName(name);
      benchTaskId.current = benchTasks.begin({
        host,
        project,
        label: name,
        kind: 'run',
        phase: ['提交运行', 'Submitting'],
      });
      benchTasks.registerStop(benchTaskId.current, stop);
      try {
        // U-05：提交期间配置正在写入比较项目目录，徽标显示"正在保存"。
        setConfigSaving(true);
        try {
          if (!remote) {
            benchTasks.update(benchTaskId.current, {
              phase: ['检查并运行', 'Checking and running'],
            });
            const result = await invokeJson<{
              report: string | null;
              run_id?: string;
              config_sha?: string;
              state?: string;
              latest_success?: unknown;
            }>('bench_run', {
              name: project,
              config: frozen.config,
              sites: frozen.sites,
              refRoot: scanDir || null,
            });
            // bench_run 会先把配置写进比较项目目录，再跑评估；能返回就说明这一版已写盘。
            setWrittenFingerprint(frozen.fingerprint);
            setLastRun({
              runId: result.run_id ?? '',
              configSha: result.config_sha ?? '',
              fingerprint: frozen.fingerprint,
              at: frozen.at,
              report: result.report,
            });
            if (result.report) await view(result.report);
            showRunNotice(
              result.run_id
                ? t(
                    `评估完成（运行 ${result.run_id}）`,
                    `Evaluation finished (run ${result.run_id})`,
                  )
                : t('评估完成', 'Evaluation finished'),
              frozen.fingerprint,
            );
            // 配置已经写进项目目录并跑完，草稿不再算"未完成"；失败/取消路径不清，留给用户恢复。
            store(compareDraftKey(host), '');
            benchTasks.finish(benchTaskId.current, { state: 'done', report: result.report });
            benchTasks.registerStop(benchTaskId.current, null);
            return;
          }
          // 配置、参考数据根目录与提交主机属于同一个上下文：这里都用渲染时的当前机器，切机器会先清配置（GUI-05）。
          const machine = host;
          // 未核对终态的已有作业：不重复提交，先核对或停止（GUI-07 的活动任务保护）。
          assertNoUnresolvedJob(machine);
          const submitted = await invokeJson<{
            job: string;
            /** 正常是调度系统的提交结果；提交回执丢失但按作业号有界重查确认已接受/无法判断时是字符串 "unconfirmed"。 */
            submitted?: unknown;
            /** 远程 run 身份 == 作业号：服务器在 <project>/runs/<id>/ 冻结这次运行的配置。 */
            run_id?: string;
            config_sha?: string;
          }>('bench_remote_run', {
            host: machine,
            name: project,
            config: frozen.config,
            sites: frozen.sites,
            refRoot: scanDir || null,
          });
          // 回执丢了不等于提交失败：后端已按作业号确认/继续跟踪，这里照常 watch（不能报错或重复提交）。
          if (submitted.submitted === 'unconfirmed')
            setNotice(
              t(
                '服务器没有确认收到提交回执，已按作业号继续跟踪；若一直查不到记录，稍后再提交。',
                'The server did not confirm receiving the submission; tracking continues by job id. If no record ever appears, submit again later.',
              ),
            );
          // 远程提交被接受时后端已在服务器上写好这一版配置；成功/失败的运行都不影响"已写盘"。
          setWrittenFingerprint(frozen.fingerprint);
          benchTasks.update(benchTaskId.current, {
            phase: ['等待服务器调度', 'Waiting for the scheduler'],
            job: submitted.job,
          });
          const runId = submitted.run_id ?? submitted.job;
          const outcome = await watch(machine, { job: submitted.job, kind: 'run', name: project });
          await afterWatch(machine, outcome, () => fetchReport(machine, project, runId));
          if (outcome.state === 'succeeded') {
            store(compareDraftKey(host), '');
            benchTasks.finish(benchTaskId.current, { state: 'done' });
            benchTasks.registerStop(benchTaskId.current, null);
            setLastRun({
              runId,
              configSha: submitted.config_sha ?? '',
              fingerprint: frozen.fingerprint,
              at: frozen.at,
              report: null,
            });
            showRunNotice(t('评估完成', 'Evaluation finished'), frozen.fingerprint);
          }
        } finally {
          setConfigSaving(false);
        }
      } catch (error) {
        // 任务条只存原文；页面自己的错误区仍用 shown() 做双语。离开页面让轮询停下（LEFT）不是运行失败：
        // 远程作业可能还在服务器上跑，标 interrupted 让用户回来再核对。
        const message = String(error).replace(/^Error: /, '');
        benchTasks.finish(benchTaskId.current, {
          state: message === LEFT ? 'interrupted' : 'failed',
          error: message,
        });
        benchTasks.registerStop(benchTaskId.current, null);
        throw error;
      }
    });
  // 只取回报告、指标与配置；合并出来的站点文件和中间数据留在服务器上。
  // GUI-08：同一比较取回期间不重复触发；失败时已显示的报告原样保留，后端错误会说明本机旧报告
  // 仍是上次成功版本（`benchErrorText` 翻成当前语言给用户看）。
  const fetchReport = async (machine: string, project: string, runId?: string) => {
    // 同一比较的不同 run 可以各自取回（报告与冻结配置按 run 绑定），所以去重键带上 run。
    const key = `${machine}\n${project}\n${runId ?? ''}`;
    if (fetching.current.has(key))
      throw new Error(
        t(
          '这个比较的报告正在取回中，等这次取回结束后再试。',
          'This comparison’s report is being fetched already; try again when that fetch finishes.',
        ),
      );
    fetching.current.add(key);
    const epoch = hostEpoch.current;
    try {
      const fetched = await invokeJson<{
        report: string | null;
        run_id?: string | null;
        config_sha?: string | null;
        state?: string | null;
        legacy?: boolean;
      }>('bench_remote_fetch', {
        host: machine,
        name: project,
        ...(runId ? { runId } : {}),
      });
      // 取回期间用户换看了别的机器：旧机器的报告不能显示到当前页面（GUI-05）。
      if (!stillCurrent(machine, epoch)) return;
      // 服务器随报告一起回来的 run 身份：页面在重载前也能说明"这份报告属于哪次运行"。
      if (fetched.run_id)
        setLastRun((old) => ({
          runId: fetched.run_id!,
          configSha: fetched.config_sha ?? '',
          fingerprint: old?.fingerprint ?? '',
          at: new Date().toISOString(),
          report: fetched.report,
        }));
      if (fetched.report) await view(fetched.report);
    } finally {
      fetching.current.delete(key);
    }
  };
  const view = async (path: string) => {
    const html = await invoke<string>('bench_report', { path });
    setReport({ path, html });
  };
  // 核对"暂时失联"作业的终态（GUI-07）：只查一次。仍在运行就更新最后确认状态、不结束跟踪；
  // 查到终态才清活动记录（写进历史）；还是查不到就保留身份并把新的原因显示出来。
  // 任务条带来的"打开这次结果"意图：只消费一次；跨机器先切过去，host 变化后这个 effect 会再跑。
  const consumedBenchFocus = useRef(false);
  useEffect(() => {
    if (consumedBenchFocus.current) return;
    const focus = benchTasks.peekFocus();
    if (!focus) return;
    if (focus.host !== host) {
      setHostState(focus.host);
      return;
    }
    consumedBenchFocus.current = true;
    benchTasks.consumeFocus();
    if (focus.report) void view(focus.report);
    else if (focus.host)
      void fetchReport(focus.host, focus.project, focus.runId ?? undefined).catch((error) => {
        // 取报告失败不能静默：页面已有的错误区显示后端给出的原因（本机旧报告仍在）。
        setError(shown(error));
      });
    // 只在进入页面/切换机器（status 就绪）时试一次；focus 是一次性意图，不需要跟着每次渲染重跑。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [host, status]);
  const checkUnsettled = () =>
    task(async () => {
      const current = unsettled;
      if (!current) return;
      const machine = current.job.host ?? host;
      let reply: RemoteJob;
      try {
        reply = await invokeJson<RemoteJob>('bench_remote_job', {
          host: machine,
          job: current.job.job,
          lines: 80,
        });
      } catch (error) {
        markUncertain(machine, current.job, String(error));
        throw error;
      }
      if (!stillCurrent(machine, hostEpoch.current)) return;
      setLog(reply.status.log_tail.split('\n').filter(Boolean));
      const end = remoteJobTerminal(reply);
      if (!end) {
        // 更新"最后确认状态"，但保持"待核对"（终态确认前不结束活动跟踪）。
        const kept: ActiveJob = {
          ...jobContacted(current.job, reply.status.state, new Date().toISOString()),
          uncertain: true,
        };
        rememberJob(machine, kept);
        setUnsettled({ job: kept, reason: '' });
        setNotice(
          reply.terminal === true
            ? t(
                '调度系统已确认作业结束，但退出码/结果还没可见；继续核对终态。',
                'The scheduler confirms the job has ended, but its exit code/result is not visible yet; check again shortly.',
              )
            : reply.cancel_requested
              ? t(
                  '取消请求已被调度系统接受；等它确认终态后才结束跟踪。',
                  'The scheduler accepted the cancellation request; tracking ends only once it confirms the final state.',
                )
              : t(
                  '作业仍在服务器上运行；确认终态之前不会结束跟踪。',
                  'The job is still running on the server; tracking ends only once its final state is confirmed.',
                ),
        );
        return;
      }
      if (end === 'unknown') {
        const reason = t(
          '服务器确认这个作业已经终止，但没有留下退出码/结果（结果未知）。可以关闭这条记录后重新提交。',
          'The server confirms this job has terminated but left no exit code or result (result unknown). Close this record and submit again.',
        );
        settleJob(machine, current.job, 'unknown', reason);
        setNotice(reason);
        return;
      }
      const reason = end === 'failed' ? jobFailureText(reply) : '';
      settleJob(machine, current.job, end, reason);
      if (end === 'failed') throw new Error(reason);
      if (end === 'cancelled') {
        setNotice(t('已确认作业已取消', 'The job was confirmed as cancelled'));
        return;
      }
      if (current.job.kind === 'run' && current.job.name)
        await fetchReport(machine, current.job.name, current.job.job);
      setNotice(t('已确认作业成功结束', 'The job was confirmed as finished successfully'));
    });
  // 重连并继续等（GUI-07）：和正常跟踪同一条路径，成功后自动取回报告；再失联就仍回到"待核对"。
  const reconnect = () =>
    task(async () => {
      const current = unsettled;
      if (!current) return;
      const machine = current.job.host ?? host;
      const outcome = await watch(machine, current.job);
      await afterWatch(machine, outcome, async () => {
        if (current.job.kind === 'run' && current.job.name)
          await fetchReport(machine, current.job.name, current.job.job);
      });
      if (outcome.state === 'succeeded')
        setNotice(t('服务器上的作业已结束', 'The server job has finished'));
    });
  // 停止一个终态未知的作业（GUI-07）：取消请求发出后再确认"已取消"；仍联系不上就保留身份与原因。
  const stopUnsettled = () =>
    task(async () => {
      const current = unsettled;
      if (!current) return;
      const machine = current.job.host ?? host;
      stopping.current = true;
      try {
        await invoke('bench_remote_cancel', { host: machine, job: current.job.job });
      } catch (error) {
        stopping.current = false;
        markUncertain(machine, current.job, String(error));
        throw error;
      }
      const outcome = await watch(machine, current.job);
      await afterWatch(machine, outcome);
    });
  // 历史里上次成功的作业：服务器上的报告还在，可以再取回一次（GUI-07 的"继续取回报告"）。
  const fetchLastReport = () =>
    task(async () => {
      const history = lastJob;
      if (!history?.name) return;
      await fetchReport(history.host ?? host, history.name, history.job);
      setNotice(t('已取回报告', 'Report fetched'));
    });

  usePageContract({
    title: t('比较', 'Compare'),
    step: 'compare',
    status: busy ? 'working' : error,
    fields: [
      {
        id: 'compare-name',
        label: t('比较名', 'Comparison name'),
        value: name,
        type: 'text',
        set: setName,
      },
      {
        id: 'compare-from',
        label: t('起始年', 'From year'),
        value: from,
        type: 'number',
        set: setFrom,
      },
      { id: 'compare-to', label: t('结束年', 'To year'), value: to, type: 'number', set: setTo },
    ],
    actions: [],
    details: {
      status,
      picks: picks.map((p) => ({ dir: p.dir, label: p.label, info: p.info })),
      config: {
        formChanged,
        stale: staleSnapshot,
        handEdited: yamlHandEdited,
        problems: [...configProblems, ...structureProblems],
        fingerprint: effectiveFingerprint,
        missingExtent,
        saveState: configState,
        summary: conditionSummary(),
      },
    },
  });

  const caseDirs = state.cases ?? [];
  return (
    <div className="page-stack">
      <header className="research-heading">
        <div>
          <h1>{t('比较与评估', 'Compare and evaluate')}</h1>
          <p>
            {t(
              '用 OpenBench 把几个模拟（不同项目、不同模型版本）放在同一批参考数据上打分，生成评估报告。"研究"负责调参数，这里负责比结果。',
              'Score several simulations (from different projects or model versions) against the same reference data with OpenBench and get an evaluation report. Research tunes parameters; this page compares results.',
            )}
          </p>
        </div>
      </header>
      {draftOffer && (
        <Notice tone="info">
          <p>
            {t(
              '上次离开前，这个页面还有一份没跑完的配置草稿（只存在本机，不含运行状态和报告）。要恢复吗？',
              'This page kept an unfinished configuration draft from your last visit (stored on this computer; no run state or report). Restore it?',
            )}
          </p>
          <div className="button-row">
            <Button onClick={restoreDraft}>{t('恢复草稿', 'Restore draft')}</Button>
            <Button variant="ghost" onClick={discardDraft}>
              {t('丢弃草稿', 'Discard draft')}
            </Button>
          </div>
        </Notice>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      {notice && <Notice tone="success">{notice}</Notice>}
      <div data-compare-section="environment">
        <Card
          title={t('环境状态', 'Environment status')}
          description={t(
            '这台机器上 OpenBench 的状态；安装、重装与维护收在下面的折叠区里。',
            'The OpenBench state on this machine; install, reinstall and maintenance are folded below.',
          )}
        >
          <Field label={t('运行位置', 'Machine')}>
            <Select value={host} disabled={busy} onChange={(e) => setHostState(e.target.value)}>
              <option value="">{t('本机', 'This computer')}</option>
              {[...new Set([...servers, ...(host ? [host] : [])])].map((value) => (
                <option key={value} value={value}>
                  {t(`服务器 ${value}`, `Server ${value}`)}
                </option>
              ))}
            </Select>
          </Field>
          {busy && (
            <p className="muted study-hint">
              {t(
                '正在处理：操作结束（服务器作业确认终态）之前不能切换运行位置；浏览别的机器不影响已有作业的身份。',
                'Working: the machine cannot be switched until this operation finishes (and the server job’s final state is settled); browsing other machines does not affect the tracked job.',
              )}
            </p>
          )}
          <p className="muted study-hint" data-compare-readiness>
            {!status
              ? t('正在检查已经装了什么…', 'Checking what is installed…')
              : ready
                ? t(
                    `OpenBench 已就绪（${status.openbench}）。`,
                    `OpenBench is ready (${status.openbench}).`,
                  )
                : t(
                    'OpenBench 还没有装好：展开下面的「安装、重装与维护」安装环境。',
                    'OpenBench is not installed yet: expand “Install, reinstall and maintenance” below to set it up.',
                  )}
          </p>
          <Advanced title={t('安装、重装与维护', 'Install, reinstall and maintenance')}>
            <p className="muted">
              {t(
                '用 OpenBench 评估。uv、Python、OpenBench 和它的登记表、缓存全部装在软件自己的数据目录里，不碰系统里的 Python，也不和你自己装的 OpenBench 混用：先装 uv（约 20 MB，校验后才用），再装专用 Python 3.12，最后装 OpenBench 和它的全部依赖（约 400 MB，需要联网）。',
                'Evaluation uses OpenBench. uv, Python, OpenBench, its reference registry and caches all live in the app’s own data folder, apart from the system Python and any OpenBench you installed yourself: uv first (about 20 MB, verified before use), then a private Python 3.12, then OpenBench with all its dependencies (about 400 MB, needs the internet).',
              )}
            </p>
            <ul className="compare-status">
              {status?.root && (
                <li>
                  {remote
                    ? t(`安装位置（服务器 ${host}）：`, `Installed in (server ${host}): `)
                    : t('安装位置：', 'Installed in: ')}
                  {status.root}{' '}
                  {!remote && (
                    <Button onClick={() => void invoke('open_path', { path: status.root })}>
                      {t('打开', 'Open')}
                    </Button>
                  )}
                </li>
              )}
              {/* 状态回来之前不说"未安装"：要起两次 Python，已装好的机器上首屏会误报（复验 C2-07）。 */}
              {!status ? (
                <li>{t('正在检查已经装了什么…', 'Checking what is installed…')}</li>
              ) : (
                <>
                  <li>
                    {t('uv：', 'uv: ')}
                    {status.uv
                      ? `${status.uv_version ?? '?'} · ${status.uv}`
                      : status.uv_version
                        ? status.uv_version
                        : t('未安装', 'not installed')}
                  </li>
                  {/* 服务器上下载 uv 要 curl（或 wget）和 sha256sum（或 shasum）；没有就提前说。 */}
                  {remote && !status.uv_version && (!status.downloader || !status.checksum) && (
                    <li>
                      {t(
                        '这台服务器缺 curl/wget 或 sha256sum/shasum，装不了 uv：请先让管理员装上。',
                        'This server lacks curl/wget or sha256sum/shasum, so uv cannot be installed; ask the administrator to add them.',
                      )}
                    </li>
                  )}
                  {remote && !status.platform && (
                    <li>
                      {t(
                        '没有给这台服务器的系统准备 uv（只支持 Linux 与 macOS 的 x86_64、aarch64）。',
                        'No uv is prepared for this server’s system (only Linux and macOS on x86_64 and aarch64).',
                      )}
                    </li>
                  )}
                  <li>
                    {t('专用 Python：', 'Private Python: ')}
                    {status.python ?? t('未安装', 'not installed')}
                  </li>
                  <li>
                    {t('OpenBench：', 'OpenBench: ')}
                    {status.openbench ?? t('未安装', 'not installed')}
                    {status.openbench &&
                      status.openbench !== status.openbench_pinned &&
                      t(
                        `（软件验证过的是 ${status.openbench_pinned}；装的是别的版本或本地源码，评估结果以它为准）`,
                        ` (the app was verified with ${status.openbench_pinned}; this is another version or a local source, so results follow it)`,
                      )}
                  </li>
                </>
              )}
            </ul>
            <div className="form-grid">
              <Field label={t('OpenBench 从哪装', 'Install OpenBench from')}>
                <Select
                  value={source}
                  onChange={(e) => setSource(e.target.value as 'pypi' | 'local')}
                >
                  <option value="pypi">
                    {t(
                      `PyPI 发布版（colm-openbench ${status?.openbench_pinned ?? ''}）`,
                      `PyPI release (colm-openbench ${status?.openbench_pinned ?? ''})`,
                    )}
                  </option>
                  <option value="local">
                    {remote
                      ? t(
                          '服务器上的源码目录（可编辑安装）',
                          'Source folder on the server (editable install)',
                        )
                      : t('本地源码目录（可编辑安装）', 'Local source folder (editable install)')}
                  </option>
                </Select>
              </Field>
              {source === 'local' && (
                <Field label={t('OpenBench 源码目录', 'OpenBench source folder')}>
                  <div className="study-path">
                    <Input value={localSource} onChange={(e) => setLocalSource(e.target.value)} />
                    {!remote && (
                      <ActionButton
                        action="compare-pick-source"
                        label={t('选择…', 'Choose…')}
                        run={async () => {
                          const dir = await invoke<string | null>('pick_folder', {
                            key: 'openbench-source',
                          });
                          if (dir) setLocalSource(dir);
                        }}
                      />
                    )}
                  </div>
                </Field>
              )}
              {source === 'pypi' && (
                <Field label={t('从哪个 PyPI 下载', 'Package index')}>
                  <Select value={index} onChange={(e) => setIndex(e.target.value)}>
                    {INDEXES.map((option) => (
                      <option key={option.value} value={option.value}>
                        {t(option.zh, option.en)}
                      </option>
                    ))}
                  </Select>
                </Field>
              )}
            </div>
            <div className="button-row">
              <ActionButton
                action="compare-install-all"
                label={
                  !status
                    ? t('一键安装', 'Install everything')
                    : steps.length
                      ? t(
                          `一键安装（${steps.length} 步）`,
                          `Install everything (${steps.length} steps)`,
                        )
                      : t('已全部装好', 'Everything is installed')
                }
                kind="commit"
                variant="primary"
                disabled={busy || !status || !steps.length || (source === 'local' && !localSource)}
                run={() => install(steps)}
              />
              {ready && (
                <ActionButton
                  action="compare-reinstall-openbench"
                  label={t('重装 OpenBench', 'Reinstall OpenBench')}
                  kind="commit"
                  disabled={busy || (source === 'local' && !localSource)}
                  run={() => install(['openbench'])}
                />
              )}
              {busy && <Button onClick={stop}>{t('停止', 'Stop')}</Button>}
              {(status?.python || ready) && (
                <Button disabled={busy} onClick={() => void uninstall('env')}>
                  {t('删掉 OpenBench', 'Remove OpenBench')}
                </Button>
              )}
              {(remote ? !!status?.uv_version : status?.uv_own) && (
                <Button variant="danger" disabled={busy} onClick={() => void uninstall('all')}>
                  {t('全部删掉', 'Remove everything')}
                </Button>
              )}
            </div>
            {log.length > 0 && (
              <pre className="compare-log">
                {log
                  .slice(-80)
                  .map((line) => {
                    // 本机日志带"[阶段] "前缀；服务器作业的日志是原样的行。
                    const m = /^(\[[^\]]*\] )([\s\S]*)$/.exec(line);
                    if (!en) return line;
                    return m ? m[1] + benchLogText(m[2]!) : benchLogText(line);
                  })
                  .join('\n')}
              </pre>
            )}
          </Advanced>
        </Card>
      </div>
      <div data-compare-section="evaluation">
        <Card
          title={t('本次评估', 'This evaluation')}
          description={t(
            '模拟、参考数据、变量、时段、范围与指标；下面的运行前条件摘要来自将要提交的同一份设计。',
            'Simulations, reference data, variables, period, region and metrics; the pre-run summary below comes from the very design that will be submitted.',
          )}
        >
          {ready ? (
            <>
              <h3 className="study-subhead">{t('模拟', 'Simulations')}</h3>
              {remote && (
                <p className="muted study-hint">
                  {t(
                    `在服务器 ${host} 上挑算例目录或 history 目录：本机项目里的算例在服务器上读不到。`,
                    `Choose case or history folders on server ${host}; cases in local projects cannot be read there.`,
                  )}
                </p>
              )}
              <div className="study-cases">
                {(remote ? [] : caseDirs).map((c) => {
                  const on = picks.some((p) => p.dir === c.dir);
                  return (
                    <label key={c.dir} className="param-check">
                      <input
                        type="checkbox"
                        checked={on}
                        disabled={busy}
                        onChange={(e) =>
                          e.target.checked
                            ? void addPick(c.dir, c.name)
                            : setPicks((old) => old.filter((p) => p.dir !== c.dir))
                        }
                      />
                      {c.name}
                    </label>
                  );
                })}
              </div>
              <div className="button-row">
                <ActionButton
                  action="compare-add-folder"
                  label={
                    remote
                      ? t(
                          '加服务器上的算例或 history 目录…',
                          'Add a case or history folder on the server…',
                        )
                      : t(
                          '加一个别的项目的算例或 history 目录…',
                          'Add a case or history folder from another project…',
                        )
                  }
                  disabled={busy}
                  run={async () => {
                    if (remote) return setBrowsing('sim');
                    const dir = await invoke<string | null>('pick_folder', {
                      key: 'openbench-sim',
                    });
                    if (dir) await addPick(dir, pickLabel(dir));
                  }}
                />
              </div>
              {picks.length > 0 && (
                <div className="table-scroll">
                  <table className="versions-table">
                    <thead>
                      <tr>
                        <th>{t('报告里的名字', 'Name in report')}</th>
                        <th>{t('种类', 'Kind')}</th>
                        <th>{t('年份', 'Years')}</th>
                        <th>{t('时间分辨率', 'Time step')}</th>
                        <th />
                      </tr>
                    </thead>
                    <tbody>
                      {picks.map((p) => (
                        <tr key={p.dir}>
                          <td>
                            <Input
                              value={p.label}
                              onChange={(e) =>
                                setPicks((old) =>
                                  old.map((q) =>
                                    q.dir === p.dir ? { ...q, label: e.target.value } : q,
                                  ),
                                )
                              }
                            />
                          </td>
                          <td>
                            {p.error
                              ? p.error
                              : !p.info
                                ? t('读取中…', 'Reading…')
                                : p.info.spatial
                                  ? t('格点', 'Gridded')
                                  : t('单点（站点）', 'Single point (station)')}
                            {p.note && <p className="muted study-hint">{p.note}</p>}
                          </td>
                          <td>{p.info ? `${p.info.syear}–${p.info.eyear}` : '—'}</td>
                          <td>{p.info?.tim_res ?? '—'}</td>
                          <td>
                            <div className="button-row">
                              <Button
                                disabled={busy}
                                title={t(
                                  '编辑这个模拟的目录、类型、时间、前缀和分组',
                                  'Edit this simulation’s folder, kind, time, prefix and grouping',
                                )}
                                onClick={() => editSimulation(p)}
                              >
                                {t('编辑模拟信息', 'Edit simulation information')}
                              </Button>
                              <Button
                                onClick={() =>
                                  setPicks((old) => old.filter((q) => q.dir !== p.dir))
                                }
                              >
                                {t('去掉', 'Remove')}
                              </Button>
                            </div>
                          </td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}

              <h3 className="study-subhead">{t('参考数据', 'Reference data')}</h3>
              <p className="muted">
                {t(
                  `OpenBench 登记表里有 ${references.length} 个参考数据集；自己的数据可以在下面的进阶登记里加入。`,
                  `The OpenBench registry lists ${references.length} reference datasets; add your own under the advanced registration below.`,
                )}
              </p>
              <div className="form-grid">
                <Field label={t('参考数据根目录', 'Reference data root')}>
                  <div className="study-path">
                    <Input value={scanDir} onChange={(e) => setScanDir(e.target.value)} />
                    <ActionButton
                      action="compare-pick-reference-root"
                      label={t('选择…', 'Choose…')}
                      run={async () => {
                        if (remote) return setBrowsing('ref');
                        const dir = await invoke<string | null>('pick_folder', {
                          key: 'openbench-ref',
                        });
                        if (dir) setScanDir(dir);
                      }}
                    />
                  </div>
                </Field>
              </div>
              <div className="button-row">
                <ActionButton
                  action="compare-scan-preview"
                  label={t('预览扫描（只读）', 'Preview scan (read-only)')}
                  disabled={busy || !scanDir}
                  run={() =>
                    task(async () =>
                      setScanText(
                        remote
                          ? (
                              await invokeJson<{ output: string }>('bench_remote_scan', {
                                host,
                                dir: scanDir,
                              })
                            ).output
                          : await invoke<string>('bench_scan_preview', { dir: scanDir }),
                      ),
                    )
                  }
                />
              </div>
              <p className="muted study-hint">
                {t(
                  '预览只调用 OpenBench 的扫描（dry run）：显示参考数据根目录里哪些目录符合标准结构、会被认成什么数据集，不写登记表、不改文件。用来看自己的数据布局能不能被识别；确认后再用「注册参考数据…」正式登记。',
                  'Preview only runs OpenBench’s scan in dry-run mode: it shows which folders under the reference root match the standard layout and what datasets they would become, without writing the registry or changing files. Use it to check a custom layout, then register it with “Register reference data…”.',
                )}
              </p>
              {scanText && <pre className="compare-log">{scanText}</pre>}
              <p className="muted study-hint">
                {t(
                  '注册自己的参考数据用下面的「注册参考数据…」按钮；注册后会出现在下面的变量参考数据选择里。',
                  'Use “Register reference data…” below to add your own dataset; it becomes selectable under the variables below after registration.',
                )}
              </p>
              <Advanced
                title={t('登记 OpenBench 数据（进阶）', 'Register OpenBench data (advanced)')}
              >
                <p className="muted">
                  {t(
                    '新模块模型变量、参考数据和非标准模拟数据都在这里登记；登记后即可在上面的本次评估里选择。本机和每台服务器各管各的登记表。',
                    'Register new-module model variables, reference data and non-standard simulations here; they become selectable in this evaluation above. This computer and each server keep separate registries.',
                  )}
                </p>
                <div className="button-row">
                  <Button
                    disabled={busy || !ready}
                    title={!ready ? t('先安装 OpenBench', 'Install OpenBench first') : undefined}
                    onClick={() => {
                      setError('');
                      setDialog('variable');
                    }}
                  >
                    {t('注册模型变量…', 'Register model variable…')}
                  </Button>
                  <Button
                    disabled={busy || !ready}
                    title={!ready ? t('先安装 OpenBench', 'Install OpenBench first') : undefined}
                    onClick={() => {
                      setError('');
                      setDialog('reference');
                    }}
                  >
                    {t('注册参考数据…', 'Register reference data…')}
                  </Button>
                  <Button
                    disabled={busy || !ready}
                    title={!ready ? t('先安装 OpenBench', 'Install OpenBench first') : undefined}
                    onClick={() => editSimulation()}
                  >
                    {t('注册模拟数据…', 'Register simulation data…')}
                  </Button>
                </div>
              </Advanced>
              <div className="form-grid">
                <Field label={t('比较名', 'Comparison name')}>
                  <Input value={name} onChange={(e) => setName(e.target.value)} />
                </Field>
                <Field label={t('起始年', 'From year')}>
                  <Input type="number" value={from} onChange={(e) => setFrom(e.target.value)} />
                </Field>
                <Field label={t('结束年', 'To year')}>
                  <Input type="number" value={to} onChange={(e) => setTo(e.target.value)} />
                </Field>
                <Field
                  label={t('比较的时间尺度', 'Time scale of the comparison')}
                  hint={t(
                    '气候态是多年平均的评估模式；OpenBench 另有年气候态、月气候态两种时间尺度。',
                    'Climatology evaluates multi-year means; OpenBench also provides annual and monthly climatology scales.',
                  )}
                >
                  <Select value={timRes} onChange={(e) => setTimRes(e.target.value)}>
                    <option value="Hour">{t('逐时', 'Hourly')}</option>
                    <option value="Day">{t('逐日', 'Daily')}</option>
                    <option value="Month">{t('逐月', 'Monthly')}</option>
                    <option value="climatology-year">{t('年气候态', 'Annual climatology')}</option>
                    <option value="climatology-month">
                      {t('月气候态', 'Monthly climatology')}
                    </option>
                  </Select>
                </Field>
                {ok.some((p) => p.info.spatial) && (
                  <Field
                    label={t(
                      '比较的网格分辨率（度；范围按格点模拟覆盖的区域裁）',
                      'Comparison grid resolution (degrees; the region follows the gridded simulations)',
                    )}
                  >
                    <Input
                      type="number"
                      value={gridRes}
                      onChange={(e) => setGridRes(e.target.value)}
                    />
                  </Field>
                )}
              </div>

              <Advanced title={t('OpenBench 高级设置', 'OpenBench advanced settings')}>
                <div className="form-grid">
                  <Field
                    label={t('聚合权重', 'Aggregation weight')}
                    hint={t(
                      '只影响聚合后的 Overall/分数；空=OpenBench 默认 area，通量变量常用 mass。',
                      'Affects aggregated Overall/scores only; empty = OpenBench default area, flux variables often use mass.',
                    )}
                  >
                    <Select value={weight} onChange={(e) => setWeight(e.target.value)}>
                      <option value="">{t('默认（area）', 'Default (area)')}</option>
                      <option value="area">area</option>
                      <option value="mass">mass</option>
                      <option value="none">{t('none（等权）', 'none (equal weight)')}</option>
                    </Select>
                  </Field>
                  <Field
                    label={t('最少有效年数', 'Minimum valid years')}
                    hint={t(
                      '有效数据不足这个年数的记录会被剔除，默认 1；留空则用 OpenBench 默认值。',
                      'Records with fewer valid years are dropped; the default is 1. Empty uses the OpenBench default.',
                    )}
                  >
                    <Input
                      type="number"
                      min={0}
                      value={minYear}
                      onChange={(e) => setMinYear(e.target.value)}
                    />
                  </Field>
                  <Field
                    label={t('时间对齐', 'Time alignment')}
                    hint={t(
                      '模拟与参考时段不一致时怎么取交集。',
                      'How to align simulation and reference periods.',
                    )}
                  >
                    <Select
                      value={timeAlignment}
                      onChange={(e) => setTimeAlignment(e.target.value)}
                    >
                      <option value="">
                        {t('默认（intersection）', 'Default (intersection)')}
                      </option>
                      <option value="intersection">intersection</option>
                      <option value="per_pair">per_pair</option>
                      <option value="strict">strict</option>
                    </Select>
                  </Field>
                  <Field
                    label={t('并行核数', 'Parallel cores')}
                    hint={t(
                      '空=OpenBench 用全部 CPU；共享机器建议填。',
                      'Empty = all CPUs; set this on shared machines.',
                    )}
                  >
                    <Input
                      type="number"
                      min={0}
                      value={numCores}
                      onChange={(e) => setNumCores(e.target.value)}
                    />
                  </Field>
                  <Field
                    label={t('指标 metrics', 'Metrics')}
                    hint={t(
                      '从当前机器已安装的 OpenBench 读取；勾选要计算的指标，至少保留一个指标或评分。',
                      'Read from the installed OpenBench on this machine; tick the metrics to compute. Keep at least one metric or score.',
                    )}
                  >
                    <div className="metric-picker">
                      <div className="button-row">
                        <Button
                          variant="ghost"
                          onClick={() => setMetrics(metricOptions.map((option) => option.name))}
                        >
                          {t('全选', 'All')}
                        </Button>
                        <Button variant="ghost" onClick={() => setMetrics([])}>
                          {t('清空', 'Clear')}
                        </Button>
                      </div>
                      <div className="metric-options">
                        {metricOptions.map((option) => (
                          <label key={option.name} className="param-check">
                            <input
                              type="checkbox"
                              checked={metrics.includes(option.name)}
                              onChange={(e) =>
                                setMetrics((old) =>
                                  e.target.checked
                                    ? [...old, option.name]
                                    : old.filter((name) => name !== option.name),
                                )
                              }
                            />
                            <span>{option.label}</span>
                            <span className="muted">{option.name}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </Field>
                  <Field
                    label={t('评分 scores', 'Scores')}
                    hint={t(
                      'OpenBench 的归一化评分；与指标至少保留一个。',
                      'OpenBench normalized scores; keep at least one metric or score.',
                    )}
                  >
                    <div className="metric-picker">
                      <div className="button-row">
                        <Button
                          variant="ghost"
                          onClick={() => setScores(scoreOptions.map((option) => option.name))}
                        >
                          {t('全选', 'All')}
                        </Button>
                        <Button variant="ghost" onClick={() => setScores([])}>
                          {t('清空', 'Clear')}
                        </Button>
                      </div>
                      <div className="metric-options">
                        {scoreOptions.map((option) => (
                          <label key={option.name} className="param-check">
                            <input
                              type="checkbox"
                              checked={scores.includes(option.name)}
                              onChange={(e) =>
                                setScores((old) =>
                                  e.target.checked
                                    ? [...old, option.name]
                                    : old.filter((name) => name !== option.name),
                                )
                              }
                            />
                            <span>{option.label}</span>
                            <span className="muted">{option.name}</span>
                          </label>
                        ))}
                      </div>
                    </div>
                  </Field>
                  <Field
                    label={t('跨模拟比较图表', 'Cross-simulation comparison charts')}
                    hint={t(
                      '勾选的图表写入配置的 comparison.items；一个都不勾表示不启用跨模拟比较，且至少两个模拟才会写入。HeatMap 至少需要一个评分、Taylor_Diagram 至少需要一个指标；其它图表缺数据时 OpenBench check 会明确报错。',
                      'Ticked charts are written to comparison.items; ticking none disables the comparison phase, and it is only emitted with at least two simulations. HeatMap needs a score and Taylor_Diagram a metric; other charts fail loudly in openbench check when their inputs are missing.',
                    )}
                  >
                    <div className="button-row">
                      <Button
                        variant="ghost"
                        onClick={() =>
                          setComparisonItems(COMPARISON_ITEM_CHOICES.map((item) => item.name))
                        }
                      >
                        {t('全选', 'All')}
                      </Button>
                      <Button variant="ghost" onClick={() => setComparisonItems([])}>
                        {t('清空（不启用）', 'Clear (disable)')}
                      </Button>
                    </div>
                    <div className="metric-options">
                      {COMPARISON_ITEM_CHOICES.map((item) => (
                        <label key={item.name} className="param-check">
                          <input
                            type="checkbox"
                            checked={comparisonItems.includes(item.name)}
                            onChange={(e) =>
                              setComparisonItems((old) =>
                                e.target.checked
                                  ? [...old, item.name]
                                  : old.filter((name) => name !== item.name),
                              )
                            }
                          />
                          <span>{item.name}</span>
                          {item.needs && (
                            <span className="muted">
                              {item.needs === 'metrics'
                                ? t('需要指标', 'needs a metric')
                                : t('需要评分', 'needs a score')}
                            </span>
                          )}
                        </label>
                      ))}
                    </div>
                  </Field>
                  <Field
                    label={t('统计检验', 'Statistical tests')}
                    hint={t(
                      'OpenBench 独立的统计检验阶段（statistics），默认不启用；勾选的方法才写进配置。Basic 与 False_Discovery_Rate 在 OpenBench 3.0.8 的配置驱动统计里跑不了（运行器缺实现/数据条目不匹配），因此不在可选项里；Three_Cornered_Hat 需要至少两个格点模拟。统计只用格点参考数据，纯单点或参考全是单点时会没有结果。',
                      'OpenBench’s independent statistical-analysis phase (statistics), disabled by default; only ticked methods are written. Basic and False_Discovery_Rate cannot run in OpenBench 3.0.8 config-driven statistics (missing implementation / mismatched data entries), so they are not offered; Three_Cornered_Hat needs at least two gridded simulations. Statistics only use gridded references, so station-only designs or station-only references produce no output.',
                    )}
                  >
                    <div className="button-row">
                      <Button
                        variant="ghost"
                        onClick={() => setStatisticsItems([...STATISTICS_ITEMS])}
                      >
                        {t('全选', 'All')}
                      </Button>
                      <Button variant="ghost" onClick={() => setStatisticsItems([])}>
                        {t('清空（不启用）', 'Clear (disable)')}
                      </Button>
                    </div>
                    <div className="metric-options">
                      {STATISTICS_ITEMS.map((name) => (
                        <label key={name} className="param-check">
                          <input
                            type="checkbox"
                            checked={statisticsItems.includes(name)}
                            onChange={(e) =>
                              setStatisticsItems((old) =>
                                e.target.checked
                                  ? [...old, name]
                                  : old.filter((item) => item !== name),
                              )
                            }
                          />
                          <span>{name}</span>
                        </label>
                      ))}
                    </div>
                    {statisticsItems.length > 0 && statisticsWarnings.length > 0 && (
                      <Notice tone="warning">
                        <ul>
                          {statisticsWarnings.map(([zh, en]) => (
                            <li key={en}>{t(zh, en)}</li>
                          ))}
                        </ul>
                      </Notice>
                    )}
                  </Field>
                </div>
              </Advanced>

              <h3 className="study-subhead">{t('变量和参考数据', 'Variables and references')}</h3>
              {!ok.length ? (
                <p className="muted study-hint">
                  {t(
                    '先在上面选模拟，再一个个加要评估的变量，给每个变量挑参考数据。',
                    'Pick simulations above first, then add the variables to evaluate one by one and choose a reference for each.',
                  )}
                </p>
              ) : (
                <div className="form-grid">
                  <Field label={t('筛选变量', 'Filter variables')}>
                    <Input
                      value={variableQuery}
                      placeholder={t('如 Heat、Runoff', 'e.g. Heat, Runoff')}
                      onChange={(e) => setVariableQuery(e.target.value)}
                    />
                  </Field>
                  <Field label={t('加一个要评估的变量', 'Add a variable to evaluate')}>
                    <Select
                      value=""
                      onChange={(e) => {
                        const picked = allVariables.find((v) => v.variable === e.target.value);
                        // 默认选第一份参考数据：同种（站点或格点）的排在前面。
                        if (picked)
                          setChosen((old) => ({
                            ...old,
                            [picked.variable]: picked.references[0]
                              ? [picked.references[0].name]
                              : [],
                          }));
                      }}
                    >
                      <option value="">
                        {t(
                          `选择（${variables.length} 个可选）`,
                          `Choose (${variables.length} available)`,
                        )}
                      </option>
                      {variables
                        .filter((v) => !(v.variable in chosen))
                        .map((v) => (
                          <option key={v.variable} value={v.variable}>
                            {v.variable}
                          </option>
                        ))}
                    </Select>
                  </Field>
                </div>
              )}
              <div className="form-grid">
                {Object.keys(chosen)
                  .sort()
                  .map((variable) => {
                    const options =
                      allVariables.find((v) => v.variable === variable)?.references ?? [];
                    const selected = chosen[variable] ?? [];
                    return (
                      <Field key={variable} label={variable}>
                        <div className="reference-picker">
                          <div className="reference-options">
                            {options.map((r) => {
                              const on = selected.includes(r.name);
                              return (
                                <label key={r.name} className="param-check">
                                  <input
                                    type="checkbox"
                                    checked={on}
                                    onChange={(e) =>
                                      setChosen((old) => {
                                        const current = old[variable] ?? [];
                                        const next = e.target.checked
                                          ? [...current, r.name]
                                          : current.filter((name) => name !== r.name);
                                        return { ...old, [variable]: next };
                                      })
                                    }
                                  />
                                  {r.name}
                                  {r.data_type ? ` · ${r.data_type}` : ''}
                                  {r.tim_res ? ` · ${r.tim_res}` : ''}
                                </label>
                              );
                            })}
                          </div>
                          <Button
                            onClick={() =>
                              setChosen((old) => {
                                const next = { ...old };
                                delete next[variable];
                                return next;
                              })
                            }
                          >
                            {t('去掉', 'Remove')}
                          </Button>
                        </div>
                      </Field>
                    );
                  })}
              </div>

              <h3 className="study-subhead">{t('运行前条件摘要', 'Pre-run condition summary')}</h3>
              <p className="muted study-hint">
                {t(
                  '下面这些就是运行时会提交的评估条件：配置部分从当前文本解析（手改配置，摘要跟着变），单点模拟的站点目录/前缀/坐标来自生成配置时冻结的站点清单。',
                  'These are the conditions that will be submitted: the configuration part is parsed from the current text (hand edits show up immediately), and the station folders/prefixes/coordinates of station simulations come from the station list frozen when the configuration was generated.',
                )}
              </p>
              <ul className="compare-status" data-compare-summary>
                {conditionSummary().map((item) => (
                  <li key={item.label[0]}>
                    <strong>{t(item.label[0], item.label[1])}</strong>：{item.value}
                  </li>
                ))}
              </ul>
              <h3 className="study-subhead">{t('评估配置', 'Evaluation configuration')}</h3>
              <div className="button-row">
                <SaveBadge
                  state={configState}
                  t={t}
                  labels={{
                    dirty: t('有未生成的修改', 'Changes not generated'),
                    saving: t('正在写入项目目录', 'Writing to the project folder'),
                    saved:
                      configState === 'saved' && writtenFingerprint === effectiveFingerprint
                        ? t('已写入项目目录', 'Written to the project folder')
                        : t('已生成（未写盘）', 'Generated (not on disk)'),
                  }}
                />
                <p className="muted study-hint">
                  {t(
                    '「生成配置」只在页面里生成文本（可以手工修改）；点「检查并运行」时才写入比较项目目录。',
                    '“Generate configuration” only produces the text on this page (editable); the configuration is written into the comparison project folder when you check and run.',
                  )}
                </p>
                <ActionButton
                  action="compare-generate"
                  label={t('生成配置', 'Generate configuration')}
                  disabled={busy || !ok.length}
                  run={regenerate}
                />
              </div>
              {runNotice && <Notice tone="success">{runNotice}</Notice>}
              {yaml && (
                <>
                  <Field
                    label={t(
                      'OpenBench 配置（可以先改再运行）',
                      'OpenBench configuration (edit before running if needed)',
                    )}
                  >
                    <textarea
                      className="code-files-text"
                      data-field="openbench-config"
                      rows={16}
                      spellCheck={false}
                      value={yaml}
                      onChange={(e) => setYaml(e.target.value)}
                    />
                  </Field>
                  {/* 过期快照下指纹退回表单身份（完成提示要失效），那段值不代表会被提交的输入，不显示。 */}
                  {effectiveConfig && !staleSnapshot && (
                    <p className="muted study-hint" data-compare-effective>
                      {t(
                        `当前有效输入指纹 ${effectiveFingerprint}：配置从这段文本解析、站点清单用生成配置时冻结的那份；运行时提交这两部分并绑定报告。`,
                        `Effective input fingerprint ${effectiveFingerprint}: the configuration is parsed from this text and the station list is the one frozen when it was generated; running submits both and binds the report to them.`,
                      )}
                    </p>
                  )}
                  {configProblems.length > 0 && (
                    <Notice tone="error">
                      <strong>
                        {t('这段配置还不能运行：', 'This configuration cannot run yet:')}
                      </strong>
                      <ul>
                        {configProblems.map((problem) => (
                          <li key={problem[0]}>{t(problem[0], problem[1])}</li>
                        ))}
                      </ul>
                    </Notice>
                  )}
                  {structureProblems.length > 0 && (
                    <Notice tone="error">
                      <strong>
                        {t(
                          '配置与页面选中的模拟对不上，不能运行：',
                          'The configuration does not match the simulations picked on the page and cannot run:',
                        )}
                      </strong>
                      <ul>
                        {structureProblems.map((problem) => (
                          <li key={problem[0]}>{t(problem[0], problem[1])}</li>
                        ))}
                      </ul>
                    </Notice>
                  )}
                  {staleSnapshot && (
                    <Notice tone="error">
                      {t(
                        '生成配置之后你又在上面改过评估设置：页面冻结的站点清单/模拟输入还是生成时的，和下面这份配置不是同一份输入，所以不能运行。请重新生成评估配置（会覆盖手改内容）后再运行。',
                        'You changed the evaluation settings after generating the configuration: the station list and simulation inputs frozen at generation no longer match the configuration below, so it cannot run. Regenerate the evaluation configuration (this replaces your edits) before running.',
                      )}
                    </Notice>
                  )}
                  {lastRun && (
                    <p className="muted study-hint" data-compare-last-run>
                      {t(
                        `上次运行 ${lastRun.runId || '（本机）'} · 配置指纹 ${lastRun.fingerprint}${lastRun.configSha ? ` · 配置哈希 ${lastRun.configSha.slice(0, 12)}` : ''} · ${lastRun.at}`,
                        `Last run ${lastRun.runId || '(this computer)'} · configuration fingerprint ${lastRun.fingerprint}${lastRun.configSha ? ` · config hash ${lastRun.configSha.slice(0, 12)}` : ''} · ${lastRun.at}`,
                      )}
                    </p>
                  )}
                  {ok.some((p) => (p.info.unsupported_variables ?? []).length > 0) && (
                    <Notice tone="warning">
                      {t(
                        `这些变量超出单点/地表评估维度，制备站点文件时会被跳过（不会静默丢掉，日志里会写明）：${ok
                          .flatMap((p) =>
                            (p.info.unsupported_variables ?? []).map(
                              (v) => `${p.label}: ${v.name}`,
                            ),
                          )
                          .join(
                            '；',
                          )}。土层/积雪层/湖泊层变量（soil/soilsnow/lake）不在这里：它们会保留层维度，交给 OpenBench 模型 profile 的 compute 处理。`,
                        `These variables have dimensions beyond a station/surface evaluation and are skipped when preparing station files (not silently: the log names them): ${ok
                          .flatMap((p) =>
                            (p.info.unsupported_variables ?? []).map(
                              (v) => `${p.label}: ${v.name}`,
                            ),
                          )
                          .join(
                            '; ',
                          )}. Soil/snow/lake layer variables are not in this list: their layer dimensions are kept for the OpenBench model profile's compute.`,
                      )}
                    </Notice>
                  )}
                  {layerWarnings.length > 0 && (
                    <Notice tone="info">
                      <ul>
                        {layerWarnings.map(([zh, en]) => (
                          <li key={en}>{t(zh, en)}</li>
                        ))}
                      </ul>
                    </Notice>
                  )}
                  {missingExtent.length > 0 && (
                    <Notice tone="warning">
                      {t(
                        `这些格点模拟读不出区域范围：${missingExtent.join('、')}。配置里只写了目标分辨率，没有区域范围（OpenBench 会按默认的全球范围比较）；请换一个带坐标的 history 目录重新读数，或在编辑模拟里补上区域范围。`,
                        `Could not read the region of: ${missingExtent.join(', ')}. The configuration sets only the target resolution, without a region (OpenBench then compares on the global grid); inspect a history folder that has coordinates, or fill in the region in the simulation dialog.`,
                      )}
                    </Notice>
                  )}
                  <div className="button-row">
                    {!scanDir && (
                      <p className="muted study-hint">
                        {t(
                          '还没填"参考数据根目录"：OpenBench 自带的参考数据都按这个目录找，不填只能用自己登记了完整路径的数据；填好后要重新生成评估配置。',
                          'No reference data root yet: OpenBench’s built-in references are looked up under it; without it only references registered with full paths work. Regenerate the evaluation configuration after filling it in.',
                        )}
                      </p>
                    )}
                    {formChanged && (
                      <Button disabled={busy} onClick={() => void regenerate()}>
                        {t('重新生成评估配置', 'Regenerate evaluation configuration')}
                      </Button>
                    )}
                    <ActionButton
                      action="compare-run"
                      label={t('检查并运行', 'Check and run')}
                      kind="commit"
                      variant="primary"
                      disabled={
                        busy ||
                        !generatedConfig ||
                        staleSnapshot ||
                        !effectiveConfig ||
                        configProblems.length > 0 ||
                        structureProblems.length > 0
                      }
                      title={
                        !generatedConfig
                          ? t(
                              '还没有生成评估配置：先点「生成配置」',
                              'No evaluation configuration yet: save one first',
                            )
                          : staleSnapshot
                            ? t(
                                '生成配置之后页面设置又改过：先重新生成评估配置',
                                'The page settings changed after the configuration was generated: regenerate it first',
                              )
                            : !effectiveConfig || configProblems.length > 0
                              ? t(
                                  '配置还不能运行：先修正上面的问题',
                                  'The configuration cannot run yet: fix the problems above first',
                                )
                              : structureProblems.length > 0
                                ? t(
                                    '配置与页面的模拟对不上：先重新生成评估配置',
                                    'The configuration does not match the page simulations: regenerate it first',
                                  )
                                : undefined
                      }
                      run={run}
                    />
                  </div>
                  {yamlHandEdited && (
                    <p className="muted study-hint">
                      {t(
                        '这份配置有你手工改过的内容：运行时提交的是「生成配置时冻结的站点清单 + 你改后的配置文本」，摘要与指纹按这两部分算。',
                        'This configuration contains edits you made: running submits the station list frozen when it was generated plus your edited configuration text, and the summary and fingerprint follow both.',
                      )}
                    </p>
                  )}
                </>
              )}
            </>
          ) : (
            <p className="muted study-hint">
              {t(
                '安装 OpenBench 之后，这里出现模拟、参考数据、变量、时段、范围和评估配置。',
                'Once OpenBench is installed, the simulations, reference data, variables, period, region and evaluation configuration appear here.',
              )}
            </p>
          )}
        </Card>
      </div>
      <div data-compare-section="jobs">
        <Card
          title={t('任务与报告', 'Jobs and reports')}
          description={t(
            '运行进度、失败原因、远程作业身份、已有报告和后续操作都在这里。',
            'Run progress, failure reasons, remote job identity, existing reports and follow-up actions are collected here.',
          )}
        >
          {busy && <p className="muted study-hint">{t('正在处理…', 'Working…')}</p>}
          {activeJob && (
            <p className="muted study-hint">
              {t(
                `正在等待服务器作业 ${activeJob.job}${activeJob.name ? `（比较 ${activeJob.name}）` : ''}…`,
                `Waiting for server job ${activeJob.job}${activeJob.name ? ` (comparison ${activeJob.name})` : ''}…`,
              )}
            </p>
          )}
          {lastJob && !activeJob && !unsettled && (
            <div className="button-row">
              <p className="muted study-hint">
                {t(
                  `上次远程作业 ${lastJob.job}（${lastJob.name ?? '安装环境'}）：${jobEndText(lastJob)}。`,
                  `Last remote job ${lastJob.job} (${lastJob.name ?? 'environment install'}): ${jobEndText(lastJob)}.`,
                )}
              </p>
              {lastJob.end === 'succeeded' && lastJob.name && (
                <Button disabled={busy} onClick={() => void fetchLastReport()}>
                  {t('再取回报告', 'Fetch report again')}
                </Button>
              )}
            </div>
          )}
          {lastJob && !activeJob && !unsettled && lastJob.end === 'failed' && lastJob.lastError && (
            <p className="muted study-hint">
              {t('上次远程作业失败的原因：', 'Reason of the last remote job failure: ')}
              {shown(lastJob.lastError)}
            </p>
          )}
          {remote && unsettled && (
            <div data-compare-subsection="unsettled">
              <h3 className="study-subhead">
                {t('远程作业：暂时失联/待核对', 'Remote job: unreachable, awaiting check')}
              </h3>
              <Notice tone="warning">
                {t(
                  `暂时联系不上服务器 ${host}：作业 ${unsettled.job.job}（${unsettled.job.name ?? '安装环境'}）的身份已经保留，远端可能仍在运行。`,
                  `Server ${host} cannot be reached right now; job ${unsettled.job.job} (${unsettled.job.name ?? 'environment install'}) is still tracked and may still be running.`,
                )}
              </Notice>
              <p className="muted study-hint">
                {t(
                  `提交时间：${unsettled.job.submittedAt ?? '未知'}；最后确认状态：${unsettled.job.lastState ?? '未知'}${unsettled.job.lastConfirmedAt ? `（${unsettled.job.lastConfirmedAt}）` : ''}。一次查询失败不等于作业失败，确认终态之前不会结束跟踪。`,
                  `Submitted: ${unsettled.job.submittedAt ?? 'unknown'}; last confirmed state: ${unsettled.job.lastState ?? 'unknown'}${unsettled.job.lastConfirmedAt ? ` (${unsettled.job.lastConfirmedAt})` : ''}. A failed query does not mean the job failed; tracking ends only once the final state is confirmed.`,
                )}
              </p>
              {unsettled.reason && <p className="muted study-hint">{shown(unsettled.reason)}</p>}
              <div className="button-row">
                <Button disabled={busy} onClick={() => void reconnect()}>
                  {t('重连并继续等待', 'Reconnect and keep waiting')}
                </Button>
                <Button disabled={busy} onClick={() => void checkUnsettled()}>
                  {t('核对终态', 'Check final state')}
                </Button>
                <Button variant="danger" disabled={busy} onClick={() => void stopUnsettled()}>
                  {t('停止作业', 'Stop job')}
                </Button>
              </div>
            </div>
          )}
          <div data-compare-subsection="reports">
            <h3 className="study-subhead">{t('报告', 'Reports')}</h3>
            <p className="muted">
              {t(
                '这里跑过的比较和它们的报告；也能载入别处生成的 OpenBench 报告。',
                'Comparisons run here and their reports; reports produced elsewhere can be loaded too.',
              )}
            </p>
            {status?.projects.length ? (
              <div className="table-scroll">
                <table className="versions-table">
                  <tbody>
                    {status.projects.map((p) => (
                      <tr key={p.dir ?? p.name}>
                        <td>{p.name}</td>
                        <td data-compare-project-run>
                          {p.latest_run ? (
                            <>
                              {p.latest_run.state === 'succeeded'
                                ? t('最近运行：成功', 'last run: succeeded')
                                : p.latest_run.state === 'failed'
                                  ? t('最近运行：失败', 'last run: failed')
                                  : t('最近运行：进行中', 'last run: running')}
                              {` · ${p.latest_run.run_id}`}
                              {p.latest_run.config_sha
                                ? ` · ${p.latest_run.config_sha.slice(0, 12)}`
                                : ''}
                              {p.latest_run.state === 'failed' && p.latest_run.error && (
                                <span className="muted"> — {shown(p.latest_run.error)}</span>
                              )}
                            </>
                          ) : (
                            t('还没有运行记录', 'no run recorded yet')
                          )}
                          {p.latest_success && p.report && (
                            <p className="muted study-hint">
                              {t(
                                `报告来自成功运行 ${p.latest_success.run_id}；之后失败的重跑不会覆盖它。`,
                                `The report is from successful run ${p.latest_success.run_id}; a later failed run does not overwrite it.`,
                              )}
                            </p>
                          )}
                        </td>
                        <td>
                          {p.report
                            ? t('有报告', 'has a report')
                            : t('还没有报告', 'no report yet')}
                        </td>
                        <td>
                          {/* 服务器上的报告先取回本机（报告、指标与配置），再在软件里看。 */}
                          {p.report && remote && (
                            <Button
                              onClick={() =>
                                void task(() =>
                                  fetchReport(host, p.name, p.latest_success?.run_id ?? undefined),
                                )
                              }
                            >
                              {t('取回并查看', 'Fetch and view')}
                            </Button>
                          )}
                          {p.report && !remote && (
                            <>
                              <Button onClick={() => void task(() => view(p.report!))}>
                                {t('在软件里看', 'View here')}
                              </Button>
                              <Button onClick={() => void invoke('open_path', { path: p.report })}>
                                {t('用浏览器打开', 'Open in browser')}
                              </Button>
                            </>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            ) : (
              <p className="muted">{t('还没有比较。', 'No comparisons yet.')}</p>
            )}
            {status?.projects.some((p) => (p.runs?.length ?? 0) > 1) && (
              <details className="study-fold" data-compare-run-history>
                <summary>{t('历史运行', 'Run history')}</summary>
                {status.projects
                  .filter((p) => (p.runs?.length ?? 0) > 1)
                  .map((p) => (
                    <div key={p.name} data-compare-project-runs={p.name}>
                      <h4 className="study-subhead">{p.name}</h4>
                      <ul className="compare-status">
                        {p.runs!.map((run) => (
                          <li key={run.run_id}>
                            <strong>{run.run_id}</strong> · {run.state}
                            {run.finished_at ? ` · ${run.finished_at}` : ''}
                            {run.report && (
                              <Button onClick={() => void task(() => view(run.report!))}>
                                {t('查看这次报告', 'View this report')}
                              </Button>
                            )}
                            {run.state === 'failed' && run.error && (
                              <span className="muted"> — {shown(run.error)}</span>
                            )}
                          </li>
                        ))}
                      </ul>
                    </div>
                  ))}
              </details>
            )}
            <div className="button-row">
              <ActionButton
                action="compare-load-report"
                label={t('载入报告…', 'Load a report…')}
                run={() =>
                  task(async () => {
                    // 选文件和读文件都在后端：前端不能拿一个任意路径去读（复验 D2-05）。
                    const picked = await invokeJson<{ path: string; html: string } | null>(
                      'bench_pick_report',
                    );
                    if (picked) setReport(picked);
                  })
                }
              />
            </div>
            {report && (
              <>
                <p className="muted">{report.path}</p>
                {(() => {
                  const bound = status?.projects.find((p) => p.report === report.path);
                  const success = bound?.latest_success;
                  const runId =
                    lastRun && lastRun.report === report.path ? lastRun.runId : success?.run_id;
                  const sha =
                    lastRun && lastRun.report === report.path
                      ? lastRun.configSha
                      : success?.config_sha;
                  if (!runId) return null;
                  return (
                    <p className="muted" data-compare-report-run>
                      {t(
                        `这份报告属于成功运行 ${runId}${sha ? `（配置哈希 ${sha.slice(0, 12)}）` : ''}；之后失败的重跑不会覆盖它。`,
                        `This report belongs to successful run ${runId}${sha ? ` (config hash ${sha.slice(0, 12)})` : ''}; a later failed run does not overwrite it.`,
                      )}
                    </p>
                  );
                })()}
                {/* 报告放进完全的沙箱：不跑脚本、不给同源权限。OpenBench 3.0.8 的报告里没有一个 <script>
                （实测两份都是 0 个），用不着脚本；Windows 上 wry 会往子框架注入 IPC 初始化脚本，
                允许脚本就只剩 origin 校验一层兜底（复验 D2-04）。 */}
                <iframe
                  className="compare-report"
                  title={t('OpenBench 报告', 'OpenBench report')}
                  sandbox=""
                  srcDoc={report.html}
                />
              </>
            )}
          </div>
        </Card>
      </div>
      <Dialog
        open={dialog === 'variable'}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t('注册模型变量', 'Register a model variable')}
        description={t(
          '新模块的输出变量如果不在 CoLM2024 profile 里，先在这里登记；参考数据里也要有同名标准变量才能评估。',
          'Register a new module output variable that is missing from the CoLM2024 profile; the reference data must also expose the same standard variable before it can be evaluated.',
        )}
      >
        <div className="form-grid">
          <Field label={t('模型 profile', 'Model profile')}>
            <Input
              value={modelVar.model}
              onChange={(e) => setModelVar((old) => ({ ...old, model: e.target.value }))}
            />
          </Field>
          <Field
            label={t('标准变量名', 'Standard variable')}
            hint={t('如 New_Module_Flux', 'e.g. New_Module_Flux')}
          >
            <Input
              value={modelVar.variable}
              onChange={(e) => setModelVar((old) => ({ ...old, variable: e.target.value }))}
            />
          </Field>
          <Field label={t('NetCDF 变量名', 'Variable name in NetCDF')}>
            <Input
              value={modelVar.varname}
              onChange={(e) => setModelVar((old) => ({ ...old, varname: e.target.value }))}
            />
          </Field>
          <Field label={t('单位', 'Unit')}>
            <Input
              value={modelVar.unit}
              onChange={(e) => setModelVar((old) => ({ ...old, unit: e.target.value }))}
            />
          </Field>
        </div>
        {/* 弹窗盖住了页面上的错误提示：注册失败的原因要在弹窗里看得到。 */}
        {error && <Notice tone="error">{error}</Notice>}
        <div className="button-row">
          <Button disabled={busy} onClick={() => void registerModelVariable()}>
            {t('注册', 'Register')}
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => setDialog(null)}>
            {t('取消', 'Cancel')}
          </Button>
        </div>
      </Dialog>

      <Dialog
        open={dialog === 'reference'}
        onOpenChange={(open) => !open && setDialog(null)}
        title={t('注册参考数据', 'Register reference data')}
        description={t(
          '登记到当前所选机器（本机或服务器）的 OpenBench 登记表；变量行格式：标准变量:文件变量:单位[:前缀[:后缀]]。',
          'Register into the current machine’s OpenBench registry; variable lines use standard:file variable:unit[:prefix[:suffix]].',
        )}
      >
        <div className="form-grid">
          {(
            [
              ['name', t('名字', 'Name')],
              ['rootDir', t('数据目录', 'Data folder')],
              ['fulllist', t('站点清单 CSV（站点数据要）', 'Station list CSV (station data)')],
              ['from', t('起始年', 'From year')],
              ['to', t('结束年', 'To year')],
            ] as const
          ).map(([key, label]) => (
            <Field key={key} label={label}>
              {key === 'rootDir' || key === 'fulllist' ? (
                <div className="study-path">
                  <Input
                    value={reg[key]}
                    onChange={(e) => setReg((old) => ({ ...old, [key]: e.target.value }))}
                  />
                  <Button
                    variant="ghost"
                    disabled={busy}
                    onClick={key === 'rootDir' ? pickReferenceDir : pickReferenceList}
                  >
                    {key === 'rootDir' ? t('选择…', 'Choose…') : t('选择 CSV…', 'Choose CSV…')}
                  </Button>
                </div>
              ) : (
                <Input
                  value={reg[key]}
                  onChange={(e) => setReg((old) => ({ ...old, [key]: e.target.value }))}
                />
              )}
            </Field>
          ))}
          <Field label={t('类型', 'Type')}>
            <Select
              value={reg.dataType}
              onChange={(e) => setReg((old) => ({ ...old, dataType: e.target.value }))}
            >
              <option value="stn">{t('站点', 'Station')}</option>
              <option value="grid">{t('格点', 'Grid')}</option>
            </Select>
          </Field>
          <Field label={t('时间分辨率', 'Time resolution')}>
            <Select
              value={reg.timRes}
              onChange={(e) => setReg((old) => ({ ...old, timRes: e.target.value }))}
            >
              {['Hour', '3Hour', '6Hour', '8Day', 'Day', 'Month', 'Year'].map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </Select>
          </Field>
          <Field
            label={t(
              '变量（每行 标准变量:文件变量:单位[:前缀[:后缀]]）',
              'Variables (one per line: standard:file variable:unit[:prefix[:suffix]])',
            )}
          >
            <textarea
              className="code-files-text compare-text"
              rows={4}
              value={reg.variables}
              onChange={(e) => setReg((old) => ({ ...old, variables: e.target.value }))}
            />
          </Field>
        </div>
        {error && <Notice tone="error">{error}</Notice>}
        <div className="button-row">
          <Button
            disabled={busy || !reg.name || !reg.rootDir || !reg.variables.trim()}
            onClick={() => void registerReference()}
          >
            {t('注册', 'Register')}
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => setDialog(null)}>
            {t('取消', 'Cancel')}
          </Button>
        </div>
      </Dialog>

      <Dialog
        open={dialog === 'simulation'}
        onOpenChange={(open) => !open && setDialog(null)}
        title={
          simEditing
            ? t('编辑模拟信息', 'Edit simulation information')
            : t('注册模拟数据', 'Register simulation data')
        }
        description={t(
          '手动登记一个 history/算例目录，并登记新模块在这个模拟里的变量名和单位；这些变量覆盖只用于本次比较。',
          'Add a history/case folder manually and register the new module’s variable names/units for this simulation; these overrides apply only to this comparison.',
        )}
      >
        {simError && <Notice tone="error">{simError.message}</Notice>}
        <div className="form-grid">
          <Field label={t('目录', 'Folder')} hint={simHint('dir')}>
            <div className="study-path">
              <Input
                data-sim-field="dir"
                value={simForm.dir}
                onChange={(e) => changeSimulationDir(e.target.value)}
              />
              <Button
                disabled={busy}
                onClick={() => {
                  if (remote) setBrowsing('simRegister');
                  else
                    void invoke<string | null>('pick_folder', { key: 'openbench-sim' }).then(
                      (dir) => {
                        if (dir) changeSimulationDir(dir);
                      },
                    );
                }}
              >
                {t('选择…', 'Choose…')}
              </Button>
              <Button
                disabled={busy || !simForm.dir.trim()}
                onClick={() => void inspectSimulation()}
              >
                {t('读取概况', 'Inspect')}
              </Button>
            </div>
          </Field>
          <Field label={t('报告里的名字', 'Name in report')}>
            <Input
              value={simForm.label}
              onChange={(e) => setSimForm((old) => ({ ...old, label: e.target.value }))}
            />
          </Field>
          <Field
            label={t('类型', 'Type')}
            hint={simHint(
              'dataType',
              t(
                '格点/站点决定配置里的 data_type 和是否生成站点清单；改类型后旧配置会作废。',
                'Grid or station decides data_type in the configuration and whether a station list is written; changing it invalidates the old configuration.',
              ),
            )}
          >
            <Select
              data-sim-field="dataType"
              value={simForm.dataType}
              onChange={(e) =>
                setSimForm((old) => ({
                  ...old,
                  dataType: e.target.value as SimForm['dataType'],
                }))
              }
            >
              <option value="auto">{t('自动/按概况', 'Auto/inspected')}</option>
              <option value="grid">{t('格点', 'Grid')}</option>
              <option value="stn">{t('站点', 'Station')}</option>
            </Select>
          </Field>
          <Field
            label={t('文件前缀', 'File prefix')}
            hint={simHint(
              'prefix',
              t(
                'history 文件名里 `_hist_` 之前的部分；留空沿用读取到的值。',
                'The part before `_hist_` in the history file names; leave it empty to use the inspected one.',
              ),
            )}
          >
            <Input
              data-sim-field="prefix"
              value={simForm.prefix}
              onChange={(e) => setSimForm((old) => ({ ...old, prefix: e.target.value }))}
            />
          </Field>
          <Field label={t('时间分辨率', 'Time resolution')} hint={simHint('timRes')}>
            <Input
              data-sim-field="timRes"
              value={simForm.timRes}
              onChange={(e) => setSimForm((old) => ({ ...old, timRes: e.target.value }))}
            />
          </Field>
          <Field
            label={t('模型 profile', 'Model profile')}
            hint={simHint(
              'profile',
              t(
                'OpenBench 的 model 字段（默认 CoLM2024）；装了别的模型 profile 时可改，OpenBench check 会校验名字。',
                'The OpenBench model field (default CoLM2024); change it when another model profile is installed — openbench check validates the name.',
              ),
            )}
          >
            <Input
              data-sim-field="profile"
              value={simForm.profile}
              onChange={(e) => setSimForm((old) => ({ ...old, profile: e.target.value }))}
            />
          </Field>
          {!simSpatial && (
            <Field
              label={t('站点 ID（可选）', 'Station ID (optional)')}
              hint={simHint(
                'stationId',
                t(
                  '与参考数据里的站点 ID 比对；留空则从 history 前缀去掉 _hist_ 推导。改报告显示名不影响它。',
                  'Matched against the station ID in the reference data; leave empty to derive it from the history prefix. The display name does not affect it.',
                ),
              )}
            >
              <Input
                data-sim-field="stationId"
                value={simForm.stationId}
                placeholder={simForm.prefix.replace(/_hist_$/, '')}
                onChange={(e) => setSimForm((old) => ({ ...old, stationId: e.target.value }))}
              />
            </Field>
          )}
          <Field label={t('起始年', 'From year')} hint={simHint('from')}>
            <Input
              type="number"
              data-sim-field="from"
              value={simForm.from}
              onChange={(e) => setSimForm((old) => ({ ...old, from: e.target.value }))}
            />
          </Field>
          <Field label={t('结束年', 'To year')} hint={simHint('to')}>
            <Input
              type="number"
              data-sim-field="to"
              value={simForm.to}
              onChange={(e) => setSimForm((old) => ({ ...old, to: e.target.value }))}
            />
          </Field>
          <Field
            label={t('文件分组 data_groupby', 'File grouping (data_groupby)')}
            hint={simHint('dataGroupby')}
          >
            <Select
              data-sim-field="dataGroupby"
              value={simForm.dataGroupby}
              onChange={(e) => setSimForm((old) => ({ ...old, dataGroupby: e.target.value }))}
            >
              <option value="">{t('自动', 'Auto')}</option>
              {['Year', 'Month', 'Day', 'single'].map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </Select>
          </Field>
          <Field
            label={t('原生网格分辨率（度）', 'Native grid resolution (degrees)')}
            hint={simHint(
              'gridRes',
              t(
                '读取概况一般会给出；留空沿用读到的值。比较用的目标分辨率在下面的「比较的网格分辨率」里另填。',
                'The inspection usually provides it; leave it empty to use the inspected value. The target resolution for the comparison is set under “Comparison grid resolution” below.',
              ),
            )}
          >
            <Input
              type="number"
              data-sim-field="gridRes"
              value={simForm.gridRes}
              onChange={(e) => setSimForm((old) => ({ ...old, gridRes: e.target.value }))}
            />
          </Field>
          {simSpatial ? (
            <Field
              label={t('区域范围（西,东,南,北；度）', 'Region (west, east, south, north; degrees)')}
              hint={simHint(
                'extent',
                t(
                  '读取概况给不出范围时留空；生成配置会说明缺范围，不会拿默认全球范围冒充已确认的区域。',
                  'Leave it empty when the inspection gives no region; the generated configuration says the region is missing instead of posing the global default as confirmed.',
                ),
              )}
            >
              <Input
                data-sim-field="extent"
                value={simForm.extent}
                placeholder="104,117.2,21.5,26.9"
                onChange={(e) => setSimForm((old) => ({ ...old, extent: e.target.value }))}
              />
            </Field>
          ) : (
            <>
              <Field label={t('经度（度）', 'Longitude (degrees)')} hint={simHint('lon')}>
                <Input
                  type="number"
                  data-sim-field="lon"
                  value={simForm.lon}
                  onChange={(e) => setSimForm((old) => ({ ...old, lon: e.target.value }))}
                />
              </Field>
              <Field label={t('纬度（度）', 'Latitude (degrees)')} hint={simHint('lat')}>
                <Input
                  type="number"
                  data-sim-field="lat"
                  value={simForm.lat}
                  onChange={(e) => setSimForm((old) => ({ ...old, lat: e.target.value }))}
                />
              </Field>
            </>
          )}
        </div>
        <h3 className="study-subhead">
          {t('新模拟变量覆盖', 'New simulation variable overrides')}
        </h3>
        {simForm.variables.map((row, at) => (
          // 行的身份只用位置：key 里带 row.variable 时每敲一个字都会重建这一行，输入框失去焦点
          // （审查报告：标准变量输入框逐字符失焦，React+jsdom 已复现）。
          <div className="form-grid" data-sim-variable-row={at} key={at}>
            <Field label={t('标准变量', 'Standard variable')}>
              <Input
                data-sim-field="variables"
                value={row.variable}
                onChange={(e) =>
                  setSimForm((old) => ({
                    ...old,
                    variables: old.variables.map((item, i) =>
                      i === at ? { ...item, variable: e.target.value } : item,
                    ),
                  }))
                }
              />
            </Field>
            <Field label={t('变量名', 'NetCDF variable')}>
              <Input
                value={row.varname}
                onChange={(e) =>
                  setSimForm((old) => ({
                    ...old,
                    variables: old.variables.map((item, i) =>
                      i === at ? { ...item, varname: e.target.value } : item,
                    ),
                  }))
                }
              />
            </Field>
            <Field label={t('单位', 'Unit')}>
              <Input
                value={row.unit}
                onChange={(e) =>
                  setSimForm((old) => ({
                    ...old,
                    variables: old.variables.map((item, i) =>
                      i === at ? { ...item, unit: e.target.value } : item,
                    ),
                  }))
                }
              />
            </Field>
            <Field label="">
              <Button
                variant="ghost"
                onClick={() =>
                  setSimForm((old) => ({
                    ...old,
                    variables: old.variables.filter((_item, i) => i !== at),
                  }))
                }
              >
                {t('去掉', 'Remove')}
              </Button>
            </Field>
          </div>
        ))}
        {error && <Notice tone="error">{error}</Notice>}
        <div className="button-row">
          <Button
            variant="ghost"
            onClick={() =>
              setSimForm((old) => ({
                ...old,
                variables: [...old.variables, { variable: '', varname: '', unit: '' }],
              }))
            }
          >
            {t('加一个变量', 'Add variable')}
          </Button>
          <Button disabled={busy} onClick={saveSimulation}>
            {t('保存模拟', 'Save simulation')}
          </Button>
          <Button variant="ghost" disabled={busy} onClick={() => setDialog(null)}>
            {t('取消', 'Cancel')}
          </Button>
        </div>
      </Dialog>

      {remote && browsing && (
        <RemotePathBrowser
          host={host}
          directory={browsing !== 'refList'}
          en={en}
          initialPath={
            browsing === 'ref'
              ? scanDir || undefined
              : browsing === 'simRegister'
                ? simForm.dir || undefined
                : browsing === 'refDataDir'
                  ? reg.rootDir || undefined
                  : browsing === 'refList'
                    ? parentPath(reg.fulllist)
                    : undefined
          }
          onSelect={(path) => {
            const target = browsing;
            setBrowsing(null);
            if (target === 'ref') setScanDir(path);
            else if (target === 'simRegister') changeSimulationDir(path);
            else if (target === 'refDataDir') setReg((old) => ({ ...old, rootDir: path }));
            else if (target === 'refList') setReg((old) => ({ ...old, fulllist: path }));
            else void addPick(path, pickLabel(path));
          }}
          onClose={() => setBrowsing(null)}
        />
      )}
    </div>
  );
}
