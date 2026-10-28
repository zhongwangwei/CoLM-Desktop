import { useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke, listen } from '../../lib/ipc';
import { usePageContract, type UiField } from '../../lib/guide';
import { ActionButton } from '../../components/ActionButton';
import {
  Button,
  Card,
  Field,
  Input,
  DateInput,
  Select,
  Advanced,
  Notice,
} from '../../components/ui';
import {
  caseDirectory,
  displayLiteral,
  cropEnabled,
  requiredNumber,
  physicsFields,
  versionMode,
  siteVersionMismatch,
  effectiveModelFields,
  isCaseField,
  selectChoice,
  type SchemaField,
  type Site,
} from './case-model';
import {
  caseDatePatch,
  newCaseProblem,
  newCaseRangeProblem,
  newCaseGeneralProblem,
  modelSetup,
  scannedForcing,
  setupRechecks,
  simulationFields,
  simulationSteps,
  isSiteConfiguration,
  isForcingScheme,
  elapsedLabel,
  kindPathFields,
  nodePlan,
  nodePlanText,
  kindPathPrefill,
  spinupProblem,
  type ModelSetup,
  type SetupRecheck,
  type SimulationStep,
} from './simulation-model';
import { fieldInputOptions, fieldLabel, optionLabel } from '../../lib/param-presentation';
import {
  caseFieldVisible,
  appendTracerFamily,
  applyTracerSelection,
  tracerRows,
  tracerKeys,
  unquote,
  type ConfigurationField,
} from '../versions/configuration-options';
import {
  compatibleKernel,
  localCoreCount,
  SITE_THREADS,
  siteBatchConcurrency,
} from '../runs/run-model';
import { configurationReason, simulationOptionField, modelSettingValue } from './model-settings';
import SimulationSteps from './SimulationSteps';
import ModelSettings from './ModelSettings';
import {
  autoClosedText,
  exclusionPatch,
  regionalRoutingPatch,
  schemeFlagKeys,
  simulationMode,
  caseSchemeFields,
  type SimulationMode,
} from './model-settings';
import type { CaseOptionPatch } from '../versions/configuration-options';
import HistoryVariablesCard from './HistoryVariablesCard';
import MultiSiteSettings from './MultiSiteSettings';
import { planMultiSites, type MultiSiteDraft, type MultiSiteMode } from './multisite-model';
import type { CaseEntry, DataDefaults } from '../../lib/types';
import RunLocationSettings from '../runs/RunLocationSettings';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import type { Server } from '../resources/resource';
import SpatialForcingFields, { type ForcingLayoutProbe } from './SpatialForcingFields';
import EngineProgress from './EngineProgress';
import TaskCard from '../../components/TaskCard';
import type { BuildProgress } from './engine-progress';

function isMethaneSite(site: Site) {
  return (
    !site.urban &&
    !site.crop &&
    (site.name === 'AT-Neu' || /ch4/i.test(site.obs_file ?? '') || /ch4/i.test(site.site_file))
  );
}

function siteInfo(s: Site, t: (zh: string, en: string) => string) {
  if (s.urban)
    return {
      badge: t('城市过程', 'Urban'),
      desc: t('城市冠层与微气象 (Urban-PLUMBER)', 'Urban canopy & microclimate (Urban-PLUMBER)'),
      adaptHint: t(
        '点击选择：自动开启城市过程并关闭 BGC',
        'Select: enables urban processes and disables BGC',
      ),
      activeHint: t('✓ 当前已选定 · 城市方案已就绪', '✓ Selected · Urban scheme active'),
    };
  if (s.crop)
    return {
      badge: t('农田作物', 'Crop'),
      desc: t('农田生态与生长发育 (FLUXNET-CROP)', 'Crop phenology & agriculture (FLUXNET-CROP)'),
      adaptHint: t(
        '点击选择：自动采用 PFT、BGC 与作物默认配置',
        'Select: uses PFT, BGC and the crop defaults',
      ),
      activeHint: t('✓ 当前已选定 · 作物方案已就绪', '✓ Selected · Crop scheme active'),
    };
  if (isMethaneSite(s))
    return {
      badge: t('甲烷 / 示踪', 'Methane / Tracer'),
      desc: t('草地甲烷过程 (FLUXNET-CH4)', 'Grassland methane (FLUXNET-CH4)'),
      adaptHint: t(
        '点击选择：自动开启 BGC 与甲烷示踪，采用 PFT/PC 和 van Genuchten',
        'Select: enables BGC and methane tracing with PFT/PC and van Genuchten',
      ),
      activeHint: t('✓ 当前已选定 · 甲烷过程已配置', '✓ Selected · Methane processes configured'),
    };
  if (s.name === 'CN-Cng')
    return {
      badge: t('自然草地', 'Grassland'),
      desc: t('常规水热能量基准 (FLUXNET2015)', 'Baseline water & heat (FLUXNET2015)'),
      adaptHint: t('点击选择该站点', 'Click to select this station'),
      activeHint: t('✓ 当前已选定', '✓ Selected'),
    };
  return {
    badge: t('自然植被', 'Natural'),
    desc: t('自然植被基准', 'Natural baseline'),
    adaptHint: t('点击选择该站点', 'Click to select this station'),
    activeHint: t('✓ 当前已选定', '✓ Selected'),
  };
}

// Engines already confirmed on a server this session, keyed by host and model revision.
// The engine only changes with the app's own source, so one confirmation per session is enough.
const preparedEngines = new Set<string>();
type EngineState = {
  host: string;
  /**
   * checking/building：正在准备；ready：刚确认完，短暂告诉用户这次是复用还是新编；
   * failed：准备失败，读取站点/创建算例会再试。
   */
  phase: 'checking' | 'building' | 'ready' | 'failed';
  since: number;
  error?: string;
  /** `ready` 时：这次准备的源码快照与是否复用了服务器上已有的引擎。 */
  id?: string;
  cached?: boolean;
};

export default function NewCasePage() {
  const { state, update, openCase, navigate, notify } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const seed = state.newCaseSeed;
  const initialRoot = seed?.form?.root ?? state.root ?? '',
    initialName = seed?.form?.name ?? '';
  const initialLandcover = seed?.form?.landcover ?? '';
  const initialSubgrid = seed?.form?.subgrid ?? (seed?.form?.landcover ? seed.form.landcover : '');
  const [form, setForm] = useState<Record<string, string>>({
    // 计算引擎、范围、网格与下垫面方案都必须由用户明确选择，不预选任何一项。
    engine: '',
    host: '',
    root: initialRoot,
    name: initialName,
    kind: '',
    landcover: initialLandcover,
    subgrid: initialSubgrid,
    sitedir: '',
    forcingdir: '',
    site: '',
    met: '',
    rawdata: '',
    runtime: '',
    start: '',
    end: '',
    spinup_end: '',
    grid: '',
    west: '',
    east: '',
    south: '',
    north: '',
    dlon: '',
    dlat: '',
    mesh: '',
    catchment: '',
    shapefile: '',
    forcing: '',
    forcing_mode: seed?.form?.forcing ? 'config' : 'directory',
    spatial_forcingdir: '',
    forcing_dataset: '',
    timestep: '1800',
    run_threads: '',
    run_ranks: '1',
    run_workers: '',
    run_scheduler: '',
    run_cpus: '',
    ...seed?.form,
    ...(['PFT', 'PC'].includes(seed?.form?.subgrid ?? '') ? { landcover: 'IGBP' } : {}),
  });
  const [servers, setServers] = useState<Server[] | null>(null);
  const [resourceError, setResourceError] = useState('');
  const [forcingDatasets, setForcingDatasets] = useState<string[]>([]);
  const [forcingCatalogError, setForcingCatalogError] = useState('');
  const forcingSequence = useRef(0);
  const forcingAutoDates = useRef<Record<string, string>>({});
  const forcingDateEdits = useRef(new Set<string>());
  const [forcingProbe, setForcingProbe] = useState<ForcingLayoutProbe | null>(null);
  const templateForcing = form.kind !== 'site' && form.forcing_mode !== 'config';
  useEffect(() => {
    if (!templateForcing || !form.kind) return;
    let current = true;
    void invoke<string[]>('forcing_datasets')
      .then((datasets) => {
        if (current) {
          setForcingDatasets(datasets);
          setForcingCatalogError('');
        }
      })
      .catch((error) => {
        if (current) setForcingCatalogError(String(error));
      });
    return () => {
      current = false;
    };
  }, [templateForcing, form.kind]);
  const resourceServer = servers?.find((server) => server.host === form.host);
  const [localData, setLocalData] = useState<DataDefaults | null>(null);
  useEffect(() => {
    let current = true;
    invoke<{ servers: Server[]; local?: DataDefaults }>('remote_config', {})
      .then((config) => {
        if (!current) return;
        setServers(config.servers);
        setLocalData({
          project_dir: '',
          rawdata_dir: '',
          runtime_dir: '',
          forcing_dir: '',
          ...config.local,
        });
      })
      .catch((e) => {
        if (current) {
          setServers([]);
          setResourceError(String(e));
        }
      });
    return () => {
      current = false;
    };
  }, []);
  // 每选定一次运行位置，用计算资源里登记的默认数据路径填一次空着的框。
  const filledDefaults = useRef<string | null>(null);
  const seededRoot = !!seed?.form?.root;
  useEffect(() => {
    if (servers === null || localData === null || filledDefaults.current === form.host) return;
    const defaults = form.host ? servers.find((server) => server.host === form.host) : localData;
    if (!defaults) return;
    // A project folder handed over by the page that opened this one stays; otherwise the
    // resource's default project folder replaces the last-used one.
    const keepRoot = !form.host && filledDefaults.current === null && seededRoot;
    filledDefaults.current = form.host;
    setForm((old) => ({
      ...old,
      root: keepRoot ? old.root : defaults.project_dir || old.root,
      rawdata: old.rawdata || defaults.rawdata_dir || '',
      runtime: old.runtime || defaults.runtime_dir || '',
    }));
  }, [form.host, servers, localData, seededRoot]);
  async function rememberDataPaths(spatial: boolean) {
    const paths = {
      project: form.root.trim() || null,
      rawdata: form.rawdata.trim() || null,
      runtime: form.runtime.trim() || null,
      forcing: spatial && templateForcing ? form.spatial_forcingdir.trim() || null : null,
    };
    try {
      await invoke('remote_remember_data_paths', { host: form.host || null, ...paths });
    } catch (e) {
      notify(t(`默认数据路径未能同步到计算资源：${e}`, `Could not sync default data paths: ${e}`));
      return;
    }
    const merge = <T extends Partial<DataDefaults>>(old: T): T => ({
      ...old,
      ...(paths.project ? { project_dir: paths.project } : {}),
      ...(paths.rawdata ? { rawdata_dir: paths.rawdata } : {}),
      ...(paths.runtime ? { runtime_dir: paths.runtime } : {}),
      ...(paths.forcing ? { forcing_dir: paths.forcing } : {}),
    });
    if (!alive.current) return;
    if (form.host)
      setServers((old) =>
        old ? old.map((server) => (server.host === form.host ? merge(server) : server)) : old,
      );
    else setLocalData((old) => (old ? merge(old) : old));
  }
  const [caseValues, setCaseValues] = useState<Record<string, string>>(seed?.caseValues ?? {}),
    [schema, setSchema] = useState<SchemaField[]>([]),
    [schemaReady, setSchemaReady] = useState(false),
    [modelFields, setModelFields] = useState<SchemaField[]>([]);
  const [historyVariables, setHistoryVariables] = useState<
    Array<{ name: string; long_name?: string }>
  >([]);
  const [sites, setSites] = useState<Site[]>(seed?.sites ?? []),
    [step, setStep] = useState<SimulationStep>(seed?.step ?? 1),
    [hint, setHint] = useState(''),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [scanning, setScanning] = useState(false),
    [engine, setEngine] = useState<EngineState | null>(null),
    [creationDir, setCreationDir] = useState<string | null>(null),
    [batchProgress, setBatchProgress] = useState<{
      stage: string;
      done: number;
      total: number;
    } | null>(null),
    [cancellingCreation, setCancellingCreation] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [ownData, setOwnData] = useState(!!seed?.form?.host || (seed?.ownData ?? false)),
    // 自动关闭提示只属于触发它的那一步，离开后不再残留。
    [autoClosed, setAutoClosed] = useState<{ text: string; step: number }>({ text: '', step: 0 });
  const stepProblem = (at: SimulationStep) => {
    try {
      return problem(at);
    } catch (e) {
      return String(e);
    }
  };
  const initialMultiSitePreference = useRef(seed?.multiSiteEnabled);
  const [multiSiteEnabled, setMultiSiteEnabled] = useState(seed?.multiSiteEnabled ?? true);
  const [multiSiteMode, setMultiSiteMode] = useState<MultiSiteMode>(
    seed?.multiSiteMode ?? 'first-year',
  );
  const [multiSiteRepeat, setMultiSiteRepeat] = useState(seed?.multiSiteRepeat ?? '1');
  const [siteDrafts, setSiteDrafts] = useState<MultiSiteDraft[]>(seed?.siteDrafts ?? []);
  const [multiResults, setMultiResults] = useState<
    Array<{
      site: string;
      name: string;
      dir: string;
      remoteDir?: string;
      root?: string;
      created?: boolean;
      error?: string;
    }>
  >([]);
  const [remotePicker, setRemotePicker] = useState<{
    key: string;
    resolve: (path: string | null) => void;
  } | null>(null);
  const multiSite = ownData && form.kind === 'site' && multiSiteEnabled;
  const singlePoint = form.kind === 'site';
  // 单点本地运行每站固定 1 线程：单点是垂直柱、时间步串行，主模型没有可拆的并行工作
  // （GUI 审查 2026-10-22）。空间算例本地单例默认用满本机逻辑核。
  const localSite = singlePoint && !form.host;
  const localCores = localCoreCount();
  const siteCount = multiSite ? Math.max(1, siteDrafts.length) : 1;
  const scheduler = form.run_scheduler || resourceServer?.scheduler || 'auto';
  // Direct execution allocates nothing, so the CPUs a case uses are simply its threads.
  const directRun = !!form.host && scheduler === 'bare' && !multiSite;
  // Rust 引擎只有一个进程：申请的核数就是线程数，不另填。只有 Fortran（MPI）要按进程填 CPU 数。
  const cpusFollowThreads = directRun || form.engine !== 'fortran';
  const runThreads = localSite
    ? String(SITE_THREADS)
    : form.run_threads || String(resourceServer?.threads || (form.host ? 1 : localCores));
  const runWorkers = multiSite
    ? form.run_workers || String(siteBatchConcurrency(siteCount, localCores))
    : '1';
  const runSettings = {
    host: form.host,
    threads: runThreads,
    ranks: form.engine === 'fortran' ? form.run_ranks : '1',
    workers: runWorkers,
    scheduler,
    cpus: cpusFollowThreads
      ? runThreads
      : form.run_cpus || String(resourceServer?.cpus || resourceServer?.threads || 1),
  };
  // A server run that needs more cores than one node has: how many nodes, or why it cannot be placed.
  const placement =
    form.host && !multiSite && runSettings.scheduler !== 'bare'
      ? nodePlan({
          cpus: Number(runSettings.cpus),
          ranks: Number(runSettings.ranks),
          coresPerNode: resourceServer?.cores_per_node ?? 0,
          nodes: resourceServer?.nodes ?? 0,
        })
      : null;
  // Paths remembered for this resource and simulation kind (site folders, boundary or grid
  // files, forcing). Filled once per resource/kind/grid/forcing mode, only into empty fields or
  // ones still holding an earlier prefill; prefills the user left alone are cleared when they no
  // longer apply, so a watershed shapefile never lingers in a region case.
  const autoFilled = useRef<Record<string, string>>({});
  const filledKind = useRef('');
  const pathSource =
    servers === null || localData === null
      ? null
      : form.host
        ? (servers.find((server) => server.host === form.host) ?? null)
        : localData;
  const kindKey = pathSource
    ? [form.host, form.kind, form.grid, form.forcing_mode, ownData].join('\n')
    : '';
  useEffect(() => {
    if (!pathSource || filledKind.current === kindKey) return;
    filledKind.current = kindKey;
    const fields =
      form.kind === 'site' && !ownData
        ? []
        : kindPathFields(form.kind, form.grid, form.forcing_mode);
    const { patch, filled } = kindPathPrefill(
      form,
      fields,
      pathSource.kind_paths?.[form.kind] ?? {},
      pathSource.forcing_dir || '',
      autoFilled.current,
    );
    autoFilled.current = filled;
    if (Object.keys(patch).length) setForm((old) => ({ ...old, ...patch }));
    // kindKey encodes the resource, kind, grid, forcing mode and data source.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [kindKey]);
  // Remembered site folders are read once on reaching the data step, so the site list is
  // there without another click. Typed folders wait for the button: a half-typed path is not
  // worth a (remote) scan.
  const autoRead = useRef('');
  useEffect(() => {
    if (
      step !== 3 ||
      form.kind !== 'site' ||
      !ownData ||
      !form.sitedir ||
      form.sitedir !== autoFilled.current.sitedir ||
      sites.length ||
      scanning ||
      busy ||
      state.busy
    )
      return;
    const key = [form.host, form.sitedir, form.forcingdir].join('\n');
    if (autoRead.current === key) return;
    autoRead.current = key;
    void readSites().catch(() => {});
    // readSites reads the same form fields listed here.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    step,
    form.kind,
    ownData,
    form.host,
    form.sitedir,
    form.forcingdir,
    sites.length,
    scanning,
    busy,
    state.busy,
  ]);
  async function rememberKindPaths(kind: string, paths: Record<string, string>) {
    const entries = Object.entries(paths).filter(([, value]) => value.trim());
    if (!kind || !entries.length) return;
    const host = form.host;
    try {
      await invoke('remote_remember_kind_paths', {
        host: host || null,
        kind,
        paths: Object.fromEntries(entries.map(([key, value]) => [key, value.trim()])),
      });
    } catch (e) {
      notify(t(`路径未能记到计算资源：${e}`, `Could not remember the paths: ${e}`));
      return;
    }
    const merge = <T extends { kind_paths?: Record<string, Record<string, string>> }>(
      old: T,
    ): T => ({
      ...old,
      kind_paths: {
        ...old.kind_paths,
        [kind]: {
          ...old.kind_paths?.[kind],
          ...Object.fromEntries(entries.map(([key, value]) => [key, value.trim()])),
        },
      },
    });
    if (!alive.current) return;
    if (host)
      setServers((old) =>
        old ? old.map((server) => (server.host === host ? merge(server) : server)) : old,
      );
    else setLocalData((old) => (old ? merge(old) : old));
  }
  useEffect(() => {
    if (!busy && (error || (multiSite && multiResults.length > 0)))
      document.querySelector('.main-pane')?.scrollTo({ top: 0 });
  }, [busy, error, multiSite, multiResults]);
  const [optionStates, setOptionStates] = useState<ConfigurationField[]>([]),
    [optionError, setOptionError] = useState(''),
    [cropDefaults, setCropDefaults] = useState<Record<string, string>>({});
  const exampleValues = useRef<Record<string, string>>(
    seed?.ownData
      ? {}
      : (seed?.exampleValues ??
          Object.fromEntries(
            ['sitedir', 'forcingdir', 'runtime', 'site', 'met', 'name', 'start', 'end'].map(
              (key) => [key, seed?.form?.[key] ?? ''],
            ),
          )),
  );
  const creation = useRef<{ dir: string; cancelRequested: boolean; batch?: string } | null>(null),
    lock = useRef(false),
    sequence = useRef(0),
    alive = useRef(true),
    classification = (form.landcover || undefined) as 'IGBP' | 'USGS' | undefined,
    mode: SimulationMode | undefined = form.subgrid
      ? simulationMode(undefined, classification, form.subgrid)
      : undefined,
    schemeFields = mode ? caseSchemeFields(mode) : {},
    physics = {
      ...effectiveModelFields(state.version?.overlay, modelFields),
      ...caseValues,
      ...schemeFields,
    },
    selected = sites.find((s) => s.site_file === form.site);
  // U-03：向导把"数据与时段"放在"模型结构与方案"之前，方案一改，前面核对过的数据就可能失效。
  // 每一步核对通过时记下当时的模型设置；之后比较即可标出哪一步需要重新检查，而不是只改步骤标题。
  const [checkedSetup, setCheckedSetup] = useState<Partial<Record<SimulationStep, ModelSetup>>>({});
  const currentSetup = modelSetup({
    engine: form.engine,
    mode: mode ?? '',
    classification: classification ?? '',
    physics,
    revision: `${state.versionRef.id}#${state.versionRef.revision}`,
  });
  // 数据步骤里"已选中的数据"：单点看当前站点，批量看草稿里的站点。用来判断方案改变后站点是否还兼容。
  const checkedSites = (
    multiSite
      ? siteDrafts
          .map((draft) => sites.find((item) => item.site_file === draft.site))
          .filter((item): item is Site => !!item)
      : selected
        ? [selected]
        : []
  ).map((item) => ({ name: item.name, crop: item.crop, urban: item.urban }));
  const staleNotices: SetupRecheck[] = ([2, 3, 5] as SimulationStep[]).flatMap((at) =>
    setupRechecks(
      checkedSetup[at] ?? null,
      currentSetup,
      at,
      { sites: checkedSites, runtimeFilled: !!form.runtime.trim() },
      en,
    ),
  );
  const bundledSite = !ownData && !multiSite && form.kind === 'site' ? selected : undefined;
  let projectDirectory = '',
    finalDirectory = '',
    destinationRoot = form.root,
    destinationError = '';
  const destinationName = bundledSite?.name ?? form.name;
  try {
    projectDirectory = caseDirectory(form.root, form.name);
    destinationRoot = bundledSite ? projectDirectory : form.root;
    finalDirectory = bundledSite
      ? caseDirectory(projectDirectory, bundledSite.name)
      : projectDirectory;
  } catch (e) {
    destinationError = String(e);
  }

  useEffect(() => {
    alive.current = true;
    update({ newCaseSeed: null, simulationReturn: null });
    return () => {
      alive.current = false;
    };
  }, [update]);
  // 新建模拟的进度从第二步起记进历史：向导里的内容只在这个页面里，切到别的页面或关掉软件就没了
  // （用户报：走到第三步切出去就找不回来）。从历史重新打开已建好的项目时不记，那个项目已有记录。
  const draft = useRef({
    id: (seed?.draftId ?? null) as string | null,
    record: seed?.recordDraft !== false,
    closed: false,
    queue: Promise.resolve() as Promise<unknown>,
  });
  function wizardSeed(at: SimulationStep) {
    return {
      form,
      sites,
      caseValues: {
        ...caseValues,
        ...schemeFields,
      },
      step: at,
      ownData,
      exampleValues: exampleValues.current,
      multiSiteEnabled,
      multiSiteMode,
      multiSiteRepeat,
      siteDrafts,
    };
  }
  // 记录失败不挡向导：这只是方便找回，写不进去时照样能建模拟。保存排成一队，免得第一次保存还没
  // 拿到记录号时，下一次又另起一条。
  // 第 1 步填的内容：起初的项目目录来自默认路径，只有改过目录或填了名称才算用户填过。
  const firstRoot = useRef(form.root);
  function saveDraft(at: SimulationStep) {
    const record = draft.current;
    // 第 1 步原来不记：填了目录和名称、点侧栏离开再回来，全都没了，历史里也找不到（2026-10-22 用户实测）。
    const filledFirstStep = !!form.name?.trim() || (form.root ?? '') !== (firstRoot.current ?? '');
    if (!record.record || record.closed || (at < 2 && !record.id && !filledFirstStep)) return;
    const snapshot = wizardSeed(at),
      name = (form.name ?? '').trim() || t('未命名模拟', 'Untitled simulation');
    record.queue = record.queue
      .then(() =>
        invoke<string>('new_simulation_draft_save', {
          id: record.id,
          name,
          step: at,
          seed: snapshot,
        }),
      )
      .then((id) => {
        if (typeof id === 'string') record.id = id;
      })
      .catch(() => {});
  }
  // 建好以后由项目自己的记录接替；排在未完成的保存之后，拿到的是最终的记录号。
  function closeDraft() {
    const record = draft.current;
    if (record.closed) return;
    record.closed = true;
    record.queue = record.queue
      .then(() => {
        if (record.id) return invoke('new_simulation_draft_close', { id: record.id });
      })
      .catch(() => {});
  }
  // 离开页面时的保存要用最后一次渲染的内容。
  const saveOnLeave = useRef(() => {});
  saveOnLeave.current = () => saveDraft(step);
  useEffect(() => () => saveOnLeave.current(), []);
  useEffect(() => {
    saveDraft(step);
    // Saved per step; leaving the page saves whatever was filled in since.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [step]);
  useEffect(() => {
    if (multiResults.some((result) => result.created)) closeDraft();
  }, [multiResults]);
  // Prepare the server engine as soon as a server is chosen. After an app update the server
  // compiles a matching engine for a few minutes; doing it here keeps that wait off
  // "Read sites" and case creation, which share the same server-side build lock.
  const engineKey = form.host ? `${form.host}\n${JSON.stringify(state.versionRef ?? null)}` : '';
  useEffect(() => {
    if (!engineKey || preparedEngines.has(engineKey)) {
      setEngine(null);
      return;
    }
    const host = form.host;
    let current = true;
    const since = Date.now();
    setEngine({ host, phase: 'checking', since });
    // An engine that is already there answers in a few seconds; longer means upload and build.
    const slow = setTimeout(() => {
      if (current)
        setEngine((old) => (old?.phase === 'checking' ? { host, phase: 'building', since } : old));
    }, 5000);
    invoke<{ ready?: boolean; engine?: { id?: unknown; cached?: unknown } | null }>(
      'remote_prepare_engine',
      { host, modelRevision: state.versionRef },
    )
      .then((reply) => {
        preparedEngines.add(engineKey);
        if (!current) return;
        // 用户反馈"刚编过又显示在准备引擎"：把这次是复用已有引擎还是为新源码快照编了一次
        // 说明白，短暂显示后收起（不占后续操作的位置）。
        const info = reply?.engine;
        setEngine({
          host,
          phase: 'ready',
          since,
          id: typeof info?.id === 'string' ? info.id : undefined,
          cached: typeof info?.cached === 'boolean' ? info.cached : undefined,
        });
      })
      .catch((e) => {
        if (current) setEngine({ host, phase: 'failed', since, error: String(e) });
      })
      .finally(() => clearTimeout(slow));
    return () => {
      current = false;
      clearTimeout(slow);
    };
    // engineKey already encodes form.host and state.versionRef.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engineKey]);
  // `ready` 只是给用户一句解释，8 秒后收起；进入下一步不受它影响。
  useEffect(() => {
    if (engine?.phase !== 'ready') return;
    const done = setTimeout(() => setEngine((old) => (old?.phase === 'ready' ? null : old)), 8000);
    return () => clearTimeout(done);
  }, [engine?.phase, engine?.since]);
  // A one-second clock for the waiting notices, running only while something is waited for.
  const [clock, setClock] = useState(() => Date.now());
  const engineBusy = !!engine && (engine.phase === 'checking' || engine.phase === 'building');
  const waitingForEngine = engineBusy;
  useEffect(() => {
    if (!waitingForEngine && !scanning) return;
    setClock(Date.now());
    const timer = setInterval(() => setClock(Date.now()), 1000);
    // Never keep a test process alive for a notice clock (Node timers only).
    (timer as unknown as { unref?: () => void }).unref?.();
    return () => clearInterval(timer);
  }, [waitingForEngine, scanning]);
  const waited = (since: number) => elapsedLabel(Math.max(0, clock - since), en);
  // 服务器在编引擎时，每 5 秒读一次它编到哪了（`remote_engine_progress`），画进度条。
  const [buildProgress, setBuildProgress] = useState<BuildProgress | null>(null);
  const buildingOn = engine?.phase === 'building' ? engine.host : '';
  useEffect(() => {
    setBuildProgress(null);
    if (!buildingOn) return;
    let current = true;
    const poll = () =>
      invoke<BuildProgress>('remote_engine_progress', { host: buildingOn })
        .then((progress) => {
          if (current) setBuildProgress(progress);
        })
        .catch(() => {});
    void poll();
    const timer = setInterval(() => void poll(), 5000);
    return () => {
      current = false;
      clearInterval(timer);
    };
  }, [buildingOn]);
  useEffect(() => {
    let current = true;
    setSchemaReady(false);
    void Promise.all([
      invoke<SchemaField[]>('describe_fields'),
      invoke<SchemaField[]>('model_fields'),
    ])
      .then(([all, model]) => {
        if (!current) return;
        const supplied = [
          'DEF_USE_LCT',
          'DEF_USE_PFT',
          'DEF_USE_PC',
          'DEF_SUBGRID_SCHEME',
          'SITE_fsitedata',
          'DEF_forcing_namelist',
          'DEF_dir_rawdata',
          'DEF_dir_runtime',
          'DEF_dir_output',
        ];
        setModelFields(model);
        setSchema(
          all
            .filter(isCaseField)
            .filter(
              (f) =>
                !model.some((m) => m.name === f.name) &&
                !supplied.includes(f.name) &&
                !f.name.startsWith('DEF_hist_vars%') &&
                !/^DEF_(domain%|GRIDBASED_|file_mesh|CatchmentMesh)/.test(f.name),
            ),
        );
        const hist = all
          .filter((f) => f.name.startsWith('DEF_hist_vars%'))
          .map((f) => ({
            name: f.name.replace('DEF_hist_vars%', ''),
            long_name: f.doc ?? undefined,
          }));
        setHistoryVariables(hist);
        setSchemaReady(true);
      })
      .catch((e) => {
        if (current) setError(String(e));
      });
    return () => {
      current = false;
    };
  }, []);

  const configurationKernel = compatibleKernel(
    state.kernels,
    {
      grid: form.kind === 'site' ? null : form.grid,
      classification: classification,
    },
    physics,
  );
  const dataPatch = {
    ...schemeFields,
    ...Object.fromEntries(
      Object.entries(caseValues).filter(
        ([key, value]) =>
          schema.some((f) => f.name === key) &&
          (value !== '' || schema.find((f) => f.name === key)?.kind.startsWith('Character')),
      ),
    ),
    ...(form.site ? { SITE_fsitedata: form.site } : {}),
    ...(form.rawdata ? { DEF_dir_rawdata: form.rawdata } : {}),
    ...(form.runtime ? { DEF_dir_runtime: form.runtime } : {}),
    ...(form.kind !== 'site' && !templateForcing && form.forcing
      ? { DEF_forcing_namelist: form.forcing }
      : {}),
    ...regionalRoutingPatch(physics, form.kind, form.grid),
  };
  const dataRequest = JSON.stringify({
    source: { kind: 'frozen', version: state.versionRef },
    caseDir: null,
    kernelDir: configurationKernel?.dir ?? null,
    pftSelection: null,
    caseValues: dataPatch,
  });
  useEffect(() => {
    let current = true;
    if (!schemaReady || !state.version) return;
    setOptionError('');
    void invoke<{ fields: ConfigurationField[]; crop_defaults?: Record<string, string> }>(
      'model_configuration',
      JSON.parse(dataRequest),
    )
      .then((result) => {
        if (current) {
          setOptionStates(result.fields);
          setCropDefaults(result.crop_defaults ?? {});
        }
      })
      .catch((e) => {
        if (current) setOptionError(String(e));
      });
    return () => {
      current = false;
    };
  }, [dataRequest, schemaReady, state.version]);
  useNavigationGuard(() =>
    lock.current
      ? t(
          '正在准备模拟，请等待当前操作完成。',
          'Wait for the current simulation operation to finish.',
        )
      : null,
  );
  // 上游选择改变后，把失去物理前提或与之互斥的已开启过程一并关闭，并告知用户。
  function reconcile(
    patch: CaseOptionPatch,
    context: { subgrid?: string; classification?: string; domain?: string; grid?: string } = {},
  ) {
    const result = exclusionPatch(physics, patch, {
      subgrid: context.subgrid ?? mode,
      classification: context.classification ?? classification,
      domain: context.domain ?? form.kind,
      grid: (context.domain ?? form.kind) === 'site' ? null : (context.grid ?? form.grid),
      compiledCrop: configurationKernel?.macros.includes('CROP') === true,
    });
    setAutoClosed({ text: autoClosedText(result.closed, en), step });
    setCaseValues((old) => {
      const next = { ...old };
      for (const [key, value] of Object.entries(result.patch)) {
        if (schemeFlagKeys.includes(key)) continue;
        if (value === null) delete next[key];
        else next[key] = value;
      }
      return next;
    });
  }
  const set = (k: string, v: string) => {
    setError('');
    if (k === 'start' || k === 'end') forcingDateEdits.current.add(k);
    if (
      ['host', 'spatial_forcingdir', 'forcing_mode', 'kind', 'forcing_dataset'].includes(k) &&
      v !== form[k]
    ) {
      ++forcingSequence.current;
      setForcingProbe(null);
    }
    if (k === 'host' && v !== form.host) {
      ++sequence.current;
      ++scanToken.current;
      setScanning(false);
      exampleValues.current = {};
      setSites([]);
      setSiteDrafts([]);
      setMultiResults([]);
      setHint('');
      setOwnData(!!v);
      if (v) setMultiSiteEnabled(initialMultiSitePreference.current ?? true);
      setCaseValues((old) =>
        Object.fromEntries(
          Object.entries(old).filter(
            ([key, value]) =>
              !/(dir_|file_|fsitedata|filename|path)/i.test(key) &&
              !/^['"]?(\/|[A-Za-z]:[\\/])/.test(value),
          ),
        ),
      );
      setForm((old) => ({
        ...old,
        host: v,
        root: v ? (servers?.find((server) => server.host === v)?.root ?? '') : (state.root ?? ''),
        sitedir: '',
        forcingdir: '',
        site: '',
        met: '',
        rawdata: '',
        runtime: '',
        mesh: '',
        catchment: '',
        shapefile: '',
        forcing: '',
        spatial_forcingdir: '',
        forcing_dataset: '',
      }));
      return;
    }
    // 改站点一律走 selectSite：直接写字段（包括助手）原来只配上强迫场，绕过作物、甲烷配置的检查；
    // 不在扫描结果里的站点文件还留着上一个站的强迫场，站点和强迫场成了两个站（GUI 审查 B-09）。
    if (k === 'site' && v !== form.site) {
      const scanned = sites.find((site) => site.site_file === v);
      if (scanned) {
        selectSite(v);
        return;
      }
    }
    delete exampleValues.current[k];
    if (k === 'kind' || k === 'grid')
      reconcile(
        k === 'kind' && v !== 'site' && cropEnabled(physics)
          ? { DEF_TUNING_CROP_PLANTING_DAY: '0' }
          : {},
        k === 'kind' ? { domain: v } : { grid: v },
      );
    const s = k === 'site' ? sites.find((site) => site.site_file === v) : undefined;
    setForm((f) => {
      const next: Record<string, string> = {
        ...f,
        [k]: v,
        ...(k === 'spatial_forcingdir' && v !== f[k] ? { forcing_dataset: '' } : {}),
        // 手选的站点文件不知道配哪个强迫场：清掉旧的，要用户自己选，不能沿用上一个站的。
        ...(k === 'site' && v !== f.site && !s ? { met: '' } : {}),
        ...(s
          ? {
              name: f.name || s.name,
              met: s.met_file ?? '',
              start: s.start ?? '',
              end: s.end ?? '',
            }
          : {}),
      };
      return next;
    });
    if (k === 'forcing_dataset' && templateForcing) void detectForcing(form.spatial_forcingdir, v);
  };

  async function detectForcing(
    path = form.spatial_forcingdir,
    dataset = path === form.spatial_forcingdir ? form.forcing_dataset : '',
  ) {
    const request = ++forcingSequence.current;
    if (!path.trim()) {
      setForcingProbe(null);
      return;
    }
    setForcingProbe({ status: 'loading', candidates: [] });
    try {
      const result = await invoke<{
        path: string;
        candidates: string[];
        truncated: boolean;
        coverage?: { start: string; end: string } | null;
        coverage_error?: string | null;
      }>('probe_forcing_directory', {
        path,
        host: form.host || null,
        ...(dataset ? { dataset } : {}),
      });
      if (!alive.current || request !== forcingSequence.current) return;
      setForcingProbe({ status: 'done', ...result });
      setForm((old) => {
        if (
          request !== forcingSequence.current ||
          old.spatial_forcingdir !== path ||
          old.host !== form.host ||
          old.kind !== form.kind ||
          old.forcing_mode !== form.forcing_mode
        )
          return old;
        const next = { ...old };
        if (!next.forcing_dataset && result.candidates.length === 1 && !result.truncated)
          next.forcing_dataset = result.candidates[0];
        if (result.coverage) {
          for (const key of ['start', 'end'] as const) {
            if (
              !forcingDateEdits.current.has(key) &&
              (!old[key] || old[key] === forcingAutoDates.current[key])
            ) {
              next[key] = result.coverage[key];
              forcingAutoDates.current[key] = result.coverage[key];
            }
          }
        }
        return next;
      });
    } catch (error) {
      if (alive.current && request === forcingSequence.current)
        setForcingProbe({ status: 'error', candidates: [], error: String(error) });
    }
  }

  function setOption(name: string, value: string) {
    setError('');
    setCaseValues((old) => {
      const next = { ...old };
      if (value) next[name] = value;
      else delete next[name];
      return next;
    });
    if (name === 'DEF_simulation_time%timestep')
      setForm((old) => ({ ...old, timestep: value || '1800' }));
  }
  async function work<T>(fn: () => Promise<T>) {
    if (lock.current) throw new Error(t('请等待当前操作。', 'Wait for the current operation.'));
    lock.current = true;
    setBusy(true);
    setError('');
    try {
      return await fn();
    } catch (e) {
      if (alive.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (alive.current) setBusy(false);
    }
  }
  // 读取站点在服务器上可能先等引擎编完（几分钟）、再逐个打开站点与强迫场文件（几十秒到一两分钟）；
  // 只写"正在读取站点…"时用户看不出在等什么（land02，2026-10-19）。
  const scanWaitsForEngine =
    !!form.host && !!engine && (engine.phase === 'checking' || engine.phase === 'building');
  function scanLabel() {
    if (!form.host)
      return t(
        '正在逐个打开站点文件和对应的强迫场，核对能覆盖的时段…',
        'Opening each site file and its forcing to check the period they cover…',
      );
    return scanWaitsForEngine
      ? t(
          `先等 ${form.host} 上的计算引擎编译完成（进度见上方），之后在服务器上读取站点与强迫场覆盖时段。`,
          `Waiting for the engine on ${form.host} to finish compiling (progress above); then sites and forcing coverage are read there.`,
        )
      : t(
          `正在从 ${form.host} 读取站点与强迫场覆盖时段：在服务器上逐个打开站点文件和对应的强迫场，核对能覆盖的时段；站点多时要几十秒到一两分钟。`,
          `Reading sites and forcing coverage on ${form.host}: each site file and its forcing are opened there to check the period they cover; many sites take from tens of seconds to a couple of minutes.`,
        );
  }
  // The user's own site folders: a scan that finds sites remembers them for this resource.
  // Reading does not lock the page: a server read can wait minutes for an engine build, and the
  // rest of the form stays usable meanwhile. Stopping only stops waiting for the answer.
  const scanToken = useRef(0);
  const [scanSince, setScanSince] = useState(0);
  function readSites() {
    if (scanning)
      return Promise.reject(
        new Error(t('正在读取站点，请稍候或先停止。', 'Sites are being read; wait or stop first.')),
      );
    const dir = form.sitedir,
      forcingDir = form.forcingdir,
      token = ++scanToken.current;
    setScanning(true);
    setScanSince(Date.now());
    setError('');
    return scan(dir, forcingDir)
      .then((result) => {
        if (result.sites.length && scanToken.current === token)
          void rememberKindPaths('site', { sitedir: dir, forcingdir: forcingDir });
        return result;
      })
      .catch((e) => {
        if (alive.current && scanToken.current === token) setError(String(e));
        throw e;
      })
      .finally(() => {
        if (alive.current && scanToken.current === token) setScanning(false);
      });
  }
  function stopReading() {
    ++scanToken.current;
    ++sequence.current;
    setScanning(false);
  }
  async function scan(dir = form.sitedir, forcingDir = form.forcingdir) {
    const id = ++sequence.current;
    const result = await invoke<{ sites: Site[]; hint?: string }>(
      form.host ? 'remote_scan_sites' : 'scan_sites',
      {
        ...(form.host ? { host: form.host, modelRevision: state.versionRef } : {}),
        dir,
        forcingDir: forcingDir || null,
        quick: false,
      },
    );
    if (!Array.isArray(result?.sites))
      throw new Error(
        t('读取站点的结果格式不对。', 'The site scan returned an unexpected result.'),
      );
    if (alive.current && id === sequence.current) {
      setSites(result.sites);
      setSiteDrafts((drafts) =>
        drafts.filter((draft) => result.sites.some((site) => site.site_file === draft.site)),
      );
      setHint(result.hint ?? '');
      setForm((f) => {
        const s = result.sites.find((site) => site.site_file === f.site);
        if (
          !s ||
          !scannedForcing(s, s.met_file ?? '') ||
          siteVersionMismatch(s, physics) ||
          (f.met && f.met !== s.met_file && f.met !== selected?.met_file)
        )
          return f;
        // 扫描期间用户改过的起止日期不覆盖（GUI 审查 B-04）：扫描结果回来得晚，原来把手改的日期冲掉了。
        return {
          ...f,
          met: s.met_file ?? '',
          start: forcingDateEdits.current.has('start') ? f.start : (s.start ?? ''),
          end: forcingDateEdits.current.has('end') ? f.end : (s.end ?? ''),
        };
      });
    }
    return result;
  }
  function selectOwnData() {
    if (lock.current) return;
    if (ownData) {
      // Already the source (a server always uses its own data): show where to fill it in.
      const input = document.querySelector<HTMLInputElement>('.simulation-own-data input');
      input?.scrollIntoView?.({ block: 'center' });
      input?.focus?.();
      return;
    }
    ++sequence.current;
    const bundled = exampleValues.current;
    exampleValues.current = {};
    setSites([]);
    setHint('');
    setError('');
    setForm((previous) => {
      const next = { ...previous };
      for (const [key, value] of Object.entries(bundled)) {
        if (key !== 'name' && next[key] === value) next[key] = '';
      }
      return next;
    });
    setMultiSiteEnabled(initialMultiSitePreference.current ?? true);
    setOwnData(true);
  }
  async function example() {
    if (form.host) throw new Error(t('请从服务器选择数据。', 'Choose data from the server.'));
    return work(async () => {
      setMultiSiteEnabled(false);
      const x = await invoke<{
        sitedir: string;
        forcingdir: string;
        root: string;
        runtimedir: string;
      }>('install_example', {});
      const found = await scan(x.sitedir, x.forcingdir);
      if (!alive.current) return;
      setOwnData(false);
      const s = found.sites.find((site) => site.name === 'CN-Cng');
      exampleValues.current = {
        sitedir: x.sitedir,
        forcingdir: x.forcingdir,
        runtime: x.runtimedir,
        name: s?.name ?? '',
        site: s?.site_file ?? '',
        met: s?.met_file ?? '',
        start: s?.start ?? '',
        end: s?.end ?? '',
      };
      if (s) selectSite(s.site_file, found.sites);
      setForm((f) => ({
        ...f,
        sitedir: x.sitedir,
        forcingdir: x.forcingdir,
        root: f.root || x.root,
        runtime: x.runtimedir,
        ...(s
          ? {
              name: f.name || s.name,
              site: s.site_file,
              met: s.met_file ?? '',
              start: s.start ?? '',
              end: s.end ?? '',
            }
          : {}),
      }));
      return { sites: found.sites.map((s) => s.name) };
    });
  }
  function selectSite(path: string, availableSites = sites) {
    const site = availableSites.find((item) => item.site_file === path);
    if (!site || site.problem) return;
    if (site.crop && !cropDefaults.DEF_TUNING_CROP_PLANTING_DAY) {
      setError(t('正在读取作物默认配置，请稍后选择。', 'Wait for crop defaults to load.'));
      return;
    }
    const methane = isMethaneSite(site);
    let methaneValues: Record<string, string> | undefined;
    if (methane) {
      if (!schemaReady) {
        setError(
          t('正在读取甲烷配置字段，请稍后选择。', 'Wait for methane configuration fields to load.'),
        );
        return;
      }
      try {
        const inheritedTracers = state.version?.overlay
          ? Object.fromEntries(
              tracerKeys.flatMap((key) => {
                const value = modelSettingValue(state.version!.overlay, key);
                return value === null ? [] : [[key, value]];
              }),
            )
          : {};
        const existing = {
          physics: { ...inheritedTracers, ...caseValues },
          parameters: [],
          process_files: [],
        };
        if (Number(modelSettingValue(existing, 'DEF_TRACER_NUM') ?? '0') > 100)
          throw new Error(
            t(
              '超过100个示踪物的定义，请先在示踪设置中核对；未更改算例。',
              'Definitions exceeding 100 tracers must be reviewed in tracer settings; the case was not changed.',
            ),
          );
        const previousRows = tracerRows(existing);
        const rows = appendTracerFamily(previousRows, 'methane').map((row) =>
          row.TYPES.toLowerCase() === 'gas' &&
          /^(ch4|methane)$/i.test(row.NAMES) &&
          /^(null|none)?$/i.test(unquote(row.PARAM_FILES).trim())
            ? { ...row, PARAM_FILES: 'standard_ch4_parameter.nml' }
            : row,
        );
        if (
          !rows.some(
            (row) => /^(ch4|methane)$/i.test(row.NAMES) && row.TYPES.toLowerCase() === 'gas',
          )
        )
          throw new Error(
            t(
              '已有 CH4 定义类型不是 gas，请先在示踪设置中修正。',
              'The existing CH4 definition is not gas; correct it in tracer settings first.',
            ),
          );
        const generatedMethaneDefault =
          rows.some(
            (row) =>
              row.TYPES.toLowerCase() === 'gas' &&
              /^(ch4|methane)$/i.test(row.NAMES) &&
              unquote(row.PARAM_FILES).trim() === 'standard_ch4_parameter.nml',
          ) &&
          !state.version?.overlay.process_files.some(
            (asset) => asset.relative_path === 'standard_ch4_parameter.nml',
          );
        methaneValues = {
          ...applyTracerSelection(existing, schema, rows).physics,
          ...(generatedMethaneDefault ? { DEF_USE_Dynamic_Wetland: '.false.' } : {}),
          DEF_USE_BGC: '.true.',
          DEF_USE_TRACER: '.true.',
          DEF_USE_Campbell_SOIL_MODEL: '.false.',
          DEF_USE_VariablySaturatedFlow: '.true.',
          DEF_URBAN_RUN: '.false.',
          DEF_USE_LULCC: '.false.',
        };
      } catch (e) {
        setError(String(e));
        return;
      }
    }
    setError('');
    if (!ownData)
      Object.assign(exampleValues.current, {
        site: path,
        name: site.name,
        met: site.met_file ?? '',
        start: site.start ?? '',
        end: site.end ?? '',
      });
    // 换了站点就用新站点的起止日期；之前手改过日期的记号随之作废（B-04）。
    forcingDateEdits.current.clear();
    setForm((previous) => ({
      ...previous,
      site: path,
      name: previous.name || site.name,
      met: site.met_file ?? '',
      start: site.start ?? '',
      end: site.end ?? '',
    }));
    setCaseValues((previous) => {
      const next: Record<string, string> = {
        ...previous,
        ...methaneValues,
        DEF_URBAN_RUN: site.urban ? '.true.' : '.false.',
      };
      if (site.crop) {
        for (const [key, value] of Object.entries(cropDefaults)) next[key] = previous[key] ?? value;
        Object.assign(next, {
          DEF_USE_BGC: '.true.',
          DEF_USE_LULCC: '.false.',
          DEF_URBAN_RUN: '.false.',
        });
      } else {
        delete next.DEF_TUNING_CROP_PLANTING_DAY;
        if (site.urban) Object.assign(next, { DEF_USE_BGC: '.false.', DEF_USE_TRACER: '.false.' });
      }
      return next;
    });
    if (site.crop || site.urban || methane)
      setForm((previous) => ({
        ...previous,
        subgrid: methane
          ? ['PFT', 'PC'].includes(previous.subgrid)
            ? previous.subgrid
            : 'PFT'
          : site.crop
            ? 'PFT'
            : 'IGBP',
        landcover: 'IGBP',
      }));
  }
  // 示例站点按数据用途配置作物、城市或甲烷过程；只修改当前算例草稿。
  // 普通站点只选站点，下垫面方案仍须在第四步明确选择。
  function adaptAndSelectSite(s: Site) {
    selectSite(s.site_file);
  }
  const folderKeys = ['root', 'sitedir', 'forcingdir', 'spatial_forcingdir', 'rawdata', 'runtime'],
    fileKeys = ['site', 'met', 'mesh', 'catchment', 'shapefile', 'forcing'];
  async function pick(k: string) {
    return work(async () => {
      const path = form.host
        ? await new Promise<string | null>((resolve) => setRemotePicker({ key: k, resolve }))
        : folderKeys.includes(k)
          ? await invoke<string | null>('pick_folder', { key: `new-${k}` })
          : await invoke<string | null>('pick_file', { key: `new-${k}`, filter: null });
      if (path) {
        set(k, path);
        if (k === 'spatial_forcingdir') await detectForcing(path);
      }
      return { path, cancelled: !path };
    });
  }

  function problem(at: SimulationStep) {
    if (at >= 1) {
      if (form.host && !servers?.some((server) => server.host === form.host))
        return (
          resourceError ||
          t(
            '请等待计算资源读取完成，或重新选择已配置的服务器。',
            'Wait for resources to load or select a configured server.',
          )
        );
      const issue = newCaseGeneralProblem(form, en);
      if (issue) return issue;
    }
    if (at >= 2) {
      if (!form.kind) return t('请选择模拟范围。', 'Choose a simulation range.');
      if (form.kind !== 'site' && !form.grid) return t('请选择网格类型。', 'Choose a grid type.');
      const issue = newCaseRangeProblem(form, en);
      if (issue) return issue;
    }
    if (at >= 3 && destinationError) return destinationError;
    if (at >= 3 && multiSite) {
      try {
        multiPlans(at >= 5 ? 5 : 3);
      } catch (error) {
        return error instanceof Error ? error.message : String(error);
      }
    }
    if (at >= 3 && !multiSite) {
      const issue = newCaseProblem(form, 3, selected, physics, en);
      if (issue) return issue;
    }
    if (at >= 4 && !mode)
      return t(
        '请选择一种下垫面 / 植被方案（IGBP、USGS、PFT 或 PC）。',
        'Choose a land-cover / vegetation scheme (IGBP, USGS, PFT or PC).',
      );
    if (at >= 4 && !['IGBP', 'USGS'].includes(form.landcover))
      return t('请选择 IGBP 或 USGS 地表分类。', 'Choose IGBP or USGS land-cover classification.');
    if (at >= 5) {
      const counts = [
        runSettings.threads,
        runSettings.ranks,
        ...(multiSite && !form.host ? [runSettings.workers] : []),
        ...(form.host ? [runSettings.cpus] : []),
      ].map(Number);
      if (
        counts.some((value) => !Number.isSafeInteger(value) || value < 1) ||
        Number(runSettings.threads) > 256 ||
        (form.host && Number(runSettings.cpus) > 1024)
      )
        return t(
          '资源数量须为正整数；线程最多256，服务器每进程CPU最多1024。',
          'Resource counts must be positive integers; at most 256 threads and 1024 server CPUs per rank.',
        );
      if (form.host && !['auto', 'bare', 'slurm', 'pbs', 'lsf'].includes(runSettings.scheduler))
        return t('请选择受支持的调度系统。', 'Select a supported scheduler.');
      if (placement && 'error' in placement) return nodePlanText(placement, en);
    }
    if (at >= 5 && !multiSite) {
      return (
        newCaseProblem(form, 5, selected, physics, en) ||
        spinupProblem(
          form.start,
          form.end,
          form.spinup_end,
          caseValues['DEF_simulation_time%spinup_repeat'] ?? '1',
          en,
        )
      );
    }
    return null;
  }

  function next() {
    // 当前步骤因模型方案改变而失效：先按现有校验重新检查，通过就更新基线。用户直接点"下一步"
    // 等于在这里重新检查，不用再点一次"重新检查"（U-03）。
    if (staleFor(step).length) {
      const stale = problem(step);
      if (stale) {
        setError(stale);
        throw new Error(stale);
      }
      recordChecked(step);
    }
    // 前面的步骤还有失效数据时不能跨过去进入检查与运行；先回对应步骤重新检查。
    if (staleNotices.some((notice) => notice.step < step)) {
      const message = recheckBlock();
      setError(message);
      throw new Error(message);
    }
    const issue = problem(step);
    if (issue) {
      setError(issue);
      throw new Error(issue);
    }
    setError('');
    // 记下这一步核对时的模型设置：之后方案变了就知道这一步的数据失效了（U-03）。
    setCheckedSetup((old) => ({
      ...old,
      [step]: currentSetup,
      // 进入"检查与运行"时记下当时的设置；之后换引擎/方案要在这里重看资源与提交。
      ...(step === 4 && !old[5] ? { 5: currentSetup } : {}),
    }));
    setStep((step + 1) as SimulationStep);
  }
  function openResources() {
    return leaveForConfiguration('resources', step);
  }
  // 改模型从"新建修改版"进去（2026-10-21 研究不再单独占侧栏一项）。
  function openResearch() {
    return leaveForConfiguration('versions', 4);
  }
  function leaveForConfiguration(route: 'resources' | 'versions', returnStep: SimulationStep) {
    update({
      ...(route === 'versions' ? { versionsView: 'create' as const } : {}),
      newCaseSeed: {
        ...wizardSeed(returnStep),
        ...(draft.current.id ? { draftId: draft.current.id } : {}),
      },
      simulationReturn: {
        route: 'new-case',
        step: returnStep,
        ...(route === 'resources' ? { resourceHost: form.host } : {}),
      },
    });
    return navigate(route);
  }
  function changeCaseOptions(patch: CaseOptionPatch) {
    setError('');
    reconcile(patch);
  }
  function selectMode(next: SimulationMode) {
    const landcover = next === 'USGS' ? 'USGS' : 'IGBP';
    setForm((f) => ({ ...f, subgrid: next, landcover }));
    reconcile(caseSchemeFields(next), { subgrid: next, classification: landcover });
    setError('');
  }

  // 输出变量开关不在通用 schema 里，但同样要随新算例写出，否则在第五步选的变量会被丢掉。
  const editedOptions = Object.entries(caseValues).filter(
    ([name, value]) =>
      name !== 'DEF_dir_output' &&
      value !== '' &&
      (schema.some((f) => f.name === name) || name.startsWith('DEF_hist_vars%')),
  );

  function multiPlans(at: 3 | 5) {
    const shared = {
      ...effectiveModelFields(state.version?.overlay, modelFields),
      ...schemeFields,
      ...Object.fromEntries(editedOptions),
    };
    for (const key of Object.keys(shared)) {
      if (
        [
          'DEF_CASE_NAME',
          'DEF_dir_output',
          'DEF_forcing_namelist',
          'SITE_fsitedata',
          'SITE_lon_location',
          'SITE_lat_location',
          'SITE_landtype',
        ].some((name) => name.toLowerCase() === key.toLowerCase()) ||
        /^DEF_simulation_time%(start|end|spinup)_/i.test(key)
      )
        delete shared[key];
    }
    return planMultiSites({
      form,
      sites,
      drafts: siteDrafts,
      mode: multiSiteMode,
      repeat: multiSiteRepeat,
      physics: shared,
      cropDefaults,
      en,
      step: at,
    });
  }

  async function createDestination(
    command: 'new_case' | 'new_spatial_case',
    args: Record<string, unknown>,
  ) {
    if (!form.host) {
      await invoke(command, args);
      return {
        dir: String(args.out),
        root: multiSite ? form.root : destinationRoot,
        remote_dir: undefined as string | undefined,
      };
    }
    return invoke<{ dir: string; root: string; remote_dir: string }>('remote_create_case', {
      host: form.host,
      modelRevision: args.modelRevision,
      request: serverRequest(command, args),
    });
  }
  // The server request is the local creation's arguments in snake_case, plus the domain kind.
  function serverRequest(command: 'new_case' | 'new_spatial_case', args: Record<string, unknown>) {
    const request = Object.fromEntries(
      Object.entries(args)
        .filter(([key]) => key !== 'modelRevision' && key !== 'runHost')
        .map(([key, value]) => [
          key.replace(/[A-Z]/g, (letter) => `_${letter.toLowerCase()}`),
          value,
        ]),
    );
    return { ...request, kind: command === 'new_case' ? 'site' : args.domain };
  }

  async function createMultiSites() {
    const plans = multiPlans(5);
    const revision = state.versionRef;
    const results = [...multiResults];
    const job: { dir: string; cancelRequested: boolean; batch?: string } = {
      dir: '',
      cancelRequested: false,
    };
    creation.current = job;
    setCancellingCreation(false);
    const siteArgs = (plan: (typeof plans)[number], out: string) => ({
      site: plan.form.site,
      out,
      name: plan.form.name,
      start: plan.form.start,
      end: plan.form.end,
      rawdata: form.rawdata || null,
      runtime: form.runtime || null,
      met: plan.form.met,
      mode: plan.mode,
      fields: physicsFields(plan.fields),
      modelRevision: revision,
      engine: form.engine,
      runHost: form.host || null,
      ...(form.host && multiSiteMode === 'first-year'
        ? { spinupRepeat: Number(multiSiteRepeat) }
        : {}),
    });
    const record = (out: string, result: (typeof results)[number]) => {
      const previous = results.findIndex((item) => (item.remoteDir || item.dir) === out);
      if (previous >= 0) results[previous] = result;
      else results.push(result);
    };
    try {
      if (form.host) {
        // One server batch: a single engine check, lock and SSH session per step, with each
        // site's own result so failures can be retried on their own.
        const pending = plans
          .map((plan) => ({ plan, out: caseDirectory(plan.form.root, plan.form.name) }))
          .filter(
            ({ out }) =>
              !results.some((result) => (result.remoteDir || result.dir) === out && !result.error),
          );
        if (pending.length) {
          const batch = `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
          job.batch = batch;
          job.dir = form.root;
          setCreationDir(form.root);
          setBatchProgress({ stage: 'prepare', done: 0, total: pending.length });
          const stop = await listen<{ batch: string; stage: string; done: number; total: number }>(
            'remote-create://progress',
            (progress) => {
              if (progress.batch === batch && alive.current) setBatchProgress(progress);
            },
          );
          try {
            const outcomes = await invoke<
              { ok?: { dir: string; root: string; remote_dir: string }; error?: string }[]
            >('remote_create_cases', {
              host: form.host,
              modelRevision: revision,
              batch,
              requests: pending.map(({ plan, out }) =>
                serverRequest('new_case', siteArgs(plan, out)),
              ),
            });
            pending.forEach(({ plan, out }, index) => {
              const outcome = outcomes[index];
              record(out, {
                site: plan.site.site_file,
                name: plan.form.name,
                dir: outcome?.ok?.dir ?? out,
                remoteDir: out,
                root: outcome?.ok?.root,
                created: !!outcome?.ok,
                ...(outcome?.ok
                  ? {}
                  : { error: outcome?.error ?? t('服务器没有返回结果', 'No server result') }),
              });
            });
          } finally {
            stop();
          }
          if (alive.current) setMultiResults([...results]);
        }
      }
      for (const plan of form.host ? [] : plans) {
        if (job.cancelRequested) break;
        const out = caseDirectory(plan.form.root, plan.form.name);
        if (results.some((result) => (result.remoteDir || result.dir) === out && !result.error))
          continue;
        job.dir = out;
        setCreationDir(out);
        const prior = results.find((result) => (result.remoteDir || result.dir) === out);
        const result = {
          site: plan.site.site_file,
          name: plan.form.name,
          dir: prior?.dir ?? out,
          remoteDir: form.host ? out : undefined,
          root: prior?.root,
          created: prior?.created,
        } as {
          site: string;
          name: string;
          dir: string;
          remoteDir?: string;
          root?: string;
          created?: boolean;
          error?: string;
        };
        try {
          if (!result.created) {
            const destination = await createDestination('new_case', siteArgs(plan, out));
            result.dir = destination.dir;
            result.root = destination.root;
          }
          result.created = true;
          if (multiSiteMode === 'first-year' && !form.host)
            await invoke('set_spinup', {
              dirs: [result.dir],
              years: 1,
              repeat: Number(multiSiteRepeat),
              replayStart: true,
              kernelDir: null,
            });
        } catch (error) {
          result.error = String(error);
        }
        record(out, result);
        if (alive.current) setMultiResults([...results]);
      }
    } finally {
      if (creation.current === job) {
        creation.current = null;
        if (alive.current) {
          setCreationDir(null);
          setBatchProgress(null);
          setCancellingCreation(false);
        }
      }
    }
    if (results.some((result) => result.created && !result.error)) {
      await rememberDataPaths(false);
      await rememberKindPaths('site', { sitedir: form.sitedir, forcingdir: form.forcingdir });
    }
    if (
      !job.cancelRequested &&
      alive.current &&
      plans.every((plan) => {
        const out = caseDirectory(plan.form.root, plan.form.name);
        return results.some(
          (result) => (result.remoteDir || result.dir) === out && result.created && !result.error,
        );
      })
    )
      await openCreatedProject(results, revision);
    return {
      created: results.filter((result) => !result.error).map((result) => result.dir),
      failed: results.filter((result) => result.error),
      cancelled: job.cancelRequested,
    };
  }

  function editUnfinishedSites() {
    if (busy || state.busy) return;
    const created = multiResults.filter((result) => result.created);
    setSiteDrafts((drafts) =>
      drafts.filter((draft) => !created.some((result) => result.site === draft.site)),
    );
    setHint(
      t(
        '已创建的算例保持原样。这里只调整尚未创建的站点。',
        'Created cases are preserved. Edit only sites that have not been created.',
      ),
    );
    setMultiResults([]);
    setError('');
    setStep(3);
  }

  async function openCreatedProject(results: typeof multiResults, revision = state.versionRef) {
    const successful = results.filter((result) => result.created && !result.error);
    if (!successful.length)
      throw new Error(t('尚无创建成功的算例。', 'No cases were created successfully.'));
    const root = form.host
      ? successful.find((result) => result.root)?.root
      : caseDirectory(form.root, form.name);
    if (!root)
      throw new Error(t('没有成功创建的服务器项目。', 'No server project has been created.'));
    // 项目已经创建成功是主事实：读取目录内容失败时明确说"项目已创建、只是没读到"，并指向
    // "查看已创建的项目"重试——这个入口只重新读列表，不会再创建一遍站点（GUI-15）。
    let cases: CaseEntry[];
    try {
      cases = await invoke<CaseEntry[]>('list_cases', { root });
    } catch (error) {
      throw new Error(
        t(
          `项目已经创建，但读取其中的算例失败：${error}。请用"查看已创建的项目"重试。`,
          `The project was created, but reading its cases failed: ${error}. Retry with "View created project".`,
        ),
        { cause: error },
      );
    }
    // 最近打开记录只是辅助：写不进去只提示，不能把已经成功创建的项目打开报成失败（GUI-15）。
    // 原来这里直接 await，存储只读时用户会以为项目没创建成功，甚至重试创建站点。
    await invoke('save_recent', { key: 'root', value: root }).catch((error: unknown) =>
      notify(
        t(
          `最近打开记录未能更新：${error}。项目已创建，可以继续使用。`,
          `The recent-project entry could not be updated: ${error}. The project was created and can still be used.`,
        ),
      ),
    );
    if (!alive.current) return;
    update({
      root,
      cases,
      selectedCase: null,
      newCaseSeed: null,
      versionRef: revision,
      projectCasesOpen: true,
      projectRunSettings: { ...runSettings, root, cases: successful.map((result) => result.dir) },
    });
    lock.current = false;
    if (navigate('project') === false) {
      update({ projectCasesOpen: false });
      throw new Error(
        t(
          '项目页未能打开，请重试“查看已创建的项目”。',
          'The project page could not open. Retry View created project.',
        ),
      );
    }
  }

  async function viewCreatedProject() {
    if (state.busy) throw new Error(t('请等待当前操作。', 'Wait for the current operation.'));
    return work(() => openCreatedProject(multiResults));
  }

  async function create() {
    return work(async () => {
      // 模型结构/方案改变后，前面的数据没有重新检查过就不能创建（U-03）：旧数据生成的算例可能
      // 与界面上选中的方案不一致。
      if (staleNotices.length) throw new Error(recheckBlock());
      const issue = problem(5);
      if (issue) throw new Error(issue);
      if (
        !schemaReady ||
        !state.version ||
        state.version.version.id !== state.versionRef.id ||
        state.version.version.revision !== state.versionRef.revision
      )
        throw new Error(t('模型与设置尚未读取完成。', 'Wait for model and settings to load.'));
      if (multiSite) return createMultiSites();
      const out = finalDirectory,
        revision = state.versionRef;
      const changed = editedOptions.map(([path, value]) => ({ path, value }));
      const spinup = form.spinup_end
        ? {
            ...caseDatePatch(form.spinup_end, 'spinup'),
            // 预热以日期为界（当日 00:00），不再暴露含义易混的“预热截止秒”。
            'DEF_simulation_time%spinup_sec': '0',
          }
        : { 'DEF_simulation_time%spinup_year': '0' };
      const fields = [
          ...physicsFields(effectiveModelFields(state.version?.overlay, modelFields)),
          ...physicsFields(schemeFields),
          ...changed,
          ...physicsFields(regionalRoutingPatch(physics, form.kind, form.grid)),
          ...Object.entries(spinup).map(([path, value]) => ({ path, value })),
        ].filter(
          (field) => !templateForcing || field.path.toLowerCase() !== 'def_forcing_namelist',
        ),
        mode = versionMode(physics, classification);
      const job = { dir: out, cancelRequested: false };
      creation.current = job;
      setCreationDir(out);
      setCancellingCreation(false);
      let destination: { dir: string; root: string };
      try {
        if (form.kind === 'site')
          destination = await createDestination('new_case', {
            site: form.site,
            out,
            name: destinationName,
            start: form.start,
            end: form.end,
            rawdata: form.rawdata || null,
            runtime: form.runtime || null,
            met: form.met,
            mode,
            fields,
            modelRevision: revision,
            engine: form.engine,
            runHost: form.host || null,
          });
        else {
          const number = (k: string) => requiredNumber(form[k], k),
            bounds = form.kind === 'region' && form.grid === 'latlon';
          destination = await createDestination('new_spatial_case', {
            domain: form.kind,
            gridKind: form.grid,
            shapefile: form.shapefile || null,
            west: bounds ? number('west') : null,
            east: bounds ? number('east') : null,
            south: bounds ? number('south') : null,
            north: bounds ? number('north') : null,
            dlon: form.grid === 'latlon' ? number('dlon') : null,
            dlat: form.grid === 'latlon' ? number('dlat') : null,
            meshFile: form.mesh || null,
            catchmentFile: form.catchment || null,
            out,
            name: form.name,
            forcing: templateForcing ? '' : form.forcing,
            forcingDataset: templateForcing ? form.forcing_dataset : null,
            forcingDir: templateForcing ? form.spatial_forcingdir : null,
            rawdata: form.rawdata,
            runtime: form.runtime,
            start: form.start,
            end: form.end,
            timestep: number('timestep'),
            mode,
            fields,
            modelRevision: revision,
            engine: form.engine,
            runHost: form.host || null,
          });
        }
      } finally {
        if (creation.current === job) {
          creation.current = null;
          if (alive.current) {
            setCreationDir(null);
            setCancellingCreation(false);
          }
        }
      }
      await rememberDataPaths(form.kind !== 'site');
      if (form.kind !== 'site' || ownData)
        await rememberKindPaths(
          form.kind,
          Object.fromEntries(
            kindPathFields(form.kind, form.grid, form.forcing_mode).map((key) => [
              key,
              form[key] ?? '',
            ]),
          ),
        );
      closeDraft();
      update({
        root: destination.root,
        versionRef: revision,
        projectRunSettings: { ...runSettings, root: destination.root, cases: [destination.dir] },
      });
      lock.current = false;
      return await openCase(destination.dir);
    });
  }
  function batchLabel(progress: { stage: string; done: number; total: number }) {
    const { stage, done, total } = progress;
    if (stage === 'server')
      return t(
        `正在 ${form.host} 上批量创建 ${total} 个站点…软件更新后第一次使用时，服务器要先编译计算引擎，可能要几分钟。`,
        `Creating ${total} sites on ${form.host}… The first use after an app update compiles the engine there first and can take several minutes.`,
      );
    if (stage === 'local')
      return t(
        `正在整理本机配置：${done} / ${total}`,
        `Preparing local configuration: ${done} / ${total}`,
      );
    if (stage === 'publish' || stage === 'done')
      return t(
        `正在把 ${total} 个站点的配置发布到 ${form.host}…`,
        `Publishing ${total} sites to ${form.host}…`,
      );
    return t(
      `正在准备 ${total} 个站点：${done} / ${total}`,
      `Preparing ${total} sites: ${done} / ${total}`,
    );
  }
  async function cancelCreation() {
    const job = creation.current;
    if (!alive.current || !job || job.cancelRequested)
      throw new Error(t('当前没有可取消的创建任务。', 'No creation can be cancelled now.'));
    job.cancelRequested = true;
    setCancellingCreation(true);
    try {
      const cancelled = job.batch
        ? await invoke<number>('remote_create_cases_cancel', { batch: job.batch })
        : await invoke<number>('case_creation_cancel', { dir: job.dir });
      if (!multiSite && !cancelled && creation.current === job)
        throw new Error(
          t(
            '尚未取消创建，请等待当前任务结束或重试取消。',
            'Creation was not cancelled. Wait for it to finish or retry cancellation.',
          ),
        );
    } catch (e) {
      if (alive.current && creation.current === job) {
        if (!multiSite) job.cancelRequested = false;
        setCancellingCreation(false);
        setError(String(e));
      }
      throw e;
    }
  }
  const labels: Record<string, string> = {
    engine: t('计算引擎', 'Computation engine'),
    root: form.host
      ? t('服务器项目目录', 'Server project folder')
      : t('保存到项目目录', 'Project folder'),
    output: t('模拟输出路径', 'Simulation output path'),
    name: t('模拟名称', 'Simulation name'),
    sitedir: t('站点数据目录', 'Site data folder'),
    forcingdir: t('强迫场目录', 'Forcing folder'),
    site: t('站点文件', 'Site file'),
    met: t('强迫场文件', 'Forcing file'),
    rawdata: t('地表数据目录', 'Surface data folder'),
    runtime: t('Runtime 数据目录', 'Runtime data folder'),
    start: t('开始日期', 'Start date'),
    end: t('结束日期', 'End date'),
    spinup_end: t('预热结束日期（留空不预热）', 'Spin-up end (blank disables spin-up)'),
    west: t('西界（°）', 'West (°)'),
    east: t('东界（°）', 'East (°)'),
    south: t('南界（°）', 'South (°)'),
    north: t('北界（°）', 'North (°)'),
    dlon: t('经度分辨率（°）', 'Longitude resolution (°)'),
    dlat: t('纬度分辨率（°）', 'Latitude resolution (°)'),
    mesh: t('网格文件', 'Mesh file'),
    catchment: t('汇水区网格文件', 'Catchment file'),
    shapefile: t('流域边界文件', 'Watershed boundary'),
    forcing: t('强迫场配置文件', 'Forcing configuration'),
    spatial_forcingdir: t('强迫场数据路径', 'Forcing data path'),
    forcing_dataset: t('强迫场数据集', 'Forcing dataset'),
  };
  // U-03：某一步的数据因模型方案改变而失效时，把原因（以及新缺的必填项）写成一句可显示的话。
  function staleText(at: SimulationStep): string {
    const notices = staleNotices.filter((notice) => notice.step === at);
    if (!notices.length) return '';
    const missing = [...new Set(notices.flatMap((notice) => notice.fields))].filter(
      (key) => !form[key]?.trim(),
    );
    const reason = notices.map((notice) => notice.text).join(' ');
    return missing.length
      ? reason +
          t(
            ` 还需要补充：${missing.map((key) => labels[key] ?? key).join('、')}。`,
            ` Still missing: ${missing.map((key) => labels[key] ?? key).join(', ')}.`,
          )
      : reason;
  }
  function staleFor(at: SimulationStep): SetupRecheck[] {
    return staleNotices.filter((notice) => notice.step === at);
  }
  function recheckBlock(): string {
    const steps = [...new Set(staleNotices.map((notice) => notice.step))].sort((a, b) => a - b);
    const named = steps.map((at) =>
      en
        ? `step ${at} (${simulationSteps[at - 1][1]})`
        : `第 ${at} 步（${simulationSteps[at - 1][0]}）`,
    );
    return en
      ? `Model structure, schemes or engine changed; the data in ${named.join(', ')} has not been rechecked. Review those steps and recheck before reviewing, running or creating the case.`
      : `模型结构、方案或计算引擎改变后，${named.join('、')}的数据还没有重新检查。请先回到这些步骤核对并点“重新检查”，再进入检查与运行或创建算例。`;
  }
  function recordChecked(at: SimulationStep) {
    setCheckedSetup((old) => ({ ...old, [at]: currentSetup }));
  }
  function recheckStep() {
    if (!staleFor(step).length) return;
    const issue = problem(step);
    if (issue) {
      setError(issue);
      throw new Error(issue);
    }
    setError('');
    recordChecked(step);
  }
  // 步骤导航上的"需重检"标记与原因。
  const recheckStepTitles: Partial<Record<SimulationStep, string>> = {};
  for (const notice of staleNotices) recheckStepTitles[notice.step] = staleText(notice.step);
  const rangeKeys = [
    ...(form.kind === 'region' && form.grid === 'latlon' ? ['west', 'east', 'south', 'north'] : []),
    ...(form.kind && form.kind !== 'site' && form.grid
      ? form.grid === 'latlon'
        ? ['dlon', 'dlat']
        : form.grid === 'catchment'
          ? ['catchment']
          : ['mesh']
      : []),
    ...(form.kind === 'watershed' && form.grid === 'latlon' ? ['shapefile'] : []),
  ];
  const dataKeys = multiSite
    ? ['rawdata', 'runtime']
    : form.kind === 'site'
      ? ['site', 'met', 'rawdata', 'runtime']
      : ['rawdata', 'runtime', templateForcing ? 'spatial_forcingdir' : 'forcing'];
  const keys =
    step === 1
      ? ['root', 'name']
      : step === 2
        ? [...rangeKeys]
        : step === 3
          ? [
              ...dataKeys,
              ...(!multiSite ? ['start', 'end', 'spinup_end'] : []),
              ...(form.kind === 'site' && ownData ? ['sitedir', 'forcingdir'] : []),
            ]
          : // 第 5 步不显示预热日期（它在第 3 步）；原来仍注册进契约，助手能改一个看不见的字段（GUI 审查 F9）。
            [];
  const visibleSchema = schema.filter(
    (field) =>
      (form.kind === 'site' || !isSiteConfiguration(field.name)) &&
      caseFieldVisible(field.name, optionStates, caseValues) &&
      // 单点只写单文件历史（'one'）；建例前后端还拿不到 SinglePoint 上下文去隐藏它，这里先挡住。
      !(form.kind === 'site' && field.name === 'DEF_HIST_mode'),
  );
  const enabled = !busy && !state.busy && !multiResults.some((result) => result.created);
  // Once some sites exist the form is locked, so the remaining ones keep the same settings.
  // Back then means "edit the sites not yet created" rather than doing nothing.
  const lockedByCreated = !busy && !state.busy && multiResults.some((result) => result.created);
  const canEditUnfinished =
    !multiResults.some((result) => result.created && result.error) &&
    siteDrafts.some(
      (draft) => !multiResults.some((result) => result.site === draft.site && result.created),
    );
  const backEnabled = enabled || (lockedByCreated && canEditUnfinished);
  function goBack() {
    if (!enabled && lockedByCreated && canEditUnfinished) return editUnfinishedSites();
    setError('');
    setStep((step - 1) as SimulationStep);
  }
  // 当前步骤的必填项没齐全时，“下一步/创建”置灰，并直接说明还差什么。
  const blocker = stepProblem(step);
  const groupFields = schema.filter((field) => simulationOptionField(field, optionStates)),
    options = [
      ...simulationFields(visibleSchema, step, advanced, form.engine).filter(
        (f) => !groupFields.includes(f),
      ),
    ],
    basicOptions = simulationFields(visibleSchema, step, false, form.engine).filter(
      (f) => !groupFields.includes(f),
    );
  const optionState = (name: string) => optionStates.find((field) => field.name === name),
    optionDisabled = (name: string) =>
      optionState(name)?.mode === 'disabled' || optionState(name)?.mode === 'readonly';
  const domains = [
    ['site', t('单点', 'Single site'), t('一个观测站或位置', 'One station or location')],
    ['watershed', t('流域', 'Watershed'), t('按流域边界开展模拟', 'Simulate a watershed')],
    ['region', t('区域', 'Region'), t('指定区域或网格范围', 'Choose a spatial region')],
    ['global', t('全球', 'Global'), t('覆盖全球陆地', 'Simulate global land')],
  ];
  const grids = [
    [
      'latlon',
      t('经纬度网格', 'Latitude/longitude'),
      t('规则网格与经纬度分辨率', 'Regular grid and resolution'),
    ],
    [
      'unstructured',
      t('非结构网格', 'Unstructured'),
      t('读取已有网格文件', 'Use an existing mesh'),
    ],
    ['catchment', t('汇水区网格', 'Catchment'), t('读取汇水区网格文件', 'Use a catchment mesh')],
  ];
  const autoLabel = t('沿用算例默认', 'Use case defaults');
  const optionValue = (f: SchemaField) =>
    caseValues[f.name] ??
    (isSiteConfiguration(f.name) || isForcingScheme(f.name)
      ? displayLiteral(optionState(f.name)?.built_in_default ?? f.default)
      : '');
  const optionLabels: Record<string, string> = {
    DEF_dir_existing_srfdata: t('已有地表资料目录（可选）', 'Existing surface data (optional)'),
    'DEF_simulation_time%timestep': t('时间步长（秒）', 'Time step (seconds)'),
    'DEF_simulation_time%greenwich': t('使用 UTC 时间', 'Use UTC time'),
    'DEF_simulation_time%spinup_repeat': t('预热循环重复次数', 'Spin-up repetitions'),
    DEF_HIST_FREQ: t('输出间隔', 'Output interval'),
    DEF_HIST_groupby: t('输出文件分组', 'Output file grouping'),
    DEF_WRST_FREQ: t('重启文件间隔', 'Restart interval'),
    DEF_dir_output: t('输出目录', 'Output folder'),
  };

  const fields: UiField[] = keys.map((k) => ({
    id: `new-${k}`,
    label: labels[k],
    value: form[k],
    type: 'text',
    disabled: !enabled,
    set: (v) => {
      set(k, v);
      if (k === 'spatial_forcingdir') void detectForcing(v);
    },
  }));
  if (step === 3 && form.kind !== 'site') {
    fields.push({
      id: 'new-forcing-mode',
      label: t('强迫场配置方式', 'Forcing setup'),
      type: 'choice',
      value: form.forcing_mode,
      disabled: !enabled,
      options: [
        { value: 'directory', label: t('从数据路径自动配置', 'Configure from data path') },
        { value: 'config', label: t('使用已有配置文件', 'Use an existing configuration') },
      ],
      set: (value) => set('forcing_mode', value),
    });
    if (templateForcing)
      fields.push({
        id: 'new-forcing-dataset',
        label: labels.forcing_dataset,
        type: 'choice',
        value: form.forcing_dataset,
        options: forcingDatasets.map((value) => ({ value, label: value })),
        disabled: !enabled,
        set: (value) => set('forcing_dataset', value),
      });
  }
  const resourceFields: UiField[] = [
    ...(localSite || (multiSite && form.host)
      ? []
      : [
          {
            id: 'new-run-threads',
            label: t('每个算例的线程数', 'Threads per case'),
            value: runSettings.threads,
            type: 'number' as const,
            set: (value: string) => set('run_threads', value),
          },
        ]),
    ...(form.engine === 'fortran'
      ? [
          {
            id: 'new-run-ranks',
            label: t('每个算例的 MPI 进程数', 'MPI ranks per case'),
            value: runSettings.ranks,
            type: 'number' as const,
            set: (value: string) => set('run_ranks', value),
          },
        ]
      : []),
    ...(multiSite && !form.host
      ? [
          {
            id: 'new-run-workers',
            label: t('同时运行的站点数', 'Simultaneous stations'),
            value: runSettings.workers,
            type: 'number' as const,
            set: (value: string) => set('run_workers', value),
          },
        ]
      : []),
    ...(form.host
      ? [
          {
            id: 'new-run-scheduler',
            label: t('调度系统', 'Scheduler'),
            value: runSettings.scheduler,
            type: 'choice' as const,
            options: [
              { value: 'auto', label: t('自动识别', 'Detect automatically') },
              { value: 'bare', label: t('直接运行（不提交调度器）', 'Direct execution') },
              ...['slurm', 'pbs', 'lsf'].map((value) => ({ value, label: value.toUpperCase() })),
            ],
            set: (value: string) => set('run_scheduler', value),
          },
          // 多站点批量在服务器上按节点打包（`colm_remote::pack`），核数由节点决定，也不另填。
          ...(cpusFollowThreads || multiSite
            ? []
            : [
                {
                  id: 'new-run-cpus',
                  label: t('申请 CPU 数（每个 MPI 进程）', 'Allocated CPUs per MPI rank'),
                  value: runSettings.cpus,
                  type: 'number' as const,
                  set: (value: string) => set('run_cpus', value),
                },
              ]),
        ]
      : []),
  ].map((field) => ({ ...field, disabled: !enabled }));
  const resourceDrafts: Record<string, string> = {
    'new-run-threads': form.run_threads,
    'new-run-cpus': form.run_cpus,
  };
  if (step === 5) fields.push(...resourceFields);
  if (step === 3 && ownData && form.kind === 'site')
    fields.push({
      id: 'new-multi-site',
      label: t('多站点创建', 'Create multiple sites'),
      type: 'switch',
      value: String(multiSiteEnabled),
      disabled: !enabled,
      set: (value) => {
        setError('');
        setMultiSiteEnabled(value === 'true');
      },
    });
  if (step === 1) {
    fields.push({
      id: 'new-case-directory',
      label: t('模拟保存目录', 'Simulation directory'),
      value: projectDirectory,
      type: 'text',
      disabled: true,
      set: () => {},
    });
    fields.push({
      id: 'new-run-host',
      label: t('运行位置', 'Run location'),
      type: 'choice',
      value: form.host,
      options: [
        { value: '', label: t('本机', 'Local') },
        ...(servers ?? []).map((server) => ({ value: server.host, label: server.host })),
      ],
      disabled: !enabled || servers === null,
      set: (value) => set('host', value),
    });
    fields.unshift({
      id: 'new-engine',
      label: labels.engine,
      value: form.engine,
      type: 'choice',
      options: [
        { value: 'rust', label: t('Rust 引擎', 'Rust engine') },
        { value: 'fortran', label: t('Fortran 官方引擎', 'Official Fortran engine') },
      ],
      disabled: !enabled,
      set: (v) => set('engine', v),
    });
  }
  if (step === 2) {
    fields.push({
      id: 'new-kind',
      label: t('模拟范围', 'Domain'),
      value: form.kind,
      type: 'choice',
      options: domains.map(([value, label]) => ({ value, label })),
      disabled: !enabled,
      set: (v) => set('kind', v),
    });
    if (form.kind !== 'site')
      fields.push({
        id: 'new-grid',
        label: t('网格类型', 'Grid type'),
        value: form.grid,
        type: 'choice',
        options: grids.map(([value, label]) => ({ value, label })),
        disabled: !enabled,
        set: (v) => set('grid', v),
      });
  }
  if ([2, 3, 5].includes(step)) {
    if (step !== 2 || form.engine === 'fortran') {
      fields.push({
        id: 'new-advanced',
        label: t('更多配置', 'More settings'),
        value: String(advanced),
        type: 'switch',
        disabled: !enabled,
        set: (v) => setAdvanced(v === 'true'),
      });
      fields.push(
        // 重复次数只在开启预热后出现在预热框里，契约字段与之保持一致。
        ...options
          .filter((f) => !f.name.endsWith('spinup_repeat') || !!form.spinup_end)
          .map((f) => ({
            id: `new-option-${f.name}`,
            label: optionLabels[f.name] || fieldLabel(f.name, state.language),
            value: optionValue(f),
            type:
              f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
                ? ('choice' as const)
                : ('text' as const),
            ...(f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
              ? {
                  options: [
                    { value: '', label: autoLabel },
                    ...(f.kind === 'Logical'
                      ? ['.true.', '.false.']
                      : optionState(f.name)?.allowed_values.length
                        ? optionState(f.name)!.allowed_values
                        : f.values
                    ).map((value: string) => ({
                      value,
                      label:
                        f.kind === 'Logical'
                          ? t(
                              value === '.true.' ? '开启' : '关闭',
                              value === '.true.' ? 'On' : 'Off',
                            )
                          : optionLabel(f.name, value, state.language),
                    })),
                  ],
                }
              : {}),
            disabled: !enabled || optionDisabled(f.name),
            set: (v: string) => setOption(f.name, v),
          })),
      );
    }
  }

  usePageContract({
    title: t('新建模拟', 'New simulation'),
    step: 'new-case',
    fields,
    // 按钮自己登记（第 737 轮）。原来清单里"选站点"直接选中，界面上的按钮会先按站点调整方案再选。
    actions: [],
    status: busy ? 'busy' : scanning ? 'reading' : error || 'ready',
    details: {
      step,
      model: state.versionRef,
      ...(blocker ? { blocker } : {}),
      // 模型方案改变后哪些步骤的数据需要重新检查（U-03）：助手与测试据此确认没有带着旧数据前进。
      ...(staleNotices.length
        ? { recheck: staleNotices.map((notice) => ({ step: notice.step, text: notice.text })) }
        : {}),
      engine: form.engine,
      run_host: form.host || null,
      ...(step >= 2
        ? { domain: form.kind, ...(form.kind !== 'site' ? { grid: form.grid } : {}) }
        : {}),
      ...(step >= 3 ? { period: { start: form.start, end: form.end } } : {}),
      ...(step === 3 && templateForcing ? { forcing: forcingProbe } : {}),
      ...(step >= 4 ? { classification: form.landcover } : {}),
      ...(step === 5 ? { name: form.name, input_check: 'performed_when_creating' } : {}),
    },
  });

  const render = (k: string) => (
    <Field key={k} label={labels[k]}>
      {['start', 'end', 'spinup_end'].includes(k) ? (
        <DateInput
          value={form[k]}
          calendarLabel={`${t('选择日期', 'Choose date')} · ${labels[k]}`}
          disabled={!enabled}
          onChange={(e) => set(k, e.target.value)}
        />
      ) : (
        <div className="browse-input">
          <Input
            value={form[k]}
            disabled={!enabled}
            onChange={(e) => set(k, e.target.value)}
            onBlur={k === 'spatial_forcingdir' ? () => void detectForcing() : undefined}
          />
          {(folderKeys.includes(k) || fileKeys.includes(k)) && (
            <ActionButton
              action={`new-pick-${k}`}
              label={labels[k]}
              disabled={!enabled}
              aria-label={`${labels[k]} · ${t('选择', 'Browse')}`}
              run={() => pick(k)}
            >
              …
            </ActionButton>
          )}
        </div>
      )}
    </Field>
  );
  const optionChoice = (f: SchemaField) =>
    selectChoice(
      optionValue(f),
      optionState(f.name)?.allowed_values.length ? optionState(f.name)!.allowed_values : f.values,
    );
  const renderOption = (f: SchemaField) => (
    <Field
      key={f.name}
      label={optionLabels[f.name] || fieldLabel(f.name, state.language)}
      hint={configurationReason(optionState(f.name)?.reason) ?? (advanced ? f.name : undefined)}
    >
      {f.kind === 'Logical' ? (
        <Select
          disabled={!enabled || optionDisabled(f.name)}
          value={optionValue(f)}
          onChange={(e) => setOption(f.name, e.target.value)}
        >
          <option value="">{autoLabel}</option>
          <option value=".true.">{t('开启', 'On')}</option>
          <option value=".false.">{t('关闭', 'Off')}</option>
        </Select>
      ) : fieldInputOptions(f.name, f.kind, f.values).length ? (
        <Select
          value={optionChoice(f).value}
          disabled={!enabled || optionDisabled(f.name)}
          onChange={(e) => setOption(f.name, e.target.value)}
        >
          <option value="">{autoLabel}</option>
          {optionChoice(f).options.map((value) => (
            <option key={value} value={value}>
              {optionLabel(f.name, value, state.language)}
            </option>
          ))}
        </Select>
      ) : (
        <Input
          value={optionValue(f)}
          placeholder={
            f.name === 'DEF_simulation_time%timestep'
              ? t('自动（1800 秒）', 'Automatic (1800 seconds)')
              : t('创建时自动配置', 'Set automatically at creation')
          }
          disabled={!enabled || optionDisabled(f.name)}
          onChange={(e) => setOption(f.name, e.target.value)}
        />
      )}
    </Field>
  );
  const timeOptions = basicOptions.filter(
    (f) => f.name.startsWith('DEF_simulation_time%') && f.name !== 'DEF_simulation_time%spinup_sec',
  );
  const outputOptions = ['DEF_HIST_FREQ', 'DEF_WRST_FREQ'].flatMap((name) =>
    basicOptions.filter((f) => f.name === name),
  );
  const extraOptions = options.filter(
    (f) =>
      !basicOptions.includes(f) &&
      !groupFields.includes(f) &&
      !f.name.startsWith('DEF_hist_vars%') &&
      f.name !== 'DEF_HIST_vars_out_default' &&
      f.name !== 'DEF_HIST_vars_namelist',
  );
  const moreOptions = () => (
    <Advanced open={advanced} onOpenChange={setAdvanced} title={t('更多配置', 'More settings')}>
      {advanced && (
        <>
          <div className="form-grid">{extraOptions.map(renderOption)}</div>
        </>
      )}
    </Advanced>
  );

  return (
    <div className="page-stack simulation-page">
      <header className="simulation-heading">
        <div>
          <h1>{t('新建模拟', 'New simulation')}</h1>
          <p className="muted">
            {t(
              '先确定总设定与范围，再配置方案、准备数据并创建。',
              'Choose general settings and range first, then configure model, data, and create.',
            )}
          </p>
        </div>
        <span className="badge">
          {state.version?.name ?? t('读取模型…', 'Loading model…')} · r{state.versionRef.revision}
        </span>
      </header>
      <SimulationSteps
        step={step}
        en={en}
        disabled={!enabled}
        actionPrefix="new"
        recheck={recheckStepTitles}
        onChange={(value) => {
          setError('');
          setStep(value);
        }}
      />
      {error && <Notice tone="error">{error}</Notice>}
      {staleNotices.length > 0 && (
        <Notice tone="warning">
          {staleFor(step).length ? (
            <>
              <p>{staleText(step)}</p>
              <ActionButton
                action="new-recheck"
                label={t('重新检查这一步', 'Recheck this step')}
                run={recheckStep}
              />
            </>
          ) : (
            <>
              <p>{recheckBlock()}</p>
              <div className="button-row">
                {[...new Set(staleNotices.map((notice) => notice.step))].map((at) => (
                  <ActionButton
                    key={at}
                    action={`new-goto-recheck-${at}`}
                    label={t(`去第 ${at} 步重新检查`, `Go to step ${at} and recheck`)}
                    run={() => {
                      setError('');
                      setStep(at);
                    }}
                  />
                ))}
              </div>
            </>
          )}
        </Notice>
      )}
      {multiSite && multiResults.length > 0 && (
        <Card title={t('创建结果', 'Creation results')}>
          <p role="status">
            {form.host
              ? t(
                  '已完成的算例可在项目页逐个打开并提交到服务器；失败或取消的站点可在此重试。',
                  'Open completed cases individually from the project page to submit them to the server; retry failed or cancelled stations here.',
                )
              : t(
                  '已完成的算例可以在项目页统一运行；失败或取消的站点可在此重试。',
                  'Run completed cases together from the project page; retry failed or cancelled stations here.',
                )}
          </p>
          {multiResults.map((result) => (
            <div key={result.dir}>
              <p>
                {result.name} · {result.error ? t('创建失败', 'Failed') : t('已创建', 'Created')}
              </p>
              <p className="path">{result.remoteDir || result.dir}</p>
              {result.error && <Notice tone="error">{result.error}</Notice>}
            </div>
          ))}
          <div className="actions">
            {!multiResults.some((result) => result.created && result.error) &&
              siteDrafts.some(
                (draft) =>
                  !multiResults.some((result) => result.site === draft.site && result.created),
              ) && (
                <ActionButton
                  action="new-multi-edit-unfinished"
                  label={t('修改尚未创建的站点', 'Edit uncreated sites')}
                  disabled={busy || state.busy}
                  run={editUnfinishedSites}
                />
              )}
            {(multiResults.some((result) => result.error) ||
              multiResults.length < siteDrafts.length) && (
              <ActionButton
                action="new-multi-retry"
                kind="commit"
                label={t('重试未完成的站点', 'Retry unfinished sites')}
                disabled={busy || state.busy}
                run={create}
              />
            )}
            {multiResults.some((result) => !result.error) && (
              <ActionButton
                action="new-multi-project"
                label={t('查看已创建的项目', 'View created project')}
                disabled={busy || state.busy}
                run={viewCreatedProject}
              />
            )}
          </div>
        </Card>
      )}
      {autoClosed.text && autoClosed.step === step && (
        <Notice tone="warning">{autoClosed.text}</Notice>
      )}
      {optionError && <Notice tone="warning">{optionError}</Notice>}
      {engine?.host === form.host &&
        (engine.phase === 'checking' || engine.phase === 'building') && (
          <EngineProgress
            host={engine.host}
            phase={engine.phase}
            waited={waited(engine.since)}
            progress={buildProgress}
            engineId={buildProgress?.engine ?? null}
            en={en}
          />
        )}
      {engine?.host === form.host && engine.phase === 'ready' && (
        <p className="muted" role="status" data-engine-ready>
          {engine.cached === true
            ? t(
                `计算引擎已就绪：复用服务器上为源码快照 ${engine.id?.slice(0, 8) ?? ''} 编好的那份，本次没有重新编译。`,
                `Compute engine ready: reusing the engine already built on the server for source snapshot ${engine.id?.slice(0, 8) ?? ''}; nothing was recompiled this time.`,
              )
            : engine.cached === false
              ? t(
                  `已为源码快照 ${engine.id?.slice(0, 8) ?? ''} 编好计算引擎；同一快照后续运行会复用它，不会重复编。`,
                  `Built the compute engine for source snapshot ${engine.id?.slice(0, 8) ?? ''}; later runs with the same snapshot reuse it instead of rebuilding.`,
                )
              : t('计算引擎已就绪。', 'The compute engine is ready.')}
        </p>
      )}
      {engine?.host === form.host && engine.phase === 'failed' && (
        <Notice tone="warning">
          {t(
            `准备 ${engine.host} 上的计算引擎失败：${engine.error}。读取站点或创建算例时会再试一次。`,
            `Preparing the engine on ${engine.host} failed: ${engine.error}. Reading sites or creating the case tries again.`,
          )}
        </Notice>
      )}
      {busy && (
        <Notice>
          {cancellingCreation
            ? t('正在取消创建，请等待清理完成…', 'Cancelling creation; waiting for cleanup…')
            : batchProgress
              ? batchLabel(batchProgress)
              : creationDir && form.host
                ? t(
                    `正在 ${form.host} 上创建算例…首次在该服务器创建、或软件更新后，需要先在服务器上编译计算引擎，可能要几分钟。`,
                    `Creating the case on ${form.host}… The first creation on a server, or the first after an app update, compiles the engine there first and can take several minutes.`,
                  )
                : t('正在处理，请稍候…', 'Working, please wait…')}
        </Notice>
      )}
      {scanning && (
        <TaskCard
          title={
            form.host
              ? t(`正在从 ${form.host} 读取站点`, `Reading sites on ${form.host}`)
              : t('正在读取站点', 'Reading sites')
          }
          waited={t(`已用 ${waited(scanSince)}`, `${waited(scanSince)} so far`)}
          steps={
            form.host
              ? [t('准备计算引擎', 'Engine'), t('读取站点与覆盖时段', 'Read sites and coverage')]
              : undefined
          }
          step={scanWaitsForEngine ? 0 : 1}
          detail={scanLabel()}
          hint={t('读取期间可以继续填写其它设置。', 'Other settings stay editable meanwhile.')}
          action={
            <ActionButton
              action="new-scan-stop"
              label={t('停止读取', 'Stop reading')}
              variant="ghost"
              run={stopReading}
            />
          }
        />
      )}
      {remotePicker && (
        <RemotePathBrowser
          host={form.host}
          initialPath={
            folderKeys.includes(remotePicker.key)
              ? form[remotePicker.key] || form.root
              : form[remotePicker.key]?.slice(0, form[remotePicker.key].lastIndexOf('/')) ||
                form.root
          }
          directory={folderKeys.includes(remotePicker.key)}
          en={en}
          onSelect={(path) => {
            remotePicker.resolve(path);
            setRemotePicker(null);
          }}
          onClose={() => {
            remotePicker.resolve(null);
            setRemotePicker(null);
          }}
        />
      )}
      {creationDir && (!form.host || batchProgress) && (
        <ActionButton
          action="new-cancel-creation"
          label={t('取消创建', 'Cancel creation')}
          disabled={cancellingCreation}
          run={cancelCreation}
        />
      )}
      {step === 1 && (
        <Card
          title={t('运行环境与保存位置', 'Run environment and save location')}
          description={t(
            '先选择运行位置和计算引擎，再设置保存路径与模拟名称。',
            'Choose the run location and computation engine before setting paths and simulation name.',
          )}
        >
          <RunLocationSettings
            host={form.host}
            serverNative={!!form.host}
            servers={servers}
            disabled={!enabled}
            error={resourceError}
            en={en}
            onChange={(value) => set('host', value)}
            onConfigure={openResources}
            configureAction="new-run-resources"
          />
          <section className="simulation-choice-group">
            <h3>{t('计算引擎', 'Computation engine')}</h3>
            <div className="simulation-choices engine-choices">
              <Button
                variant="ghost"
                className="simulation-choice-card"
                aria-pressed={form.engine === 'rust'}
                disabled={!enabled}
                onClick={() => set('engine', 'rust')}
              >
                <strong>{t('Rust 引擎（推荐）', 'Rust engine (recommended)')}</strong>
                <span>
                  {t(
                    '现代单进程多线程并发，免 Fortran 编译与依赖',
                    'Modern multithreaded engine, zero Fortran compilation required',
                  )}
                </span>
              </Button>
              <Button
                variant="ghost"
                className="simulation-choice-card"
                aria-pressed={form.engine === 'fortran'}
                disabled={!enabled}
                onClick={() => set('engine', 'fortran')}
              >
                <strong>{t('Fortran 官方引擎', 'Official Fortran engine')}</strong>
                <span>
                  {t(
                    '传统科学基线，使用与算例匹配的编译配置',
                    'Official scientific baseline, using a compatible build configuration',
                  )}
                </span>
              </Button>
            </div>
          </section>
          {form.engine === 'fortran' && (
            <Field
              label={t('匹配的编译配置', 'Matched build configuration')}
              hint={t(
                '根据模拟范围、土地覆盖与作物设置自动匹配。',
                'Automatically matched to the domain, land cover and crop settings.',
              )}
            >
              <Input
                readOnly
                value={
                  configurationKernel?.preset ??
                  t(
                    '未找到匹配预设，可在版本管理中编译',
                    'No matching preset, compile in Model Versions',
                  )
                }
              />
            </Field>
          )}
          <div className="form-grid">{['root', 'name'].map(render)}</div>
          <Field label={t('模拟保存目录', 'Simulation directory')}>
            <Input
              readOnly
              value={projectDirectory}
              placeholder={t(
                '填写项目目录与模拟名称后显示',
                'Choose a project folder and simulation name',
              )}
            />
          </Field>
          <p className="muted">
            {t(
              '项目目录与模拟名称组合成最终算例目录。配置、运行记录和结果均保存在这里；每次模型版本运行使用 runs/ 下的独立目录。原始输入数据保留引用。',
              'The project folder and simulation name form the final case directory. Configurations, run records and outputs stay here; each model-version run uses its own directory under runs/. Original input datasets remain referenced.',
            )}
          </p>
        </Card>
      )}
      {step === 2 && (
        <Card title={t('范围与网格', 'Range and grid')}>
          <section className="simulation-choice-group">
            <h3>{t('选择模拟范围', 'Choose a simulation range')}</h3>
            <div className="simulation-choices domain-choices">
              {domains.map(([value, label, description]) => (
                <Button
                  key={value}
                  variant="ghost"
                  className="simulation-choice-card"
                  aria-pressed={form.kind === value}
                  disabled={!enabled}
                  onClick={() => set('kind', value)}
                >
                  <strong>{label}</strong>
                  <span>{description}</span>
                </Button>
              ))}
            </div>
          </section>
          {form.kind !== 'site' && (
            <section className="simulation-choice-group">
              <h3>{t('选择网格类型', 'Choose the grid type')}</h3>
              <div className="simulation-choices">
                {grids.map(([value, label, description]) => (
                  <Button
                    key={value}
                    variant="ghost"
                    className="simulation-choice-card"
                    aria-pressed={form.grid === value}
                    disabled={!enabled}
                    onClick={() => set('grid', value)}
                  >
                    <strong>{label}</strong>
                    <span>{description}</span>
                  </Button>
                ))}
              </div>
            </section>
          )}
          {rangeKeys.length > 0 && <div className="form-grid">{rangeKeys.map(render)}</div>}
          {form.engine === 'fortran' && moreOptions()}
        </Card>
      )}
      {step === 3 && (
        <Card
          title={t('数据与时段', 'Data and period')}
          description={t(
            '选择数据来源和模拟起止时段。',
            'Choose data sources and simulation period.',
          )}
        >
          {bundledSite && finalDirectory && (
            <p className="path">
              {t('本站点保存位置：', 'Station directory: ')}
              {finalDirectory}
            </p>
          )}
          {form.kind === 'site' && (
            <>
              <div className="actions simulation-source">
                {!form.host && (
                  <ActionButton
                    action="new-example"
                    kind="commit"
                    label={t('使用内置示例', 'Use bundled examples')}
                    className={!ownData && sites.length > 0 ? 'selected' : ''}
                    aria-pressed={!ownData && sites.length > 0}
                    disabled={!enabled}
                    run={example}
                  />
                )}
                <ActionButton
                  action="new-own-data"
                  label={t('使用自己的数据', 'Use your data')}
                  className={ownData ? 'selected' : ''}
                  aria-pressed={ownData}
                  disabled={!enabled}
                  run={selectOwnData}
                />
              </div>
              {form.host && (
                <Notice>
                  {t(
                    '请选择服务器上已有的输入数据。内置示例仅用于本机。',
                    'Choose existing server input data. Bundled examples are available locally.',
                  )}
                </Notice>
              )}
              {ownData && (
                <div className="simulation-own-data">
                  <div className="form-grid">{['sitedir', 'forcingdir'].map(render)}</div>
                  <ActionButton
                    action="new-scan"
                    label={
                      scanning
                        ? t('正在读取站点…', 'Reading sites…')
                        : t('读取站点与覆盖时段', 'Read sites and coverage')
                    }
                    disabled={!enabled || !form.sitedir || scanning}
                    run={() => readSites()}
                  />
                </div>
              )}
              {hint && <Notice>{hint}</Notice>}
              {ownData && (
                <Field label={t('创建方式', 'Creation scope')}>
                  <Select
                    disabled={!enabled}
                    value={multiSiteEnabled ? 'multiple' : 'single'}
                    onChange={(event) => {
                      setError('');
                      setMultiSiteEnabled(event.target.value === 'multiple');
                    }}
                  >
                    <option value="single">{t('单站点', 'Single site')}</option>
                    <option value="multiple">{t('多站点', 'Multiple sites')}</option>
                  </Select>
                </Field>
              )}
              {multiSite && (
                <MultiSiteSettings
                  sites={sites}
                  drafts={siteDrafts}
                  mode={multiSiteMode}
                  repeat={multiSiteRepeat}
                  projectDirectory={finalDirectory}
                  disabled={!enabled}
                  en={en}
                  onDraftsChange={setSiteDrafts}
                  onModeChange={setMultiSiteMode}
                  onRepeatChange={setMultiSiteRepeat}
                />
              )}
              {!multiSite && sites.length > 0 && (
                <div className="simulation-site-list">
                  {sites.map((s) => {
                    const info = siteInfo(s, t);
                    const issue = s.problem;
                    const isSelected = s.site_file === form.site;
                    return (
                      <div
                        key={s.site_file}
                        style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}
                      >
                        <ActionButton
                          action={`new-select-${s.name}`}
                          label={s.name}
                          className={isSelected ? 'selected' : ''}
                          disabled={!enabled || Boolean(s.problem)}
                          title={issue ?? undefined}
                          run={() => adaptAndSelectSite(s)}
                        >
                          <div
                            style={{
                              display: 'flex',
                              justifyContent: 'space-between',
                              alignItems: 'center',
                              width: '100%',
                            }}
                          >
                            <strong>{s.name}</strong>
                            <span
                              className="badge"
                              style={{
                                fontSize: '10px',
                                background: 'var(--accent-soft)',
                                color: 'var(--accent)',
                              }}
                            >
                              {info.badge}
                            </span>
                          </div>
                          <span style={{ fontSize: '11px' }}>
                            {issue ?? `${info.desc} · ${s.lat.toFixed(2)}°, ${s.lon.toFixed(2)}°`}
                          </span>
                        </ActionButton>
                        {!s.problem &&
                          (isSelected ? (
                            <span
                              style={{
                                color: 'var(--accent)',
                                fontSize: '11px',
                                padding: '0 4px',
                                fontWeight: 500,
                              }}
                            >
                              {info.activeHint}
                            </span>
                          ) : (
                            <button
                              type="button"
                              style={{
                                color: 'var(--accent)',
                                background: 'none',
                                border: 'none',
                                textDecoration: 'underline',
                                cursor: 'pointer',
                                fontSize: '11px',
                                textAlign: 'left',
                                padding: '0 4px',
                              }}
                              onClick={() => adaptAndSelectSite(s)}
                            >
                              {info.adaptHint}
                            </button>
                          ))}
                      </div>
                    );
                  })}
                </div>
              )}
              {!multiSite && scannedForcing(selected, form.met) && selected && (
                <p className="muted">
                  {t('已读取强迫场覆盖', 'Scanned forcing coverage')}：{selected.start} —{' '}
                  {selected.end}
                </p>
              )}
            </>
          )}
          {!multiSite && (
            <>
              <div className="simulation-dates">{['start', 'end'].map(render)}</div>
              <p className="field-hint">
                {t(
                  '可填 YYYY-MM-DD 或 YYYYMMDD；完整有效日期自动补齐分隔符，也可点日历选择。单点通常包含结束当天，到强迫场末尾时以实际覆盖截止。',
                  'Enter YYYY-MM-DD or YYYYMMDD; complete valid dates gain separators automatically. You can also choose from the calendar. Site runs normally include the end date, limited by forcing coverage.',
                )}
              </p>
              <div
                className="simulation-spinup-box"
                style={{
                  marginTop: '1.25rem',
                  padding: '1rem',
                  borderRadius: 'var(--r-md)',
                  background: 'var(--surface)',
                  border: '1px solid var(--border)',
                }}
              >
                <div
                  style={{
                    display: 'flex',
                    justifyContent: 'space-between',
                    alignItems: 'center',
                    flexWrap: 'wrap',
                    gap: '8px',
                  }}
                >
                  <div>
                    <strong style={{ fontSize: '14px' }}>
                      {t('模型预热（Spin-up）设定', 'Model Spin-up Settings')}
                    </strong>
                    <p className="muted" style={{ margin: '2px 0 0', fontSize: '12px' }}>
                      {t(
                        '让土壤温湿与碳库等慢变量先趋于平衡；预热期不写输出，正式输出从预热结束后开始。',
                        'Let soil moisture, temperature and carbon pools reach equilibrium; outputs begin after spin-up.',
                      )}
                    </p>
                  </div>
                  <Button
                    variant={form.spinup_end ? 'primary' : 'secondary'}
                    style={{ fontSize: '12px', padding: '4px 10px' }}
                    disabled={!enabled}
                    onClick={() => {
                      if (form.spinup_end) {
                        set('spinup_end', '');
                        setOption('DEF_simulation_time%spinup_repeat', '');
                      } else {
                        const startYear = form.start ? Number(form.start.slice(0, 4)) : 0;
                        const endYear = form.end ? Number(form.end.slice(0, 4)) : 0;
                        // 预热在结束日当天 00:00 截止，默认取开始后一整年（次年 1 月 1 日）。
                        const defaultCutoff =
                          startYear && endYear && endYear > startYear
                            ? `${startYear + 1}-01-01`
                            : form.start
                              ? `${form.start.slice(0, 7)}-15`
                              : '';
                        set('spinup_end', defaultCutoff);
                        setOption(
                          'DEF_simulation_time%spinup_repeat',
                          caseValues['DEF_simulation_time%spinup_repeat'] || '1',
                        );
                      }
                    }}
                  >
                    {form.spinup_end
                      ? t('✓ 已开启预热（点击关闭）', '✓ Spin-up Enabled (Click to disable)')
                      : t('＋ 开启预热（Spin-up）', '＋ Enable Spin-up')}
                  </Button>
                </div>
                {Boolean(form.spinup_end) && (
                  <div className="form-grid" style={{ marginTop: '12px' }}>
                    <Field
                      label={t('预热起始日期', 'Spin-up start')}
                      hint={t(
                        'CoLM 预热总是从模拟开始日期起算',
                        'CoLM spin-up always starts at the simulation start',
                      )}
                    >
                      <Input readOnly value={form.start} />
                    </Field>
                    {render('spinup_end')}
                    <Field
                      label={t('预热循环重复次数', 'Spin-up repetitions')}
                      hint={t(
                        '从起始日期跑到结束日期（当日 00:00 截止）后重置时间，重复这一段的次数（通常 1~5）',
                        'Times the start-to-end period (ending 00:00 that day) is repeated (normally 1–5)',
                      )}
                    >
                      <Input
                        type="number"
                        min={1}
                        max={99}
                        step={1}
                        disabled={!enabled}
                        value={caseValues['DEF_simulation_time%spinup_repeat'] || '1'}
                        onChange={(e) =>
                          setOption('DEF_simulation_time%spinup_repeat', e.target.value)
                        }
                      />
                    </Field>
                  </div>
                )}
              </div>
            </>
          )}
          <div className="form-grid" style={{ marginTop: '1rem' }}>
            {form.kind !== 'site' && (
              <Field label={t('强迫场配置方式', 'Forcing setup')}>
                <Select
                  disabled={!enabled}
                  value={form.forcing_mode}
                  onChange={(event) => set('forcing_mode', event.target.value)}
                >
                  <option value="directory">
                    {t('从数据路径自动配置', 'Configure from data path')}
                  </option>
                  <option value="config">
                    {t('使用已有配置文件', 'Use an existing configuration')}
                  </option>
                </Select>
              </Field>
            )}
            {dataKeys.filter((key) => key !== 'spatial_forcingdir').map(render)}
            {basicOptions.filter((f) => !f.name.endsWith('spinup_repeat')).map(renderOption)}
          </div>
          {templateForcing && (
            <SpatialForcingFields
              directory={form.spatial_forcingdir}
              dataset={form.forcing_dataset}
              datasets={forcingDatasets}
              probe={forcingProbe}
              catalogError={forcingCatalogError}
              disabled={!enabled}
              en={en}
              onDirectoryChange={(value) => set('spatial_forcingdir', value)}
              onDirectoryBlur={() => void detectForcing()}
              onPick={() => void pick('spatial_forcingdir').catch(() => {})}
              pickAction="new-pick-spatial_forcingdir"
              detectAction="new-detect-forcing"
              onDatasetChange={(value) => set('forcing_dataset', value)}
              onDetect={() => void detectForcing()}
            />
          )}

          {moreOptions()}
        </Card>
      )}
      {step === 4 && (
        <ModelSettings
          runHost={form.host || undefined}
          version={state.version}
          disabled={!enabled}
          classification={classification}
          mode={mode}
          onModeChange={selectMode}
          kernel={configurationKernel}
          setupEnvironment={{
            engine: form.engine,
            subgrid: mode,
            caseValues: dataPatch,
            domain: form.kind,
            grid: form.kind === 'site' ? null : form.grid,
            classification: classification,
            kernel: configurationKernel,
            cropKernel: compatibleKernel(
              state.kernels,
              {
                grid: form.kind === 'site' ? null : form.grid,
                classification: classification,
                crop: true,
              },
              physics,
            ),
          }}
          onCaseChange={changeCaseOptions}
          onClassificationChange={
            form.subgrid === 'PFT' || form.subgrid === 'PC'
              ? undefined
              : (value) => set('landcover', value)
          }
          onEdit={() => openResearch()}
        />
      )}
      {step === 5 && (
        <>
          <Card
            title={t('检查并创建算例', 'Review and create the case')}
            description={t(
              '设置时间步长、预热与输出频率，核对设置后创建算例。',
              'Configure time step, spinup and output frequency, review settings, and create case.',
            )}
          >
            <h3 className="simulation-subheading">{t('运行资源', 'Run resources')}</h3>
            <p>
              {t('运行位置：', 'Run location: ')}
              {form.host || t('本机', 'Local')}
            </p>
            <div className="form-grid">
              {resourceFields.map((field) => (
                <Field key={field.id} label={field.label}>
                  {field.type === 'choice' ? (
                    <Select
                      value={String(field.value)}
                      disabled={!enabled}
                      onChange={(event) => field.set(event.target.value)}
                    >
                      {field.options?.map((option) => (
                        <option key={option.value} value={option.value}>
                          {option.label}
                        </option>
                      ))}
                    </Select>
                  ) : (
                    <Input
                      type="number"
                      min="1"
                      // Threads and CPUs fall back to the server's own numbers when left empty, so
                      // the box shows what was typed (and the fallback as a hint): showing the
                      // fallback itself made the field refill the moment it was cleared.
                      value={
                        field.id in resourceDrafts ? resourceDrafts[field.id] : String(field.value)
                      }
                      placeholder={field.id in resourceDrafts ? String(field.value) : undefined}
                      disabled={!enabled}
                      onChange={(event) => field.set(event.target.value)}
                    />
                  )}
                </Field>
              ))}
            </div>
            {placement && (
              <p
                className={'error' in placement ? 'field-hint simulation-blocker' : 'muted'}
                role="status"
              >
                {nodePlanText(placement, en)}
              </p>
            )}
            <p className="muted">
              {localSite
                ? multiSite
                  ? t(
                      '单点每站固定 1 线程；这里只决定同时跑几个站点，默认填满本机核数。',
                      'Each single-point station is one thread; only the number of stations running at once is configurable, defaulting to the local core count.',
                    )
                  : t(
                      '单点模型每个站点单线程运行；要加速请创建多个站点，并调整同时运行的站点数。',
                      'A single-point model runs one thread per station; create several stations to run them in parallel.',
                    )
                : multiSite
                  ? t(
                      '站点并发控制同时运行几个站点；线程和 MPI 进程用于每个站点。本机并发还会按可用 CPU 限制。',
                      'Station concurrency controls simultaneous sites; threads and MPI ranks apply to each site. Local concurrency also respects available CPUs.',
                    )
                  : t(
                      '空间算例使用线程或 MPI 进程并行计算；单点算例每站单线程运行。',
                      'Spatial cases use threads or MPI ranks; single-point cases run one thread per station.',
                    )}
            </p>
            {form.host && (
              <p className="muted">
                {t(
                  '队列分区、账户、时限与内存沿用计算资源设置。修改运行位置请返回第一步；这里不更改服务器连接设置。',
                  'Partition, account, time and memory use the resource configuration. Return to step 1 to change the run location; connection settings are unchanged here.',
                )}
              </p>
            )}
            <h3 className="simulation-subheading">{t('时间步长与输出', 'Time step and output')}</h3>
            <div className="form-grid">
              {timeOptions
                .filter((f) => f.name === 'DEF_simulation_time%timestep')
                .map(renderOption)}
            </div>
            <h3 className="simulation-subheading">{t('输出设置', 'Output settings')}</h3>
            <div className="form-grid simulation-output-options">
              {outputOptions.map(renderOption)}
            </div>
            <div className="simulation-review" style={{ marginTop: '1.5rem' }}>
              <dl>
                <dt>{t('模拟', 'Simulation')}</dt>
                <dd>{form.name}</dd>
                <dt>{t('运行位置', 'Run location')}</dt>
                <dd>{form.host || t('本机', 'Local')}</dd>
                <dt>{t('计算引擎', 'Computation engine')}</dt>
                <dd>
                  {form.engine === 'rust'
                    ? t('Rust 引擎', 'Rust engine')
                    : t('Fortran 官方引擎', 'Official Fortran engine')}
                </dd>
                <dt>{t('范围', 'Range')}</dt>
                <dd>
                  {domains.find(([value]) => value === form.kind)?.[1]}
                  {form.kind !== 'site'
                    ? ` · ${grids.find(([value]) => value === form.grid)?.[1]}`
                    : ''}
                </dd>
                <dt>{t('时段', 'Period')}</dt>
                <dd>
                  {multiSite
                    ? t('按各站点设置', 'Per-site periods')
                    : `${form.start} — ${form.end}`}
                </dd>
                <dt>{t('模型', 'Model')}</dt>
                <dd>
                  {state.version?.name} · r{state.versionRef.revision}
                </dd>
                <dt>{t('下垫面方案', 'Land-cover scheme')}</dt>
                <dd>
                  {mode ?? '—'}
                  {mode === 'PFT' || mode === 'PC' ? ` · ${form.landcover}` : ''}
                </dd>
                <dt>{t('预热', 'Spin-up')}</dt>
                <dd>
                  {multiSite
                    ? multiSiteMode === 'first-year'
                      ? t(
                          `各站首年重复 ${multiSiteRepeat} 次`,
                          `Each site's first year repeated ${multiSiteRepeat} times`,
                        )
                      : t('分站点配置', 'Per-site settings')
                    : form.spinup_end
                      ? `${form.start} — ${form.spinup_end} × ${caseValues['DEF_simulation_time%spinup_repeat'] || '1'}`
                      : t('不预热', 'Disabled')}
                </dd>
                <dt>{t('保存位置', 'Save location')}</dt>
                <dd className="path">
                  {finalDirectory} ({t('每次运行保存在 runs/ 下', 'individual runs under runs/')})
                </dd>
              </dl>
              <div className="simulation-checks">
                <p className="check-fact">
                  <span>✓</span>
                  {t('必填设置已填写', 'Required settings filled')}
                </p>
                {!multiSite && scannedForcing(selected, form.met) && selected && (
                  <p className="check-fact">
                    <span>✓</span>
                    {t('时段在已扫描覆盖内', 'Period within scanned coverage')}
                  </p>
                )}
                <p className="check-pending">
                  <span>○</span>
                  {t('创建时检查实际输入与方案', 'Inputs and schemes checked when creating')}
                </p>
              </div>
            </div>
            {multiSite && (
              <p>
                {t(`已选择 ${siteDrafts.length} 个站点`, `${siteDrafts.length} stations selected`)}
              </p>
            )}
            {multiSite && form.host && (
              <Notice>
                {t(
                  '各站点将记住所选服务器，可逐站提交远程运行；项目批量运行目前仅支持本机。',
                  'Each case retains this server for individual remote runs. Project batch execution currently runs locally only.',
                )}
              </Notice>
            )}
            {multiSite && (
              <Notice>
                {t(
                  '每个站点在当前项目下保存为独立算例，城市与作物站点分别采用相应过程设置。',
                  'Each site is saved as a separate case under this project. Urban and crop sites use their respective processes. ',
                )}
                {multiSiteMode === 'first-year'
                  ? t(
                      '首年预热完成后，从原始起始日期重新开始正式模拟，输出包含第一年。',
                      'After first-year spinup, formal simulation restarts at the original start date and includes the first year in output.',
                    )
                  : t(
                      '正式输出从各站预热截止日期开始；未启用预热的站点从起始日期开始。',
                      'Formal output starts at each spinup cutoff, or the start date for sites without spinup.',
                    )}
              </Notice>
            )}
            <Notice>
              {multiSite
                ? t(
                    '创建后可在项目中批量运行。',
                    'After creation, run the cases together from the project.',
                  )
                : t(
                    '创建后进入运行页，由你选择何时开始计算。所有预热、输出与变量仍可在运行前返回调整。',
                    'After creation, choose when to start in the Run page. Spinup, output and variables remain adjustable before running.',
                  )}
            </Notice>
            <Advanced title={t('查看输入路径与修改选项', 'Inspect inputs and edited options')}>
              <dl className="simulation-paths">
                {dataKeys.map((k) => (
                  <div key={k}>
                    <dt>{labels[k]}</dt>
                    <dd className="path">{form[k] || t('未指定', 'Not specified')}</dd>
                  </div>
                ))}
                {editedOptions.map(([key, value]) => (
                  <div key={key}>
                    <dt>{optionLabels[key] || key}</dt>
                    <dd>{value}</dd>
                  </div>
                ))}
              </dl>
            </Advanced>
            {moreOptions()}
          </Card>

          <HistoryVariablesCard
            actionPrefix="new"
            variables={historyVariables.map((v) => ({
              name: v.name,
              settable: true,
              longName: v.long_name,
            }))}
            values={caseValues}
            defaultOn
            disabled={!enabled}
            en={en}
            onChange={(patch) => setCaseValues((old) => ({ ...old, ...patch }))}
          />
        </>
      )}
      {blocker && !error && (
        <p className="field-hint simulation-blocker" role="status">
          {blocker}
        </p>
      )}
      {lockedByCreated && (
        <p className="field-hint simulation-blocker" role="status">
          {canEditUnfinished
            ? t(
                '已有站点创建成功，设置已锁定。点"上一步"或"修改尚未创建的站点"只调整其余站点，已创建的保持不变。',
                'Some sites are created and settings are locked. Back or "Edit uncreated sites" adjusts only the remaining sites; created ones stay as they are.',
              )
            : t(
                '已有站点创建成功，设置已锁定。请"重试未完成的站点"，或"查看已创建的项目"。',
                'Some sites are created and settings are locked. Retry unfinished sites, or view the created project.',
              )}
        </p>
      )}
      <footer className="simulation-footer">
        {step > 1 ? (
          <ActionButton
            action="new-previous"
            label={t('上一步', 'Back')}
            disabled={!backEnabled}
            run={goBack}
          />
        ) : (
          <span />
        )}
        {step < 5 ? (
          <ActionButton
            action="new-next"
            label={t('下一步', 'Next')}
            variant="primary"
            // 当前步骤自身失效时仍可点"下一步"：它会先按现有校验重新检查。只有前面步骤还失效时
            // 才不能往前走，免得带着旧数据进检查与运行（U-03）。
            disabled={
              !enabled ||
              (step > 1 && !schemaReady) ||
              !!blocker ||
              staleNotices.some((notice) => notice.step < step)
            }
            title={
              blocker ??
              (staleNotices.some((notice) => notice.step < step) ? recheckBlock() : undefined)
            }
            run={next}
          >
            {t('下一步', 'Next')} →
          </ActionButton>
        ) : (
          <ActionButton
            action="new-create"
            kind="commit"
            label={t('创建算例', 'Create case')}
            variant="primary"
            disabled={
              !enabled || !schemaReady || !state.version || !!blocker || staleNotices.length > 0
            }
            title={blocker ?? (staleNotices.length ? recheckBlock() : undefined)}
            run={create}
          >
            {multiSite && busy
              ? t(
                  `正在创建站点（已处理 ${multiResults.length}/${siteDrafts.length}）`,
                  `Creating sites (${multiResults.length}/${siteDrafts.length} processed)`,
                )
              : multiSite
                ? t('创建所选站点', 'Create selected sites')
                : t('创建算例', 'Create case')}
          </ActionButton>
        )}
      </footer>
    </div>
  );
}
