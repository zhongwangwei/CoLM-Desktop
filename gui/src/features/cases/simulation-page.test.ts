import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as caseModel from './case-model';
import * as configuration from '../versions/configuration-options';
import * as simulation from './simulation-model';
import * as multiSiteModel from './multisite-model';
import * as historyModel from '../history/history';
import * as modelSettings from './model-settings';
import * as historyVariables from './history-variables';
import { compatibleKernel, type KernelChoice } from '../runs/run-model';
import type { UiField, UiAction } from '../../lib/guide';
import type { AppState, Overlay, Server } from '../../lib/types';
import { pageExecutionContext, publicUiResult, waitForContract } from '../../lib/utils';
import { fieldInputOptions, fieldLabel, optionLabel } from '../../lib/param-presentation';

const field = (name: string, section = '时间与预热', value = '0'): caseModel.SchemaField => ({
  name,
  section,
  default: value,
  kind: 'Integer',
  group: 'case',
  derived: false,
  values: name === 'DEF_HIST_FREQ' ? ['HOURLY', 'DAILY', 'NONE'] : [],
});
const initialValues = {
  ...simulation.caseDatePatch('2000-01-01', 'start'),
  ...simulation.caseDatePatch('2002-01-01', 'end'),
  ...simulation.caseDatePatch('2001-06-01', 'spinup'),
  'DEF_simulation_time%start_sec': '3600',
  'DEF_simulation_time%end_sec': '7200',
  'DEF_simulation_time%spinup_sec': '1800',
  'DEF_simulation_time%spinup_repeat': '3',
  'DEF_simulation_time%timestep': '1800',
  DEF_HIST_FREQ: 'HOURLY',
  DEF_WRST_FREQ: 'YEARLY',
  DEF_dir_output: '/project/site/out',
};
const schema = [
  ...Object.entries(initialValues).map(([name, value]) =>
    field(name, name.startsWith('DEF_simulation_time%') ? '时间与预热' : '输出与重启', value),
  ),
  field('DEF_dir_existing_srfdata', '地表数据', '/existing/surface'),
  { ...field('DEF_simulation_time%greenwich', '时间与预热', '.false.'), kind: 'Logical' },
];
type Contract = {
  fields: UiField[];
  actions: UiAction[];
  details: Record<string, unknown>;
  status: string;
};
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function page(
  name: 'NewCasePage' | 'CasesPage',
  seed?: Record<string, unknown>,
  scannedSites: caseModel.Site[] = [site],
  sourceValues: Record<string, string> = initialValues,
  options: {
    profile?: { grid?: string; domain_kind?: string; run_host?: string; remote_directory?: string };
    servers?: (Pick<Server, 'host' | 'root' | 'maps'> & Partial<Server>)[];
    histories?: { name: string; on: boolean; writable: boolean; settable: boolean }[];
    modelFields?: caseModel.SchemaField[];
    kernels?: KernelChoice[];
    schema?: caseModel.SchemaField[];
    configuration?: (args: Record<string, unknown>) => Promise<unknown>;
    create?: (args: Record<string, unknown>) => Promise<unknown>;
    cancelCreation?: (args: Record<string, unknown>) => Promise<number>;
    scan?: () => Promise<unknown>;
    prepare?: (args: Record<string, unknown>) => Promise<unknown>;
    progress?: () => unknown;
    navigate?: (route: string) => boolean;
    listCases?: (args: Record<string, unknown>) => Promise<unknown>;
    spinup?: (args: Record<string, unknown>) => Promise<unknown>;
    forcingProbe?: (args: Record<string, unknown>) => Promise<unknown>;
    pickFolder?: string;
    forcingInput?: () => Promise<{ directory: string; dataset: string }>;
    local?: { project_dir?: string; rawdata_dir: string; runtime_dir: string; forcing_dir: string };
  } = {},
) {
  const model = {
    name: 'Official',
    version: { id: 'official', revision: 4 },
    overlay: { physics: {}, parameters: [], process_files: [] },
  };
  const state = {
    language: 'zh',
    root: '/project',
    expert: false,
    busy: false,
    version: model,
    versionRef: model.version,
    kernels: options.kernels ?? [],
    running: {},
    caseTab: 'settings',
    selectedCase: { entry: { dir: '/project/site', name: 'site' }, profile: options.profile ?? {} },
    newCaseSeed: seed ?? null,
    simulationReturn: null as unknown,
    projectRunSettings: undefined as AppState['projectRunSettings'],
    projectView: undefined as AppState['projectView'],
  };
  const listeners: { event: string; callback: (payload: unknown) => void }[] = [];
  const calls: { command: string; args: Record<string, unknown> }[] = [],
    routes: string[] = [],
    opened: string[] = [],
    scrolls: unknown[] = [],
    elements: Element[] = [];
  let slots: unknown[] = [],
    cursor = 0,
    effectCursor = 0,
    contract: Contract,
    guard: () => unknown = () => null,
    failCreate = false;
  let effects: (() => unknown)[] = [],
    registered = new Map<number, { dependencies: unknown[] | undefined; cleanup?: () => void }>();
  const update = (patch: object | ((old: typeof state) => object)) =>
      Object.assign(state, typeof patch === 'function' ? patch(state) : patch),
    notify = () => {};
  const navigate = (route: string) => {
    if (guard() || options.navigate?.(route) === false) return false;
    routes.push(route);
    return true;
  };
  const context = {
    document: {
      querySelector: () => ({ scrollTo: (value: unknown) => scrolls.push(value) }),
    },
    ...caseModel,
    ...configuration,
    ...simulation,
    ...multiSiteModel,
    ...historyModel,
    ...modelSettings,
    ...historyVariables,
    fieldInputOptions,
    fieldLabel,
    optionLabel,
    compatibleKernel,
    React: {
      createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => {
        const element = { type, props: props ?? {}, children };
        elements.push(element);
        return element;
      },
    },
    ...Object.fromEntries(
      [
        'Button',
        'Card',
        'Field',
        'Input',
        'DateInput',
        'Select',
        'Advanced',
        'Notice',
        'SimulationSteps',
        'ModelSettings',
        'MultiSiteSettings',
        'RunPanel',
        'RunLocationSettings',
        'RemotePathBrowser',
        'ResultsPage',
        'HistoryPage',
        'HistoryVariablesCard',
        'SpatialForcingFields',
        'EngineProgress',
        'TaskCard',
      ].map((key) => [key, key]),
    ),
    useApp: () => ({
      state,
      update,
      notify,
      navigate,
      openCase: async (path: string) => {
        opened.push(path);
      },
    }),
    useNavigationGuard: (value: () => unknown) => {
      guard = value;
    },
    usePageContract: (value: Contract) => {
      contract = value;
    },
    useRef: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = { current: initial };
      return slots[key];
    },
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (value: unknown) => {
          slots[key] = typeof value === 'function' ? value(slots[key]) : value;
        },
      ];
    },
    useEffect: (fn: () => unknown, dependencies?: unknown[]) => {
      const key = effectCursor++,
        previous = registered.get(key);
      if (
        !previous ||
        !dependencies ||
        !previous.dependencies ||
        dependencies.some((value, index) => !Object.is(value, previous.dependencies?.[index]))
      ) {
        const entry = { dependencies, cleanup: undefined as (() => void) | undefined };
        registered.set(key, entry);
        effects.push(() => {
          previous?.cleanup?.();
          const cleanup = fn();
          if (typeof cleanup === 'function') entry.cleanup = cleanup as () => void;
        });
      }
    },
    listen: async (event: string, callback: (payload: unknown) => void) => {
      listeners.push({ event, callback });
      return () => {
        const index = listeners.findIndex((listener) => listener.callback === callback);
        if (index >= 0) listeners.splice(index, 1);
      };
    },
    invoke: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      switch (command) {
        case 'case_forcing_input':
          return options.forcingInput?.() ?? { directory: '/data/current', dataset: 'ERA5LAND' };
        case 'forcing_datasets':
          return ['ERA5', 'ERA5LAND', 'ERA5_LEddy', 'CMFD'];
        case 'probe_forcing_directory':
          return (
            options.forcingProbe?.(args) ?? {
              path: args.path,
              candidates: ['ERA5LAND'],
              truncated: false,
            }
          );
        case 'pick_folder':
          return options.pickFolder ?? null;
        case 'remote_config':
          return { servers: options.servers ?? [], local: options.local };
        case 'remote_remember_data_paths':
          return true;
        case 'set_case_run_host':
          return {};
        case 'describe_fields':
          return options.schema ?? schema;
        case 'model_configuration':
          return options.configuration
            ? options.configuration(args)
            : {
                fields: [],
                crop_defaults: { DEF_TUNING_CROP_PLANTING_DAY: '120', DEF_USE_BGC: '.true.' },
              };
        case 'model_fields':
          return options.modelFields ?? [];
        case 'install_example':
          return {
            sitedir: '/bundled/sites',
            forcingdir: '/bundled/forcing',
            root: '/bundled',
            runtimedir: '/bundled/runtime',
          };
        case 'remote_remember_kind_paths':
        case 'remote_forget_kind_path':
          return true;
        case 'remote_prepare_engine':
          return options.prepare ? options.prepare(args) : { ready: true };
        case 'remote_engine_progress':
          return options.progress ? options.progress() : {};
        case 'remote_scan_sites':
        case 'scan_sites':
          return options.scan ? options.scan() : { sites: scannedSites };
        case 'pick_file':
          return scannedSites[0]?.site_file ?? null;
        case 'model_case_context':
          return { entry: { dir: '/preview/site' }, profile: options.profile ?? {} };
        case 'hist_vars':
          return options.histories ?? [];
        case 'read_text':
          return '';
        case 'read_case':
          return Object.entries(sourceValues).map(([path, value]) => ({ path, value }));
        case 'remote_create_cases': {
          const outcomes = [];
          for (const request of args.requests as { out: string }[]) {
            try {
              outcomes.push({
                ok: options.create
                  ? await options.create({ ...args, request })
                  : {
                      dir: `/cache/project/${request.out.split('/').pop()}`,
                      root: '/cache/project',
                      remote_dir: request.out,
                    },
              });
            } catch (error) {
              outcomes.push({ error: String(error) });
            }
          }
          return outcomes;
        }
        case 'remote_create_cases_cancel':
          return 1;
        case 'remote_create_case':
          if (options.create) return options.create(args);
          return {
            dir: '/cache/project/site',
            root: '/cache/project',
            remote_dir: (args.request as { out: string }).out,
          };
        case 'new_spatial_case':
        case 'new_case':
          if (options.create) return options.create(args);
          if (failCreate) throw new Error('creation failed');
          return {};
        case 'list_cases':
          return options.listCases?.(args) ?? [];
        case 'save_recent':
          return {};
        case 'set_spinup':
          return options.spinup?.(args) ?? {};
        case 'case_creation_cancel':
          return options.cancelCreation?.(args) ?? 0;
        case 'case_data_save':
          if (failCreate) throw new Error('save failed');
          return {};
        case 'new_simulation_draft_save':
          return (args.id as string | null) ?? 'draft-1';
        case 'new_simulation_draft_close':
          return null;
        default:
          throw new Error(`Unexpected IPC: ${command}`);
      }
    },
  };
  const source = readFileSync(new URL(`./${name}.tsx`, import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ${name};`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    effectCursor = 0;
    elements.length = 0;
    Component();
    return contract;
  }
  async function settle() {
    for (let round = 0; round < 20; round++) {
      const pending = effects;
      effects = [];
      pending.forEach((fn) => fn());
      for (let i = 0; i < 16; i++) await Promise.resolve();
      render();
      if (!effects.length) return contract;
    }
    throw new Error('Component effects did not stabilize');
  }
  async function mount() {
    render();
    return settle();
  }
  function set(id: string, value: string) {
    const target = contract.fields.find((item) => item.id === id);
    assert.ok(target, `visible field ${id}`);
    target.set(value);
    return render();
  }
  // 已在目标步骤时不再点击（步骤导航只列出当前步骤之前的步骤）。
  async function goto(id: string) {
    const target = Number(id.split('-').pop());
    const current = Number(
      (contract.details as { step?: number; simulation_step?: number } | undefined)?.step ??
        (contract.details as { simulation_step?: number } | undefined)?.simulation_step,
    );
    if (current === target) return render();
    if (contract.actions.some((item) => item.id === id)) return click(id);
    for (
      let i = 0;
      i < 6 &&
      Number(
        (contract.details as { step?: number; simulation_step?: number })?.step ??
          (contract.details as { simulation_step?: number })?.simulation_step,
      ) < target;
      i++
    ) {
      const next = contract.actions.find(
        (item) => item.id === 'new-next' || item.id === 'case-next',
      );
      if (!next || next.enabled === false) break;
      await next.run();
      render();
    }
    return render();
  }
  async function click(id: string) {
    const target = contract.actions.find((item) => item.id === id);
    assert.ok(target, `visible action ${id}`);
    assert.notEqual(target.enabled, false);
    await target.run();
    return render();
  }
  function element(type: string) {
    const result = elements.find((item) => item.type === type);
    assert.ok(result, `rendered ${type}`);
    return result;
  }
  async function remount() {
    for (const entry of registered.values()) entry.cleanup?.();
    slots = [];
    registered = new Map();
    effects = [];
    return mount();
  }
  return {
    state,
    calls,
    listeners,
    routes,
    opened,
    scrolls,
    elements,
    render,
    mount,
    settle,
    set,
    click,
    goto,
    element,
    remount,
    guard: () => guard(),
    fail: () => {
      failCreate = true;
    },
  };
}
// 向导要求下一步前显式选择的项：未齐时“下一步”置灰，并在页面契约里说明缺什么。
function blocked(
  p: { render: () => { actions: { id: string; enabled?: boolean }[]; details?: unknown } },
  id: string,
  reason: RegExp,
) {
  const c = p.render();
  assert.equal(c.actions.find((a) => a.id === id)?.enabled, false, `${id} disabled`);
  assert.match(String((c.details as { blocker?: string }).blocker), reason);
}
const form = {
  engine: 'rust',
  root: '/project',
  name: 'CN-Cng',
  kind: 'site',
  landcover: 'IGBP',
  subgrid: 'IGBP',
  site: '/site.nc',
  met: '/forcing.nc',
  start: '2001-01-01',
  end: '2001-12-31',
  spinup_end: '',
  grid: 'latlon',
  runtime: '/runtime',
  rawdata: '/surface',
  timestep: '1800',
};
const site = {
  name: 'CN-Cng',
  site_file: '/site.nc',
  met_file: '/forcing.nc',
  start: '2001-01-01',
  end: '2002-01-01',
  lat: 35,
  lon: 100,
  crop: false,
  urban: false,
  obs_file: null,
  problem: null,
};

test('Results opens the station chooser before a case has been selected', async () => {
  const p = page('CasesPage');
  p.state.selectedCase.entry.dir = '';
  p.state.caseTab = 'results';
  await p.mount();
  assert.ok(p.element('ResultsPage'));
  assert.ok(!p.elements.some((element) => element.type === 'Card'));
  p.state.projectView = { root: p.state.root, page: 'batch', home: true };
  p.render();
  (p.element('ResultsPage').props.onBack as () => void)();
  assert.deepEqual(p.routes, ['project'], 'results returns to the original batch');
  assert.equal(p.state.projectView?.home, false);
});

test('selecting a station defaults to its entire forcing period and subsequent manual dates remain editable', async () => {
  const other = {
    ...site,
    name: 'Long-site',
    site_file: '/long-site.nc',
    met_file: '/long-met.nc',
    start: '1991-01-01',
    end: '2021-06-30',
  };
  const p = page('NewCasePage', { form, sites: [site, other], step: 3 });
  await p.mount();
  let c = await p.click('new-select-Long-site');
  assert.deepEqual(c.details.period, { start: other.start, end: other.end });
  assert.equal(c.fields.find((f) => f.id === 'new-met')?.value, other.met_file);
  p.set('new-start', '2001-01-01');
  p.set('new-end', '2001-12-31');
  c = p.set('new-runtime', '/another-runtime');
  assert.deepEqual(c.details.period, { start: '2001-01-01', end: '2001-12-31' });
});
test('station file picking and assistant filling use the same full coverage as station cards', async () => {
  const other = {
    ...site,
    name: 'Long-site',
    site_file: '/long-site.nc',
    met_file: '/long-met.nc',
    start: '1991-01-01',
    end: '2021-06-30',
  };
  for (const pick of [true, false]) {
    const p = page('NewCasePage', { form, sites: [site, other], step: 3 }, [other]);
    await p.mount();
    if (pick) await p.click('new-pick-site');
    else await guideFor(p).fill('new-site', other.site_file);
    const c = p.render();
    assert.deepEqual(c.details.period, { start: other.start, end: other.end });
    assert.equal(c.fields.find((f) => f.id === 'new-met')?.value, other.met_file);
  }
});
test('rescanning selected forcing fills its full new coverage and does not borrow dates for a custom forcing file', async () => {
  const newer = {
    ...site,
    met_file: '/updated-forcing.nc',
    start: '1991-01-01',
    end: '2024-12-31',
  };
  const p = page(
    'NewCasePage',
    {
      multiSiteEnabled: false,
      form: { ...form, sitedir: '/sites' },
      sites: [site],
      ownData: true,
      step: 3,
    },
    [newer],
  );
  await p.mount();
  let c = await p.click('new-scan');
  assert.deepEqual(c.details.period, { start: newer.start, end: newer.end });
  assert.equal(c.fields.find((f) => f.id === 'new-met')?.value, newer.met_file);
  assert.deepEqual(p.calls.find((call) => call.command === 'scan_sites')?.args, {
    dir: '/sites',
    forcingDir: null,
    quick: false,
  });
  const custom = page(
    'NewCasePage',
    {
      multiSiteEnabled: false,
      form: { ...form, sitedir: '/sites', met: '/custom.nc' },
      sites: [site],
      ownData: true,
      step: 3,
    },
    [newer],
  );
  await custom.mount();
  c = await custom.click('new-scan');
  assert.deepEqual(c.details.period, { start: form.start, end: form.end });
  assert.equal(c.fields.find((f) => f.id === 'new-met')?.value, '/custom.nc');
});

test('five-step creation gates each step, chooses the scheme after data, and sends only edited case options', async () => {
  const p = page('NewCasePage');
  await p.mount();
  blocked(p, 'new-next', /计算引擎/);
  p.set('new-engine', 'rust');
  blocked(p, 'new-next', /名称/);
  p.set('new-name', 'CN-Cng');
  p.set('new-root', '');
  blocked(p, 'new-next', /项目目录/);
  p.set('new-root', '/project');
  let c = await p.click('new-next');
  assert.equal(c.details.step, 2);
  blocked(p, 'new-next', /范围/);
  p.set('new-kind', 'site');
  c = await p.click('new-next');
  assert.equal(c.details.step, 3);
  blocked(p, 'new-next', /时段/);
  for (const [key, value] of Object.entries(form))
    if (p.render().fields.some((f) => f.id === `new-${key}`)) p.set(`new-${key}`, value);
  c = await p.click('new-next');
  assert.equal(c.details.step, 4);
  assert.equal(typeof p.element('ModelSettings').props.onEdit, 'function');
  assert.equal(p.element('ModelSettings').props.mode, undefined, 'no scheme is preselected');
  blocked(p, 'new-next', /方案/);
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('IGBP');
  p.render();
  c = await p.click('new-next');
  assert.equal(c.details.step, 5);
  p.set('new-option-DEF_HIST_FREQ', 'DAILY');
  assert.equal(p.render().actions.find((a) => a.id === 'new-create')?.kind, 'commit');
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case');
  assert.ok(request);
  assert.equal(request.args.engine, 'rust');
  const fields = request.args.fields as { path: string; value: string }[];
  assert.deepEqual(
    fields.filter((f) =>
      ['DEF_HIST_FREQ', 'DEF_WRST_FREQ', 'DEF_simulation_time%timestep'].includes(f.path),
    ),
    [{ path: 'DEF_HIST_FREQ', value: 'DAILY' }],
  );
  assert.deepEqual(p.opened, ['/project/CN-Cng']);
  assert.ok(!p.calls.some((call) => /run_start|run_case|submit/.test(call.command)));
});
test('an unfinished simulation is recorded from step 2 and saved again when the page is left', async () => {
  const p = page('NewCasePage');
  await p.mount();
  const drafts = () => p.calls.filter((call) => call.command === 'new_simulation_draft_save');
  p.set('new-engine', 'rust');
  p.set('new-name', 'CN-Cng');
  p.set('new-root', '/project');
  await p.settle();
  assert.equal(drafts().length, 0, 'step 1 alone is not worth a record');
  await p.click('new-next');
  await p.settle();
  assert.equal(drafts().length, 1);
  const first = drafts()[0].args as { id: unknown; name: string; step: number; seed: unknown };
  assert.equal(first.id, null);
  assert.equal(first.name, 'CN-Cng');
  assert.equal(first.step, 2);
  const seed = first.seed as { form: Record<string, string>; step: number };
  assert.equal(seed.form.name, 'CN-Cng');
  assert.equal(seed.step, 2);
  p.set('new-kind', 'site');
  // Leaving the page keeps what was filled in since, on the same record.
  await p.remount();
  await p.settle();
  const left = drafts().at(-1)!.args as { id: unknown; seed: { form: Record<string, string> } };
  assert.equal(left.id, 'draft-1');
  assert.equal(left.seed.form.kind, 'site');
});
test('a simulation resumed from history keeps its record and closes it once created', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 3, draftId: 'draft-7' });
  await p.mount();
  await p.settle();
  const saves = p.calls.filter((call) => call.command === 'new_simulation_draft_save');
  assert.deepEqual(
    saves.map((call) => [call.args.id, call.args.step]),
    [['draft-7', 3]],
    'reopening at step 3 continues the same record',
  );
  // React runs effects after every commit; this harness only on settle.
  await p.click('new-next');
  await p.settle();
  await p.click('new-next');
  await p.settle();
  await p.click('new-create');
  await p.settle();
  assert.equal(p.opened.length, 1, 'the created case opened');
  assert.deepEqual(
    p.calls
      .filter((call) => call.command === 'new_simulation_draft_save')
      .map((call) => [call.args.id, call.args.step]),
    [
      ['draft-7', 3],
      ['draft-7', 4],
      ['draft-7', 5],
    ],
    'each step is saved on the same record',
  );
  const close = p.calls.find((call) => call.command === 'new_simulation_draft_close');
  assert.deepEqual(close?.args, { id: 'draft-7' });
  // Closed drafts are not saved again when the created case takes over the page.
  const count = p.calls.filter((call) => call.command === 'new_simulation_draft_save').length;
  await p.remount();
  await p.settle();
  assert.equal(
    p.calls.filter((call) => call.command === 'new_simulation_draft_save').length,
    count,
  );
});
test('a created project reopened from history is not recorded as an unfinished simulation', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 4, recordDraft: false });
  await p.mount();
  await p.click('new-next');
  await p.remount();
  await p.settle();
  assert.ok(!p.calls.some((call) => call.command.startsWith('new_simulation_draft')));
});
test('data step rejects scanned forcing overflow and failed creation never opens or starts a run', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 3 });
  await p.mount();
  p.set('new-end', '2003-01-01');
  blocked(p, 'new-next', /覆盖/);
  p.set('new-end', '2001-12-31');
  await p.click('new-next');
  await p.click('new-next');
  p.fail();
  await assert.rejects(p.click('new-create'), /creation failed/);
  assert.deepEqual(p.opened, []);
  assert.ok(!p.calls.some((call) => /run_start|run_case|submit/.test(call.command)));
});
test('research entry round trip retains form, scanned sites, option edits and the model second step', async () => {
  const option = { DEF_HIST_FREQ: 'DAILY' },
    p = page('NewCasePage', { form, sites: [site], caseValues: option, step: 4 });
  await p.mount();
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.deepEqual(p.routes, ['versions'], 'model changes start from 新建修改版');
  const seed = p.state.newCaseSeed as {
    form: typeof form;
    sites: unknown[];
    caseValues: typeof option;
    step: number;
  };
  assert.equal(seed.form.start, form.start);
  assert.deepEqual(seed.sites, [site]);
  assert.deepEqual(seed.caseValues, { ...option, ...modelSettings.caseSchemeFields('IGBP') });
  assert.equal(seed.step, 4);
  let c = await p.remount();
  assert.equal(c.details.step, 4);
  c = await p.click('new-next');
  assert.equal(c.fields.find((f) => f.id === 'new-option-DEF_HIST_FREQ')?.value, 'DAILY');
});
test('typing 2001 dates keeps partial input visible, blocks save, and preserves seconds and spinup', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-3');
  for (const partial of [
    '2',
    '20',
    '200',
    '2001',
    '2001-',
    '2001-0',
    '2001-01',
    '2001-01-',
    '2001-01-0',
  ]) {
    const c = p.set('case-start-date', partial);
    assert.equal(c.fields.find((f) => f.id === 'case-start-date')?.value, partial);
    assert.equal(c.status, 'unsaved');
    assert.ok(!c.actions.some((a) => a.id === 'case-next'));
    await assert.rejects(p.click('case-save'), /时段/);
  }
  assert.equal(
    p.calls.filter((call) => call.command === 'case_data_save').length,
    0,
    'old valid date must not be saved in place of an incomplete edit',
  );
  p.set('case-start-date', '2001-01-01');
  assert.ok(p.guard(), 'dirty date prevents route changes');
  await p.click('case-save');
  const saved = p.calls.find((call) => call.command === 'case_data_save');
  assert.ok(saved);
  assert.deepEqual(saved.args.fields, [{ path: 'DEF_simulation_time%start_year', value: '2001' }]);
  assert.equal(p.guard(), null);
  assert.equal(p.render().details.simulation_step, 4);
});
test('dirty case options disable model adjustment and running until saved or discarded', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-5');
  p.set('case-DEF_HIST_FREQ', 'DAILY');
  await p.goto('case-step-4');
  assert.equal(p.element('ModelSettings').props.disabled, true);
  assert.ok(p.guard());
  assert.equal(p.element('RunPanel').props.disabled, true);
  await p.click('case-discard');
  assert.equal(p.element('ModelSettings').props.disabled, false);
  assert.equal(p.guard(), null);
});

test('data options exposed to the assistant are rendered and editable in the data step', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 3 });
  const c = await p.mount();
  assert.ok(c.fields.some((f) => f.id === 'new-option-DEF_dir_existing_srfdata'));
  const wrapper = p.elements.find(
    (e) => e.type === 'Field' && String(e.props.label).startsWith('已有地表资料目录'),
  );
  assert.ok(wrapper);
  const input = wrapper.children.find((child) => (child as Element)?.type === 'Input') as
    Element | undefined;
  assert.ok(input, 'existing surface data is a visible input');
  (input.props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '/custom/surface' },
  });
  assert.equal(
    p.render().fields.find((f) => f.id === 'new-option-DEF_dir_existing_srfdata')?.value,
    '/custom/surface',
  );
});
test('spin-up is set in the data step by start, end and repetitions without the ambiguous cutoff second', async () => {
  const p = page('NewCasePage', {
    form: { ...form, spinup_end: '2002-01-01', end: '2002-06-30' },
    sites: [{ ...site, end: '2002-12-31' }],
    step: 3,
  });
  await p.mount();
  p.set('new-advanced', 'true');
  assert.ok(
    !p.render().fields.some((f) => f.id === 'new-option-DEF_simulation_time%spinup_sec'),
    'spinup seconds are no longer a separate option',
  );
  assert.ok(
    p.elements.some((e) => e.type === 'Input' && e.props.readOnly && e.props.value === form.start),
    'the spin-up start is shown read-only as the simulation start',
  );
  p.set('new-option-DEF_simulation_time%spinup_repeat', '3');
  await p.click('new-next');
  await p.click('new-next');
  await p.click('new-create');
  const request = p.calls.find((c) => c.command === 'new_case');
  assert.ok(request);
  const fields = request.args.fields as { path: string; value: string }[];
  const value = (path: string) => fields.filter((f) => f.path === path).map((f) => f.value);
  assert.deepEqual(value('DEF_simulation_time%spinup_year'), ['2002']);
  assert.deepEqual(value('DEF_simulation_time%spinup_month'), ['1']);
  assert.deepEqual(value('DEF_simulation_time%spinup_sec'), ['0']);
  assert.deepEqual(value('DEF_simulation_time%spinup_repeat'), ['3']);
});

function guideFor(p: ReturnType<typeof page>) {
  const source = readFileSync(new URL('../../lib/guide.ts', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace(/^export /gm, '');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022 },
  }).outputText;
  const owner = Symbol('simulation');
  const guide = new Function(
    'pageExecutionContext',
    'publicUiResult',
    'waitForContract',
    'requestAnimationFrame',
    `${code};return {pages,drive,bind:value=>{app=value;}};`,
  )(pageExecutionContext, publicUiResult, waitForContract, (callback: () => void) => {
    guide.pages.set(owner, p.render());
    callback();
  }) as {
    pages: Map<symbol, Contract>;
    drive: (action: string, args: Record<string, unknown>) => Promise<unknown>;
    bind: (value: unknown) => void;
  };
  guide.bind({ state: p.state });
  guide.pages.set(owner, p.render());
  return {
    fill: async (field: string, value: string) => {
      guide.pages.set(owner, p.render());
      return guide.drive('fill', { fields: [{ field, value }] });
    },
  };
}
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} accepts assistant text and numeric edits, and restricts logical values through the real guide driver`, async () => {
    const p = page(name, name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined);
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    const guide = guideFor(p),
      prefix = name === 'NewCasePage' ? 'new-option-' : 'case-';
    await guide.fill(`${prefix}DEF_dir_existing_srfdata`, '/custom surface/path');
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_dir_existing_srfdata`)?.value,
      '/custom surface/path',
    );
    if (name === 'NewCasePage') {
      await p.click('new-next');
      await p.click('new-next');
    } else {
      await p.click('case-save');
      await p.goto('case-step-5');
    }
    await guide.fill(`${prefix}DEF_simulation_time%timestep`, '900');
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_simulation_time%timestep`)?.value,
      '900',
    );
    const logical = `${prefix}DEF_simulation_time%greenwich`;
    assert.equal(p.render().fields.find((f) => f.id === logical)?.type, 'choice');
    for (const value of ['.true.', '.false.']) {
      await guide.fill(logical, value);
      assert.equal(p.render().fields.find((f) => f.id === logical)?.value, value);
    }
    await assert.rejects(guide.fill(logical, 'yes'), /unsupported option/);
    assert.equal(
      p.render().fields.find((f) => f.id === logical)?.value,
      '.false.',
      'invalid values never reach the setter',
    );
  });

test('existing case refuses spinup cutoff outside its period and clearing it saves only the disabled year', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-spinup-date', '2030-01-01');
  await assert.rejects(p.click('case-save'), /预热|Spinup/i);
  assert.equal(p.calls.filter((call) => call.command === 'case_data_save').length, 0);
  p.set('case-spinup-date', '');
  await p.click('case-save');
  const request = p.calls.find((call) => call.command === 'case_data_save');
  assert.ok(request);
  assert.deepEqual(request.args.fields, [{ path: 'DEF_simulation_time%spinup_year', value: '0' }]);
  assert.equal(p.render().details.simulation_step, 4);
});
test('moving simulation start beyond the existing spinup cutoff refuses to save unchanged invalid spinup', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-start-date', '2001-08-01');
  await assert.rejects(p.click('case-save'), /预热|Spinup/i);
  assert.equal(p.calls.filter((call) => call.command === 'case_data_save').length, 0);
  assert.equal(p.render().details.simulation_step, 3);
  assert.ok(p.guard());
});

test('new output defaults stay automatic and clearing an edited frequency removes its override', async () => {
  const p = page('NewCasePage', {
    form,
    sites: [site],
    step: 5,
    caseValues: { DEF_dir_output: '/old/unmanaged/output' },
  });
  const c = await p.mount();
  const frequency = c.fields.find((f) => f.id === 'new-option-DEF_HIST_FREQ');
  assert.ok(frequency);
  assert.equal(frequency.value, '');
  assert.ok(frequency.options?.some((o) => o.value === ''));
  assert.ok(!c.fields.some((f) => f.id === 'new-option-DEF_dir_output'));
  const guide = guideFor(p);
  await guide.fill('new-option-DEF_HIST_FREQ', 'DAILY');
  await guide.fill('new-option-DEF_HIST_FREQ', '');
  assert.equal(p.render().fields.find((f) => f.id === 'new-option-DEF_HIST_FREQ')?.value, '');
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case');
  assert.ok(request);
  const fields = request.args.fields as { path: string; value: string }[];
  assert.ok(
    !fields.some((f) => ['DEF_HIST_FREQ', 'DEF_WRST_FREQ', 'DEF_dir_output'].includes(f.path)),
    'automatic defaults and stale output overrides are not submitted',
  );
});
test('the managed output directory stays inside the case and refuses assistant changes', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-1');
  const id = 'case-DEF_dir_output',
    expected = '/project/site/out';
  const output = p.render().fields.find((f) => f.id === id);
  assert.ok(output);
  assert.equal(output.value, expected);
  assert.equal(output.disabled, true);
  const input = p.elements.find((e) => e.type === 'Input' && e.props.value === expected);
  assert.ok(input);
  assert.equal(input.props.readOnly, true);
  await assert.rejects(guideFor(p).fill(id, '/external/output'), /cannot be changed/);
  assert.equal(p.render().fields.find((f) => f.id === id)?.value, expected);
});
test('clearing spatial timestep restores the required automatic 1800 seconds without a case override', async () => {
  const p = page('NewCasePage', {
    form: { ...form, kind: 'global', dlon: '1', dlat: '1', forcing: '/forcing.nml' },
    step: 5,
  });
  await p.mount();
  const guide = guideFor(p);
  await guide.fill('new-option-DEF_simulation_time%timestep', '900');
  await guide.fill('new-option-DEF_simulation_time%timestep', '');
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_spatial_case');
  assert.ok(request);
  assert.equal(request.args.timestep, 1800);
  assert.ok(
    !(request.args.fields as { path: string }[]).some(
      (f) => f.path === 'DEF_simulation_time%timestep',
    ),
  );
});

test('research round trips preserve explicit own-data disclosure rather than inferring it from bundled directories', async () => {
  const p = page('NewCasePage', {
    multiSiteEnabled: false,
    form: { ...form, sitedir: '/bundled/sites', forcingdir: '/bundled/forcing' },
    sites: [site],
    step: 3,
  });
  let contract = await p.mount();
  assert.ok(
    !contract.fields.some((f) => f.id === 'new-sitedir'),
    'bundled directories do not open the own-data section',
  );
  await p.click('new-next');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.equal((p.state.newCaseSeed as { ownData: boolean }).ownData, false);
  await p.remount();
  contract = await p.click('new-previous');
  assert.ok(
    !contract.fields.some((f) => f.id === 'new-sitedir'),
    'returning from model settings keeps bundled data collapsed',
  );
  contract = await p.click('new-own-data');
  assert.equal(contract.fields.find((f) => f.id === 'new-sitedir')?.value, '');
  p.set('new-sitedir', '/custom/sites');
  await p.click('new-scan');
  await p.click('new-select-CN-Cng');
  for (const key of ['site', 'met', 'runtime', 'start', 'end'] as const)
    p.set(`new-${key}`, form[key]);
  await p.click('new-next');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.equal((p.state.newCaseSeed as { ownData: boolean }).ownData, true);
  await p.remount();
  contract = await p.click('new-previous');
  assert.equal(
    contract.fields.find((f) => f.id === 'new-sitedir')?.value,
    '/custom/sites',
    'an explicitly opened own-data section remains open after model editing',
  );
});

test('USGS classification survives model editing and creates the chosen template without fake physics switches', async () => {
  for (const urban of [false, true]) {
    const p = page(
      'NewCasePage',
      {
        form,
        sites: [{ ...site, urban }],
        step: 4,
        caseValues: urban ? { DEF_URBAN_RUN: '.true.' } : {},
      },
      [site],
      initialValues,
      { schema: [...schema, { ...field('DEF_URBAN_RUN', '城市', '.false.'), kind: 'Logical' }] },
    );
    await p.mount();
    assert.equal(p.element('ModelSettings').props.classification, 'IGBP');
    (p.element('ModelSettings').props.onClassificationChange as (value: string) => void)('USGS');
    assert.equal(p.render().details.classification, 'USGS');
    (p.element('ModelSettings').props.onEdit as () => void)();
    assert.deepEqual(p.state.simulationReturn, { route: 'new-case', step: 4 });
    assert.equal((p.state.newCaseSeed as { form: { landcover: string } }).form.landcover, 'USGS');
    const returned = await p.remount();
    assert.equal(returned.details.step, 4);
    assert.equal(p.element('ModelSettings').props.classification, 'USGS');
    await p.click('new-next');
    await p.click('new-create');
    const request = p.calls.find((call) => call.command === 'new_case');
    assert.ok(request);
    assert.equal(request.args.mode, urban ? 'urban-usgs' : 'usgs');
    const fields = request.args.fields as { path: string; value: string }[];
    assert.ok(
      !fields.some((f) => ['DEF_USE_USGS', 'DEF_USE_IGBP'].includes(f.path)),
      'classification must not fabricate unsupported compile switches',
    );
    assert.deepEqual(p.state.version.overlay.physics, {});
  }
});
test('existing case returns from version editing to the model step after the data step', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-4');
  (p.element('ModelSettings').props.onEdit as () => void)();
  assert.deepEqual(p.routes, ['versions'], 'model changes start from 新建修改版');
  assert.deepEqual(p.state.simulationReturn, { route: 'case', step: 4 });
  const returned = await p.remount();
  assert.equal(returned.details.simulation_step, 4);
  assert.ok(p.element('ModelSettings'));
  const data = await p.click('case-previous');
  assert.equal(data.details.simulation_step, 3);
  assert.ok(data.fields.some((field) => field.id === 'case-start-date'));
});
test('existing case can reach its data step to repair dates after first reviewing model settings', async () => {
  const p = page('CasesPage', undefined, [site], {
    ...initialValues,
    'DEF_simulation_time%start_year': '',
  });
  await p.mount();
  await p.goto('case-step-4');
  const data = await p.click('case-previous');
  assert.equal(data.details.simulation_step, 3);
  await assert.rejects(p.click('case-next'), /时段/);
  assert.equal(p.render().details.simulation_step, 3);
});

const processSchema = [
  ...schema,
  ...['DEF_USE_BGC', 'DEF_USE_TRACER', 'DEF_USE_GridRiverLakeFlow', 'DEF_URBAN_RUN'].map(
    (name) => ({ ...field(name, '地表数据', '.false.'), kind: 'Logical' }),
  ),
  { ...field('DEF_USE_CN_INIT', '初始场', '.false.'), kind: 'Logical' },
  { ...field('DEF_file_cn_init', '初始场', 'null'), kind: 'Character' },
  { ...field('DEF_UnitCatchment_file', '河道与水库', 'null'), kind: 'Character' },
  { ...field('DEF_TRACER_SOIL_INIT_FILE', '初始场', 'null'), kind: 'Character' },
  { ...field('DEF_URBAN_type_scheme', '地表数据', '1'), kind: 'Integer' },
];
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} queries applicability using the exact frozen revision and current unsaved case data`, async () => {
    const draft = {
      DEF_USE_BGC: '.true.',
      DEF_USE_CN_INIT: '.true.',
      DEF_file_cn_init: '/unsaved-cn.nc',
    };
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3, caseValues: draft } : undefined,
      [site],
      { ...initialValues, ...draft },
      { schema: processSchema },
    );
    await p.mount();
    if (name === 'CasesPage') {
      await p.goto('case-step-3');
      await p.settle();
      assert.deepEqual(
        p.calls.filter((call) => call.command === 'model_configuration').at(-1)?.args.caseValues,
        {},
      );
      p.set('case-advanced', 'true');
      await guideFor(p).fill('case-DEF_dir_existing_srfdata', '/changed-cn.nc');
      await p.settle();
    }
    const request = p.calls.filter((call) => call.command === 'model_configuration').at(-1);
    assert.ok(
      request,
      'case data must request contextual applicability rather than reusing initial hidden fields',
    );
    assert.deepEqual(request.args.source, { kind: 'frozen', version: p.state.versionRef });
    assert.equal(request.args.caseDir, name === 'CasesPage' ? '/project/site' : null);
    assert.ok(
      JSON.stringify(request.args.caseValues).includes(
        name === 'CasesPage' ? '/changed-cn.nc' : '/unsaved-cn.nc',
      ),
      'condition query uses the current case values',
    );
  });

const dataGroups = [
  ['bgc', 'DEF_USE_BGC', 'DEF_file_cn_init'],
  ['river', 'DEF_USE_GridRiverLakeFlow', 'DEF_UnitCatchment_file'],
  ['tracer', 'DEF_USE_TRACER', 'DEF_TRACER_SOIL_INIT_FILE'],
  ['urban', 'DEF_URBAN_RUN', 'DEF_URBAN_type_scheme'],
] as const;
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} keeps process settings out of data-step fields and guide actions without deleting values`, async () => {
    const retained = Object.fromEntries(
      dataGroups.flatMap(([, parent, child]) => [
        [parent, '.true.'],
        [child, '/retained.nc'],
      ]),
    );
    const p = page(
      name,
      // 保留的配置开着城市过程：数据步骤会核对站点类型，所以这里用城市站。
      name === 'NewCasePage'
        ? { form, sites: [{ ...site, urban: true }], step: 3, caseValues: retained }
        : undefined,
      [site],
      { ...initialValues, ...retained },
      { schema: processSchema },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const contract = p.render();
    assert.ok(!contract.actions.some((a) => a.id.includes('data-group-')));
    assert.ok(
      !contract.fields.some((f) =>
        dataGroups.some(([, parent, child]) => f.id.endsWith(parent) || f.id.endsWith(child)),
      ),
    );
    assert.ok(contract.fields.some((f) => f.id.endsWith('DEF_dir_existing_srfdata')));
    assert.ok(!p.elements.some((e) => e.type === 'ProcessDataGroups'));
    assert.ok(!p.calls.some((call) => call.command === 'case_data_save'));
    await p.goto(name === 'NewCasePage' ? 'new-step-4' : 'case-step-4');
    const environment = p.element('ModelSettings').props.setupEnvironment as {
      caseValues: Record<string, string>;
    };
    for (const [, , child] of dataGroups)
      assert.equal(environment.caseValues[child], '/retained.nc');
  });
test('existing case run and result tabs never expose settings-only process data actions', async () => {
  const p = page(
    'CasesPage',
    undefined,
    [site],
    { ...initialValues, DEF_USE_BGC: '.true.' },
    { schema: processSchema },
  );
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-advanced', 'true');
  assert.ok(!p.render().actions.some((a) => a.id === 'case-data-group-bgc'));
  for (const tab of ['runs', 'results']) {
    await p.click(`case-tab-${tab}`);
    assert.ok(!p.render().actions.some((a) => a.id.startsWith('case-data-group-')));
    assert.deepEqual(p.render().fields, []);
    await p.click('case-tab-settings');
  }
});

for (const mode of ['IGBP', 'USGS', 'PFT', 'PC'] as const)
  test(`${mode} selection saves directly to the new case without a model revision`, async () => {
    const p = page('NewCasePage', {
      form,
      sites: [site],
      caseValues: { DEF_HIST_FREQ: 'DAILY' },
      step: 4,
    });
    await p.mount();
    (p.element('ModelSettings').props.onModeChange as (mode: string) => void)(mode);
    p.render();
    assert.deepEqual(p.routes, []);
    assert.equal(p.element('ModelSettings').props.mode, mode);
    assert.deepEqual(p.state.version.overlay.physics, {});
    await p.click('new-next');
    await p.click('new-create');
    const request = p.calls.find((c) => c.command === 'new_case');
    assert.ok(request);
    assert.equal(request.args.mode, mode.toLowerCase());
    const fields = request.args.fields as { path: string; value: string }[];
    assert.equal(fields.find((f) => f.path === 'DEF_SUBGRID_SCHEME')?.value, mode);
    assert.ok(!p.calls.some((c) => c.command === 'model_create'));
  });
test('case scheme changes preserve explicit model physics and never derive versions', async () => {
  const p = page('NewCasePage', { form: { ...form, subgrid: 'PFT' }, sites: [site], step: 4 });
  Object.assign(p.state.version.overlay.physics, { DEF_USE_BGC: '.true.' });
  await p.mount();
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('USGS');
  p.render();
  assert.deepEqual(p.routes, []);
  assert.equal(p.element('ModelSettings').props.mode, 'USGS');
  assert.equal((p.state.version.overlay.physics as Record<string, string>).DEF_USE_BGC, '.true.');
});

test('new-case setup edits stay in the case and preserve inputs without visiting versions', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, landcover: 'USGS' }, sites: [site], step: 4 },
    [site],
    initialValues,
    {
      schema: [
        ...schema,
        ...['DEF_USE_Campbell_SOIL_MODEL', 'DEF_USE_VariablySaturatedFlow'].map((name) => ({
          ...field(name, '水热过程', '.false.'),
          kind: 'Logical',
        })),
      ],
    },
  );
  await p.mount();
  const model = p.element('ModelSettings');
  const patch = { DEF_USE_Campbell_SOIL_MODEL: '.false.', DEF_USE_VariablySaturatedFlow: '.true.' };
  (model.props.onCaseChange as (p: object) => void)(patch);
  p.render();
  assert.deepEqual(p.routes, []);
  assert.deepEqual(p.state.version.overlay.physics, {});
  assert.equal(
    (p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> })
      .caseValues.DEF_USE_Campbell_SOIL_MODEL,
    '.false.',
  );
});
test('existing case options save atomically and clear explicit crop settings without a model revision', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-4');
  const patch = {
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_USE_VariablySaturatedFlow: '.true.',
    DEF_TUNING_CROP_PLANTING_DAY: null,
  };
  await (p.element('ModelSettings').props.onCaseChange as (p: object) => Promise<void>)(patch);
  await p.settle();
  const saved = p.calls.find((c) => c.command === 'case_data_save')!;
  assert.deepEqual(saved.args.fields, [
    { path: 'DEF_USE_Campbell_SOIL_MODEL', value: '.false.' },
    { path: 'DEF_USE_VariablySaturatedFlow', value: '.true.' },
  ]);
  assert.deepEqual(saved.args.removeFields, ['DEF_TUNING_CROP_PLANTING_DAY']);
  assert.deepEqual(p.routes, []);
  assert.equal(p.guard(), null);
  assert.deepEqual(p.state.version.overlay.physics, {});
});

for (const mode of ['IGBP', 'USGS', 'PFT', 'PC'] as const)
  test(`existing case ${mode} selection writes case fields without deriving a revision`, async () => {
    const p = page('CasesPage');
    await p.mount();
    await p.goto('case-step-4');
    (p.element('ModelSettings').props.onModeChange as (mode: string) => void)(mode);
    await p.settle();
    const saved = p.calls.find((c) => c.command === 'case_data_save');
    assert.ok(saved);
    assert.deepEqual(
      saved.args.fields,
      Object.entries(modelSettings.caseSchemeFields(mode)).map(([path, value]) => ({
        path,
        value,
      })),
    );
    assert.deepEqual(p.routes, []);
    assert.deepEqual(p.state.version.overlay.physics, {});
    assert.equal(p.guard(), null);
    assert.ok(!p.calls.some((c) => c.command === 'model_create'));
  });
test('failed case scheme save leaves saved case state and selected version intact', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-4');
  p.fail();
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('PC');
  await p.settle();
  assert.equal(p.guard(), null);
  assert.deepEqual(p.state.version.overlay.physics, {});
  assert.ok(!p.calls.some((c) => c.command === 'model_create'));
  assert.match(JSON.stringify(p.elements), /save failed/);
});

for (const mode of ['PFT', 'PC'] as const)
  test(`new ${mode} uses the supported IGBP data source when switching from USGS`, async () => {
    const p = page('NewCasePage', { form: { ...form, landcover: 'USGS' }, sites: [site], step: 4 });
    await p.mount();
    (p.element('ModelSettings').props.onModeChange as (value: string) => void)(mode);
    p.render();
    assert.equal(p.element('ModelSettings').props.classification, 'IGBP');
    assert.equal(p.element('ModelSettings').props.onClassificationChange, undefined);
    assert.equal(
      (p.element('ModelSettings').props.setupEnvironment as { subgrid: string }).subgrid,
      mode,
    );
  });

test('same-kernel case scheme changes requery process conditions with the saved scheme', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-4');
  for (const mode of ['IGBP', 'PFT', 'PC', 'USGS']) {
    const before = p.calls.filter((c) => c.command === 'model_configuration').length;
    (p.element('ModelSettings').props.onModeChange as (mode: string) => void)(mode);
    await p.settle();
    const queries = p.calls.filter((c) => c.command === 'model_configuration');
    assert.ok(queries.length > before);
    assert.equal(
      (queries.at(-1)!.args.caseValues as Record<string, string>).DEF_SUBGRID_SCHEME,
      mode,
    );
    assert.equal(queries.at(-1)!.args.kernelDir, null);
    assert.equal(p.guard(), null);
  }
});

for (const subgrid of ['PFT', 'PC'])
  test(`restored ${subgrid} draft uses IGBP while preserving the other pending inputs`, async () => {
    const restored = {
        ...form,
        subgrid,
        landcover: 'USGS',
        name: 'restored vegetation',
        start: '2001-02-01',
      },
      options = { DEF_HIST_FREQ: 'DAILY' };
    const p = page('NewCasePage', {
      form: restored,
      sites: [site],
      caseValues: options,
      step: 4,
      ownData: true,
    });
    await p.mount();
    const model = p.element('ModelSettings');
    assert.equal(model.props.mode, subgrid);
    assert.equal(model.props.classification, 'IGBP');
    assert.equal(model.props.onClassificationChange, undefined);
    (model.props.onEdit as () => void)();
    p.render();
    const seed = p.state.newCaseSeed as {
      form: typeof restored;
      sites: unknown[];
      caseValues: Record<string, string>;
      step: number;
      ownData: boolean;
    };
    assert.deepEqual(seed.form, {
      ...restored,
      host: '',
      landcover: 'IGBP',
      sitedir: '',
      forcingdir: '',
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
      forcing_mode: 'directory',
      spatial_forcingdir: '',
      forcing_dataset: '',
      run_threads: '',
      run_ranks: '1',
      run_workers: '1',
      run_scheduler: '',
      run_cpus: '',
    });
    assert.deepEqual(seed.sites, [site]);
    assert.equal(seed.caseValues.DEF_HIST_FREQ, 'DAILY');
    assert.equal(seed.step, 4);
    assert.equal(seed.ownData, true);
  });

test('same-kernel process saves refresh case-data prerequisite queries and preserve source settings', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-4');
  for (const value of ['.true.', '.false.']) {
    const before = p.calls.filter((c) => c.command === 'model_configuration').length;
    await (p.element('ModelSettings').props.onCaseChange as (patch: object) => Promise<void>)({
      DEF_USE_BGC: value,
    });
    await p.settle();
    assert.ok(p.calls.filter((c) => c.command === 'model_configuration').length > before);
    assert.equal(
      (p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> })
        .caseValues.DEF_USE_BGC,
      value,
    );
  }
  assert.deepEqual(p.routes, []);
  assert.deepEqual(p.state.version.overlay.physics, {});
});

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} research entry never derives a model revision`, async () => {
    const p = page(name, name === 'NewCasePage' ? { form, sites: [site], step: 4 } : undefined);
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-4');
    (p.element('ModelSettings').props.onEdit as () => void)();
    assert.deepEqual(p.routes, ['versions'], 'model changes start from 新建修改版');
    assert.ok(
      !p.calls.some(
        (call) =>
          call.command === 'model_create' ||
          call.command === 'model_save_draft' ||
          call.command === 'model_freeze',
      ),
    );
  });

const setupSchema = [
  ...schema,
  { ...field('DEF_URBAN_RUN', '城市', '.false.'), kind: 'Logical' },
  { ...field('DEF_USE_BGC', '生态与生地化', '.false.'), kind: 'Logical' },
  field('DEF_TUNING_CROP_PLANTING_DAY', '生态与生地化', '0'),
];
const setupKernels = [
  { dir: '/kernel/default', preset: 'default', macros: ['SinglePoint', 'LULC_IGBP'] },
  { dir: '/kernel/crop', preset: 'crop', macros: ['SinglePoint', 'LULC_IGBP', 'CROP'] },
];
test('Fortran preset shows the actual automatic match without an editable preset field', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, engine: 'fortran' }, sites: [site], step: 1 },
    [site],
    initialValues,
    { schema: setupSchema, kernels: setupKernels },
  );
  await p.mount();
  const preset = () => {
    const field = p.elements.find(
      (element) => element.type === 'Field' && element.props.label === '匹配的编译配置',
    );
    assert.ok(field);
    const input = field.children[0] as Element;
    assert.equal(input.type, 'Input');
    assert.equal(input.props.readOnly, true);
    assert.equal(input.props.onChange, undefined);
    assert.ok(!p.render().fields.some((field) => field.id === 'new-kernel'));
    return input.props.value;
  };
  assert.equal(preset(), 'default');
  await p.goto('new-step-4');
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).preset, 'default');
  await p.goto('new-step-1');
  p.state.kernels = [];
  p.render();
  assert.match(String(preset()), /未找到匹配预设/);
});

test('case urban switch controls station compatibility and urban creation mode without changing model parameters', async () => {
  const urban = { ...site, name: 'urban', urban: true };
  const p = page(
    'NewCasePage',
    { form, sites: [urban, site], step: 4 },
    [urban, site],
    initialValues,
    {
      schema: setupSchema,
      kernels: setupKernels,
      modelFields: [field('DEF_TUNING_CSOILC', '生态与生地化')],
    },
  );
  Object.assign(p.state.version.overlay.physics, { DEF_TUNING_CSOILC: '0.7' });
  await p.mount();
  (p.element('ModelSettings').props.onCaseChange as (patch: object) => void)({
    DEF_URBAN_RUN: '.true.',
  });
  p.render();
  await p.click('new-previous');
  const c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-select-urban')!.enabled, true);
  assert.equal(c.actions.find((a) => a.id === 'new-select-CN-Cng')!.enabled, true);
  await p.click('new-select-urban');
  await p.click('new-next');
  await p.click('new-next');
  await p.click('new-create');
  const create = p.calls.find((c) => c.command === 'new_case')!;
  assert.equal(create.args.mode, 'urban-igbp');
  const sent = create.args.fields as { path: string; value: string }[];
  assert.equal(sent.filter((f) => f.path === 'DEF_URBAN_RUN').length, 1);
  assert.equal(sent.find((f) => f.path === 'DEF_URBAN_RUN')!.value, '.true.');
  assert.equal(sent.filter((f) => f.path === 'DEF_TUNING_CSOILC').length, 1);
  assert.deepEqual(p.state.version.overlay.physics, { DEF_TUNING_CSOILC: '0.7' });
});
test('case crop planting selects the crop kernel and station; clearing it returns to the default kernel', async () => {
  const crop = { ...site, name: 'crop', crop: true };
  const p = page(
    'NewCasePage',
    { form: { ...form, subgrid: 'PFT' }, sites: [crop, site], step: 4 },
    [crop, site],
    initialValues,
    { schema: setupSchema, kernels: setupKernels },
  );
  await p.mount();
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).dir, '/kernel/default');
  (p.element('ModelSettings').props.onCaseChange as (patch: object) => void)({
    DEF_USE_BGC: '.true.',
    DEF_TUNING_CROP_PLANTING_DAY: '120',
  });
  p.render();
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).dir, '/kernel/crop');
  await p.click('new-previous');
  const c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-select-crop')!.enabled, true);
  assert.equal(c.actions.find((a) => a.id === 'new-select-CN-Cng')!.enabled, true);
  // 作物配置需要作物站点：选中作物站后数据步骤才放行，回到模型步骤。
  await p.click('new-select-crop');
  await p.click('new-next');
  (p.element('ModelSettings').props.onCaseChange as (patch: object) => void)({
    DEF_TUNING_CROP_PLANTING_DAY: null,
  });
  p.render();
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).dir, '/kernel/default');
  assert.deepEqual(p.state.version.overlay.physics, {});
});
test('case BGC selection requires runtime input even when the model revision has no process switches', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, runtime: '', subgrid: 'PFT' }, sites: [site], step: 4 },
    [site],
    initialValues,
    { schema: setupSchema },
  );
  await p.mount();
  (p.element('ModelSettings').props.onCaseChange as (patch: object) => void)({
    DEF_USE_BGC: '.true.',
  });
  p.render();
  await p.click('new-previous');
  blocked(p, 'new-next', /Runtime/);
  assert.ok(!p.calls.some((c) => c.command === 'new_case'));
  assert.deepEqual(p.state.version.overlay.physics, {});
});

test('legacy version switches cannot activate urban crop BGC or a vegetation scheme in a new case', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 4 }, [site], initialValues, {
    schema: setupSchema,
    kernels: setupKernels,
    modelFields: [field('DEF_TUNING_CSOILC', '生态与生地化')],
  });
  const legacy = {
    DEF_URBAN_RUN: '.true.',
    DEF_USE_BGC: '.true.',
    DEF_TUNING_CROP_PLANTING_DAY: '0',
    DEF_USE_PC: '.true.',
    DEF_TUNING_CSOILC: '0.7',
  };
  Object.assign(p.state.version.overlay.physics, legacy);
  await p.mount();
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).dir, '/kernel/default');
  await p.click('new-previous');
  assert.equal(p.render().actions.find((a) => a.id === 'new-select-CN-Cng')!.enabled, true);
  await p.click('new-next');
  await p.click('new-next');
  await p.click('new-create');
  const args = p.calls.find((c) => c.command === 'new_case')!.args;
  assert.equal(args.mode, 'igbp');
  const fields = args.fields as { path: string; value: string }[];
  assert.ok(
    !fields.some((f) =>
      ['DEF_URBAN_RUN', 'DEF_USE_BGC', 'DEF_TUNING_CROP_PLANTING_DAY'].includes(f.path),
    ),
  );
  assert.equal(fields.find((f) => f.path === 'DEF_TUNING_CSOILC')!.value, '0.7');
  assert.deepEqual(p.state.version.overlay.physics, legacy);
});
test('existing case ignores schema planting-day zero and stale crop flags when the field is absent', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, {
    schema: setupSchema,
    kernels: setupKernels,
  });
  Object.assign(p.state.version.overlay.physics, {
    DEF_TUNING_CROP_PLANTING_DAY: '120',
    DEF_USE_BGC: '.true.',
    DEF_URBAN_RUN: '.true.',
  });
  await p.mount();
  await p.goto('case-step-4');
  assert.equal((p.element('ModelSettings').props.kernel as KernelChoice).dir, '/kernel/default');
  const actual = (
    p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> }
  ).caseValues;
  assert.equal(actual.DEF_TUNING_CROP_PLANTING_DAY, undefined);
  assert.equal(actual.DEF_USE_BGC, undefined);
  assert.equal(actual.DEF_URBAN_RUN, undefined);
});

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} hides inactive and unspecified process inputs even in advanced settings`, async () => {
    for (const parentValue of [undefined, '.false.']) {
      const retained = Object.fromEntries(
        dataGroups.flatMap(([, parent, child]) => [
          [child, '/retained.nc'],
          ...(parentValue ? [[parent, parentValue]] : []),
        ]),
      );
      const p = page(
        name,
        name === 'NewCasePage' ? { form, sites: [site], step: 3, caseValues: retained } : undefined,
        [site],
        { ...initialValues, ...retained },
        { schema: processSchema },
      );
      p.state.expert = true;
      await p.mount();
      if (name === 'CasesPage') await p.goto('case-step-3');
      const contract = p.render();
      assert.ok(!contract.actions.some((a) => a.id.includes('data-group-')));
      for (const [, , child] of dataGroups)
        assert.ok(!contract.fields.some((f) => f.id.endsWith(child)));
      assert.equal(
        p.calls.some((call) => call.command === 'case_data_save'),
        false,
      );
    }
  });

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} hides confirmed inactive child inputs while keeping unknown and readonly data`, async () => {
    const children = [
      'DEF_USE_IRRIGATION',
      'DEF_USE_FERT',
      'DEF_TUNING_CROP_PLANTING_DAY',
      'DEF_file_cn_init',
      'DEF_file_Ozone',
      'DEF_Aerosol_Clim',
      'DEF_TRACER_SOIL_INIT_FILE',
    ];
    const values = {
      ...initialValues,
      DEF_USE_BGC: '.true.',
      DEF_USE_TRACER: '.true.',
      ...Object.fromEntries(children.map((key) => [key, 'retained'])),
    };
    const extra = [
      'DEF_USE_BGC',
      'DEF_USE_TRACER',
      'UNKNOWN_DATA',
      'READONLY_DATA',
      'DEF_USE_CN_INIT',
      ...children,
    ].map((key) => field(key, '地表数据', ''));
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3, caseValues: values } : undefined,
      [site],
      values,
      {
        schema: [...schema, ...extra],
        configuration: async () => ({
          fields: [
            ...children.map((key) => ({ name: key, mode: 'disabled', pending: false })),
            { name: 'UNKNOWN_DATA', mode: 'disabled', pending: true },
            { name: 'READONLY_DATA', mode: 'readonly', pending: false },
          ],
        }),
      },
    );
    p.state.expert = true;
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const controls = p.render().fields;
    for (const key of children) assert.ok(!controls.some((f) => f.id.endsWith(key)), key);
    for (const key of ['UNKNOWN_DATA', 'READONLY_DATA'])
      assert.ok(
        controls.some((f) => f.id.endsWith(key)),
        key,
      );
    assert.ok(!p.calls.some((c) => c.command === 'case_data_save' || c.command === 'new_case'));
  });

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} uses backend parent gates without hiding independent unknown fields`, async () => {
    for (const enabled of [undefined, false, true]) {
      const p = page(
        name,
        name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
        [site],
        initialValues,
        {
          schema: [
            ...schema,
            field('CHILD_DATA', '地表数据', ''),
            field('AUXILIARY_DATA', '地表数据', ''),
          ],
          configuration: async () => ({
            fields: [
              {
                name: 'CHILD_DATA',
                mode: 'editable',
                pending: true,
                parent_gate: 'EXISTING_PROCESS_SWITCH',
                parent_gate_enabled: enabled,
              },
              { name: 'AUXILIARY_DATA', mode: 'editable', pending: true },
            ],
          }),
        },
      );
      p.state.expert = true;
      await p.mount();
      if (name === 'CasesPage') await p.goto('case-step-3');
      const controls = p.render().fields;
      assert.equal(
        controls.some((f) => f.id.endsWith('CHILD_DATA') && !f.id.endsWith('AUXILIARY_DATA')),
        enabled === true,
      );
      assert.ok(controls.some((f) => f.id.endsWith('AUXILIARY_DATA')));
    }
  });

test('selecting an urban station configures its case automatically and keeps ordinary stations selectable', async () => {
  const urban = {
    ...site,
    name: 'AU-Preston',
    site_file: '/urban/site.nc',
    met_file: '/urban/met.nc',
    urban: true,
  };
  const p = page(
    'NewCasePage',
    { form, sites: [urban, site], step: 3 },
    [urban, site],
    initialValues,
    { schema: setupSchema, kernels: setupKernels },
  );
  await p.mount();
  assert.equal(p.render().actions.find((a) => a.id === 'new-select-AU-Preston')!.enabled, true);
  await p.click('new-select-AU-Preston');
  await p.click('new-next');
  let settings = p.element('ModelSettings');
  assert.equal(
    (settings.props.setupEnvironment as { caseValues: Record<string, string> }).caseValues
      .DEF_URBAN_RUN,
    '.true.',
  );
  assert.equal(
    (settings.props.setupEnvironment as { caseValues: Record<string, string> }).caseValues
      .DEF_USE_BGC,
    '.false.',
  );
  assert.equal((settings.props.kernel as KernelChoice).dir, '/kernel/default');
  await p.click('new-previous');
  assert.equal(p.render().actions.find((a) => a.id === 'new-select-CN-Cng')!.enabled, true);
  await p.click('new-select-CN-Cng');
  await p.click('new-next');
  settings = p.element('ModelSettings');
  assert.equal(
    (settings.props.setupEnvironment as { caseValues: Record<string, string> }).caseValues
      .DEF_URBAN_RUN,
    '.false.',
  );
  assert.deepEqual(p.state.version.overlay.physics, {});
  assert.ok(!p.calls.some((c) => c.command === 'model_create'));
});

test('selecting a crop example uses backend defaults and matches PFT BGC and a CROP kernel', async () => {
  const crop = {
    ...site,
    name: 'US-Ne3',
    site_file: '/crop/site.nc',
    met_file: '/crop/met.nc',
    crop: true,
  };
  const p = page(
    'NewCasePage',
    { form, sites: [crop, site], step: 3 },
    [crop, site],
    initialValues,
    { schema: setupSchema, kernels: setupKernels },
  );
  await p.mount();
  assert.equal(p.render().actions.find((a) => a.id === 'new-select-US-Ne3')!.enabled, true);
  await p.click('new-select-US-Ne3');
  await p.click('new-next');
  const setup = p.element('ModelSettings').props;
  assert.equal(setup.mode, 'PFT');
  assert.equal((setup.kernel as KernelChoice).dir, '/kernel/crop');
  const fields = (setup.setupEnvironment as { caseValues: Record<string, string> }).caseValues;
  assert.equal(fields.DEF_USE_BGC, '.true.');
  assert.equal(fields.DEF_URBAN_RUN, '.false.');
  assert.equal(fields.DEF_TUNING_CROP_PLANTING_DAY, '120');
  await p.click('new-next');
  await p.click('new-create');
  const create = p.calls.find((c) => c.command === 'new_case')!;
  assert.equal(create.args.mode, 'pft');
  assert.equal(create.args.site, '/crop/site.nc');
  assert.equal(create.args.met, '/crop/met.nc');
  assert.equal(
    (create.args.fields as { path: string; value: string }[]).find(
      (f) => f.path === 'DEF_TUNING_CROP_PLANTING_DAY',
    )?.value,
    '120',
  );
  assert.ok(!p.calls.some((c) => c.command === 'model_create'));
  assert.deepEqual(p.state.version.overlay.physics, {});
});

test('existing case range fields are editable and save only the case before refreshing context', async () => {
  const rangeValues = {
    SITE_fsitedata: '/site.nc',
    SITE_lon_location: '11.3',
    SITE_lat_location: '47.1',
  };
  const p = page(
    'CasesPage',
    undefined,
    [site],
    { ...initialValues, ...rangeValues },
    {
      schema: [
        ...schema,
        ...Object.entries(rangeValues).map(([name, value]) => field(name, '单点站点', value)),
      ],
    },
  );
  await p.mount();
  const before = await p.goto('case-step-2');
  for (const key of ['SITE_lon_location', 'SITE_lat_location'])
    assert.ok(before.fields.find((item) => item.id === `case-${key}`));
  assert.ok(!before.fields.some((item) => item.id === 'case-SITE_fsitedata'));
  p.set('case-SITE_lon_location', '12.5');
  await p.click('case-save');
  assert.deepEqual(
    [...p.calls].reverse().find((call) => call.command === 'case_data_save')!.args.fields,
    [{ path: 'SITE_lon_location', value: '12.5' }],
  );
  assert.equal(p.calls.at(-1)!.command, 'model_case_context');
  assert.ok(!p.calls.some((call) => call.command === 'model_create'));
});

test('new case cancellation waits for backend creation cleanup and retains the full form for retry', async () => {
  let finish!: (error: Error) => void, acknowledge!: (count: number) => void;
  let attempts = 0;
  const p = page(
    'NewCasePage',
    { form, sites: [site], step: 5, caseValues: { DEF_HIST_FREQ: 'DAILY' } },
    [site],
    initialValues,
    {
      create: async () => {
        if (attempts++ === 0)
          return new Promise((_resolve, reject) => {
            finish = reject;
          });
        return {};
      },
      cancelCreation: () =>
        new Promise((resolve) => {
          acknowledge = resolve;
        }),
    },
  );
  await p.mount();
  assert.ok(!p.render().actions.some((a) => a.id === 'new-cancel-creation'));
  const pending = p.click('new-create');
  const rejected = assert.rejects(pending, /cancelled after cleanup/);
  let c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-create')?.enabled, false);
  const cancel = p.click('new-cancel-creation');
  assert.deepEqual(p.calls.find((call) => call.command === 'case_creation_cancel')?.args, {
    dir: '/project/CN-Cng/CN-Cng',
  });
  c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-cancel-creation')?.enabled, false);
  acknowledge(1);
  await cancel;
  c = p.render();
  assert.equal(c.status, 'busy');
  assert.equal(c.actions.find((a) => a.id === 'new-create')?.enabled, false);
  assert.deepEqual(p.opened, []);
  finish(new Error('cancelled after cleanup'));
  await rejected;
  c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-create')?.enabled, true);
  assert.ok(!c.actions.some((a) => a.id === 'new-cancel-creation'));
  assert.equal(c.details.step, 5);
  await p.click('new-create');
  const requests = p.calls.filter((call) => call.command === 'new_case');
  assert.deepEqual(
    requests[1].args,
    requests[0].args,
    'retry retains all dates, files and option values',
  );
  assert.deepEqual(p.opened, ['/project/CN-Cng/CN-Cng']);
});
test('failed or unmatched creation cancellation cannot unlock the form while the backend is still working', async () => {
  for (const count of [null, 0]) {
    let finish!: (value: unknown) => void;
    const p = page('NewCasePage', { form, sites: [site], step: 5 }, [site], initialValues, {
      create: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
      cancelCreation: async () => {
        if (count === null) throw new Error('cancel failed');
        return count;
      },
    });
    await p.mount();
    const pending = p.click('new-create');
    p.render();
    await assert.rejects(
      p.click('new-cancel-creation'),
      count === null ? /cancel failed/ : /未取消/,
    );
    const c = p.render();
    assert.equal(c.status, 'busy');
    assert.equal(c.actions.find((a) => a.id === 'new-create')?.enabled, false);
    assert.equal(c.actions.find((a) => a.id === 'new-cancel-creation')?.enabled, true);
    assert.ok(p.elements.some((e) => e.type === 'Notice' && e.props.tone === 'error'));
    finish({});
    await pending;
    assert.deepEqual(p.opened, ['/project/CN-Cng/CN-Cng']);
  }
});

test('site scanning never offers the creation cancellation action', async () => {
  let finish!: (value: unknown) => void;
  const p = page(
    'NewCasePage',
    { form: { ...form, sitedir: '/sites' }, sites: [site], step: 3, ownData: true },
    [site],
    initialValues,
    {
      scan: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    },
  );
  await p.mount();
  const pending = p.click('new-scan');
  const c = p.render();
  assert.equal(c.status, 'reading');
  assert.ok(!c.actions.some((a) => a.id === 'new-cancel-creation'));
  assert.ok(!p.elements.some((e) => e.type === 'Button' && e.children.includes('取消创建')));
  finish({ sites: [site] });
  await pending;
  assert.ok(!p.calls.some((call) => call.command === 'case_creation_cancel'));
});
test('reading sites leaves the page usable and can be stopped without applying a late answer', async () => {
  let finish!: (value: unknown) => void;
  const other = { ...site, name: 'US-Ha1', site_file: '/US-Ha1.nc' };
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'compute', sitedir: '/server/sites' },
      sites: [site],
      step: 3,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [runServer],
      scan: () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    },
  );
  await p.mount();
  assert.ok(p.render().actions.some((a) => a.id === `new-select-${site.name}`));
  void p.click('new-scan').catch(() => {});
  let c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-scan')?.enabled, false);
  assert.ok(c.actions.some((a) => a.id === 'new-scan-stop'));
  assert.notEqual(c.actions.find((a) => a.id === 'new-pick-sitedir')?.enabled, false);
  await p.click('new-scan-stop');
  c = p.render();
  assert.ok(!c.actions.some((a) => a.id === 'new-scan-stop'));
  assert.notEqual(c.actions.find((a) => a.id === 'new-scan')?.enabled, false);
  finish({ sites: [other] });
  await p.settle();
  c = p.render();
  assert.ok(!c.actions.some((a) => a.id === 'new-select-US-Ha1'), 'a stopped read is ignored');
  assert.ok(c.actions.some((a) => a.id === `new-select-${site.name}`));
});

test('spatial creation cancellation targets its own output directory', async () => {
  let finish!: (error: Error) => void;
  const p = page(
    'NewCasePage',
    {
      form: {
        ...form,
        name: 'spatial',
        kind: 'global',
        dlon: '1',
        dlat: '1',
        forcing: '/forcing.nml',
      },
      step: 5,
    },
    [site],
    initialValues,
    {
      create: () =>
        new Promise((_resolve, reject) => {
          finish = reject;
        }),
      cancelCreation: async () => 1,
    },
  );
  await p.mount();
  const pending = p.click('new-create');
  const rejected = assert.rejects(pending, /cancelled/);
  p.render();
  await p.click('new-cancel-creation');
  assert.deepEqual(p.calls.find((call) => call.command === 'case_creation_cancel')?.args, {
    dir: '/project/spatial',
  });
  assert.ok(p.calls.some((call) => call.command === 'new_spatial_case'));
  assert.equal(p.render().status, 'busy');
  finish(new Error('cancelled'));
  await rejected;
  assert.deepEqual(p.opened, []);
});

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} renders optional initial-state file placeholders as editable paths in the page and guide`, async () => {
    const key = 'DEF_file_SoilInit',
      path = '/Volumes/Data/CoLMruntime/initial/soil state.nc';
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      {
        schema: [
          ...schema,
          {
            ...field(key, '初始场', 'null'),
            kind: 'Character { len: 256 }',
            values: ['', 'NONE', 'NULL'],
          },
        ],
      },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const id = `${name === 'NewCasePage' ? 'new-option-' : 'case-'}${key}`;
    assert.equal(p.render().fields.find((f) => f.id === id)?.type, 'text');
    await guideFor(p).fill(id, path);
    assert.equal(p.render().fields.find((f) => f.id === id)?.value, path);
    assert.ok(p.elements.some((e) => e.type === 'Input' && e.props.value === path));
    assert.ok(!p.elements.some((e) => e.type === 'Select' && e.props.value === path));
  });

test('crop and nitrogen history fields stay in Step 4 output selection with their existing kernel gates', async () => {
  const names = ['cropprod1c', 'fertnitro_corn', 'ndep_to_sminn'];
  const histories = names.map((name) => ({
    name,
    on: true,
    writable: name !== 'fertnitro_corn',
    settable: name !== 'fertnitro_corn',
  }));
  const p = page('CasesPage', undefined, [site], initialValues, {
    kernels: setupKernels,
    histories,
    schema: [
      ...schema,
      ...names.map((name) => ({
        ...field(`DEF_hist_vars%${name}`, '输出变量', '.true.'),
        kind: 'Logical',
        group: 'nl_colm_history',
      })),
    ],
  });
  await p.mount();
  await p.goto('case-step-5');
  p.set('case-advanced', 'true');
  const c = p.render();
  for (const name of names) {
    const output = c.fields.find((f) => f.id === `case-DEF_hist_vars%${name}`);
    assert.ok(output, name);
    assert.equal(output.value, '.true.');
    assert.equal(output.disabled, name === 'fertnitro_corn');
  }
  assert.ok(p.calls.some((call) => call.command === 'hist_vars'));
});

test('switching the new case to a land-cover class closes BGC and says why', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, subgrid: 'PFT' },
      sites: [site],
      caseValues: { DEF_USE_BGC: '.true.' },
      step: 4,
    },
    [site],
    initialValues,
    { schema: setupSchema },
  );
  await p.mount();
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('IGBP');
  p.render();
  assert.equal(
    (p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> })
      .caseValues.DEF_USE_BGC,
    '.false.',
  );
  assert.ok(
    p.elements.some(
      (e) => e.type === 'Notice' && String(e.children).includes('生物地球化学（BGC）'),
    ),
  );
});

test('the auto-close notice stays with the step that caused it', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, subgrid: 'PFT' },
      sites: [site],
      caseValues: { DEF_USE_BGC: '.true.' },
      step: 4,
    },
    [site],
    initialValues,
    { schema: setupSchema },
  );
  await p.mount();
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('IGBP');
  p.render();
  const notice = () =>
    p.elements.some((e) => e.type === 'Notice' && String(e.children).includes('已自动关闭'));
  assert.ok(notice());
  await p.click('new-previous');
  assert.equal(p.render().details.step, 3);
  assert.ok(!notice(), 'the notice does not follow the user to another step');
});

test('NewCasePage unifies project folder and case name in step 1 keeping outputs together', async () => {
  const p = page('NewCasePage', { form: { ...form, name: 'CN-Cng' } });
  await p.mount();
  assert.ok(p.render().fields.some((f) => f.id === 'new-root'));
  assert.ok(p.render().fields.some((f) => f.id === 'new-name'));
  assert.ok(
    !p.render().fields.some((f) => f.id === 'new-output'),
    'separate output path is merged into case directory',
  );
});

test('next button is strictly disabled when required settings are missing', async () => {
  const p = page('NewCasePage');
  await p.mount();
  const findNextButton = () =>
    p.elements.find(
      (e) =>
        e.type === 'Button' &&
        e.props.variant === 'primary' &&
        (e.children as unknown[]).some((c) => typeof c === 'string' && c.includes('下一步')),
    );
  assert.equal(findNextButton()?.props.disabled, true);
  p.set('new-name', 'CN-Cng');
  // 计算引擎不预选：选定之前“下一步”仍置灰。
  assert.equal(findNextButton()?.props.disabled, true);
  p.set('new-engine', 'rust');
  assert.equal(findNextButton()?.props.disabled, false);
  p.set('new-root', '');
  assert.equal(findNextButton()?.props.disabled, true);
  p.set('new-root', '/project');
  assert.equal(findNextButton()?.props.disabled, false);
  p.set('new-engine', '');
  assert.equal(findNextButton()?.props.disabled, true);
  p.set('new-engine', 'rust');
  assert.equal(findNextButton()?.props.disabled, false);
});

test('choosing an ordinary station never preselects a land-cover scheme', async () => {
  const p = page('NewCasePage', {
    form: { ...form, subgrid: '', landcover: '' },
    sites: [site],
    step: 3,
  });
  await p.mount();
  await p.click('new-select-CN-Cng');
  const c = await p.click('new-next');
  assert.equal(c.details.step, 4);
  assert.equal(p.element('ModelSettings').props.mode, undefined);
  blocked(p, 'new-next', /方案/);
});

test('output variables chosen while creating a case are written with it', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 5 });
  await p.mount();
  (p.element('HistoryVariablesCard').props.onChange as (patch: object) => void)({
    'DEF_hist_vars%fsena': '.true.',
    'DEF_hist_vars%leafc': '.false.',
  });
  p.render();
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case');
  assert.ok(request);
  const fields = request.args.fields as { path: string; value: string }[];
  assert.equal(fields.find((f) => f.path === 'DEF_hist_vars%fsena')?.value, '.true.');
  assert.equal(fields.find((f) => f.path === 'DEF_hist_vars%leafc')?.value, '.false.');
});

for (const kind of ['region', 'watershed', 'global'])
  test(`${kind} creation writes automatic routing after any stale override`, async () => {
    const p = page(
      'NewCasePage',
      {
        form: {
          ...form,
          kind,
          dlon: '1',
          dlat: '1',
          west: '100',
          east: '102',
          south: '30',
          north: '32',
          shapefile: '/basin.shp',
          forcing: '/forcing.nml',
        },
        step: 5,
        caseValues: {
          DEF_USE_GridRiverLakeFlow: '.true.',
          DEF_USE_LULCC: '.false.',
          DEF_UnitCatchment_regional: kind === 'global' ? '.true.' : '.false.',
        },
      },
      [site],
      initialValues,
      {
        schema: [
          ...schema,
          ...['DEF_USE_GridRiverLakeFlow', 'DEF_USE_LULCC', 'DEF_UnitCatchment_regional'].map(
            (name) => ({ ...field(name, '河道与水库', '.false.'), kind: 'Logical' }),
          ),
        ],
      },
    );
    await p.mount();
    await p.click('new-create');
    const request = p.calls.find((call) => call.command === 'new_spatial_case');
    assert.ok(request);
    const fields = request.args.fields as { path: string; value: string }[];
    assert.equal(
      fields.filter((f) => f.path === 'DEF_UnitCatchment_regional').at(-1)?.value,
      kind === 'global' ? '.false.' : '.true.',
    );
  });

for (const kind of ['region', 'watershed'])
  test(`${kind} latlon creation turns on regional routing when river-lake routing is left at its default`, async () => {
    const p = page(
      'NewCasePage',
      {
        form: {
          ...form,
          kind,
          dlon: '1',
          dlat: '1',
          west: '100',
          east: '102',
          south: '30',
          north: '32',
          shapefile: '/basin.shp',
          forcing: '/forcing.nml',
        },
        step: 5,
        caseValues: {},
      },
      [site],
      initialValues,
      {
        schema: [
          ...schema,
          { ...field('DEF_USE_GridRiverLakeFlow', '河道与水库', '.true.'), kind: 'Logical' },
          ...['DEF_USE_LULCC', 'DEF_UnitCatchment_regional'].map((name) => ({
            ...field(name, '河道与水库', '.false.'),
            kind: 'Logical',
          })),
        ],
      },
    );
    await p.mount();
    await p.click('new-create');
    const request = p.calls.find((call) => call.command === 'new_spatial_case');
    assert.ok(request);
    const fields = request.args.fields as { path: string; value: string }[];
    assert.equal(
      fields.some((f) => f.path === 'DEF_USE_GridRiverLakeFlow'),
      false,
    );
    assert.equal(
      fields.filter((f) => f.path === 'DEF_UnitCatchment_regional').at(-1)?.value,
      '.true.',
    );
  });

for (const kind of ['region', 'watershed', 'global'])
  test(`switching site crop to ${kind} keeps crop enabled and creates with the per-CFT calendar`, async () => {
    const p = page(
      'NewCasePage',
      {
        form: {
          ...form,
          subgrid: 'PFT',
          dlon: '1',
          dlat: '1',
          west: '100',
          east: '102',
          south: '30',
          north: '32',
          shapefile: '/basin.shp',
          forcing: '/forcing.nml',
        },
        step: 2,
        caseValues: { DEF_USE_BGC: '.true.', DEF_TUNING_CROP_PLANTING_DAY: '145' },
      },
      [site],
      initialValues,
      { schema: setupSchema },
    );
    await p.mount();
    p.set('new-kind', kind);
    await p.settle();
    await p.goto('new-step-4');
    const values = (
      p.element('ModelSettings').props.setupEnvironment as {
        caseValues: Record<string, string>;
      }
    ).caseValues;
    assert.equal(values.DEF_TUNING_CROP_PLANTING_DAY, '0');
    assert.equal(caseModel.cropEnabled(values), true);
    assert.equal(values.DEF_USE_BGC, '.true.');
    await p.goto('new-step-5');
    await p.click('new-create');
    const request = p.calls.find((call) => call.command === 'new_spatial_case');
    assert.ok(request);
    const sent = request.args.fields as { path: string; value: string }[];
    assert.equal(sent.filter((f) => f.path === 'DEF_TUNING_CROP_PLANTING_DAY').at(-1)?.value, '0');
  });

for (const name of ['NewCasePage', 'CasesPage'] as const) {
  test(`${name} keeps single-point settings in data extras and model schemes separate while preserving values`, async () => {
    const modelKeys = [
      'USE_SITE_soilparameters',
      'USE_SITE_LAI',
      'SITE_lakedepth',
      'DEF_LAI_CHANGE_YEAR',
      'DEF_LAI_MONTHLY',
      'DEF_HIGHRESURBAN_ALBEDO',
      'DEF_forcing_interp_method',
      'DEF_USE_Forcing_Downscaling',
    ];
    const retained = Object.fromEntries(modelKeys.map((key, index) => [key, `kept-${index}`]));
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3, caseValues: retained } : undefined,
      [site],
      { ...initialValues, ...retained, SITE_fsitedata: '/site.nc' },
      {
        schema: [
          ...schema,
          ...modelKeys.map((key) => field(key, '地表数据', '')),
          field('SITE_fsitedata', '单点模拟', '/site.nc'),
          field('DEF_case_name', '基础配置', 'site'),
        ],
      },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const contract = p.render();
    for (const key of modelKeys.filter(
      (key) => simulation.isSiteConfiguration(key) || simulation.isForcingScheme(key),
    )) {
      assert.ok(
        contract.fields.some((f) => f.id.endsWith(key)),
        key,
      );
      assert.ok(
        p.elements.some((e) => e.type === 'Field' && e.props.key === key),
        key,
      );
    }
    for (const key of [
      ...modelKeys.filter(
        (key) => !simulation.isSiteConfiguration(key) && !simulation.isForcingScheme(key),
      ),
      'DEF_case_name',
    ]) {
      assert.ok(!contract.fields.some((f) => f.id.endsWith(key)), key);
      assert.ok(!p.elements.some((e) => e.type === 'Field' && e.props.key === key), key);
    }
    assert.ok(contract.fields.some((f) => f.id.endsWith('DEF_dir_existing_srfdata')));
    assert.ok(
      contract.fields.some(
        (f) => f.id === (name === 'NewCasePage' ? 'new-site' : 'case-SITE_fsitedata'),
      ),
    );
    assert.ok(
      contract.fields.some(
        (f) => f.id === (name === 'NewCasePage' ? 'new-start' : 'case-start-date'),
      ),
    );
    await p.goto(name === 'NewCasePage' ? 'new-step-4' : 'case-step-4');
    const environment = p.element('ModelSettings').props.setupEnvironment as {
      caseValues: Record<string, string>;
    };
    for (const key of modelKeys) assert.equal(environment.caseValues[key], retained[key], key);
    assert.ok(!p.calls.some((call) => call.command === 'case_data_save'));
  });

  test(`${name} omits generic applicability hints while retaining concrete restrictions`, async () => {
    const generic = '适用条件待按所选源码、算例和内核核对；目录默认值仅供参考';
    const reason = 'This input requires an existing surface data directory';
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      {
        schema: [...schema, field('EXTRA_INPUT', '地表数据', '')],
        configuration: async () => ({
          fields: [
            { name: 'DEF_dir_existing_srfdata', mode: 'editable', pending: true, reason: generic },
            { name: 'EXTRA_INPUT', mode: 'readonly', pending: true, reason },
          ],
        }),
      },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    p.render();
    assert.ok(!p.elements.some((e) => e.props.hint === generic));
    assert.ok(p.elements.some((e) => e.props.hint === reason));
  });
}

test('existing single-point range renders coordinates once and leaves the site file in data', async () => {
  const p = page(
    'CasesPage',
    undefined,
    [site],
    {
      ...initialValues,
      SITE_fsitedata: '/site.nc',
      SITE_lon_location: '116.3',
      SITE_lat_location: '40.1',
    },
    {
      schema: [
        ...schema,
        ...['SITE_fsitedata', 'SITE_lon_location', 'SITE_lat_location'].map((key) =>
          field(key, '单点模拟', ''),
        ),
      ],
    },
  );
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-advanced', 'true');
  await p.goto('case-step-2');
  let contract = p.render();
  assert.ok(!contract.fields.some((f) => f.id === 'case-SITE_fsitedata'));
  for (const key of ['SITE_lon_location', 'SITE_lat_location']) {
    assert.equal(contract.fields.filter((f) => f.id === `case-${key}`).length, 1);
    assert.equal(p.elements.filter((e) => e.type === 'Field' && e.props.key === key).length, 1);
  }
  await p.goto('case-step-3');
  contract = p.render();
  assert.equal(contract.fields.filter((f) => f.id === 'case-SITE_fsitedata').length, 1);
  assert.equal(
    p.elements.filter((e) => e.type === 'Field' && e.props.key === 'SITE_fsitedata').length,
    1,
  );
  assert.ok(
    !contract.fields.some((f) =>
      ['case-SITE_lon_location', 'case-SITE_lat_location'].includes(f.id),
    ),
  );
});

test('switching bundled examples to own data clears example bindings but preserves manual values', async () => {
  const p = page('NewCasePage', { multiSiteEnabled: false, form, step: 3 });
  await p.mount();
  await p.click('new-example');
  p.set('new-runtime', '/custom/runtime');
  p.set('new-start', '2001-03-01');
  const contract = await p.click('new-own-data');
  for (const key of ['sitedir', 'forcingdir', 'site', 'met', 'end']) {
    assert.equal(contract.fields.find((f) => f.id === `new-${key}`)?.value, '', key);
  }
  assert.equal(contract.fields.find((f) => f.id === 'new-runtime')?.value, '/custom/runtime');
  assert.equal(contract.fields.find((f) => f.id === 'new-start')?.value, '2001-03-01');
  assert.ok(!contract.actions.some((a) => a.id.startsWith('new-select-')));
  assert.ok(!p.calls.some((c) => /delete|remove/.test(c.command)));
  p.set('new-sitedir', '/custom/sites');
  await p.click('new-scan');
  await p.click('new-select-CN-Cng');
  const repeated = await p.click('new-own-data');
  assert.equal(repeated.fields.find((f) => f.id === 'new-sitedir')?.value, '/custom/sites');
  assert.equal(repeated.fields.find((f) => f.id === 'new-site')?.value, site.site_file);
  assert.ok(repeated.actions.some((a) => a.id === 'new-select-CN-Cng'));
  const bundled = await p.click('new-example');
  assert.ok(!bundled.fields.some((f) => f.id === 'new-sitedir'));
  const own = await p.click('new-own-data');
  assert.equal(own.fields.find((f) => f.id === 'new-runtime')?.value, '');
});

test('research round trips preserve manual overrides while clearing remaining bundled bindings', async () => {
  const p = page('NewCasePage', { multiSiteEnabled: false, form, step: 3 });
  await p.mount();
  await p.click('new-example');
  p.set('new-runtime', '/custom/runtime');
  p.set('new-start', '2001-03-01');
  await p.click('new-next');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  await p.remount();
  await p.click('new-previous');
  const own = await p.click('new-own-data');
  assert.equal(own.fields.find((f) => f.id === 'new-runtime')?.value, '/custom/runtime');
  assert.equal(own.fields.find((f) => f.id === 'new-start')?.value, '2001-03-01');
  assert.equal(own.fields.find((f) => f.id === 'new-site')?.value, '');
  assert.equal(own.fields.find((f) => f.id === 'new-met')?.value, '');
});

for (const name of ['NewCasePage', 'CasesPage'] as const) {
  test(`${name} edits single-point data defaults and saves only the selected override`, async () => {
    const key = 'USE_SITE_soilparameters';
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      { schema: [...schema, { ...field(key, '站点', '.true.'), kind: 'Logical' }] },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    assert.ok(
      !p.render().fields.some((f) => f.id.endsWith(key)),
      'hidden until More settings opens',
    );
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const id = p.render().fields.find((f) => f.id.endsWith(key))!.id;
    assert.equal(p.render().fields.find((f) => f.id === id)?.value, '.true.');
    p.set(id, '.false.');
    if (name === 'NewCasePage') {
      p.set(id, '');
      assert.equal(
        p.render().fields.find((f) => f.id === id)?.value,
        '.true.',
        'inherit restores the displayed default',
      );
      p.set(id, '.false.');
      await p.goto('new-step-5');
      await p.click('new-create');
    } else await p.click('case-save');
    const request = p.calls.find(
      (call) => call.command === (name === 'NewCasePage' ? 'new_case' : 'case_data_save'),
    )!;
    assert.ok(request);
    assert.ok(
      (request.args.fields as { path: string; value: string }[]).some(
        (f) => f.path === key && f.value === '.false.',
      ),
    );
  });
  test(`${name} hides single-point settings from spatial data extras`, async () => {
    const key = 'USE_SITE_soilparameters';
    const p = page(
      name,
      name === 'NewCasePage' ? { form: { ...form, kind: 'region' }, step: 3 } : undefined,
      [site],
      initialValues,
      {
        profile: { grid: 'latlon', domain_kind: 'region' },
        schema: [...schema, field(key, '站点', '.true.')],
      },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    assert.ok(!p.render().fields.some((f) => f.id.endsWith(key)));
    assert.ok(!p.elements.some((e) => e.type === 'Field' && e.props.key === key));
  });
}

test('final case directory preview follows the exact validated creation destination', async () => {
  const p = page('NewCasePage', { form, step: 1 });
  await p.mount();
  p.set('new-root', '/projects///');
  p.set('new-name', 'Station-A');
  let c = p.render();
  const preview = () => c.fields.find((f) => f.id === 'new-case-directory')!;
  assert.equal(preview().value, '/projects/Station-A');
  assert.equal(preview().disabled, true);
  p.set('new-name', 'Station-B');
  c = p.render();
  assert.equal(preview().value, '/projects/Station-B');
  for (const name of ['', ' .. ', '../escape', ' name ']) {
    p.set('new-name', name);
    c = p.render();
    assert.equal(preview().value, '');
  }
});

for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} edits forcing processing in data extras and saves its override`, async () => {
    const key = 'DEF_USE_Forcing_Downscaling';
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      { schema: [...schema, { ...field(key, '强迫场', '.false.'), kind: 'Logical' }] },
    );
    await p.mount();
    if (name === 'CasesPage') await p.goto('case-step-3');
    assert.ok(!p.render().fields.some((f) => f.id.endsWith(key)));
    p.set(name === 'NewCasePage' ? 'new-advanced' : 'case-advanced', 'true');
    const control = p.render().fields.find((f) => f.id.endsWith(key))!;
    assert.equal(control.value, '.false.');
    p.set(control.id, '.true.');
    if (name === 'NewCasePage') {
      await p.goto('new-step-5');
      await p.click('new-create');
    } else await p.click('case-save');
    const request = p.calls.find(
      (call) => call.command === (name === 'NewCasePage' ? 'new_case' : 'case_data_save'),
    )!;
    assert.ok(request);
    assert.ok(
      (request.args.fields as { path: string; value: string }[]).some(
        (f) => f.path === key && f.value === '.true.',
      ),
    );
  });

const multiSites = [
  { ...site, end: '2004-12-31' },
  {
    ...site,
    name: 'other',
    site_file: '/other-site.nc',
    met_file: '/other-met.nc',
    start: '2005-03-01',
    end: '2008-12-31',
  },
];
const multiSeed = {
  form: { ...form, name: 'multi-project' },
  sites: multiSites,
  step: 5,
  ownData: true,
  multiSiteEnabled: true,
  multiSiteMode: 'first-year',
  multiSiteRepeat: '3',
  siteDrafts: multiSites.map(multiSiteModel.siteDraft),
};

test('custom multi-site creation uses each station inputs and independently calculated spinup', async () => {
  const p = page('NewCasePage', multiSeed, multiSites);
  await p.mount();
  await p.click('new-create');
  const created = p.calls.filter((call) => call.command === 'new_case');
  assert.equal(created.length, 2);
  assert.deepEqual(
    created.map((call) => [call.args.site, call.args.met, call.args.out, call.args.start]),
    [
      ['/site.nc', '/forcing.nc', '/project/multi-project/CN-Cng', '2001-01-01'],
      ['/other-site.nc', '/other-met.nc', '/project/multi-project/other', '2005-03-01'],
    ],
  );
  const cutoffs = created.map((call) =>
    Object.fromEntries(
      (call.args.fields as { path: string; value: string }[]).map((f) => [f.path, f.value]),
    ),
  );
  assert.equal(cutoffs[0]['DEF_simulation_time%spinup_year'], '2002');
  assert.equal(cutoffs[1]['DEF_simulation_time%spinup_year'], '2006');
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'set_spinup').map((call) => call.args),
    created.map((call) => ({
      dirs: [call.args.out],
      years: 1,
      repeat: 3,
      replayStart: true,
      kernelDir: null,
    })),
  );
  assert.equal(p.calls.filter((call) => call.command === 'run_case').length, 0);
  assert.equal(p.render().actions.find((action) => action.id === 'new-create')?.enabled, false);
  assert.deepEqual(p.opened, []);
});

test('per-site creation saves disabled and custom spinup separately without the common setter', async () => {
  const drafts = multiSites.map(multiSiteModel.siteDraft);
  drafts[1].spinup_end = '2007-03-01';
  drafts[1].spinup_repeat = '5';
  const p = page(
    'NewCasePage',
    { ...multiSeed, multiSiteMode: 'per-site', siteDrafts: drafts },
    multiSites,
  );
  await p.mount();
  await p.click('new-create');
  const created = p.calls.filter((call) => call.command === 'new_case');
  const fields = created.map((call) =>
    Object.fromEntries(
      (call.args.fields as { path: string; value: string }[]).map((f) => [f.path, f.value]),
    ),
  );
  assert.equal(fields[0]['DEF_simulation_time%spinup_year'], '0');
  assert.equal(fields[1]['DEF_simulation_time%spinup_year'], '2007');
  assert.equal(fields[1]['DEF_simulation_time%spinup_repeat'], '5');
  assert.ok(!p.calls.some((call) => call.command === 'set_spinup'));
});

test('a failed destination does not overwrite or block another station and retry skips successes', async () => {
  let failed = true;
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    create: async (args) => {
      if (failed && args.name === 'CN-Cng') throw new Error('destination already exists');
    },
  });
  await p.mount();
  await p.click('new-create');
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 2);
  assert.deepEqual(p.routes, []);
  failed = false;
  await p.click('new-multi-retry');
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'new_case').map((call) => call.args.name),
    ['CN-Cng', 'other', 'CN-Cng'],
  );
  assert.deepEqual(p.routes, ['project']);
});

test('a spinup write failure retries its settings without recreating the completed case', async () => {
  let failed = true;
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    spinup: async () => {
      if (failed) throw new Error('write failed');
    },
  });
  await p.mount();
  await p.click('new-create');
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 2);
  failed = false;
  await p.click('new-multi-retry');
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 2);
  assert.equal(p.calls.filter((call) => call.command === 'set_spinup').length, 4);
});

test('cancelling multi-site creation prevents the remaining queue from starting', async () => {
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    create: () => gate,
    cancelCreation: async () => 0,
  });
  await p.mount();
  const pending = p.click('new-create');
  p.render();
  assert.match(JSON.stringify(p.elements), /正在创建站点（已处理 0\/2）/);
  await p.click('new-cancel-creation');
  finish();
  await pending;
  assert.deepEqual(p.routes, []);
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 1);
  assert.equal(p.render().actions.find((action) => action.id === 'new-multi-retry')?.enabled, true);
  await p.settle();
  assert.deepEqual(
    p.scrolls.at(-1),
    { top: 0 },
    'cancelled results must be visible above the long review',
  );
});

test('invalid multi-site coverage is rejected before any case is created', async () => {
  const drafts = multiSites.map(multiSiteModel.siteDraft);
  drafts[1].end = '2005-12-31';
  const p = page('NewCasePage', { ...multiSeed, siteDrafts: drafts }, multiSites);
  await p.mount();
  assert.equal(p.render().actions.find((action) => action.id === 'new-create')?.enabled, false);
  assert.ok(!p.calls.some((call) => call.command === 'new_case'));
});

test('editing failed stations preserves completed cases and retries only the remaining selection', async () => {
  let failed = true;
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    create: async (args) => {
      if (failed && args.name === 'CN-Cng') throw new Error('runtime incomplete');
    },
  });
  await p.mount();
  await p.click('new-create');
  await p.click('new-multi-edit-unfinished');
  const controls = p.element('MultiSiteSettings');
  assert.deepEqual(
    (controls.props.drafts as multiSiteModel.MultiSiteDraft[]).map((draft) => draft.name),
    ['CN-Cng'],
  );
  assert.equal(controls.props.disabled, false);
  p.set('new-runtime', '/complete-runtime');
  failed = false;
  await p.goto('new-step-5');
  await p.settle();
  await p.click('new-create');
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'new_case').map((call) => call.args.name),
    ['CN-Cng', 'other', 'CN-Cng'],
  );
  assert.equal(
    p.calls.filter((call) => call.command === 'new_case').at(-1)?.args.runtime,
    '/complete-runtime',
  );
});

test('a cancellation IPC failure still stops the remaining multi-site queue', async () => {
  let finish!: () => void;
  const gate = new Promise<void>((resolve) => {
    finish = resolve;
  });
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    create: () => gate,
    cancelCreation: async () => {
      throw new Error('cancel connection failed');
    },
  });
  await p.mount();
  const pending = p.click('new-create');
  p.render();
  await assert.rejects(p.click('new-cancel-creation'), /cancel connection failed/);
  finish();
  await pending;
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 1);
});

const runServer = {
  host: 'compute',
  root: '/remote/jobs',
  maps: [{ local: '/surface', remote: '/data/surface' }],
};

test('General setup selects run location before paths and preserves it while configuring resources', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 1 }, [site], initialValues, {
    servers: [runServer],
  });
  await p.mount();
  const c = p.set('new-run-host', 'compute');
  assert.equal(c.fields.find((f) => f.id === 'new-root')?.label, '服务器项目目录');
  assert.equal(p.element('RunLocationSettings').props.host, 'compute');
  await p.click('new-run-resources');
  assert.equal(p.routes.at(-1), 'resources');
  const seed = p.state.newCaseSeed as { form: Record<string, string>; step: number };
  assert.equal(seed.form.host, 'compute');
  assert.equal(seed.form.root, runServer.root);
  assert.equal(seed.form.site, '');
  assert.equal(seed.form.runtime, '');
  assert.equal(seed.step, 1);
});

test('selected resource is persisted during case creation instead of reverting to local', async () => {
  const p = page(
    'NewCasePage',
    { multiSiteEnabled: false, form: { ...form, host: 'compute' }, sites: [site], step: 5 },
    [site],
    initialValues,
    { servers: [runServer] },
  );
  await p.mount();
  await p.click('new-create');
  const call = p.calls.find((call) => call.command === 'remote_create_case');
  assert.equal(call?.args.host, 'compute');
  assert.equal((call?.args.request as Record<string, unknown>).kind, 'site');
  assert.equal((call?.args.request as Record<string, unknown>).out, `${form.root}/${form.name}`);
  assert.deepEqual(p.opened, ['/cache/project/site']);
  assert.equal(p.state.root, '/cache/project');
  assert.ok(!p.calls.some((call) => call.command === 'new_case'));
});

test('a removed run location blocks new creation instead of silently selecting local', async () => {
  const p = page('NewCasePage', { form: { ...form, host: 'missing' }, sites: [site], step: 1 });
  const c = await p.mount();
  assert.equal(c.actions.find((action) => action.id === 'new-next')?.enabled, false);
  assert.equal(c.fields.find((field) => field.id === 'new-run-host')?.value, 'missing');
});

test('existing General setup saves the target and synchronizes the hidden run panel profile', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, {
    profile: { run_host: 'compute' },
    servers: [runServer],
  });
  await p.mount();
  await p.goto('case-step-1');
  assert.equal(p.render().fields.find((field) => field.id === 'case-run-host')?.value, 'compute');
  p.set('case-run-host', '');
  await p.settle();
  assert.deepEqual(p.calls.find((call) => call.command === 'set_case_run_host')?.args, {
    dir: '/project/site',
    host: null,
  });
  assert.equal(p.state.selectedCase.profile.run_host, null);
  await p.click('case-run-resources');
  assert.equal(p.routes.at(-1), 'resources');
  assert.deepEqual(p.state.simulationReturn, { route: 'case', step: 1, resourceHost: '' });
});

test('server-native existing cases keep their run location fixed', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, {
    profile: { run_host: 'compute', remote_directory: '/server/project/site' },
    servers: [runServer],
  });
  await p.mount();
  await p.goto('case-step-1');
  const host = p.render().fields.find((field) => field.id === 'case-run-host')!;
  assert.equal(host.disabled, true);
  await host.set('');
  assert.equal(
    p.calls.some((call) => call.command === 'set_case_run_host'),
    false,
  );
  assert.equal(p.state.selectedCase.profile.run_host, 'compute');
});

test('server input pickers return selected paths and scan only the chosen server', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, host: 'compute' }, ownData: true, sites: [site], step: 3 },
    [site],
    initialValues,
    { servers: [runServer] },
  );
  await p.mount();
  assert.ok(!p.render().actions.some((action) => action.id === 'new-example'));
  const pick = p.click('new-pick-sitedir');
  p.render();
  const browser = p.element('RemotePathBrowser');
  assert.equal(browser.props.host, 'compute');
  assert.equal(browser.props.directory, true);
  (browser.props.onSelect as (path: string) => void)('/server/sites');
  await pick;
  p.render();
  assert.equal(
    p.render().fields.find((field) => field.id === 'new-sitedir')?.value,
    '/server/sites',
  );
  await p.click('new-scan');
  assert.equal(
    p.calls.find((call) => call.command === 'remote_scan_sites')?.args.dir,
    '/server/sites',
  );
  assert.ok(
    !p.calls.some((call) => call.command === 'scan_sites' || call.command === 'pick_folder'),
  );
});

const notices = (p: { elements: Element[] }) =>
  p.elements
    .filter((element) => element.type === 'Notice')
    .map((element) => (element.children as unknown[]).flat(Infinity).join(''));

test('choosing a server prepares its engine in the background and says when it is compiling', async (t) => {
  t.mock.timers.enable({ apis: ['setTimeout', 'setInterval', 'Date'] });
  let ready: (value: unknown) => void = () => {};
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'compute', sitedir: '/server/sites' },
      ownData: true,
      sites: [site],
      step: 3,
    },
    [site],
    initialValues,
    {
      servers: [runServer],
      prepare: () => new Promise((resolve) => (ready = resolve)),
      scan: () => new Promise(() => {}),
      progress: () => ({
        engine: 'abc',
        stage: 'building the Rust engine',
        done: 312,
        total: 664,
        current: 'colm-core',
      }),
    },
  );
  await p.mount();
  const prepare = p.calls.filter((call) => call.command === 'remote_prepare_engine');
  assert.equal(prepare.length, 1);
  assert.equal(prepare[0].args.host, 'compute');
  assert.deepEqual(prepare[0].args.modelRevision, p.state.versionRef);
  const card = () => p.element('EngineProgress').props as Record<string, unknown>;
  assert.deepEqual([card().host, card().phase], ['compute', 'checking']);
  assert.ok(
    !p.calls.some((call) => call.command === 'remote_engine_progress'),
    'not while checking',
  );

  t.mock.timers.tick(5000);
  await p.settle();
  assert.deepEqual([card().phase, card().waited], ['building', '5 秒']);
  // Building: the server's progress is read and drawn, then again every five seconds.
  assert.deepEqual(
    p.calls.filter((call) => call.command === 'remote_engine_progress').at(-1)?.args,
    {
      host: 'compute',
    },
  );
  assert.equal((card().progress as { done: number }).done, 312);
  const polls = p.calls.filter((call) => call.command === 'remote_engine_progress').length;
  t.mock.timers.tick(125_000);
  await p.settle();
  assert.equal(card().waited, '2 分 10 秒');
  assert.ok(p.calls.filter((call) => call.command === 'remote_engine_progress').length > polls);

  void p.click('new-scan');
  p.render();
  const reading = () => p.element('TaskCard').props as Record<string, unknown>;
  assert.equal(reading().title, '正在从 compute 读取站点');
  assert.deepEqual([reading().step, reading().waited], [0, '已用 0 秒']);
  assert.match(String(reading().detail), /先等 compute 上的计算引擎编译完成/);
  t.mock.timers.tick(3000);
  p.render();
  assert.equal(reading().waited, '已用 3 秒');
  assert.ok(!notices(p).some((text) => text.includes('正在处理，请稍候')));

  ready({ ready: true });
  await p.settle();
  assert.ok(!p.elements.some((element) => element.type === 'EngineProgress'));
  assert.equal(reading().step, 1);
  assert.match(String(reading().detail), /正在从 compute 读取站点与强迫场覆盖时段/);
  await p.remount();
  assert.equal(p.calls.filter((call) => call.command === 'remote_prepare_engine').length, 1);
});

test('a failed engine preparation is shown and reading sites names the server', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'compute', sitedir: '/server/sites' },
      ownData: true,
      sites: [site],
      step: 3,
    },
    [site],
    initialValues,
    {
      servers: [runServer],
      prepare: async () => {
        throw new Error('cargo is missing');
      },
      scan: () => new Promise(() => {}),
    },
  );
  await p.mount();
  assert.ok(
    notices(p).some(
      (text) => text.includes('准备 compute 上的计算引擎失败') && text.includes('cargo is missing'),
    ),
  );
  void p.click('new-scan');
  p.render();
  const reading = p.element('TaskCard').props as Record<string, unknown>;
  assert.equal(reading.step, 1, 'a failed preparation is not waited for');
  assert.match(String(reading.detail), /正在从 compute 读取站点与强迫场覆盖时段/);
});

test('local reading of sites says what it is doing', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, sitedir: '/sites' }, ownData: true, sites: [site], step: 3 },
    [site],
    initialValues,
    { scan: () => new Promise(() => {}) },
  );
  await p.mount();
  assert.ok(!p.calls.some((call) => call.command === 'remote_prepare_engine'));
  void p.click('new-scan');
  p.render();
  const reading = p.element('TaskCard').props as Record<string, unknown>;
  assert.equal(reading.title, '正在读取站点');
  assert.equal(reading.steps, undefined, 'no engine step on this computer');
  assert.match(String(reading.detail), /核对能覆盖的时段/);
});

test('a scan result without a site list is reported instead of breaking the page', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'compute', sitedir: '/server/sites' },
      ownData: true,
      sites: [site],
      step: 3,
    },
    [site],
    initialValues,
    { servers: [runServer], scan: async () => [site] },
  );
  await p.mount();
  await assert.rejects(p.click('new-scan'), /读取站点的结果格式不对/);
  p.render();
  assert.ok(
    p.elements.some(
      (element) =>
        element.type === 'Notice' &&
        element.props.tone === 'error' &&
        String((element.children as unknown[]).flat(Infinity).join('')).includes('格式不对'),
    ),
  );
});

test('remembered paths cover the fields each simulation kind and grid uses', () => {
  assert.deepEqual(simulation.kindPathFields('site', '', 'directory'), ['sitedir', 'forcingdir']);
  assert.deepEqual(simulation.kindPathFields('watershed', 'latlon', 'directory'), [
    'shapefile',
    'spatial_forcingdir',
    'forcing_dataset',
  ]);
  assert.deepEqual(simulation.kindPathFields('region', 'latlon', 'config'), ['forcing']);
  assert.deepEqual(simulation.kindPathFields('global', 'catchment', 'directory'), [
    'catchment',
    'spatial_forcingdir',
    'forcing_dataset',
  ]);
  assert.deepEqual(simulation.kindPathFields('region', 'unstructure', 'config'), [
    'mesh',
    'forcing',
  ]);
  assert.deepEqual(simulation.kindPathFields('', '', 'directory'), []);
});

test('prefill fills empty fields, replaces untouched prefills and keeps what the user typed', () => {
  const memory = {
    shapefile: '/basins/heihe.shp',
    spatial_forcingdir: '/forcing/jra',
    forcing_dataset: 'JRA3Q',
  };
  const fields = simulation.kindPathFields('watershed', 'latlon', 'directory');
  const empty = { shapefile: '', spatial_forcingdir: '', forcing_dataset: '', mesh: '' };
  const first = simulation.kindPathPrefill(empty, fields, memory, '/default/forcing', {});
  assert.deepEqual(first.patch, memory);
  assert.deepEqual(first.filled, memory);

  // Switching to a lat-lon region: the untouched shapefile no longer applies and is cleared,
  // the forcing folder follows the region's memory and the dataset is not carried over.
  const region = simulation.kindPathPrefill(
    { ...empty, ...first.patch },
    simulation.kindPathFields('region', 'latlon', 'directory'),
    { spatial_forcingdir: '/forcing/era5' },
    '/default/forcing',
    first.filled,
  );
  assert.deepEqual(region.patch, {
    shapefile: '',
    spatial_forcingdir: '/forcing/era5',
    forcing_dataset: '',
  });

  // Without memory the resource default forcing folder is used; typed values stay.
  const typed = simulation.kindPathPrefill(
    { ...empty, shapefile: '/mine.shp' },
    fields,
    {},
    '/default/forcing',
    {},
  );
  assert.equal(typed.patch.shapefile, undefined);
  assert.equal(typed.patch.spatial_forcingdir, '/default/forcing');
  assert.equal(typed.patch.forcing_dataset, '');

  // A remembered dataset is only used with its own folder.
  const otherFolder = simulation.kindPathPrefill(
    { ...empty, spatial_forcingdir: '/typed/forcing' },
    fields,
    memory,
    '',
    {},
  );
  assert.equal(otherFolder.patch.forcing_dataset, '');
});

test('remembered site folders are prefilled per server and read once on reaching the data step', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'compute', sitedir: '', forcingdir: '' },
      ownData: true,
      sites: [],
      step: 3,
    },
    [site],
    initialValues,
    {
      servers: [
        {
          ...runServer,
          kind_paths: {
            site: { sitedir: '/server/Sitedata', forcingdir: '/server/Forcing' },
            region: { mesh: '/server/mesh.nc' },
          },
        },
      ],
    },
  );
  await p.mount();
  await p.settle();
  const c = p.render();
  assert.equal(c.fields.find((field) => field.id === 'new-sitedir')?.value, '/server/Sitedata');
  assert.equal(c.fields.find((field) => field.id === 'new-forcingdir')?.value, '/server/Forcing');
  const scans = p.calls.filter((call) => call.command === 'remote_scan_sites');
  assert.equal(scans.length, 1);
  assert.equal(scans[0].args.dir, '/server/Sitedata');
  assert.equal(scans[0].args.forcingDir, '/server/Forcing');
  assert.deepEqual(p.calls.find((call) => call.command === 'remote_remember_kind_paths')?.args, {
    host: 'compute',
    kind: 'site',
    paths: { sitedir: '/server/Sitedata', forcingdir: '/server/Forcing' },
  });
  await p.settle();
  assert.equal(p.calls.filter((call) => call.command === 'remote_scan_sites').length, 1);
});

test('a typed site folder waits for the read button and is remembered once sites are found', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...form, sitedir: '', forcingdir: '' }, ownData: true, sites: [], step: 3 },
    [site],
    initialValues,
  );
  await p.mount();
  p.set('new-sitedir', '/typed/Sitedata');
  await p.settle();
  assert.ok(!p.calls.some((call) => call.command === 'scan_sites'));
  await p.click('new-scan');
  await p.settle();
  assert.equal(p.calls.filter((call) => call.command === 'scan_sites').length, 1);
  assert.deepEqual(p.calls.find((call) => call.command === 'remote_remember_kind_paths')?.args, {
    host: null,
    kind: 'site',
    paths: { sitedir: '/typed/Sitedata' },
  });
});

test('remote multi-site retries retain server identity and request atomic replay settings', async () => {
  let failed = true;
  const p = page(
    'NewCasePage',
    { ...multiSeed, form: { ...multiSeed.form, host: 'compute' } },
    multiSites,
    initialValues,
    {
      servers: [runServer],
      create: async (args) => {
        const request = args.request as { name: string; out: string; spinup_repeat: number };
        if (failed && request.name === 'CN-Cng') throw new Error('remote temporary error');
        return {
          dir: `/cache/multi-project/${request.name}`,
          root: '/cache/multi-project',
          remote_dir: request.out,
        };
      },
    },
  );
  await p.mount();
  await p.click('new-create');
  failed = false;
  await p.click('new-multi-retry');
  const batches = p.calls.filter((call) => call.command === 'remote_create_cases');
  assert.equal(batches.length, 2, 'one server batch per attempt');
  assert.ok(!p.calls.some((call) => call.command === 'remote_create_case'));
  const requests = batches.flatMap((call) => call.args.requests as Record<string, unknown>[]);
  assert.deepEqual(
    requests.map((request) => request.name),
    ['CN-Cng', 'other', 'CN-Cng'],
  );
  assert.ok(batches.every((call) => call.args.host === 'compute' && call.args.batch));
  assert.ok(requests.every((request) => request.spinup_repeat === 3));
  assert.ok(!p.calls.some((call) => call.command === 'set_spinup' || call.command === 'new_case'));
});

test('a server multi-site batch shows its progress and can be cancelled', async () => {
  let release: () => void = () => {};
  const p = page(
    'NewCasePage',
    { ...multiSeed, form: { ...multiSeed.form, host: 'compute' } },
    multiSites,
    initialValues,
    {
      servers: [runServer],
      create: async (args) => {
        await new Promise<void>((resolve) => (release = resolve));
        const request = args.request as { name: string; out: string };
        return {
          dir: `/cache/multi/${request.name}`,
          root: '/cache/multi',
          remote_dir: request.out,
        };
      },
    },
  );
  await p.mount();
  const creating = p.click('new-create');
  await p.settle();
  const call = p.calls.find((item) => item.command === 'remote_create_cases')!;
  const batch = call.args.batch as string;
  const progress = p.listeners.find((listener) => listener.event === 'remote-create://progress')!;
  const notices = () =>
    p.elements
      .filter((element) => element.type === 'Notice')
      .map((element) => (element.children as unknown[]).flat(Infinity).join(''));
  progress.callback({ batch: 'another', stage: 'local', done: 9, total: 9 });
  progress.callback({ batch, stage: 'server', done: 0, total: 2 });
  p.render();
  assert.ok(notices().some((text) => text.includes('正在 compute 上批量创建 2 个站点')));
  progress.callback({ batch, stage: 'local', done: 1, total: 2 });
  p.render();
  assert.ok(notices().some((text) => text.includes('正在整理本机配置：1 / 2')));
  await p.click('new-cancel-creation');
  assert.deepEqual(p.calls.find((item) => item.command === 'remote_create_cases_cancel')?.args, {
    batch,
  });
  release();
  await p.settle();
  release();
  await creating;
  await p.settle();
  assert.equal(p.listeners.length, 0, 'the progress listener is removed');
  assert.ok(!notices().some((text) => text.includes('整理本机配置')));
});

test('server spatial creation passes server paths and grid settings without local creation', async () => {
  const p = page(
    'NewCasePage',
    {
      form: {
        ...form,
        host: 'compute',
        kind: 'global',
        dlon: '1',
        dlat: '2',
        forcing: '/server/forcing.nml',
      },
      step: 5,
    },
    [site],
    initialValues,
    { servers: [runServer] },
  );
  await p.mount();
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'remote_create_case')?.args
    .request as Record<string, unknown>;
  assert.equal(request.kind, 'global');
  assert.equal(request.grid_kind, 'latlon');
  assert.equal(request.dlat, 2);
  assert.equal(request.forcing, '/server/forcing.nml');
  assert.ok(!p.calls.some((call) => call.command === 'new_spatial_case'));
  assert.deepEqual(p.opened, ['/cache/project/site']);
});

const spatialForcingForm = { ...form, kind: 'global', dlon: '1', dlat: '2' };
for (const host of ['', 'compute']) {
  test(`spatial forcing directory auto-selects a unique template and creates ${host || 'local'} configuration`, async () => {
    const p = page(
      'NewCasePage',
      { form: { ...spatialForcingForm, host }, step: 3 },
      [site],
      initialValues,
      {
        servers: [runServer],
        pickFolder: '/data/forcing',
      },
    );
    await p.mount();
    assert.ok(!p.render().fields.some((field) => field.id === 'new-forcing'));
    if (host) {
      const picking = p.click('new-pick-spatial_forcingdir');
      p.render();
      const browser = p.element('RemotePathBrowser');
      assert.equal(browser.props.directory, true);
      (browser.props.onSelect as (path: string) => void)('/data/forcing');
      await picking;
    } else await p.click('new-pick-spatial_forcingdir');
    await p.settle();
    const probe = p.calls.find((call) => call.command === 'probe_forcing_directory');
    assert.deepEqual(probe?.args, { path: '/data/forcing', host: host || null });
    assert.equal(
      p.render().fields.find((field) => field.id === 'new-forcing-dataset')?.value,
      'ERA5LAND',
    );
    await p.click('new-next');
    await p.click('new-next');
    await p.click('new-create');
    const created = p.calls.find(
      (call) => call.command === (host ? 'remote_create_case' : 'new_spatial_case'),
    )!;
    const request = (host ? created.args.request : created.args) as Record<string, unknown>;
    assert.equal(request[host ? 'forcing_dataset' : 'forcingDataset'], 'ERA5LAND');
    assert.equal(request[host ? 'forcing_dir' : 'forcingDir'], '/data/forcing');
    assert.equal(request.forcing, '');
    assert.ok(
      !(request.fields as { path: string }[]).some(
        (field) => field.path.toLowerCase() === 'def_forcing_namelist',
      ),
    );
  });
}

for (const result of [
  { candidates: ['ERA5', 'ERA5_LEddy'], truncated: false },
  { candidates: [], truncated: false },
  { candidates: ['ERA5LAND'], truncated: true },
]) {
  test(`spatial forcing requires manual confirmation for ${JSON.stringify(result)}`, async () => {
    const p = page('NewCasePage', { form: spatialForcingForm, step: 3 }, [site], initialValues, {
      forcingProbe: async (args) => ({ path: args.path, ...result }),
    });
    await p.mount();
    p.set('new-spatial_forcingdir', '/data/forcing');
    await p.settle();
    assert.equal(p.render().fields.find((field) => field.id === 'new-forcing-dataset')?.value, '');
    blocked(p, 'new-next', /确认数据集/);
    p.set('new-forcing-dataset', 'ERA5');
    assert.equal(p.render().actions.find((action) => action.id === 'new-next')?.enabled, true);
  });
}

test('spatial forcing drops late recognition results after path, mode and host changes', async () => {
  const pending: ((value: unknown) => void)[] = [];
  const p = page('NewCasePage', { form: spatialForcingForm, step: 3 }, [site], initialValues, {
    servers: [runServer],
    forcingProbe: () =>
      new Promise((done) => {
        pending.push(done);
      }),
  });
  await p.mount();
  p.set('new-spatial_forcingdir', '/old');
  p.set('new-spatial_forcingdir', '/new');
  pending[1]({ path: '/new', candidates: ['CMFD'], truncated: false });
  await p.settle();
  pending[0]({ path: '/old', candidates: ['ERA5LAND'], truncated: false });
  await p.settle();
  assert.equal(
    p.render().fields.find((field) => field.id === 'new-forcing-dataset')?.value,
    'CMFD',
  );
  p.set('new-spatial_forcingdir', '/later');
  p.set('new-forcing-mode', 'config');
  pending[2]({ path: '/later', candidates: ['ERA5LAND'], truncated: false });
  await p.settle();
  assert.ok(!p.render().fields.some((field) => field.id === 'new-forcing-dataset'));
  p.set('new-forcing-mode', 'directory');
  assert.equal(p.render().fields.find((field) => field.id === 'new-forcing-dataset')?.value, '');
  p.set('new-spatial_forcingdir', '/host-late');
  await p.click('new-step-1');
  p.set('new-run-host', 'compute');
  pending[3]({ path: '/host-late', candidates: ['ERA5LAND'], truncated: false });
  await p.goto('new-step-3');
  assert.equal(p.render().fields.find((field) => field.id === 'new-spatial_forcingdir')?.value, '');
});

test('spatial forcing keeps manual choices during detection and recovers from errors', async () => {
  let resolve: (value: unknown) => void = () => {};
  let reject = false;
  const p = page('NewCasePage', { form: spatialForcingForm, step: 3 }, [site], initialValues, {
    forcingProbe: () =>
      reject
        ? Promise.reject(new Error('SSH unavailable'))
        : new Promise((done) => {
            resolve = done;
          }),
  });
  await p.mount();
  p.set('new-spatial_forcingdir', '/data');
  p.set('new-forcing-dataset', 'CMFD');
  resolve({ path: '/data', candidates: ['ERA5LAND'], truncated: false });
  await p.settle();
  assert.equal(
    p.render().fields.find((field) => field.id === 'new-forcing-dataset')?.value,
    'CMFD',
  );
  reject = true;
  p.set('new-spatial_forcingdir', '/unavailable');
  await p.settle();
  assert.match(JSON.stringify(p.render().details.forcing), /SSH unavailable/);
  blocked(p, 'new-next', /确认数据集/);
  p.set('new-forcing-mode', 'config');
  p.set('new-forcing', '/custom/forcing.nml');
  assert.equal(p.render().actions.find((action) => action.id === 'new-next')?.enabled, true);
});

for (const host of ['', 'compute']) {
  test(`existing spatial case preserves forcing settings when changing ${host || 'local'} data directory`, async () => {
    const p = page('CasesPage', undefined, [site], initialValues, {
      profile: {
        grid: 'latlon',
        run_host: host,
        ...(host ? { remote_directory: '/server/site' } : {}),
      },
      pickFolder: '/data/replacement',
    });
    await p.mount();
    await p.goto('case-step-3');
    assert.ok(p.calls.some((call) => call.command === 'case_forcing_input'));
    assert.ok(!p.render().fields.some((field) => field.id === 'case-DEF_forcing_namelist'));
    assert.equal(p.element('SpatialForcingFields').props.currentDataset, 'ERA5LAND');
    if (host) {
      await p.click('case-pick-forcing-directory');
      const browser = p.element('RemotePathBrowser');
      assert.equal(browser.props.host, 'compute');
      (browser.props.onSelect as (path: string) => void)('/data/replacement');
    } else await p.click('case-pick-forcing-directory');
    await p.settle();
    assert.equal(p.render().fields.find((field) => field.id === 'case-forcing-dataset')?.value, '');
    await p.click('case-save');
    const args = p.calls.find((call) => call.command === 'case_data_save')?.args;
    assert.deepEqual(args?.fields, []);
    assert.deepEqual(args?.forcing, { directory: '/data/replacement', dataset: null });
    assert.equal(p.render().status, 'ready');
  });
}

test('existing spatial forcing template choice is saved explicitly and discard restores the original', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, { profile: { grid: 'latlon' } });
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-forcing-directory', '/new/forcing');
  p.set('case-forcing-dataset', 'CMFD');
  assert.ok(p.guard());
  await p.click('case-discard');
  assert.equal(
    p.render().fields.find((field) => field.id === 'case-forcing-directory')?.value,
    '/data/current',
  );
  assert.equal(p.render().fields.find((field) => field.id === 'case-forcing-dataset')?.value, '');
  assert.equal(p.guard(), null);
  p.set('case-forcing-directory', '/new/forcing');
  p.set('case-forcing-dataset', 'CMFD');
  await p.click('case-save');
  assert.deepEqual(p.calls.find((call) => call.command === 'case_data_save')?.args.forcing, {
    directory: '/new/forcing',
    dataset: 'CMFD',
  });
});

test('local-owned remote-run cases keep local forcing paths until runtime mapping', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, {
    profile: { grid: 'latlon', run_host: 'compute' },
    pickFolder: '/local/forcing',
  });
  await p.mount();
  await p.goto('case-step-3');
  await p.click('case-pick-forcing-directory');
  assert.ok(p.calls.some((call) => call.command === 'pick_folder'));
  assert.deepEqual(p.calls.find((call) => call.command === 'probe_forcing_directory')?.args, {
    path: '/local/forcing',
    host: null,
  });
});

test('unreadable existing forcing configuration retains the original file repair field', async () => {
  const p = page(
    'CasesPage',
    undefined,
    [site],
    { ...initialValues, DEF_forcing_namelist: '/missing.nml' },
    {
      profile: { grid: 'latlon' },
      schema: [...schema, { ...field('DEF_forcing_namelist', '强迫场'), kind: 'Character' }],
      forcingInput: async () => {
        throw new Error('forcing configuration missing');
      },
    },
  );
  await p.mount();
  await p.goto('case-step-3');
  assert.equal(
    p.render().fields.find((field) => field.id === 'case-DEF_forcing_namelist')?.value,
    '/missing.nml',
  );
  assert.ok(!p.render().fields.some((field) => field.id === 'case-forcing-directory'));
  p.set('case-DEF_forcing_namelist', '/repaired.nml');
  await p.click('case-save');
  assert.equal(p.calls.find((call) => call.command === 'case_data_save')?.args.forcing, undefined);
});

test('existing forcing recognition cannot restore discarded paths or results from another case', async () => {
  const pending: ((value: unknown) => void)[] = [];
  const p = page('CasesPage', undefined, [site], initialValues, {
    profile: { grid: 'latlon' },
    forcingProbe: () => new Promise((resolve) => pending.push(resolve)),
  });
  await p.mount();
  await p.goto('case-step-3');
  p.set('case-forcing-directory', '/old');
  p.set('case-forcing-directory', '/new');
  pending[1]({ path: '/new', candidates: ['CMFD'], truncated: false });
  await p.settle();
  pending[0]({ path: '/old', candidates: ['ERA5LAND'], truncated: false });
  await p.settle();
  assert.deepEqual((p.render().details.forcing as { candidates: string[] }).candidates, ['CMFD']);
  p.set('case-forcing-directory', '/discard');
  await p.click('case-discard');
  pending[2]({ path: '/discard', candidates: ['CMFD'], truncated: false });
  await p.settle();
  assert.equal(p.render().details.forcing, null);
  p.set('case-forcing-directory', '/previous-case');
  p.state.selectedCase.entry.dir = '/project/other';
  await p.settle();
  pending[3]({ path: '/previous-case', candidates: ['CMFD'], truncated: false });
  await p.settle();
  assert.equal(p.render().details.forcing, null);
  assert.equal(
    p.render().fields.find((field) => field.id === 'case-forcing-directory')?.value,
    '/data/current',
  );
});

const methaneSchema = [
  ...setupSchema,
  ...[
    'DEF_USE_TRACER',
    'DEF_USE_Campbell_SOIL_MODEL',
    'DEF_USE_VariablySaturatedFlow',
    'DEF_USE_LULCC',
    'DEF_USE_Dynamic_Wetland',
  ].map((name) => ({ ...field(name, '示踪剂', '.false.'), kind: 'Logical' })),
  ...configuration.tracerKeys.map((name) => ({
    ...field(name, '示踪剂', name === 'DEF_TRACER_NUM' ? '0' : "''"),
    kind: name === 'DEF_TRACER_NUM' ? 'Integer' : 'Character',
  })),
];
for (const marked of ['name', 'site', 'observation']) {
  test(`methane example selection configures BGC and a valid CH4 tracer from ${marked}`, async () => {
    const methane = {
      ...site,
      name: marked === 'name' ? 'AT-Neu' : 'Methane-site',
      site_file: marked === 'site' ? '/example/ch4/site.nc' : '/methane/site.nc',
      obs_file: marked === 'observation' ? '/obs/flux-ch4.nc' : null,
    };
    const p = page('NewCasePage', { form, sites: [methane], step: 3 }, [methane], initialValues, {
      schema: methaneSchema,
      kernels: setupKernels,
    });
    await p.mount();
    await p.click(`new-select-${methane.name}`);
    await p.click('new-next');
    const setup = p.element('ModelSettings').props;
    assert.equal(setup.mode, 'PFT');
    const values = (setup.setupEnvironment as { caseValues: Record<string, string> }).caseValues;
    assert.equal(values.DEF_USE_BGC, '.true.');
    assert.equal(values.DEF_USE_TRACER, '.true.');
    assert.equal(values.DEF_USE_Campbell_SOIL_MODEL, '.false.');
    assert.equal(values.DEF_USE_VariablySaturatedFlow, '.true.');
    assert.equal(values.DEF_USE_LULCC, '.false.');
    assert.equal(values.DEF_URBAN_RUN, '.false.');
    const rows = configuration.tracerRows({ physics: values, parameters: [], process_files: [] });
    assert.equal(rows.length, 1);
    assert.equal(rows[0].NAMES, 'CH4');
    assert.equal(rows[0].TYPES, 'gas');
    assert.equal(rows[0].PARAM_FILES, 'standard_ch4_parameter.nml');
    await p.click('new-next');
    await p.click('new-create');
    const created = p.calls.find((call) => call.command === 'new_case')!;
    assert.ok(created);
    const written = created.args.fields as { path: string; value: string }[];
    for (const key of [
      'DEF_USE_TRACER',
      'DEF_USE_BGC',
      'DEF_TRACER_NAMES',
      'DEF_TRACER_PARAM_FILES',
    ])
      assert.equal(written.find((entry) => entry.path === key)?.value, values[key]);
    assert.deepEqual(p.state.version.overlay.physics, {});
    assert.ok(!p.calls.some((call) => call.command === 'model_create'));
  });
}

test('methane example preserves PC and custom tracer metadata while removing crop leftovers', async () => {
  const methane = { ...site, name: 'AT-Neu' };
  const p = page(
    'NewCasePage',
    {
      form: { ...form, subgrid: 'PC' },
      sites: [methane],
      step: 3,
      caseValues: {
        DEF_TRACER_NUM: '2',
        DEF_TRACER_NAMES: "'Cl,CH4'",
        DEF_TRACER_TYPES: "'solute,gas'",
        DEF_TRACER_PARAM_FILES: "'Cl:/custom/cl.nml,CH4:/custom/ch4.nml'",
        DEF_TUNING_CROP_PLANTING_DAY: '120',
        DEF_HIST_FREQ: "'DAILY'",
      },
    },
    [methane],
    initialValues,
    { schema: methaneSchema, kernels: setupKernels },
  );
  await p.mount();
  await p.click('new-select-AT-Neu');
  await p.click('new-next');
  const setup = p.element('ModelSettings').props;
  assert.equal(setup.mode, 'PC');
  const values = (setup.setupEnvironment as { caseValues: Record<string, string> }).caseValues;
  assert.equal(values.DEF_TRACER_NUM, '2');
  assert.equal(values.DEF_TRACER_NAMES, "'Cl,CH4'");
  assert.equal(values.DEF_TRACER_PARAM_FILES, "'Cl:/custom/cl.nml,CH4:/custom/ch4.nml'");
  assert.equal(values.DEF_TUNING_CROP_PLANTING_DAY, undefined);
  assert.equal(values.DEF_HIST_FREQ, "'DAILY'");
});

test('methane selection with missing tracer schema leaves the original site and settings untouched', async () => {
  const methane = { ...site, name: 'AT-Neu', site_file: '/methane/site.nc' };
  const p = page(
    'NewCasePage',
    { form, sites: [site, methane], step: 3 },
    [site, methane],
    initialValues,
    { schema: setupSchema },
  );
  await p.mount();
  await p.click('new-select-AT-Neu');
  await p.click('new-next');
  const values = (
    p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> }
  ).caseValues;
  assert.notEqual(values.DEF_USE_TRACER, '.true.');
  assert.deepEqual(p.state.version.overlay.physics, {});
});

test('selecting urban after methane disables BGC and tracers while retaining definitions', async () => {
  const methane = { ...site, name: 'AT-Neu', site_file: '/methane/site.nc' };
  const urban = { ...site, name: 'AU-Preston', site_file: '/urban/site.nc', urban: true };
  const p = page(
    'NewCasePage',
    { form, sites: [methane, urban], step: 3 },
    [methane, urban],
    initialValues,
    { schema: methaneSchema, kernels: setupKernels },
  );
  await p.mount();
  await p.click('new-select-AT-Neu');
  await p.click('new-select-AU-Preston');
  await p.click('new-next');
  const setup = p.element('ModelSettings').props;
  assert.equal(setup.mode, 'IGBP');
  const values = (setup.setupEnvironment as { caseValues: Record<string, string> }).caseValues;
  assert.equal(values.DEF_USE_BGC, '.false.');
  assert.equal(values.DEF_USE_TRACER, '.false.');
  assert.equal(values.DEF_URBAN_RUN, '.true.');
  assert.equal(values.DEF_TRACER_NAMES, "'CH4'");
});

test('methane example retains inherited custom tracer scalar metadata without copying unrelated version flags', async () => {
  const methane = { ...site, name: 'AT-Neu' };
  const p = page('NewCasePage', { form, sites: [methane], step: 3 }, [methane], initialValues, {
    schema: methaneSchema,
    kernels: setupKernels,
  });
  const overlay = p.state.version.overlay as Overlay;
  Object.assign(overlay.physics, {
    DEF_TRACER_NUM: '1',
    DEF_TRACER_NAMES: "'Cl'",
    DEF_TRACER_TYPES: "'solute'",
    DEF_TRACER_PARAM_FILES: "'CustomCl:/custom/cl.nml'",
    DEF_UNRELATED_VERSION_FLAG: '.true.',
  });
  overlay.parameters.push({
    parameter_id: 'case:DEF_TRACER_NAMES',
    raw_key: 'DEF_TRACER_NAMES',
    scope_instance: { kind: 'case-scalar' },
    value: "'CustomCl'",
  });
  await p.mount();
  await p.click('new-select-AT-Neu');
  await p.click('new-next');
  const values = (
    p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> }
  ).caseValues;
  const rows = configuration.tracerRows({ physics: values, parameters: [], process_files: [] });
  assert.deepEqual(
    rows.map((row) => [row.NAMES, row.TYPES]),
    [
      ['CustomCl', 'solute'],
      ['CH4', 'gas'],
    ],
  );
  assert.equal(rows[0].PARAM_FILES, '/custom/cl.nml');
  assert.equal(values.DEF_UNRELATED_VERSION_FLAG, undefined);
  assert.equal(overlay.physics.DEF_TRACER_NAMES, "'Cl'");
  assert.equal(overlay.parameters[0].value, "'CustomCl'");
});

for (const retained of ['new', 'missing-file', 'builtin-file', 'custom-file', 'custom-asset']) {
  test(`methane wetwat default only clears dynamic wetlands when generated: ${retained}`, async () => {
    const methane = { ...site, name: 'AT-Neu' };
    const caseValues = {
      DEF_USE_Dynamic_Wetland: '.true.',
      ...(['missing-file', 'builtin-file', 'custom-file'].includes(retained)
        ? {
            DEF_TRACER_NUM: '1',
            DEF_TRACER_NAMES: "'CH4'",
            DEF_TRACER_TYPES: "'gas'",
            DEF_TRACER_PARAM_FILES:
              retained === 'custom-file'
                ? "'CH4:/custom/ch4.nml'"
                : retained === 'builtin-file'
                  ? "'CH4:standard_ch4_parameter.nml'"
                  : "'null'",
          }
        : {}),
    };
    const p = page(
      'NewCasePage',
      { form, caseValues, sites: [methane], step: 3 },
      [methane],
      initialValues,
      { schema: methaneSchema, kernels: setupKernels },
    );
    if (retained === 'custom-asset')
      (p.state.version.overlay as Overlay).process_files.push({
        relative_path: 'standard_ch4_parameter.nml',
        sha256: 'custom',
      });
    await p.mount();
    await p.click('new-select-AT-Neu');
    await p.click('new-next');
    const values = (
      p.element('ModelSettings').props.setupEnvironment as { caseValues: Record<string, string> }
    ).caseValues;
    const expected = ['custom-file', 'custom-asset'].includes(retained) ? '.true.' : '.false.';
    assert.equal(values.DEF_USE_Dynamic_Wetland, expected);
    await p.click('new-next');
    await p.click('new-create');
    const created = p.calls.find((call) => call.command === 'new_case')!;
    assert.equal(
      (created.args.fields as { path: string; value: string }[]).find(
        (item) => item.path === 'DEF_USE_Dynamic_Wetland',
      )?.value,
      expected,
    );
  });
}

test('methane example rejects oversized tracer definitions without truncating or changing the selected site', async () => {
  const methane = { ...site, name: 'AT-Neu', site_file: '/methane/site.nc' };
  const caseValues = {
    DEF_TRACER_NUM: '101',
    DEF_TRACER_NAMES: `'${['CH4', ...Array.from({ length: 100 }, (_, index) => `Cl${index}`)].join(',')}'`,
    DEF_TRACER_TYPES: `'${['gas', ...Array.from({ length: 100 }, () => 'solute')].join(',')}'`,
    DEF_TRACER_PARAM_FILES: "'CH4:/custom/ch4.nml'",
  };
  const p = page(
    'NewCasePage',
    { form, caseValues, sites: [site, methane], step: 3 },
    [site, methane],
    initialValues,
    { schema: methaneSchema, kernels: setupKernels },
  );
  await p.mount();
  await p.click('new-select-AT-Neu');
  await p.click('new-next');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  const seed = p.state.newCaseSeed as { form: typeof form; caseValues: Record<string, string> };
  assert.equal(seed.form.site, form.site);
  assert.equal(seed.form.name, form.name);
  assert.deepEqual(seed.caseValues, { ...caseValues, ...modelSettings.caseSchemeFields('IGBP') });
});

for (const station of ['AT-Neu', 'CN-Cng']) {
  test(`bundled ${station} creates inside the user's simulation container`, async () => {
    const methane = { ...site, name: 'AT-Neu', site_file: '/methane/site.nc' };
    const p = page(
      'NewCasePage',
      { form: { ...form, name: 'test' }, sites: [site, methane], step: 3 },
      [site, methane],
      initialValues,
      { schema: methaneSchema, kernels: setupKernels },
    );
    await p.mount();
    await p.click('new-select-AT-Neu');
    await p.click(`new-select-${station}`);
    assert.match(JSON.stringify(p.elements), new RegExp(`/project/test/${station}`));
    await p.click('new-next');
    await p.click('new-next');
    assert.equal(p.render().details.name, 'test');
    await p.click('new-create');
    const request = p.calls.find((call) => call.command === 'new_case')!;
    assert.equal(request.args.name, station);
    assert.equal(request.args.out, `/project/test/${station}`);
    assert.deepEqual(p.opened, [`/project/test/${station}`]);
    assert.equal(p.state.root, '/project/test');
  });
}

test('example installation and switching to own data retain the simulation name', async () => {
  const p = page('NewCasePage', { form: { ...form, name: 'test' }, sites: [site], step: 3 }, [
    site,
  ]);
  await p.mount();
  await p.click('new-example');
  await p.click('new-own-data');
  await p.click('new-previous');
  await p.click('new-previous');
  await p.click('new-run-resources');
  const seed = p.state.newCaseSeed as { form: typeof form };
  assert.equal(seed.form.name, 'test');
});

test('own-data single station keeps a single-level destination and user name', async () => {
  const p = page(
    'NewCasePage',
    {
      multiSiteEnabled: false,
      form: { ...form, name: 'test' },
      ownData: true,
      sites: [site],
      step: 5,
    },
    [site],
  );
  await p.mount();
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case')!;
  assert.equal(request.args.name, 'test');
  assert.equal(request.args.out, '/project/test');
  assert.deepEqual(p.opened, ['/project/test']);
  assert.equal(p.state.root, '/project');
});

test('invalid bundled station names cannot escape the simulation container', async () => {
  const invalid = { ...site, name: '../escape' };
  const p = page('NewCasePage', { form: { ...form, name: 'test' }, sites: [invalid], step: 5 }, [
    invalid,
  ]);
  await p.mount();
  assert.equal(p.render().actions.find((action) => action.id === 'new-create')!.enabled, false);
  await assert.rejects(p.click('new-create'));
  assert.ok(!p.calls.some((call) => call.command === 'new_case'));
});

test('creation review exposes station concurrency and carries resource choices to the run page', async () => {
  const p = page('NewCasePage', multiSeed, multiSites);
  await p.mount();
  assert.equal(
    p.render().fields.find((field) => field.id === 'new-run-workers')?.label,
    '同时运行的站点数',
  );
  assert.ok(!p.render().fields.some((field) => field.id === 'new-run-ranks'));
  p.set('new-run-threads', '2');
  p.set('new-run-workers', '3');
  await p.click('new-create');
  assert.equal(p.state.projectRunSettings?.threads, '2');
  assert.equal(p.state.projectRunSettings?.workers, '3');
  assert.equal(p.state.projectRunSettings?.cases.length, 2);
  assert.ok(!p.calls.some((call) => call.command === 'run_batch'));
});

test('server resource choices inherit configured defaults and reject invalid numbers', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'cluster', root: '/server/project' },
      sites: [site],
      step: 5,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [
        { host: 'cluster', root: '/work', maps: [], threads: 8, cpus: 4, scheduler: 'slurm' },
      ],
    },
  );
  await p.mount();
  assert.equal(p.render().fields.find((field) => field.id === 'new-run-threads')?.value, '8');
  assert.ok(
    !p.render().fields.some((field) => field.id === 'new-run-cpus'),
    'the Rust engine requests its threads; only Fortran asks for CPUs per MPI rank',
  );
  assert.equal(p.render().fields.find((field) => field.id === 'new-run-scheduler')?.value, 'slurm');
  assert.ok(!p.render().fields.some((field) => field.id === 'new-run-workers'));
  for (const invalid of ['0', '-1', '1.5', '257']) {
    p.set('new-run-threads', invalid);
    assert.equal(p.render().actions.find((action) => action.id === 'new-create')?.enabled, false);
  }
  p.set('new-run-threads', '2');
  p.set('new-run-scheduler', 'bare');
  await p.click('new-create');
  assert.equal(p.state.projectRunSettings?.scheduler, 'bare');
  assert.equal(p.state.projectRunSettings?.threads, '2');
});

test('a Rust run on a scheduler requests as many CPUs as threads, not the server CPU setting', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'cluster', root: '/server/project' },
      sites: [site],
      step: 5,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [
        { host: 'cluster', root: '/work', maps: [], threads: 8, cpus: 4, scheduler: 'slurm' },
      ],
    },
  );
  await p.mount();
  p.set('new-run-threads', '6');
  await p.click('new-create');
  assert.equal(p.state.projectRunSettings?.scheduler, 'slurm');
  assert.equal(p.state.projectRunSettings?.cpus, '6');
});

test('the CPU and thread boxes can be emptied: the server default is only a hint', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, engine: 'fortran', host: 'cluster', root: '/server/project' },
      sites: [site],
      step: 5,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [
        { host: 'cluster', root: '/work', maps: [], threads: 8, cpus: 4, scheduler: 'slurm' },
      ],
    },
  );
  await p.mount();
  const box = (hint: string) =>
    p.elements.find(
      (e) => e.type === 'Input' && (e.props as { placeholder?: string }).placeholder === hint,
    );
  p.render();
  // Untouched: empty box, default as the hint, default as the effective value.
  assert.equal((box('4')?.props as { value: string }).value, '');
  assert.equal(p.render().fields.find((f) => f.id === 'new-run-cpus')?.value, '4');
  p.set('new-run-cpus', '12');
  p.render();
  assert.equal(p.render().fields.find((f) => f.id === 'new-run-cpus')?.value, '12');
  // Deleting the typed number leaves the box empty (it used to refill with the default at once).
  p.set('new-run-cpus', '');
  p.render();
  assert.equal((box('4')?.props as { value: string }).value, '');
  assert.equal(p.render().fields.find((f) => f.id === 'new-run-cpus')?.value, '4');
});

test('step 5 says how many nodes a big MPI run takes, and refuses one that cannot be placed', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, engine: 'fortran', host: 'cluster', root: '/server/project' },
      sites: [site],
      step: 5,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [
        {
          host: 'cluster',
          root: '/work',
          maps: [],
          threads: 1,
          cpus: 1,
          cores_per_node: 48,
          scheduler: 'lsf',
        },
      ],
    },
  );
  await p.mount();
  const note = () =>
    p.elements
      .filter((e) => e.type === 'p')
      .map((e) => (e.children as unknown[]).join(''))
      .find((text) => /占用|节点/.test(text));
  p.set('new-run-ranks', '96');
  p.render();
  assert.equal(note(), '将占用 2 个节点，每个节点 48 核，共 96 核（自动计算）。');
  assert.equal(p.render().actions.find((a) => a.id === 'new-create')?.enabled, true);
  // One process cannot be spread over nodes: it is said, and creating is refused.
  p.set('new-run-ranks', '1');
  p.set('new-run-cpus', '96');
  p.render();
  assert.match(note() ?? '', /单个进程要 96 个核/);
  assert.equal(p.render().actions.find((a) => a.id === 'new-create')?.enabled, false);
  // Fits on one node: nothing is said.
  p.set('new-run-cpus', '8');
  p.render();
  assert.equal(note(), undefined);
});

test('direct execution has no separate CPU request: the CPUs are the case threads', async () => {
  const p = page(
    'NewCasePage',
    {
      form: { ...form, host: 'cluster', root: '/server/project' },
      sites: [site],
      step: 5,
      ownData: true,
      multiSiteEnabled: false,
    },
    [site],
    initialValues,
    {
      servers: [
        { host: 'cluster', root: '/work', maps: [], threads: 8, cpus: 4, scheduler: 'bare' },
      ],
    },
  );
  await p.mount();
  assert.equal(p.render().fields.find((field) => field.id === 'new-run-scheduler')?.value, 'bare');
  assert.ok(!p.render().fields.some((field) => field.id === 'new-run-cpus'));
  p.set('new-run-threads', '6');
  await p.click('new-create');
  assert.equal(p.state.projectRunSettings?.threads, '6');
  assert.equal(p.state.projectRunSettings?.cpus, '6');
});

test('multi-site success opens the created project only after all spinup writes finish', async () => {
  const cases = [{ dir: '/project/multi-project/CN-Cng', name: 'CN-Cng' }];
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    listCases: async () => cases,
  });
  await p.mount();
  await p.click('new-create');
  assert.deepEqual(p.routes, ['project']);
  assert.equal(p.state.root, '/project/multi-project');
  assert.deepEqual((p.state as typeof p.state & { cases: unknown }).cases, cases);
  assert.equal(p.state.selectedCase, null);
  assert.equal(p.state.newCaseSeed, null);
  assert.equal((p.state as typeof p.state & { projectCasesOpen: boolean }).projectCasesOpen, true);
  assert.deepEqual(p.state.projectRunSettings, {
    root: '/project/multi-project',
    cases: multiSites.map((station) => `/project/multi-project/${station.name}`),
    host: '',
    threads: '1',
    ranks: '1',
    workers: '1',
    scheduler: 'auto',
    cpus: '1',
  });
  assert.deepEqual(p.calls.find((call) => call.command === 'save_recent')?.args, {
    key: 'root',
    value: '/project/multi-project',
  });
  const listed = p.calls.findIndex((call) => call.command === 'list_cases');
  assert.ok(
    p.calls
      .map((call, index) => (call.command === 'set_spinup' ? index : -1))
      .every((index) => index < listed),
  );
  assert.ok(!p.calls.some((call) => /run_case|run_start|submit/.test(call.command)));
});

test('failed project loading keeps completed results and manual retry never recreates stations', async () => {
  let failed = true;
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    listCases: async () => {
      if (failed) throw new Error('project list unavailable');
      return [];
    },
  });
  await p.mount();
  await assert.rejects(p.click('new-create'), /project list unavailable/);
  assert.deepEqual(p.routes, []);
  assert.equal(
    p.render().actions.find((action) => action.id === 'new-multi-project')?.enabled,
    true,
  );
  assert.equal(
    p.render().actions.find((action) => action.id === 'new-multi-retry')?.enabled,
    false,
  );
  const resultIndex = p.elements.findIndex(
    (element) => element.type === 'Card' && element.props.title === '创建结果',
  );
  const reviewIndex = p.elements.findIndex(
    (element) => element.type === 'Card' && element.props.title !== '创建结果',
  );
  assert.ok(resultIndex >= 0 && resultIndex < reviewIndex);
  failed = false;
  await p.click('new-multi-project');
  assert.deepEqual(p.routes, ['project']);
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 2);
  assert.equal(p.calls.filter((call) => call.command === 'set_spinup').length, 2);
});

test('remote multi-site completion opens the returned local project mirror', async () => {
  const p = page(
    'NewCasePage',
    { ...multiSeed, form: { ...multiSeed.form, host: 'compute', root: '/remote/projects' } },
    multiSites,
    initialValues,
    {
      servers: [runServer],
      create: async (args) => {
        const request = args.request as { name: string; out: string };
        return {
          dir: `/cache/project/${request.name}`,
          root: '/cache/project',
          remote_dir: request.out,
        };
      },
    },
  );
  await p.mount();
  await p.click('new-create');
  assert.deepEqual(p.calls.find((call) => call.command === 'list_cases')?.args, {
    root: '/cache/project',
  });
  assert.equal(p.state.root, '/cache/project');
  assert.deepEqual(p.routes, ['project']);
});

test('refused project navigation reports an error and retries without recreating cases', async () => {
  let permitted = false;
  const p = page('NewCasePage', multiSeed, multiSites, initialValues, {
    navigate: () => permitted,
  });
  await p.mount();
  await assert.rejects(p.click('new-create'), /项目页未能打开/);
  assert.deepEqual(p.routes, []);
  assert.equal((p.state as typeof p.state & { projectCasesOpen: boolean }).projectCasesOpen, false);
  assert.equal(
    p.render().actions.find((action) => action.id === 'new-multi-project')?.enabled,
    true,
  );
  permitted = true;
  await p.click('new-multi-project');
  assert.deepEqual(p.routes, ['project']);
  assert.equal(p.calls.filter((call) => call.command === 'new_case').length, 2);
});

for (const preference of [undefined, false, true]) {
  test(`own-data creation defaults to multiple stations while preserving seed preference ${preference}`, async () => {
    const p = page(
      'NewCasePage',
      {
        form,
        sites: [site],
        ownData: true,
        step: 3,
        ...(preference === undefined ? {} : { multiSiteEnabled: preference }),
      },
      [site],
    );
    await p.mount();
    assert.equal(
      p.render().fields.find((field) => field.id === 'new-multi-site')!.value,
      String(preference ?? true),
    );
  });
}

test('switching bundled data to own data defaults to multiple stations', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 3 }, [site]);
  await p.mount();
  await p.click('new-example');
  await p.click('new-own-data');
  assert.equal(p.render().fields.find((field) => field.id === 'new-multi-site')!.value, 'true');
});

test('multi-site confirmation summarizes the count without duplicating per-station details', async () => {
  const p = page('NewCasePage', multiSeed, multiSites);
  await p.mount();
  const text = JSON.stringify(p.elements);
  assert.match(text, /已选择 2 个站点/);
  assert.doesNotMatch(text, /2005-03-01|2008-12-31|multi-project\/other/);
  assert.equal(p.render().actions.find((action) => action.id === 'new-create')!.enabled, true);
});

test('new case calendar fields use the form setters and preserve spinup configuration', async () => {
  const p = page('NewCasePage', {
    form: { ...form, spinup_end: '2001-06-01' },
    sites: [site],
    step: 3,
  });
  await p.mount();
  const dates = p.elements.filter((e) => e.type === 'DateInput');
  assert.equal(dates.length, 3);
  assert.ok(dates.every((e) => e.props.disabled === false && e.props.calendarLabel));
  (dates[0].props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '20010201' },
  });
  assert.equal(p.render().fields.find((f) => f.id === 'new-start')?.value, '20010201');
  (dates[0].props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '2001-02-01' },
  });
  const c = p.render();
  assert.equal(c.fields.find((f) => f.id === 'new-start')?.value, '2001-02-01');
  assert.equal(c.fields.find((f) => f.id === 'new-spinup_end')?.value, '2001-06-01');
  assert.equal(c.actions.find((a) => a.id === 'new-next')?.enabled, true);
});

test('existing case calendar fields save dates without changing time-of-day or spinup', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.goto('case-step-3');
  const dates = p.elements.filter((e) => e.type === 'DateInput');
  assert.equal(dates.length, 3);
  assert.ok(dates.every((e) => e.props.disabled === false && e.props.calendarLabel));
  (dates[0].props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '20010101' },
  });
  assert.equal(p.render().fields.find((f) => f.id === 'case-start-date')?.value, '20010101');
  await assert.rejects(p.click('case-save'), /时段/);
  (dates[0].props.onChange as (event: { target: { value: string } }) => void)({
    target: { value: '2001-01-01' },
  });
  p.render();
  await p.click('case-save');
  const saved = p.calls.find((call) => call.command === 'case_data_save');
  assert.ok(saved);
  assert.deepEqual(saved.args.fields, [{ path: 'DEF_simulation_time%start_year', value: '2001' }]);
});

test('spatial coverage fills automatic dates and protects manual edits on dataset changes', async () => {
  const p = page(
    'NewCasePage',
    { form: { ...spatialForcingForm, start: '', end: '' }, step: 3 },
    [site],
    initialValues,
    {
      forcingProbe: async (args) => ({
        path: args.path,
        candidates: ['ERA5LAND'],
        truncated: false,
        coverage:
          args.dataset === 'CMFD'
            ? { start: '2010-01-01', end: '2012-12-31' }
            : { start: '2000-01-01', end: '2005-12-31' },
        coverage_error: null,
      }),
    },
  );
  await p.mount();
  p.set('new-spatial_forcingdir', '/data');
  await p.settle();
  assert.equal(p.render().fields.find((f) => f.id === 'new-start')?.value, '2000-01-01');
  assert.equal(p.render().fields.find((f) => f.id === 'new-end')?.value, '2005-12-31');
  p.set('new-start', '200');
  p.set('new-forcing-dataset', 'CMFD');
  await p.settle();
  assert.equal(p.render().fields.find((f) => f.id === 'new-start')?.value, '200');
  assert.equal(p.render().fields.find((f) => f.id === 'new-end')?.value, '2012-12-31');
  assert.deepEqual(p.calls.filter((c) => c.command === 'probe_forcing_directory').at(-1)?.args, {
    path: '/data',
    host: null,
    dataset: 'CMFD',
  });
});

test('spatial coverage ignores stale datasets and unblurred directory edits', async () => {
  const pending: ((value: unknown) => void)[] = [];
  const p = page(
    'NewCasePage',
    { form: { ...spatialForcingForm, start: '', end: '' }, step: 3 },
    [site],
    initialValues,
    {
      forcingProbe: () => new Promise((resolve) => pending.push(resolve)),
    },
  );
  await p.mount();
  p.set('new-spatial_forcingdir', '/data');
  p.set('new-forcing-dataset', 'CMFD');
  const response = (start: string) => ({
    path: '/data',
    candidates: ['CMFD'],
    truncated: false,
    coverage: { start, end: '2012-12-31' },
  });
  pending[0](response('2000-01-01'));
  await p.settle();
  assert.equal(p.render().fields.find((f) => f.id === 'new-start')?.value, '');
  (p.element('SpatialForcingFields').props.onDirectoryChange as (value: string) => void)('/typing');
  pending[1](response('2010-01-01'));
  await p.settle();
  assert.equal(p.render().fields.find((f) => f.id === 'new-start')?.value, '');
  assert.equal(p.render().details.forcing, null);
});

test('new simulations prefill the resource default data paths and sync edits back', async () => {
  const p = page('NewCasePage', undefined, [site], initialValues, {
    local: {
      project_dir: '/defaults/projects',
      rawdata_dir: '/defaults/raw',
      runtime_dir: '/defaults/runtime',
      forcing_dir: '',
    },
  });
  await p.mount();
  assert.equal(p.render().fields.find((f) => f.id === 'new-root')?.value, '/defaults/projects');
  p.set('new-engine', 'rust');
  p.set('new-name', 'CN-Cng');
  p.set('new-root', '/project');
  await p.click('new-next');
  p.set('new-kind', 'site');
  const c = await p.click('new-next');
  assert.equal(c.fields.find((f) => f.id === 'new-rawdata')?.value, '/defaults/raw');
  assert.equal(c.fields.find((f) => f.id === 'new-runtime')?.value, '/defaults/runtime');
  for (const [key, value] of Object.entries(form))
    if (key !== 'rawdata' && p.render().fields.some((f) => f.id === `new-${key}`))
      p.set(`new-${key}`, value);
  await p.click('new-next');
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('IGBP');
  p.render();
  await p.click('new-next');
  await p.click('new-create');
  assert.equal(p.calls.find((call) => call.command === 'new_case')?.args.rawdata, '/defaults/raw');
  assert.deepEqual(p.calls.find((call) => call.command === 'remote_remember_data_paths')?.args, {
    host: null,
    project: '/project',
    rawdata: '/defaults/raw',
    runtime: '/runtime',
    forcing: null,
  });
});

test('on a server the own-data source shows as chosen and pressing it again stays put', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 1 }, [site], initialValues, {
    servers: [runServer],
  });
  await p.mount();
  p.set('new-run-host', 'compute');
  p.set('new-name', 'CN-Cng');
  await p.click('new-next');
  p.set('new-kind', 'site');
  await p.click('new-next');
  const ownButton = () =>
    p.elements.find(
      (element) =>
        element.type === 'Button' && JSON.stringify(element.children).includes('使用自己的数据'),
    )!;
  assert.equal(ownButton().props['aria-pressed'], true);
  assert.ok(!p.elements.some((e) => JSON.stringify(e.children).includes('使用内置示例')));
  await p.click('new-own-data');
  assert.equal(ownButton().props['aria-pressed'], true);
  assert.ok(p.render().actions.some((action) => action.id === 'new-scan'));
});

test('waiting notices count seconds and minutes', () => {
  assert.equal(simulation.elapsedLabel(0), '0 秒');
  assert.equal(simulation.elapsedLabel(45_900), '45 秒');
  assert.equal(simulation.elapsedLabel(130_000), '2 分 10 秒');
  assert.equal(simulation.elapsedLabel(130_000, true), '2 min 10 s');
  assert.equal(simulation.elapsedLabel(-5), '0 秒');
});

test('after a partly failed server batch, Back edits only the sites not yet created', async () => {
  const p = page(
    'NewCasePage',
    { ...multiSeed, form: { ...multiSeed.form, host: 'compute' } },
    multiSites,
    initialValues,
    {
      servers: [runServer],
      create: async (args) => {
        const request = args.request as { name: string; out: string };
        if (request.name === 'CN-Cng') throw new Error('model registry is locked');
        return {
          dir: `/cache/multi/${request.name}`,
          root: '/cache/multi',
          remote_dir: request.out,
        };
      },
    },
  );
  await p.mount();
  await p.click('new-create');
  let c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'new-previous')?.enabled, true);
  assert.ok(
    p.elements.some(
      (e) =>
        e.type === 'p' &&
        String((e.children as unknown[]).flat(Infinity).join('')).includes('设置已锁定'),
    ),
  );
  await p.click('new-previous');
  c = p.render();
  assert.equal((c.details as { step?: number }).step, 3);
  const drafts = p.element('MultiSiteSettings').props.drafts as { site: string }[];
  assert.deepEqual(
    drafts.map((draft) => draft.site),
    multiSites.filter((site) => site.name === 'CN-Cng').map((site) => site.site_file),
  );
});
