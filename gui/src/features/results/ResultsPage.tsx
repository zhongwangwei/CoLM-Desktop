import { bilingual } from '../../lib/bilingual';
import { useCallback, useEffect, useRef, useState } from 'react';
import { sameSeries } from './chart-data';
import uPlot from 'uplot';
import { timeTicks } from './time-ticks';
import { utcSeconds } from '../../lib/dates';
import 'uplot/dist/uPlot.min.css';
import { hasBackend, invoke, invokeJson } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Card, Field, Input, Select, Notice, Advanced, Dialog } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import {
  boundedMap,
  download,
  fullEnergyStats,
  fullSeriesCsv,
  parentRuns,
  printReport,
  reportFile,
  type EnergyDiagnostic,
  type RecordedRun,
  type ReportFormat,
  type ResultMetric,
  type ResultReport,
  type SeriesData,
} from './series';
import {
  overviewMetric,
  overviewRunStatus,
  overviewVariable,
  overviewRunLabel,
  overviewFinding,
  overviewVariableLabel,
  overviewRunLocation,
} from './result-overview';
import {
  chartVariables,
  isGridCatalog,
  readErrorText,
  variableOption,
  type CatalogVariable,
} from './variable-labels';
import GridMap from './GridMap';
import NavIcon from '../../components/NavIcon';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import { defaultObservation } from './default-observation';
import './results-page.css';
type Catalog = {
  files: number;
  steps: number;
  start: number;
  end: number;
  variables: CatalogVariable[];
};
type Run = RecordedRun & {
  run_id?: string;
  id?: string;
  legacy?: boolean;
  /** Server runs: the server, and whether their outputs were fetched to this computer. */
  server?: string | null;
  outputs_local?: boolean;
  [key: string]: unknown;
};
/** `model_project_runs` 返回的一条可比较运行（带所属站点，供多站点比较选择器分组）。 */
type ProjectRun = Run & {
  run_case: string;
  case_dir: string;
  case_name: string;
  finished_ms?: number | null;
};
/** 多站点比较的一行：一个运行副本 + 它的观测文件。 */
type ComparisonEntry = { run: ProjectRun; obs: string };
type BaselineComparison = {
  same_design: boolean;
  better?: string | null;
  notes?: string[];
  variables?: { variable: string; same_pairs: boolean; better: string | null; metric: string }[];
  metrics_a?: Metric[];
  metrics_b?: Metric[];
};
type Metric = ResultMetric & {
  provenance?: Run;
  /** Multi-station comparison: the observation file that produced this row. */
  comparison_obs?: string;
  time?: number[];
  model?: (number | null)[];
  obs?: (number | null)[];
  units?: string;
};
const tabs = [
  'overview',
  'catalog',
  'series',
  'evaluation',
  'comparison',
  'diagnostics',
  'report',
] as const;
/** 页签的中英文名称；页签按钮与助手契约共用一份，避免两处各写一遍再走样。 */
const tabLabels = {
  en: [
    'Overview',
    'Data catalog',
    'Time series',
    'Observation evaluation',
    'Site comparison',
    'Diagnostics',
    'Report',
  ],
  zh: ['结果总览', '数据目录', '时序分析', '观测评估', '多站比较', '闭合诊断', '报告导出'],
} as const;
/**
 * 站点内的页签：随上面选中的站点/运行变化。多站点比较不是页签，而是"分析范围"里的另一个选项，
 * 由结果页上方的单站点/多站点选择器切换（2026-10-28 用户反馈：别把跨站点功能混进站点页签）。
 * 报告导出视图已从界面移除（用户要求），保留内部 tab 支持。
 */
const stationTabs = ['overview', 'catalog', 'series', 'evaluation', 'diagnostics'] as const;
const tabIndex = (name: (typeof tabs)[number]) => tabs.indexOf(name);
export function ScienceChart({
  data,
  labels,
  theme,
  paired = false,
  compact = false,
  styles,
}: {
  data: uPlot.AlignedData;
  labels: string[];
  theme: string;
  paired?: boolean;
  compact?: boolean;
  /** 逐系列的线色、虚线、线宽；不给就用默认的三色循环（研究的分位带要两色加虚线，见 StudyResult）。 */
  styles?: { stroke: string; dash?: number[]; width?: number }[];
}) {
  const ref = useRef<HTMLDivElement>(null);
  const plotRef = useRef<uPlot | null>(null);
  // 调用方每次渲染都新建 data/labels 数组：原来 effect 以它们为依赖，页面上任何输入都会让图整张
  // 销毁重建，看得见地闪一下（第 735 轮）。内容没变就沿用上一次的引用；只有数据变了用 setData，
  // 标签、主题、尺寸变了才重建。
  const stableData = useRef(data);
  if (!sameSeries(stableData.current, data)) stableData.current = data;
  const shownData = stableData.current;
  const labelKey = labels.join('\u0000');
  const styleKey = JSON.stringify(styles ?? null);
  const hasData = data[0].length > 0;
  useEffect(() => {
    if (!ref.current || !hasData) return;
    const host = ref.current;
    // 调用方每次渲染都新建 styles 数组：按内容（styleKey）重建，不按引用。
    const seriesStyles = JSON.parse(styleKey) as typeof styles;
    const plot = new uPlot(
      {
        width: Math.max(160, host.clientWidth),
        height: compact ? 220 : 300,
        legend: { show: !compact },
        tzDate: (ts) => uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'),
        series: [
          {},
          ...labelKey.split('\u0000').map((label, i) => ({
            label,
            stroke:
              seriesStyles?.[i]?.stroke ??
              (paired ? ['#0d9488', '#858b94'] : ['#0d9488', '#e87945', '#6366f1'])[i % 3],
            dash: seriesStyles?.[i]?.dash ?? (paired && i === 1 ? [6, 3] : []),
            width: seriesStyles?.[i]?.width ?? 1.5,
            points: compact ? { show: false } : undefined,
          })),
        ],
        axes: [
          { stroke: theme === 'dark' ? '#b8c7d9' : '#475569', values: timeTicks },
          { stroke: theme === 'dark' ? '#b8c7d9' : '#475569' },
        ],
      },
      stableData.current,
      host,
    );
    plotRef.current = plot;
    const resize = new ResizeObserver(() =>
      plot.setSize({ width: Math.max(160, host.clientWidth), height: compact ? 220 : 300 }),
    );
    resize.observe(host);
    return () => {
      resize.disconnect();
      plot.destroy();
      plotRef.current = null;
    };
  }, [hasData, labelKey, theme, paired, compact, styleKey]);
  useEffect(() => {
    plotRef.current?.setData(shownData);
  }, [shownData]);
  return <div ref={ref} aria-label={labels.join(', ')} style={{ minWidth: 0 }} />;
}
function runLabel(path: string) {
  const parts = path.replace(/[\\/]+$/, '').split(/[\\/]/);
  const at = parts.lastIndexOf('runs');
  return at > 0 && parts[at + 1] ? `${parts[at - 1]} · ${parts[at + 1].slice(0, 8)}` : path;
}
const number = (v: unknown, digits = 3) =>
  typeof v === 'number' && Number.isFinite(v) ? v.toFixed(digits) : '—';
export default function ResultsPage({
  casePath,
  onBack,
}: {
  casePath?: string;
  onBack?: () => void;
}) {
  const { state, navigate, openCase, update } = useApp();
  // "模型改进"进"新建修改版"的 6 种方式（研究不再单独占侧栏一项）。
  const improveModel = () => {
    update({ versionsView: 'create' });
    return navigate('versions');
  };
  const en = state.language === 'en';
  const tr = (zh: string, english: string) => (en ? english : zh);
  const caseDir = casePath || state.selectedCase?.dir || '';
  const versionId = state.versionRef.id;
  const revision = state.versionRef.revision;
  const epoch = useRef(0);
  const stationTarget = useRef(caseDir);
  const [stationRetry, setStationRetry] = useState(0);
  const [stationError, setStationError] = useState('');
  useEffect(() => setStationError(''), [caseDir]);
  const lock = useRef(false);
  const cache = useRef(new Map<string, unknown>());
  const [seriesKey, setSeriesKey] = useState('');
  const [advanced, setAdvanced] = useState(false);
  const [settingsOpen, setSettingsOpen] = useState(false),
    [runChooser, setRunChooser] = useState(false);
  const [catalogKey, setCatalogKey] = useState('');
  const [singleEvaluation, setSingleEvaluation] = useState<{ key: string; rows: Metric[] } | null>(
    null,
  );
  const evaluationKey = JSON.stringify({ caseDir, sourceVersion: versionId, revision });
  const [baseline, setBaseline] = useState<BaselineComparison | null>(null);
  const [baselineNote, setBaselineNote] = useState('');
  const [reportFormat, setReportFormat] = useState<ReportFormat>('json');
  const [exportNotice, setExportNotice] = useState('');
  const [tab, setTab] = useState<(typeof tabs)[number]>('overview');
  // 单站点/多站点是"分析范围"，不是页签：多站点就是多运行比较页，页签只保留站点内视图。
  // 记住进多站点前看的是哪个站点内视图，切回单站点时回到原处。
  const lastStationTab = useRef<(typeof tabs)[number]>('overview');
  if (tab !== 'comparison' && tab !== 'report') lastStationTab.current = tab;
  const multiStation = tab === 'comparison';
  const [runs, setRuns] = useState<Run[]>([]);
  const [source, setSource] = useState('');
  const completedRunCase =
    state.running?.[caseDir]?.status === 'succeeded' ? state.running[caseDir]?.run_case : undefined;
  const latestCompletedRun = useRef(completedRunCase);
  latestCompletedRun.current = completedRunCase;
  const [catalog, setCatalog] = useState<Catalog | null>(null);
  const [variable, setVariable] = useState('');
  const [obs, setObs] = useState('');
  const observationEdited = useRef(false);
  const automaticObservation = useRef('');
  const automaticEvaluation = useRef('');
  const automaticEvaluator = useRef<() => void>(() => {});
  const chooseObservation = (path: string) => {
    observationEdited.current = true;
    setObs(path);
  };
  // 服务器项目的观测在服务器上，评估也在那里算：选文件要浏览服务器的目录，不是本机文件框。
  const [observationBrowser, setObservationBrowser] = useState(false);
  // 只有真在取回时按钮才写"正在取回…"：原来看的是整页 busy，服务器上算评估时也显示成正在取回 0.5 GB。
  const [fetching, setFetching] = useState(false);
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [spinup, setSpinup] = useState('0');
  const [corrected, setCorrected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [series, setSeries] = useState<SeriesData | null>(null);
  const [metrics, setMetrics] = useState<Metric[]>([]);
  const [diagnostic, setDiagnostic] = useState<EnergyDiagnostic | null>(null);
  // 多站点比较：选中运行（有序，每行一个观测文件）+ 项目里可比较的运行清单（一次 IPC 查完）。
  const [comparisonEntries, setComparisonEntries] = useState<ComparisonEntry[]>([]);
  const [projectRuns, setProjectRuns] = useState<ProjectRun[]>([]);
  const [projectRunsLoading, setProjectRunsLoading] = useState(false);
  const [projectRunsError, setProjectRunsError] = useState('');
  const [comparisonProgress, setComparisonProgress] = useState<{
    done: number;
    total: number;
  } | null>(null);
  // 跨项目比较：手填一个运行目录（不在当前项目的清单里）。
  const [externalRun, setExternalRun] = useState('');
  // 90 个站点时运行列表很长：按站点/运行/版本搜索，并可只看结果还在服务器上的运行。
  const [comparisonQuery, setComparisonQuery] = useState('');
  const [comparisonServerOnly, setComparisonServerOnly] = useState(false);
  // 站间指标对比图：同一变量、同一指标（RMSE）按站点横向条形，长度按本次最大值归一化。
  const [comparisonVariable, setComparisonVariable] = useState('');
  // 站点清单按内容做依赖：`state.cases` 每次渲染都可能是新数组，直接放进 effect 依赖会无限重查。
  const caseDirsKey = state.cases.map((entry) => entry.dir).join('\n');
  const [evaluation, setEvaluation] = useState<
    {
      name: string;
      available: boolean;
      reason?: string;
      missing_model?: string[];
      missing_observation?: string[];
    }[]
  >([]);
  const selectionKey = JSON.stringify({ evaluationKey, source, obs, from, to, spinup, corrected });
  const selectedSeriesKey = JSON.stringify({ source, variable, from, to });
  const selectedMetric =
    singleEvaluation?.key === selectionKey
      ? overviewMetric(singleEvaluation.rows, variable)
      : undefined;
  const selectedRun = runs.find((r) => r.run_case === source);
  useEffect(() => {
    const requestEpoch = epoch;
    requestEpoch.current++;
    stationTarget.current = caseDir;
    cache.current.clear();
    setRuns([]);
    setSource('');
    setCatalog(null);
    setSeries(null);
    setMetrics([]);
    setDiagnostic(null);
    setObs('');
    observationEdited.current = false;
    automaticObservation.current = '';
    automaticEvaluation.current = '';
    setFrom('');
    setTo('');
    setSpinup('0');
    setCorrected(false);
    setComparisonEntries([]);
    setError('');
    let cancelled = false;
    if (!caseDir) return;
    void defaultObservation(caseDir)
      .then((path) => {
        if (!cancelled && stationTarget.current === caseDir && !observationEdited.current && path) {
          automaticObservation.current = path;
          setObs(path);
        }
      })
      .catch(() => {});
    void invokeJson<Run[]>('model_case_runs', { dir: caseDir })
      .then((data) => {
        if (!cancelled && stationTarget.current === caseDir) {
          setRuns(data);
          setSource(
            data.find((r) => r.run_case === latestCompletedRun.current)?.run_case ||
              data.find(
                (r) =>
                  r.run_case && r.version?.id === versionId && r.version?.revision === revision,
              )?.run_case ||
              // A ledger-imported legacy placeholder points at the case itself and has no output.
              data.find((r) => r.run_case && r.version)?.run_case ||
              data.find((r) => r.run_case)?.run_case ||
              '',
          );
        }
      })
      .catch((e) => {
        if (!cancelled && stationTarget.current === caseDir) setError(String(e));
      });
    return () => {
      cancelled = true;
      requestEpoch.current++;
    };
  }, [caseDir, versionId, revision, stationRetry]);
  // A run of this case finishing adds and selects its output without discarding the chosen
  // observations, window, spin-up or diagnostics; starting a run changes nothing here.
  useEffect(() => {
    if (!caseDir || !completedRunCase) return;
    let cancelled = false;
    void invokeJson<Run[]>('model_case_runs', { dir: caseDir })
      .then((data) => {
        if (cancelled || stationTarget.current !== caseDir) return;
        setRuns(data);
        if (data.some((r) => r.run_case === completedRunCase)) setSource(completedRunCase);
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [caseDir, completedRunCase]);
  useEffect(() => {
    const requestEpoch = epoch;
    requestEpoch.current++;
    let cancelled = false;
    setError('');
    setCatalog(null);
    setSeries(null);
    setMetrics([]);
    setDiagnostic(null);
    setEvaluation([]);
    setCatalogKey('');
    if (!source) return;
    void invokeJson<Catalog>('history_catalog', { case: source })
      .then((data) => {
        if (!cancelled && stationTarget.current === caseDir) {
          setCatalog(data);
          setCatalogKey(source);
          setVariable(overviewVariable(data.variables));
        }
      })
      .catch((e) => {
        if (!cancelled && stationTarget.current === caseDir) setError(String(e));
      });
    return () => {
      cancelled = true;
      requestEpoch.current++;
    };
  }, [source, caseDir, versionId, revision]);
  useEffect(() => {
    epoch.current++;
    setMetrics([]);
    setDiagnostic(null);
    setSingleEvaluation(null);
  }, [from, to, obs, spinup, corrected, comparisonEntries]);
  useEffect(() => {
    setSeries(null);
    setSeriesKey('');
  }, [from, to]);
  useEffect(() => {
    epoch.current++;
    setSeries(null);
  }, [variable]);
  useEffect(() => {
    let cancelled = false;
    setBaseline(null);
    setBaselineNote('');
    const selected = runs.find((run) => run.run_case === source);
    if (!selected?.version || !selected.operation_id) return;
    const version = selected.version;
    const operationId = selected.operation_id;
    const timer = setTimeout(() => {
      void (async () => {
        const model = await invokeJson<{ parent: { id: string; revision: number } | null }>(
          'model_get',
          version,
        );
        if (cancelled) return;
        if (!model.parent) {
          setBaselineNote('official');
          return;
        }
        const candidates = parentRuns(runs, model.parent);
        if (!candidates.length) {
          setBaselineNote('missing');
          return;
        }
        if (!obs.trim()) {
          setBaselineNote('observation');
          return;
        }
        const records = Number(spinup);
        if (!Number.isInteger(records) || records < 0)
          throw new Error(
            bilingual('跳过的模型记录数要是非负整数', 'Invalid skipped record count'),
          );
        const parse = (value: string) => (value ? utcSeconds(value) : null);
        const start = parse(from),
          end = parse(to);
        if (
          (start !== null && !Number.isFinite(start)) ||
          (end !== null && !Number.isFinite(end)) ||
          (start !== null && end !== null && start >= end)
        )
          throw new Error(bilingual('评估时段不对：开始要早于结束', 'Invalid evaluation window'));
        for (const candidate of candidates) {
          if (cancelled) return;
          const result = await invokeJson<BaselineComparison>('history_compare', {
            a: operationId,
            b: candidate.operation_id,
            obs,
            spinup: records,
            from: start,
            to: end,
            corrected,
            variable: variable || null,
          });
          if (cancelled) return;
          setBaseline(result);
          if (result.same_design) return;
        }
      })().catch((error) => {
        if (!cancelled && stationTarget.current === caseDir) setBaselineNote(String(error));
      });
    }, 350);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [runs, source, obs, spinup, from, to, corrected, variable, caseDir]);
  // 多站点比较：项目里所有站点的成功运行一次查完（原来的实现要逐站点调 model_case_runs，90 个站点
  // 就是 90 次 IPC + 台账扫描）。进入多站点范围时加载一次，失败也不挡住其他视图。
  useEffect(() => {
    if (!multiStation) return;
    const dirs = (caseDirsKey ? caseDirsKey.split('\n') : [caseDir]).filter(Boolean);
    if (!dirs.length) return;
    let cancelled = false;
    setProjectRunsLoading(true);
    setProjectRunsError('');
    invokeJson<ProjectRun[]>('model_project_runs', { dirs })
      .then((rows) => {
        if (!cancelled) setProjectRuns(Array.isArray(rows) ? rows : []);
      })
      .catch((failure) => {
        if (!cancelled) setProjectRunsError(String(failure));
      })
      .finally(() => {
        if (!cancelled) setProjectRunsLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [multiStation, caseDirsKey, caseDir]);
  async function task<T>(fn: (current: () => boolean) => Promise<T>): Promise<T> {
    if (lock.current) throw new Error('Operation in progress');
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
  const run = (fn: () => Promise<unknown>) => {
    void fn().catch(() => {});
  };
  // 结果还在服务器上的运行，变量表和曲线由后端改在服务器上读（`remote-read`），只传回算好的数据；
  // 要 NetCDF 文件或评估指标时才按需取回这一个站点、这一次运行（一个站点约 0.5 GB）。
  const awaitingFetch = !!selectedRun?.server && selectedRun.outputs_local === false;
  async function fetchSelectedRun() {
    const target = source,
      dir = caseDir;
    setFetching(true);
    try {
      await task(async (current) => {
        await invokeJson('remote_fetch', {
          case: target,
          host: null,
          all: false,
          overwriteNewer: false,
        });
        const data = await invokeJson<Run[]>('model_case_runs', { dir });
        if (current() && stationTarget.current === dir) setRuns(data);
      });
    } finally {
      setFetching(false);
    }
  }
  const windowArgs = useCallback(() => {
    const parse = (value: string) => {
      if (!value) return null;
      const n = utcSeconds(value);
      if (!Number.isFinite(n)) throw new Error(bilingual('日期格式不对', 'Invalid UTC date'));
      return n;
    };
    const a = parse(from),
      b = parse(to);
    if (a !== null && b !== null && a >= b)
      throw new Error(bilingual('开始时间要早于结束时间', 'Start must precede end'));
    return { from: a, to: b };
  }, [from, to]);
  const readSeries = useCallback(
    async function readSeries(vars: string, maxPoints: number | null = 2400) {
      const args = { case: source, vars, ...windowArgs(), maxPoints };
      const key = JSON.stringify(args);
      if (maxPoints !== null && cache.current.has(key)) return cache.current.get(key) as SeriesData;
      const value = await invokeJson<SeriesData>('series', args).catch((error: unknown) => {
        throw new Error(readErrorText(error, en));
      });
      if (maxPoints !== null) {
        if (cache.current.size >= 16) cache.current.delete(cache.current.keys().next().value!);
        cache.current.set(key, value);
      }
      return value;
    },
    [source, windowArgs, en],
  );
  useEffect(() => {
    let cancelled = false;
    if (tab !== 'overview' || !source || !variable || catalogKey !== source) return;
    void readSeries(variable)
      .then((data) => {
        if (!cancelled && stationTarget.current === caseDir) {
          setSeries(data);
          setSeriesKey(selectedSeriesKey);
        }
      })
      .catch((error) => {
        if (!cancelled && stationTarget.current === caseDir) setError(String(error));
      });
    return () => {
      cancelled = true;
    };
  }, [readSeries, source, variable, selectedSeriesKey, tab, catalogKey, caseDir]);
  const plot = () =>
    task(async (current) => {
      const data = await readSeries(variable);
      if (current()) {
        setSeries(data);
        setSeriesKey(selectedSeriesKey);
      }
      return data;
    });
  const evaluate = () =>
    task(async (current) => {
      if (!obs.trim()) throw new Error(tr('请填写真实观测文件路径', 'Provide an observation file'));
      const days = Number(spinup);
      if (!Number.isInteger(days) || days < 0)
        throw new Error(bilingual('预热要是非负整数', 'Invalid spinup'));
      const available = await invokeJson<typeof evaluation>('evaluation_catalog', {
        case: source,
        obs,
      });
      if (!current()) return;
      setEvaluation(available);
      const names = available.filter((v) => v.available).map((v) => v.name);
      if (!names.length) {
        const rows: Metric[] = [];
        setMetrics(rows);
        setSingleEvaluation({ key: selectionKey, rows });
        setSettingsOpen(false);
        return rows;
      }
      const rows = await invokeJson<Metric[]>('metrics', {
        case: source,
        obs,
        spinup: days,
        corrected,
        summaryOnly: false,
        pairVars: names,
        maxPoints: 2400,
        ...windowArgs(),
      });
      if (current()) {
        setMetrics(rows);
        setSingleEvaluation({ key: selectionKey, rows });
        setSettingsOpen(false);
      }
      return rows;
    });
  automaticEvaluator.current = () => run(evaluate);
  useEffect(() => {
    if (
      busy ||
      !source ||
      catalogKey !== source ||
      !obs ||
      observationEdited.current ||
      automaticObservation.current !== obs ||
      automaticEvaluation.current === selectionKey
    )
      return;
    automaticEvaluation.current = selectionKey;
    automaticEvaluator.current();
  }, [busy, source, catalogKey, obs, selectionKey]);
  const diagnose = () =>
    task(async (current) => {
      setDiagnostic(null);
      const data = await invokeJson<EnergyDiagnostic>('energy_summary', {
        case: source,
        ...windowArgs(),
      }).catch((error: unknown) => {
        throw new Error(readErrorText(error, en));
      });
      fullEnergyStats(data);
      if (current()) {
        setDiagnostic(data);
      }
    });
  const compare = () =>
    task(async (current) => {
      if (!Number.isInteger(Number(spinup)) || Number(spinup) < 0)
        throw new Error(bilingual('预热要是非负整数', 'Invalid spinup'));
      const entries = comparisonEntries;
      if (!entries.length || entries.length > 20)
        throw new Error(bilingual('要选 1–20 个运行再比较', 'Select 1–20 runs to compare'));
      if (entries.some((entry) => !entry.obs.trim()))
        throw new Error(
          bilingual(
            '每个运行都要有观测文件；可以先在上面填一个共用观测再点「应用到所有行」',
            'Every run needs an observation file; fill the shared one and apply it to all rows',
          ),
        );
      // 项目运行清单已经一次查回来了就直接核对；不在清单里的（跨项目/手填路径）只按运行目录反推
      // 所属算例、查那一个算例，不再扫全项目。
      let sources: Run[] = projectRuns;
      const unknown = entries.filter(
        (entry) => !sources.some((run) => run.run_case === entry.run.run_case),
      );
      if (unknown.length) {
        const dirs = [
          ...new Set(
            unknown
              .map((entry) => caseDirOfRunPath(entry.run.run_case))
              .filter((dir): dir is string => !!dir),
          ),
        ];
        const inventories = await boundedMap(dirs, 2, (dir) =>
          invokeJson<Run[]>('model_case_runs', { dir }).catch(() => []),
        );
        if (!current()) return;
        sources = [...sources, ...inventories.flat()];
      }
      for (const entry of entries) {
        if (!sources.some((r) => r.run_case === entry.run.run_case))
          throw new Error(`Not a recorded run copy: ${entry.run.run_case}`);
      }
      setComparisonProgress({ done: 0, total: entries.length });
      // 远程每行一次 SSH；10 个会话的上限内 6 路并行，90 个站点时比 4 路更快。
      const rows = await boundedMap(entries, 6, async (entry) => {
        const values = await invokeJson<Metric[]>('metrics', {
          case: entry.run.run_case,
          obs: entry.obs.trim(),
          spinup: Number(spinup),
          corrected,
          summaryOnly: true,
          pairVars: null,
          maxPoints: 2400,
          ...windowArgs(),
        });
        setComparisonProgress((previous) =>
          previous ? { ...previous, done: previous.done + 1 } : previous,
        );
        return values.map((v) => ({
          ...v,
          site: entry.run.run_case,
          comparison_obs: entry.obs.trim(),
          provenance: sources.find((r) => r.run_case === entry.run.run_case) ?? entry.run,
        }));
      });
      setComparisonProgress(null);
      if (current()) setMetrics(rows.flat());
    });
  function exportMessage(result: Awaited<ReturnType<typeof download>>) {
    if (result.status === 'cancelled') return tr('已取消导出。', 'Export cancelled.');
    if (result.status === 'saved') return tr('已保存到：', 'Saved to: ') + result.path;
    return tr(
      '已请求下载；请在下载位置检查文件。',
      'Download requested. Check the download location.',
    );
  }
  const exportFullSeries = () =>
    task(async (current) => {
      setExportNotice('');
      if (!source || !variable)
        throw new Error(
          bilingual('请先选一次运行和一个变量', 'Select a recorded run and variable'),
        );
      const data = await readSeries(variable, null);
      const text = fullSeriesCsv(data, variable);
      if (!current()) throw new Error('Analysis selection changed before the export');
      const name = `${source.split(/[\\/]/).at(-1) || 'colm'}-${variable}.csv`.replace(
        /[<>:"/\\|?*\p{Cc}]/gu,
        '_',
      );
      const result = await download(name, text, 'text/csv;charset=utf-8');
      if (current()) setExportNotice(exportMessage(result));
      return { ...result, n: data.n, source_n: data.source_n, downsampled: false };
    });
  function reportData(): ResultReport {
    return {
      generated_at: new Date().toISOString(),
      source,
      provenance: runs.find((r) => r.run_case === source) || null,
      catalog,
      evaluation,
      metrics: metrics.map((metric) => {
        const summary = { ...metric };
        delete summary.time;
        delete summary.model;
        delete summary.obs;
        return summary;
      }),
      diagnostic,
      baseline,
      baselineNote,
      conditions: {
        obs,
        from,
        to,
        spinup,
        corrected,
        comparisonSources: comparisonText(comparisonEntries),
      },
      notice: 'No automatic ranking across unequal inputs or evaluation conditions',
    };
  }
  const report = () =>
    task(async (current) => {
      setExportNotice('');
      if (!catalog)
        throw new Error(
          bilingual('请先读出一次运行的结果再导出', 'Load a recorded result before exporting'),
        );
      const data = reportData();
      if (reportFormat === 'pdf') {
        const result = await printReport(
          data,
          en,
          hasBackend() ? () => invoke('print_report') : undefined,
        );
        if (current())
          setExportNotice(
            tr(
              '已请求打开打印对话框；请选择“另存为 PDF”。',
              'Print dialog requested. Choose Save as PDF.',
            ),
          );
        return result;
      }
      const file = reportFile(data, reportFormat, en);
      const result = await download(file.filename, file.text, file.mime_type);
      if (current()) setExportNotice(exportMessage(result));
      return result;
    });
  async function chooseStation(dir: string) {
    if (lock.current || dir === caseDir) return;
    if (!state.cases.some((entry) => entry.dir === dir))
      throw new Error(tr('该站点不在当前项目中', 'This station is not in the current project'));
    epoch.current++;
    stationTarget.current = dir;
    setStationError('');
    await task(async () => {
      cache.current.clear();
      setRuns([]);
      setSource('');
      setCatalog(null);
      setCatalogKey('');
      setSeries(null);
      setSeriesKey('');
      setSingleEvaluation(null);
      setMetrics([]);
      setDiagnostic(null);
      setBaseline(null);
      setBaselineNote('');
      setEvaluation([]);
      setRunChooser(false);
      try {
        // 没有结果 = 被后来的页面切换或打开取代了：不算切换失败、不报错，但还在本页时把原来的站点读回来。
        if (!(await openCase(dir, 'results'))) {
          stationTarget.current = caseDir;
          setStationRetry((previous) => previous + 1);
        }
      } catch (error) {
        stationTarget.current = caseDir;
        setStationError(String(error));
        setStationRetry((previous) => previous + 1);
        throw error;
      }
    });
  }
  function chooseRecordedRun(path: string) {
    if (!runs.some((run) => run.run_case === path))
      throw new Error(tr('该运行记录不可用', 'This recorded run is unavailable'));
    epoch.current++;
    setSource(path);
    setRunChooser(false);
  }
  /** 多站点比较的契约文本：每行"运行目录 | 观测文件"，界面用结构化列表维护。 */
  function comparisonText(entries: ComparisonEntry[]) {
    return entries.map((entry) => `${entry.run.run_case} | ${entry.obs}`).join('\n');
  }
  function applyComparisonText(value: string) {
    const lines = value
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean);
    setComparisonEntries(
      lines
        .map((line) => {
          const [runCase, observation = ''] = line.split('|').map((part) => part.trim());
          const run =
            projectRuns.find((item) => item.run_case === runCase) ??
            ({ run_case: runCase, case_dir: '', case_name: '', status: 'recorded' } as ProjectRun);
          return { run, obs: observation };
        })
        .filter((entry) => entry.run.run_case),
    );
  }
  function comparisonRunFact(run: ProjectRun) {
    const parts = [overviewRunStatus(run, en)];
    parts.push(
      run.version
        ? `${run.version.id}@${run.version.revision}`
        : en
          ? 'version unknown'
          : '版本身份未知',
    );
    if (run.server)
      parts.push(
        run.outputs_local === false
          ? en
            ? 'outputs on server'
            : '结果在服务器上'
          : en
            ? 'outputs fetched locally'
            : '结果已取回本机',
      );
    if (run.finished_ms)
      parts.push(new Date(run.finished_ms).toLocaleString(en ? 'en-US' : 'zh-CN'));
    return parts.join(' · ');
  }
  /** 搜索框与"只看结果在服务器上"过滤后的运行；分组展示前先过滤。 */
  function visibleProjectRuns(runs: ProjectRun[]) {
    const query = comparisonQuery.trim().toLowerCase();
    return runs.filter((run) => {
      if (comparisonServerOnly && !(run.server && run.outputs_local === false)) return false;
      if (!query) return true;
      const version = run.version ? `${run.version.id}@${run.version.revision}` : '';
      return [run.case_name, run.case_dir, run.run_case, run.operation_id, version, run.status]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(query));
    });
  }
  /** 按站点分组（保持后端返回的最近优先顺序），供多选列表使用。 */
  function groupedProjectRuns(runs: ProjectRun[]) {
    const groups = new Map<string, { caseName: string; runs: ProjectRun[] }>();
    for (const run of runs) {
      const key = run.case_dir || run.case_name || run.run_case;
      const group: { caseName: string; runs: ProjectRun[] } = groups.get(key) ?? {
        caseName: run.case_name || key.split(/[\\/]/).at(-1) || key,
        runs: [] as ProjectRun[],
      };
      group.runs.push(run);
      groups.set(key, group);
    }
    return [...groups.entries()].map(([dir, group]) => ({ dir, ...group }));
  }
  /** 运行目录 `<算例>/runs/<编号>` 的所属算例：比较时只查这些算例，不再扫全项目。 */
  function caseDirOfRunPath(runCase: string) {
    const parts = runCase.replace(/[\\/]+$/, '').split(/[\\/]/);
    const at = parts.lastIndexOf('runs');
    return at > 0 ? parts.slice(0, at).join('/') : null;
  }
  function addExternalComparisonRun() {
    const path = externalRun.trim();
    if (!path) return;
    setComparisonEntries((current) =>
      current.some((entry) => entry.run.run_case === path)
        ? current
        : [
            ...current,
            {
              run: {
                run_case: path,
                case_dir: caseDirOfRunPath(path) ?? '',
                case_name: caseDirOfRunPath(path)?.split(/[\\/]/).at(-1) ?? path,
                status: 'recorded',
              } as ProjectRun,
              obs: obs.trim(),
            },
          ],
    );
    setExternalRun('');
  }
  function toggleComparisonRun(run: ProjectRun) {
    setComparisonEntries((current) =>
      current.some((entry) => entry.run.run_case === run.run_case)
        ? current.filter((entry) => entry.run.run_case !== run.run_case)
        : [...current, { run, obs: obs.trim() }],
    );
  }
  function removeComparisonEntry(index: number) {
    setComparisonEntries((current) => current.filter((_, item) => item !== index));
  }
  function setComparisonEntryObs(index: number, value: string) {
    setComparisonEntries((current) =>
      current.map((entry, item) => (item === index ? { ...entry, obs: value } : entry)),
    );
  }
  function applySharedObservation() {
    setComparisonEntries((current) => current.map((entry) => ({ ...entry, obs: obs.trim() })));
  }
  const exportComparison = () =>
    task(async (current) => {
      setExportNotice('');
      if (!metrics.length)
        throw new Error(
          tr('先运行比较，再导出指标', 'Run the comparison before exporting metrics'),
        );
      const escape = (value: unknown) => {
        const text = value == null ? '' : String(value);
        return /[",\n]/.test(text) ? `"${text.replaceAll('"', '""')}"` : text;
      };
      const header = [
        'site',
        'run_case',
        'status',
        'version',
        'observation',
        'from',
        'to',
        'spinup',
        'corrected',
        'variable',
        'units',
        'n',
        'rmse',
        'mae',
        'bias',
        'r2',
        'r',
        'nse',
        'kge',
      ];
      const lines = [header.join(',')];
      for (const metric of metrics) {
        const run =
          (metric.provenance as ProjectRun | undefined) ??
          comparisonEntries.find((entry) => entry.run.run_case === metric.site)?.run;
        lines.push(
          [
            run
              ? `${run.case_name || run.case_dir} · ${String(metric.site).split(/[\\/]/).at(-1)}`
              : metric.site,
            metric.site,
            run?.status ?? '',
            run?.version ? `${run.version.id}@${run.version.revision}` : '',
            metric.comparison_obs ?? '',
            from,
            to,
            spinup,
            corrected ? '1' : '0',
            metric.name,
            metric.units,
            metric.n,
            metric.rmse,
            metric.mae,
            metric.bias,
            metric.r2,
            metric.correlation,
            metric.nse,
            metric.kge,
          ]
            .map(escape)
            .join(','),
        );
      }
      const result = await download(
        'multi-station-metrics.csv',
        lines.join('\n'),
        'text/csv;charset=utf-8',
      );
      if (current()) setExportNotice(exportMessage(result));
      return result;
    });
  function changeSettings(open: boolean) {
    if (!lock.current) setSettingsOpen(open);
  }
  function changeRunChooser(open: boolean) {
    if (!lock.current) setRunChooser(open);
  }
  const fields: UiField[] = runChooser
    ? [
        {
          id: 'result-source',
          label: tr('运行副本', 'Run copy'),
          type: 'choice',
          value: source,
          options: runs
            .filter((r) => r.run_case)
            .map((r) => ({ value: r.run_case!, label: overviewRunLabel(r, en) })),
          disabled: busy,
          set: chooseRecordedRun,
        },
      ]
    : settingsOpen
      ? []
      : [
          {
            id: 'result-tab',
            label: tr('分析视图', 'Analysis view'),
            type: 'choice',
            // 报告导出视图不再出现在界面里（用户 2026-10-28：报告导出按钮不要）；保留内部 tab 支持，
            // 因此这里只列出界面上真正能看到的页签。
            value: tab,
            options: tabs
              .filter((v) => v !== 'report')
              .map((v) => ({
                value: v,
                label: en ? tabLabels.en[tabIndex(v)] : tabLabels.zh[tabIndex(v)],
              })),
            disabled: busy,
            set: (v) => setTab(v as typeof tab),
          },
        ];
  if (
    !runChooser &&
    !settingsOpen &&
    state.cases.length > 0 &&
    (!caseDir || state.cases.length > 1) &&
    !multiStation
  )
    fields.unshift({
      id: 'result-station',
      label: tr('站点', 'Station'),
      type: 'choice',
      value: caseDir,
      options: state.cases.map((entry) => ({
        value: entry.dir,
        label: entry.name || entry.dir.split(/[\\/]/).at(-1) || entry.dir,
      })),
      disabled: busy,
      set: (dir) => run(() => chooseStation(dir)),
    });
  if (!runChooser && !settingsOpen)
    fields.unshift({
      id: 'result-scope',
      label: tr('分析范围', 'Scope'),
      type: 'choice',
      value: multiStation ? 'multi' : 'single',
      options: [
        { value: 'single', label: tr('单站点', 'Single station') },
        { value: 'multi', label: tr('多站点', 'Multiple stations') },
      ],
      disabled: busy,
      set: (value) =>
        setTab(value === 'multi' ? 'comparison' : (lastStationTab.current as typeof tab)),
    });
  if (!runChooser && settingsOpen)
    fields.push({
      id: 'result-advanced',
      label: tr('高级评估设置', 'Advanced evaluation settings'),
      type: 'switch',
      value: String(advanced),
      disabled: busy,
      set: (value) => setAdvanced(value === 'true'),
    });
  if (
    !runChooser &&
    ((settingsOpen && advanced) ||
      ['series', 'evaluation', 'comparison', 'diagnostics'].includes(tab))
  )
    fields.push(
      ...[
        {
          id: 'result-from',
          label: tr('开始时间（UTC）', 'Start (UTC)'),
          value: from,
          set: setFrom,
        },
        { id: 'result-to', label: tr('结束时间（UTC）', 'End (UTC)'), value: to, set: setTo },
      ].map((f) => ({ ...f, type: 'text' as const, disabled: busy })),
    );
  if (!runChooser && !settingsOpen && (tab === 'series' || tab === 'overview'))
    fields.push({
      id: 'result-variable',
      label: tr('图表变量', 'Chart variable'),
      value: variable,
      type: 'choice',
      options: chartVariables(catalog?.variables ?? []).map((v) => ({
        value: v.name,
        label: variableOption(v, en),
      })),
      disabled: busy,
      set: setVariable,
    });
  if (!runChooser && (settingsOpen || tab === 'evaluation' || tab === 'comparison'))
    fields.push({
      id: 'result-observation',
      label: tr('观测文件', 'Observation file'),
      value: obs,
      type: 'text',
      disabled: busy,
      set: chooseObservation,
    });
  if (!runChooser && tab === 'comparison')
    fields.push({
      id: 'comparison-sources',
      label: 'Run paths | observations',
      value: comparisonText(comparisonEntries),
      type: 'text',
      disabled: busy,
      set: applyComparisonText,
    });
  if (!runChooser && ((settingsOpen && advanced) || tab === 'evaluation' || tab === 'comparison'))
    fields.push(
      {
        id: 'result-spinup',
        label: tr('跳过模型记录数', 'Skip model records'),
        value: spinup,
        type: 'number',
        disabled: busy,
        set: setSpinup,
      },
      {
        id: 'result-corrected',
        label: tr('观测能量闭合订正', 'Observation energy correction'),
        value: String(corrected),
        type: 'switch',
        disabled: busy,
        set: (v) => setCorrected(v === 'true'),
      },
    );
  if (!runChooser && tab === 'report')
    fields.push({
      id: 'result-report-format',
      label: tr('报告格式', 'Report format'),
      value: reportFormat,
      type: 'choice',
      options: [
        { value: 'json', label: 'JSON' },
        { value: 'csv', label: 'CSV' },
        { value: 'markdown', label: 'Markdown' },
        { value: 'html', label: 'HTML' },
        { value: 'pdf', label: 'PDF / Print' },
      ],
      disabled: busy,
      set: (value) => setReportFormat(value as ReportFormat),
    });
  const displayedSeries = tab === 'diagnostics' ? diagnostic?.series : series;
  usePageContract({
    title: runChooser
      ? tr('选择运行', 'Choose a run')
      : settingsOpen
        ? tr('评估设置', 'Evaluation settings')
        : tr('结果与评估', 'Results and evaluation'),
    step: 'results',
    modal: runChooser || settingsOpen,
    status: busy ? 'working' : error || 'ready',
    fields,
    actions: [],
    details: {
      source,
      provenance: selectedRun,
      catalog,
      metrics,
      diagnostic: diagnostic && {
        full_stats: diagnostic.full_stats,
        expression: diagnostic.expression,
        window: diagnostic.window,
        closure_assessed: diagnostic.closure_assessed,
        limitation: diagnostic.limitation,
      },
    },
  });
  const chart =
    displayedSeries &&
    ([displayedSeries.time, ...Object.values(displayedSeries.vars)] as uPlot.AlignedData);
  const variableInfo = catalog?.variables.find((item) => item.name === variable);
  const finding = overviewFinding(selectedMetric, variable, en);
  const validPairs = !!selectedMetric && selectedMetric.n > 0;
  const noSharedVariables =
    singleEvaluation?.key === selectionKey && !singleEvaluation.rows.length && !!obs;
  const comparisonVariables = [...new Set(metrics.map((metric) => metric.name))];
  const comparisonMetricName = comparisonVariables.includes(comparisonVariable)
    ? comparisonVariable
    : (comparisonVariables[0] ?? '');
  const comparisonRows = metrics.filter((metric) => metric.name === comparisonMetricName);
  const comparisonMax = Math.max(...comparisonRows.map((metric) => Math.abs(metric.rmse ?? 0)), 0);
  return (
    <div className="page-stack results-page">
      <header className="results-heading">
        <div>
          <h1>{tr('结果与评估', 'Results and evaluation')}</h1>
          <p>
            {caseDir.split(/[\\/]/).at(-1) || tr('未选择算例', 'No case selected')}
            {catalog
              ? ` · ${new Date(catalog.start * 1000).toISOString().slice(0, 10)} — ${new Date(catalog.end * 1000).toISOString().slice(0, 10)}`
              : ''}
          </p>
          <div className="results-badges">
            <span className="result-badge version">
              {selectedRun?.version
                ? selectedRun.version.id === 'official'
                  ? `${tr('官方版', 'Official')} · r${selectedRun.version.revision}`
                  : `${selectedRun.version.id} · r${selectedRun.version.revision}`
                : tr('版本身份未知', 'Unknown revision')}
            </span>
            {overviewRunLocation(selectedRun, en) && (
              <span className="result-badge location">
                <NavIcon name="resources" />
                {overviewRunLocation(selectedRun, en)}
              </span>
            )}
            <span
              className={`result-badge status ${selectedRun?.status === 'succeeded' && !selectedRun?.needs_reconciliation ? 'completed' : ''}`}
            >
              {overviewRunStatus(selectedRun, en)}
            </span>
          </div>
        </div>
        <ActionButton
          action="result-runs"
          label={tr('切换运行记录', 'Switch recorded run')}
          kind="click"
          disabled={busy || !runs.some((item) => item.run_case)}
          run={() => changeRunChooser(true)}
        >
          <NavIcon name="results" />
          {tr('切换运行记录', 'Switch recorded run')}
        </ActionButton>
      </header>
      {/* 分析范围（单站点/多站点）放在站点选择旁边：多站点不是页签，而是看整个项目的多站比较页。 */}
      <div className="results-scope-row">
        <div className="results-scope" role="group" aria-label={tr('分析范围', 'Scope')}>
          <span className="results-scope-label">{tr('分析范围', 'Scope')}</span>
          <div className="results-segmented">
            <button
              type="button"
              className={`results-scope-option${multiStation ? '' : ' is-active'}`}
              aria-pressed={!multiStation}
              disabled={busy}
              onClick={() => setTab(lastStationTab.current)}
            >
              {tr('单站点', 'Single station')}
            </button>
            <button
              type="button"
              className={`results-scope-option${multiStation ? ' is-active' : ''}`}
              aria-pressed={multiStation}
              disabled={busy}
              onClick={() => setTab('comparison')}
            >
              {tr('多站点', 'Multiple stations')}
            </button>
          </div>
        </div>
        {!multiStation && state.cases.length > 0 && (!caseDir || state.cases.length > 1) && (
          <Field label={tr('站点', 'Station')}>
            <Select
              value={caseDir}
              disabled={busy}
              onChange={(event) => run(() => chooseStation(event.target.value))}
            >
              {!caseDir && (
                <option value="" disabled>
                  {tr('请选择站点', 'Choose a station')}
                </option>
              )}
              {state.cases.map((entry) => (
                <option key={entry.dir} value={entry.dir}>
                  {entry.name || entry.dir.split(/[\\/]/).at(-1) || entry.dir}
                </option>
              ))}
            </Select>
          </Field>
        )}
      </div>
      {/* 工具条：单站点时是站点内页签 + 页面动作；多站点时是"多站点比较"标题 + 返回类动作。
          「模型改进」只留在结果总览的"下一步可以做什么"卡片里，不再和"返回总览/返回模拟"并列。 */}
      <div className="results-toolbar">
        {multiStation ? (
          <h2 className="results-scope-heading">
            {tr('多站点比较', 'Multiple-station comparison')}
          </h2>
        ) : (
          <nav
            className="results-tabs"
            role="tablist"
            aria-label={tr('当前站点的分析视图', 'Views for the selected station')}
          >
            {stationTabs.map((name) => (
              <button
                key={name}
                type="button"
                role="tab"
                aria-selected={tab === name}
                className={`results-tab${tab === name ? ' is-active' : ''}`}
                disabled={busy}
                onClick={() => setTab(name)}
              >
                {en ? tabLabels.en[tabIndex(name)] : tabLabels.zh[tabIndex(name)]}
              </button>
            ))}
          </nav>
        )}
        <div className="results-actions">
          {tab !== 'overview' && (
            <ActionButton
              action="result-overview"
              label={tr('返回结果总览', 'Back to results overview')}
              kind="click"
              variant="ghost"
              disabled={busy}
              run={() => setTab('overview')}
            >
              {tr('← 结果总览', '← Overview')}
            </ActionButton>
          )}
          {!multiStation && tab === 'overview' && (
            <ActionButton
              action="export-full-series-csv"
              label={tr('导出完整时序 CSV', 'Export full time-series CSV')}
              kind="commit"
              variant="ghost"
              disabled={!source || !variable || busy}
              run={exportFullSeries}
            >
              {tr('导出完整 CSV', 'Export full CSV')}
            </ActionButton>
          )}
          {onBack && (
            <ActionButton
              action="result-back"
              label={tr('返回模拟', 'Back to simulation')}
              kind="click"
              variant="ghost"
              run={onBack}
            >
              {tr('返回模拟', 'Back to simulation')}
            </ActionButton>
          )}
        </div>
      </div>
      {awaitingFetch && (
        <Notice>
          {tr(
            `数据在服务器 ${selectedRun?.server ?? ''} 上：变量、曲线和评估指标都在服务器上计算，只传回要显示的数据，每次几秒。要 NetCDF 文件时，再取回这一个站点、这一次运行的结果（约 0.5 GB）。`,
            `Data stays on ${selectedRun?.server ?? ''}: variables, curves and evaluation metrics are computed there and only the displayed values come back, a few seconds each. Fetch this station's run (about 0.5 GB) when you need the NetCDF files.`,
          )}
          <ActionButton
            action="result-fetch"
            label={tr('取回结果', 'Fetch results')}
            kind="commit"
            disabled={busy}
            run={fetchSelectedRun}
          >
            {fetching ? tr('正在取回…', 'Fetching…') : tr('取回结果', 'Fetch results')}
          </ActionButton>
        </Notice>
      )}
      {source && !selectedRun?.version && (
        <Notice>
          {tr(
            '历史结果缺少版本出处，不能用于当前修订的改进结论。',
            'This legacy result cannot establish improvement for the selected revision.',
          )}
        </Notice>
      )}
      {stationError && <Notice tone="error">{stationError}</Notice>}
      {error && !settingsOpen && <Notice tone="error">{error}</Notice>}
      {noSharedVariables && (
        <Notice>
          {tr(
            '观测数据已载入，但本次模拟输出与观测没有共同变量。请在模型改进中检查过程和输出设置。',
            'Observations are loaded, but this run has no shared output variables. Check process and output settings under Improve model.',
          )}
        </Notice>
      )}
      {exportNotice && <Notice>{exportNotice}</Notice>}
      {!source && (
        <Notice>
          {tr('尚无已记录的运行。请先运行当前算例。', 'No recorded run. Run this case first.')}
        </Notice>
      )}
      {tab === 'overview' && (
        <section className="results-overview">
          <section className="results-evidence" aria-label={tr('结果摘要', 'Results summary')}>
            <div>
              <svg
                viewBox="0 0 24 24"
                aria-hidden="true"
                fill="none"
                stroke="currentColor"
                strokeWidth="1.6"
              >
                <path d="M9 18h6M10 21h4M8 14a6 6 0 118 0l-1 3H9zM12 1v2M3 8H1M23 8h-2" />
              </svg>
              <div>
                <strong>
                  {validPairs
                    ? tr('已确认的现象', 'Confirmed finding')
                    : tr('当前结果', 'Current result')}
                </strong>
                <p>{finding}</p>
              </div>
            </div>
            <div>
              <NavIcon name="results" />
              <div>
                <strong>
                  {validPairs
                    ? tr('有效观测已匹配', 'Valid observations matched')
                    : noSharedVariables
                      ? tr('没有共同评估变量', 'No shared evaluation variables')
                      : tr('等待观测评估', 'Observation evaluation pending')}
                </strong>
                <p>
                  {validPairs
                    ? tr(
                        `基于 ${selectedMetric!.n} 组实际有效配对。`,
                        `${selectedMetric!.n} actual valid pairs.`,
                      )
                    : noSharedVariables
                      ? tr(
                          '观测文件已载入，当前无法计算配对指标。',
                          'The observation file is loaded; paired metrics are unavailable.',
                        )
                      : tr(
                          '选择观测后，再判断模拟与观测的差异。',
                          'Select observations to evaluate the differences.',
                        )}
                </p>
              </div>
            </div>
          </section>
          <Card className="results-plot-card">
            <div className="results-plot-heading">
              <h2>{tr('模拟与观测', 'Simulation and observations')}</h2>
              <div className="results-plot-tools">
                <Select
                  aria-label={tr('图表变量', 'Chart variable')}
                  value={variable}
                  disabled={busy || !catalog}
                  onChange={(event) => setVariable(event.target.value)}
                >
                  {chartVariables(catalog?.variables ?? []).map((item) => (
                    <option key={item.name} value={item.name}>
                      {variableOption(item, en)}
                    </option>
                  ))}
                </Select>
                <ActionButton
                  action="result-settings"
                  label={tr('评估设置', 'Evaluation settings')}
                  kind="click"
                  variant="ghost"
                  disabled={!catalog || busy}
                  run={() => changeSettings(true)}
                >
                  <NavIcon name="settings" />
                  {tr('评估设置', 'Evaluation settings')}
                </ActionButton>
              </div>
            </div>
            <div className="results-plot-meta">
              <span>
                {tr('单位', 'Units')}: {selectedMetric?.units || variableInfo?.units || '—'}
                {variableInfo?.kind === 'grid'
                  ? tr(
                      ' · 区域平均（按纬度加权，跳过缺测）',
                      ' · area mean (latitude-weighted, missing cells skipped)',
                    )
                  : ''}
              </span>
              <div className="results-legend">
                {validPairs && <span className="observed">{tr('观测', 'Observation')}</span>}
                <span className="simulated">{tr('模拟', 'Simulation')}</span>
              </div>
            </div>
            {selectedMetric?.time?.length && selectedMetric.model && selectedMetric.obs ? (
              <ScienceChart
                compact
                paired
                data={
                  [
                    selectedMetric.time,
                    selectedMetric.model,
                    selectedMetric.obs,
                  ] as uPlot.AlignedData
                }
                labels={[tr('模拟', 'Simulation'), tr('观测', 'Observation')]}
                theme={state.theme}
              />
            ) : chart && seriesKey === selectedSeriesKey ? (
              <ScienceChart
                compact
                data={chart}
                labels={[tr('模拟', 'Simulation')]}
                theme={state.theme}
              />
            ) : (
              <div className="results-chart-empty">
                {source
                  ? tr('正在读取真实模拟结果…', 'Reading simulation results…')
                  : tr(
                      '运行完成后，在这里查看曲线。',
                      'The simulation curve appears here after a run.',
                    )}
              </div>
            )}
            {isGridCatalog(catalog?.variables ?? []) && (
              <p className="results-sampling">
                {tr(
                  '区域模拟：曲线是整个区域的平均，下面的空间分布图可以看每个格点；观测评估按站点做，这里不配对。',
                  'Regional run: the curve is the area mean and the map below shows each cell; observation evaluation is per site, so nothing is paired here.',
                )}
              </p>
            )}
            {(selectedMetric?.pair_downsampled || series?.downsampled) && (
              <p className="results-sampling">
                {tr(
                  '曲线显示抽样点，指标使用全部有效配对。',
                  'Curves show sampled points; metrics use all valid pairs.',
                )}
              </p>
            )}
            <div className="results-metrics">
              {[
                [
                  tr('平均偏差', 'Mean bias'),
                  number(selectedMetric?.bias, 1),
                  selectedMetric?.units || variableInfo?.units,
                ],
                [
                  tr('RMSE', 'RMSE'),
                  number(selectedMetric?.rmse, 1),
                  selectedMetric?.units || variableInfo?.units,
                ],
                [tr('有效配对', 'Valid pairs'), selectedMetric?.n ?? '—', ''],
              ].map(([label, value, unit]) => (
                <div key={String(label)}>
                  <span>{label}</span>
                  <strong>
                    {value} <small>{unit}</small>
                  </strong>
                </div>
              ))}
            </div>
            {!!selectedMetric?.beta_warning && (
              <Notice tone="warning">{String(selectedMetric.beta_warning)}</Notice>
            )}
          </Card>
          {variableInfo?.kind === 'grid' && source && catalog && (
            <Card className="results-map-card">
              <GridMap
                source={source}
                variable={variable}
                title={variableOption(variableInfo, en)}
                steps={catalog.steps}
                en={en}
              />
            </Card>
          )}
          <Card className="results-next">
            <div>
              <h2>{tr('下一步可以做什么', 'Next steps')}</h2>
              <p>{tr('候选原因需要进一步验证。', 'Candidate causes need further validation.')}</p>
            </div>
            <div>
              <ActionButton
                action="result-closure"
                label={tr('诊断偏差', 'Diagnose deviations')}
                kind="click"
                disabled={busy}
                run={() => setTab('diagnostics')}
              >
                <NavIcon name="simulation" />
                {tr('诊断偏差', 'Diagnose deviations')}
              </ActionButton>
              <ActionButton
                action="result-improve"
                label={tr('模型改进', 'Improve model')}
                kind="click"
                disabled={busy}
                run={improveModel}
              >
                <NavIcon name="settings" />
                {tr('模型改进', 'Improve model')}
              </ActionButton>
            </div>
          </Card>
        </section>
      )}
      <Dialog
        title={tr('切换运行记录', 'Switch recorded run')}
        description={tr(
          '选一条运行记录看它的结果。要比较多条运行，请进入"多站点"范围后勾选。',
          'Choose which recorded run the page shows. To compare several runs, switch the scope to "Multiple stations" and tick them.',
        )}
        open={runChooser}
        onOpenChange={changeRunChooser}
        closeAction="result-runs-close"
      >
        <div className="result-run-choices">
          {runs
            .filter((item) => item.run_case)
            .map((item, index) => (
              <ActionButton
                key={index}
                action={`result-run-${index}`}
                label={`${overviewRunLabel(item, en)} · ${overviewRunStatus(item, en)}`}
                kind="click"
                disabled={busy}
                aria-pressed={source === item.run_case}
                run={() => chooseRecordedRun(item.run_case!)}
              >
                <strong>{overviewRunLabel(item, en)}</strong>
                <span>{overviewRunStatus(item, en)}</span>
              </ActionButton>
            ))}
        </div>
        <ActionButton
          action="result-run-comparison"
          label={tr('比较多选运行', 'Compare selected runs')}
          kind="click"
          disabled={busy}
          run={() => {
            setRunChooser(false);
            setTab('comparison');
          }}
        >
          {tr('比较多选运行', 'Compare selected runs')}
        </ActionButton>
      </Dialog>
      <Dialog
        title={tr('评估设置', 'Evaluation settings')}
        description={tr(
          '选择观测与评估条件，计算后会更新当前图表和指标。',
          'Choose observations and conditions; evaluation updates the chart and metrics.',
        )}
        open={settingsOpen}
        onOpenChange={changeSettings}
        closeAction="result-settings-close"
      >
        {error && <Notice tone="error">{error}</Notice>}
        <Field label={tr('观测文件', 'Observation file')}>
          <div className="browse-input">
            <Input
              value={obs}
              disabled={busy}
              onChange={(event) => chooseObservation(event.target.value)}
            />
            <ActionButton
              action="result-pick-observation"
              label={tr('选择文件', 'Choose a file')}
              kind="click"
              disabled={busy}
              run={async () => {
                if (selectedRun?.server) {
                  setObservationBrowser(true);
                  return null;
                }
                const file = await invoke<string | null>('pick_file', {
                  key: 'result-observations',
                  filter: 'nc',
                });
                if (file) chooseObservation(file);
                return file;
              }}
            >
              {tr('选择文件', 'Choose a file')}
            </ActionButton>
            {observationBrowser && selectedRun?.server && (
              <RemotePathBrowser
                host={selectedRun.server}
                initialPath={obs.includes('/') ? obs.slice(0, obs.lastIndexOf('/')) : undefined}
                directory={false}
                en={en}
                onSelect={(path) => {
                  chooseObservation(path);
                  setObservationBrowser(false);
                }}
                onClose={() => setObservationBrowser(false)}
              />
            )}
          </div>
        </Field>
        <Advanced
          title={tr('高级评估设置', 'Advanced evaluation settings')}
          action="result-advanced-settings"
          actionLabel={tr('高级评估设置', 'Advanced evaluation settings')}
          open={advanced}
          onOpenChange={setAdvanced}
        >
          <div className="form-grid">
            <Field label={tr('开始时间（UTC）', 'Start (UTC)')}>
              <Input
                type="datetime-local"
                value={from}
                disabled={busy}
                onChange={(event) => setFrom(event.target.value)}
              />
            </Field>
            <Field label={tr('结束时间（UTC）', 'End (UTC)')}>
              <Input
                type="datetime-local"
                value={to}
                disabled={busy}
                onChange={(event) => setTo(event.target.value)}
              />
            </Field>
          </div>
          <Field label={tr('跳过模型记录数', 'Skip model records')}>
            <Input
              type="number"
              min={0}
              value={spinup}
              disabled={busy}
              onChange={(event) => setSpinup(event.target.value)}
            />
          </Field>
          <label>
            <input
              type="checkbox"
              checked={corrected}
              disabled={busy}
              onChange={(event) => setCorrected(event.target.checked)}
            />
            {tr('观测能量闭合订正', 'Correct observation energy closure')}
          </label>
        </Advanced>
        <div className="actions">
          <ActionButton
            action="evaluate-observations"
            label={tr('更新评估', 'Update evaluation')}
            kind="click"
            variant="primary"
            disabled={!source || !obs.trim() || busy}
            run={evaluate}
          >
            {busy ? tr('正在评估…', 'Evaluating…') : tr('更新评估', 'Update evaluation')}
          </ActionButton>
          <ActionButton
            action="plot-series"
            label={tr('刷新模拟曲线', 'Refresh simulation curve')}
            kind="click"
            disabled={!source || !variable || busy}
            run={plot}
          >
            {tr('刷新模拟曲线', 'Refresh simulation curve')}
          </ActionButton>
        </div>
      </Dialog>
      <details className="results-advanced">
        <summary>
          {tr('版本出处与科学比较', 'Version provenance and scientific comparison')}
        </summary>
        <Card title={tr('与来源版本比较', 'Comparison with the parent revision')}>
          <p>
            {baselineNote === 'official'
              ? tr('此版本没有来源版本。', 'This version has no parent revision.')
              : baselineNote === 'missing'
                ? tr(
                    '没有来源修订的成功运行；不会自动追加计算。',
                    'No successful run of the parent revision. No simulation will be started.',
                  )
                : baselineNote === 'observation'
                  ? tr(
                      '已找到来源修订结果；请在观测评估或比较页填写同一观测文件。',
                      'A parent run exists. Enter the shared observation file under Observations or Sites.',
                    )
                  : baselineNote ||
                    (!baseline
                      ? tr('选择有版本出处的运行。', 'Select a run with version provenance.')
                      : '')}
          </p>
          {baseline && (
            <>
              <Notice>
                {baseline.same_design
                  ? tr(
                      '输入、模拟时段、观测与有效配对均由后端核对；A 为当前运行，B 为来源运行。',
                      'The backend verifies inputs, simulation period, observations and valid pairs. A is this run; B is the parent.',
                    )
                  : tr(
                      '条件不足，不能解释版本优劣。',
                      'The conditions do not establish version superiority.',
                    )}
              </Notice>
              {baseline.notes?.map((note, i) => (
                <p key={i}>{note}</p>
              ))}
              {baseline.same_design && (
                <table>
                  <thead>
                    <tr>
                      <th>{tr('变量', 'Variable')}</th>
                      <th>A RMSE</th>
                      <th>B RMSE</th>
                      <th>{tr('配对一致', 'Same pairs')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {baseline.variables?.map((item) => (
                      <tr key={item.variable}>
                        <td>{item.variable}</td>
                        <td>
                          {number(baseline.metrics_a?.find((m) => m.name === item.variable)?.rmse)}
                        </td>
                        <td>
                          {number(baseline.metrics_b?.find((m) => m.name === item.variable)?.rmse)}
                        </td>
                        <td>{String(item.same_pairs)}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
            </>
          )}
        </Card>
      </details>
      {tab === 'catalog' && catalog && (
        <Card title={tr('History 数据目录', 'History catalog')}>
          <p>
            {tr(
              `${catalog.files} 个文件 · ${catalog.steps} 个时间步`,
              `${catalog.files} files · ${catalog.steps} steps`,
            )}{' '}
            · {new Date(catalog.start * 1000).toISOString()} —{' '}
            {new Date(catalog.end * 1000).toISOString()}
          </p>
          <table>
            <thead>
              <tr>
                <th>{tr('变量', 'Variable')}</th>
                <th>{tr('说明', 'Description')}</th>
                <th>{tr('单位', 'Units')}</th>
                <th>{tr('维度', 'Shape')}</th>
              </tr>
            </thead>
            <tbody>
              {catalog.variables.map((v) => (
                <tr key={v.name}>
                  <td>{v.name}</td>
                  <td>{overviewVariableLabel(v.name, en, v.long_name)}</td>
                  <td>{v.units || '—'}</td>
                  <td>{JSON.stringify(v.dimensions)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {multiStation && (
        <Card>
          <div className="results-comparison-picker">
            <div>
              <h2>{tr('选择要比较的运行', 'Choose runs to compare')}</h2>
              <p className="muted">
                {tr(
                  '按站点分组列出项目里成功的运行；勾选后可以逐个改观测文件，也可先填一个共用观测再应用到所有行。',
                  'Successful runs in this project, grouped by station. Tick the runs, then edit each observation or fill one shared one and apply it to all rows.',
                )}
              </p>
            </div>
            {projectRunsLoading && (
              <p role="status">{tr('正在读取项目里的运行记录…', 'Reading recorded runs…')}</p>
            )}
            {projectRunsError && <Notice tone="error">{projectRunsError}</Notice>}
            {!projectRunsLoading && !projectRunsError && !projectRuns.length && (
              <Notice>
                {tr(
                  '项目里还没有成功的运行记录：先跑完至少一个站点再来比较。',
                  'No successful run in this project yet; finish at least one station first.',
                )}
              </Notice>
            )}
            {!projectRunsLoading && !!projectRuns.length && (
              <div className="comparison-run-filters">
                <Input
                  aria-label={tr('搜索运行', 'Search runs')}
                  placeholder={tr(
                    '搜索站点 / 运行编号 / 版本…',
                    'Search station, run id or version…',
                  )}
                  value={comparisonQuery}
                  disabled={busy}
                  onChange={(event) => setComparisonQuery(event.target.value)}
                />
                <label>
                  <input
                    type="checkbox"
                    checked={comparisonServerOnly}
                    disabled={busy}
                    onChange={(event) => setComparisonServerOnly(event.target.checked)}
                  />
                  {tr('只看结果还在服务器上的', 'Only runs whose outputs are still on the server')}
                </label>
                <small>
                  {tr(
                    `显示 ${visibleProjectRuns(projectRuns).length} / ${projectRuns.length} 条运行`,
                    `${visibleProjectRuns(projectRuns).length} of ${projectRuns.length} runs shown`,
                  )}
                </small>
              </div>
            )}
            {!projectRunsLoading && !!projectRuns.length && (
              <div className="comparison-run-groups">
                {groupedProjectRuns(visibleProjectRuns(projectRuns)).map((group) => (
                  <section key={group.dir} className="comparison-run-group">
                    <h3>
                      {group.caseName}
                      <small>{group.dir}</small>
                    </h3>
                    <ul>
                      {group.runs.map((run) => {
                        const checked = comparisonEntries.some(
                          (entry) => entry.run.run_case === run.run_case,
                        );
                        return (
                          <li key={run.run_case}>
                            <label className="comparison-run-option">
                              <input
                                type="checkbox"
                                checked={checked}
                                disabled={busy}
                                onChange={() => toggleComparisonRun(run)}
                              />
                              <span>
                                <strong>{overviewRunLabel(run, en)}</strong>
                                <small>{comparisonRunFact(run)}</small>
                              </span>
                            </label>
                          </li>
                        );
                      })}
                    </ul>
                  </section>
                ))}
              </div>
            )}
            <Field
              label={tr(
                '从其它项目添加运行（填运行目录，如 /path/站点/runs/<编号>；按目录反推算例，不扫描项目）',
                'Add a run from another project (run folder, e.g. /path/station/runs/<id>; the case is inferred from the path)',
              )}
            >
              <div className="browse-input">
                <Input
                  value={externalRun}
                  disabled={busy}
                  placeholder={tr('运行目录', 'Run folder')}
                  onChange={(e) => setExternalRun(e.target.value)}
                />
                <ActionButton
                  action="comparison-add-external"
                  label={tr('添加', 'Add')}
                  disabled={!externalRun.trim() || busy}
                  run={addExternalComparisonRun}
                />
              </div>
            </Field>
            <Field
              label={tr(
                '共用观测文件（选一次可应用到所有行；服务器项目的观测填服务器路径）',
                'Shared observation file (choose once, apply to all rows; server projects need a server path)',
              )}
            >
              <div className="browse-input">
                <Input
                  value={obs}
                  disabled={busy}
                  onChange={(e) => {
                    epoch.current++;
                    setMetrics([]);
                    chooseObservation(e.target.value);
                  }}
                />
                <ActionButton
                  action="comparison-apply-observation"
                  label={tr('应用到所有行', 'Apply to all rows')}
                  disabled={!comparisonEntries.length || !obs.trim() || busy}
                  run={applySharedObservation}
                />
              </div>
            </Field>
            {!!comparisonEntries.length && (
              <div className="comparison-selected">
                <h3>
                  {tr(
                    `已选 ${comparisonEntries.length} 个运行`,
                    `${comparisonEntries.length} run(s) selected`,
                  )}
                </h3>
                <ul>
                  {comparisonEntries.map((entry, index) => (
                    <li key={entry.run.run_case}>
                      <span>
                        <strong>{overviewRunLabel(entry.run, en)}</strong>
                        <small>{entry.run.case_name || entry.run.case_dir}</small>
                      </span>
                      <Input
                        aria-label={tr('观测文件', 'Observation file')}
                        value={entry.obs}
                        disabled={busy}
                        onChange={(e) => setComparisonEntryObs(index, e.target.value)}
                      />
                      <ActionButton
                        action={`comparison-remove-${index}`}
                        label={tr('移除', 'Remove')}
                        disabled={busy}
                        run={() => removeComparisonEntry(index)}
                      />
                    </li>
                  ))}
                </ul>
              </div>
            )}
            <Notice>
              {tr(
                '相同时段和评估设置下展示指标；不同输入不解释版本优劣。',
                'Metrics use the selected evaluation window; different inputs do not establish version superiority.',
              )}
            </Notice>
            {comparisonProgress && (
              <p role="status">
                {tr(
                  `正在计算指标 ${comparisonProgress.done}/${comparisonProgress.total}…`,
                  `Computing metrics ${comparisonProgress.done}/${comparisonProgress.total}…`,
                )}
              </p>
            )}
            <ActionButton
              action="compare-sites"
              label={tr('比较多选运行', 'Compare selected runs')}
              kind="click"
              disabled={!comparisonEntries.length || busy}
              run={compare}
            >
              {tr(
                `比较选中的 ${comparisonEntries.length} 个运行`,
                `Compare ${comparisonEntries.length} selected run(s)`,
              )}
            </ActionButton>
          </div>
        </Card>
      )}
      {['series', 'evaluation', 'comparison', 'diagnostics'].includes(tab) && (
        <Card>
          <div className="grid gap-3 md:grid-cols-2">
            <Field label={tr('开始时间（UTC）', 'Start (UTC)')}>
              <Input
                type="datetime-local"
                value={from}
                onChange={(e) => {
                  epoch.current++;
                  setSeries(null);
                  setMetrics([]);
                  setFrom(e.target.value);
                }}
              />
            </Field>
            <Field label={tr('结束时间（UTC）', 'End (UTC)')}>
              <Input
                type="datetime-local"
                value={to}
                onChange={(e) => {
                  epoch.current++;
                  setSeries(null);
                  setMetrics([]);
                  setTo(e.target.value);
                }}
              />
            </Field>
          </div>
          {tab === 'series' && (
            <Field label={tr('变量', 'Variable')}>
              <Select
                value={variable}
                onChange={(e) => {
                  setVariable(e.target.value);
                  setSeries(null);
                }}
              >
                {catalog?.variables
                  .filter((v) => v.kind === 'series' && v.name !== 'time')
                  .map((v) => (
                    <option key={v.name} value={v.name}>
                      {v.name} [{v.units}]
                    </option>
                  ))}
              </Select>
            </Field>
          )}
          {tab === 'evaluation' && (
            <Field
              label={tr(
                '来源版本比较使用的观测文件',
                'Observation file for parent-version comparison',
              )}
            >
              <Input
                value={obs}
                onChange={(e) => {
                  epoch.current++;
                  setMetrics([]);
                  chooseObservation(e.target.value);
                }}
              />
            </Field>
          )}
          {(tab === 'evaluation' || tab === 'comparison') && (
            <div className="flex gap-4">
              <Field label={tr('跳过模型记录数（不是天数）', 'Skip model records (not days)')}>
                <Input
                  type="number"
                  min={0}
                  value={spinup}
                  onChange={(e) => setSpinup(e.target.value)}
                />
              </Field>
              <label>
                <input
                  type="checkbox"
                  checked={corrected}
                  onChange={(e) => setCorrected(e.target.checked)}
                />
                {tr('观测能量闭合订正', 'Correct observation energy closure')}
              </label>
            </div>
          )}
          {tab === 'series' && (
            <>
              <ActionButton
                action="plot-series"
                label={tr('绘制时序', 'Plot series')}
                kind="click"
                disabled={!source || !variable || busy}
                run={plot}
              >
                {tr('绘制时序', 'Plot series')}
              </ActionButton>
              <ActionButton
                action="export-full-series-csv"
                label={tr('导出完整时序 CSV', 'Export full time-series CSV')}
                kind="commit"
                disabled={!source || !variable || busy}
                run={exportFullSeries}
              >
                {tr('导出完整时序 CSV', 'Export full time-series CSV')}
              </ActionButton>
            </>
          )}
          {tab === 'evaluation' && (
            <ActionButton
              action="evaluate-observations"
              label={tr('评估观测', 'Evaluate observations')}
              kind="click"
              disabled={!source || !obs || busy}
              run={evaluate}
            >
              {tr('评估观测', 'Evaluate observations')}
            </ActionButton>
          )}
          {tab === 'diagnostics' && (
            <ActionButton
              action="diagnose-closure"
              label={tr('统计能量通量残差', 'Summarize energy flux residual')}
              kind="click"
              disabled={!catalog || busy}
              run={diagnose}
            >
              {tr('统计能量通量残差', 'Summarize energy flux residual')}
            </ActionButton>
          )}
          {(tab === 'series' || tab === 'diagnostics') && displayedSeries && (
            <ActionButton
              action="export-series-data"
              label={tr('导出显示数据（抽样 JSON）', 'Export displayed data (sampled JSON)')}
              kind="commit"
              disabled={busy}
              run={() =>
                task(async (current) => {
                  setExportNotice('');
                  const result = await download(
                    'series-sampled.json',
                    JSON.stringify({ source, ...displayedSeries }),
                  );
                  if (current()) setExportNotice(exportMessage(result));
                  return result;
                })
              }
            >
              {tr('导出显示数据（抽样 JSON）', 'Export displayed data (sampled JSON)')}
            </ActionButton>
          )}
          {tab === 'diagnostics' && (
            <p>
              {tr(
                'Rnet − Qh − Qle − Qg；统计使用所选窗口的全部共同有效记录，不补零；起止标签均包含在内，沿用 History 记录时钟，图表最多显示 2400 个抽样点。该残差未计储热等项，不能据此判定模型收支闭合通过。',
                'Rnet − Qh − Qle − Qg. Statistics use all complete records without zero-filling. Both endpoint labels are included using the history clock; the plot shows at most 2400 sampled points. Storage and other terms are excluded, so this does not establish model budget closure.',
              )}
            </p>
          )}
          {diagnostic && tab === 'diagnostics' && (
            <>
              <p>
                {tr('完整记录', 'Full records')}={diagnostic.full_stats.source_n} · n=
                {diagnostic.full_stats.n} · {tr('缺测', 'missing')}={diagnostic.full_stats.missing}{' '}
                · {tr('有效记录算术平均', 'Arithmetic mean of complete records')}=
                {number(diagnostic.full_stats.mean)} {diagnostic.full_stats.units}
              </p>
              {diagnostic.area_mean && (
                <p className="muted">
                  {tr(
                    '区域模拟：每一步先逐格计算残差（四项通量都有值的格子），再按纬度加权取区域平均。',
                    'Regional run: each step takes the residual cell by cell (cells with all four fluxes), then the latitude-weighted area mean.',
                  )}
                </p>
              )}
              {diagnostic.full_stats.status === 'unavailable' && (
                <Notice>
                  {tr(
                    '没有完整有效的四项通量记录，无法计算均值；缺值没有补零。',
                    'No complete finite flux tuples are available. The mean cannot be calculated; missing values remain missing.',
                  )}
                </Notice>
              )}
            </>
          )}
          {chart && (tab === 'series' || tab === 'diagnostics') && (
            <>
              <ScienceChart
                data={chart}
                labels={Object.keys(displayedSeries!.vars)}
                theme={state.theme}
              />
              <p>
                {tr(
                  `${displayedSeries!.n}/${displayedSeries!.source_n} 个点${displayedSeries!.downsampled ? '（已抽样）' : ''}`,
                  `${displayedSeries!.n}/${displayedSeries!.source_n} points${displayedSeries!.downsampled ? ' (sampled)' : ''}`,
                )}
              </p>
            </>
          )}
        </Card>
      )}
      {(tab === 'evaluation' || tab === 'comparison') && (
        <Card title={tr('有效配对指标', 'Paired metrics')}>
          {tab === 'comparison' && (
            <div className="results-comparison-conditions">
              <p>
                {tr(
                  `比较 ${comparisonEntries.length} 个运行；评估时段 ${from || '全部'} — ${to || '全部'}，跳过 ${spinup} 条记录，观测闭合订正：${corrected ? '是' : '否'}。`,
                  `Comparing ${comparisonEntries.length} run(s); window ${from || 'all'} — ${to || 'all'}, skip ${spinup} records, observation energy correction: ${corrected ? 'yes' : 'no'}.`,
                )}
              </p>
              <ActionButton
                action="export-comparison-csv"
                label={tr('导出比较指标 CSV', 'Export comparison metrics as CSV')}
                kind="commit"
                disabled={!metrics.length || busy}
                run={exportComparison}
              >
                {tr('导出比较指标 CSV', 'Export comparison CSV')}
              </ActionButton>
            </div>
          )}
          <table>
            <thead>
              <tr>
                {[
                  tr('站点', 'Site'),
                  ...(tab === 'comparison' ? [tr('观测文件', 'Observation')] : []),
                  tr('变量', 'Variable'),
                  tr('配对数', 'n'),
                  'RMSE',
                  'MAE',
                  tr('偏差', 'Bias'),
                  'R²',
                  'r',
                  'NSE',
                  'KGE',
                ].map((s) => (
                  <th key={s}>{s}</th>
                ))}
              </tr>
            </thead>
            <tbody>
              {metrics.map((m, i) => (
                <tr key={i}>
                  {/* 多运行比较的站点是运行目录（<算例>/runs/<编号>），整条路径占满一列：显示算例名和编号前 8 位。 */}
                  <td title={m.site || source}>{runLabel(m.site || source)}</td>
                  {tab === 'comparison' && (
                    <td title={m.comparison_obs}>
                      {m.comparison_obs?.split(/[\\/]/).at(-1) || '—'}
                    </td>
                  )}
                  <td>{m.name}</td>
                  <td>{m.n}</td>
                  {[m.rmse, m.mae, m.bias, m.r2, m.correlation, m.nse, m.kge].map((v, j) => (
                    <td key={j}>{number(v)}</td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
          {metrics
            .filter((m) => m.beta_warning)
            .map((m, i) => (
              <Notice key={i} tone="warning">
                {m.name}: {String(m.beta_warning)}
              </Notice>
            ))}
          {!metrics.length && (
            <p>
              {tr(
                '尚无有效指标；缺少观测时仅可分析模拟。',
                'No valid metrics yet. Observations are required.',
              )}
            </p>
          )}
          {evaluation
            .filter((v) => !v.available)
            .map((v) => (
              <p key={v.name}>
                {v.name}:{' '}
                {v.reason ||
                  [...(v.missing_model || []), ...(v.missing_observation || [])].join(', ') ||
                  tr('不可配对', 'Unavailable')}
              </p>
            ))}
          {tab === 'evaluation' &&
            metrics
              .filter((m) => m.time?.length && m.model && m.obs)
              .map((m) => (
                <ScienceChart
                  key={m.name}
                  data={[m.time!, m.model!, m.obs!] as uPlot.AlignedData}
                  labels={[`${m.name} model`, `${m.name} obs`]}
                  theme={state.theme}
                />
              ))}
        </Card>
      )}
      {tab === 'comparison' && !!comparisonRows.length && (
        <Card title={tr('站间指标对比', 'Metrics across stations')}>
          <p className="muted">
            {tr(
              '同一变量、同一 RMSE 按站点并排；条形长度按本次比较里的最大 RMSE 归一化（数值越小越好）。不同输入不解释版本优劣。',
              'The same variable and RMSE side by side per station; bars are scaled to the largest RMSE in this comparison (lower is better). Different inputs do not establish version superiority.',
            )}
          </p>
          <Field label={tr('对比变量', 'Variable to compare')}>
            <Select
              value={comparisonMetricName}
              disabled={busy}
              onChange={(event) => setComparisonVariable(event.target.value)}
            >
              {comparisonVariables.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </Select>
          </Field>
          <div
            className="comparison-bars"
            role="img"
            aria-label={tr('各站点 RMSE 对比', 'RMSE by station')}
          >
            {comparisonRows.map((metric) => {
              const width = comparisonMax
                ? Math.max(1, Math.round((Math.abs(metric.rmse ?? 0) / comparisonMax) * 100))
                : 0;
              return (
                <div key={metric.site} className="comparison-bar-row">
                  <span className="comparison-bar-label" title={metric.site}>
                    {runLabel(metric.site || '')}
                  </span>
                  <div className="comparison-bar-track">
                    <div className="comparison-bar-fill" style={{ width: `${width}%` }} />
                  </div>
                  <span className="comparison-bar-value">{number(metric.rmse)}</span>
                </div>
              );
            })}
          </div>
        </Card>
      )}
      {tab === 'report' && (
        <Card title={tr('可追溯报告', 'Traceable report')}>
          <p>
            {tr(
              '导出目录、真实指标、诊断、出处与评估条件。没有来源基准时不声称改进。',
              'Export catalog, measured metrics, diagnostics, provenance and evaluation settings. No improvement claim without a valid baseline.',
            )}
          </p>
          <Field label={tr('报告格式', 'Report format')}>
            <Select
              value={reportFormat}
              disabled={busy}
              onChange={(event) => setReportFormat(event.target.value as ReportFormat)}
            >
              {fields
                .find((field) => field.id === 'result-report-format')
                ?.options?.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
            </Select>
          </Field>
          <ActionButton
            action="export-report"
            label={
              reportFormat === 'pdf'
                ? tr('打印 / 另存为 PDF', 'Print / Save as PDF')
                : tr('导出报告', 'Export report')
            }
            kind="commit"
            disabled={!catalog || busy}
            run={report}
          >
            {reportFormat === 'pdf'
              ? tr('打印 / 另存为 PDF', 'Print / Save as PDF')
              : tr('导出报告', 'Export report')}
          </ActionButton>
          <pre className="overflow-auto">
            {JSON.stringify(
              runs.find((r) => r.run_case === source) || { run_case: source },
              null,
              2,
            )}
          </pre>
        </Card>
      )}
    </div>
  );
}
