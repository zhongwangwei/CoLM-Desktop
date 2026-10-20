import { useCallback, useEffect, useRef, useState } from 'react';
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
import type { VersionRef } from '../../lib/types';
import { REMOTE_CHECK_MS } from '../runs/run-model';
import LiveServerLoad from '../runs/LiveServerLoad';
import { researchMethods, methodFromStudy, completedCandidates } from './research-home';
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
      setResult('');
      setFit(null);
      if (value === 'process') chooseSlot('soil_stress');
      else if (value === 'parameters') chooseSlot('pft');
    }
    setWorkbench(true);
  }
  function showHome() {
    if (lock.current || runLock.current) throw new Error('A study operation is in progress');
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
        (v) =>
          setSites((old) =>
            v === 'true' ? [...new Set([...old, c.dir])] : old.filter((d) => d !== c.dir),
          ),
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
      'study-save-version',
      tr('保存为新模型版本', 'Save as new model version'),
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
  const modelUsed = envelope?.model_version?.model_revision?.version;
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
    tr('返回研究首页', 'Back to research'),
    showHome,
    !launching,
  );
  const completed = completedCandidates(
    remoteReply?.study?.completed_candidates ?? envelope?.state?.completed_candidates,
  );
  const currentState = remotePermissions.exists ? remoteStatus : status;
  const methodTitle = researchMethods.find((item) => item.id === tab)!;
  const currentMethod = researchMethods.find((item) => item.id === methodFromStudy(frozenSpec))!;
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

  function renderFields(fields: UiField[]) {
    return fields.map((f) => (
      <Field key={f.id} label={f.label}>
        {f.type === 'choice' ? (
          <Select
            id={f.id}
            value={String(f.value)}
            disabled={f.disabled}
            onChange={(e) => f.set(e.target.value)}
          >
            {f.options?.map((o) => (
              <option key={o.value} value={o.value}>
                {o.label}
              </option>
            ))}
          </Select>
        ) : f.type === 'switch' ? (
          <input
            id={f.id}
            type="checkbox"
            checked={f.value === 'true'}
            disabled={f.disabled}
            onChange={(e) => f.set(String(e.target.checked))}
          />
        ) : f.id === 'research-observations' ? (
          <textarea
            id={f.id}
            rows={3}
            className="w-full"
            value={String(f.value)}
            disabled={f.disabled}
            onChange={(e) => f.set(e.target.value)}
          />
        ) : (
          <Input
            id={f.id}
            type={
              f.id.includes('-from') || f.id.includes('-to')
                ? 'datetime-local'
                : f.type === 'number'
                  ? 'number'
                  : 'text'
            }
            value={String(f.value)}
            disabled={f.disabled}
            onChange={(e) => f.set(e.target.value)}
          />
        )}
      </Field>
    ));
  }
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
                '先明确研究目标，再选择方法。',
                'Start with a research question, then choose a method.',
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
              onClick={() => execute(() => chooseMethod(item.id))}
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
  return (
    <div className="research-workbench space-y-5">
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
        {renderActions([backAction])}
      </header>
      <Card
        title={tr('研究工作台', 'Research workbench')}
        description={tr(
          '率定、敏感性与 AI 训练均使用内置后端；每个产物保留研究范围。',
          'Calibration, sensitivity and AI training use the built-in backend with explicit applicability.',
        )}
      >
        <div className="grid gap-3 md:grid-cols-2">{renderFields(baseFields)}</div>
        <div className="actions">{renderActions([resourcesAction])}</div>
        {host && servers.includes(host) && (
          <LiveServerLoad host={host} partition={serverPartitions.current[host]} en={en} />
        )}
        {host && (
          <Notice>
            {tr(
              '服务器认证、资源配额与数据路径映射沿用计算资源页面的配置。研究提交后独立运行，按真实状态监控；结束后取回并校验结果。',
              'Server authentication, quotas and input mappings use the compute resources settings. Submitted studies run independently; monitor their actual state, then fetch verified results.',
            )}
          </Notice>
        )}
      </Card>
      {error && <Notice tone="error">{error}</Notice>}
      {ai && (
        <Card title={tr('AI 网络设计', 'AI network design')}>
          <Notice>
            {tr(
              'AI 网络仅支持 Rust。过程 β 不适用于 PC 或已启用植物水力的配置；预检以实际算例为准。',
              'AI networks require Rust. Soil-stress β requires compatible land cover and disabled plant hydraulics; preflight checks the actual cases.',
            )}
          </Notice>
          <div className="grid gap-3 md:grid-cols-2">{renderFields(networkFields)}</div>
          <div className="flex flex-wrap gap-2">{renderActions(aiActions.slice(0, 3))}</div>
          {hybridInfo && <pre className="overflow-auto">{JSON.stringify(hybridInfo, null, 2)}</pre>}
        </Card>
      )}
      <Card title={tr('参数范围与研究设计', 'Parameter ranges and study design')}>
        <div className="flex gap-2">{renderActions(designActions.slice(0, 1))}</div>
        {state.researchSeed?.parameters.length && !contexts.length ? (
          <Notice>
            {tr(
              `从版本页带来 ${state.researchSeed.parameters.length} 个要研究的参数：选好算例和编译配置、读取参数后会自动勾上。`,
              `${state.researchSeed.parameters.length} parameters came from the version page; they are ticked once cases and a build configuration are chosen and parameters are loaded.`,
            )}
          </Notice>
        ) : null}
        {seedNotice && <Notice tone="success">{seedNotice}</Notice>}
        {!contexts.length && (
          <p>
            {tr(
              '选择编译配置后读取共同适用参数；不默认猜测采样上下界。',
              'Load parameters applicable to the selected cases. Sampling bounds must be entered explicitly.',
            )}
          </p>
        )}
        <div className="grid gap-3 md:grid-cols-2">
          {renderFields(parameterFields)}
          {renderFields(designFields)}
        </div>
        <Notice>{budgetText}</Notice>
        <p>
          {tr(
            '校准与验证时段不重叠；时间按 UTC。研究包含真实基准任务，仅在明确启动后运行。',
            'Training and validation windows are disjoint UTC intervals. Baseline tasks run only when you explicitly start the study.',
          )}
        </p>
        <div className="flex gap-2">{renderActions(designActions.slice(1))}</div>
        {preflight && <pre className="overflow-auto">{preflight}</pre>}
      </Card>
      <Card title={tr('研究任务与产物', 'Study lifecycle and products')}>
        <div className="grid gap-3 md:grid-cols-2">{renderFields(lifecycleFields)}</div>
        {frozenSpec && (
          <Notice>
            <p>
              {tr(
                '已冻结研究：运行使用以下设定与模型修订；修改上方设计后需创建新研究。',
                'Frozen study: execution uses this design and model revision. Create a new study to apply edits above.',
              )}{' '}
              {modelUsed
                ? `${modelUsed.id}@${modelUsed.revision}`
                : tr(
                    '冻结修订身份未知；请查看原版本出处。',
                    'Frozen revision unknown; inspect the recorded source version.',
                  )}
            </p>
            <pre className="overflow-auto">
              {JSON.stringify(
                {
                  kind: frozenSpec.kind,
                  method: frozenSpec.method,
                  budget: frozenSpec.budget,
                  cases: Array.isArray(frozenSpec.base_cases)
                    ? frozenSpec.base_cases.length
                    : undefined,
                  parameters: Array.isArray(frozenSpec.parameters)
                    ? frozenSpec.parameters.length
                    : undefined,
                  targets: Array.isArray(frozenSpec.targets)
                    ? frozenSpec.targets.length
                    : undefined,
                },
                null,
                2,
              )}
            </pre>
          </Notice>
        )}
        <p role="status">
          {remotePermissions.exists ? remoteStatus : status}{' '}
          {launching ? tr('· 命令正在执行', '· command active') : ''} ·{' '}
          {remoteReply?.study?.completed_candidates ?? envelope?.state?.completed_candidates ?? '—'}{' '}
          {tr('已完成候选', 'completed candidates')}
        </p>
        {remotePermissions.exists && (
          <div className="space-y-3">
            <p>
              {tr('服务器', 'Server')}: {remoteIdentity?.host || host} · {tr('作业', 'Job')}:{' '}
              {remoteIdentity?.job || '—'} · {remoteReply?.status?.phase}
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
                  '取消请求已受理，仍等待服务器的真实退出状态。',
                  'Cancellation was accepted; a real server exit is still required.',
                )}
              </Notice>
            )}
            {(remoteReply?.status?.state === 'lost' ||
              remoteReply?.status?.state === 'unknown' ||
              remoteReply?.outcome === 'uncertain') && (
              <Notice tone="warning">
                {tr(
                  '服务器状态或操作结果尚未核对，不能当作完成、失败或已取消，也不会重复提交。',
                  'The server state or action outcome is unconfirmed. It is not treated as completion, failure or cancellation, and will not be resubmitted.',
                )}
              </Notice>
            )}
            {remotePermissions.fetch && (
              <Notice>
                {tr('服务器已退出，退出码', 'Server exited with code')}{' '}
                {remoteReply?.status?.exit_code}。
                {tr(
                  '取回后将核对冻结身份与研究成员，再读取有效结果。',
                  'Fetch verifies frozen identity and members before reading valid results.',
                )}
              </Notice>
            )}
            {remotePermissions.fetched && (
              <Notice tone={remotePermissions.scientificVerified === true ? 'success' : 'warning'}>
                {remotePermissions.scientificVerified === true
                  ? tr(
                      '服务器产物已取回，完整科学身份核对已通过；模型改进仍需以训练和验证指标为依据。',
                      'Server artifacts are fetched and full scientific identity is verified. Model improvement still requires training and validation evidence.',
                    )
                  : tr(
                      '服务器诊断产物已取回；未提供或未通过完整科学身份核对，不能据此声称模型改进。',
                      'Server diagnostic artifacts are fetched. Full scientific identity is absent or unverified; this does not establish model improvement.',
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
        <div className="flex flex-wrap gap-2">{renderActions(lifecycleActions)}</div>
        <Notice>
          {tr(
            '产物保存为新的不可变模型修订，原算例不被替换。缺失同条件基准时不解释优劣；不会追加计算。',
            'Products become new immutable model revisions. Comparison requires a real baseline under the same conditions; no additional runs are started.',
          )}
        </Notice>
        {envelope?.state?.candidates && (
          <table>
            <thead>
              <tr>
                <th>{tr('候选成员', 'Member')}</th>
                <th>{tr('可行', 'Feasible')}</th>
                <th>{tr('训练损失', 'Training loss')}</th>
                <th>{tr('验证损失', 'Validation loss')}</th>
                <th>{tr('原因', 'Reason')}</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(envelope.state.candidates)
                .slice(0, 1000)
                .map(([key, c]) => (
                  <tr key={key}>
                    <td>{key}</td>
                    <td>{String(c.feasible ?? false)}</td>
                    <td>{c.calibration ?? '—'}</td>
                    <td>{c.validation ?? '—'}</td>
                    <td>{c.reason}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        )}
        {result && <StudyResult text={result} theme={state.theme} />}
      </Card>
      {ai && (
        <Card title={tr('从已完成率定学习参数', 'Learn parameters from completed calibration')}>
          <div className="flex flex-wrap gap-2">{renderActions(aiActions.slice(3))}</div>
          {renderFields(
            fitFields.filter(
              (f) => !f.id.startsWith('fit-ridge') && !f.id.startsWith('fit-epochs'),
            ),
          )}
          {fit && (
            <>
              <Notice>
                {fitValidation(fit.report) === true
                  ? tr('留一研究验证已通过。', 'Leave-one-study-out validation passed.')
                  : fitValidation(fit.report) === false
                    ? tr(
                        '留一研究验证未通过；禁止将此拟合保存为可部署模型。',
                        'Leave-one-study-out validation failed; saving this fit as a deployable model is disabled.',
                      )
                    : tr(
                        '未提供独立交叉验证；不能推断在新站点上的改进。',
                        'No independent cross-validation; improvement at new sites is unproven.',
                      )}
              </Notice>
              <Notice>
                {tr(
                  '拟合产物和留一验证报告已生成。未通过适用性与验证门槛不能据此声称优于基准。',
                  'Fit artifacts and leave-one-study-out report are available. Applicability and validation gates must pass before claiming an improvement.',
                )}
              </Notice>
              <pre className="overflow-auto">{JSON.stringify(fit, null, 2)}</pre>
            </>
          )}
        </Card>
      )}
      <Advanced
        title={tr('专家设置与研究出处', 'Expert settings and provenance')}
        open={advanced}
        onOpenChange={setAdvanced}
      >
        {ai && renderFields(fitFields.filter((f) => f.id === 'fit-ridge' || f.id === 'fit-epochs'))}
        {renderFields(remoteFields)}
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
    </div>
  );
}
