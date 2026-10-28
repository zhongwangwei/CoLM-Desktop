export type VersionRef = { id: string; revision: number };
export type CodeRef = {
  kind: 'official' | 'workspace';
  workspace: string | null;
  commit: string | null;
  source_sha256: string;
};
export type ParameterScope = {
  kind: string;
  scheme?: string | null;
  index?: number | null;
  type_name?: string | null;
  process_file?: string | null;
};
export type ParameterOverride = {
  parameter_id: string;
  raw_key: string;
  scope_instance: ParameterScope;
  value: string;
  expected_old_value?: string | null;
  unit?: string | null;
};
export type Asset = { relative_path: string; sha256: string };
export type Overlay = {
  parameters: ParameterOverride[];
  physics: Record<string, string>;
  hybrid?: { config: string; assets: Asset[] } | null;
  process_files: Asset[];
};
export type Applicability = {
  sites: string[];
  land_classes: string[];
  calibration?: { start: string; end: string } | null;
  validation?: { start: string; end: string } | null;
};
export type ModelDraft = {
  id: string;
  name: string;
  note: string;
  parent: VersionRef;
  code: CodeRef;
  overlay: Overlay;
  generation: number;
  applicability: Applicability;
  created_unix: number;
  derived_from?: string | null;
};
export type FrozenRevision = {
  schema: number;
  version: VersionRef;
  name: string;
  note: string;
  parent: VersionRef | null;
  code: CodeRef;
  overlay: Overlay;
  overlay_sha256: string;
  fingerprint: string;
  created_unix: number;
  applicability: Applicability;
  derived_from: string | null;
};
export type CaseEntry = {
  dir: string;
  name: string;
  has_history: boolean;
  spatial: boolean;
  run_host?: string | null;
  remote_directory?: string | null;
};
export interface Site {
  name: string;
  site_file: string;
  met_file: string | null;
  obs_file: string | null;
  urban: boolean;
  crop: boolean;
  lon: number;
  lat: number;
  start: string | null;
  end: string | null;
  problem: string | null;
}
export type CaseProfile = {
  spatial: boolean;
  grid: string | null;
  classification?: 'IGBP' | 'USGS';
  classification_source?: 'kernel' | 'creation' | 'legacy-default';
  subgrid: string;
  soil: string;
  urban: boolean;
  bgc: boolean;
  crop: boolean;
  tracer?: boolean;
  kernel_preset: string | null;
  engine?: string;
  run_host?: string | null;
  remote_directory?: string | null;
  obs_file?: string | null;
  inputs: {
    site_file?: string;
    rawdata?: string;
    runtime?: string;
    forcing?: string;
    start: string;
    end: string;
    timestep: number;
    [key: string]: unknown;
  };
  [key: string]: unknown;
};
export type OpenedCase = {
  entry: CaseEntry;
  root: string;
  profile: CaseProfile;
  dir: string;
  name: string;
  binding?: { version: VersionRef; [key: string]: unknown };
};
export type KernelEntry = {
  preset: string;
  dir: string;
  generator_args: string;
  macros: string[];
  colm_git_sha: string;
  platform: string;
};
/** How a case's most recent run ended, from the history ledger. */
export type LastRun = {
  status: string;
  operation_id: string;
  needs_reconciliation: boolean;
  finished_ms: number | null;
  location: string | null;
  host: string | null;
  /** 远程运行结束时台账记下的最后阶段与模拟步，加上算例总步数；旧记录没有。 */
  progress?: {
    stages?: [string, string][];
    step?: number | null;
    date?: string | null;
    total_steps?: number | null;
  } | null;
};
export type RunState = {
  location?: 'local' | 'remote';
  run_id?: string;
  operation_id?: string;
  status: string;
  running: boolean;
  case: string;
  run_case?: string;
  requested_stage?: string | null;
  logs: string[];
  progress?: Record<string, unknown>;
  stages: Record<string, string>;
  error?: string;
};
export type Route =
  | 'project'
  | 'case'
  | 'new-case'
  | 'preparation'
  | 'versions'
  | 'resources'
  | 'history'
  | 'research'
  | 'compare'
  | 'settings'
  | 'help';
export type AppState = {
  /** True until the first backend/kernel/version load has finished; the cover shows meanwhile. */
  starting?: boolean;
  /** Startup progress; absent once the first load succeeded. The cover stays on `failed` with a retry. */
  startup?: {
    phase: 'booting' | 'failed';
    stage: 'backend' | 'kernels' | 'versions';
    startedAt: number;
    error?: string;
  };
  route: Route;
  caseTab: 'settings' | 'runs' | 'results';
  selectedCase: OpenedCase | null;
  cases: CaseEntry[];
  kernels: KernelEntry[];
  kernelDir: string | null;
  version: FrozenRevision | null;
  versionRef: VersionRef;
  chosenVersion: VersionRef | null;
  versions: FrozenRevision[];
  root: string;
  projectCasesOpen?: boolean;
  projectView?: {
    root: string;
    page: 'cases' | 'results' | 'batch';
    home?: boolean;
    fromCreation?: boolean;
  };
  projectRunSettings?: {
    root: string;
    cases: string[];
    batch?: boolean;
    host: string;
    threads: string;
    ranks: string;
    workers: string;
    /** 同时运行站点数还是自动值（按所选站点数与本机核数算）；为真时换了选择会跟着重算。 */
    autoWorkers?: boolean;
    scheduler: string;
    cpus: string;
  };
  newCaseSeed: {
    form: Record<string, string>;
    sites: Site[];
    caseValues?: Record<string, string>;
    step?: 1 | 2 | 3 | 4 | 5;
    ownData?: boolean;
    exampleValues?: Record<string, string>;
    multiSiteEnabled?: boolean;
    multiSiteMode?: import('../features/cases/multisite-model').MultiSiteMode;
    multiSiteRepeat?: string;
    siteDrafts?: import('../features/cases/multisite-model').MultiSiteDraft[];
    /** The history record keeping this unfinished simulation (see `simulation_draft.rs`). */
    draftId?: string;
    /** False when the seed reopens a project that was already created. */
    recordDraft?: boolean;
  } | null;
  simulationReturn: {
    route: 'new-case' | 'case';
    resourceHost?: string;
    step: 1 | 2 | 3 | 4 | 5;
    subgrid?: 'LCT' | 'PFT' | 'PC';
    setup?: { id: string; value: string };
  } | null;
  /** 模型版本页显示哪一页：`create` 是"新建修改版"的 6 种方式，`library` 是全部版本。 */
  versionsView?: 'create' | 'library';
  /**
   * 研究的模型基线（U-01）：从"新建修改版"选中的那一版，研究读参数、建研究和冻结研究修订都用它。
   * 它和 `version`/`versionRef`/`chosenVersion`（模拟正在用的运行版本）是两件事：选研究基线不得改
   * 全局运行版本，已有研究则用它自己记下的冻结修订（研究页里优先于本字段）。没有显式基线时研究页
   * 退回当前运行修订；离开研究入口或从研究首页重新开始时清掉。
   */
  researchBaseline?: VersionRef | null;
  /** 从"新建修改版"点进研究时要直接打开的方法（tuning、uncertainty…），研究页用完清掉。 */
  researchMethod?: string | null;
  /** 版本页选好的要研究的参数：研究页加载参数行之后据此勾选，用完清掉。 */
  researchSeed?: { parameters: StudyPick[] } | null;
  /**
   * 研究结果要写回的草稿（从草稿的"研究"页签进去时设置）。研究页切走就卸载，所以记在这里而不是页面里；
   * 没有它时，研究结果写进一个以研究所用版本为父版本的新草稿。
   */
  researchTarget?: { draftId: string; name: string } | null;
  /** 侧栏点了"研究"：研究页回到首页并丢掉草稿带来的写回目标，用完清掉。 */
  researchHome?: boolean;
  /**
   * 研究首页点了"修改参数（手动）"或"修改源码"：版本页进来就按这种方式建草稿，和在"新建修改版"点卡片一样，
   * 用完清掉。每次点击是一个新对象，版本页靠对象身份只认一次。
   */
  versionMethod?: { method: 'manual' | 'source' } | null;
  /** 版本页进来时直接打开的草稿（研究结果写进去之后去审阅、冻结），版本页用完清掉。 */
  openDraft?: string | null;
  /**
   * 研究结果写回草稿时一起带过去的网络资产暂存目录（`model_import_study` 返回的 `asset_root`）。
   * 版本页冻结这份草稿时把它传给 `model_freeze --asset-root`，否则 DE 训练出的网络文件不会发布进
   * 冻结版本。只对这次打开的草稿有效；版本页消费一次就清掉，不同草稿之间不串用。
   */
  openDraftAssets?: string | null;
  /** 版本页进来时直接打开查看的冻结版本（历史记录的"打开版本"），只看不改模拟所用的版本，用完清掉。 */
  openVersion?: VersionRef | null;
  expert: boolean;
  language: 'zh' | 'en';
  theme: 'light' | 'dark';
  assistantOpen: boolean;
  assistantSettingsOpen: boolean;
  running: Record<string, RunState>;
  notices: string[];
  /** Notices raised as errors (`notify(message, true)`), shown in the error style. */
  errorNotices?: string[];
  busy: boolean;
};
/** 一个要研究的参数：哪个参数、哪个地类或植物类型、取值范围与尺度。 */
export type StudyPick = {
  parameter_id: string;
  raw_key: string;
  scheme: string | null;
  index: number | null;
  min: string;
  max: string;
  scale: string;
};
export type AppPatch = Partial<AppState> | ((state: AppState) => Partial<AppState>);
/** A saved project turned back into the inputs of the five 新建模拟 steps. */
export type WizardSeed = {
  form: Record<string, string>;
  case_values: Record<string, string>;
  sites: Site[];
  drafts: import('../features/cases/multisite-model').MultiSiteDraft[];
  multi_site_mode: import('../features/cases/multisite-model').MultiSiteMode;
  multi_site_repeat: string;
};
export type HistoryOperation = {
  run_group?: { run_id: string; members: HistoryOperation[] };
  /** One listing entry for a whole project folder; members newest first. */
  project_group?: { path: string; name: string; members: HistoryOperation[] };
  operation_id: string;
  event_id: string;
  kind: string;
  phase: string;
  status: string;
  time_ms: number | null;
  started_ms: number | null;
  finished_ms: number | null;
  duration_ms: number | null;
  case_path: string | null;
  case_id: string | null;
  version_id: string | null;
  revision: string | null;
  host: string | null;
  location: string | null;
  initiator: string | null;
  parent_operation_id: string | null;
  source_reference: string | null;
  detail: Record<string, unknown>;
  command?: { program: string; argv: string[]; cwd: string | null } | null;
  event_count: number;
  needs_reconciliation: boolean;
};

export type Server = {
  host: string;
  /** 界面显示名称（例如"摆渡船"）；空时按集群预设或 host 显示。 */
  name?: string;
  username: string;
  port: number;
  auth: string;
  identity_file: string;
  root: string;
  maps: { local: string; remote: string }[];
  threads: number;
  scheduler: string;
  partition: string;
  account: string;
  walltime: string;
  cpus: number;
  memory_gb: number;
  nodes: number;
  /** Cores one node has (摆渡船: 48); 0 = unknown. Used to plan how many nodes a big MPI run takes. */
  cores_per_node: number;
  fetch_vars: string;
  env_script: string;
  directives: string[];
  /** 新建模拟时预填的默认项目目录与数据路径（服务器路径）。 */
  project_dir?: string;
  rawdata_dir?: string;
  runtime_dir?: string;
  forcing_dir?: string;
  /** Last-used paths per simulation kind (site/watershed/region/global). */
  kind_paths?: KindPaths;
};
export type DataDefaults = {
  project_dir: string;
  rawdata_dir: string;
  runtime_dir: string;
  forcing_dir: string;
  kind_paths?: KindPaths;
};
/** Simulation kind → form field → last value used on that resource. */
export type KindPaths = Record<string, Record<string, string>>;
