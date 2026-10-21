import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as model from './model';
import * as setup from './scientific-setup';
import { researchVersion } from './research-version';
import { hubMethods } from './hub-methods';
import { appLocale } from '../history/history';
import { schemeVersionName } from './version-tree';
import type { ParameterOverride } from '../../lib/types';
import { compatibleKernel } from '../runs/run-model';
import { effectiveModelFields } from '../cases/case-model';
import { withCatalogLabels } from '../../lib/param-presentation';
import type { UiField, UiAction } from '../../lib/guide';
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function page(
  subgrid?: 'LCT' | 'PFT' | 'PC',
  physics: Record<string, string> = {},
  parameters: ParameterOverride[] = [],
  codeRef: Record<string, unknown> = { kind: 'official' },
) {
  const version = {
    name: 'Official',
    note: '',
    version: { id: 'official', revision: 1 },
    code: codeRef,
    overlay: { physics, parameters, process_files: [], hybrid: {} },
    applicability: { sites: [], land_classes: [] },
  };
  let failedCommand = '';
  let draft = { ...version, id: 'custom', generation: 1 };
  const calls: { command: string; args: Record<string, unknown> }[] = [],
    scrolls: unknown[] = [],
    routes: string[] = [];
  const state = {
    language: 'zh',
    version,
    versionRef: version.version,
    versions: [version],
    simulationReturn: (subgrid ? { route: 'new-case', step: 2, subgrid } : null) as {
      route: 'new-case' | 'case';
      step: 2;
      subgrid?: 'LCT' | 'PFT' | 'PC';
      setup?: setup.SetupRequest;
    } | null,
    newCaseSeed: undefined as { form: Record<string, string> } | undefined,
    selectedCase: undefined as { entry: { dir: string }; profile: { engine: string } } | undefined,
    expert: false,
    kernels: [],
  };
  let guard: (() => string | null) | undefined;
  let cursor = 0,
    effectCursor = 0,
    contract: { fields: UiField[]; actions: UiAction[]; status: string };
  const slots: unknown[] = [],
    registered = new Set<number>(),
    effectDeps = new Map<number, unknown[]>(),
    effects: (() => unknown)[] = [],
    elements: Element[] = [];
  const context = {
    document: {
      querySelector: (selector: string) => {
        assert.equal(selector, '.main-pane');
        return { scrollTo: (position: unknown) => scrolls.push(position) };
      },
    },
    ...model,
    ...setup,
    ScientificSetupCards: 'ScientificSetupCards',
    researchVersion,
    hubMethods,
    appLocale,
    schemeVersionName,
    compatibleKernel,
    effectiveModelFields,
    withCatalogLabels,
    ConfigurationOptions: 'ConfigurationOptions',
    React: {
      createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => {
        const result = { type, props: props ?? {}, children };
        elements.push(result);
        return result;
      },
    },
    ...Object.fromEntries(
      [
        'Button',
        'Card',
        'Field',
        'Input',
        'Select',
        'Notice',
        'Advanced',
        'CodeWorkspace',
        'CodeFiles',
        'StudyPicks',
        'StudyImportDialog',
        'VersionChanges',
        'VersionLibrary',
        'VersionHub',
        'ParameterDialog',
      ].map((k) => [k, k]),
    ),
    IGBP: [],
    USGS: [],
    PFT: [],
    useApp: () => ({
      state,
      update: (patch: object) => Object.assign(state, patch),
      // Like the app: a page's leave guard can refuse the navigation.
      navigate: (route: string) => {
        if (guard?.()) return false;
        routes.push(route);
        return true;
      },
      refreshVersions: async () => {},
    }),
    useNavigationGuard: (g: () => string | null) => {
      guard = g;
    },
    usePageContract: (c: typeof contract) => {
      contract = c;
    },
    useState: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = initial;
      return [
        slots[key],
        (value: unknown) => {
          slots[key] = value;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const key = cursor++;
      if (!(key in slots)) slots[key] = { current: initial };
      return slots[key];
    },
    useEffect: (fn: () => unknown, deps?: unknown[]) => {
      const key = effectCursor++;
      const previous = effectDeps.get(key);
      // Like React: run once, and again whenever a dependency changes.
      if (
        !registered.has(key) ||
        (deps && previous && deps.some((value, index) => !Object.is(value, previous[index])))
      ) {
        registered.add(key);
        effects.push(fn);
      }
      if (deps) effectDeps.set(key, deps);
    },
    invokeJson: async (command: string, args: Record<string, unknown> = {}) => {
      calls.push({ command, args });
      if (command === failedCommand) throw new Error(command + ' failed');
      switch (command) {
        case 'model_create':
          return draft;
        case 'model_save_draft':
          draft = args.draft as typeof draft;
          return { ...draft, generation: draft.generation + 1 };
        case 'model_freeze':
          return { ...draft, version: { id: draft.id, revision: 2 } };
        case 'model_configuration':
          return {
            fields: [],
            process_files: [],
            pft_parameters: [],
            warnings: [],
            context_complete: false,
          };
        case 'parameter_catalog':
          return [];
        case 'model_fields':
          return setup.setupHandledKeys.map((name) => ({
            name,
            kind: name === 'DEF_TUNING_CROP_PLANTING_DAY' ? 'integer' : 'logical',
            values: [],
            section: '',
            derived: false,
            default: '.false.',
          }));
        case 'model_drafts':
          return [draft];
        case 'history_list':
          return { items: [] };
        case 'model_draft':
          return draft;
        case 'model_freeze_check':
          return {
            ready: false,
            items: [
              { code: 'clean', ok: true, blocking: false, message: '源码干净' },
              {
                code: 'engine_built',
                ok: false,
                blocking: true,
                message: '先在当前提交上编译 Rust 引擎',
              },
            ],
          };
        default:
          throw Error(command);
      }
    },
  };
  const source = readFileSync(new URL('./VersionsPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return VersionsPage;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    effectCursor = 0;
    elements.length = 0;
    Component();
    return contract;
  }
  // 版本页先显示版本库；这些测试看的是一个版本的工作台，所以像用户一样先打开模拟正在用的那个版本。
  let opened = false;
  async function mount() {
    render();
    effects.splice(0).forEach((f) => f());
    for (let i = 0; i < 12; i++) await Promise.resolve();
    if (!opened) {
      opened = true;
      await render()
        .actions.find(
          (a) => a.id === `version-select-${version.version.id}-${version.version.revision}`,
        )
        ?.run();
    }
    effects.splice(0).forEach((f) => f());
    for (let i = 0; i < 12; i++) await Promise.resolve();
    return render();
  }
  function card() {
    return elements.find((e) => e.type === 'ScientificSetupCards')!;
  }
  function choose(id: string, value: string) {
    const p = card().props;
    if (p.disabled || !p.onChange) throw new Error('当前版本不可修改');
    (p.onChange as (overlay: unknown) => void)(
      setup.applySetupRequest(
        p.overlay as Parameters<typeof setup.applySetupRequest>[0],
        setup.setupHandledKeys.map((name) => ({
          name,
          kind: name === 'DEF_TUNING_CROP_PLANTING_DAY' ? 'integer' : 'logical',
          derived: false,
          default: '.false.',
          values: [],
          section: '',
        })),
        { id, value },
        p.environment as setup.SetupEnvironment,
      ),
    );
    return render();
  }
  return {
    render,
    mount,
    elements,
    state,
    calls,
    routes,
    version,
    card,
    choose,
    scrolls,
    fail: (command: string) => {
      failedCommand = command;
    },
    settle: async () => {
      for (let i = 0; i < 24; i++) await Promise.resolve();
      return render();
    },
  };
}
for (const mode of ['PFT', 'PC', 'LCT'] as const)
  test(`${mode} legacy return context cannot create a vegetation revision`, async () => {
    const p = page(mode),
      c = await p.mount();
    assert.equal(
      c.actions.some((a) => a.id === 'version-apply-simulation-structure'),
      false,
    );
    assert.equal(
      p.elements.some((e) => e.type === 'ScientificSetupCards'),
      false,
    );
    assert.ok(c.actions.find((a) => a.id === 'version-return-simulation')?.enabled);
    assert.ok(!p.calls.some((c) => c.command === 'model_create'));
  });
test('saved drafts can return while unsaved model edits keep the return button locked', async () => {
  const p = page('PFT');
  let c = await p.mount();
  await c.actions.find((a) => a.id === 'version-draft-custom')!.run();
  c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'version-return-simulation')?.enabled, true);
  c.fields.find((f) => f.id === 'draft-name')!.set('changed draft');
  c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'version-return-simulation')?.enabled, false);
  await c.actions.find((a) => a.id === 'version-save')!.run();
  c = p.render();
  assert.equal(c.actions.find((a) => a.id === 'version-return-simulation')?.enabled, true);
  await c.actions.find((a) => a.id === 'version-return-simulation')!.run();
  assert.deepEqual(p.routes, ['new-case']);
  assert.equal(p.state.versionRef.id, 'official');
  assert.ok(!p.calls.some((c) => c.command === 'model_freeze'));
});

test('research configuration retains the engine of a new or existing simulation', async () => {
  for (const origin of ['new', 'existing']) {
    const h = page('LCT');
    if (origin === 'new') h.state.newCaseSeed = { form: { engine: 'fortran', kind: 'site' } };
    else {
      h.state.simulationReturn = { route: 'case', step: 2 };
      h.state.selectedCase = { entry: { dir: '/case' }, profile: { engine: 'fortran' } };
    }
    await h.mount();
    const config = h.elements.find((e) => e.type === 'ConfigurationOptions')!;
    assert.equal(config.props.engine, 'fortran');
  }
});

test('a new version can copy its parent code into a workspace of its own', async () => {
  const p = page('PFT');
  let c = await p.mount();
  const set = (id: string, value: string) => {
    c.fields.find((field) => field.id === id)!.set(value);
    c = p.render();
  };
  set('version-name', 'Soil change');
  set('version-note', 'New soil evaporation scheme');
  assert.ok(
    !c.fields.some((field) => field.id === 'version-workspace'),
    'no workspace box unless chosen',
  );
  set('version-code', 'branch');
  await c.actions.find((action) => action.id === 'version-create')!.run();
  const call = p.calls.find((entry) => entry.command === 'model_create')!;
  assert.equal(call.args.branchCode, true);
  assert.equal(call.args.workspace, null);
  assert.deepEqual(
    c.fields.find((field) => field.id === 'version-code')!.options!.map((o) => o.value),
    ['inherit', 'branch'],
    'an existing workspace is chosen from the draft’s code tab, not when creating',
  );
});

test('a draft on its own code shows what blocks freezing and cannot be frozen until it is fixed', async () => {
  const p = page(undefined, {}, [], { kind: 'workspace', workspace: 'ver-1', commit: 'abc' });
  let c = await p.mount();
  await c.actions.find((a) => a.id === 'version-draft-custom')!.run();
  c = await p.mount();
  assert.ok(p.calls.some((call) => call.command === 'model_freeze_check'));
  assert.equal(c.actions.find((a) => a.id === 'version-freeze')?.enabled, false);
  const texts = p.elements.flatMap((e) =>
    ((e as { children?: unknown[] }).children ?? []).filter((x) => typeof x === 'string'),
  );
  assert.ok(texts.includes('先在当前提交上编译 Rust 引擎'));
});

test('the version page shows one version in tabs that hide panels without unmounting them', async () => {
  const p = page('PFT');
  await p.mount();
  const tabs = p.elements.filter((e) => e.props.role === 'tab');
  assert.equal(tabs.length, 4);
  assert.equal(tabs.filter((e) => e.props['aria-selected']).length, 1, 'one tab is current');
  const panels = p.elements.filter(
    (e) => e.type === 'div' && e.props.className === 'versions-panel',
  );
  assert.equal(panels.length, 4);
  assert.equal(panels.filter((e) => !e.props.hidden).length, 1, 'only the current panel shows');
  // Hidden panels still render: their page contracts and unsaved edits stay.
  assert.ok(p.elements.some((e) => e.type === 'ConfigurationOptions'));
  assert.ok(p.elements.some((e) => e.type === 'CodeWorkspace'));
  assert.ok(p.elements.some((e) => e.type === 'ParameterDialog'));
  assert.ok(!p.elements.some((e) => e.type === 'VersionLibrary'), 'the library is its own page');
});

test('新建修改版 offers six ways and starts each without a confirmation form', async () => {
  const p = page();
  const c = await p.mount();
  const ids = c.actions.filter((a) => a.id.startsWith('version-method-')).map((a) => a.id);
  assert.deepEqual(ids, [
    'version-method-manual',
    'version-method-tuning',
    'version-method-uncertainty',
    'version-method-parameters',
    'version-method-process',
    'version-method-source',
  ]);
  // By hand: a draft is created straight away, named automatically, and the parameter dialog opens.
  await c.actions.find((a) => a.id === 'version-method-manual')!.run();
  await p.settle();
  const create = p.calls.find((call) => call.command === 'model_create')!;
  assert.equal(create.args.branchCode, false);
  assert.match(String(create.args.name), /^手动修改参数 /);
  assert.ok(String(create.args.note).includes('基于 Official r1'));
  const dialog = p.elements.find((e) => e.type === 'ParameterDialog')!;
  assert.equal(dialog.props.open, true);
  // A study method goes to the research page with that method chosen.
  const q = page();
  const d = await q.mount();
  await d.actions.find((a) => a.id === 'version-method-tuning')!.run();
  assert.equal(q.routes.at(-1), 'research');
  assert.equal((q.state as { researchMethod?: string }).researchMethod, 'tuning');
});

test('the research home starts 修改参数（手动） and 修改源码 here, once per click', async () => {
  const p = page();
  await p.mount();
  const creates = () => p.calls.filter((call) => call.command === 'model_create');
  const manual = { method: 'manual' as const };
  Object.assign(p.state, { versionMethod: manual });
  await p.mount();
  assert.equal(creates().length, 1);
  assert.equal(creates()[0]!.args.branchCode, false);
  assert.equal((p.state as { versionMethod?: unknown }).versionMethod, null);
  assert.equal(p.elements.find((e) => e.type === 'ParameterDialog')!.props.open, true);
  // Development mode runs the effect twice with the same request: no second, empty draft.
  Object.assign(p.state, { versionMethod: manual });
  await p.mount();
  assert.equal(creates().length, 1);
  // Back goes where the user came from: research home, not 新建修改版.
  const back = p.elements.find(
    (e) => e.type === 'button' && e.children.flat().includes('研究首页'),
  )!;
  (back.props.onClick as () => void)();
  await p.settle();
  assert.equal(p.routes.at(-1), 'research');
  Object.assign(p.state, { versionMethod: { method: 'source' } });
  await p.mount();
  assert.equal(creates().length, 2);
  assert.equal(creates()[1]!.args.branchCode, true);
});

test('closing the parameter table of a fresh hand-edit draft without changes cancels it', async () => {
  const dialog = (p: ReturnType<typeof page>) =>
    p.elements.find((e) => e.type === 'ParameterDialog')!.props as {
      open: boolean;
      onOpenChange: (open: boolean) => void;
      onChange: (next: unknown) => void;
      overlay: Record<string, unknown>;
    };
  const deletes = (p: ReturnType<typeof page>) =>
    p.calls.filter((call) => call.command === 'model_delete_draft').length;
  // From research home: back to research home, and the empty draft is gone.
  const r = page();
  await r.mount();
  Object.assign(r.state, { versionMethod: { method: 'manual' } });
  await r.mount();
  dialog(r).onOpenChange(false);
  await r.settle();
  assert.equal(deletes(r), 1);
  assert.equal(r.routes.at(-1), 'research');
  // From 新建修改版: back to the six ways on the same page.
  const h = page();
  const c = await h.mount();
  await c.actions.find((a) => a.id === 'version-method-manual')!.run();
  await h.settle();
  dialog(h).onOpenChange(false);
  await h.settle();
  assert.equal(deletes(h), 1);
  assert.deepEqual(h.routes, []);
  assert.ok(h.elements.some((el) => el.type === 'VersionHub'));
  // Something was changed: closing keeps the draft open for saving and freezing.
  const e = page();
  const d = await e.mount();
  await d.actions.find((a) => a.id === 'version-method-manual')!.run();
  await e.settle();
  dialog(e).onChange({ ...dialog(e).overlay, physics: { DEF_USE_PLANTHYDRAULICS: '.true.' } });
  e.render();
  dialog(e).onOpenChange(false);
  await e.settle();
  assert.equal(deletes(e), 0);
  assert.ok(!e.elements.some((el) => el.type === 'VersionHub'));
  // Opening the table again from the draft and closing it leaves the draft alone as well.
  dialog(e).onOpenChange(true);
  dialog(e).onOpenChange(false);
  await e.settle();
  assert.equal(deletes(e), 0);
});

test('a draft offers importing a study only on its research tab', async () => {
  const p = page();
  const c = await p.mount();
  await c.actions.find((a) => a.id === 'version-draft-custom')!.run();
  await p.mount();
  const imports = p.elements.filter(
    (e) => e.type === 'Button' && e.children.flat().includes('从研究导入最优参数…'),
  );
  assert.equal(imports.length, 1);
});
