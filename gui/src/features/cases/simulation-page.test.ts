import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as caseModel from './case-model';
import * as simulation from './simulation-model';
import * as modelSettings from './model-settings';
import type { UiField, UiAction } from '../../lib/guide';
import { pageExecutionContext, publicUiResult, waitForContract } from '../../lib/utils';
import { fieldLabel, optionLabel } from '../../lib/param-presentation';

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
    schema?: caseModel.SchemaField[];
    configuration?: (args: Record<string, unknown>) => Promise<unknown>;
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
    kernels: [],
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
    ...simulation,
    ...modelSettings,
    fieldLabel,
    optionLabel,
    compatibleKernel: () => null,
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
        'ProcessDataGroups',
        'ModelSettings',
        'RunPanel',
        'ResultsPage',
        'HistoryPage',
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
          return options.configuration ? options.configuration(args) : { fields: [] };
        case 'model_fields':
          return [];
        case 'scan_sites':
          return { sites: scannedSites };
        case 'pick_file':
          return scannedSites[0]?.site_file ?? null;
        case 'model_case_context':
          return { entry: { dir: '/preview/site' }, profile: {} };
        case 'read_text':
          return '';
        case 'read_case':
          return Object.entries(sourceValues).map(([path, value]) => ({ path, value }));
        case 'new_spatial_case':
        case 'new_case':
          if (failCreate) throw new Error('creation failed');
          return {};
        case 'case_data_save':
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
    element,
    remount,
    guard: () => guard(),
    fail: () => {
      failCreate = true;
    },
  };
}
const form = {
  root: '/project',
  name: 'CN-Cng',
  kind: 'site',
  landcover: 'IGBP',
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
  c = p.set('new-root', '/another-project');
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

test('five-step creation defers dates until data, exposes model settings, and sends only edited case options', async () => {
  const p = page('NewCasePage');
  await p.mount();
  p.set('new-name', 'CN-Cng');
  let c = await p.click('new-next');
  assert.equal(c.details.step, 2);
  assert.equal(typeof p.element('ModelSettings').props.onEdit, 'function');
  c = await p.click('new-next');
  assert.equal(c.details.step, 3, 'model setup does not require dates before the data step');
  await assert.rejects(p.click('new-next'), /时段/);
  assert.equal(p.render().details.step, 3);
  for (const [key, value] of Object.entries(form))
    if (p.render().fields.some((f) => f.id === `new-${key}`)) p.set(`new-${key}`, value);
  c = await p.click('new-next');
  assert.equal(c.details.step, 4);
  p.set('new-option-DEF_HIST_FREQ', 'DAILY');
  c = await p.click('new-next');
  assert.equal(c.details.step, 5);
  assert.equal(c.actions.find((a) => a.id === 'new-create')?.kind, 'commit');
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case');
  assert.ok(request);
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
  await assert.rejects(p.click('new-next'), /覆盖/);
  p.set('new-end', '2001-12-31');
  await p.click('new-next');
  await p.click('new-next');
  p.fail();
  await assert.rejects(p.click('new-create'), /creation failed/);
  assert.deepEqual(p.opened, []);
  assert.ok(!p.calls.some((call) => /run_start|run_case|submit/.test(call.command)));
});
test('model editor round trip retains form, scanned sites, option edits and the model second step', async () => {
  const option = { DEF_HIST_FREQ: 'DAILY' },
    p = page('NewCasePage', { form, sites: [site], caseValues: option, step: 2 });
  await p.mount();
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.deepEqual(p.routes, ['versions']);
  const seed = p.state.newCaseSeed as {
    form: typeof form;
    sites: unknown[];
    caseValues: typeof option;
    step: number;
  };
  assert.equal(seed.form.start, form.start);
  assert.deepEqual(seed.sites, [site]);
  assert.deepEqual(seed.caseValues, option);
  assert.equal(seed.step, 2);
  let c = await p.remount();
  assert.equal(c.details.step, 2);
  await p.click('new-next');
  c = await p.click('new-next');
  assert.equal(c.fields.find((f) => f.id === 'new-option-DEF_HIST_FREQ')?.value, 'DAILY');
});
test('typing 2001 dates keeps partial input visible, blocks save, and preserves seconds and spinup', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.click('case-step-3');
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
  await p.click('case-step-4');
  p.set('case-DEF_HIST_FREQ', 'DAILY');
  await p.click('case-step-2');
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
test('creation preserves explicitly edited spinup seconds instead of resetting them to midnight', async () => {
  const p = page('NewCasePage', {
    form: { ...form, spinup_end: '2001-06-01' },
    sites: [site],
    step: 4,
  });
  await p.mount();
  p.set('new-advanced', 'true');
  p.set('new-option-DEF_simulation_time%spinup_sec', '3600');
  await p.click('new-next');
  await p.click('new-create');
  const request = p.calls.find((c) => c.command === 'new_case');
  assert.ok(request);
  const fields = request.args.fields as { path: string; value: string }[];
  const seconds = fields.filter((f) => f.path === 'DEF_simulation_time%spinup_sec');
  assert.ok(seconds.length);
  assert.ok(
    seconds.every((f) => f.value === '3600'),
    'every sent spinup seconds value must retain the user edit',
  );
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
    if (name === 'CasesPage') await p.click('case-step-3');
    const guide = guideFor(p),
      prefix = name === 'NewCasePage' ? 'new-option-' : 'case-';
    await guide.fill(`${prefix}DEF_dir_existing_srfdata`, '/custom surface/path');
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_dir_existing_srfdata`)?.value,
      '/custom surface/path',
    );
    if (name === 'NewCasePage') await p.click('new-next');
    else await p.click('case-save');
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
  await p.click('case-step-4');
  p.set('case-spinup-date', '2030-01-01');
  await assert.rejects(p.click('case-save'), /预热|Spinup/i);
  assert.equal(p.calls.filter((call) => call.command === 'case_data_save').length, 0);
  p.set('case-spinup-date', '');
  await p.click('case-save');
  const request = p.calls.find((call) => call.command === 'case_data_save');
  assert.ok(request);
  assert.deepEqual(request.args.fields, [{ path: 'DEF_simulation_time%spinup_year', value: '0' }]);
  assert.equal(p.render().details.simulation_step, 5);
});
test('moving simulation start beyond the existing spinup cutoff refuses to save unchanged invalid spinup', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.click('case-step-3');
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
    step: 4,
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
  await p.click('new-next');
  await p.click('new-create');
  const request = p.calls.find((call) => call.command === 'new_case');
  assert.ok(request);
  const fields = request.args.fields as { path: string; value: string }[];
  assert.ok(
    !fields.some((f) => ['DEF_HIST_FREQ', 'DEF_WRST_FREQ', 'DEF_dir_output'].includes(f.path)),
    'automatic defaults and stale output overrides are not submitted',
  );
});
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} shows the managed output directory and refuses assistant changes`, async () => {
    const p = page(
      name,
      name === 'NewCasePage'
        ? { form: { ...form, name: '  CN-Cng  ' }, sites: [site], step: 4 }
        : undefined,
    );
    await p.mount();
    if (name === 'CasesPage') await p.click('case-step-4');
    const id = name === 'NewCasePage' ? 'new-output-directory' : 'case-DEF_dir_output',
      expected = name === 'NewCasePage' ? '/project/CN-Cng/out' : '/project/site/out';
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
    step: 4,
  });
  await p.mount();
  const guide = guideFor(p);
  await guide.fill('new-option-DEF_simulation_time%timestep', '900');
  await guide.fill('new-option-DEF_simulation_time%timestep', '');
  await p.click('new-next');
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

test('model round trips preserve explicit own-data disclosure rather than inferring it from bundled directories', async () => {
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
  await p.click('new-previous');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.equal((p.state.newCaseSeed as { ownData: boolean }).ownData, false);
  await p.remount();
  contract = await p.click('new-next');
  assert.ok(
    !contract.fields.some((f) => f.id === 'new-sitedir'),
    'returning from model settings keeps bundled data collapsed',
  );
  contract = await p.click('new-own-data');
  assert.equal(contract.fields.find((f) => f.id === 'new-sitedir')?.value, '/bundled/sites');
  await p.click('new-previous');
  (p.element('ModelSettings').props.onEdit as () => unknown)();
  assert.equal((p.state.newCaseSeed as { ownData: boolean }).ownData, true);
  await p.remount();
  contract = await p.click('new-next');
  assert.equal(
    contract.fields.find((f) => f.id === 'new-sitedir')?.value,
    '/bundled/sites',
    'an explicitly opened own-data section remains open after model editing',
  );
});

test('USGS classification survives model editing and creates the chosen template without fake physics switches', async () => {
  for (const urban of [false, true]) {
    const p = page('NewCasePage', { form, sites: [{ ...site, urban }], step: 2 });
    if (urban) Object.assign(p.state.version.overlay.physics, { DEF_URBAN_RUN: '.true.' });
    await p.mount();
    assert.equal(p.element('ModelSettings').props.classification, 'IGBP');
    (p.element('ModelSettings').props.onClassificationChange as (value: string) => void)('USGS');
    assert.equal(p.render().details.classification, 'USGS');
    (p.element('ModelSettings').props.onEdit as () => void)();
    assert.deepEqual(p.state.simulationReturn, { route: 'new-case', step: 2 });
    assert.equal((p.state.newCaseSeed as { form: { landcover: string } }).form.landcover, 'USGS');
    const returned = await p.remount();
    assert.equal(returned.details.step, 2);
    assert.equal(p.element('ModelSettings').props.classification, 'USGS');
    await p.click('new-next');
    await p.click('new-next');
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
    assert.deepEqual(p.state.version.overlay.physics, urban ? { DEF_URBAN_RUN: '.true.' } : {});
  }
});
test('existing case returns from version editing to the model second step before visiting data', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.click('case-step-2');
  (p.element('ModelSettings').props.onEdit as () => void)();
  assert.deepEqual(p.routes, ['versions']);
  assert.deepEqual(p.state.simulationReturn, { route: 'case', step: 2 });
  const returned = await p.remount();
  assert.equal(returned.details.simulation_step, 2);
  assert.ok(p.element('ModelSettings'));
  const data = await p.click('case-next');
  assert.equal(data.details.simulation_step, 3);
  assert.ok(data.fields.some((field) => field.id === 'case-start-date'));
});
test('existing case can reach its data step to repair dates after first reviewing model settings', async () => {
  const p = page('CasesPage', undefined, [site], {
    ...initialValues,
    'DEF_simulation_time%start_year': '',
  });
  await p.mount();
  await p.click('case-step-2');
  const data = await p.click('case-next');
  assert.equal(data.details.simulation_step, 3);
  await assert.rejects(p.click('case-next'), /时段/);
  assert.equal(p.render().details.simulation_step, 3);
});

const processSchema = [
  ...schema,
  { ...field('DEF_USE_CN_INIT', '初始场', '.false.'), kind: 'Logical' },
  { ...field('DEF_file_cn_init', '初始场', 'null'), kind: 'Character' },
  { ...field('DEF_UnitCatchment_file', '河道与水库', 'null'), kind: 'Character' },
  { ...field('DEF_TRACER_SOIL_INIT_FILE', '初始场', 'null'), kind: 'Character' },
  { ...field('DEF_URBAN_type_scheme', '地表数据', '1'), kind: 'Integer' },
];
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} queries applicability using the exact frozen revision and current unsaved case data`, async () => {
    const draft = { DEF_USE_CN_INIT: '.true.', DEF_file_cn_init: '/unsaved-cn.nc' };
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3, caseValues: draft } : undefined,
      [site],
      { ...initialValues, ...draft },
      { schema: processSchema },
    );
    await p.mount();
    if (name === 'CasesPage') {
      await p.click('case-step-3');
      await p.settle();
      assert.deepEqual(
        p.calls.filter((call) => call.command === 'model_configuration').at(-1)?.args.caseValues,
        {},
      );
      await p.click('case-data-group-bgc');
      await guideFor(p).fill('case-DEF_file_cn_init', '/changed-cn.nc');
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

function cnSnapshot(mode = 'enabled', reason: string | null = null) {
  return {
    fields: [
      {
        name: 'DEF_file_cn_init',
        mode,
        configured_value: '/retained-cn.nc',
        status: 'configured',
        pending: false,
        reason,
        allowed_values: [],
      },
    ],
  };
}
const dataGroups = [
  ['bgc', 'DEF_USE_BGC', 'DEF_file_cn_init'],
  ['river', 'DEF_USE_GridRiverLakeFlow', 'DEF_UnitCatchment_file'],
  ['tracer', 'DEF_USE_TRACER', 'DEF_TRACER_SOIL_INIT_FILE'],
  ['urban', 'DEF_URBAN_RUN', 'DEF_URBAN_type_scheme'],
] as const;
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} shows process data only in opened groups and retains edits when groups close`, async () => {
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      { schema: processSchema },
    );
    await p.mount();
    if (name === 'CasesPage') await p.click('case-step-3');
    const prefix = name === 'NewCasePage' ? 'new-option-' : 'case-',
      groupPrefix = name === 'NewCasePage' ? 'new-data-group-' : 'case-data-group-';
    for (const [id, , fieldName] of dataGroups) {
      assert.ok(
        p.render().actions.some((a) => a.id === `${groupPrefix}${id}`),
        'each data group remains discoverable',
      );
      assert.ok(
        !p.render().fields.some((f) => f.id === `${prefix}${fieldName}`),
        'closed groups do not expose invisible controls',
      );
      await p.click(`${groupPrefix}${id}`);
      assert.ok(p.render().fields.some((f) => f.id === `${prefix}${fieldName}`));
    }
    await guideFor(p).fill(`${prefix}DEF_file_cn_init`, '/keep-cn.nc');
    await p.settle();
    await p.click(`${groupPrefix}bgc`);
    await assert.rejects(
      guideFor(p).fill(`${prefix}DEF_file_cn_init`, '/hidden-change.nc'),
      /unique visible control/,
    );
    await p.click(`${groupPrefix}bgc`);
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.value,
      '/keep-cn.nc',
    );
  });
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} opens enabled parent groups by default and recomputes CN input gates before saving`, async () => {
    const p = page(
      name,
      name === 'NewCasePage'
        ? { form, sites: [site], step: 3, caseValues: { DEF_file_cn_init: '/retained-cn.nc' } }
        : undefined,
      [site],
      { ...initialValues, DEF_USE_CN_INIT: '.false.', DEF_file_cn_init: '/retained-cn.nc' },
      {
        schema: processSchema,
        configuration: async (args) =>
          cnSnapshot(
            (args.caseValues as Record<string, string>).DEF_USE_CN_INIT === '.true.'
              ? 'enabled'
              : 'disabled',
            'CN initial state is off',
          ),
      },
    );
    Object.assign(
      p.state.version.overlay.physics,
      Object.fromEntries(dataGroups.map(([, parent]) => [parent, '.true.'])),
    );
    await p.mount();
    if (name === 'CasesPage') await p.click('case-step-3');
    const prefix = name === 'NewCasePage' ? 'new-option-' : 'case-';
    for (const [, , fieldName] of dataGroups)
      assert.ok(p.render().fields.some((f) => f.id === `${prefix}${fieldName}`));
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.disabled,
      true,
    );
    await assert.rejects(
      guideFor(p).fill(`${prefix}DEF_file_cn_init`, '/blocked.nc'),
      /cannot be changed/,
    );
    const render = p.element('ProcessDataGroups').props.render as (
      field: caseModel.SchemaField,
    ) => Element;
    const displayed = render(processSchema.find((f) => f.name === 'DEF_file_cn_init')!);
    assert.ok(
      JSON.stringify(displayed.props).includes('CN initial state is off'),
      'disabled reason is shown next to the retained field',
    );
    await guideFor(p).fill(`${prefix}DEF_USE_CN_INIT`, '.true.');
    await p.settle();
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.disabled,
      false,
    );
    assert.equal(
      (
        p.calls.filter((c) => c.command === 'model_configuration').at(-1)?.args
          .caseValues as Record<string, string>
      ).DEF_USE_CN_INIT,
      '.true.',
    );
    await guideFor(p).fill(`${prefix}DEF_USE_CN_INIT`, '.false.');
    await p.settle();
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.value,
      '/retained-cn.nc',
    );
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.disabled,
      true,
    );
    assert.ok(
      !p.calls.some((c) => c.command === 'case_data_save' || c.command === 'new_case'),
      'conditions update without saving or creating',
    );
  });
for (const name of ['NewCasePage', 'CasesPage'] as const)
  test(`${name} discards delayed condition replies after newer unsaved inputs and version changes`, async () => {
    const waiting: { args: Record<string, unknown>; resolve: (value: unknown) => void }[] = [];
    const p = page(
      name,
      name === 'NewCasePage' ? { form, sites: [site], step: 3 } : undefined,
      [site],
      initialValues,
      {
        schema: processSchema,
        configuration: (args) => new Promise((resolve) => waiting.push({ args, resolve })),
      },
    );
    Object.assign(p.state.version.overlay.physics, { DEF_USE_BGC: '.true.' });
    await p.mount();
    if (name === 'CasesPage') await p.click('case-step-3');
    const prefix = name === 'NewCasePage' ? 'new-option-' : 'case-';
    assert.ok(waiting.length);
    waiting.at(-1)!.resolve(cnSnapshot());
    await p.settle();
    await guideFor(p).fill(`${prefix}DEF_USE_CN_INIT`, '.true.');
    await p.settle();
    const older = waiting.at(-1)!;
    await guideFor(p).fill(`${prefix}DEF_USE_CN_INIT`, '.false.');
    await p.settle();
    const newer = waiting.at(-1)!;
    assert.notEqual(older, newer);
    newer.resolve(cnSnapshot('readonly', 'latest inputs are read-only'));
    await p.settle();
    older.resolve(cnSnapshot('enabled'));
    await p.settle();
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.disabled,
      true,
      'late enabled reply cannot undo current readonly input gate',
    );
    await guideFor(p).fill(`${prefix}DEF_USE_CN_INIT`, '.true.');
    await p.settle();
    const oldVersion = waiting.at(-1)!;
    p.state.versionRef = { id: 'official', revision: 5 };
    p.state.version = { ...p.state.version, version: p.state.versionRef };
    p.render();
    await p.settle();
    const currentVersion = waiting.at(-1)!;
    assert.equal(
      (currentVersion.args.source as { version: { revision: number } }).version.revision,
      5,
    );
    currentVersion.resolve(cnSnapshot('disabled', 'current revision disallows CN'));
    await p.settle();
    oldVersion.resolve(cnSnapshot('enabled'));
    await p.settle();
    assert.equal(
      p.render().fields.find((f) => f.id === `${prefix}DEF_file_cn_init`)?.disabled,
      true,
      'old revision cannot replace the current revision gate',
    );
  });
test('existing case run and result tabs never expose settings-only process data actions', async () => {
  const p = page('CasesPage', undefined, [site], initialValues, { schema: processSchema });
  await p.mount();
  await p.click('case-step-3');
  assert.ok(p.render().actions.some((a) => a.id === 'case-data-group-bgc'));
  for (const tab of ['runs', 'results']) {
    await p.click(`case-tab-${tab}`);
    assert.ok(!p.render().actions.some((a) => a.id.startsWith('case-data-group-')));
    assert.deepEqual(p.render().fields, []);
    await p.click('case-tab-settings');
  }
});

for (const mode of ['PFT', 'PC'] as const)
  test(`${mode} entry requests a frozen model change while preserving case inputs and classification`, async () => {
    const p = page('NewCasePage', {
      form: { ...form, landcover: 'USGS' },
      sites: [site],
      caseValues: { DEF_HIST_FREQ: 'DAILY' },
      step: 2,
    });
    await p.mount();
    (p.element('ModelSettings').props.onModeChange as (mode: string) => void)(mode);
    p.render();
    assert.deepEqual(p.routes, ['versions']);
    assert.deepEqual(p.state.simulationReturn, { route: 'new-case', step: 2, subgrid: mode });
    const seed = p.state.newCaseSeed as { form: typeof form; caseValues: Record<string, string> };
    assert.equal(seed.form.landcover, 'USGS');
    assert.equal(seed.form.start, form.start);
    assert.equal(seed.caseValues.DEF_HIST_FREQ, 'DAILY');
    assert.deepEqual(
      p.state.version.overlay.physics,
      {},
      'entry cannot mutate the frozen model or merely change the case template',
    );
    assert.ok(!p.calls.some((c) => c.command === 'new_case' || c.command === 'model_create'));
    Object.assign(p.state.version.overlay.physics, {
      DEF_USE_LCT: '.false.',
      DEF_USE_PFT: mode === 'PFT' ? '.true.' : '.false.',
      DEF_USE_PC: mode === 'PC' ? '.true.' : '.false.',
    });
    await p.remount();
    assert.equal(p.element('ModelSettings').props.mode, mode);
    assert.equal(p.element('ModelSettings').props.classification, 'USGS');
    (p.element('ModelSettings').props.onModeChange as (mode: string) => void)(mode);
    assert.equal(p.routes.length, 1, 'same structure needs no repeated editor round trip');
    await p.click('new-next');
    await p.click('new-next');
    await p.click('new-next');
    await p.click('new-create');
    const request = p.calls.find((c) => c.command === 'new_case');
    assert.ok(request);
    assert.equal(request.args.mode, 'usgs', 'USGS kernel identity must survive PFT/PC structure');
    const fields = request.args.fields as { path: string; value: string }[];
    assert.equal(fields.find((f) => f.path === `DEF_USE_${mode}`)?.value, '.true.');
  });
test('choosing a land-class card from PFT requests LCT without silently disabling BGC', async () => {
  const p = page('NewCasePage', { form, sites: [site], step: 2 });
  Object.assign(p.state.version.overlay.physics, {
    DEF_USE_LCT: '.false.',
    DEF_USE_PFT: '.true.',
    DEF_USE_PC: '.false.',
    DEF_USE_BGC: '.true.',
  });
  await p.mount();
  (p.element('ModelSettings').props.onModeChange as (mode: string) => void)('USGS');
  p.render();
  assert.deepEqual(p.state.simulationReturn, { route: 'new-case', step: 2, subgrid: 'LCT' });
  assert.equal((p.state.newCaseSeed as { form: typeof form }).form.landcover, 'USGS');
  assert.equal((p.state.version.overlay.physics as Record<string, string>).DEF_USE_BGC, '.true.');
  assert.equal((p.state.version.overlay.physics as Record<string, string>).DEF_USE_PFT, '.true.');
});

test('a soil-card request carries the ordered setup intent and preserves the complete pending case form', async () => {
  const p = page('NewCasePage', { form: { ...form, landcover: 'USGS' }, sites: [site], step: 2 });
  await p.mount();
  const model = p.element('ModelSettings');
  assert.equal((model.props.setupEnvironment as { domain: string }).domain, 'site');
  (model.props.onSetupRequest as (request: { id: string; value: string }) => void)({
    id: 'soil',
    value: 'vG',
  });
  p.render();
  assert.deepEqual(p.state.simulationReturn, {
    route: 'new-case',
    step: 2,
    setup: { id: 'soil', value: 'vG' },
  });
  assert.deepEqual(p.routes, ['versions']);
  assert.equal((p.state.newCaseSeed as { form: typeof form }).form.landcover, 'USGS');
  assert.deepEqual(p.state.version.overlay.physics, {});
});
test('existing case setup-card request goes through its model revision without saving case data', async () => {
  const p = page('CasesPage');
  await p.mount();
  await p.click('case-step-2');
  (
    p.element('ModelSettings').props.onSetupRequest as (request: {
      id: string;
      value: string;
    }) => void
  )({ id: 'soil', value: 'vG' });
  assert.deepEqual(p.state.simulationReturn, {
    route: 'case',
    step: 2,
    setup: { id: 'soil', value: 'vG' },
  });
  assert.deepEqual(p.routes, ['versions']);
  assert.ok(!p.calls.some((c) => c.command === 'case_data_save'));
});
