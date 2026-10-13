import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { bilingual } from '../../lib/bilingual';
import * as setup from './scientific-setup';
import { modelSettingValue } from '../cases/model-settings';
import type { ConfigurationSource, ConfigurationResult } from './configuration-options';
import { caseOptionOverlay, caseOptionPatch, type CaseOptionPatch } from './configuration-options';
import type { Overlay, KernelEntry } from '../../lib/types';
import type { ModelField } from './model';
import type { UiAction, UiField } from '../../lib/guide';
type Contract = { fields: UiField[]; actions: UiAction[]; status: string };
function harness() {
  const overlay: Overlay = {
    physics: {
      DEF_USE_LCT: '.false.',
      DEF_USE_PFT: '.true.',
      DEF_USE_PC: '.false.',
      DEF_USE_BGC: '.false.',
    },
    parameters: [],
    process_files: [],
  };
  const fields = setup.setupHandledKeys.map(
    (name) =>
      ({
        name,
        kind: 'logical',
        derived: false,
        values: [],
        default: '.false.',
        section: '',
      }) as ModelField,
  );
  const changes: Overlay[] = [],
    forward: setup.SetupRequest[] = [];
  const props = {
    caseDir: '/case/a',
    source: { kind: 'draft', id: 'd1', generation: 3, overlay } as ConfigurationSource,
    overlay,
    fields,
    environment: { subgrid: 'PFT', domain: 'single', grid: 'single' } as setup.SetupEnvironment,
    includeSubgrid: true,
    disabled: false,
    onChange: (o: Overlay) => changes.push(o),
    onCaseChange: undefined as undefined | ((patch: CaseOptionPatch) => void | Promise<void>),
  };
  const requests: { args: Record<string, unknown>; resolve: (r: ConfigurationResult) => void }[] =
      [],
    slots: unknown[] = [],
    effects: { deps: unknown[]; cleanup?: () => void }[] = [],
    pending: (() => void)[] = [],
    nodes: { type: string; props: Record<string, unknown>; children: unknown[] }[] = [];
  let cursor = 0,
    ei = 0,
    contract: Contract;
  const context = {
    ...setup,
    caseOptionOverlay,
    caseOptionPatch,
    modelSettingValue,
    bilingual,
    React: {
      createElement: (type: string, p: Record<string, unknown>, ...children: unknown[]) => {
        const node = { type, props: p ?? {}, children };
        nodes.push(node);
        return node;
      },
    },
    ...Object.fromEntries(['Button', 'Card', 'Field', 'Input', 'Notice'].map((k) => [k, k])),
    useApp: () => ({ state: { language: 'zh' } }),
    usePageContract: (c: Contract) => {
      contract = c;
    },
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (v: unknown) => {
          slots[i] = v;
        },
      ];
    },
    useRef: (v: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: v };
      return slots[i];
    },
    useEffect: (fn: () => void | (() => void), deps: unknown[]) => {
      const i = ei++;
      if (!effects[i] || deps.some((d, j) => d !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          effects[i].cleanup = fn() || undefined;
        });
      }
    },
    invokeJson: (_cmd: string, args: Record<string, unknown>) =>
      new Promise<ConfigurationResult>((resolve) => requests.push({ args, resolve })),
  };
  const source = readFileSync(new URL('./ScientificSetupCards.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ScientificSetupCards;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    ei = 0;
    nodes.length = 0;
    Component(props);
    pending.splice(0).forEach((fn) => fn());
    return contract;
  }
  const response: ConfigurationResult = {
    fields: [],
    process_files: [],
    pft_parameters: [],
    warnings: [],
    context_complete: false,
  };
  async function resolve(index = requests.length - 1) {
    requests[index].resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    return render();
  }
  return { props, render, resolve, requests, changes, forward, nodes, response };
}
test('cards and guide share visible controls, exact context, loading and disabled locks', async () => {
  const h = harness();
  let c = h.render();
  assert.equal(c.status, 'loading');
  assert.equal(h.requests[0].args.caseDir, '/case/a');
  assert.equal((h.requests[0].args.source as { generation: number }).generation, 3);
  c = await h.resolve();
  assert.equal(c.status, 'pending_source_verification');
  assert.deepEqual(
    c.fields.map((f) => f.id),
    setup
      .setupChoices(h.props.overlay, h.props.fields, h.props.environment)
      .map((o) => `setup-${o.id}`),
  );
  assert.equal(h.nodes.filter((n) => n.type === 'Card').length, c.fields.length);
  c.fields.find((f) => f.id === 'setup-DEF_USE_BGC')!.set('.true.');
  assert.equal(h.changes.length, 1);
  const beforeLock = c.fields[0];
  h.props.disabled = true;
  c = h.render();
  assert.throws(() => beforeLock.set('PC'));
  assert.ok(c.fields.every((f) => f.disabled));
  assert.throws(() => c.fields[0].set('PC'));
  h.props.disabled = false;
  h.props.includeSubgrid = false;
  c = h.render();
  assert.ok(!c.fields.some((f) => f.id === 'setup-subgrid'));
});
test('old query and old guide callback cannot write after draft/source changes', async () => {
  const h = harness();
  h.render();
  let c = await h.resolve();
  const old = c.fields[0];
  h.props.source = { kind: 'draft', id: 'd2', generation: 9, overlay: h.props.overlay };
  c = h.render();
  assert.throws(() => old.set('PC'), /stale/);
  assert.equal(c.status, 'loading');
  c = await h.resolve();
  assert.notEqual(c.status, 'loading');
  const early = harness();
  early.render();
  early.props.caseDir = '/case/b';
  early.render();
  c = await early.resolve(0);
  assert.equal(c.status, 'loading');
  c = await early.resolve(1);
  assert.equal(c.status, 'pending_source_verification');
});
test('frozen model cards write case fields directly and leave the selected revision unchanged', async () => {
  const h = harness();
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 5 } };
  h.render();
  let c = await h.resolve();
  assert.ok(c.fields.every((f) => f.disabled));
  const patches: CaseOptionPatch[] = [];
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  c = h.render();
  const before = structuredClone(h.props.overlay);
  await c.fields.find((f) => f.id === 'setup-soil')!.set('vG');
  assert.deepEqual(patches, [
    { DEF_USE_Campbell_SOIL_MODEL: '.false.', DEF_USE_VariablySaturatedFlow: '.true.' },
  ]);
  assert.deepEqual(h.props.overlay, before);
  assert.equal(h.changes.length, 0);
});

test('crop card starts unspecified and requires explicit planting-day input before changing overlay', async () => {
  const h = harness();
  h.props.overlay.physics.DEF_USE_BGC = '.true.';
  h.props.fields.find((f) => f.name === 'DEF_TUNING_CROP_PLANTING_DAY')!.kind = 'integer';
  h.props.environment.cropKernel = { dir: '/crop', macros: ['CROP'] } as KernelEntry;
  h.render();
  let c = await h.resolve();
  assert.equal(c.fields.find((f) => f.id === 'setup-crop')!.value, '');
  c.fields.find((f) => f.id === 'setup-crop')!.set('.true.');
  c = h.render();
  assert.equal(h.changes.length, 0);
  assert.equal(c.fields.find((f) => f.id === 'setup-crop-planting-day')!.value, '');
  assert.equal(c.actions.find((a) => a.id === 'setup-crop-apply')!.enabled, false);
  c.fields.find((f) => f.id === 'setup-crop-planting-day')!.set('120');
  c = h.render();
  c.actions.find((a) => a.id === 'setup-crop-apply')!.run();
  assert.equal(h.changes[0].physics.DEF_TUNING_CROP_PLANTING_DAY, '120');
});

test('visible river options and guide enforce the same verified methane-mode gate', async () => {
  for (const mode of ['routing', 'hybrid', 'wetwat']) {
    const h = harness();
    Object.assign(h.props.overlay.physics, {
      DEF_USE_TRACER: '.true.',
      DEF_USE_BGC: '.true.',
      DEF_USE_GridRiverLakeFlow: '.true.',
      DEF_TRACER_NAMES: "'CH4'",
    });
    h.response.process_files = [
      {
        file: 'CH4.nml',
        title: '',
        section: '',
        verified: true,
        active_methane_asset: true,
        entries: [
          {
            path: 'DEF_METHANE%inundation_mode',
            value: mode,
            unset: false,
            group: 'methane',
            kind: 'choice',
            asset_value: mode,
            default: 'wetwat',
            options: ['wetwat', 'routing', 'hybrid'],
            editable: true,
            descriptor_id: null,
            pending: false,
          },
        ],
      },
    ] as ConfigurationResult['process_files'];
    h.render();
    const c = await h.resolve(),
      river = c.fields.find((f) => f.id === 'setup-DEF_USE_GridRiverLakeFlow')!;
    if (mode === 'wetwat') {
      assert.ok(river.options!.some((o) => o.value === '.false.'));
      river.set('.false.');
      assert.equal(h.changes.length, 1);
    } else {
      assert.ok(!river.options!.some((o) => o.value === '.false.'));
      assert.throws(() => river.set('.false.'), /methane/);
      assert.equal(h.changes.length, 0);
    }
  }
});
