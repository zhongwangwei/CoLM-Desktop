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
  | 'settings'
  | 'help';
export type AppState = {
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
  } | null;
  simulationReturn: {
    route: 'new-case' | 'case';
    resourceHost?: string;
    step: 1 | 2 | 3 | 4 | 5;
    subgrid?: 'LCT' | 'PFT' | 'PC';
    setup?: { id: string; value: string };
  } | null;
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
export type AppPatch = Partial<AppState> | ((state: AppState) => Partial<AppState>);
export type HistoryOperation = {
  run_group?: { run_id: string; members: HistoryOperation[] };
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
