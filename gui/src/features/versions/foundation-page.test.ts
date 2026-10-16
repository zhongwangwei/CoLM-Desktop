import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as model from './model';
import * as setup from './scientific-setup';
import { researchVersion } from './research-version';
import { schemeVersionName } from './version-tree';
import type { ParameterOverride } from '../../lib/types';
import { compatibleKernel } from '../runs/run-model';
import { effectiveModelFields } from '../cases/case-model';
import type { UiField, UiAction } from '../../lib/guide';
type Element = { type: string; props: Record<string, unknown>; children: unknown[] };
function page(
  subgrid?: 'LCT' | 'PFT' | 'PC',
  physics: Record<string, string> = {},
  parameters: ParameterOverride[] = [],
) {
  const version = {
    name: 'Official',
    note: '',
    version: { id: 'official', revision: 1 },
    code: { kind: 'official' },
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
  let cursor = 0,
    effectCursor = 0,
    contract: { fields: UiField[]; actions: UiAction[]; status: string };
  const slots: unknown[] = [],
    registered = new Set<number>(),
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
    schemeVersionName,
    compatibleKernel,
    effectiveModelFields,
    ConfigurationOptions: 'ConfigurationOptions',
    React: {
      createElement: (type: string, props: Record<string, unknown>, ...children: unknown[]) => {
        const result = { type, props: props ?? {}, children };
        elements.push(result);
        return result;
      },
    },
    ...Object.fromEntries(
      ['Button', 'Card', 'Field', 'Input', 'Select', 'Notice', 'Advanced', 'CodeWorkspace'].map(
        (k) => [k, k],
      ),
    ),
    IGBP: [],
    USGS: [],
    PFT: [],
    useApp: () => ({
      state,
      update: (patch: object) => Object.assign(state, patch),
      navigate: (route: string) => {
        routes.push(route);
        return true;
      },
      refreshVersions: async () => {},
    }),
    useNavigationGuard: () => {},
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
    useEffect: (fn: () => unknown) => {
      const key = effectCursor++;
      if (!registered.has(key)) {
        registered.add(key);
        effects.push(fn);
      }
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
  async function mount() {
    render();
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
