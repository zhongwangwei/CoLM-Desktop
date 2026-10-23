import { Fragment, useCallback, useEffect, useRef, useState } from 'react';
import type uPlot from 'uplot';
import { ScienceChart } from '../results/ResultsPage';
import { invoke, invokeJson } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { withCatalogLabels } from '../../lib/param-presentation';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { usePageContract, type UiAction, type UiField } from '../../lib/guide';
import { ActionButton } from '../../components/ActionButton';
import { Button, Card, Field, Input, Select, Notice, Advanced } from '../../components/ui';
import {
  budgetAdvice,
  calibrationWindows,
  candidateBudget,
  fitValidation,
  gridTargetDefaults,
  gridTargetSpec,
  type GridTargetDraft,
  memberVerdict,
  networkSpec,
  networkDefaults,
  parameterSpec,
  studyPermissions,
  studyDiagnostics,
  studyRunOutcome,
  frozenDesign,
  sensitivityResponses,
  sensitivityRanking,
  calibrationSeed,
  networkWeightCount,
  studyProgress,
  convergenceRows,
  nearOptimalRows,
  crossValidationRows,
  suggestedRange,
  suggestedWindows,
  type ParameterContext,
  type SiteTiming,
  applyResearchSeed,
} from './study';
import {
  remoteStudyPermissions,
  remoteStudyReply,
  studyModelRevision,
  type RemoteStudyRecord,
  type RemoteStudyReply,
} from './remote-study';
import type { CaseProfile, ModelDraft, VersionRef } from '../../lib/types';
import StudyImportDialog, { type StudyPreview } from '../versions/StudyImportDialog';
import { REMOTE_CHECK_MS, compatibleKernel } from '../runs/run-model';
import LiveServerLoad from '../runs/LiveServerLoad';
import {
  researchMethods,
  methodFromStudy,
  completedCandidates,
  resultFileLabel,
  shownResultFiles,
} from './research-home';
import { hubMethods } from '../versions/hub-methods';
import { IGBP, PFT, USGS } from '../versions/scope-labels';
import './research-home.css';

/** 格点研究能选的地类或植物类型（后端 `study_type_choices`）。 */
type TypeChoices = {
  spatial: boolean;
  kind: 'igbp' | 'usgs' | 'pft' | 'pc';
  max: number;
  present: number[] | null;
};
type Envelope = {
  state?: {
    status?: string;
    best_member?: string;
    completed_candidates?: number;
    candidates?: Record<
      string,
      {
        generation?: number;
        feasible?: boolean;
        calibration?: number;
        validation?: number;
        reason?: string;
      }
    >;
    tasks?: Record<string, unknown>;
    warnings?: string[];
  };
  manifest?: { id?: string; spec?: unknown };
  results?: (string | { path: string })[];
  model_version?: { model_revision?: { version?: VersionRef } | null } | null;
  /** 第一次运行时记下的引擎与前处理（`run-options.json`）；之后的运行都用它。 */
  run_options?: { engine: string; preprocessors: string } | null;
  remote?: RemoteStudyRecord | null;
  remote_pending?: RemoteStudyRecord | null;
};
type HybridStudy = { dir: string; sites: string[]; parameters: string[]; status: string };

function StudyResult({ text, theme, en }: { text: string; theme: string; en: boolean }) {
  const tr = (zh: string, english: string) => (en ? english : zh);
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return <pre className="max-h-96 overflow-auto">{text}</pre>;
  }
  if (value && typeof value === 'object' && !Array.isArray(value)) {
    const data = value as Record<string, unknown>;
    if (
      Array.isArray(data.time) &&
      ['baseline', 'p05', 'p50', 'p95'].every(
        (key) =>
          Array.isArray(data[key]) &&
          (data[key] as unknown[]).length === (data.time as unknown[]).length,
      )
    ) {
      const length = (data.time as number[]).length;
      const stride = Math.max(1, Math.ceil(length / 2400));
      const sample = (values: unknown) =>
        (values as (number | null)[]).filter((_, i) => i % stride === 0);
      // 基准一个色相；P05/P50/P95 是集合的分位带，同一个色相，上下沿用虚线（颜色之外的第二种区分）。
      // 原来四条线三色循环，P95 和基准同色（GUI 审查 C-05）。两色已用 dataviz 校验器在亮、暗两种底色下校验。
      const ensemble = '#6366f1';
      const unstable = Array.isArray(data.stable)
        ? (data.stable as unknown[]).filter((v) => v === false).length
        : 0;
      const units = typeof data.units === 'string' && data.units ? ` (${data.units})` : '';
      return (
        <>
          <p className="study-subhead">
            {String(data.site ?? '')} · {String(data.variable ?? '')}
            {units}
            {typeof data.members === 'number'
              ? tr(` · ${data.members} 个样本成员`, ` · ${data.members} sample members`)
              : ''}
          </p>
          <ScienceChart
            data={
              [
                sample(data.time),
                ...['baseline', 'p05', 'p50', 'p95'].map((key) => sample(data[key])),
              ] as uPlot.AlignedData
            }
            labels={[tr('基准', 'Baseline'), 'P05', tr('中位数', 'Median'), 'P95']}
            styles={[
              { stroke: '#0d9488', width: 2 },
              { stroke: ensemble, dash: [5, 4] },
              { stroke: ensemble, width: 2 },
              { stroke: ensemble, dash: [5, 4] },
            ]}
            theme={theme}
          />
          {unstable > 0 && (
            <p className="muted study-hint">
              {tr(
                `有 ${unstable} 个时刻成功的成员太少，那里不画分位带（线是断开的），只有基准。`,
                `At ${unstable} timestamps too few members succeeded, so the band is left out there (the lines break) and only the baseline is drawn.`,
              )}
            </p>
          )}
          <p>
            {tr(
              `${length} 个时刻，图上每 ${stride} 个取一个。分位数描述的是这次抽样设计下的分布，不是预测的置信区间。`,
              `${length} source points; chart stride ${stride}. Quantiles describe this sampling design, not a predictive confidence interval.`,
            )}
          </p>
        </>
      );
    }
  }
  const rows = Array.isArray(value)
    ? value
    : value && typeof value === 'object'
      ? Object.entries(value).map(([key, v]) => ({
          key,
          ...(v && typeof v === 'object' ? (v as object) : { value: v }),
        }))
      : [];
  if (rows.length && rows.every((row) => row && typeof row === 'object' && !Array.isArray(row))) {
    const columns = [...new Set(rows.slice(0, 1000).flatMap((row) => Object.keys(row)))];
    return (
      <div className="overflow-auto">
        <table>
          <thead>
            <tr>
              {columns.map((key) => (
                <th key={key}>{key}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.slice(0, 1000).map((row, index) => (
              <tr key={index}>
                {columns.map((key) => (
                  <td key={key}>
                    {row[key] == null
                      ? '—'
                      : typeof row[key] === 'object'
                        ? JSON.stringify(row[key])
                        : String(row[key])}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
        {rows.length > 1000 && (
          <p>
            {tr(
              '只显示前 1000 行；导出的研究包含全部记录。',
              'Showing first 1000 rows; export includes all records.',
            )}
          </p>
        )}
      </div>
    );
  }
  return <pre className="max-h-96 overflow-auto">{text}</pre>;
}

/**
 * `active`：研究页打开过一次就常驻（App 里隐藏而不卸载）。本机跑研究要几十分钟到几小时，原来靠导航守卫
 * 把整个应用锁在研究页（卸载就丢了正在跑的研究和它的进度）；现在可以离开去看结果、历史，回来还是那个研究。
 */
/** `evaluation_plan` 的一行：观测标准名（研究目标用它）与是否各站都能比。 */
type TargetChoice = {
  name: string;
  label_zh: string;
  label_en: string;
  units: string;
  available: boolean;
};
type ParameterDraft = { enabled: boolean; min: string; max: string; scale: string };
/** 刚勾上、上下限都空着的参数预填一个合法范围；已经填过的不动。 */
function withSuggestedRange(row: ParameterContext, p: ParameterDraft): ParameterDraft {
  if (!p.enabled || p.min.trim() || p.max.trim()) return p;
  const [min, max] = suggestedRange(row, p.scale);
  return { ...p, min, max };
}
export default function ResearchPage({ active = true }: { active?: boolean }) {
  const { state, notify, refreshVersions, navigate, update } = useApp();
  const en = state.language === 'en';
  const tr = (zh: string, english: string) => (en ? english : zh);
  const dir = state.selectedCase?.dir || '';
  const versionId = state.versionRef.id;
  const revision = state.versionRef.revision;
  const [workbench, setWorkbench] = useState(false);
  const [tab, setTab] = useState('tuning');
  const [engine, setEngine] = useState('rust');
  const [preprocessors, setPreprocessors] = useState('rust');
  const [host, setHost] = useState('');
  const [servers, setServers] = useState<string[]>([]);
  // 各服务器设置里的分区：服务器状况面板高亮它。和 servers 同时写入，借 setServers 的重绘显示。
  const serverPartitions = useRef<Record<string, string>>({});
  const [remoteReply, setRemoteReply] = useState<RemoteStudyReply | null>(null);
  const [sites, setSites] = useState<string[]>([]);
  // 格点算例（流域、区域、全球）也能做研究（2026-10-21 用户要求）。一次研究里只有一种算例，编译配置跟着换：
  // 单点用带 SinglePoint 的，格点用格点配置。
  const spatialStudy = sites.some((d) => state.cases.find((c) => c.dir === d)?.spatial);
  const studyKernels = state.kernels.filter(
    (k) => k.macros.includes('SinglePoint') !== spatialStudy,
  );
  const [kernel, setKernel] = useState(
    () =>
      studyKernels.find((k) => k.dir === state.kernelDir)?.dir ??
      studyKernels.find((k) => k.preset === 'default')?.dir ??
      studyKernels[0]?.dir ??
      '',
  );
  const [contexts, setContexts] = useState<ParameterContext[]>([]);
  const contextsRef = useRef<ParameterContext[]>([]);
  contextsRef.current = contexts;
  const [parameters, setParameters] = useState<
    Record<string, { enabled: boolean; min: string; max: string; scale: string }>
  >({});
  const [observations, setObservations] = useState('');
  // 格点算例要研究哪些地类或植物类型（单点按本站地类自动定）；present 是 landdata 里各算例都有的。
  const [typeChoices, setTypeChoices] = useState<TypeChoices | null>(null);
  const [types, setTypes] = useState<number[]>([]);
  // 格点算例的率定目标：格点观测产品、区域里的通量站、水文站流量。
  const [gridTargets, setGridTargets] = useState<GridTargetDraft[]>([
    { ...gridTargetDefaults.grid },
  ]);
  const [variables, setVariables] = useState('Qle');
  // 单点率定能选哪些目标：各站都能比的变量（按 case.nml 的输出与观测文件判断，算例不必先跑过）。
  const [targetChoices, setTargetChoices] = useState<TargetChoice[] | null>(null);
  // 不确定性分析的输出是模型 history 变量名（f_lfevpa 一类），和率定目标的观测名（Qle）不是一套：
  // 原来两者共用一个框、默认 Qle，切到不确定性分析直接预检就报"算例没开这个输出"。
  const [outputs, setOutputs] = useState('f_lfevpa');
  const [method, setMethod] = useState('lhs');
  const [count, setCount] = useState('40');
  // 样本数和 Morris 轨迹数共用一个框，含义不同：换方法时没手改过就换成该方法的缺省（LHS 40 组、Morris 10 条）。
  const countEdited = useRef(false);
  const chooseSampling = (value: string) => {
    setMethod(value);
    if (!countEdited.current) setCount(value === 'morris' ? '10' : '40');
  };
  const editCount = (value: string) => {
    countEdited.current = true;
    setCount(value);
  };
  const [population, setPopulation] = useState('8');
  const [generations, setGenerations] = useState('4');
  // 单点模拟很轻，默认同时跑 4 个；原来默认 1 个，几十次模拟一个接一个跑。
  const [jobs, setJobs] = useState('4');
  // 并发数和种群按机器与参数个数给默认值；人改过就不再跟着变。
  const jobsEdited = useRef(false);
  const populationEdited = useRef(false);
  const editJobs = (value: string) => {
    jobsEdited.current = true;
    setJobs(value);
  };
  const editPopulation = (value: string) => {
    populationEdited.current = true;
    setPopulation(value);
  };
  const [seed, setSeed] = useState('1');
  // 差分进化连续多少代没有改善就提前停；0 表示跑满（后端缺省 3）。原来没有入口，提前停也关不掉。
  const [patience, setPatience] = useState('3');
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [valFrom, setValFrom] = useState('');
  const [valTo, setValTo] = useState('');
  const [metric, setMetric] = useState('nrmse');
  const [minPairs, setMinPairs] = useState('20');
  const [confirmed, setConfirmed] = useState(false);
  const [studyDir, setStudyDir] = useState('');
  const [envelope, setEnvelope] = useState<Envelope | null>(null);
  const [preflight, setPreflight] = useState('');
  const [resultPath, setResultPath] = useState('');
  const [result, setResult] = useState('');
  // 不确定性研究的敏感性（results/importance.json）、正在看的响应量、要带去率定的参数。
  const [importance, setImportance] = useState<unknown>(null);
  const [sensitivityKey, setSensitivityKey] = useState('');
  const [carryKeys, setCarryKeys] = useState<string[]>([]);
  // 率定的近优集合（results/near_optimal.json）：参数能被观测定到多准。
  const [nearOptimal, setNearOptimal] = useState<unknown>(null);
  // 多站点共享参数的留一站交叉验证（results/cross_validation.json）。
  const [crossValidation, setCrossValidation] = useState<unknown>(null);
  const [out, setOut] = useState('');
  // 导出研究的目录；原来和 AI 拟合的输出目录共用 out，填了一个另一个也跟着变。
  const [exportDir, setExportDir] = useState('');
  const [name, setName] = useState('research-model');
  const [member, setMember] = useState('');
  // 选中的成员不比默认参数好时，写回默认是灰的；对这个研究的这个成员明确勾了"仍要写入"才放开。
  // 记成"研究目录#成员"，换研究或换成员就自动失效。
  const [overrideFor, setOverrideFor] = useState('');
  // Qle/Qh 是否拿能量闭合订正后的观测比（只对站点观测有效）。
  const [corrected, setCorrected] = useState(false);
  // 站点目标按什么时间尺度比：空 = 观测原来的分辨率（半小时）；daily/monthly 先平均再比。
  const [aggregate, setAggregate] = useState<'' | 'daily' | 'monthly'>('');
  // 对照几个变量时各自的权重（变量名 → 文本）；没填的按 1。
  const [weights, setWeights] = useState<Record<string, string>>({});
  // 率定期、验证期是怎么预填的，告诉用户扣了什么。
  const [windowNote, setWindowNote] = useState<[string, string] | null>(null);
  const [busy, setBusy] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState('');
  // 后端和校验器的报错原样显示，只去掉 String(error) 带出来的 "Error: "。
  const shownError = error.replace(/^Error: /, '');
  // 版本页带过来的参数勾上了几个（有的在这些算例里没有，比如算例里没有那个地类或植物类型）。
  const [seedNotice, setSeedNotice] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const [slot, setSlot] = useState('pft');
  const [features, setFeatures] = useState('clim_tair, clim_vpd, clim_prec');
  const [output, setOutput] = useState('DEF_PFT_VMAX25');
  const [lower, setLower] = useState('0.5');
  const [upper, setUpper] = useState('2');
  const [hidden, setHidden] = useState('linear');
  const [relative, setRelative] = useState(true);
  const [hybridInfo, setHybridInfo] = useState<Record<string, unknown> | null>(null);
  const [hybridStudies, setHybridStudies] = useState<HybridStudy[]>([]);
  const [chosenStudies, setChosenStudies] = useState<string[]>([]);
  const [fit, setFit] = useState<Record<string, unknown> | null>(null);
  const [ridge, setRidge] = useState('0.001');
  const [epochs, setEpochs] = useState('200');
  const [trainNetwork, setTrainNetwork] = useState(false);
  // 研究工作台分四步走（2026-10-21 用户反映原来一页铺开所有字段"乱七八糟"）：
  // 1 选算例与运行位置，2 选参数（AI：网络设计），3 目标与预算，4 运行与结果。
  const [step, setStep] = useState<1 | 2 | 3 | 4>(1);
  const loadedFor = useRef('');
  // 从"新建修改版"进来的（返回时回那里）；从草稿进来的看 state.researchTarget；都不是就是侧栏的研究首页。
  const [fromHub, setFromHub] = useState(false);
  // 研究结果正要写进的草稿与研究成员的参数（勾选对话框）；created 表示草稿是为这次写入新建的。
  const [writing, setWriting] = useState<
    (StudyPreview & { draft: ModelDraft; created: boolean }) | null
  >(null);
  const epoch = useRef(0),
    lock = useRef(false),
    runLock = useRef(false),
    remoteRequest = useRef(0);
  const shown = useRef(active);
  shown.current = active;
  const ai = tab === 'parameters' || tab === 'process';
  const tuning = tab !== 'uncertainty';
  // 不确定性分析也对照观测算目标函数（默认勾上）：敏感性就能按"调这个参数能不能让拟合变好"排，
  // 而不只是"它会不会让输出变"，排出来的参数可以直接带去率定。没有观测时取消勾选。
  const [scoreObjective, setScoreObjective] = useState(true);
  const scoring = tuning || scoreObjective;
  // 只拦短操作（读参数、预检、建研究、提交服务器）；本机研究跑起来以后可以离开，页面常驻不丢。
  useNavigationGuard(() =>
    active && (lock.current || (runLock.current && !!host))
      ? tr(
          '研究操作正在执行，请等待当前操作完成。',
          'Wait for the current study operation to finish.',
        )
      : null,
  );
  const remotePermissions = remoteStudyPermissions(
    envelope?.remote,
    envelope?.remote_pending,
    remoteReply,
  );
  useEffect(() => {
    let current = true;
    invokeJson<{ servers: { host: string; partition?: string }[] }>('remote_config')
      .then((value) => {
        if (!current) return;
        serverPartitions.current = Object.fromEntries(
          value.servers.map((server) => [server.host, server.partition ?? '']),
        );
        setServers(value.servers.map((server) => server.host));
      })
      .catch((e) => {
        if (current) notify(String(e), true);
      });
    return () => {
      current = false;
    };
  }, [notify]);
  useEffect(() => {
    const requestEpoch = epoch;
    requestEpoch.current++;
    setSites(dir ? [dir] : []);
    // 本机研究正在跑时离开研究页去开了别的算例：设计部分照常重置，正在跑的研究留着，回来还看得到进度。
    if (!runLock.current) {
      setStudyDir('');
      setEnvelope(null);
      setRemoteReply(null);
    }
    setContexts([]);
    setParameters({});
    setPreflight('');
    setResult('');
    setError('');
    setFit(null);
    setHybridInfo(null);
    setHybridStudies([]);
    setChosenStudies([]);
    setObservations('');
    setConfirmed(false);
    return () => {
      requestEpoch.current++;
    };
  }, [dir, versionId, revision]);
  useEffect(() => {
    epoch.current++;
    setContexts([]);
    setParameters({});
    setTypeChoices(null);
    setTypes([]);
    setPreflight('');
    if (!runLock.current) {
      setEnvelope(null);
      setRemoteReply(null);
      setStudyDir('');
      // 换了算例或编译配置就是另一个研究，上一个研究勾的"已核对预算"不能带过来。
      setConfirmed(false);
    }
    setFit(null);
  }, [sites, kernel]);
  useEffect(() => {
    epoch.current++;
    setPreflight('');
    // 上一次预检或创建的报错说的是改之前的设计；换了方法或改了设定还留着，像是新设计也错了。
    setError('');
  }, [
    tab,
    observations,
    variables,
    outputs,
    method,
    count,
    population,
    generations,
    jobs,
    seed,
    from,
    to,
    valFrom,
    valTo,
    metric,
    minPairs,
    confirmed,
    parameters,
    slot,
    features,
    output,
    lower,
    upper,
    hidden,
    relative,
    trainNetwork,
    corrected,
    gridTargets,
    scoreObjective,
  ]);
  useEffect(() => {
    epoch.current++;
    setResult('');
  }, [resultPath]);
  useEffect(() => {
    epoch.current++;
    setFit(null);
  }, [
    slot,
    features,
    output,
    lower,
    upper,
    hidden,
    relative,
    chosenStudies,
    ridge,
    epochs,
    out,
    name,
  ]);
  async function task<T>(fn: (current: () => boolean) => Promise<T>): Promise<T> {
    if (lock.current)
      throw new Error(tr('上一个研究操作还没结束', 'Another study operation is in progress'));
    lock.current = true;
    setBusy(true);
    setError('');
    const token = epoch.current;
    try {
      return await fn(() => token === epoch.current);
    } catch (e) {
      if (token === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  const execute = (fn: () => unknown | Promise<unknown>) => {
    void Promise.resolve()
      .then(fn)
      .catch(() => {});
  };
  const readStatus = useCallback(
    async (path = studyDir, adopt = false) => {
      const token = epoch.current;
      const data = await invokeJson<Envelope>('study_status', { studyDir: path });
      const remote = remoteStudyReply({ record: data.remote }).record;
      const pending = remoteStudyReply({ record: data.remote_pending }).record;
      const safe = { ...data, remote, remote_pending: pending };
      if (token === epoch.current) {
        if (adopt) setStudyDir(path);
        setEnvelope(safe);
        if (remote?.host || pending?.host) setHost(remote?.host || pending?.host || '');
        // 轮询刷新时保留用户在下拉框里选的成员，只有它不在成员里时才回到最优。
        setMember((previous) =>
          previous && data.state?.candidates?.[previous] ? previous : data.state?.best_member || '',
        );
        const files = shownResultFiles(data.results);
        setResultPath((previous) => (files.includes(previous) ? previous : files[0] || ''));
      }
      return safe;
    },
    [studyDir],
  );
  const readRemoteStatus = useCallback(async () => {
    const token = epoch.current;
    const sequence = ++remoteRequest.current;
    const data = remoteStudyReply(await invokeJson('remote_study_status', { studyDir }));
    if (token === epoch.current && sequence === remoteRequest.current) setRemoteReply(data);
    return data;
  }, [studyDir]);
  useEffect(() => {
    if (
      !active ||
      !studyDir ||
      busy ||
      remotePermissions.exists ||
      (!launching && envelope?.state?.status?.toLowerCase() !== 'running')
    )
      return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      if (stopped || lock.current) return;
      try {
        await readStatus();
      } catch (e) {
        if (!stopped) setError(String(e));
      }
      if (!stopped) timer = setTimeout(poll, 3000);
    };
    timer = setTimeout(poll, 3000);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [
    active,
    studyDir,
    busy,
    launching,
    remotePermissions.exists,
    envelope?.state?.status,
    readStatus,
  ]);
  useEffect(() => {
    if (!active || !studyDir || busy || launching || !remotePermissions.poll) return;
    let stopped = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        await readRemoteStatus();
      } catch (e) {
        if (!stopped) setError(String(e));
      }
      if (!stopped) timer = setTimeout(poll, REMOTE_CHECK_MS);
    };
    timer = setTimeout(poll, 1000);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [active, studyDir, busy, launching, remotePermissions.poll, readRemoteStatus]);
  async function effectiveCase(path: string) {
    const context = await invokeJson<{ entry: { dir: string } }>('model_case_context', {
      dir: path,
      modelRevision: state.versionRef,
    });
    return context.entry.dir;
  }
  const loadContexts = () =>
    task(async (current) => {
      if (!sites.length || !kernel)
        throw new Error(tr('请先选算例和编译配置', 'Select cases and a build configuration'));
      // 被别的设计改动打断（epoch 变了）就清掉"已读过"的标记：这次任务结束时 busy 翻回 false，
      // 自动读取的 effect 会按当前算例重读。原来标记留着，同一次更新里既换算例又进第 2 步时
      //（敏感性"带入率定"）读取被丢弃、再也不读，参数表一直是空的（2026-10-23 真实窗口实测）。
      const superseded = () => {
        loadedFor.current = '';
      };
      const effective = [];
      for (const path of sites) {
        effective.push(await effectiveCase(path));
        if (!current()) return superseded();
      }
      const rows = await invokeJson<ParameterContext[]>('study_parameter_contexts', {
        dirs: effective,
        kernelDir: kernel,
        types: spatialStudy ? types : null,
      });
      if (current()) {
        // 勾选按行号记；格点研究改勾地类后重新读出的行序会变（新类型的行插在中间），原来已勾的参数
        // 会悄悄落到别的参数上。按参数与作用域把已勾的搬到新行号上，读不到的丢掉。
        const keyOf = (row: ParameterContext) => `${row.id}|${JSON.stringify(row.scope_instance)}`;
        const previous = contextsRef.current;
        setParameters((old) => {
          const byKey = new Map(previous.map((row, i) => [keyOf(row), old[String(i)]]));
          return Object.fromEntries(
            rows.flatMap((row, i) => {
              const kept = byKey.get(keyOf(row));
              return kept ? [[String(i), kept]] : [];
            }),
          );
        });
        setContexts(withCatalogLabels(rows));
        const seed = state.researchSeed?.parameters;
        if (seed?.length) {
          const picked = applyResearchSeed(rows, seed);
          setParameters((old) => ({ ...old, ...picked }));
          const matched = Object.keys(picked).length;
          setSeedNotice(
            tr(
              `已勾上带来的 ${matched} 个参数${matched < seed.length ? `；另 ${seed.length - matched} 个在这些算例里没有（比如算例里没有那个地类或植物类型）` : ''}。`,
              `Ticked ${matched} of the parameters brought along${matched < seed.length ? `; ${seed.length - matched} do not apply to these cases (for example, the class or plant type is absent)` : ''}.`,
            ),
          );
          update({ researchSeed: null });
        }
      } else superseded();
      return rows;
    });
  // 站点目标的权重与时间尺度（单点率定、带目标的不确定性分析、区域里的通量站共用）。
  function siteTargetExtras(variable: string) {
    const text = weights[variable]?.trim() ?? '';
    const weight = text ? Number(text) : 1;
    if (!Number.isFinite(weight) || weight <= 0)
      throw new Error(
        tr(`${variable} 的权重要是正数`, `The weight of ${variable} must be a positive number`),
      );
    return { weight, ...(aggregate ? { aggregate } : {}) };
  }
  // 率定、不确定性和两种 AI 方法都在算例上跑；手动改参数和修改源码不用算例。
  function needsCases(method: string) {
    return method !== 'manual' && method !== 'source';
  }
  // 只有湍流通量有能量闭合订正版（`colm_hist::obs::corrected`）。
  function fluxCorrected(variable: string) {
    return ['qle', 'qh'].includes(variable.trim().toLowerCase());
  }
  function network() {
    return networkSpec(slot, features, output, lower, upper, hidden, relative);
  }
  function chooseSlot(value: string) {
    const defaults = networkDefaults(value);
    setSlot(defaults.slot);
    setFeatures(defaults.features);
    setOutput(defaults.output);
    setLower(defaults.lower);
    setUpper(defaults.upper);
    setRelative(defaults.relative);
    // AI 方法默认就是训练网络。原来默认不勾、勾选框又在第 3 步最下面：替换过程照默认走必然报错，
    // 学习参数静默退化成普通率定，网络设计全被忽略（GUI 审查 C-03）。
    setTrainNetwork(true);
  }
  // 预检只是把规格交给引擎检查，不落盘也不跑，所以不要求先勾"已核查"；
  // 原来两个按钮都卡在这个勾上，没勾时预检是灰的，看不出为什么。只有创建研究要它。
  function spec(forCreate: boolean) {
    if (!sites.length || !kernel)
      throw new Error(tr('请先选算例和编译配置', 'Select cases and a build configuration'));
    if (forCreate && !confirmed)
      throw new Error(
        tr(
          '请先勾选"我已核对参数范围、预算和适用范围"',
          'Confirm that you reviewed parameter ranges, budget and applicability',
        ),
      );
    const roots = new Set(sites.map((s) => s.replace(/[/\\][^/\\]+$/, '')));
    if (roots.size !== 1)
      throw new Error(
        tr('所选算例要在同一个项目目录里', 'All cases must share a project directory'),
      );
    const params = contexts.flatMap((row, i) => {
      const p = parameters[String(i)];
      return p?.enabled ? [parameterSpec(row, p.min, p.max, p.scale)] : [];
    });
    if (!params.length && !(ai && (trainNetwork || tab === 'process')))
      throw new Error(
        tr(
          '请至少勾选一个参数，或选择训练网络',
          'Select at least one parameter or network training',
        ),
      );
    const budget = candidateBudget(
      tuning ? 'tuning' : 'uncertainty',
      method,
      params.length,
      Number(count),
      Number(population),
      Number(generations),
      sites.length,
    );
    const concurrency = Number(jobs),
      random = Number(seed);
    if (
      !Number.isInteger(concurrency) ||
      concurrency < 1 ||
      concurrency > 16 ||
      !Number.isSafeInteger(random) ||
      random < 0
    )
      throw new Error(
        tr(
          '同时运行的模拟数要在 1–16 之间，随机种子要是非负整数',
          'Jobs must be 1–16; seed must be a nonnegative integer',
        ),
      );
    const names = (tuning ? variables : outputs)
      .split(',')
      .map((v) => v.trim())
      .filter(Boolean);
    if (!names.length && !(tuning && spatialStudy))
      throw new Error(tr('请填写目标或输出变量', 'Choose target/output variables'));
    const base = {
      kind: tuning ? 'tuning' : 'uncertainty',
      method: tuning ? 'differential-evolution' : method,
      seed: random,
      kernel_dir: kernel,
      base_cases: sites,
      parameters: params,
      site_mode: 'shared',
    };
    if (!tuning) {
      // 分析时段：输出统计、分位带和目标函数都只看这一段。原来不写，统计里混进初值还没稳定的
      // 第一年（GUI 审查 C-04）。两个框都空着才表示整段模拟期。
      const analysis = from.trim() || to.trim() ? calibrationWindows(from, to, '', '') : null;
      // Morris 的 candidate_count 是轨迹数，后端按 轨迹数 × (k+1) 生成样本。
      const uncertainty = {
        ...base,
        outputs: names,
        ...(analysis ? { analysis_from: analysis.from, analysis_to: analysis.to } : {}),
        budget: {
          candidate_count:
            method === 'oat' ? undefined : method === 'morris' ? Number(count) : budget.candidates,
          jobs: concurrency,
        },
      };
      if (!scoreObjective)
        return { caseRoot: [...roots][0]!, specJson: JSON.stringify(uncertainty) };
      const window = calibrationWindows(from, to, '', '');
      const pairs = Number(minPairs);
      if (!Number.isInteger(pairs) || pairs < 2)
        throw new Error(tr('最少观测配对数至少是 2', 'Minimum pairs must be ≥2'));
      if (spatialStudy) {
        if (!gridTargets.length)
          throw new Error(
            tr(
              '请至少加一个对照目标，或取消"对照观测算目标函数"',
              'Add at least one target, or untick scoring against observations',
            ),
          );
        return {
          caseRoot: [...roots][0]!,
          specJson: JSON.stringify({
            ...uncertainty,
            targets: gridTargets.map((t, i) => ({
              ...gridTargetSpec(t, i, window, metric, pairs),
              ...(corrected && t.kind === 'station' && fluxCorrected(t.variable)
                ? { corrected: true }
                : {}),
            })),
          }),
        };
      }
      const targetNames = variableNames;
      if (!targetNames.length)
        throw new Error(
          tr(
            '请选要对照的观测变量，或取消"对照观测算目标函数"',
            'Choose observed variables to score, or untick scoring against observations',
          ),
        );
      return {
        caseRoot: [...roots][0]!,
        specJson: JSON.stringify({
          ...uncertainty,
          observations: Object.fromEntries(
            sites.map((path) => {
              const site = siteName(path);
              if (!observationOf[site])
                throw new Error(
                  tr(
                    `站点 ${site} 还没选观测文件；没有观测就取消"对照观测算目标函数"`,
                    `Observation missing for ${site}; without observations untick scoring`,
                  ),
                );
              return [site, observationOf[site]];
            }),
          ),
          targets: sites.flatMap((path) =>
            targetNames.map((variable) => ({
              key: `${variable}@${siteName(path)}`,
              site: siteName(path),
              variable,
              metric,
              min_pairs: pairs,
              ...window,
              ...siteTargetExtras(variable),
              ...(corrected && fluxCorrected(variable) ? { corrected: true } : {}),
            })),
          ),
        }),
      };
    }
    const obs = Object.fromEntries(
      observations
        .split('\n')
        .filter(Boolean)
        .map((line) => {
          const [site, ...path] = line.split('|');
          return [site.trim(), path.join('|').trim()];
        }),
    );
    const windows = calibrationWindows(from, to, valFrom, valTo);
    const stopAfter = Number(patience);
    if (!Number.isInteger(stopAfter) || stopAfter < 0)
      throw new Error(
        tr(
          '"连续几代没有改善就停"要填 0 或正整数（0 表示跑满全部代数）',
          'Stop-after-generations must be 0 or a positive integer (0 runs every generation)',
        ),
      );
    const pairs = Number(minPairs);
    if (!Number.isInteger(pairs) || pairs < 2)
      throw new Error(tr('最少观测配对数至少是 2', 'Minimum pairs must be ≥2'));
    // 格点算例：每个目标自带观测文件（格点产品、通量站、水文站），不按站点给观测。
    if (spatialStudy) {
      if (!gridTargets.length)
        throw new Error(tr('请至少加一个率定目标', 'Add at least one calibration target'));
      return {
        caseRoot: [...roots][0]!,
        specJson: JSON.stringify({
          ...base,
          targets: gridTargets.map((t, i) => ({
            ...gridTargetSpec(t, i, windows, metric, pairs),
            ...(t.kind === 'station' && aggregate ? { aggregate } : {}),
            ...(corrected && t.kind === 'station' && fluxCorrected(t.variable)
              ? { corrected: true }
              : {}),
          })),
          budget: {
            population: Number(population),
            generations: Number(generations),
            jobs: concurrency,
            include_baseline: true,
            patience: stopAfter,
          },
          ...(ai && (trainNetwork || tab === 'process') ? { hybrid: network() } : {}),
        }),
      };
    }
    const targets = sites.flatMap((path) => {
      const site = path.split(/[/\\]/).pop()!;
      if (!obs[site])
        throw new Error(tr(`站点 ${site} 还没选观测文件`, `Observation missing: ${site}`));
      return names.map((variable) => ({
        key: `${variable}@${site}`,
        site,
        variable,
        metric,
        min_pairs: pairs,
        ...windows,
        ...siteTargetExtras(variable),
        ...(corrected && fluxCorrected(variable) ? { corrected: true } : {}),
      }));
    });
    return {
      caseRoot: [...roots][0]!,
      specJson: JSON.stringify({
        ...base,
        observations: obs,
        targets,
        budget: {
          population: Number(population),
          generations: Number(generations),
          jobs: concurrency,
          include_baseline: true,
          patience: stopAfter,
        },
        ...(ai && (trainNetwork || tab === 'process') ? { hybrid: network() } : {}),
      }),
    };
  }
  const check = () =>
    task(async (current) => {
      const plan = spec(false);
      const value = await invoke('study_preflight_json', {
        ...plan,
        modelRevision: state.versionRef,
      });
      if (current())
        setPreflight(typeof value === 'string' ? value : JSON.stringify(value, null, 2));
      return value;
    });
  const create = () =>
    task(async (current) => {
      const plan = spec(true);
      await invoke('study_preflight_json', { ...plan, modelRevision: state.versionRef });
      if (!current()) throw new Error('Context changed');
      const path = await invoke<string>('study_create_json', {
        ...plan,
        modelRevision: state.versionRef,
      });
      if (current()) {
        setRemoteReply(null);
        setStudyDir(path.trim());
        await readStatus(path.trim());
        notify(tr('研究任务已创建', 'Study created'));
      } else notify(`Created study: ${path}`);
      return path.trim();
    });
  // 已建好的研究按它冻结的编译配置运行。打开已有研究时页面上选的可能是别的配置（格点研究冻结的是格点
  // 配置，没勾算例时这里默认单点的 default），原来照传，后端报"Study was created for kernel …"。
  const frozenKernel = (envelope?.manifest?.spec as { kernel_dir?: unknown } | undefined)
    ?.kernel_dir;
  const runKernel =
    typeof frozenKernel === 'string' && /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(frozenKernel)
      ? frozenKernel
      : kernel;
  const start = async () => {
    if (runLock.current || lock.current)
      throw new Error(
        tr('研究操作正在进行，请等它结束', 'A study operation is already in progress'),
      );
    const kernel = runKernel;
    if (!studyDir || !kernel || !envelope || !confirmed)
      throw new Error(
        tr(
          '请先打开研究、选好编译配置，并勾选"我已核对冻结的范围和预算"',
          'Open a study, choose a build configuration and confirm the frozen ranges and budget',
        ),
      );
    if (remotePermissions.exists)
      throw new Error(
        tr(
          '这个研究已经提交到服务器，请核对或取回服务器上的结果，不要再开一次运行',
          'This study has a server submission; inspect or fetch it instead of starting another run',
        ),
      );
    const concurrency = Number(jobs);
    if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 16)
      throw new Error(tr('同时运行的模拟数要在 1–16 之间', 'Jobs must be 1–16'));
    const modelRevision = studyModelRevision(state.versionRef, envelope.model_version);
    runLock.current = true;
    remoteRequest.current++;
    setLaunching(true);
    setError('');
    const token = epoch.current;
    if (host) setRemoteReply({ host, status: { state: 'submitting' }, warnings: [] });
    try {
      if (host) {
        const submitted = remoteStudyReply(
          await invokeJson('remote_study_run', {
            studyDir,
            host,
            kernel,
            jobs: concurrency,
            engine: ai ? 'rust' : engine,
            preprocessors,
            modelVersion: null,
            modelRevision,
          }),
        );
        if (token === epoch.current) {
          setRemoteReply(submitted);
          await readStatus();
          notify(
            tr(
              '研究已提交服务器，正在核对真实进度',
              'Study submitted to the server; checking actual progress',
            ),
          );
        }
        return submitted;
      }
      const response = await invoke('study_run', {
        studyDir,
        kernel,
        stream: true,
        jobs: concurrency,
        retryFailed: false,
        // 研究跑过就用它记下的引擎，不用页面当前的选择（后端会拒绝中途换引擎，GUI 审查 R8）。
        engine: envelope.run_options?.engine ?? (ai ? 'rust' : engine),
        modelVersion: null,
        modelRevision,
      });
      // 跑完时人在别的页面：提示一句，回研究页看结果。
      if (!shown.current)
        notify(
          tr(
            `研究 ${studyDir.split(/[/\\]/).pop()} 已运行结束，到"研究"页查看结果。`,
            `Study ${studyDir.split(/[/\\]/).pop()} finished running; open Research to see the results.`,
          ),
        );
      if (token === epoch.current) return await readStatus();
      return response;
    } catch (e) {
      if (token === epoch.current) {
        // 后端把取消、失败都当错误返回一段 JSON（`study_cancelled`/`study_failed`），原来整段当红字显示
        //（GUI 审查 B-04）。取消是正常操作，只提示一句；失败只显示原因。
        const outcome = studyRunOutcome(e);
        if (outcome.cancelled) notify(tr(outcome.zh, outcome.en));
        else setError(tr(outcome.zh, outcome.en));
        if (host) {
          // 提交失败时，研究自己的记录里既没有服务器作业、也没有"已发出提交"的意图，说明什么都没交上去：
          // 收回本地的"提交中"占位，可以改了再交。原来一律记成"状态未确认"，"开始运行"从此一直灰着，
          // 核对服务器也清不掉（GUI 审查 R2）。有记录时照旧按"未确认"处理，不能重复提交。
          const recorded = await readStatus()
            .then((fresh) => !!(fresh.remote || fresh.remote_pending))
            .catch(() => true);
          if (!recorded && token === epoch.current) setRemoteReply(null);
          else if (token === epoch.current)
            setRemoteReply((previous) => ({
              ...previous,
              host,
              status: { state: 'unknown' },
              warnings: [String(e)],
            }));
        }
      }
      throw e;
    } finally {
      runLock.current = false;
      setLaunching(false);
    }
  };
  const control = async (
    command: 'study_pause' | 'study_resume' | 'study_cancel' | 'study_retry',
  ) => {
    if (remotePermissions.exists)
      throw new Error(
        tr(
          '这个研究在服务器上，请用服务器的操作按钮',
          'Use the recorded server controls for this study',
        ),
      );
    // "待复核"的任务是进程身份核对不上的（多是软件中途被关）：原来重试固定不带它们，按了等于没按
    //（GUI 审查 R3）。问过用户确认它们确实不在运行，再连同待复核的一起重跑；后端还会核对残留的运行锁。
    const includeReview =
      command === 'study_retry' &&
      status.replace(/_/g, '').toLowerCase() === 'needsreview' &&
      (await askConfirm(
        tr(
          '有成员处于"待复核"：软件没法确认它们的进程是否已经结束（多是中途关了软件）。确认它们已经不在运行后再重跑这些成员？',
          'Some members need review: the app cannot confirm their processes ended (usually the app was closed mid-run). Rerun them once you are sure they are no longer running?',
        ),
        { confirmLabel: tr('重跑', 'Rerun') },
      ));
    if (
      command === 'study_retry' &&
      status.replace(/_/g, '').toLowerCase() === 'needsreview' &&
      !includeReview
    )
      return null;
    // 取消不可撤销：排队的成员标成已取消，之后不能重试（DE 已收代的成员重跑会改写选择历史，
    // 所以后端有意不重试 Cancelled）。原来一点就生效，同屏的重试反倒有确认（GUI 审查 C-01）。
    if (
      command === 'study_cancel' &&
      !(await askConfirm(
        tr(
          '取消后，还没跑的成员不再运行，之后也不能重试；已经跑完的成员结果会保留。要重新跑，只能用同样的设定新建一个研究。确定取消？',
          'Cancelling stops the members that have not run yet, and they cannot be retried later; finished members keep their results. Running them again needs a new study with the same design. Cancel the study?',
        ),
        { confirmLabel: tr('取消研究', 'Cancel study') },
      ))
    )
      return null;
    const token = epoch.current;
    const value = await task(async (current) => {
      const response = await invoke(command, {
        studyDir,
        ...(command === 'study_retry' ? { includeReview } : {}),
      });
      return current() ? await readStatus() : response;
    });
    if (command === 'study_resume' && !runLock.current && token === epoch.current)
      return await start();
    return value;
  };
  const loadResult = () =>
    task(async (current) => {
      const text = await invoke<string>('study_result', { studyDir, path: resultPath });
      if (current()) setResult(text);
      return text;
    });
  const cancelRemote = async () => {
    // 和本机取消一样不可撤销，先问（GUI 审查 C-01）。
    if (
      !(await askConfirm(
        tr(
          '请求服务器取消这个研究：还没跑的成员不再运行，之后也不能重试；已经跑完的成员结果会保留。确定取消？',
          'Ask the server to cancel this study: members that have not run will not run and cannot be retried; finished members keep their results. Cancel it?',
        ),
        { confirmLabel: tr('取消研究', 'Cancel study') },
      ))
    )
      return null;
    return cancelRemoteNow();
  };
  const cancelRemoteNow = () =>
    task(async (current) => {
      remoteRequest.current++;
      const answer = remoteStudyReply(await invokeJson('remote_study_cancel', { studyDir }));
      if (current()) setRemoteReply(answer);
      return answer;
    });
  const fetchRemote = () =>
    task(async (current) => {
      if (!remotePermissions.fetch)
        throw new Error(
          tr(
            '要等服务器确认作业已退出才能取回结果',
            'A confirmed server exit is required before fetching',
          ),
        );
      remoteRequest.current++;
      const answer = await invokeJson<{
        fetched: boolean;
        exit_code: number;
        status: string;
        scientific_verified?: boolean;
        verification_error?: string;
      }>('remote_study_fetch', { studyDir });
      if (!answer.fetched)
        throw new Error(
          tr('服务器没有确认结果已取回', 'Server did not confirm a successful fetch'),
        );
      if (current()) {
        setRemoteReply((previous) => ({
          ...previous,
          scientific_verified: answer.scientific_verified,
          warnings: [
            ...(previous?.warnings ?? []),
            ...(answer.verification_error ? [answer.verification_error] : []),
          ],
        }));
        await readStatus();
        notify(
          tr(
            '服务器研究结果已取回；科学证据另行核对',
            'Server results fetched; scientific evidence is verified separately',
          ),
        );
      }
      return answer;
    });
  const inspectRemote = () =>
    task(async () => {
      await readStatus();
      return await readRemoteStatus();
    });
  const browseStudy = () =>
    task(async (current) => {
      const path = await invoke<string | null>('pick_folder', { key: 'study-open' });
      if (!path || !current()) return path;
      const token = ++epoch.current;
      try {
        const loaded = await readStatus(path, true);
        if (token === epoch.current) {
          setRemoteReply(null);
          setResult('');
          setConfirmed(false);
          const loadedMethod = methodFromStudy(loaded.manifest?.spec);
          setTab(loadedMethod);
          if (loadedMethod === 'process') chooseSlot('soil_stress');
          else if (loadedMethod === 'parameters') chooseSlot('pft');
          setWorkbench(true);
        }
        return loaded;
      } catch (error) {
        if (token === epoch.current) setError(String(error));
        throw error;
      }
    });
  // 研究结果进模型版本和手动改参数走同一条路：先写进草稿，到版本页审阅、冻结。从草稿进来的写回那个草稿，
  // 否则以研究所用的版本为父版本新建一个。原来要么直接冻结成版本（跳过审阅和冻结检查），要么回版本页
  // 再手动找研究目录导入（2026-10-21 用户要求把研究理进"构建新版本"）。
  // 下拉框显示的和真正写入的是同一个成员：没选时用最优；不确定性分析没有最优，原来下拉框显示
  // m000000、实际传空值（后端当作 best），报"Study has no best member yet"。没有最优时取第一个样本成员。
  const defaultMember = () => {
    const best = envelope?.state?.best_member;
    if (best) return best;
    const keys = Object.keys(envelope?.state?.candidates ?? {}).sort();
    return keys.find((key) => key !== 'm000000') ?? keys[0] ?? '';
  };
  const writeBack = () =>
    task(async (current) => {
      const target = state.researchTarget;
      if (!studyDir) throw new Error(tr('请先打开一个跑完的研究', 'Open a finished study first'));
      // 新草稿的父版本只能是研究真正用的版本；没记下来就不猜（当前选的版本未必是它）。
      if (!target && !modelUsed)
        throw new Error(
          tr(
            '这个研究没有记下所用的模型版本，不能据此新建版本；可以在某个草稿的"研究"页签里打开研究，再写回那个草稿。',
            "This study did not record its model version, so no new version can be based on it; open it from a draft's Research tab and write back into that draft.",
          ),
        );
      let draft: ModelDraft;
      let created = false;
      if (target) {
        try {
          draft = await invokeJson<ModelDraft>('model_draft', { id: target.draftId });
        } catch {
          update({ researchTarget: null });
          throw new Error(
            tr(
              `草稿"${target.name}"已经不在了（可能已冻结或删除）；再点一次会新建一个草稿。`,
              `Draft "${target.name}" no longer exists (frozen or deleted); click again to create a new draft.`,
            ),
          );
        }
      } else {
        draft = await invokeJson<ModelDraft>('model_create', {
          name: `${en ? methodTitle.en : methodTitle.zh} ${envelope?.manifest?.id ?? ''}`.trim(),
          note: `${tr('研究结果', 'Study results')}: ${studyDir}`,
          parent: modelUsed!,
          workspace: null,
          branchCode: false,
        });
        created = true;
      }
      try {
        const preview = await invokeJson<Omit<StudyPreview, 'study'>>('model_import_study', {
          id: draft.id,
          studyDir,
          member: member || defaultMember() || null,
          expectedGeneration: draft.generation,
          preview: true,
          only: null,
        });
        if (!current()) throw new Error('Context changed');
        setWriting({ ...preview, study: studyDir, draft, created });
        return preview;
      } catch (e) {
        if (created)
          await invokeJson('model_delete_draft', { id: draft.id, deleteWorkspace: false }).catch(
            () => {},
          );
        throw e;
      }
    });
  function cancelWriteBack() {
    const w = writing;
    setWriting(null);
    // 为这次写入新建的草稿还什么都没写，取消就删掉，不留空草稿。
    if (w?.created)
      void invokeJson('model_delete_draft', { id: w.draft.id, deleteWorkspace: false }).catch(
        () => {},
      );
  }
  const confirmWriteBack = async (chosen: [string, string | null, number | null][]) => {
    const w = writing;
    if (!w) return null;
    const result = await task(() =>
      invokeJson<{ draft: ModelDraft; member: string; parameters: number }>('model_import_study', {
        id: w.draft.id,
        studyDir: w.study,
        member: w.member,
        expectedGeneration: w.draft.generation,
        preview: null,
        only: chosen,
      }),
    );
    setWriting(null);
    notify(
      tr(
        `已把研究成员 ${result.member} 的 ${result.parameters} 个参数写进草稿"${w.draft.name}"，审阅后冻结成新版本。`,
        `Wrote ${result.parameters} parameters of study member ${result.member} into draft "${w.draft.name}"; review it and freeze a new version.`,
      ),
    );
    // 这个研究以后再写也进同一个草稿；导航放在 task 之外，task 持锁时导航守卫会拦下。
    update({
      researchTarget: { draftId: w.draft.id, name: w.draft.name },
      openDraft: w.draft.id,
    });
    navigate('versions');
    return result;
  };
  // AI 的检查、拟合用第 1 步选的算例。原来用的是"模拟"页打开的那个算例（state.selectedCase），
  // 和研究选的算例不是一回事；没在别处打开算例时这两个按钮一直是灰的。
  const aiCase = sites[0] ?? dir;
  const inspectHybrid = () =>
    task(async (current) => {
      const data = await invokeJson<Record<string, unknown>>('hybrid_info', {
        case: await effectiveCase(aiCase),
      });
      if (current()) setHybridInfo(data);
      return data;
    });
  const climate = () =>
    task(async (current) => {
      const text = await invoke('hybrid_climate', { dirs: sites, kernelDir: kernel });
      if (current()) setResult(String(text));
      return text;
    });
  const checkHybrid = () =>
    task(async (current) => {
      const data = await invokeJson('hybrid_check', {
        case: aiCase,
        kernelDir: kernel,
        modelRevision: state.versionRef,
        preprocessors,
      });
      if (current()) setResult(JSON.stringify(data, null, 2));
      return data;
    });
  const findStudies = () =>
    task(async (current) => {
      const rows = await invokeJson<HybridStudy[]>('hybrid_studies', { root: state.root });
      if (current()) setHybridStudies(rows);
      return rows;
    });
  const fitNetwork = () =>
    task(async (current) => {
      if (!/^[A-Za-z0-9_-]+$/.test(name))
        throw new Error(
          tr(
            '拟合名称只能用字母、数字、- 和 _',
            'Fit name must contain only letters, numbers, - and _',
          ),
        );
      if (!chosenStudies.length || !out.trim())
        throw new Error(
          tr(
            '请勾选已完成的研究并选好输出目录',
            'Select completed studies and an output directory',
          ),
        );
      const net = network();
      const penalty = Number(ridge),
        iterations = Number(epochs);
      if (
        !Number.isFinite(penalty) ||
        penalty < 0 ||
        !Number.isInteger(iterations) ||
        iterations < 1
      )
        throw new Error(
          tr('拟合设置不对：岭回归正则要非负，训练轮数要是正整数', 'Invalid fit settings'),
        );
      const data = await invokeJson<Record<string, unknown>>('hybrid_fit', {
        request: {
          studies: chosenStudies,
          network: net,
          kernel_dir: kernel,
          out_dir: out,
          name,
          ridge: hidden === 'linear' ? penalty : null,
          epochs: hidden === 'linear' ? null : iterations,
        },
      });
      if (current())
        setFit({
          ...data,
          networkSpec: net,
          studyDirs: [...chosenStudies],
          sourceCase: aiCase,
          parent: state.versionRef,
        });
      return data;
    });
  const saveFitVersion = () =>
    task(async (current) => {
      if (!fit || typeof fit.model !== 'string' || !fit.networkSpec)
        throw new Error(tr('请先完成拟合', 'Complete a fit first'));
      if (fitValidation(fit.report) === false)
        throw new Error(tr('交叉验证没通过，不能存成版本', 'Cross-validation gate failed'));
      const net = fit.networkSpec as ReturnType<typeof networkSpec>;
      const draft = await invokeJson<{ id: string }>('model_create', {
        name,
        note: JSON.stringify({
          kind: 'hybrid-fit',
          report: fit.report_file,
          network: fit.network,
          studies: fit.studyDirs,
          validation: fitValidation(fit.report),
        }),
        parent: fit.parent,
      });
      try {
        const attached = await invokeJson<{ draft: { id: string }; assetRoot: string }>(
          'model_attach_network',
          {
            id: draft.id,
            caseDir: fit.sourceCase,
            modelPath: fit.model,
            slot: net.slot,
            features: net.features,
            outputs: net.outputs.map(
              (o) =>
                `${o.name}:${o.range[0]}:${o.range[1]}:${o.transform}${'relative' in o && o.relative ? ':relative' : ''}`,
            ),
            normalize: fit.normalize,
            studyDirs: fit.studyDirs,
          },
        );
        const version = await invokeJson<{ version: { id: string; revision: number } }>(
          'model_freeze',
          { id: attached.draft.id, assetRoot: attached.assetRoot },
        );
        await refreshVersions();
        notify(
          `${tr('已保存 AI 模型版本', 'Saved AI model version')}: ${version.version.id}@${version.version.revision}`,
        );
        if (current()) setFit({ ...fit, savedVersion: version.version });
        return version;
      } catch (error) {
        notify(`${tr('保留可恢复草稿', 'Recoverable draft retained')}: ${draft.id}`, true);
        throw error;
      }
    });
  const status = envelope?.state?.status || 'Draft';
  // 打开的研究以它记下的版本为准（可能不是当前选给模拟用的那个）。
  const versionLabel = (v: VersionRef) =>
    `${state.versions.find((x) => x.version.id === v.id && x.version.revision === v.revision)?.name ?? v.id} r${v.revision}`;
  const modelUsed = envelope?.model_version?.model_revision?.version;
  // 研究状态的中文说法；后端的状态名原样留给助手和日志。
  const statusText = (value: string) => {
    const key = value.replace(/_/g, '').toLowerCase();
    const names: Record<string, [string, string]> = {
      draft: ['还没创建', 'Not created'],
      ready: ['已创建，等待运行', 'Created, waiting to run'],
      running: ['运行中', 'Running'],
      paused: ['已暂停', 'Paused'],
      completed: ['已完成', 'Completed'],
      completedwithfailures: ['已完成，有失败的成员', 'Completed with failures'],
      needsreview: ['待复核', 'Needs review'],
      failed: ['失败', 'Failed'],
      cancelled: ['已取消', 'Cancelled'],
      canceled: ['已取消', 'Cancelled'],
      submitted: ['已提交服务器', 'Submitted'],
    };
    const hit = names[key];
    return hit ? tr(hit[0], hit[1]) : value;
  };
  // 本机这次运行还没返回时一律按运行中算：中途读到的检查点状态（旧版后端在批次之间记 Paused、
  // Completed）不能放开"恢复""写入新版本"。
  const permissions = studyPermissions(launching ? 'Running' : status);
  let budgetText: string;
  try {
    const b = candidateBudget(
      tuning ? 'tuning' : 'uncertainty',
      method,
      Object.values(parameters).filter((p) => p.enabled).length,
      Number(count),
      Number(population),
      Number(generations),
      sites.length,
    );
    budgetText = `${b.candidates} candidates + baseline · ${b.tasks} member/site tasks · ${b.stageRuns} stage runs`;
  } catch (e) {
    budgetText = String(e);
  }
  const fieldLabels: Record<string, string> = {
    'Training start UTC': '训练开始 UTC',
    'Training end UTC': '训练结束 UTC',
    'Validation start UTC': '验证开始 UTC',
    'Validation end UTC': '验证结束 UTC',
    Objective: '目标指标',
    'Minimum paired observations': '最少有效观测配对数',
    Trajectories: 'Morris 轨迹数',
    'Score samples against observations': '对照观测算目标函数',
    Population: '种群数量',
    Generations: '迭代代数',
    'Sampling method': '采样方法',
    Candidates: '候选成员数',
    'Concurrent jobs (1–16)': '并发任务（1–16）',
    Seed: '随机种子',
    Slot: '网络作用位置',
    'Input features': '输入特征',
    'Output parameter': '输出参数',
    'Output minimum': '输出下界',
    'Output maximum': '输出上界',
    Network: '网络结构',
    'Relative multiplier': '相对物理值乘数',
    'Train network with differential evolution': '通过差分进化训练网络',
    'Study directory': '研究任务目录',
    'AI fit name': 'AI 拟合名称',
    'Export directory': '导出目录',
    'Fit output directory': '拟合输出目录',
    'Member to write': '要写入的成员',
    'Result file': '研究结果文件',
    'Ridge regularization': '岭回归正则系数',
    'Adam epochs': 'Adam 训练轮数',
    Advanced: '专家设置',
  };
  const unresolvedRemote =
    remotePermissions.exists && !remotePermissions.terminal && !remotePermissions.fetched;
  function chooseMethod(value: string) {
    if (lock.current || runLock.current || unresolvedRemote)
      throw new Error(
        tr(
          '请等当前研究操作结束，或先核对服务器上的研究',
          'Wait for the current study operation or server record to be resolved',
        ),
      );
    if (!hubMethods.some((item) => item.id === value))
      throw new Error(tr('没有这种研究方法', 'Unknown research method'));
    if (value === 'manual' || value === 'source') {
      startVersionMethod(value);
      return;
    }
    // 点方法卡片就是新建这个方法的研究。原来方法没变时直接回到已打开研究的第 4 步，
    // 想用同一方法再建一个研究没有入口（GUI 审查 A-02）；已打开的研究从首页"查看进度"回去。
    if (value !== tab || studyDir) {
      epoch.current++;
      setTab(value);
      setStep(1);
      setConfirmed(false);
      setResult('');
      setFit(null);
      setStudyDir('');
      setEnvelope(null);
      setRemoteReply(null);
      setMember('');
      if (value === 'process') chooseSlot('soil_stress');
      else if (value === 'parameters') chooseSlot('pft');
    }
    setWorkbench(true);
  }
  // 手动改参数和修改源码不跑研究：到版本页直接建好草稿（和"新建修改版"点卡片一样），基于模拟正在用的版本。
  function startVersionMethod(method: 'manual' | 'source') {
    update({
      researchTarget: null,
      researchSeed: null,
      versionsView: 'create',
      versionMethod: { method },
    });
    if (!navigate('versions')) update({ versionMethod: null });
  }
  // 从"新建修改版"或草稿的研究页签点进来：直接打开那个方法，不再停在研究首页。
  useEffect(() => {
    const method = state.researchMethod;
    if (!method) return;
    update({ researchMethod: null });
    setFromHub(!state.researchTarget);
    try {
      chooseMethod(method);
    } catch (e) {
      setError(String(e));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.researchMethod]);
  // 第 3 步的率定期和观测文件原来全空着，要手填日期和绝对路径。所选算例的模拟期和算例记下的观测文件
  // 都现成：进第 3 步时自动填上（几个算例取共同的模拟期），每组算例只算一次。
  // 换了算例要重算：原来只填空着的框，从 4 年的算例换到 3 年的，验证期还是旧值、超出了新的模拟期
  //（GUI 审查 A-01）。现在记住上次自动填的值，框里还是那个值（或空着）就换成新的；用户改过的不动。
  const [casePeriod, setCasePeriod] = useState<[string, string] | null>(null);
  const prefilledFor = useRef('');
  const autoWindows = useRef({ from: '', to: '', valFrom: '', valTo: '' });
  // 每个算例的预热设置决定 history 从哪天开始、要不要再扣一年预热（`suggestedWindows`）。
  // 读不到预热设置就退回整段模拟期，不留验证期；没有共同模拟期（period 为 null）就清掉自动填的值。
  const prefillWindows = async (period: [string, string] | null, live: () => boolean) => {
    const windows = period
      ? await Promise.all(
          sites.map((dir) =>
            invoke<{ start: string; end: string; output_start: string; spinup_years: number }>(
              'read_timing',
              { dirs: [dir] },
            ),
          ),
        )
          .then((timings) =>
            suggestedWindows(
              timings.map((t): SiteTiming => ({
                start: t.start,
                end: t.end,
                outputStart: t.output_start,
                spinupYears: t.spinup_years,
              })),
            ),
          )
          .catch(() => null)
      : null;
    if (!live()) return;
    const day = (text: string) => (text ? `${text}T00:00` : '');
    // 不确定性分析不留验证期：分析时段是扣掉预热后的整段。
    const next = windows
      ? tuning
        ? {
            from: day(windows.from),
            to: day(windows.to),
            valFrom: day(windows.validationFrom),
            valTo: day(windows.validationTo),
          }
        : {
            from: day(windows.from),
            to: day(windows.validationTo || windows.to),
            valFrom: '',
            valTo: '',
          }
      : period
        ? { from: day(period[0]), to: day(period[1]), valFrom: '', valTo: '' }
        : { from: '', to: '', valFrom: '', valTo: '' };
    const auto = autoWindows.current;
    const pick = (key: keyof typeof next) => (old: string) =>
      !old || old === auto[key] ? next[key] : old;
    autoWindows.current = next;
    setFrom(pick('from'));
    setTo(pick('to'));
    setValFrom(pick('valFrom'));
    setValTo(pick('valTo'));
    setCasePeriod(period);
    if (!windows) {
      setWindowNote(null);
      return;
    }
    const w = windows;
    if (!tuning) {
      setWindowNote(
        w.warmup
          ? [
              '算例没有设预热，分析时段从第二年开始，避开初值还没稳定的第一年。',
              'The cases have no spin-up, so the analysis starts in the second year, after the initial state settles.',
            ]
          : null,
      );
      return;
    }
    setWindowNote([
      [
        w.warmup ? '算例没有设预热，率定期从第二年开始，避开初值还没稳定的第一年。' : '',
        w.validationYears
          ? `最后 ${w.validationYears} 年留作验证期，用来检查率定出的参数换个时段是否仍然更好。`
          : w.warmup
            ? '扣掉预热的第一年后不足三年，没有留验证期。'
            : '模拟期不足三年，没有留验证期。',
      ].join(''),
      [
        w.warmup
          ? 'The cases have no spin-up, so calibration starts in the second year, after the initial state settles. '
          : '',
        w.validationYears
          ? `The last ${w.validationYears} year${w.validationYears === 1 ? '' : 's'} are held out for validation, to check that the calibrated parameters still do better in another period.`
          : w.warmup
            ? 'Less than three years remain after the warm-up year, so no validation period is held out.'
            : 'The simulation is shorter than three years, so no validation period is held out.',
      ].join(''),
    ]);
  };
  const siteKey = sites.join('|');
  useEffect(() => {
    // 率定和不确定性分析的预填不同（后者不留验证期），换了方法要重算。
    const key = `${tuning}|${sites.join('|')}#${state.versionRef.id}@${state.versionRef.revision}`;
    if (!workbench || step !== 3 || ai || !sites.length || prefilledFor.current === key) return;
    prefilledFor.current = key;
    let live = true;
    void invoke<Record<string, CaseProfile>>('model_case_profiles', {
      dirs: sites,
      modelRevision: state.versionRef,
    })
      .then((profiles) => {
        if (!live) return;
        const all = sites.map((d) => profiles[d]).filter((p): p is CaseProfile => !!p);
        const starts = all.map((p) => p.inputs.start).filter(Boolean);
        const ends = all.map((p) => p.inputs.end).filter(Boolean);
        const latestStart = [...starts].sort().at(-1),
          earliestEnd = [...ends].sort()[0];
        void prefillWindows(
          starts.length === sites.length &&
            ends.length === sites.length &&
            latestStart &&
            earliestEnd &&
            latestStart < earliestEnd
            ? [latestStart, earliestEnd]
            : null,
          () => live,
        );
        const base = (path: string) =>
          path
            .replace(/[/\\]+$/, '')
            .split(/[/\\]/)
            .pop() || path;
        setObservations((old) => {
          const known = new Set(old.split('\n').map((line) => line.split('|')[0]!.trim()));
          const added = sites
            .filter((d) => profiles[d]?.obs_file && !known.has(base(d)))
            .map((d) => `${base(d)} | ${profiles[d]!.obs_file}`);
          return [old, ...added].filter(Boolean).join('\n');
        });
      })
      // 读不到就照旧手填，不报错。
      .catch(() => {});
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [workbench, step, ai, tuning, siteKey, state.versionRef.id, state.versionRef.revision]);
  // 侧栏的"研究"：回研究首页。离开草稿时留在全局状态里的写回目标若不清掉，之后从首页打开的
  // 任何研究都会写回那个草稿。但为那个草稿跑的研究本身仍该写回它：原来跑完从侧栏回来，
  // 写回悄悄从"写回草稿 X"变成"新建版本"（GUI 审查 B-05）。按研究目录记下它的草稿，
  // 从首页"查看进度"回到同一个研究时还原。
  const studyTargets = useRef<Record<string, NonNullable<typeof state.researchTarget>>>({});
  useEffect(() => {
    if (!state.researchHome) return;
    update({ researchHome: false });
    // 研究正在跑（页面常驻，离开期间一直在跑）：回来直接看它的进度，不回首页、也不报错。
    if (lock.current || runLock.current) {
      setWorkbench(true);
      if (studyDir) setStep(4);
      return;
    }
    setFromHub(false);
    if (studyDir && state.researchTarget) studyTargets.current[studyDir] = state.researchTarget;
    update({ researchTarget: null, researchSeed: null });
    setWorkbench(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.researchHome]);
  // 返回到进来的地方：草稿、"新建修改版"或侧栏的研究首页（侧栏的研究 2026-10-21 先保留）。
  function showHome() {
    if (lock.current || runLock.current)
      throw new Error(tr('研究操作正在进行，请等它结束', 'A study operation is in progress'));
    const target = state.researchTarget;
    // 没用上的参数种子不留到下一次研究。
    update({ researchSeed: null });
    if (target) {
      update({ openDraft: target.draftId });
      if (navigate('versions')) return;
      update({ openDraft: null });
    } else if (fromHub) {
      update({ versionsView: 'create' });
      if (navigate('versions')) return;
    }
    setWorkbench(false);
  }

  const field = (
    id: string,
    label: string,
    value: string,
    set: (v: string) => void,
    type: UiField['type'] = 'text',
    options?: { value: string; label: string }[],
  ): UiField => ({
    id,
    label: en ? label : fieldLabels[label] || label,
    value,
    set,
    type,
    options,
    disabled: busy || launching || unresolvedRemote,
  });
  const knownHosts = [...new Set([...servers, ...(host ? [host] : [])])];
  const baseFields: UiField[] = [
    {
      ...field('research-host', tr('运行位置', 'Machine'), host, setHost, 'choice', [
        { value: '', label: tr('本机', 'Local') },
        ...knownHosts.map((value) => ({ value, label: value })),
      ]),
      disabled: busy || launching || remotePermissions.exists,
    },
    ...(!ai
      ? [
          field('research-engine', tr('执行引擎', 'Engine'), engine, setEngine, 'choice', [
            { value: 'rust', label: 'Rust' },
            { value: 'fortran', label: 'Fortran' },
          ]),
        ]
      : []),
    field('research-tab', tr('研究类型', 'Research type'), tab, chooseMethod, 'choice', [
      { value: 'tuning', label: tr('参数率定', 'Calibration') },
      { value: 'uncertainty', label: tr('不确定性', 'Uncertainty') },
      { value: 'parameters', label: tr('AI 参数学习', 'AI parameter learning') },
      { value: 'process', label: tr('AI 过程替代', 'AI process replacement') },
    ]),
    field(
      'research-kernel',
      tr('编译配置目录', 'Build configuration directory'),
      kernel,
      setKernel,
      'choice',
      [
        { value: '', label: tr('请选择可用的编译配置', 'Select an available build configuration') },
        ...studyKernels.map((k) => ({ value: k.dir, label: k.dir })),
      ],
    ),
    ...state.cases.map((c) =>
      field(
        `research-case-${c.dir}`,
        c.dir,
        String(sites.includes(c.dir)),
        (v) => {
          sitesTouched.current = true;
          setSites((old) =>
            v === 'true' ? [...new Set([...old, c.dir])] : old.filter((d) => d !== c.dir),
          );
        },
        'switch',
      ),
    ),
  ];
  const designFields: UiField[] = [
    field(
      'research-variables',
      tuning
        ? tr('目标变量（逗号分隔）', 'Target variables (comma separated)')
        : tr(
            '输出变量（模型 history 变量名，逗号分隔）',
            'Output variables (model history names, comma separated)',
          ),
      tuning ? variables : outputs,
      tuning ? setVariables : setOutputs,
    ),
    ...(tuning
      ? [
          field(
            'research-observations',
            tr('站点名 | 观测绝对路径，每行一个', 'Site name | observation path, one per line'),
            observations,
            setObservations,
          ),
          field('training-from', 'Training start UTC', from, setFrom),
          field('training-to', 'Training end UTC', to, setTo),
          field('validation-from', 'Validation start UTC', valFrom, setValFrom),
          field('validation-to', 'Validation end UTC', valTo, setValTo),
          field(
            'objective-metric',
            'Objective',
            metric,
            setMetric,
            'choice',
            ['nrmse', 'mae', 'abs_bias', 'nse', 'kge', 'r2', 'r'].map((v) => ({
              value: v,
              label: v,
            })),
          ),
          field('minimum-pairs', 'Minimum paired observations', minPairs, setMinPairs, 'number'),
          field('population', 'Population', population, editPopulation, 'number'),
          field('generations', 'Generations', generations, setGenerations, 'number'),
        ]
      : [
          field('sampling-method', 'Sampling method', method, chooseSampling, 'choice', [
            { value: 'lhs', label: 'Latin hypercube' },
            { value: 'oat', label: 'One at a time' },
            { value: 'morris', label: 'Morris screening' },
          ]),
          ...(method === 'lhs'
            ? [field('candidate-count', 'Candidates', count, editCount, 'number')]
            : method === 'morris'
              ? [field('morris-trajectories', 'Trajectories', count, editCount, 'number')]
              : []),
          field(
            'score-objective',
            'Score samples against observations',
            String(scoreObjective),
            (v) => setScoreObjective(v === 'true'),
            'switch',
          ),
        ]),
    field('research-jobs', 'Concurrent jobs (1–16)', jobs, editJobs, 'number'),
    field('research-seed', 'Seed', seed, setSeed, 'number'),
    field(
      'confirm-ranges',
      tr(
        '我已核查参数范围、预算和适用范围',
        'I reviewed parameter ranges, budget and applicability',
      ),
      String(confirmed),
      (v) => setConfirmed(v === 'true'),
      'switch',
    ),
  ];
  const parameterFields = contexts.flatMap((row, i) => {
    const p = parameters[String(i)] || {
      enabled: false,
      min: '',
      max: '',
      scale: row.scale || 'linear',
    };
    const update = (patch: Partial<typeof p>) =>
      setParameters((old) => ({
        ...old,
        [String(i)]: withSuggestedRange(row, { ...p, ...patch }),
      }));
    const label = `${en ? row.label_en : row.label_zh} · ${row.name} · ${JSON.stringify(row.scope_instance)}`;
    return [
      field(
        `parameter-${i}`,
        label,
        String(p.enabled),
        (v) => update({ enabled: v === 'true' }),
        'switch',
      ),
      ...(p.enabled
        ? [
            field(
              `parameter-min-${i}`,
              `${row.name} min`,
              p.min,
              (v) => update({ min: v }),
              'number',
            ),
            field(
              `parameter-max-${i}`,
              `${row.name} max`,
              p.max,
              (v) => update({ max: v }),
              'number',
            ),
            field(
              `parameter-scale-${i}`,
              `${row.name} scale`,
              p.scale,
              (v) => update({ scale: v }),
              'choice',
              [
                { value: 'linear', label: 'Linear' },
                { value: 'log', label: 'Log' },
              ],
            ),
          ]
        : []),
    ];
  });
  const networkFields: UiField[] = ai
    ? [
        field(
          'network-slot',
          'Slot',
          slot,
          chooseSlot,
          'choice',
          (tab === 'process' ? ['soil_stress'] : ['pft', 'land_class']).map((v) => ({
            value: v,
            label: v,
          })),
        ),
        field('network-features', 'Input features', features, setFeatures),
        field('network-output', 'Output parameter', output, setOutput),
        field('network-min', 'Output minimum', lower, setLower, 'number'),
        field('network-max', 'Output maximum', upper, setUpper, 'number'),
        field('network-hidden', 'Network', hidden, setHidden, 'choice', [
          { value: 'linear', label: 'Linear' },
          { value: '4', label: '4 hidden units' },
          { value: '8', label: '8 hidden units' },
        ]),
        field(
          'network-relative',
          'Relative multiplier',
          String(relative),
          (v) => setRelative(v === 'true'),
          'switch',
        ),
        field(
          'network-training',
          'Train network with differential evolution',
          String(trainNetwork),
          // 替换过程必须训练网络（不训练就没有东西替换）。
          (v) => setTrainNetwork(v === 'true' || tab === 'process'),
          'switch',
        ),
      ]
    : [];
  const lifecycleFields = [
    field('study-directory', 'Study directory', studyDir, (v) => {
      epoch.current++;
      setStudyDir(v);
      setEnvelope(null);
      setRemoteReply(null);
      setResult('');
      setConfirmed(false);
    }),
    field('product-name', 'AI fit name', name, setName),
    field('export-directory', 'Export directory', exportDir, setExportDir),
    field('fit-directory', 'Fit output directory', out, setOut),
    ...(envelope
      ? [
          field('study-member', 'Member to write', member, setMember),
          field(
            'study-result-file',
            'Result file',
            resultPath,
            setResultPath,
            'choice',
            shownResultFiles(envelope.results).map((path) => ({
              value: path,
              label: resultFileLabel(path, en),
            })),
          ),
        ]
      : []),
  ];
  const fitFields = ai
    ? [
        ...hybridStudies.map((s, i) =>
          field(
            `fit-study-${i}`,
            `${s.sites.join(', ')} · ${s.status} · ${s.dir}`,
            String(chosenStudies.includes(s.dir)),
            (v) =>
              setChosenStudies((old) =>
                v === 'true' ? [...new Set([...old, s.dir])] : old.filter((d) => d !== s.dir),
              ),
            'switch',
          ),
        ),
        ...(advanced
          ? [
              field('fit-ridge', 'Ridge regularization', ridge, setRidge, 'number'),
              field('fit-epochs', 'Adam epochs', epochs, setEpochs, 'number'),
            ]
          : []),
      ]
    : [];
  const remoteFields =
    advanced && host
      ? [
          field(
            'research-preprocessors',
            tr('服务器前处理引擎', 'Server preprocessors'),
            preprocessors,
            setPreprocessors,
            'choice',
            [
              { value: 'rust', label: 'Rust' },
              { value: 'fortran', label: 'Fortran' },
            ],
          ),
        ]
      : [];
  const action = (
    id: string,
    label: string,
    run: () => unknown | Promise<unknown>,
    enabled = true,
    commit = false,
  ): UiAction => ({ id, label, run, enabled: enabled && !busy, kind: commit ? 'commit' : 'click' });
  const designActions = [
    action(
      'load-parameter-contexts',
      tr('读取适用参数', 'Load applicable parameters'),
      loadContexts,
      !!kernel && sites.length > 0 && !launching && !unresolvedRemote,
    ),
    action(
      'study-preflight',
      tr('预检', 'Preflight'),
      check,
      !!kernel && !launching && !unresolvedRemote,
    ),
    action(
      'study-create',
      tr('创建研究任务', 'Create study'),
      create,
      !!kernel && confirmed && !launching && !unresolvedRemote,
      true,
    ),
  ];
  // 敏感性"带入率定"：换到参数率定，算例、对照的观测与变量、指标照这个不确定性研究，勾上选中的参数
  // （范围与尺度用它冻结的）。参数靠研究种子在第 2 步读出参数后勾上，和版本页带参数进来是同一条路。
  function carryToCalibration() {
    const spec = frozenSpec;
    const picks = calibrationSeed(spec, carryKeys);
    if (!spec || !picks.length)
      throw new Error(
        tr('请先勾选要带去率定的参数', 'Tick the parameters to carry into calibration'),
      );
    const root = studyDir.replace(/[/\\]\.colm[/\\]studies[/\\][^/\\]+[/\\]?$/, '');
    const sep = studyDir.includes('/') ? '/' : '\\';
    const names = Array.isArray(spec.base_cases) ? spec.base_cases.map(String) : [];
    const targets = (Array.isArray(spec.targets) ? spec.targets : []) as {
      variable?: unknown;
      metric?: unknown;
      corrected?: unknown;
      source?: unknown;
    }[];
    const observed = (spec.observations ?? {}) as Record<string, unknown>;
    chooseMethod('tuning');
    setSites(names.map((name) => `${root}${sep}${name}`));
    if (targets.length && targets.every((t) => !t.source)) {
      setVariables([...new Set(targets.map((t) => String(t.variable)))].join(', '));
      if (typeof targets[0]!.metric === 'string') setMetric(targets[0]!.metric);
      setCorrected(targets.some((t) => t.corrected === true));
      setObservations(
        Object.entries(observed)
          .map(([site, path]) => `${site} | ${String(path)}`)
          .join('\n'),
      );
    }
    loadedFor.current = '';
    update({ researchSeed: { parameters: picks } });
    setStep(2);
  }
  const writeMember = member || defaultMember();
  const writeVerdict = tuning ? memberVerdict(envelope?.state?.candidates, writeMember) : null;
  const writeAllowed = !writeVerdict?.blocked || overrideFor === `${studyDir}#${writeMember}`;
  const lifecycleActions = [
    action(
      'study-browse',
      tr('打开已有研究', 'Open existing study'),
      browseStudy,
      !launching && !unresolvedRemote,
    ),
    action(
      'study-status',
      tr('刷新本机记录', 'Refresh local record'),
      () => task(async () => await readStatus()),
      !!studyDir,
    ),
    action(
      'study-run',
      host ? tr('提交服务器', 'Submit to server') : tr('开始运行', 'Run study'),
      start,
      permissions.run && !!runKernel && confirmed && !launching && !remotePermissions.exists,
      true,
    ),
    ...(!remotePermissions.exists
      ? [
          action(
            'study-pause',
            tr('暂停', 'Pause'),
            () => control('study_pause'),
            permissions.pause,
            true,
          ),
          action(
            'study-resume',
            tr('恢复', 'Resume'),
            () => control('study_resume'),
            // 恢复之后接着就是运行，同样要先核对过冻结的预算。
            permissions.resume && confirmed,
            true,
          ),
          action(
            'study-retry',
            tr('重试失败任务', 'Retry failed tasks'),
            () => control('study_retry'),
            permissions.retry,
            true,
          ),
          action(
            'study-cancel',
            tr('取消研究', 'Cancel study'),
            () => control('study_cancel'),
            permissions.cancel,
            true,
          ),
        ]
      : [
          action(
            'remote-study-status',
            tr('核对服务器状态', 'Check server status'),
            inspectRemote,
            !!studyDir && !launching,
          ),
          action(
            'remote-study-cancel',
            tr('请求取消服务器研究', 'Request server cancellation'),
            cancelRemote,
            remotePermissions.cancel && !launching,
            true,
          ),
          action(
            'remote-study-fetch',
            tr('取回服务器结果', 'Fetch server results'),
            fetchRemote,
            remotePermissions.fetch && !launching,
            true,
          ),
        ]),
    action(
      'study-export',
      tr('导出研究', 'Export study'),
      () =>
        task(async (current) => {
          const value = await invoke('study_export', { studyDir, out: exportDir });
          if (current()) setResult(String(value));
          // 原来导出成功没有任何提示（GUI 审查 A-06）。
          notify(tr(`研究已导出到 ${exportDir}`, `Study exported to ${exportDir}`));
          return value;
        }),
      !!studyDir && !!exportDir && !launching && !unresolvedRemote,
      true,
    ),
    action(
      'study-results',
      tr('读取结果', 'Read results'),
      loadResult,
      !!studyDir && !!resultPath && !unresolvedRemote,
    ),
    action(
      'study-write-version',
      state.researchTarget
        ? tr(
            `写回草稿"${state.researchTarget.name}"…`,
            `Write back to "${state.researchTarget.name}"…`,
          )
        : tr('写入新版本…', 'Write into a new version…'),
      writeBack,
      permissions.results &&
        (!!state.researchTarget || !!modelUsed) &&
        !unresolvedRemote &&
        writeAllowed,
      true,
    ),
  ];
  const aiActions = ai
    ? [
        action('hybrid-info', tr('检查适用性', 'Inspect applicability'), inspectHybrid, !!aiCase),
        action(
          'hybrid-climate',
          tr('生成气候特征', 'Generate climate features'),
          climate,
          !!kernel && sites.length > 0,
          true,
        ),
        action(
          'hybrid-check',
          tr('在所选算例上检查网络', 'Check the network on the chosen case'),
          checkHybrid,
          !!aiCase && !!kernel,
        ),
        action(
          'hybrid-studies',
          tr('查找率定产物', 'Find calibration studies'),
          findStudies,
          !!state.root,
        ),
        action(
          'hybrid-fit',
          tr('拟合特征到参数', 'Fit features to parameters'),
          fitNetwork,
          chosenStudies.length > 0 && !!out && !!kernel,
          true,
        ),
        action(
          'hybrid-save-version',
          tr('保存 AI 为新模型版本', 'Save AI as a new model version'),
          saveFitVersion,
          !!fit && !fit.savedVersion && fitValidation(fit.report) !== false,
          true,
        ),
      ]
    : [];
  const advancedField = {
    ...field(
      'research-advanced',
      'Advanced',
      String(advanced),
      (v) => setAdvanced(v === 'true'),
      'switch',
    ),
    disabled: false,
  };
  const frozenSpec =
    envelope?.manifest?.spec && typeof envelope.manifest.spec === 'object'
      ? (envelope.manifest.spec as Record<string, unknown>)
      : null;
  // 冻结的编译配置不带 SinglePoint 就是格点研究（打开的已有研究不一定勾着算例，不能看 sites）。
  const frozenSpatial = !!state.kernels
    .find((k) => k.dir === runKernel)
    ?.macros.every((macro) => macro !== 'SinglePoint');
  const remoteIdentity =
    envelope?.remote ?? envelope?.remote_pending ?? remoteReply?.record ?? remoteReply;
  const remoteStatus = remotePermissions.fetched
    ? status
    : (remoteReply?.status?.state ?? (envelope?.remote ? 'submitted' : 'unknown'));
  const homeMethod: UiField = {
    id: 'research-tab',
    label: tr('选择研究方法', 'Choose a research method'),
    value: '',
    type: 'choice',
    options: hubMethods.map((item) => ({ value: item.id, label: en ? item.en : item.zh })),
    set: chooseMethod,
    disabled: busy || launching || unresolvedRemote,
  };
  const hasStudy = !!studyDir && !!envelope;
  function returnToSimulation() {
    const target = state.simulationReturn;
    if (!target) return;
    return navigate(target.route, target.route === 'case' ? 'settings' : undefined);
  }
  const homeActions = [
    lifecycleActions[0],
    ...(hasStudy
      ? [
          action(
            'research-current',
            tr('查看进度', 'View progress'),
            () => {
              const kept = studyTargets.current[studyDir];
              if (kept && !state.researchTarget) update({ researchTarget: kept });
              setWorkbench(true);
            },
            !launching,
          ),
        ]
      : []),
    ...(state.simulationReturn
      ? [
          action(
            'research-return-simulation',
            tr('← 返回模拟配置', '← Back to simulation setup'),
            returnToSimulation,
          ),
        ]
      : []),
  ];
  const backAction = action(
    'research-home',
    state.researchTarget
      ? tr('← 回到草稿', '← Back to the draft')
      : fromHub
        ? tr('← 新建修改版', '← New modified version')
        : tr('← 研究首页', '← Research home'),
    showHome,
    !launching,
  );
  const completed = completedCandidates(
    remoteReply?.study?.completed_candidates ?? envelope?.state?.completed_candidates,
  );
  const currentState = remotePermissions.exists ? remoteStatus : launching ? 'Running' : status;
  const methodTitle = researchMethods.find((item) => item.id === tab)!;
  const currentMethod = researchMethods.find((item) => item.id === methodFromStudy(frozenSpec))!;
  // 编译配置：没选、或选的与算例种类不配时重挑。单点挑 default；格点按第一个算例的网格与分类挑
  // （与项目页运行时同一个 compatibleKernel）。界面上只显示配置名。
  useEffect(() => {
    // 没有与算例种类相配的编译配置（比如只有单点配置却选了格点算例）：清掉旧的选择，后面的步骤
    // 才会真的拦住，原来沿用单点配置还能一路点下去（GUI 审查 R11）。
    if (!studyKernels.length) {
      if (kernel) setKernel('');
      return;
    }
    if (studyKernels.some((k) => k.dir === kernel)) return;
    if (!spatialStudy) {
      setKernel((studyKernels.find((k) => k.preset === 'default') ?? studyKernels[0]!).dir);
      return;
    }
    let current = true;
    invoke<{ profile: Parameters<typeof compatibleKernel>[1] }>('model_case_context', {
      dir: sites[0],
      modelRevision: state.versionRef,
    })
      .then((context) => {
        if (current)
          setKernel((compatibleKernel(studyKernels, context.profile) ?? studyKernels[0]!).dir);
      })
      .catch(() => {
        if (current) setKernel(studyKernels[0]!.dir);
      });
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [state.kernels, kernel, spatialStudy, sites]);
  // 项目里只有一个单点算例时直接勾上（没有别的可选）。换版本会清空所选算例，所以不是只勾一次；
  // 用户自己动过勾选就不再替他勾。
  const sitesTouched = useRef(false);
  useEffect(() => {
    if (sitesTouched.current || sites.length) return;
    const single = state.cases.filter((c) => !c.spatial);
    if (single.length === 1) setSites([single[0]!.dir]);
  }, [state.cases, sites]);
  // 格点算例：进到第 2 步先读能选哪些地类或植物类型（landdata 里有的），版本页带来的类型自动勾上。
  useEffect(() => {
    if (step !== 2 || !spatialStudy || !kernel || !sites.length || typeChoices) return;
    let current = true;
    invokeJson<TypeChoices>('study_type_choices', {
      dirs: sites,
      kernelDir: kernel,
    })
      .then((choices) => {
        if (!current) return;
        setTypeChoices(choices);
        const seeded = (state.researchSeed?.parameters ?? [])
          .map((pick) => pick.index)
          .filter((index): index is number => index != null)
          .filter((index) => index >= 1 && index <= choices.max)
          .filter((index) => !choices.present || choices.present.includes(index));
        if (seeded.length) setTypes([...new Set(seeded)].sort((a, b) => a - b));
      })
      .catch((e) => {
        if (current) setError(String(e));
      });
    return () => {
      current = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, spatialStudy, kernel, sites, typeChoices]);
  // 进到第 2 步就读出这些算例能研究的参数（原来要先点"读取适用参数"）；同一组算例、同一组类型只读一次。
  // 格点算例要先选了类型（不选也能读：只有整算例的系数）。
  useEffect(() => {
    const key = `${kernel}|${sites.join('|')}|${spatialStudy ? types.join(',') : ''}`;
    if (step !== 2 || !kernel || !sites.length || busy || loadedFor.current === key) return;
    if (spatialStudy && !typeChoices) return;
    loadedFor.current = key;
    execute(loadContexts);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, kernel, sites, busy, types, typeChoices, spatialStudy]);
  // 建好或打开了一个研究：直接看运行与结果。
  useEffect(() => {
    if (studyDir) setStep(4);
  }, [studyDir]);
  useEffect(() => {
    setTargetChoices(null);
    // 交服务器的研究，观测是服务器上的路径，本机读不到；格点率定用目标编辑器。
    if (step !== 3 || !scoring || spatialStudy || host || !kernel || !sites.length) return;
    const obsOf = new Map(
      observations
        .split('\n')
        .map((line) => line.split('|'))
        .filter((parts) => parts.length >= 2)
        .map(([name, ...rest]) => [name!.trim(), rest.join('|').trim()]),
    );
    const pairs = sites.flatMap((d) => {
      const obs = obsOf.get(
        d
          .replace(/[/\\]+$/, '')
          .split(/[/\\]/)
          .pop() || d,
      );
      return obs ? [[d, obs] as const] : [];
    });
    if (!pairs.length) return;
    let live = true;
    Promise.all(
      pairs.map(([d, obs]) =>
        invokeJson<TargetChoice[]>('evaluation_plan', { case: d, obs, kernelDir: kernel }),
      ),
    )
      .then((plans) => {
        if (!live) return;
        const everywhere = (name: string) =>
          plans.every((plan) => plan.some((row) => row.name === name && row.available));
        setTargetChoices(plans[0]!.filter((row) => everywhere(row.name)));
      })
      // 读不到（观测路径填错、文件不是 NetCDF）就只留手填框，预检会给出具体原因。
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [step, scoring, spatialStudy, host, kernel, sites, observations]);
  const tickedCount = Object.values(parameters).filter((p) => p.enabled).length;
  // 差分进化的维数 = 勾选的参数 + 要训练的网络权重。原来只数勾选的参数：训练网络时通常一个都不勾，
  // 种群停在 8，而默认网络有 4–41 个权重（GUI 审查 B-02）。网络设计没填好时按 0 个权重算。
  const weightCount = (() => {
    if (!(ai && (trainNetwork || tab === 'process'))) return 0;
    try {
      return networkWeightCount(network());
    } catch {
      return 0;
    }
  })();
  const dimensionCount = tickedCount + weightCount;
  useEffect(() => {
    // 原来固定 4：16 核的本机和 48 核的服务器都只跑 4 个。本机留四分之一给界面和系统；
    // 服务器取研究规格允许的上限 16。
    if (!jobsEdited.current) {
      const cores = globalThis.navigator?.hardwareConcurrency || 4;
      setJobs(String(host ? 16 : Math.min(16, Math.max(1, Math.floor(cores * 0.75)))));
    }
    // 差分进化的种群太小会早熟收敛；常用的经验是维数的 5–10 倍，这里取 5 倍、至少 8 组。
    // 上限 100：再大一代就要跑上百次模拟，网络很大时预算提示会建议缩小网络或减少代数。
    if (!populationEdited.current)
      setPopulation(String(Math.min(100, Math.max(8, 5 * dimensionCount))));
  }, [host, dimensionCount]);
  // 勾过"我已核对参数范围、预算和适用范围"之后又改了设计，勾选还留着，核对的已不是要建的那个研究
  //（GUI 审查 B-06）。已打开的研究核对的是冻结的规格，与这些框无关，不动它。
  useEffect(() => {
    if (!studyDir) setConfirmed(false);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    tab,
    observations,
    variables,
    outputs,
    method,
    count,
    population,
    generations,
    seed,
    from,
    to,
    valFrom,
    valTo,
    metric,
    minPairs,
    parameters,
    corrected,
    gridTargets,
    trainNetwork,
    scoreObjective,
    patience,
    aggregate,
    weights,
  ]);
  // 换了研究，"用哪个成员"还留着上一个研究的成员编号（GUI 审查 A-07），"仍要写入"也一样。
  useEffect(() => {
    setMember('');
    setOverrideFor('');
  }, [studyDir]);
  // 研究页离开后只是被隐藏，写回对话框挂在 body 上（Radix Portal），会浮在新页面上并锁住焦点
  //（GUI 审查 R12/C-10）。离开就关掉；草稿此时还没写，回来重新点"写入"即可。
  useEffect(() => {
    if (!active) setWriting(null);
  }, [active]);
  // 不确定性研究跑完就读敏感性，不用再从结果文件下拉框里找 importance.json。默认看目标函数，
  // 勾上影响最大的前三个（影响为 0 的不勾）。
  const hasImportance = (envelope?.results ?? []).some(
    (entry) => (typeof entry === 'string' ? entry : entry.path) === 'importance.json',
  );
  useEffect(() => {
    setImportance(null);
    setCarryKeys([]);
    if (!studyDir || !hasImportance) return;
    let live = true;
    void invoke<string>('study_result', { studyDir, path: 'importance.json' })
      .then((text) => {
        if (!live) return;
        const rows: unknown = JSON.parse(text);
        const first = sensitivityResponses(rows)[0]?.key ?? '';
        setImportance(rows);
        setSensitivityKey(first);
        setCarryKeys(
          sensitivityRanking(rows, first, frozenSpec)
            .filter((row) => (row.impact ?? 0) > 0)
            .slice(0, 3)
            .map((row) => row.parameter),
        );
      })
      // 读不到就不显示这张卡片；结果文件下拉框里仍能打开它看原因。
      .catch(() => {});
    return () => {
      live = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [studyDir, hasImportance]);
  const hasNearOptimal = (envelope?.results ?? []).some(
    (entry) => (typeof entry === 'string' ? entry : entry.path) === 'near_optimal.json',
  );
  useEffect(() => {
    setNearOptimal(null);
    if (!studyDir || !hasNearOptimal) return;
    let live = true;
    void invoke<string>('study_result', { studyDir, path: 'near_optimal.json' })
      .then((text) => {
        if (live) setNearOptimal(JSON.parse(text));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [studyDir, hasNearOptimal]);
  const hasCrossValidation = (envelope?.results ?? []).some(
    (entry) => (typeof entry === 'string' ? entry : entry.path) === 'cross_validation.json',
  );
  useEffect(() => {
    setCrossValidation(null);
    if (!studyDir || !hasCrossValidation) return;
    let live = true;
    void invoke<string>('study_result', { studyDir, path: 'cross_validation.json' })
      .then((text) => {
        if (live) setCrossValidation(JSON.parse(text));
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [studyDir, hasCrossValidation]);
  usePageContract(
    workbench
      ? {
          title: tr('研究工作台', 'Research workbench'),
          step: 'research',
          kernelDir: kernel || null,
          preprocessors,
          status: busy ? 'working' : launching ? 'running' : error || currentState,
          fields: [
            ...baseFields,
            ...networkFields,
            ...parameterFields,
            ...designFields,
            ...lifecycleFields,
            ...fitFields,
            ...remoteFields,
            advancedField,
          ],
          // 按钮自己登记（第 737 轮）：原来清单里另有一份动作，助手按的是清单里的回调，
          // 界面改了清单不一定跟着改（出过幽灵按钮）。现在只认真正渲染出来的按钮。
          actions: [],
          // 给助手的是研究摘要，不是整份 envelope：候选、任务、事件在合法上限下几百 KiB，每条消息都带上
          // 又慢又贵（GUI 审查 D-03）。要看成员细节，助手可以读结果文件。
          details: {
            studyDir,
            study: envelope && {
              id: envelope.manifest?.id,
              status: envelope.state?.status,
              completed_candidates: envelope.state?.completed_candidates,
              best_member: envelope.state?.best_member,
              best: envelope.state?.best_member
                ? envelope.state?.candidates?.[envelope.state.best_member]
                : undefined,
              baseline: envelope.state?.candidates?.m000000,
              warnings: studyDiagnostics(envelope.state?.warnings).map(([zh, english]) =>
                en ? english : zh,
              ),
              results: shownResultFiles(envelope.results),
              run_options: envelope.run_options,
              spec: frozenSpec,
            },
            remote: remoteReply,
            preflight,
            budget: budgetText,
            hybridInfo,
            fit,
            contexts,
            hybridStudies,
            result,
            error,
          },
        }
      : {
          title: tr('研究与模型改进', 'Research and model improvement'),
          step: 'research',
          status: busy ? 'working' : error || currentState,
          fields: [homeMethod],
          actions: [],
          details: {
            studyDir: hasStudy ? studyDir : null,
            status: hasStudy ? currentState : null,
            completedCandidates: hasStudy ? completed : null,
            error,
          },
        },
    active,
  );

  // 动作由渲染出的按钮自己登记：助手看到的 enabled 和是否显示都与按钮本身一致。
  // `plain` 里的动作只渲染普通按钮、不登记：同一个动作在界面上有两个按钮时（研究首页顶部的
  // "返回模拟配置"和"当前研究"卡片里的同一个），动作 id 只能登记一次，否则助手按 id 找不到唯一按钮。
  const renderActions = (actions: UiAction[], plain: string[] = []) =>
    actions.map((a) => {
      const variant = (
        a.id === 'study-cancel' ? 'danger' : a.kind === 'commit' ? 'primary' : 'secondary'
      ) as 'primary' | 'secondary' | 'danger';
      return plain.includes(a.id) ? (
        <Button key={a.id} disabled={!a.enabled} variant={variant} onClick={() => execute(a.run)}>
          {a.label}
        </Button>
      ) : (
        <ActionButton
          key={a.id}
          action={a.id}
          label={a.label}
          kind={a.kind}
          disabled={!a.enabled}
          variant={variant}
          run={a.run}
        />
      );
    });
  // 没有算例时给出能点的下一步，而不是只写一句"先去项目里"（GUI 审查 C-09）。
  const noCasesActions = (
    <div className="button-row">
      <ActionButton
        action="research-open-project"
        label={tr('打开项目', 'Open a project')}
        run={() => navigate('project')}
      />
      <ActionButton
        action="research-new-case"
        label={tr('新建模拟', 'Create a simulation')}
        run={() => navigate('new-case')}
      />
    </div>
  );
  if (!workbench)
    return (
      <div className="research-home">
        {state.simulationReturn && (
          <div className="button-row">
            {/* 首页顶部的返回按钮始终显示，交给它登记；"当前研究"卡片里那个同名按钮只渲染、不登记。 */}
            <ActionButton
              action="research-return-simulation"
              label={tr('← 返回模拟配置', '← Back to simulation setup')}
              run={returnToSimulation}
            >
              ← {tr('返回模拟配置', 'Back to simulation setup')}
            </ActionButton>
          </div>
        )}
        <header className="research-heading">
          <div>
            <h1>{tr('研究与模型改进', 'Research and model improvement')}</h1>
            <p>
              {tr(
                '先明确研究目标，再选择方法。研究跑完可以把结果写进一个新的模型版本。',
                'Start with a research question, then choose a method. Finished results can go into a new model version.',
              )}
            </p>
          </div>
        </header>
        {error && <Notice tone="error">{shownError}</Notice>}
        {/* 率定、不确定性和 AI 都在项目的算例上跑；还没打开项目时先说清楚，不让人选了方法才发现没算例。 */}
        {!state.cases.length && (
          <Notice>
            {tr(
              '率定、不确定性分析和 AI 研究要在项目的算例上跑：先打开项目或新建模拟。手动改参数和修改源码不需要算例。',
              'Calibration, uncertainty and AI studies run on the cases of a project: open a project or create a simulation first. Manual parameter changes and source edits need no case.',
            )}
            {noCasesActions}
          </Notice>
        )}
        <div className="research-methods" role="group" aria-label={homeMethod.label}>
          {hubMethods.map((item) => (
            <ActionButton
              key={item.id}
              action={`research-method-${item.id}`}
              label={en ? item.en : item.zh}
              id={`research-method-${item.id}`}
              className="research-method"
              // 没有算例时要算例的方法点进去是死路（GUI 审查 C-09）：灰着并说明，手动改参数、改源码照常可点。
              disabled={homeMethod.disabled || (!state.cases.length && needsCases(item.id))}
              title={
                !state.cases.length && needsCases(item.id)
                  ? tr('先打开项目或新建模拟', 'Open a project or create a simulation first')
                  : undefined
              }
              run={() => {
                // 研究首页新开的研究不写回之前的草稿。
                setFromHub(false);
                // 草稿页签带来的参数和要写回的草稿只属于从那里进来的那次研究。
                update({ researchTarget: null, researchSeed: null });
                chooseMethod(item.id);
              }}
            >
              <span className="research-method-icon">
                <svg
                  viewBox="0 0 24 24"
                  aria-hidden="true"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                >
                  <path d={item.icon} />
                </svg>
              </span>
              <span className="research-method-copy">
                <strong>{en ? item.en : item.zh}</strong>
                <span>{en ? item.descriptionEn : item.descriptionZh}</span>
                {(item.id === 'manual' || item.id === 'source') && (
                  <span className="version-hub-tag">
                    {tr('不跑研究，直接建草稿', 'No study: opens a draft directly')}
                  </span>
                )}
              </span>
              <span className="research-method-arrow" aria-hidden="true">
                ›
              </span>
            </ActionButton>
          ))}
        </div>
        <section aria-label={tr('当前研究', 'Current study')}>
          <div className="research-current-heading">
            <h2>{tr('当前研究', 'Current study')}</h2>
            {renderActions(homeActions.slice(0, 1))}
          </div>
          {hasStudy ? (
            <div className="research-current">
              <div className="research-current-copy">
                <h3>
                  {en ? currentMethod.en : currentMethod.zh} ·{' '}
                  {envelope?.manifest?.id || studyDir.split(/[/\\]/).pop()}
                </h3>
                <p>
                  {envelope?.model_version?.model_revision?.version
                    ? versionLabel(envelope.model_version.model_revision.version)
                    : tr('模型修订待核对', 'Model revision unverified')}{' '}
                  ·{' '}
                  {envelope
                    ? remoteIdentity?.host || tr('本机', 'Local')
                    : tr('运行位置待核对', 'Machine unverified')}
                </p>
                <p className="path">{studyDir}</p>
              </div>
              <div className="research-progress">
                {envelope ? statusText(currentState) : tr('状态待核对', 'Status unverified')}
                <span>
                  {completed === null
                    ? tr('尚无候选完成记录', 'No candidate progress recorded')
                    : `${tr('已完成候选', 'Completed candidates')}: ${completed}`}
                </span>
              </div>
              {/* 顶部的返回按钮已经登记了这个动作，卡片里这个是同一个动作的第二个按钮。 */}
              {renderActions(homeActions.slice(1), ['research-return-simulation'])}
            </div>
          ) : (
            <div className="research-empty">
              <p>
                {state.cases.length
                  ? tr(
                      '尚未打开研究任务。选择上方方法开始，或打开已有研究。',
                      'No study is open. Choose a method above or open an existing study.',
                    )
                  : tr(
                      '尚未打开研究任务。打开项目、有了算例之后，就能在上方选方法开始；也可以打开已有研究。',
                      'No study is open. Once a project with cases is open you can choose a method above, or open an existing study.',
                    )}
              </p>
              <p>
                {tr(
                  '这里显示本次打开的研究和它的进度。',
                  'The study opened in this session and its recorded status appear here.',
                )}
              </p>
            </div>
          )}
        </section>
      </div>
    );
  const kernelEntry = state.kernels.find((k) => k.dir === kernel);
  const siteName = (path: string) =>
    path
      .replace(/[/\\]+$/, '')
      .split(/[/\\]/)
      .pop() || path;
  // 观测文件仍按原来的"站点名 | 绝对路径"一行一个存（研究规格就这样读），界面上按站点逐个选。
  const observationOf: Record<string, string> = Object.fromEntries(
    observations
      .split('\n')
      .map((line) => line.split('|'))
      .filter((parts) => parts.length >= 2 && parts[0]!.trim())
      .map(([name, ...rest]) => [name!.trim(), rest.join('|').trim()]),
  );
  const setObservation = (site: string, path: string) => {
    const next = { ...observationOf, [site]: path };
    setObservations(
      sites
        .map(siteName)
        .filter((name) => next[name])
        .map((name) => `${name} | ${next[name]}`)
        .join('\n'),
    );
  };
  const pickObservation = (site: string) =>
    execute(async () => {
      const path = await invoke<string | null>('pick_file', { key: 'study-observation' });
      if (path) setObservation(site, path);
    });
  const setGridTarget = (index: number, patch: Partial<GridTargetDraft>) =>
    setGridTargets((old) => old.map((t, i) => (i === index ? { ...t, ...patch } : t)));
  const pickTargetFile = (index: number) =>
    execute(async () => {
      const path = await invoke<string | null>('pick_file', { key: 'study-observation' });
      if (path) setGridTarget(index, { path });
    });
  const targetKinds: [GridTargetDraft['kind'], string, string][] = [
    ['grid', '格点观测产品（NetCDF）', 'Gridded product (NetCDF)'],
    ['station', '区域里的通量站', 'Flux station in the domain'],
    ['gauge', '水文站流量', 'River gauge discharge'],
  ];
  const metricNames: Record<string, [string, string]> = {
    nrmse: ['NRMSE 归一化均方根误差（越小越好）', 'NRMSE (lower is better)'],
    mae: ['MAE 平均绝对误差（越小越好）', 'MAE (lower is better)'],
    abs_bias: ['绝对偏差（越小越好）', 'Absolute bias (lower is better)'],
    nse: ['NSE 效率系数（越大越好）', 'NSE (higher is better)'],
    kge: ['KGE 效率系数（越大越好）', 'KGE (higher is better)'],
    r2: ['R² 决定系数（越大越好）', 'R² (higher is better)'],
    r: ['r 相关系数（越大越好）', 'Correlation r (higher is better)'],
  };
  const gridTargetEditor = (
    <>
      <h3 className="study-subhead">{tr('率定目标', 'Calibration targets')}</h3>
      <p className="muted study-hint">
        {tr(
          '格点算例拿三种观测比：格点产品（如 GLEAM 蒸散）按模型格子平均后逐格比；通量站比站点所在那一格；水文站比站点所在汇流单元的流量。',
          'Gridded cases are compared with three kinds of observations: a gridded product (such as GLEAM evaporation) averaged into model cells and compared cell by cell; a flux station against the cell it sits in; a river gauge against the discharge of its unit catchment.',
        )}
      </p>
      <div className="study-targets">
        {gridTargets.map((t, index) => (
          <div key={index} className="study-target">
            <Field label={tr('种类', 'Kind')}>
              <Select
                value={t.kind}
                onChange={(e) => {
                  const kind = e.target.value as GridTargetDraft['kind'];
                  setGridTarget(index, {
                    ...gridTargetDefaults[kind],
                    path: t.path,
                    metric: t.metric,
                  });
                }}
              >
                {targetKinds.map(([value, zh, english]) => (
                  <option key={value} value={value}>
                    {en ? english : zh}
                  </option>
                ))}
              </Select>
            </Field>
            <Field
              label={
                t.kind === 'station'
                  ? tr('观测变量（如 Qle）', 'Observed variable (e.g. Qle)')
                  : tr('模型输出变量', 'Model output variable')
              }
            >
              <Input
                value={t.variable}
                onChange={(e) => setGridTarget(index, { variable: e.target.value })}
              />
            </Field>
            {t.kind !== 'station' && (
              <Field
                label={
                  t.kind === 'grid'
                    ? tr('观测文件里的变量名', 'Variable in the observation file')
                    : tr('观测变量名（NetCDF 才填）', 'Observed variable (NetCDF only)')
                }
              >
                <Input
                  value={t.obsVariable}
                  onChange={(e) => setGridTarget(index, { obsVariable: e.target.value })}
                />
              </Field>
            )}
            {t.kind === 'grid' ? (
              <Field label={tr('观测乘多少换成模型单位', 'Scale from observed to model units')}>
                <Input
                  type="number"
                  value={t.scale}
                  onChange={(e) => setGridTarget(index, { scale: e.target.value })}
                />
              </Field>
            ) : (
              <>
                <Field label={tr('站点经度', 'Station longitude')}>
                  <Input
                    type="number"
                    value={t.lon}
                    onChange={(e) => setGridTarget(index, { lon: e.target.value })}
                  />
                </Field>
                <Field label={tr('站点纬度', 'Station latitude')}>
                  <Input
                    type="number"
                    value={t.lat}
                    onChange={(e) => setGridTarget(index, { lat: e.target.value })}
                  />
                </Field>
                {t.kind === 'station' && (
                  <Field
                    label={tr(
                      '观测比世界时早几小时（PLUMBER2 是当地标准时，中国填 8）',
                      'Hours the observations lead UTC (PLUMBER2 uses local standard time; 8 in China)',
                    )}
                  >
                    <Input
                      type="number"
                      value={t.utcOffset}
                      onChange={(e) => setGridTarget(index, { utcOffset: e.target.value })}
                    />
                  </Field>
                )}
              </>
            )}
            {t.kind !== 'station' && (
              <Field label={tr('按什么时段平均后比', 'Compare averages over')}>
                <Select
                  value={t.period}
                  onChange={(e) =>
                    setGridTarget(index, { period: e.target.value as GridTargetDraft['period'] })
                  }
                >
                  <option value="">{tr('按观测的时间间隔', 'The observation interval')}</option>
                  <option value="daily">{tr('逐日', 'Daily')}</option>
                  <option value="monthly">{tr('逐月', 'Monthly')}</option>
                </Select>
              </Field>
            )}
            {/* 格点率定常同时比流量与通量：流量看 NSE/KGE，通量看 NRMSE，原来只能共用一个（第 734 轮）。 */}
            <Field label={tr('比较指标', 'Metric')}>
              <Select
                value={t.metric ?? ''}
                onChange={(e) => setGridTarget(index, { metric: e.target.value || undefined })}
              >
                <option value="">
                  {tr(
                    `沿用总体指标（${metricNames[metric]?.[0].split(' ')[0] ?? metric}）`,
                    `Same as the study metric (${metricNames[metric]?.[1].split(' ')[0] ?? metric})`,
                  )}
                </option>
                {Object.entries(metricNames).map(([value, [zh, english]]) => (
                  <option key={value} value={value}>
                    {en ? english : zh}
                  </option>
                ))}
              </Select>
            </Field>
            <div className="study-path study-wide">
              <Input
                value={t.path}
                placeholder={
                  t.kind === 'gauge'
                    ? tr(
                        '观测文件（NetCDF，或"日期,流量"的 CSV）',
                        'Observation file (NetCDF, or a date,discharge CSV)',
                      )
                    : tr('观测文件的绝对路径', 'Absolute path of the observation file')
                }
                onChange={(e) => setGridTarget(index, { path: e.target.value })}
              />
              <ActionButton
                action={`study-target-${index}-pick`}
                label={tr('选择观测文件', 'Choose observation file')}
                run={() => pickTargetFile(index)}
              >
                {tr('选择…', 'Choose…')}
              </ActionButton>
            </div>
            {gridTargets.length > 1 && (
              <div className="study-wide">
                <ActionButton
                  bare
                  action={`study-target-${index}-remove`}
                  label={tr('删掉这个目标', 'Remove this target')}
                  className="library-action"
                  run={() => setGridTargets((old) => old.filter((_, i) => i !== index))}
                >
                  {tr('删掉这个目标', 'Remove this target')}
                </ActionButton>
              </div>
            )}
          </div>
        ))}
      </div>
      <ActionButton
        bare
        action="study-target-add"
        label={tr('再加一个目标', 'Add another target')}
        className="library-action"
        run={() => setGridTargets((old) => [...old, { ...gridTargetDefaults.grid }])}
      >
        {tr('再加一个目标', 'Add another target')}
      </ActionButton>
    </>
  );
  const pickFolder = (key: string, set: (path: string) => void) =>
    execute(async () => {
      const path = await invoke<string | null>('pick_folder', { key });
      if (path) set(path);
    });
  const resultFiles = shownResultFiles(envelope?.results);
  const chosenCount = Object.values(parameters).filter((p) => p.enabled).length;
  const variableNames = variables
    .split(',')
    .map((name) => name.trim())
    .filter(Boolean);
  // 和后端一样不分大小写（GUI 审查 B-01）。
  const unknownTargets = variableNames.filter(
    (name) => !targetChoices?.some((row) => row.name.toLowerCase() === name.toLowerCase()),
  );
  const unranged = contexts.flatMap((row, i) => {
    const p = parameters[String(i)];
    return p?.enabled && (!p.min.trim() || !p.max.trim()) ? [row.name] : [];
  });
  let budgetSummary: string;
  let budgetOk = true;
  try {
    const b = candidateBudget(
      tuning ? 'tuning' : 'uncertainty',
      method,
      chosenCount,
      Number(count),
      Number(population),
      Number(generations),
      sites.length,
    );
    budgetSummary = tr(
      `${b.candidates} 组参数 + 1 组基准，${sites.length} 个${spatialStudy ? '算例' : '站点'}，共 ${b.tasks} 次模拟`,
      `${b.candidates} parameter sets + 1 baseline across ${sites.length} ${spatialStudy ? 'case' : 'site'}${sites.length === 1 ? '' : 's'}: ${b.tasks} simulations`,
    );
  } catch {
    budgetOk = false;
    budgetSummary = tuning
      ? tr(
          '种群至少 4、迭代至少 1 代，总数不超过 1000。',
          'Population ≥ 4, generations ≥ 1, at most 1000 in total.',
        )
      : tr(
          '样本数 1–1000；一次一个法需要先选参数。',
          'Samples 1–1000; one-at-a-time needs parameters chosen first.',
        );
  }
  const scopeText = (row: ParameterContext) => {
    const s = row.scope_instance;
    if (s.kind === 'land-cover-class') {
      const table = s.scheme === 'USGS' ? USGS : IGBP;
      const hit = table.find(([id]) => id === s.index);
      return `${s.scheme ?? ''} ${s.index ?? ''} ${hit ? (en ? hit[2] : hit[1]) : ''}`.trim();
    }
    if (s.kind === 'pft-type' || s.kind === 'pc-pft-component') {
      const hit = PFT.find(([id]) => id === s.index);
      return `${s.kind === 'pc-pft-component' ? 'PC ' : 'PFT '}${s.index ?? ''} ${hit ? (en ? hit[2] : hit[1]) : (s.type_name ?? '')}`;
    }
    if (s.kind === 'soil-layer')
      return tr(`第 ${s.index ?? ''} 层土壤`, `Soil layer ${s.index ?? ''}`);
    return tr('全局参数', 'Global parameters');
  };
  const groupKey = (row: ParameterContext) =>
    `${row.scope_instance.kind}|${row.scope_instance.scheme ?? ''}|${row.scope_instance.index ?? ''}`;
  // 参数目录用 ±最小正规数表示"严格大于/小于 0"（照搬 Fortran 的 tiny），直接显示成 -2.2e-308 没人看得懂。
  const bound = (value: number | undefined, inclusive: boolean | undefined, lower: boolean) => {
    if (value == null) return '';
    const zero = value !== 0 && Math.abs(value) < 1e-300;
    const strict = zero || inclusive === false;
    return `${lower ? (strict ? '>' : '≥') : strict ? '<' : '≤'} ${zero ? 0 : Number(value.toPrecision(6))}`;
  };
  const groupOrder = [
    'case-scalar',
    'land-cover-class',
    'pft-type',
    'pc-pft-component',
    'soil-layer',
  ];
  const contextOrder = contexts
    .map((row, i) => ({ row, i }))
    .sort(
      (a, b) =>
        groupOrder.indexOf(a.row.scope_instance.kind) -
          groupOrder.indexOf(b.row.scope_instance.kind) ||
        (a.row.scope_instance.index ?? 0) - (b.row.scope_instance.index ?? 0),
    );
  const setParameter = (
    i: number,
    row: ParameterContext,
    patch: Partial<{ enabled: boolean; min: string; max: string; scale: string }>,
  ) =>
    setParameters((old) => {
      const p = old[String(i)] || {
        enabled: false,
        min: '',
        max: '',
        scale: row.scale || 'linear',
      };
      return { ...old, [String(i)]: withSuggestedRange(row, { ...p, ...patch }) };
    });
  const ranked = Object.entries(envelope?.state?.candidates ?? {})
    .sort(
      ([, a], [, b]) =>
        (a.calibration ?? Number.POSITIVE_INFINITY) - (b.calibration ?? Number.POSITIVE_INFINITY),
    )
    .slice(0, 200);
  const windowHint = casePeriod && (
    <p className="muted study-hint study-wide">
      {tr(
        `所选算例的模拟期：${casePeriod[0]} 至 ${casePeriod[1]}。`,
        `Simulation period of the chosen cases: ${casePeriod[0]} to ${casePeriod[1]}.`,
      )}
      {windowNote && ` ${tr(windowNote[0], windowNote[1])}`}
    </p>
  );
  const diagnostics = studyDiagnostics(envelope?.state?.warnings);
  const convergence = convergenceRows(envelope?.state?.candidates);
  const near = nearOptimalRows(nearOptimal);
  const folds = crossValidationRows(crossValidation);
  const responses = sensitivityResponses(importance);
  const ranking = sensitivityRanking(importance, sensitivityKey, frozenSpec);
  const objectiveView = responses.find((r) => r.key === sensitivityKey)?.variable === 'objective';
  const rankingMethod = ranking[0]?.method ?? '';
  const carryAction = action(
    'study-carry-calibration',
    tr(`带入率定（${carryKeys.length} 个参数）`, `Carry into calibration (${carryKeys.length})`),
    carryToCalibration,
    carryKeys.length > 0 && !launching && !lock.current,
  );
  const best = envelope?.state?.best_member;
  const controlIds = [
    'study-run',
    'study-pause',
    'study-resume',
    'study-retry',
    'study-cancel',
    'remote-study-status',
    'remote-study-cancel',
    'remote-study-fetch',
  ];
  // 运行、恢复原来只在可用时才渲染：打开一个待运行的研究，"开始运行"整颗不存在，勾了核对才凭空出现
  //（GUI 审查 C-29）。这两个按状态该出现就出现，不可用时灰着并说明原因；其余仍只在可用时出现。
  const shownWhenDisabled: Record<string, boolean> = {
    'study-run': permissions.run && !remotePermissions.exists,
    'study-resume': permissions.resume && !remotePermissions.exists,
  };
  const controls = lifecycleActions.filter(
    (a) => controlIds.includes(a.id) && (a.enabled || shownWhenDisabled[a.id]),
  );
  const runBlocked = controls.some((a) => shownWhenDisabled[a.id] && !a.enabled)
    ? !confirmed
      ? tr('先勾选上面的"我已核对……"才能运行。', 'Tick the review box above to run.')
      : !runKernel
        ? tr('没有可用的编译配置。', 'No build configuration is available.')
        : launching
          ? tr('研究正在运行。', 'The study is running.')
          : busy
            ? tr('上一个操作还没结束。', 'Another operation is still in progress.')
            : ''
    : '';
  const frozen = frozenDesign(frozenSpec);
  const byId = (id: string) => lifecycleActions.find((a) => a.id === id);
  const stepNames: [1 | 2 | 3 | 4, string][] = [
    [1, tr('选算例', 'Cases')],
    [2, ai ? tr('网络设计', 'Network') : tr('选参数', 'Parameters')],
    [3, tr('目标与预算', 'Targets and budget')],
    [4, tr('运行与结果', 'Run and results')],
  ];
  const canStep = (n: number) =>
    n === 1 ||
    (n === 2 && sites.length > 0 && !!kernel) ||
    (n === 3 && sites.length > 0 && !!kernel && (ai || chosenCount > 0)) ||
    (n === 4 && !!studyDir);
  // 步骤灰着时说清缺什么（GUI 审查 C-08），原来只有灰掉的按钮，要用户自己猜。
  const stepBlocked = (n: number) =>
    canStep(n)
      ? ''
      : !sites.length
        ? tr('先在第 1 步勾选算例。', 'Choose cases in step 1 first.')
        : !kernel
          ? spatialStudy
            ? tr(
                '没有格点用的编译配置：先到"计算资源"编译一个格点配置。',
                'No gridded build configuration: build one under Compute resources first.',
              )
            : tr(
                '没有单点用的编译配置：先到"计算资源"编译 default 配置。',
                'No single-point build configuration: build the default one under Compute resources first.',
              )
          : n === 3 && !ai && !chosenCount
            ? tr('先在第 2 步勾选至少一个参数。', 'Tick at least one parameter in step 2 first.')
            : n === 4
              ? tr(
                  '建好或打开一个研究后才有运行与结果。',
                  'Create or open a study to see its run and results.',
                )
              : '';
  const typeTable = !typeChoices
    ? []
    : typeChoices.kind === 'igbp'
      ? IGBP
      : typeChoices.kind === 'usgs'
        ? USGS
        : PFT.filter(([id]) => id >= 1 && id <= typeChoices.max);
  const typePicker = spatialStudy && (
    <div className="study-types">
      <h3 className="study-subhead">
        {typeChoices?.kind === 'pft' || typeChoices?.kind === 'pc'
          ? tr('要研究哪些植物类型', 'Plant types to study')
          : tr('要研究哪些地类', 'Land classes to study')}
      </h3>
      {!typeChoices ? (
        <p className="versions-empty">{tr('正在读取…', 'Reading…')}</p>
      ) : (
        <>
          <p className="muted study-hint">
            {typeChoices.present
              ? tr(
                  '只列出这些格点算例里都有的类型。勾上的类型各自出一组参数行；不勾只研究整个算例的系数。',
                  'Only types present in all these gridded cases are listed. Each ticked type adds its own parameter rows; with none ticked, only case-wide coefficients are studied.',
                )
              : tr(
                  '这些算例还没做过前处理，列出全部类型；区域里没有的类型，建研究时会报出来。',
                  'These cases have not been preprocessed yet, so every type is listed; a type absent from the domain is reported when the study is created.',
                )}
          </p>
          <div className="study-type-grid">
            {typeTable
              .filter(([id]) => !typeChoices.present || typeChoices.present.includes(id))
              .map(([id, zh, english]) => (
                <label key={id} className="param-check">
                  <input
                    type="checkbox"
                    checked={types.includes(id)}
                    disabled={busy || launching}
                    onChange={(e) =>
                      setTypes((old) =>
                        e.target.checked
                          ? [...new Set([...old, id])].sort((a, b) => a - b)
                          : old.filter((value) => value !== id),
                      )
                    }
                  />
                  {id} {en ? english : zh}
                </label>
              ))}
          </div>
        </>
      )}
    </div>
  );
  const parameterTable = (
    <>
      {typePicker}
      {state.researchSeed?.parameters.length && !contexts.length ? (
        <Notice>
          {tr(
            `带来了 ${state.researchSeed.parameters.length} 个参数，读出参数后会自动勾上。`,
            `${state.researchSeed.parameters.length} parameters were brought along and are ticked once loaded.`,
          )}
        </Notice>
      ) : null}
      {seedNotice && <Notice tone="success">{seedNotice}</Notice>}
      {!contexts.length ? (
        <p className="versions-empty">
          {busy
            ? tr('正在读取这些算例能研究的参数…', 'Reading the parameters these cases can study…')
            : tr('还没读到参数。', 'No parameters loaded yet.')}{' '}
          <ActionButton
            bare
            action="load-parameter-contexts"
            label={tr('读取适用参数', 'Load applicable parameters')}
            className="library-action"
            disabled={!designActions[0]!.enabled}
            run={() => designActions[0]!.run()}
          >
            {tr('重新读取', 'Load again')}
          </ActionButton>
        </p>
      ) : (
        <div className="table-scroll">
          <table className="versions-table study-parameters">
            <thead>
              <tr>
                <th />
                <th>{tr('参数', 'Parameter')}</th>
                <th>{tr('默认', 'Default')}</th>
                <th>{tr('下限', 'Min')}</th>
                <th>{tr('上限', 'Max')}</th>
                <th>{tr('尺度', 'Scale')}</th>
              </tr>
            </thead>
            <tbody>
              {contextOrder.map(({ row, i }, n) => {
                const label = en ? row.label_en : row.label_zh;
                // 上游新加、名称表里还没配名字的参数，界面名就是去掉前缀的键名，这时只显示一次键名。
                const unnamed = row.name.toUpperCase().endsWith(label.toUpperCase());
                const heading = n === 0 || groupKey(contextOrder[n - 1]!.row) !== groupKey(row);
                const p = parameters[String(i)] || {
                  enabled: false,
                  min: '',
                  max: '',
                  scale: row.scale || 'linear',
                };
                return (
                  <Fragment key={i}>
                    {heading && (
                      <tr className="study-group">
                        <th colSpan={6}>{scopeText(row)}</th>
                      </tr>
                    )}
                    <tr className={p.enabled ? 'is-on' : undefined}>
                      <td>
                        <input
                          type="checkbox"
                          aria-label={row.name}
                          checked={p.enabled}
                          disabled={busy || launching}
                          onChange={(e) => setParameter(i, row, { enabled: e.target.checked })}
                        />
                      </td>
                      <td>
                        {unnamed ? (
                          <code>{row.name}</code>
                        ) : (
                          <>
                            <strong>{label}</strong>
                            <code className="param-key">{row.name}</code>
                          </>
                        )}
                      </td>
                      <td className="muted">{Number(row.default.toPrecision(6))}</td>
                      {(['min', 'max'] as const).map((key) => (
                        <td key={key}>
                          <Input
                            className="param-input"
                            type="number"
                            value={p[key]}
                            placeholder={
                              key === 'min'
                                ? bound(row.min, row.min_inclusive, true)
                                : bound(row.max, row.max_inclusive, false)
                            }
                            disabled={!p.enabled || busy || launching}
                            onChange={(e) => setParameter(i, row, { [key]: e.target.value })}
                          />
                        </td>
                      ))}
                      <td>
                        <Select
                          value={p.scale}
                          disabled={!p.enabled || busy || launching}
                          onChange={(e) => setParameter(i, row, { scale: e.target.value })}
                        >
                          <option value="linear">{tr('线性', 'Linear')}</option>
                          <option value="log">{tr('对数', 'Log')}</option>
                        </Select>
                      </td>
                    </tr>
                  </Fragment>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      <p className="muted study-hint">
        {tr(`已选 ${chosenCount} 个参数。`, `${chosenCount} parameters chosen.`)}
        {/* 预填范围与"不替用户猜物理先验"的约定冲突过（GUI 审查 C-12）：保留预填，但写明它只是起点。 */}
        {chosenCount > 0 &&
          tr(
            '勾选时预填的范围是默认值的 0.5–1.5 倍（夹在参数允许的范围内），只是起点，不是物理先验，请按你掌握的文献或观测改。研究跑完若提示最优值贴着边界，就把那一侧放宽重跑。',
            ' The prefilled range is 0.5–1.5 × the default (kept inside the allowed bounds): a starting point, not a physical prior; adjust it to what the literature or observations support. If the study reports the best value on a bound, widen that side and rerun.',
          )}
      </p>
      {unranged.length > 0 && (
        <Notice tone="warning">
          {tr(
            `${unranged.join('、')} 没有能自动给出的范围，请填上下限，否则预检会被拒。`,
            `${unranged.join(', ')} ${unranged.length > 1 ? 'have' : 'has'} no automatic range; fill in the bounds or preflight will refuse it.`,
          )}
        </Notice>
      )}
    </>
  );
  const footer = (back: boolean, next?: { label: string; enabled: boolean; to: 1 | 2 | 3 | 4 }) => (
    <div className="study-footer">
      {back && (
        <ActionButton
          action="research-step-back"
          label={tr('上一步', 'Back')}
          variant="ghost"
          run={() => setStep((step - 1) as 1 | 2 | 3)}
        >
          ← {tr('上一步', 'Back')}
        </ActionButton>
      )}
      {next && (
        <ActionButton
          action="research-step-next"
          label={next.label}
          variant="primary"
          disabled={!next.enabled}
          run={() => setStep(next.to)}
        >
          {next.label} →
        </ActionButton>
      )}
      {next && !next.enabled && stepBlocked(next.to) && (
        <span className="muted study-hint">{stepBlocked(next.to)}</span>
      )}
    </div>
  );
  return (
    <div className="research-workbench study-workbench">
      {state.simulationReturn && (
        <div className="button-row">
          <ActionButton
            action="research-return-simulation"
            label={tr('← 返回模拟配置', '← Back to simulation setup')}
            run={returnToSimulation}
          >
            ← {tr('返回模拟配置', 'Back to simulation setup')}
          </ActionButton>
        </div>
      )}
      <header className="research-heading">
        <div>
          <h1>{en ? methodTitle.en : methodTitle.zh}</h1>
          <p>{en ? methodTitle.descriptionEn : methodTitle.descriptionZh}</p>
        </div>
        <div className="study-heading-actions">
          {renderActions([lifecycleActions[0]!])}
          {renderActions([backAction])}
        </div>
      </header>
      <p className="study-context">
        {state.researchTarget ? (
          <>
            {tr(
              `正在构建草稿"${state.researchTarget.name}"：研究用它冻结的修订运行，跑完把结果写回这个草稿，在版本页审阅后冻结。`,
              `Building draft "${state.researchTarget.name}": the study runs on its frozen revision and the results go back into the draft, which you review and freeze on the versions page.`,
            )}{' '}
            <ActionButton
              bare
              action="research-new-version"
              label={tr('改为另建新版本', 'Use a new version instead')}
              className="library-action"
              disabled={busy || launching}
              run={() => update({ researchTarget: null })}
            >
              {tr('改为另建新版本', 'Use a new version instead')}
            </ActionButton>
          </>
        ) : studyDir && envelope && !modelUsed ? (
          tr('这个研究没有记下所用的模型版本。', 'This study did not record its model version.')
        ) : (
          tr(
            `研究用的模型版本：${versionLabel(modelUsed ?? state.versionRef)}。跑完可以把结果写进以它为基础的新版本草稿，审阅后冻结。`,
            `Model version for the study: ${versionLabel(modelUsed ?? state.versionRef)}. Finished results can go into a new version draft based on it, which you review and freeze.`,
          )
        )}
      </p>
      <ol className="study-steps">
        {stepNames.map(([n, label]) => (
          <li key={n} className={n === step ? 'is-current' : n < step ? 'is-done' : undefined}>
            <ActionButton
              bare
              action={`research-step-${n}`}
              label={label}
              disabled={!canStep(n)}
              title={stepBlocked(n) || undefined}
              run={() => setStep(n)}
            >
              <span className="study-step-number">{n < step ? '✓' : n}</span>
              {label}
            </ActionButton>
          </li>
        ))}
      </ol>
      {error && <Notice tone="error">{shownError}</Notice>}

      {step === 1 && (
        <Card
          title={tr('选算例与运行位置', 'Cases and where to run')}
          description={tr(
            '研究在这些算例上反复运行模型。一次研究里只能是同一种算例：单点（可选多个站点，参数在各站点共用），或格点（流域、区域、全球）。',
            'The study runs the model repeatedly on these cases, all of one kind: single-point (several sites share the parameters) or gridded (basin, region or globe).',
          )}
        >
          {state.cases.length === 0 ? (
            <div className="versions-empty">
              <p>
                {tr(
                  '当前项目里还没有算例。先打开项目或新建模拟。',
                  'The current project has no cases yet. Open a project or create a simulation first.',
                )}
              </p>
              {noCasesActions}
            </div>
          ) : (
            <div className="study-cases">
              {state.cases.map((c) => {
                const on = sites.includes(c.dir);
                return (
                  <label key={c.dir} className={`study-case${on ? ' is-on' : ''}`}>
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={busy || launching}
                      onChange={(e) => {
                        sitesTouched.current = true;
                        // 一次研究里只有一种算例：勾上与已选种类不同的算例时，换成只选它。
                        setSites((old) => {
                          if (!e.target.checked) return old.filter((d) => d !== c.dir);
                          const same = old.filter(
                            (d) => !!state.cases.find((x) => x.dir === d)?.spatial === c.spatial,
                          );
                          return [...new Set([...same, c.dir])];
                        });
                      }}
                    />
                    <span>
                      <strong>{c.name}</strong>
                      <small>
                        {c.spatial
                          ? tr('格点（流域、区域或全球）', 'Gridded (basin, region or globe)')
                          : c.run_host
                            ? tr(
                                `单点 · 服务器 ${c.run_host}`,
                                `Single point · server ${c.run_host}`,
                              )
                            : tr('单点', 'Single point')}
                      </small>
                    </span>
                  </label>
                );
              })}
            </div>
          )}
          <div className="study-grid">
            <Field label={tr('在哪里运行', 'Run on')}>
              <Select
                value={host}
                disabled={busy || launching || remotePermissions.exists}
                onChange={(e) => setHost(e.target.value)}
              >
                <option value="">{tr('本机', 'This computer')}</option>
                {knownHosts.map((value) => (
                  <option key={value} value={value}>
                    {tr(`服务器 ${value}`, `Server ${value}`)}
                  </option>
                ))}
              </Select>
            </Field>
            {!ai && (
              <Field label={tr('计算引擎', 'Engine')}>
                <Select
                  value={engine}
                  disabled={busy || launching}
                  onChange={(e) => setEngine(e.target.value)}
                >
                  <option value="rust">Rust</option>
                  <option value="fortran">Fortran</option>
                </Select>
              </Field>
            )}
            <Field label={tr('编译配置', 'Build configuration')}>
              <Select
                value={kernel}
                disabled={busy || launching}
                onChange={(e) => setKernel(e.target.value)}
              >
                <option value="">{tr('请选择', 'Choose one')}</option>
                {studyKernels.map((k) => (
                  <option key={k.dir} value={k.dir}>
                    {k.preset}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          {kernelEntry && (
            <p className="muted study-hint">
              {kernelEntry.macros.join(' · ')} · {kernelEntry.platform}
            </p>
          )}
          {host && servers.includes(host) && (
            <LiveServerLoad host={host} partition={serverPartitions.current[host]} en={en} />
          )}
          {host && (
            <p className="muted study-hint">
              {tr(
                '服务器的登录、资源和数据路径用"计算资源"里的设置；提交后研究在服务器上独立运行，跑完取回结果。',
                'Server login, resources and data paths come from Compute resources; a submitted study runs on the server by itself and its results are fetched when done.',
              )}{' '}
              <ActionButton
                bare
                action="research-resources"
                label={tr('管理计算资源', 'Manage compute resources')}
                className="library-action"
                run={() => navigate('resources')}
              >
                {tr('计算资源 →', 'Compute resources →')}
              </ActionButton>
            </p>
          )}
          {footer(false, {
            label: ai
              ? tr('下一步：网络设计', 'Next: network')
              : tr('下一步：选参数', 'Next: parameters'),
            enabled: canStep(2),
            to: 2,
          })}
        </Card>
      )}

      {step === 2 && ai && (
        <Card
          title={tr('网络设计', 'Network design')}
          description={tr(
            'AI 网络根据站点特征给出参数（或替换一个过程），只支持 Rust 引擎。土壤水分胁迫 β 不适用于 PC 或开了植物水力的配置。',
            'The network turns site features into a parameter (or replaces a process); Rust engine only. Soil-stress β does not apply with PC or plant hydraulics.',
          )}
        >
          <div className="study-grid">
            <Field label={tr('网络作用在', 'Applies to')}>
              {tab === 'process' ? (
                // 替换过程目前只有这一项，一个选项的下拉框没得选。
                <p className="study-fixed">{tr('土壤水分胁迫 β', 'Soil-water stress β')}</p>
              ) : (
                <Select value={slot} onChange={(e) => chooseSlot(e.target.value)}>
                  <option value="pft">{tr('每个植物功能型', 'Each plant functional type')}</option>
                  <option value="land_class">{tr('每个地类', 'Each land class')}</option>
                </Select>
              )}
            </Field>
            <Field label={tr('输入特征（逗号分隔）', 'Input features (comma separated)')}>
              <Input value={features} onChange={(e) => setFeatures(e.target.value)} />
            </Field>
            <Field label={tr('输出参数', 'Output parameter')}>
              <Input value={output} onChange={(e) => setOutput(e.target.value)} />
            </Field>
            <Field label={tr('网络结构', 'Network')}>
              <Select value={hidden} onChange={(e) => setHidden(e.target.value)}>
                <option value="linear">{tr('线性', 'Linear')}</option>
                <option value="4">{tr('4 个隐藏单元', '4 hidden units')}</option>
                <option value="8">{tr('8 个隐藏单元', '8 hidden units')}</option>
              </Select>
            </Field>
            <Field label={tr('输出下限', 'Output minimum')}>
              <Input type="number" value={lower} onChange={(e) => setLower(e.target.value)} />
            </Field>
            <Field label={tr('输出上限', 'Output maximum')}>
              <Input type="number" value={upper} onChange={(e) => setUpper(e.target.value)} />
            </Field>
          </div>
          <label className="param-check">
            <input
              type="checkbox"
              checked={relative}
              onChange={(e) => setRelative(e.target.checked)}
            />
            {tr(
              '输出是对原参数值的倍数（相对值）',
              'The output multiplies the original value (relative)',
            )}
          </label>
          <div className="button-row">{renderActions(aiActions.slice(0, 3))}</div>
          {hybridInfo && (
            <Advanced title={tr('适用性检查结果', 'Applicability check')}>
              <pre className="overflow-auto">{JSON.stringify(hybridInfo, null, 2)}</pre>
            </Advanced>
          )}
          {result && (
            <Advanced title={tr('检查与气候特征输出', 'Check and climate-feature output')}>
              <pre className="overflow-auto">{result}</pre>
            </Advanced>
          )}
          <h3 className="study-subhead">
            {tr('同时率定的参数（可不选）', 'Parameters calibrated alongside (optional)')}
          </h3>
          {parameterTable}
          {footer(true, {
            label: tr('下一步：目标与预算', 'Next: targets and budget'),
            enabled: canStep(3),
            to: 3,
          })}
        </Card>
      )}

      {step === 2 && !ai && (
        <Card
          title={tr('选要研究的参数', 'Parameters to study')}
          description={tr(
            '下面是这些算例里能研究的参数。勾上要研究的，填取值范围；对数尺度适合跨好几个数量级的参数。',
            'These parameters can be studied on the chosen cases. Tick the ones to study and give their ranges; a log scale suits parameters spanning orders of magnitude.',
          )}
        >
          {parameterTable}
          {footer(true, {
            label: tr('下一步：目标与预算', 'Next: targets and budget'),
            enabled: canStep(3),
            to: 3,
          })}
        </Card>
      )}

      {step === 3 && (
        <Card
          title={
            tuning
              ? tr('目标与预算', 'Targets and budget')
              : tr('输出与抽样', 'Outputs and sampling')
          }
          description={
            tuning
              ? spatialStudy
                ? tr(
                    '率定按观测找参数：逐个加率定目标（格点产品、通量站或水文站），定好率定期（可再留一段验证期）和比较指标。',
                    'Calibration fits parameters to observations: add calibration targets (gridded product, flux station or river gauge), a calibration period (optionally a validation period) and a metric.',
                  )
                : tr(
                    '率定按观测找参数：每个站点选一个观测文件，定好率定期（可再留一段验证期）和比较指标。',
                    'Calibration fits parameters to observations: choose an observation file per site, a calibration period (optionally a validation period) and a metric.',
                  )
              : tr(
                  '不确定性分析在参数范围里抽样，看输出变化有多大。',
                  'Uncertainty analysis samples the parameter ranges and shows how much the outputs vary.',
                )
          }
        >
          {ai && (
            <>
              <label className="param-check">
                <input
                  type="checkbox"
                  checked={trainNetwork || tab === 'process'}
                  disabled={tab === 'process'}
                  onChange={(e) => setTrainNetwork(e.target.checked)}
                />
                {tr(
                  '这次研究用差分进化训练上一步设计的网络',
                  'Train the network designed in the previous step with differential evolution',
                )}
              </label>
              {tab === 'process' ? (
                <p className="muted study-hint">
                  {tr(
                    '替换过程一定要训练网络，这一项不能关。',
                    'Process replacement always trains the network.',
                  )}
                </p>
              ) : (
                !trainNetwork && (
                  <p className="muted study-hint">
                    {tr(
                      '不训练网络时，这个研究只是普通的参数率定，上一步的网络设计不会用上。',
                      'Without training, this is a plain parameter calibration and the network design is not used.',
                    )}
                  </p>
                )
              )}
            </>
          )}
          {!(tuning && spatialStudy) && (
            <Field
              label={
                tuning
                  ? tr(
                      '目标变量（逗号分隔，如 Qle, Qh）',
                      'Target variables (comma separated, e.g. Qle, Qh)',
                    )
                  : tr(
                      '输出变量（模型 history 变量名，逗号分隔，如 f_lfevpa, f_fsena）',
                      'Output variables (model history names, comma separated, e.g. f_lfevpa, f_fsena)',
                    )
              }
            >
              <Input
                value={tuning ? variables : outputs}
                onChange={(e) => (tuning ? setVariables : setOutputs)(e.target.value)}
              />
            </Field>
          )}
          {!tuning && (
            <div className="study-grid">
              {windowHint}
              <Field label={tr('分析时段开始（UTC，可空）', 'Analysis start (UTC, optional)')}>
                <Input
                  type="datetime-local"
                  value={from}
                  onChange={(e) => setFrom(e.target.value)}
                />
              </Field>
              <Field label={tr('分析时段结束（UTC，可空）', 'Analysis end (UTC, optional)')}>
                <Input type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} />
              </Field>
            </div>
          )}
          {!tuning && (
            <label className="param-check">
              <input
                type="checkbox"
                checked={scoreObjective}
                disabled={busy || launching}
                onChange={(e) => setScoreObjective(e.target.checked)}
              />
              {tr(
                '同时对照观测算目标函数，敏感性按"调这个参数能不能让拟合变好"排（推荐；排出来的参数可以直接带去率定）',
                'Also score each sample against observations, so sensitivity ranks parameters by whether they improve the fit (recommended; the ranking can be carried into calibration)',
              )}
            </label>
          )}
          {!tuning && scoreObjective && !spatialStudy && (
            <Field
              label={tr(
                '对照的观测变量（逗号分隔，如 Qle, Qh）',
                'Observed variables to score (comma separated, e.g. Qle, Qh)',
              )}
            >
              <Input value={variables} onChange={(e) => setVariables(e.target.value)} />
            </Field>
          )}
          {scoring && !spatialStudy && targetChoices && (
            <div className="study-type-grid">
              {targetChoices.length ? (
                targetChoices.map((row) => {
                  const chosen = variableNames.includes(row.name);
                  return (
                    <label key={row.name} className="param-check">
                      <input
                        type="checkbox"
                        checked={chosen}
                        disabled={busy || launching}
                        onChange={(e) =>
                          setVariables(
                            (e.target.checked
                              ? [...variableNames, row.name]
                              : variableNames.filter((name) => name !== row.name)
                            ).join(', '),
                          )
                        }
                      />
                      {en ? row.label_en : row.label_zh} · {row.name}
                      {row.units ? ` (${row.units})` : ''}
                    </label>
                  );
                })
              ) : (
                <p className="muted study-hint">
                  {tr(
                    '这些站的观测和模型输出没有共同的可比变量。',
                    'The observations and model output of these sites share no comparable variable.',
                  )}
                </p>
              )}
            </div>
          )}
          {scoring && !spatialStudy && targetChoices && unknownTargets.length > 0 && (
            <Notice tone="warning">
              {tr(
                `${unknownTargets.join('、')} 不是所有站都能比的变量，预检会拒。`,
                `${unknownTargets.join(', ')} cannot be compared at every site; preflight will refuse it.`,
              )}
            </Notice>
          )}
          {!tuning && spatialStudy && (
            <p className="muted study-hint">
              {tr(
                '格点算例的输出取全区按陆地面积加权的平均（每个时刻一个数）。',
                'For gridded cases each output is the land-area-weighted mean over the domain (one value per time).',
              )}
            </p>
          )}
          {scoring && (
            <>
              {spatialStudy ? (
                gridTargetEditor
              ) : (
                <>
                  <h3 className="study-subhead">{tr('观测文件', 'Observation files')}</h3>
                  <div className="study-observations">
                    {sites.map((d) => {
                      const name = siteName(d);
                      return (
                        <div key={d} className="study-observation">
                          <strong>{name}</strong>
                          <Input
                            value={observationOf[name] ?? ''}
                            placeholder={tr(
                              '观测文件的绝对路径（*_Flux.nc）',
                              'Absolute path of the observation file (*_Flux.nc)',
                            )}
                            onChange={(e) => setObservation(name, e.target.value)}
                          />
                          <ActionButton
                            action={`study-observation-pick:${d}`}
                            label={tr('选择观测文件', 'Choose observation file')}
                            run={() => pickObservation(name)}
                          >
                            {tr('选择…', 'Choose…')}
                          </ActionButton>
                        </div>
                      );
                    })}
                  </div>
                </>
              )}
              <div className="study-grid">
                {/* 不确定性分析的时段在上面"分析时段"里，输出统计和目标函数共用；留出验证是率定的事。 */}
                {tuning && (
                  <>
                    {windowHint}
                    <Field label={tr('率定期开始（UTC）', 'Calibration start (UTC)')}>
                      <Input
                        type="datetime-local"
                        value={from}
                        onChange={(e) => setFrom(e.target.value)}
                      />
                    </Field>
                    <Field label={tr('率定期结束（UTC）', 'Calibration end (UTC)')}>
                      <Input
                        type="datetime-local"
                        value={to}
                        onChange={(e) => setTo(e.target.value)}
                      />
                    </Field>
                    <Field label={tr('验证期开始（可空）', 'Validation start (optional)')}>
                      <Input
                        type="datetime-local"
                        value={valFrom}
                        onChange={(e) => setValFrom(e.target.value)}
                      />
                    </Field>
                    <Field label={tr('验证期结束（可空）', 'Validation end (optional)')}>
                      <Input
                        type="datetime-local"
                        value={valTo}
                        onChange={(e) => setValTo(e.target.value)}
                      />
                    </Field>
                  </>
                )}
                <Field
                  label={
                    spatialStudy
                      ? tr('总体比较指标（各目标可另选）', 'Study metric (targets may override)')
                      : tr('比较指标', 'Metric')
                  }
                >
                  <Select value={metric} onChange={(e) => setMetric(e.target.value)}>
                    {Object.entries(metricNames).map(([value, [zh, english]]) => (
                      <option key={value} value={value}>
                        {en ? english : zh}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field
                  label={
                    spatialStudy
                      ? tr('每个目标至少多少对观测', 'Minimum observation pairs per target')
                      : tr('每个站点至少多少对观测', 'Minimum observation pairs per site')
                  }
                >
                  <Input
                    type="number"
                    value={minPairs}
                    onChange={(e) => setMinPairs(e.target.value)}
                  />
                </Field>
                {/* 只有 Qle/Qh 有订正版：目标里没有它们时这一项是空操作，不显示（GUI 审查 A-03）。 */}
                {(spatialStudy
                  ? gridTargets.some((t) => t.kind === 'station' && fluxCorrected(t.variable))
                  : variableNames.some(fluxCorrected)) && (
                  <label className="param-check study-wide">
                    <input
                      type="checkbox"
                      checked={corrected}
                      onChange={(e) => setCorrected(e.target.checked)}
                    />
                    {tr(
                      '潜热、感热（Qle、Qh）用能量闭合订正后的通量站观测。通量站测到的湍流通量通常比可用能量少一截，拿未订正的率定会把模型的湍流通量压低；观测文件里要有 Qle_cor、Qh_cor。',
                      'Compare latent and sensible heat (Qle, Qh) with energy-balance-corrected flux-tower observations. Measured turbulent fluxes usually fall short of the available energy, so calibrating against uncorrected ones pulls the model low; the observation file needs Qle_cor and Qh_cor.',
                    )}
                  </label>
                )}
                <Field label={tr('比较的时间尺度', 'Time scale of the comparison')}>
                  <Select
                    value={aggregate}
                    onChange={(e) => setAggregate(e.target.value as '' | 'daily' | 'monthly')}
                  >
                    <option value="">{tr('观测原来的分辨率', 'Observation resolution')}</option>
                    <option value="daily">
                      {tr('逐日平均（推荐）', 'Daily means (recommended)')}
                    </option>
                    <option value="monthly">{tr('逐月平均', 'Monthly means')}</option>
                  </Select>
                </Field>
                <p className="muted study-hint study-wide">
                  {tr(
                    '半小时尺度的误差主要来自日变化，看不出季节和水分限制；逐日平均更接近率定想调的东西。模型和观测在同一批有效时刻上平均，有效时刻不到一半的那天不算。"最少观测配对数"相应变成最少天数（或月数）。',
                    'Half-hourly errors are dominated by the diurnal cycle and hide seasonal and water limitation; daily means are closer to what calibration should tune. Model and observations are averaged over the same valid steps, and days with less than half their steps are dropped. The minimum pairs then counts days (or months).',
                  )}
                </p>
                {!spatialStudy && variableNames.length > 1 && (
                  <div className="study-wide">
                    <p className="muted study-hint">
                      {tr(
                        '几个变量的权重（各自先按指标归一化，再按权重合并；没填的按 1）：',
                        'Weights of the variables (each is normalised by its metric first, then combined by weight; blank means 1):',
                      )}
                    </p>
                    <div className="study-grid">
                      {variableNames.map((name) => (
                        <Field key={name} label={name}>
                          <Input
                            type="number"
                            value={weights[name] ?? ''}
                            placeholder="1"
                            onChange={(e) =>
                              setWeights((old) => ({ ...old, [name]: e.target.value }))
                            }
                          />
                        </Field>
                      ))}
                    </div>
                  </div>
                )}
                {tuning && (
                  <>
                    <Field
                      label={tr(
                        '种群大小（每代多少组参数）',
                        'Population (parameter sets per generation)',
                      )}
                    >
                      <Input
                        type="number"
                        value={population}
                        onChange={(e) => editPopulation(e.target.value)}
                      />
                    </Field>
                    <Field label={tr('迭代代数', 'Generations')}>
                      <Input
                        type="number"
                        value={generations}
                        onChange={(e) => setGenerations(e.target.value)}
                      />
                    </Field>
                  </>
                )}
              </div>
            </>
          )}
          {!tuning && (
            <div className="study-grid">
              <Field label={tr('抽样方法', 'Sampling')}>
                <Select value={method} onChange={(e) => chooseSampling(e.target.value)}>
                  <option value="lhs">
                    {tr('拉丁超立方（整体不确定性）', 'Latin hypercube (overall uncertainty)')}
                  </option>
                  <option value="oat">
                    {tr('一次改一个（逐个看敏感性）', 'One at a time (per-parameter sensitivity)')}
                  </option>
                  <option value="morris">
                    {tr(
                      'Morris 筛选（挑出值得率定的参数）',
                      'Morris screening (find the parameters worth calibrating)',
                    )}
                  </option>
                </Select>
              </Field>
              {method === 'lhs' && (
                <Field label={tr('样本数', 'Samples')}>
                  <Input type="number" value={count} onChange={(e) => editCount(e.target.value)} />
                </Field>
              )}
              {method === 'morris' && (
                <Field label={tr('轨迹数（常用 10–20）', 'Trajectories (10–20 is typical)')}>
                  <Input type="number" value={count} onChange={(e) => editCount(e.target.value)} />
                </Field>
              )}
              {method === 'morris' && (
                <p className="muted study-hint study-wide">
                  {tr(
                    'Morris 每条轨迹从一个随机点出发，每步只改一个参数（跨抽样范围的 2/3），共跑 轨迹数 ×（参数数 + 1）次。比一次改一个多看到非线性和参数间的交互，比拉丁超立方更省，适合在率定前先挑参数。',
                    'Each Morris trajectory starts at a random point and changes one parameter per step (by 2/3 of its range): trajectories × (parameters + 1) runs. It sees non-linearity and interactions that one-at-a-time misses, at less cost than Latin hypercube, so it suits picking parameters before calibration.',
                  )}
                </p>
              )}
            </div>
          )}
          <Advanced title={tr('更多设置', 'More settings')}>
            <div className="study-grid">
              <Field label={tr('同时运行几个模拟（1–16）', 'Simulations at a time (1–16)')}>
                <Input type="number" value={jobs} onChange={(e) => editJobs(e.target.value)} />
              </Field>
              <Field label={tr('随机种子', 'Random seed')}>
                <Input type="number" value={seed} onChange={(e) => setSeed(e.target.value)} />
              </Field>
              {tuning && (
                <Field
                  label={tr(
                    '连续几代没有改善就停（0 = 跑满全部代数）',
                    'Stop after generations without improvement (0 = run all)',
                  )}
                >
                  <Input
                    type="number"
                    value={patience}
                    onChange={(e) => setPatience(e.target.value)}
                  />
                </Field>
              )}
              {host && (
                <Field label={tr('服务器前处理引擎', 'Server preprocessors')}>
                  <Select value={preprocessors} onChange={(e) => setPreprocessors(e.target.value)}>
                    <option value="rust">Rust</option>
                    <option value="fortran">Fortran</option>
                  </Select>
                </Field>
              )}
            </div>
          </Advanced>
          <div className={`study-budget${budgetOk ? '' : ' is-bad'}`}>
            <span>{tr('预算', 'Budget')}</span>
            <strong>{budgetSummary}</strong>
          </div>
          {tuning &&
            budgetOk &&
            budgetAdvice(dimensionCount, Number(population), Number(generations)).map(
              ([zh, english]) => (
                <p key={zh} className="muted study-hint">
                  {tr(zh, english)}
                </p>
              ),
            )}
          <label className="param-check study-confirm">
            <input
              type="checkbox"
              checked={confirmed}
              disabled={busy || launching}
              onChange={(e) => setConfirmed(e.target.checked)}
            />
            {tr(
              '我已核对参数范围、预算和适用范围（研究会真的运行这么多次模拟）',
              'I reviewed the ranges, budget and applicability (the study really runs this many simulations)',
            )}
          </label>
          {preflight && (
            <>
              <Notice tone="success">
                {tr(
                  '预检通过：算例、参数范围和观测都能用，可以创建研究。',
                  'Preflight passed: the cases, ranges and observations are usable; the study can be created.',
                )}
              </Notice>
              <Advanced title={tr('预检详情', 'Preflight details')}>
                <pre className="overflow-auto">{preflight}</pre>
              </Advanced>
            </>
          )}
          {!confirmed && (
            <p className="muted study-hint">
              {tr(
                '预检随时可以做；勾上上面的核对项后才能创建研究。',
                'Preflight works any time; tick the review box above to create the study.',
              )}
            </p>
          )}
          <div className="study-footer">
            <ActionButton
              action="research-step-back"
              label={tr('上一步', 'Back')}
              variant="ghost"
              run={() => setStep(2)}
            >
              ← {tr('上一步', 'Back')}
            </ActionButton>
            <ActionButton
              action="study-preflight"
              label={tr('预检', 'Preflight')}
              disabled={!designActions[1]!.enabled}
              run={() => designActions[1]!.run()}
            >
              {tr('预检', 'Preflight')}
            </ActionButton>
            <ActionButton
              action="study-create"
              label={tr('创建研究任务', 'Create study')}
              kind="commit"
              variant="primary"
              disabled={!designActions[2]!.enabled}
              run={() => designActions[2]!.run()}
            >
              {tr('创建研究 →', 'Create study →')}
            </ActionButton>
          </div>
        </Card>
      )}

      {step === 4 && (
        <>
          <Card className="study-status-card">
            <div className="study-status-head">
              <div>
                <h2>
                  {studyDir
                    ? envelope?.manifest?.id || siteName(studyDir)
                    : tr('还没有研究', 'No study yet')}
                </h2>
                <p className="muted">
                  {modelUsed
                    ? tr(`模型版本 ${versionLabel(modelUsed)}`, `Model ${versionLabel(modelUsed)}`)
                    : ''}
                  {remoteIdentity?.host || host
                    ? ` · ${tr('服务器', 'server')} ${remoteIdentity?.host || host}`
                    : ` · ${tr('本机', 'this computer')}`}
                </p>
              </div>
              <span className={`badge study-state is-${currentState.toLowerCase()}`}>
                {statusText(currentState)}
                {launching ? tr(' · 正在执行', ' · working') : ''}
              </span>
            </div>
            {(() => {
              const progress = studyProgress(frozenSpec, completed ?? 0, currentState);
              return (
                <div className="study-progress">
                  <div className="study-progress-bar">
                    <span style={{ width: `${progress.percent}%` }} />
                  </div>
                  <span>
                    {tr(progress.zh, progress.en)}
                    {best ? tr(` · 当前最优 ${best}`, ` · best so far ${best}`) : ''}
                  </span>
                  {progress.note && (
                    <p className="muted study-hint">{tr(progress.note[0], progress.note[1])}</p>
                  )}
                </div>
              );
            })()}
            {frozenSpec && (
              <p className="muted study-hint">
                {tr('冻结的设定：', 'Frozen design: ')}
                {[
                  frozenSpec.kind === 'tuning'
                    ? tr('率定', 'calibration')
                    : tr('不确定性分析', 'uncertainty'),
                  Array.isArray(frozenSpec.parameters) &&
                    tr(
                      `${frozenSpec.parameters.length} 个参数`,
                      `${frozenSpec.parameters.length} parameter${frozenSpec.parameters.length === 1 ? '' : 's'}`,
                    ),
                  Array.isArray(frozenSpec.base_cases) &&
                    (frozenSpatial
                      ? tr(
                          `${frozenSpec.base_cases.length} 个格点算例`,
                          `${frozenSpec.base_cases.length} gridded case${frozenSpec.base_cases.length === 1 ? '' : 's'}`,
                        )
                      : tr(
                          `${frozenSpec.base_cases.length} 个站点`,
                          `${frozenSpec.base_cases.length} site${frozenSpec.base_cases.length === 1 ? '' : 's'}`,
                        )),
                  Array.isArray(frozenSpec.targets) &&
                    tr(
                      `${frozenSpec.targets.length} 个目标`,
                      `${frozenSpec.targets.length} target${frozenSpec.targets.length === 1 ? '' : 's'}`,
                    ),
                ]
                  .filter(Boolean)
                  .join(' · ')}
                {tr('。改设计要新建研究。', '. Create a new study to change the design.')}
              </p>
            )}
            {frozen.parameters.length > 0 && (
              <div className="table-scroll">
                <table className="versions-table">
                  <thead>
                    <tr>
                      <th>{tr('参数', 'Parameter')}</th>
                      <th>{tr('抽样范围', 'Sampling range')}</th>
                      <th>{tr('尺度', 'Scale')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {frozen.parameters.map((p, i) => (
                      <tr key={i}>
                        <td>
                          <code>{p.name}</code>
                        </td>
                        <td>{p.range}</td>
                        <td>{p.log ? tr('对数', 'log') : tr('线性', 'linear')}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
            {envelope?.run_options && (
              <p className="muted study-hint">
                {tr(
                  `运行用 ${envelope.run_options.engine === 'fortran' ? 'Fortran' : 'Rust'} 模型、${envelope.run_options.preprocessors === 'fortran' ? 'Fortran' : 'Rust'} 前处理（第一次运行时定下，之后不变）。`,
                  `Runs with the ${envelope.run_options.engine === 'fortran' ? 'Fortran' : 'Rust'} model and ${envelope.run_options.preprocessors === 'fortran' ? 'Fortran' : 'Rust'} preprocessors (fixed at the first run).`,
                )}
              </p>
            )}
            {frozen.budget.length > 0 && (
              <p className="muted study-hint">
                {frozen.budget.map(([zh, english]) => tr(zh, english)).join(en ? '; ' : '；')}
              </p>
            )}
            {/* 打开的已有研究还没核对过：原来勾选框只在第 3 步，这一步的"开始运行""恢复"一直不出现。 */}
            {!confirmed && !remotePermissions.exists && (permissions.run || permissions.resume) && (
              <label className="param-check study-confirm">
                <input
                  type="checkbox"
                  checked={confirmed}
                  disabled={busy || launching}
                  onChange={(e) => setConfirmed(e.target.checked)}
                />
                {tr(
                  '我已核对这个研究冻结的参数范围和预算（运行会真的跑这么多次模拟）',
                  "I reviewed this study's frozen ranges and budget (running really performs this many simulations)",
                )}
              </label>
            )}
            <div className="button-row">
              {renderActions(controls)}
              {renderActions([byId('study-status')!].filter((a) => a.enabled))}
            </div>
            {runBlocked && <p className="muted study-hint">{runBlocked}</p>}
            {remotePermissions.exists && (
              <div className="study-remote">
                <p className="muted">
                  {tr('作业', 'Job')} {remoteIdentity?.job || '—'} · {remoteReply?.status?.phase}
                </p>
                {envelope?.remote_pending && (
                  <Notice tone="warning">
                    {tr(
                      '提交结果待核对：已保留提交意图。核对保存的服务器作业；不要再次提交。',
                      'Submission is uncertain and its intent is recorded. Check the recorded server job; do not submit it again.',
                    )}
                  </Notice>
                )}
                {remoteReply?.outcome === 'cancel_requested' && !remotePermissions.terminal && (
                  <Notice>
                    {tr(
                      '取消请求已受理，等服务器真正退出。',
                      'Cancellation accepted; waiting for the server to exit.',
                    )}
                  </Notice>
                )}
                {(remoteReply?.status?.state === 'lost' ||
                  remoteReply?.status?.state === 'unknown' ||
                  remoteReply?.outcome === 'uncertain') && (
                  <Notice tone="warning">
                    {tr(
                      '服务器状态还没核对，不能当作完成、失败或已取消，也不会重复提交。',
                      'The server state is unconfirmed; it is not treated as finished, failed or cancelled, and will not be resubmitted.',
                    )}
                  </Notice>
                )}
                {remotePermissions.fetch && (
                  <Notice>
                    {tr(
                      `服务器已退出（退出码 ${remoteReply?.status?.exit_code ?? '—'}）。取回时会核对研究身份和各成员，再读结果。`,
                      `The server exited (code ${remoteReply?.status?.exit_code ?? '—'}). Fetching checks the study identity and members before reading results.`,
                    )}
                  </Notice>
                )}
                {remotePermissions.fetched && (
                  <Notice
                    tone={remotePermissions.scientificVerified === true ? 'success' : 'warning'}
                  >
                    {remotePermissions.scientificVerified === true
                      ? tr('服务器结果已取回并核对。', 'Server results are fetched and verified.')
                      : tr(
                          '服务器诊断结果已取回，但没有通过完整核对，不能据此声称模型改进。',
                          'Server diagnostics are fetched but not fully verified; they do not establish an improvement.',
                        )}
                  </Notice>
                )}
                {envelope?.remote?.verification_error && (
                  <Notice tone="warning">{envelope.remote.verification_error}</Notice>
                )}
                {remoteReply?.warnings.map((warning, i) => (
                  <Notice tone="warning" key={i}>
                    {warning}
                  </Notice>
                ))}
                {remoteReply?.status?.log_tail && (
                  <pre className="run-log">{remoteReply.status.log_tail}</pre>
                )}
              </div>
            )}
          </Card>
          {/* 诊断原来放在成绩表卡片里，没有成绩（基准失败、不确定性分析）时整块不显示，
              最该看的提示反而看不到（GUI 审查 B-03）。 */}
          {studyDir && responses.length > 0 && (
            <Card
              title={tr('参数敏感性', 'Parameter sensitivity')}
              description={
                rankingMethod === 'morris'
                  ? tr(
                      '按 Morris μ*（参数在抽样范围内变化时响应量的平均绝对变化）排序；σ 大说明影响随别的参数或参数本身的取值而变（非线性、交互）。',
                      'Ranked by Morris μ* (mean absolute change of the response across the sampling range); a large σ means the effect depends on other parameters or on the value itself (non-linearity, interaction).',
                    )
                  : rankingMethod === 'spearman'
                    ? tr(
                        '按 |Spearman ρ| 排序：样本里参数和响应量的单调相关有多强。',
                        'Ranked by |Spearman ρ|: how strongly the response follows the parameter across the samples.',
                      )
                    : tr(
                        '按参数从下限扫到上限时响应量的变化排序（其余参数取默认值）。',
                        'Ranked by the change of the response when the parameter sweeps its range (others at default).',
                      )
              }
            >
              <div className="study-grid">
                <Field label={tr('看哪个响应量', 'Response')}>
                  <Select
                    value={sensitivityKey}
                    onChange={(e) => setSensitivityKey(e.target.value)}
                  >
                    {responses.map((r) => (
                      <option key={r.key} value={r.key}>
                        {r.variable === 'objective'
                          ? tr('目标函数（与观测的差距）', 'Objective (misfit to observations)')
                          : r.key}
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
              {!responses.some((r) => r.variable === 'objective') && (
                <p className="muted study-hint">
                  {tr(
                    '这个研究没有对照观测，只能看参数对输出的影响；要按拟合好坏挑参数，新建研究时勾上"对照观测算目标函数"。',
                    'This study was not scored against observations, so it only shows effects on outputs; to pick parameters by fit, tick scoring against observations in a new study.',
                  )}
                </p>
              )}
              <div className="table-scroll">
                <table className="versions-table">
                  <thead>
                    <tr>
                      <th>{tr('带去率定', 'Carry')}</th>
                      <th>{tr('参数', 'Parameter')}</th>
                      <th>{tr('影响', 'Impact')}</th>
                      <th>{tr('增大参数时', 'When increased')}</th>
                      {rankingMethod === 'morris' && <th>σ</th>}
                      <th>n</th>
                    </tr>
                  </thead>
                  <tbody>
                    {ranking.map((row) => (
                      <tr key={row.parameter}>
                        <td>
                          <input
                            type="checkbox"
                            aria-label={row.parameter}
                            checked={carryKeys.includes(row.parameter)}
                            onChange={(e) =>
                              setCarryKeys((old) =>
                                e.target.checked
                                  ? [...old, row.parameter]
                                  : old.filter((key) => key !== row.parameter),
                              )
                            }
                          />
                        </td>
                        <td>
                          <code>{row.parameter}</code>
                        </td>
                        <td>{row.impact == null ? '—' : Number(row.impact.toPrecision(3))}</td>
                        <td>
                          {row.direction === 0
                            ? '—'
                            : objectiveView
                              ? row.direction < 0
                                ? tr('拟合变好', 'fit improves')
                                : tr('拟合变差', 'fit worsens')
                              : row.direction > 0
                                ? tr('变大', 'increases')
                                : tr('变小', 'decreases')}
                        </td>
                        {rankingMethod === 'morris' && (
                          <td>{row.sigma == null ? '—' : Number(row.sigma.toPrecision(3))}</td>
                        )}
                        <td>{row.n}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {objectiveView && rankingMethod === 'morris' && (
                <p className="muted study-hint">
                  {tr(
                    '"拟合变好/变差"取的是各条轨迹效应的平均方向；σ 和 μ* 相当时方向不稳定，率定时保留整个范围。',
                    'Fit direction is the mean sign over trajectories; when σ is comparable to μ* the direction is unstable, so keep the whole range when calibrating.',
                  )}
                </p>
              )}
              <div className="button-row">{renderActions([carryAction])}</div>
              <p className="muted study-hint">
                {tr(
                  '带入率定会换到"参数率定"，用同样的算例、观测和指标，并勾上选中的参数（范围用这个研究的）。',
                  'Carrying switches to calibration with the same cases, observations and metric, and ticks the chosen parameters with this study’s ranges.',
                )}
              </p>
            </Card>
          )}
          {studyDir &&
            frozenSpec?.kind === 'tuning' &&
            (convergence.length > 1 || near.count > 0 || folds.length > 0) && (
              <Card
                title={tr('收敛与可信度', 'Convergence and confidence')}
                description={tr(
                  '看这次率定是不是已经收敛、找到的参数能被观测定到多准。',
                  'Whether the calibration has converged and how tightly the observations pin the parameters.',
                )}
              >
                {convergence.length > 1 && (
                  <div className="table-scroll">
                    <table className="versions-table">
                      <thead>
                        <tr>
                          <th>{tr('代', 'Generation')}</th>
                          <th>{tr('这一代最好', 'Best in generation')}</th>
                          <th>{tr('到此为止最好', 'Best so far')}</th>
                        </tr>
                      </thead>
                      <tbody>
                        {convergence.map((row) => (
                          <tr key={row.generation}>
                            <td>{row.generation}</td>
                            <td>{Number(row.best.toPrecision(4))}</td>
                            <td>{Number(row.bestSoFar.toPrecision(4))}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )}
                {convergence.length > 2 &&
                  convergence.at(-1)!.bestSoFar < convergence.at(-2)!.bestSoFar && (
                    <p className="muted study-hint">
                      {tr(
                        '最后一代还在变好：再加几代可能还能改善。',
                        'The last generation still improved: more generations may help.',
                      )}
                    </p>
                  )}
                {near.count > 0 && (
                  <>
                    <p className="muted study-hint">
                      {tr(
                        `目标函数在最优值 ${Math.round(near.margin * 100)}% 以内的有 ${near.count} 组参数。各参数在这些组里取值的范围：`,
                        `${near.count} parameter sets score within ${Math.round(near.margin * 100)}% of the best. The range each parameter takes across them:`,
                      )}
                    </p>
                    <div className="table-scroll">
                      <table className="versions-table">
                        <thead>
                          <tr>
                            <th>{tr('参数', 'Parameter')}</th>
                            <th>{tr('近优范围', 'Near-optimal range')}</th>
                            <th>{tr('占抽样范围', 'Share of sampling range')}</th>
                            <th>{tr('判断', 'Reading')}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {near.parameters.map((row) => (
                            <tr key={row.parameter}>
                              <td>
                                <code>{row.parameter}</code>
                              </td>
                              <td>
                                {row.min == null || row.max == null
                                  ? '—'
                                  : `${Number(row.min.toPrecision(4))} – ${Number(row.max.toPrecision(4))}`}
                              </td>
                              <td>{Math.round(row.spread * 100)}%</td>
                              <td>
                                {row.pinned === 'tight'
                                  ? tr('定得住', 'well pinned')
                                  : row.pinned === 'loose'
                                    ? tr('大致定得住', 'loosely pinned')
                                    : tr(
                                        '定不住：这份观测分不出它的取值',
                                        'not pinned: these observations cannot tell its value',
                                      )}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </>
                )}
                {folds.length > 0 && (
                  <>
                    <p className="muted study-hint">
                      {tr(
                        '留一站交叉验证：每次拿掉一个站，只按其余站挑最优参数，再看它在拿掉的站上是否仍比默认参数好。',
                        'Leave-one-site-out: each time one site is left out, the best set is picked on the others and scored at the left-out site against the defaults.',
                      )}
                    </p>
                    <div className="table-scroll">
                      <table className="versions-table">
                        <thead>
                          <tr>
                            <th>{tr('拿掉的站', 'Left-out site')}</th>
                            <th>{tr('挑出的成员', 'Chosen member')}</th>
                            <th>{tr('在该站：挑出的 / 默认', 'There: chosen / default')}</th>
                            <th>{tr('判断', 'Reading')}</th>
                          </tr>
                        </thead>
                        <tbody>
                          {folds.map((fold) => (
                            <tr key={fold.site}>
                              <td>{fold.site}</td>
                              <td>
                                <code>{fold.chosen}</code>
                              </td>
                              <td>
                                {fold.heldOut == null ? '—' : Number(fold.heldOut.toPrecision(4))}
                                {' / '}
                                {fold.baseline == null ? '—' : Number(fold.baseline.toPrecision(4))}
                              </td>
                              <td>
                                {fold.better == null
                                  ? tr('算不出', 'unavailable')
                                  : fold.better
                                    ? tr('换到这个站仍然更好', 'still better at this site')
                                    : tr(
                                        '换到这个站反而更差：参数在拟合别的站',
                                        'worse at this site: the parameters fit the other sites',
                                      )}
                              </td>
                            </tr>
                          ))}
                        </tbody>
                      </table>
                    </div>
                  </>
                )}
              </Card>
            )}
          {studyDir && diagnostics.length > 0 && (
            <Card title={tr('诊断与提示', 'Diagnostics')}>
              {diagnostics.map(([zh, english], i) => (
                <p key={i} className="muted study-hint">
                  {tr(zh, english)}
                </p>
              ))}
            </Card>
          )}
          {ranked.some(([, c]) => c.calibration != null) && (
            <Card
              title={tr('各组参数的成绩', 'Scores of each parameter set')}
              description={tr(
                '按率定期成绩排序（目标函数越小越好）；验证期成绩用来看是否过拟合。',
                'Sorted by the calibration score (lower objective is better); the validation score shows over-fitting.',
              )}
            >
              <div className="table-scroll study-ranking">
                <table className="versions-table">
                  <thead>
                    <tr>
                      <th>{tr('成员', 'Member')}</th>
                      <th>{tr('率定期', 'Calibration')}</th>
                      <th>{tr('验证期', 'Validation')}</th>
                      <th>{tr('说明', 'Note')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {ranked.map(([key, c]) => (
                      <tr key={key} className={key === best ? 'is-best' : undefined}>
                        <td>
                          <code>{key}</code>
                          {/* sample::design 总把基准放在第一个成员 m000000。 */}
                          {key === 'm000000' && (
                            <span className="badge versions-badge">{tr('基准', 'Baseline')}</span>
                          )}
                          {key === best && (
                            <span className="badge versions-badge is-used">
                              {tr('最优', 'Best')}
                            </span>
                          )}
                        </td>
                        <td>
                          {c.calibration == null ? '—' : Number(c.calibration.toPrecision(4))}
                        </td>
                        <td>{c.validation == null ? '—' : Number(c.validation.toPrecision(4))}</td>
                        <td className="muted">
                          {c.feasible === false ? c.reason || tr('不可行', 'Infeasible') : ''}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            </Card>
          )}
          {studyDir && (
            <Card
              title={
                state.researchTarget
                  ? tr('写回草稿', 'Write back to the draft')
                  : tr('写入新版本', 'Into a new version')
              }
              description={
                tuning
                  ? tr(
                      '把选中成员（默认最优）的参数写进草稿：先列出每个参数的研究值，勾选后写入，再到版本页审阅、冻结成新版本。原来的算例不变。',
                      'Write the chosen member (the best by default) into a draft: each parameter is listed with its study value, the ticked ones are written, then you review and freeze a new version on the versions page. The original cases stay unchanged.',
                    )
                  : tr(
                      '不确定性分析主要看输出的分布；需要时也可以把某个样本成员的参数写进新版本草稿。',
                      'Uncertainty analysis is mainly about the spread of outputs; a sample member can still be written into a new version draft when needed.',
                    )
              }
            >
              <div className="study-grid">
                <Field label={tr('用哪个成员', 'Member')}>
                  {/* 原来是手填成员编号的输入框；成员就是上面成绩表里那些，直接选。 */}
                  <Select value={writeMember} onChange={(e) => setMember(e.target.value)}>
                    {ranked.map(([key, c]) => (
                      <option key={key} value={key}>
                        {key}
                        {key === best ? tr('（最优）', ' (best)') : ''}
                        {key === 'm000000' ? tr('（基准）', ' (baseline)') : ''}
                        {c.calibration == null ? '' : ` · ${Number(c.calibration.toPrecision(4))}`}
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
              {writeVerdict && (
                <p className="muted study-hint">{tr(writeVerdict.zh, writeVerdict.en)}</p>
              )}
              {writeVerdict?.blocked && (
                <label className="param-check">
                  <input
                    type="checkbox"
                    checked={overrideFor === `${studyDir}#${writeMember}`}
                    onChange={(e) =>
                      setOverrideFor(e.target.checked ? `${studyDir}#${writeMember}` : '')
                    }
                  />
                  {tr(
                    '我知道这组参数不比默认参数好，仍要写入草稿',
                    'I know this set does not beat the default parameters; write it anyway',
                  )}
                </label>
              )}
              {!permissions.results ? (
                <p className="muted study-hint">
                  {tr('研究完成后才能写入。', 'Available once the study has finished.')}
                </p>
              ) : (
                !state.researchTarget &&
                !modelUsed && (
                  <p className="muted study-hint">
                    {tr(
                      '这个研究没有记下所用的模型版本，不能据此新建版本；从某个草稿的"研究"页签打开它，就能写回那个草稿。',
                      "This study did not record its model version, so no new version can be based on it; open it from a draft's Research tab to write back into that draft.",
                    )}
                  </p>
                )
              )}
              <div className="button-row">{renderActions([byId('study-write-version')!])}</div>
            </Card>
          )}
          {studyDir && (
            <Card title={tr('结果文件', 'Result files')}>
              {resultFiles.length > 0 ? (
                <>
                  <div className="study-grid">
                    <Field label={tr('选一个结果文件', 'Result file')}>
                      <Select value={resultPath} onChange={(e) => setResultPath(e.target.value)}>
                        {resultFiles.map((path) => (
                          <option key={path} value={path}>
                            {resultFileLabel(path, en)}
                          </option>
                        ))}
                      </Select>
                    </Field>
                  </div>
                  <div className="button-row">{renderActions([byId('study-results')!])}</div>
                </>
              ) : (
                <p className="versions-empty">{tr('还没有结果文件。', 'No result files yet.')}</p>
              )}
              {result && <StudyResult text={result} theme={state.theme} en={en} />}
              {/* 导出按钮在这个折叠区里：标题本身登记为动作，助手先展开才能按到"导出研究"。 */}
              <Advanced action="research-export-panel" title={tr('导出研究', 'Export the study')}>
                <Field label={tr('导出到哪个目录', 'Export to')}>
                  <div className="study-path">
                    <Input value={exportDir} onChange={(e) => setExportDir(e.target.value)} />
                    <ActionButton
                      action="study-export-pick"
                      label={tr('选择导出目录', 'Choose export directory')}
                      run={() => pickFolder('study-export', setExportDir)}
                    >
                      {tr('选择…', 'Choose…')}
                    </ActionButton>
                  </div>
                </Field>
                <div className="button-row">{renderActions([byId('study-export')!])}</div>
              </Advanced>
            </Card>
          )}
          {ai && (
            <Card
              title={tr('从已完成的率定学习参数', 'Learn parameters from finished calibrations')}
              description={tr(
                '把几个站点各自率定出的参数，按站点特征拟合成一个网络；留一研究验证通过才能存成版本。',
                'Fit a network from site features to the parameters calibrated at several sites; it can be saved as a version only if leave-one-study-out validation passes.',
              )}
            >
              <div className="button-row">{renderActions(aiActions.slice(3, 4))}</div>
              {hybridStudies.length > 0 && (
                <div className="study-cases">
                  {hybridStudies.map((s) => (
                    <label
                      key={s.dir}
                      className={`study-case${chosenStudies.includes(s.dir) ? ' is-on' : ''}`}
                    >
                      <input
                        type="checkbox"
                        checked={chosenStudies.includes(s.dir)}
                        onChange={(e) =>
                          setChosenStudies((old) =>
                            e.target.checked
                              ? [...new Set([...old, s.dir])]
                              : old.filter((d) => d !== s.dir),
                          )
                        }
                      />
                      <span>
                        <strong>{s.sites.join(', ')}</strong>
                        <small>
                          {statusText(s.status)} · {siteName(s.dir)}
                        </small>
                      </span>
                    </label>
                  ))}
                </div>
              )}
              <div className="study-grid">
                <Field
                  label={tr(
                    '拟合名称（字母、数字、- 和 _）',
                    'Fit name (letters, digits, - and _)',
                  )}
                >
                  <Input value={name} onChange={(e) => setName(e.target.value)} />
                </Field>
                <Field label={tr('拟合结果放在哪个目录', 'Fit output directory')}>
                  <div className="study-path">
                    <Input value={out} onChange={(e) => setOut(e.target.value)} />
                    <ActionButton
                      action="hybrid-fit-output-pick"
                      label={tr('选择拟合法输出目录', 'Choose fit output directory')}
                      run={() => pickFolder('hybrid-fit-out', setOut)}
                    >
                      {tr('选择…', 'Choose…')}
                    </ActionButton>
                  </div>
                </Field>
              </div>
              <div className="button-row">{renderActions(aiActions.slice(4))}</div>
              {fit && (
                <Notice tone={fitValidation(fit.report) === true ? 'success' : 'warning'}>
                  {fitValidation(fit.report) === true
                    ? tr(
                        '留一研究验证通过，可以存成版本。',
                        'Leave-one-study-out validation passed; it can be saved as a version.',
                      )
                    : fitValidation(fit.report) === false
                      ? tr(
                          '留一研究验证没通过，不能存成可用的版本。',
                          'Leave-one-study-out validation failed; it cannot be saved as a usable version.',
                        )
                      : tr(
                          '没有独立的交叉验证，不能推断在新站点上更好。',
                          'No independent cross-validation; better results at new sites are unproven.',
                        )}
                </Notice>
              )}
              <Advanced title={tr('拟合设置与报告', 'Fit settings and report')}>
                <div className="study-grid">
                  <Field label={tr('岭回归正则', 'Ridge regularization')}>
                    <Input type="number" value={ridge} onChange={(e) => setRidge(e.target.value)} />
                  </Field>
                  <Field label={tr('Adam 训练轮数', 'Adam epochs')}>
                    <Input
                      type="number"
                      value={epochs}
                      onChange={(e) => setEpochs(e.target.value)}
                    />
                  </Field>
                </div>
                {fit && <pre className="overflow-auto">{JSON.stringify(fit, null, 2)}</pre>}
              </Advanced>
            </Card>
          )}
          <Advanced
            title={tr('研究的完整设定与出处', 'Full study settings and provenance')}
            open={advanced}
            onOpenChange={setAdvanced}
          >
            <p className="muted study-hint">{studyDir}</p>
            <pre className="overflow-auto">
              {JSON.stringify(
                {
                  modelRevision: modelUsed ?? null,
                  selectedModelRevision: state.versionRef,
                  recordedVersion: envelope?.model_version,
                  manifest: envelope?.manifest,
                  tasks: envelope?.state?.tasks,
                },
                null,
                2,
              )}
            </pre>
          </Advanced>
          {!studyDir && footer(true)}
        </>
      )}
      {writing && (
        <StudyImportDialog
          preview={writing}
          overlay={writing.draft.overlay}
          en={en}
          busy={busy}
          error={shownError}
          onClose={cancelWriteBack}
          onImport={(chosen) => execute(() => confirmWriteBack(chosen))}
        />
      )}
    </div>
  );
}
