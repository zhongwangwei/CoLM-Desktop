import { useCallback, useEffect, useRef, useState } from 'react';
import uPlot from 'uplot';
import { timeTicks } from './time-ticks';
import { utcSeconds } from '../../lib/dates';
import 'uplot/dist/uPlot.min.css';
import { hasBackend, invoke, invokeJson } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiAction, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Select, Notice, Advanced, Dialog } from '../../components/ui';
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
import NavIcon from '../../components/NavIcon';
import { defaultObservation } from './default-observation';
import './results-page.css';
type Catalog = {
  files: number;
  steps: number;
  start: number;
  end: number;
  variables: { name: string; units?: string; kind: string; dimensions: unknown[] }[];
};
type Run = RecordedRun & { run_id?: string; id?: string; legacy?: boolean; [key: string]: unknown };
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
export function ScienceChart({
  data,
  labels,
  theme,
  paired = false,
  compact = false,
}: {
  data: uPlot.AlignedData;
  labels: string[];
  theme: string;
  paired?: boolean;
  compact?: boolean;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!ref.current || !data[0].length) return;
    const host = ref.current;
    const plot = new uPlot(
      {
        width: Math.max(160, host.clientWidth),
        height: compact ? 220 : 300,
        legend: { show: !compact },
        tzDate: (ts) => uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'),
        series: [
          {},
          ...labels.map((label, i) => ({
            label,
            stroke: (paired ? ['#0d9488', '#858b94'] : ['#0d9488', '#e87945', '#6366f1'])[i % 3],
            dash: paired && i === 1 ? [6, 3] : [],
            width: 1.5,
            points: compact ? { show: false } : undefined,
          })),
        ],
        axes: [
          { stroke: theme === 'dark' ? '#b8c7d9' : '#475569', values: timeTicks },
          { stroke: theme === 'dark' ? '#b8c7d9' : '#475569' },
        ],
      },
      data,
      host,
    );
    const resize = new ResizeObserver(() =>
      plot.setSize({ width: Math.max(160, host.clientWidth), height: compact ? 220 : 300 }),
    );
    resize.observe(host);
    return () => {
      resize.disconnect();
      plot.destroy();
    };
  }, [data, labels, theme, paired, compact]);
  return <div ref={ref} aria-label={labels.join(', ')} style={{ minWidth: 0 }} />;
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
  const { state, navigate, openCase } = useApp();
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
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [spinup, setSpinup] = useState('0');
  const [corrected, setCorrected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [series, setSeries] = useState<SeriesData | null>(null);
  const [metrics, setMetrics] = useState<Metric[]>([]);
  const [diagnostic, setDiagnostic] = useState<EnergyDiagnostic | null>(null);
  const [comparison, setComparison] = useState('');
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
    setComparison('');
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
  }, [from, to, obs, spinup, corrected, comparison]);
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
          throw new Error('Invalid skipped record count');
        const parse = (value: string) => (value ? utcSeconds(value) : null);
        const start = parse(from),
          end = parse(to);
        if (
          (start !== null && !Number.isFinite(start)) ||
          (end !== null && !Number.isFinite(end)) ||
          (start !== null && end !== null && start >= end)
        )
          throw new Error('Invalid evaluation window');
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
  const windowArgs = useCallback(() => {
    const parse = (value: string) => {
      if (!value) return null;
      const n = utcSeconds(value);
      if (!Number.isFinite(n)) throw new Error('Invalid UTC date');
      return n;
    };
    const a = parse(from),
      b = parse(to);
    if (a !== null && b !== null && a >= b) throw new Error('Start must precede end');
    return { from: a, to: b };
  }, [from, to]);
  const readSeries = useCallback(
    async function readSeries(vars: string, maxPoints: number | null = 2400) {
      const args = { case: source, vars, ...windowArgs(), maxPoints };
      const key = JSON.stringify(args);
      if (maxPoints !== null && cache.current.has(key)) return cache.current.get(key) as SeriesData;
      const value = await invokeJson<SeriesData>('series', args);
      if (maxPoints !== null) {
        if (cache.current.size >= 16) cache.current.delete(cache.current.keys().next().value!);
        cache.current.set(key, value);
      }
      return value;
    },
    [source, windowArgs],
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
      if (!Number.isInteger(days) || days < 0) throw new Error('Invalid spinup');
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
      });
      fullEnergyStats(data);
      if (current()) {
        setDiagnostic(data);
      }
    });
  const compare = () =>
    task(async (current) => {
      if (!Number.isInteger(Number(spinup)) || Number(spinup) < 0)
        throw new Error('Invalid spinup');
      const entries = comparison
        .split('\n')
        .filter(Boolean)
        .map((line) => line.split('|').map((v) => v.trim()));
      if (
        !entries.length ||
        entries.length > 20 ||
        entries.some((x) => x.length !== 2 || !x[0] || !x[1])
      )
        throw new Error('Use 1–20 lines: run_case | observation');
      const inventories = await boundedMap(
        [...new Set([caseDir, ...state.cases.map((c) => c.dir)].filter(Boolean))],
        2,
        (path) => invokeJson<Run[]>('model_case_runs', { dir: path }),
      );
      if (!current()) return;
      const sources = inventories.flat();
      for (const [path] of entries) {
        if (!sources.some((r) => r.run_case === path))
          throw new Error(`Not a recorded run copy: ${path}`);
      }
      const rows = await boundedMap(entries, 2, async ([path, observation]) => {
        const values = await invokeJson<Metric[]>('metrics', {
          case: path,
          obs: observation,
          spinup: Number(spinup),
          corrected,
          summaryOnly: true,
          pairVars: null,
          maxPoints: 2400,
          ...windowArgs(),
        });
        return values.map((v) => ({
          ...v,
          site: path,
          provenance: sources.find((r) => r.run_case === path),
        }));
      });
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
      if (!source || !variable) throw new Error('Select a recorded run and variable');
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
      conditions: { obs, from, to, spinup, corrected, comparisonSources: comparison },
      notice: 'No automatic ranking across unequal inputs or evaluation conditions',
    };
  }
  const report = () =>
    task(async (current) => {
      setExportNotice('');
      if (!catalog) throw new Error('Load a recorded result before exporting');
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
        const opened = await openCase(dir, 'results');
        if (!opened) throw new Error(tr('未能切换站点', 'Could not switch stations'));
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
            label: tr('更多分析与导出', 'More analyses and exports'),
            type: 'choice',
            value: tab,
            options: tabs.map((v, i) => ({
              value: v,
              label: en
                ? [
                    'Overview',
                    'Data catalog',
                    'Time series',
                    'Observation evaluation',
                    'Site comparison',
                    'Diagnostics',
                    'Report',
                  ][i]
                : [
                    '结果总览',
                    '数据目录',
                    '时序分析',
                    '观测评估',
                    '多站比较',
                    '闭合诊断',
                    '报告导出',
                  ][i],
            })),
            disabled: busy,
            set: (v) => setTab(v as typeof tab),
          },
        ];
  if (
    !runChooser &&
    !settingsOpen &&
    state.cases.length > 0 &&
    (!caseDir || state.cases.length > 1)
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
        { id: 'result-from', label: 'UTC start', value: from, set: setFrom },
        { id: 'result-to', label: 'UTC end', value: to, set: setTo },
      ].map((f) => ({ ...f, type: 'text' as const, disabled: busy })),
    );
  if (!runChooser && !settingsOpen && (tab === 'series' || tab === 'overview'))
    fields.push({
      id: 'result-variable',
      label: tr('图表变量', 'Chart variable'),
      value: variable,
      type: 'choice',
      options: catalog?.variables
        .filter((v) => v.kind === 'series' && v.name !== 'time')
        .map((v) => ({ value: v.name, label: overviewVariableLabel(v.name, en) })),
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
      value: comparison,
      type: 'text',
      disabled: busy,
      set: setComparison,
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
  const actions: UiAction[] = [
    ...(tab === 'overview'
      ? [
          {
            id: 'result-closure',
            enabled: !busy,
            label: tr('诊断偏差', 'Diagnose deviations'),
            kind: 'click' as const,
            run: () => setTab('diagnostics'),
          },
          {
            id: 'result-improve',
            enabled: !busy,
            label: tr('模型改进', 'Improve model'),
            kind: 'click' as const,
            run: () => navigate('research'),
          },
        ]
      : []),
    ...(tab !== 'overview'
      ? [
          {
            id: 'result-overview',
            enabled: !busy,
            label: tr('返回结果总览', 'Back to results overview'),
            kind: 'click' as const,
            run: () => setTab('overview'),
          },
          {
            id: 'result-improve',
            enabled: !busy,
            label: tr('模型改进', 'Improve model'),
            kind: 'click' as const,
            run: () => navigate('research'),
          },
        ]
      : []),
    ...(tab === 'series' || tab === 'overview'
      ? [
          {
            id: 'plot-series',
            label: tr('绘制时序', 'Plot series'),
            kind: 'click' as const,
            enabled: !!source && !!variable && !busy,
            run: plot,
          },
          {
            id: 'export-full-series-csv',
            label: tr('导出完整时序 CSV', 'Export full time-series CSV'),
            kind: 'commit' as const,
            enabled: !!source && !!variable && !busy,
            run: exportFullSeries,
          },
        ]
      : []),
    ...(tab === 'evaluation' || tab === 'overview'
      ? [
          {
            id: 'evaluate-observations',
            label: tr('评估观测', 'Evaluate observations'),
            kind: 'click' as const,
            enabled: !!source && !!obs && !busy,
            run: evaluate,
          },
        ]
      : []),
    ...(tab === 'comparison'
      ? [
          {
            id: 'compare-sites',
            label: tr('比较站点', 'Compare sites'),
            kind: 'click' as const,
            enabled: !!comparison && !busy,
            run: compare,
          },
        ]
      : []),
    ...(tab === 'diagnostics'
      ? [
          {
            id: 'diagnose-closure',
            label: tr('统计能量通量残差', 'Summarize energy flux residual'),
            kind: 'click' as const,
            enabled: !!catalog && !busy,
            run: diagnose,
          },
        ]
      : []),
    ...((tab === 'series' || tab === 'diagnostics') && displayedSeries
      ? [
          {
            id: 'export-series-data',
            label: tr('导出显示数据（抽样 JSON）', 'Export displayed data (sampled JSON)'),
            kind: 'commit' as const,
            enabled: !busy,
            run: () =>
              task(async (current) => {
                setExportNotice('');
                const result = await download(
                  'series-sampled.json',
                  JSON.stringify({ source, ...displayedSeries }),
                );
                if (current()) setExportNotice(exportMessage(result));
                return result;
              }),
          },
        ]
      : []),
    ...(tab === 'report'
      ? [
          {
            id: 'export-report',
            label:
              reportFormat === 'pdf'
                ? tr('打印 / 另存为 PDF', 'Print / Save as PDF')
                : tr('导出报告', 'Export report'),
            kind: 'commit' as const,
            enabled: !!catalog && !busy,
            run: report,
          },
        ]
      : []),
  ];
  const pageActions: UiAction[] = runChooser
    ? [
        {
          id: 'result-runs-close',
          label: tr('关闭', 'Close'),
          kind: 'click',
          enabled: !busy,
          run: () => changeRunChooser(false),
        },
        {
          id: 'result-run-comparison',
          label: tr('进入多运行比较', 'Open multi-run comparison'),
          kind: 'click',
          enabled: !busy,
          run: () => {
            setRunChooser(false);
            setTab('comparison');
          },
        },
      ]
    : settingsOpen
      ? [
          ...actions.filter(
            (action) => action.id === 'evaluate-observations' || action.id === 'plot-series',
          ),
          {
            id: 'result-settings-close',
            label: tr('关闭', 'Close'),
            kind: 'click',
            enabled: !busy,
            run: () => changeSettings(false),
          },
          {
            id: 'result-pick-observation',
            label: tr('选择文件', 'Choose a file'),
            kind: 'click',
            enabled: !busy,
            run: async () => {
              const file = await invoke<string | null>('pick_file', {
                key: 'result-observations',
                filter: 'nc',
              });
              if (file) chooseObservation(file);
              return file;
            },
          },
        ]
      : [
          {
            id: 'result-runs',
            label: tr('比较其他运行', 'Compare other runs'),
            kind: 'click',
            enabled: !busy && runs.some((item) => item.run_case),
            run: () => changeRunChooser(true),
          },
          ...(tab === 'overview'
            ? [
                {
                  id: 'result-settings',
                  label: tr('评估设置', 'Evaluation settings'),
                  kind: 'click' as const,
                  enabled: !!catalog && !busy,
                  run: () => changeSettings(true),
                },
                ...actions
                  .filter((action) =>
                    ['result-closure', 'result-improve', 'export-full-series-csv'].includes(
                      action.id,
                    ),
                  )
                  .map((action) => ({ ...action, enabled: action.enabled !== false && !busy })),
              ]
            : actions),
        ];
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
    actions: pageActions,
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
        <Button
          disabled={busy || !runs.some((item) => item.run_case)}
          onClick={() => changeRunChooser(true)}
        >
          <NavIcon name="results" />
          {tr('比较其他运行', 'Compare other runs')}
        </Button>
      </header>
      {state.cases.length > 0 && (!caseDir || state.cases.length > 1) && (
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
                  {catalog?.variables
                    .filter((item) => item.kind === 'series' && item.name !== 'time')
                    .map((item) => (
                      <option key={item.name} value={item.name}>
                        {overviewVariableLabel(item.name, en)}
                      </option>
                    ))}
                </Select>
                <Button
                  variant="ghost"
                  disabled={!catalog || busy}
                  onClick={() => changeSettings(true)}
                >
                  <NavIcon name="settings" />
                  {tr('评估设置', 'Evaluation settings')}
                </Button>
              </div>
            </div>
            <div className="results-plot-meta">
              <span>
                {tr('单位', 'Units')}: {selectedMetric?.units || variableInfo?.units || '—'}
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
          <Card className="results-next">
            <div>
              <h2>{tr('下一步可以做什么', 'Next steps')}</h2>
              <p>{tr('候选原因需要进一步验证。', 'Candidate causes need further validation.')}</p>
            </div>
            <div>
              {actions
                .filter((action) => ['result-closure', 'result-improve'].includes(action.id))
                .map((action, index) => (
                  <Button
                    key={action.id}
                    disabled={busy}
                    onClick={() => run(async () => action.run())}
                  >
                    <NavIcon name={(['simulation', 'settings', 'results'] as const)[index]} />
                    {action.label}
                  </Button>
                ))}
            </div>
          </Card>
        </section>
      )}
      <footer className="results-secondary">
        {tab !== 'overview' && (
          <>
            <Button variant="ghost" disabled={busy} onClick={() => setTab('overview')}>
              {tr('← 返回结果总览', '← Back to results overview')}
            </Button>
            <Button disabled={busy} onClick={() => navigate('research')}>
              {tr('模型改进', 'Improve model')}
            </Button>
          </>
        )}
        <Field label={tr('更多分析与导出', 'More analyses and exports')}>
          <Select
            value={tab}
            disabled={busy}
            onChange={(event) => setTab(event.target.value as typeof tab)}
          >
            {tabs.map((name, index) => (
              <option key={name} value={name}>
                {en
                  ? [
                      'Overview',
                      'Data catalog',
                      'Time series',
                      'Observation evaluation',
                      'Site comparison',
                      'Diagnostics',
                      'Report',
                    ][index]
                  : [
                      '结果总览',
                      '数据目录',
                      '时序分析',
                      '观测评估',
                      '多站比较',
                      '闭合诊断',
                      '报告导出',
                    ][index]}
              </option>
            ))}
          </Select>
        </Field>
        {tab === 'overview' && (
          <Button
            variant="ghost"
            disabled={!source || !variable || busy}
            onClick={() => run(exportFullSeries)}
          >
            {tr('导出完整 CSV', 'Export full CSV')}
          </Button>
        )}
        {onBack && (
          <Button variant="ghost" onClick={onBack}>
            {tr('返回模拟', 'Back to simulation')}
          </Button>
        )}
      </footer>
      <Dialog
        title={tr('选择运行', 'Choose a run')}
        description={tr(
          '选择真实运行记录查看结果，或进入多运行比较。',
          'Select a recorded run or open multi-run comparison.',
        )}
        open={runChooser}
        onOpenChange={changeRunChooser}
      >
        <div className="result-run-choices">
          {runs
            .filter((item) => item.run_case)
            .map((item, index) => (
              <Button
                key={index}
                disabled={busy}
                aria-pressed={source === item.run_case}
                onClick={() => chooseRecordedRun(item.run_case!)}
              >
                <strong>{overviewRunLabel(item, en)}</strong>
                <span>{overviewRunStatus(item, en)}</span>
              </Button>
            ))}
        </div>
        <Button
          disabled={busy}
          onClick={() => {
            setRunChooser(false);
            setTab('comparison');
          }}
        >
          {tr('进入多运行比较', 'Open multi-run comparison')}
        </Button>
      </Dialog>
      <Dialog
        title={tr('评估设置', 'Evaluation settings')}
        description={tr(
          '选择观测与评估条件，计算后会更新当前图表和指标。',
          'Choose observations and conditions; evaluation updates the chart and metrics.',
        )}
        open={settingsOpen}
        onOpenChange={changeSettings}
      >
        {error && <Notice tone="error">{error}</Notice>}
        <Field label={tr('观测文件', 'Observation file')}>
          <div className="browse-input">
            <Input
              value={obs}
              disabled={busy}
              onChange={(event) => chooseObservation(event.target.value)}
            />
            <Button
              disabled={busy}
              onClick={() =>
                run(async () => {
                  const file = await invoke<string | null>('pick_file', {
                    key: 'result-observations',
                    filter: 'nc',
                  });
                  if (file) chooseObservation(file);
                  return file;
                })
              }
            >
              {tr('选择文件', 'Choose a file')}
            </Button>
          </div>
        </Field>
        <Advanced
          title={tr('高级评估设置', 'Advanced evaluation settings')}
          open={advanced}
          onOpenChange={setAdvanced}
        >
          <div className="form-grid">
            <Field label="UTC start">
              <Input
                type="datetime-local"
                value={from}
                disabled={busy}
                onChange={(event) => setFrom(event.target.value)}
              />
            </Field>
            <Field label="UTC end">
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
          <Button
            variant="primary"
            disabled={!source || !obs.trim() || busy}
            onClick={() => run(evaluate)}
          >
            {busy ? tr('正在评估…', 'Evaluating…') : tr('更新评估', 'Update evaluation')}
          </Button>
          <Button disabled={!source || !variable || busy} onClick={() => run(plot)}>
            {tr('刷新模拟曲线', 'Refresh simulation curve')}
          </Button>
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
                      <th>Variable</th>
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
            {catalog.files} files · {catalog.steps} steps ·{' '}
            {new Date(catalog.start * 1000).toISOString()} —{' '}
            {new Date(catalog.end * 1000).toISOString()}
          </p>
          <table>
            <thead>
              <tr>
                <th>Variable</th>
                <th>Units</th>
                <th>Shape</th>
              </tr>
            </thead>
            <tbody>
              {catalog.variables.map((v) => (
                <tr key={v.name}>
                  <td>{v.name}</td>
                  <td>{v.units || '—'}</td>
                  <td>{JSON.stringify(v.dimensions)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </Card>
      )}
      {['series', 'evaluation', 'comparison', 'diagnostics'].includes(tab) && (
        <Card>
          <div className="grid gap-3 md:grid-cols-2">
            <Field label="UTC start">
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
            <Field label="UTC end">
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
          {(tab === 'evaluation' || tab === 'comparison') && (
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
          {tab === 'comparison' && (
            <>
              <Field label="run_case | observation (one station per line)">
                <textarea
                  className="w-full"
                  rows={4}
                  value={comparison}
                  onChange={(e) => setComparison(e.target.value)}
                />
              </Field>
              <Notice>
                {tr(
                  '相同时段和评估设置下展示指标；输入不同不解释版本优劣。',
                  'Metrics use the selected evaluation window; different inputs do not establish version superiority.',
                )}
              </Notice>
            </>
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
          {actions
            .filter((a) => a.id !== 'result-overview' && a.id !== 'result-improve')
            .map((a) => (
              <Button key={a.id} disabled={!a.enabled} onClick={() => run(async () => a.run())}>
                {a.label}
              </Button>
            ))}
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
                {displayedSeries!.n}/{displayedSeries!.source_n} points{' '}
                {displayedSeries!.downsampled ? '(sampled)' : ''}
              </p>
            </>
          )}
        </Card>
      )}
      {(tab === 'evaluation' || tab === 'comparison') && (
        <Card title={tr('有效配对指标', 'Paired metrics')}>
          <table>
            <thead>
              <tr>
                {['Site', 'Variable', 'n', 'RMSE', 'MAE', 'Bias', 'R²', 'r', 'NSE', 'KGE'].map(
                  (s) => (
                    <th key={s}>{s}</th>
                  ),
                )}
              </tr>
            </thead>
            <tbody>
              {metrics.map((m, i) => (
                <tr key={i}>
                  <td>{m.site || source}</td>
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
          {actions.map((action) => (
            <Button
              key={action.id}
              disabled={!action.enabled}
              onClick={() => run(async () => action.run())}
            >
              {action.label}
            </Button>
          ))}
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
