import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as caseModel from './case-model';
import { caseFieldVisible } from '../versions/configuration-options';
import * as simulation from './simulation-model';
import * as modelSettings from './model-settings';
import * as historyVariables from './history-variables';
import { compatibleKernel, type KernelChoice } from '../runs/run-model';
import type { UiField, UiAction } from '../../lib/guide';
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
    histories?: { name: string; on: boolean; writable: boolean; settable: boolean }[];
    modelFields?: caseModel.SchemaField[];
    kernels?: KernelChoice[];
    schema?: caseModel.SchemaField[];
    configuration?: (args: Record<string, unknown>) => Promise<unknown>;
    create?: (args: Record<string, unknown>) => Promise<unknown>;
    cancelCreation?: (args: Record<string, unknown>) => Promise<number>;
    scan?: () => Promise<unknown>;
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
    selectedCase: { entry: { dir: '/project/site', name: 'site' }, profile: {} },
    newCaseSeed: seed ?? null,
    simulationReturn: null as unknown,
  };
  const calls: { command: string; args: Record<string, unknown> }[] = [],
    routes: string[] = [],
    opened: string[] = [],
    elements: Element[] = [];
  let slots: unknown[] = [],
    cursor = 0,
    effectCursor = 0,
    contract: Contract,
    guard: () => unknown = () => null,
    failCreate = false;
  let effects: (() => unknown)[] = [],
    registered = new Map<number, { dependencies: unknown[] | undefined; cleanup?: () => void }>();
  const update = (patch: object) => Object.assign(state, patch),
    notify = () => {};
  const navigate = (route: string) => {
    if (guard()) return false;
    routes.push(route);
    return true;
  };
  const context = {
    ...caseModel,
    caseFieldVisible,
    ...simulation,
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
        'Select',
        'Advanced',
        'Notice',
        'SimulationSteps',
        'ModelSettings',
        'RunPanel',
        'ResultsPage',
        'HistoryPage',
        'HistoryVariablesCard',
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
    invoke: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      switch (command) {
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
        case 'scan_sites':
          return options.scan ? options.scan() : { sites: scannedSites };
        case 'pick_file':
          return scannedSites[0]?.site_file ?? null;
        case 'model_case_context':
          return { entry: { dir: '/preview/site' }, profile: {} };
        case 'hist_vars':
          return options.histories ?? [];
        case 'read_text':
          return '';
        case 'read_case':
          return Object.entries(sourceValues).map(([path, value]) => ({ path, value }));
        case 'new_spatial_case':
        case 'new_case':
          if (options.create) return options.create(args);
          if (failCreate) throw new Error('creation failed');
          return {};
        case 'case_creation_cancel':
          return options.cancelCreation?.(args) ?? 0;
        case 'case_data_save':
          if (failCreate) throw new Error('save failed');
          return {};
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
    routes,
    opened,
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
    { form: { ...form, sitedir: '/sites' }, sites: [site], ownData: true, step: 3 },
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
  blocked(p, 'new-next', /计算核心/);
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
  assert.deepEqual(p.routes, ['research']);
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
  assert.equal(contract.fields.find((f) => f.id === 'new-sitedir')?.value, '/bundled/sites');
  await p.click('new-next');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.equal((p.state.newCaseSeed as { ownData: boolean }).ownData, true);
  await p.remount();
  contract = await p.click('new-previous');
  assert.equal(
    contract.fields.find((f) => f.id === 'new-sitedir')?.value,
    '/bundled/sites',
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
  assert.deepEqual(p.routes, ['research']);
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
      kernel: '',
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
    assert.deepEqual(p.routes, ['research']);
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
  for (const key of Object.keys(rangeValues))
    assert.ok(before.fields.find((item) => item.id === `case-${key}`));
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
    dir: '/project/CN-Cng',
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
  assert.deepEqual(p.opened, ['/project/CN-Cng']);
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
    assert.deepEqual(p.opened, ['/project/CN-Cng']);
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
  assert.equal(c.status, 'busy');
  assert.ok(!c.actions.some((a) => a.id === 'new-cancel-creation'));
  assert.ok(!p.elements.some((e) => e.type === 'Button' && e.children.includes('取消创建')));
  finish({ sites: [site] });
  await pending;
  assert.ok(!p.calls.some((call) => call.command === 'case_creation_cancel'));
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
  // 计算核心不预选：选定之前“下一步”仍置灰。
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
