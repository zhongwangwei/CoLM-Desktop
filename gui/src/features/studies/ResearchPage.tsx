import { Fragment, useCallback, useEffect, useRef, useState } from 'react';
import type uPlot from 'uplot';
import { ScienceChart } from '../results/ResultsPage';
import { invoke, invokeJson } from '../../lib/ipc';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { usePageContract, type UiAction, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Select, Notice, Advanced } from '../../components/ui';
import {
  calibrationWindows,
  candidateBudget,
  fitValidation,
  networkSpec,
  networkDefaults,
  parameterSpec,
  studyPermissions,
  type ParameterContext,
  applyResearchSeed,
} from './study';
import {
  remoteStudyPermissions,
  remoteStudyReply,
  studyModelRevision,
  type RemoteStudyRecord,
  type RemoteStudyReply,
} from './remote-study';
import type { ModelDraft, VersionRef } from '../../lib/types';
import StudyImportDialog, { type StudyPreview } from '../versions/StudyImportDialog';
import { REMOTE_CHECK_MS } from '../runs/run-model';
import LiveServerLoad from '../runs/LiveServerLoad';
import { researchMethods, methodFromStudy, completedCandidates } from './research-home';
import { IGBP, PFT, USGS } from '../versions/scope-labels';
import './research-home.css';
type Envelope = {
  state?: {
    status?: string;
    best_member?: string;
    completed_candidates?: number;
    candidates?: Record<
      string,
      { feasible?: boolean; calibration?: number; validation?: number; reason?: string }
    >;
    tasks?: Record<string, unknown>;
  };
  manifest?: { id?: string; spec?: unknown };
  results?: (string | { path: string })[];
  model_version?: { model_revision?: { version?: VersionRef } | null } | null;
  remote?: RemoteStudyRecord | null;
  remote_pending?: RemoteStudyRecord | null;
};
type HybridStudy = { dir: string; sites: string[]; parameters: string[]; status: string };

function StudyResult({ text, theme }: { text: string; theme: string }) {
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
      return (
        <>
          <ScienceChart
            data={
              [
                sample(data.time),
                ...['baseline', 'p05', 'p50', 'p95'].map((key) => sample(data[key])),
              ] as uPlot.AlignedData
            }
            labels={['Baseline', 'P05', 'Median', 'P95']}
            theme={theme}
          />
          <p>
            {length} source points; chart stride {stride}. Quantiles describe this sampling design,
            not a predictive confidence interval.
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
        {rows.length > 1000 && <p>Showing first 1000 rows; export includes all records.</p>}
      </div>
    );
  }
  return <pre className="max-h-96 overflow-auto">{text}</pre>;
}

export default function ResearchPage() {
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
  const [kernel, setKernel] = useState(state.kernelDir || '');
  const [contexts, setContexts] = useState<ParameterContext[]>([]);
  const [parameters, setParameters] = useState<
    Record<string, { enabled: boolean; min: string; max: string; scale: string }>
  >({});
  const [observations, setObservations] = useState('');
  const [variables, setVariables] = useState('Qle');
  const [method, setMethod] = useState('lhs');
  const [count, setCount] = useState('40');
  const [population, setPopulation] = useState('8');
  const [generations, setGenerations] = useState('4');
  const [jobs, setJobs] = useState('1');
  const [seed, setSeed] = useState('1');
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
  const [out, setOut] = useState('');
  const [name, setName] = useState('research-model');
  const [member, setMember] = useState('');
  const [busy, setBusy] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState('');
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
  const ai = tab === 'parameters' || tab === 'process';
  const tuning = tab !== 'uncertainty';
  useNavigationGuard(() =>
    runLock.current || lock.current
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
    if (!kernel && state.kernelDir) setKernel(state.kernelDir);
  }, [state.kernelDir, kernel]);
  useEffect(() => {
    const requestEpoch = epoch;
    requestEpoch.current++;
    setSites(dir ? [dir] : []);
    setStudyDir('');
    setEnvelope(null);
    setRemoteReply(null);
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
    setPreflight('');
    setEnvelope(null);
    setRemoteReply(null);
    setStudyDir('');
    setFit(null);
  }, [sites, kernel]);
  useEffect(() => {
    epoch.current++;
    setPreflight('');
  }, [
    tab,
    observations,
    variables,
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
        setMember(data.state?.best_member || '');
        const files = (data.results || []).map((v) => (typeof v === 'string' ? v : v.path));
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
  }, [studyDir, busy, launching, remotePermissions.exists, envelope?.state?.status, readStatus]);
  useEffect(() => {
    if (!studyDir || busy || launching || !remotePermissions.poll) return;
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
  }, [studyDir, busy, launching, remotePermissions.poll, readRemoteStatus]);
  async function effectiveCase(path: string) {
    const context = await invokeJson<{ entry: { dir: string } }>('model_case_context', {
      dir: path,
      modelRevision: state.versionRef,
    });
    return context.entry.dir;
  }
  const loadContexts = () =>
    task(async (current) => {
      if (!sites.length || !kernel) throw new Error('Select cases and a build configuration');
      const effective = [];
      for (const path of sites) {
        effective.push(await effectiveCase(path));
        if (!current()) return;
      }
      const rows = await invokeJson<ParameterContext[]>('study_parameter_contexts', {
        dirs: effective,
        kernelDir: kernel,
      });
      if (current()) {
        setContexts(rows);
        const seed = state.researchSeed?.parameters;
        if (seed?.length) {
          const picked = applyResearchSeed(rows, seed);
          setParameters((old) => ({ ...old, ...picked }));
          const matched = Object.keys(picked).length;
          setSeedNotice(
            tr(
              `已按版本页选的参数勾上 ${matched} 个${matched < seed.length ? `；另 ${seed.length - matched} 个在这些算例里没有（比如算例里没有那个地类或植物类型）` : ''}。`,
              `Ticked ${matched} of the parameters chosen on the version page${matched < seed.length ? `; ${seed.length - matched} do not apply to these cases (for example, the class or plant type is absent)` : ''}.`,
            ),
          );
          update({ researchSeed: null });
        }
      }
      return rows;
    });
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
  }
  function spec() {
    if (!sites.length || !kernel || !confirmed)
      throw new Error('Select cases, a build configuration and confirm reviewed ranges');
    const roots = new Set(sites.map((s) => s.replace(/[/\\][^/\\]+$/, '')));
    if (roots.size !== 1) throw new Error('All cases must share a project directory');
    const params = contexts.flatMap((row, i) => {
      const p = parameters[String(i)];
      return p?.enabled ? [parameterSpec(row, p.min, p.max, p.scale)] : [];
    });
    if (!params.length && !(ai && trainNetwork))
      throw new Error('Select at least one parameter or network training');
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
      throw new Error('Jobs must be 1–16; seed must be a nonnegative integer');
    const names = variables
      .split(',')
      .map((v) => v.trim())
      .filter(Boolean);
    if (!names.length) throw new Error('Choose target/output variables');
    const base = {
      kind: tuning ? 'tuning' : 'uncertainty',
      method: tuning ? 'differential-evolution' : method,
      seed: random,
      kernel_dir: kernel,
      base_cases: sites,
      parameters: params,
      site_mode: 'shared',
    };
    if (!tuning)
      return {
        caseRoot: [...roots][0]!,
        specJson: JSON.stringify({
          ...base,
          outputs: names,
          budget: {
            candidate_count: method === 'oat' ? undefined : budget.candidates,
            jobs: concurrency,
          },
        }),
      };
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
    const pairs = Number(minPairs);
    if (!Number.isInteger(pairs) || pairs < 2) throw new Error('Minimum pairs must be ≥2');
    const targets = sites.flatMap((path) => {
      const site = path.split(/[/\\]/).pop()!;
      if (!obs[site]) throw new Error(`Observation missing: ${site}`);
      return names.map((variable) => ({
        key: `${variable}@${site}`,
        site,
        variable,
        metric,
        weight: 1,
        min_pairs: pairs,
        ...windows,
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
        },
        ...(ai && trainNetwork ? { hybrid: network() } : {}),
      }),
    };
  }
  const check = () =>
    task(async (current) => {
      const plan = spec();
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
      const plan = spec();
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
  const start = async () => {
    if (runLock.current || lock.current)
      throw new Error('A study operation is already in progress');
    if (!studyDir || !kernel || !envelope || !confirmed)
      throw new Error(
        'Select a study and build configuration, refresh its status and review its frozen budget',
      );
    if (remotePermissions.exists)
      throw new Error(
        'This study has a server submission; inspect or fetch it instead of starting another run',
      );
    const concurrency = Number(jobs);
    if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 16)
      throw new Error('Jobs must be 1–16');
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
        engine: ai ? 'rust' : engine,
        modelVersion: null,
        modelRevision,
      });
      if (token === epoch.current) return await readStatus();
      return response;
    } catch (e) {
      if (token === epoch.current) {
        setError(String(e));
        if (host)
          setRemoteReply((previous) => ({
            ...previous,
            host,
            status: { state: 'unknown' },
            warnings: [String(e)],
          }));
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
      throw new Error('Use the recorded server controls for this study');
    const token = epoch.current;
    const value = await task(async (current) => {
      const response = await invoke(command, {
        studyDir,
        ...(command === 'study_retry' ? { includeReview: false } : {}),
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
  const cancelRemote = () =>
    task(async (current) => {
      remoteRequest.current++;
      const answer = remoteStudyReply(await invokeJson('remote_study_cancel', { studyDir }));
      if (current()) setRemoteReply(answer);
      return answer;
    });
  const fetchRemote = () =>
    task(async (current) => {
      if (!remotePermissions.fetch)
        throw new Error('A confirmed server exit is required before fetching');
      remoteRequest.current++;
      const answer = await invokeJson<{
        fetched: boolean;
        exit_code: number;
        status: string;
        scientific_verified?: boolean;
        verification_error?: string;
      }>('remote_study_fetch', { studyDir });
      if (!answer.fetched) throw new Error('Server did not confirm a successful fetch');
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
  const saveVersion = () =>
    task(async (current) => {
      if (!member || !name.trim()) throw new Error('Member and version name required');
      const version = await invokeJson<{ version: { id: string; revision: number } }>(
        'study_save_version',
        {
          studyDir,
          member,
          name,
          note: 'Research product; applicability retained from study training and validation scope',
        },
      );
      await refreshVersions();
      if (current()) notify(`${version.version.id}@${version.version.revision}`);
      return version;
    });
  // 研究结果进模型版本和手动改参数走同一条路：先写进草稿，到版本页审阅、冻结。从草稿进来的写回那个草稿，
  // 否则以研究所用的版本为父版本新建一个。原来要么直接冻结成版本（跳过审阅和冻结检查），要么回版本页
  // 再手动找研究目录导入（2026-10-21 用户要求把研究理进"构建新版本"）。
  const writeBack = () =>
    task(async (current) => {
      const target = state.researchTarget;
      if (!studyDir) throw new Error('Open a finished study first');
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
          member: member || null,
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
  const inspectHybrid = () =>
    task(async (current) => {
      const data = await invokeJson<Record<string, unknown>>('hybrid_info', {
        case: await effectiveCase(dir),
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
        case: dir,
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
        throw new Error('Fit name must contain only letters, numbers, - and _');
      if (!chosenStudies.length || !out.trim())
        throw new Error('Select completed studies and an output directory');
      const net = network();
      const penalty = Number(ridge),
        iterations = Number(epochs);
      if (
        !Number.isFinite(penalty) ||
        penalty < 0 ||
        !Number.isInteger(iterations) ||
        iterations < 1
      )
        throw new Error('Invalid fit settings');
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
          sourceCase: dir,
          parent: state.versionRef,
        });
      return data;
    });
  const saveFitVersion = () =>
    task(async (current) => {
      if (!fit || typeof fit.model !== 'string' || !fit.networkSpec)
        throw new Error('Complete a fit first');
      if (fitValidation(fit.report) === false) throw new Error('Cross-validation gate failed');
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
  const permissions = studyPermissions(launching && status === 'Ready' ? 'Running' : status);
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
    'New model version / fit name': '新模型版本 / 拟合名称',
    'Export / fit output directory': '导出 / 拟合输出目录',
    'Member to save': '要保存的候选成员',
    'Result file': '研究结果文件',
    'Ridge regularization': '岭回归正则系数',
    'Adam epochs': 'Adam 训练轮数',
    Advanced: '专家设置',
  };
  const unresolvedRemote =
    remotePermissions.exists && !remotePermissions.terminal && !remotePermissions.fetched;
  function chooseMethod(value: string) {
    if (lock.current || runLock.current || unresolvedRemote)
      throw new Error('Wait for the current study operation or server record to be resolved');
    if (!researchMethods.some((item) => item.id === value))
      throw new Error('Unknown research method');
    if (value === 'source') {
      navigate('versions');
      return;
    }
    if (value !== tab) {
      epoch.current++;
      setTab(value);
      setStep(1);
      setResult('');
      setFit(null);
      if (value === 'process') chooseSlot('soil_stress');
      else if (value === 'parameters') chooseSlot('pft');
    }
    setWorkbench(true);
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
  // 返回到进来的地方：草稿、"新建修改版"或侧栏的研究首页（侧栏的研究 2026-10-21 先保留）。
  function showHome() {
    if (lock.current || runLock.current) throw new Error('A study operation is in progress');
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
        ...state.kernels.map((k) => ({ value: k.dir, label: k.dir })),
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
        : tr('输出变量（逗号分隔）', 'Output variables (comma separated)'),
      variables,
      setVariables,
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
          field('population', 'Population', population, setPopulation, 'number'),
          field('generations', 'Generations', generations, setGenerations, 'number'),
        ]
      : [
          field('sampling-method', 'Sampling method', method, setMethod, 'choice', [
            { value: 'lhs', label: 'Latin hypercube' },
            { value: 'oat', label: 'One at a time' },
          ]),
          ...(method === 'lhs'
            ? [field('candidate-count', 'Candidates', count, setCount, 'number')]
            : []),
        ]),
    field('research-jobs', 'Concurrent jobs (1–16)', jobs, setJobs, 'number'),
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
      setParameters((old) => ({ ...old, [String(i)]: { ...p, ...patch } }));
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
          (v) => setTrainNetwork(v === 'true'),
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
    field('product-name', 'New model version / fit name', name, setName),
    field('export-directory', 'Export / fit output directory', out, setOut),
    ...(envelope
      ? [
          field('study-member', 'Member to save', member, setMember),
          field(
            'study-result-file',
            'Result file',
            resultPath,
            setResultPath,
            'choice',
            (envelope.results || []).map((v) => {
              const path = typeof v === 'string' ? v : v.path;
              return { value: path, label: path };
            }),
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
      !!kernel && confirmed && !launching && !unresolvedRemote,
    ),
    action(
      'study-create',
      tr('创建研究任务', 'Create study'),
      create,
      !!kernel && confirmed && !launching && !unresolvedRemote,
      true,
    ),
  ];
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
      permissions.run && !!kernel && confirmed && !launching && !remotePermissions.exists,
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
            permissions.resume,
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
          const value = await invoke('study_export', { studyDir, out });
          if (current()) setResult(String(value));
          return value;
        }),
      !!studyDir && !!out && !launching && !unresolvedRemote,
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
      permissions.results && (!!state.researchTarget || !!modelUsed) && !unresolvedRemote,
      true,
    ),
    action(
      'study-save-version',
      tr('不经草稿，直接保存为版本', 'Save as a version without a draft'),
      saveVersion,
      permissions.results && !!member && !!name && !unresolvedRemote,
      true,
    ),
  ];
  const aiActions = ai
    ? [
        action('hybrid-info', tr('检查适用性', 'Inspect applicability'), inspectHybrid, !!dir),
        action(
          'hybrid-climate',
          tr('生成气候特征', 'Generate climate features'),
          climate,
          !!kernel && sites.length > 0,
          true,
        ),
        action(
          'hybrid-check',
          tr('检查当前算例网络', 'Check case network'),
          checkHybrid,
          !!dir && !!kernel,
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
  const resourcesAction = action(
    'research-resources',
    tr('管理计算资源', 'Manage compute resources'),
    () => navigate('resources'),
    !launching,
  );
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
    options: researchMethods.map((item) => ({ value: item.id, label: en ? item.en : item.zh })),
    set: chooseMethod,
    disabled: busy || launching || unresolvedRemote,
  };
  const newStudyAction = action(
    'research-new',
    tr('＋ 新建研究', '＋ New study'),
    () => document.getElementById('research-method-tuning')?.focus(),
    !launching && !unresolvedRemote,
  );
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
            () => setWorkbench(true),
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
  const currentState = remotePermissions.exists ? remoteStatus : status;
  const methodTitle = researchMethods.find((item) => item.id === tab)!;
  const currentMethod = researchMethods.find((item) => item.id === methodFromStudy(frozenSpec))!;
  // 编译配置：没选时挑一个单点内核（优先 default）；界面上只显示配置名，不显示目录。
  useEffect(() => {
    if (kernel || !state.kernels.length) return;
    const pick =
      state.kernels.find((k) => k.preset === 'default') ??
      state.kernels.find((k) => k.macros.includes('SinglePoint')) ??
      state.kernels[0];
    if (pick) setKernel(pick.dir);
  }, [state.kernels, kernel]);
  // 项目里只有一个单点算例时直接勾上（没有别的可选）。换版本会清空所选算例，所以不是只勾一次；
  // 用户自己动过勾选就不再替他勾。
  const sitesTouched = useRef(false);
  useEffect(() => {
    if (sitesTouched.current || sites.length) return;
    const single = state.cases.filter((c) => !c.spatial);
    if (single.length === 1) setSites([single[0]!.dir]);
  }, [state.cases, sites]);
  // 进到第 2 步就读出这些算例能研究的参数（原来要先点"读取适用参数"）；同一组算例只读一次。
  useEffect(() => {
    const key = `${kernel}|${sites.join('|')}`;
    if (step !== 2 || !kernel || !sites.length || busy || loadedFor.current === key) return;
    loadedFor.current = key;
    execute(loadContexts);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step, kernel, sites, busy]);
  // 建好或打开了一个研究：直接看运行与结果。
  useEffect(() => {
    if (studyDir) setStep(4);
  }, [studyDir]);
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
          actions: [
            backAction,
            ...designActions,
            ...lifecycleActions,
            ...aiActions,
            resourcesAction,
          ],
          details: {
            studyDir,
            envelope,
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
          actions: [newStudyAction, ...homeActions],
          details: {
            studyDir: hasStudy ? studyDir : null,
            status: hasStudy ? currentState : null,
            completedCandidates: hasStudy ? completed : null,
            error,
          },
        },
  );

  const renderActions = (actions: UiAction[]) =>
    actions.map((a) => (
      <Button
        key={a.id}
        disabled={!a.enabled}
        variant={a.id === 'study-cancel' ? 'danger' : a.kind === 'commit' ? 'primary' : 'secondary'}
        onClick={() => execute(a.run)}
      >
        {a.label}
      </Button>
    ));
  if (!workbench)
    return (
      <div className="research-home">
        {state.simulationReturn && (
          <div className="button-row">
            <Button onClick={returnToSimulation}>
              ← {tr('返回模拟配置', 'Back to simulation setup')}
            </Button>
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
          <Button
            variant="primary"
            disabled={!newStudyAction.enabled}
            onClick={() => execute(newStudyAction.run)}
          >
            {newStudyAction.label}
          </Button>
        </header>
        {error && <Notice tone="error">{error}</Notice>}
        <div className="research-methods" role="group" aria-label={homeMethod.label}>
          {researchMethods.map((item) => (
            <Button
              key={item.id}
              id={`research-method-${item.id}`}
              className="research-method"
              disabled={homeMethod.disabled}
              onClick={() =>
                execute(() => {
                  // 研究首页新开的研究不写回之前的草稿。
                  setFromHub(false);
                  // 草稿页签带来的参数和要写回的草稿只属于从那里进来的那次研究。
                  update({ researchTarget: null, researchSeed: null });
                  chooseMethod(item.id);
                })
              }
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
              </span>
              <span className="research-method-arrow" aria-hidden="true">
                ›
              </span>
            </Button>
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
                    ? `${envelope.model_version.model_revision.version.id}@${envelope.model_version.model_revision.version.revision}`
                    : tr('模型修订待核对', 'Model revision unverified')}{' '}
                  ·{' '}
                  {envelope
                    ? remoteIdentity?.host || tr('本机', 'Local')
                    : tr('运行位置待核对', 'Machine unverified')}
                </p>
                <p className="path">{studyDir}</p>
              </div>
              <div className="research-progress">
                {envelope ? currentState : tr('状态待核对', 'Status unverified')}
                <span>
                  {completed === null
                    ? tr('尚无候选完成记录', 'No candidate progress recorded')
                    : `${tr('已完成候选', 'Completed candidates')}: ${completed}`}
                </span>
              </div>
              {renderActions(homeActions.slice(1))}
            </div>
          ) : (
            <div className="research-empty">
              <p>
                {tr(
                  '尚未打开研究任务。选择上方方法开始，或打开已有研究。',
                  'No study is open. Choose a method above or open an existing study.',
                )}
              </p>
              <p>
                {tr(
                  '这里显示本次打开的研究及其真实状态。',
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
  const metricNames: Record<string, [string, string]> = {
    nrmse: ['NRMSE 归一化均方根误差（越小越好）', 'NRMSE (lower is better)'],
    mae: ['MAE 平均绝对误差（越小越好）', 'MAE (lower is better)'],
    abs_bias: ['绝对偏差（越小越好）', 'Absolute bias (lower is better)'],
    nse: ['NSE 效率系数（越大越好）', 'NSE (higher is better)'],
    kge: ['KGE 效率系数（越大越好）', 'KGE (higher is better)'],
    r2: ['R² 决定系数（越大越好）', 'R² (higher is better)'],
    r: ['r 相关系数（越大越好）', 'Correlation r (higher is better)'],
  };
  const chosenCount = Object.values(parameters).filter((p) => p.enabled).length;
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
      `${b.candidates} 组参数 + 1 组基准，${sites.length} 个站点，共 ${b.tasks} 次模拟`,
      `${b.candidates} parameter sets + 1 baseline across ${sites.length} sites: ${b.tasks} simulations`,
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
      return { ...old, [String(i)]: { ...p, ...patch } };
    });
  const plannedTotal = (() => {
    const b = frozenSpec?.budget as Record<string, unknown> | undefined;
    if (!b) return null;
    const c = Number(b.candidate_count);
    if (c > 0) return c;
    const p = Number(b.population),
      g = Number(b.generations);
    return p > 0 && g >= 0 ? p * (g + 1) : null;
  })();
  const ranked = Object.entries(envelope?.state?.candidates ?? {})
    .sort(
      ([, a], [, b]) =>
        (a.calibration ?? Number.POSITIVE_INFINITY) - (b.calibration ?? Number.POSITIVE_INFINITY),
    )
    .slice(0, 200);
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
  const controls = lifecycleActions.filter((a) => controlIds.includes(a.id) && a.enabled);
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
  const parameterTable = (
    <>
      {state.researchSeed?.parameters.length && !contexts.length ? (
        <Notice>
          {tr(
            `从版本页带来 ${state.researchSeed.parameters.length} 个参数，读出参数后会自动勾上。`,
            `${state.researchSeed.parameters.length} parameters came from the version page and are ticked once loaded.`,
          )}
        </Notice>
      ) : null}
      {seedNotice && <Notice tone="success">{seedNotice}</Notice>}
      {!contexts.length ? (
        <p className="versions-empty">
          {busy
            ? tr('正在读取这些算例能研究的参数…', 'Reading the parameters these cases can study…')
            : tr('还没读到参数。', 'No parameters loaded yet.')}{' '}
          <button
            type="button"
            className="library-action"
            disabled={!designActions[0]!.enabled}
            onClick={() => execute(designActions[0]!.run)}
          >
            {tr('重新读取', 'Load again')}
          </button>
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
                // 很多全局参数目录里还没有中文名，标签就是键名换成空格，这时只显示一次键名。
                const unnamed = label.replace(/\s+/g, '_').toUpperCase() === row.name.toUpperCase();
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
      </p>
    </>
  );
  const footer = (back: boolean, next?: { label: string; enabled: boolean; to: 1 | 2 | 3 | 4 }) => (
    <div className="study-footer">
      {back && (
        <Button variant="ghost" onClick={() => setStep((step - 1) as 1 | 2 | 3)}>
          ← {tr('上一步', 'Back')}
        </Button>
      )}
      {next && (
        <Button variant="primary" disabled={!next.enabled} onClick={() => setStep(next.to)}>
          {next.label} →
        </Button>
      )}
    </div>
  );
  return (
    <div className="research-workbench study-workbench">
      {state.simulationReturn && (
        <div className="button-row">
          <Button onClick={returnToSimulation}>
            ← {tr('返回模拟配置', 'Back to simulation setup')}
          </Button>
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
            <button
              type="button"
              className="library-action"
              disabled={busy || launching}
              onClick={() => update({ researchTarget: null })}
            >
              {tr('改为另建新版本', 'Use a new version instead')}
            </button>
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
            <button type="button" disabled={!canStep(n)} onClick={() => setStep(n)}>
              <span className="study-step-number">{n < step ? '✓' : n}</span>
              {label}
            </button>
          </li>
        ))}
      </ol>
      {error && <Notice tone="error">{error}</Notice>}

      {step === 1 && (
        <Card
          title={tr('选算例与运行位置', 'Cases and where to run')}
          description={tr(
            '研究在这些算例上反复运行模型。目前只支持单点算例；选多个站点时参数在所有站点共用。',
            'The study runs the model repeatedly on these cases. Only single-point cases are supported for now; with several sites the parameters are shared.',
          )}
        >
          {state.cases.length === 0 ? (
            <p className="versions-empty">
              {tr(
                '当前项目里还没有算例。先在"项目"里打开项目或新建模拟。',
                'The current project has no cases yet. Open a project or create a simulation first.',
              )}
            </p>
          ) : (
            <div className="study-cases">
              {state.cases.map((c) => {
                const on = sites.includes(c.dir);
                return (
                  <label
                    key={c.dir}
                    className={`study-case${on ? ' is-on' : ''}${c.spatial ? ' is-off' : ''}`}
                  >
                    <input
                      type="checkbox"
                      checked={on}
                      disabled={c.spatial || busy || launching}
                      onChange={(e) => {
                        sitesTouched.current = true;
                        setSites((old) =>
                          e.target.checked
                            ? [...new Set([...old, c.dir])]
                            : old.filter((d) => d !== c.dir),
                        );
                      }}
                    />
                    <span>
                      <strong>{c.name}</strong>
                      <small>
                        {c.spatial
                          ? tr('格点算例，研究暂不支持', 'Gridded case, not supported yet')
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
                {state.kernels.map((k) => (
                  <option key={k.dir} value={k.dir}>
                    {k.preset}
                    {k.macros.includes('SinglePoint') ? '' : tr('（区域）', ' (regional)')}
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
              <button
                type="button"
                className="library-action"
                onClick={() => navigate('resources')}
              >
                {tr('计算资源 →', 'Compute resources →')}
              </button>
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
              <Select value={slot} onChange={(e) => chooseSlot(e.target.value)}>
                {(tab === 'process' ? ['soil_stress'] : ['pft', 'land_class']).map((v) => (
                  <option key={v} value={v}>
                    {v === 'pft'
                      ? tr('每个植物功能型', 'Each plant functional type')
                      : v === 'land_class'
                        ? tr('每个地类', 'Each land class')
                        : tr('土壤水分胁迫 β', 'Soil-water stress β')}
                  </option>
                ))}
              </Select>
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
              ? tr(
                  '率定按观测找参数：每个站点选一个观测文件，定好率定期（可再留一段验证期）和比较指标。',
                  'Calibration fits parameters to observations: choose an observation file per site, a calibration period (optionally a validation period) and a metric.',
                )
              : tr(
                  '不确定性分析在参数范围里抽样，看输出变化有多大。',
                  'Uncertainty analysis samples the parameter ranges and shows how much the outputs vary.',
                )
          }
        >
          <Field
            label={
              tuning
                ? tr(
                    '目标变量（逗号分隔，如 Qle, Qh）',
                    'Target variables (comma separated, e.g. Qle, Qh)',
                  )
                : tr('输出变量（逗号分隔）', 'Output variables (comma separated)')
            }
          >
            <Input value={variables} onChange={(e) => setVariables(e.target.value)} />
          </Field>
          {tuning && (
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
                      <Button onClick={() => pickObservation(name)}>
                        {tr('选择…', 'Choose…')}
                      </Button>
                    </div>
                  );
                })}
              </div>
              <div className="study-grid">
                <Field label={tr('率定期开始（UTC）', 'Calibration start (UTC)')}>
                  <Input
                    type="datetime-local"
                    value={from}
                    onChange={(e) => setFrom(e.target.value)}
                  />
                </Field>
                <Field label={tr('率定期结束（UTC）', 'Calibration end (UTC)')}>
                  <Input type="datetime-local" value={to} onChange={(e) => setTo(e.target.value)} />
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
                <Field label={tr('比较指标', 'Metric')}>
                  <Select value={metric} onChange={(e) => setMetric(e.target.value)}>
                    {Object.entries(metricNames).map(([value, [zh, english]]) => (
                      <option key={value} value={value}>
                        {en ? english : zh}
                      </option>
                    ))}
                  </Select>
                </Field>
                <Field label={tr('每个站点至少多少对观测', 'Minimum observation pairs per site')}>
                  <Input
                    type="number"
                    value={minPairs}
                    onChange={(e) => setMinPairs(e.target.value)}
                  />
                </Field>
                <Field
                  label={tr(
                    '种群大小（每代多少组参数）',
                    'Population (parameter sets per generation)',
                  )}
                >
                  <Input
                    type="number"
                    value={population}
                    onChange={(e) => setPopulation(e.target.value)}
                  />
                </Field>
                <Field label={tr('迭代代数', 'Generations')}>
                  <Input
                    type="number"
                    value={generations}
                    onChange={(e) => setGenerations(e.target.value)}
                  />
                </Field>
              </div>
              {ai && (
                <label className="param-check">
                  <input
                    type="checkbox"
                    checked={trainNetwork}
                    onChange={(e) => setTrainNetwork(e.target.checked)}
                  />
                  {tr(
                    '这次研究直接用差分进化训练网络（不选就只率定上一步的参数）',
                    'Train the network with differential evolution in this study',
                  )}
                </label>
              )}
            </>
          )}
          {!tuning && (
            <div className="study-grid">
              <Field label={tr('抽样方法', 'Sampling')}>
                <Select value={method} onChange={(e) => setMethod(e.target.value)}>
                  <option value="lhs">
                    {tr('拉丁超立方（整体不确定性）', 'Latin hypercube (overall uncertainty)')}
                  </option>
                  <option value="oat">
                    {tr('一次改一个（逐个看敏感性）', 'One at a time (per-parameter sensitivity)')}
                  </option>
                </Select>
              </Field>
              {method === 'lhs' && (
                <Field label={tr('样本数', 'Samples')}>
                  <Input type="number" value={count} onChange={(e) => setCount(e.target.value)} />
                </Field>
              )}
            </div>
          )}
          <Advanced title={tr('更多设置', 'More settings')}>
            <div className="study-grid">
              <Field label={tr('同时运行几个模拟（1–16）', 'Simulations at a time (1–16)')}>
                <Input type="number" value={jobs} onChange={(e) => setJobs(e.target.value)} />
              </Field>
              <Field label={tr('随机种子', 'Random seed')}>
                <Input type="number" value={seed} onChange={(e) => setSeed(e.target.value)} />
              </Field>
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
          <div className="study-footer">
            <Button variant="ghost" onClick={() => setStep(2)}>
              ← {tr('上一步', 'Back')}
            </Button>
            <Button
              disabled={!designActions[1]!.enabled}
              onClick={() => execute(designActions[1]!.run)}
            >
              {tr('预检', 'Preflight')}
            </Button>
            <Button
              variant="primary"
              disabled={!designActions[2]!.enabled}
              onClick={() => execute(designActions[2]!.run)}
            >
              {tr('创建研究 →', 'Create study →')}
            </Button>
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
              <span
                className={`badge study-state is-${(remotePermissions.exists ? remoteStatus : status).toLowerCase()}`}
              >
                {statusText(remotePermissions.exists ? remoteStatus : status)}
                {launching ? tr(' · 正在执行', ' · working') : ''}
              </span>
            </div>
            {(() => {
              const done = completed ?? 0;
              const pct = plannedTotal
                ? Math.min(100, Math.round((done / plannedTotal) * 100))
                : null;
              return (
                <div className="study-progress">
                  <div className="study-progress-bar">
                    <span style={{ width: `${pct ?? 0}%` }} />
                  </div>
                  <span>
                    {plannedTotal
                      ? tr(
                          `已完成 ${done} / ${plannedTotal} 组参数`,
                          `${done} of ${plannedTotal} parameter sets done`,
                        )
                      : tr(`已完成 ${done} 组参数`, `${done} parameter sets done`)}
                    {best ? tr(` · 当前最优 ${best}`, ` · best so far ${best}`) : ''}
                  </span>
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
                    tr(
                      `${frozenSpec.base_cases.length} 个站点`,
                      `${frozenSpec.base_cases.length} site${frozenSpec.base_cases.length === 1 ? '' : 's'}`,
                    ),
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
            <div className="button-row">
              {renderActions(controls)}
              {renderActions([byId('study-status')!].filter((a) => a.enabled))}
            </div>
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
          {ranked.length > 0 && (
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
                  <Input
                    value={member}
                    placeholder={best ?? ''}
                    onChange={(e) => setMember(e.target.value)}
                  />
                </Field>
              </div>
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
              <Advanced title={tr('不经草稿，直接保存为版本', 'Save as a version without a draft')}>
                <p className="muted study-hint">
                  {tr(
                    '直接冻结成一个版本，跳过版本页的审阅和冻结检查。',
                    'Freezes a version directly, skipping the review and freeze checks on the versions page.',
                  )}
                </p>
                <Field label={tr('新版本名称', 'New version name')}>
                  <Input value={name} onChange={(e) => setName(e.target.value)} />
                </Field>
                <div className="button-row">{renderActions([byId('study-save-version')!])}</div>
              </Advanced>
            </Card>
          )}
          {studyDir && (
            <Card title={tr('结果文件', 'Result files')}>
              {(envelope?.results?.length ?? 0) > 0 ? (
                <>
                  <div className="study-grid">
                    <Field label={tr('选一个结果文件', 'Result file')}>
                      <Select value={resultPath} onChange={(e) => setResultPath(e.target.value)}>
                        <option value="">—</option>
                        {(envelope?.results || []).map((v) => {
                          const path = typeof v === 'string' ? v : v.path;
                          return (
                            <option key={path} value={path}>
                              {path}
                            </option>
                          );
                        })}
                      </Select>
                    </Field>
                  </div>
                  <div className="button-row">{renderActions([byId('study-results')!])}</div>
                </>
              ) : (
                <p className="versions-empty">{tr('还没有结果文件。', 'No result files yet.')}</p>
              )}
              {result && <StudyResult text={result} theme={state.theme} />}
              <Advanced title={tr('导出研究', 'Export the study')}>
                <Field label={tr('导出到哪个目录', 'Export to')}>
                  <Input value={out} onChange={(e) => setOut(e.target.value)} />
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
              <Field label={tr('拟合结果放在哪个目录', 'Fit output directory')}>
                <Input value={out} onChange={(e) => setOut(e.target.value)} />
              </Field>
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
          onClose={cancelWriteBack}
          onImport={(chosen) => execute(() => confirmWriteBack(chosen))}
        />
      )}
    </div>
  );
}
