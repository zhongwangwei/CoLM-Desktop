import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import * as helpers from './configuration-options';
import * as settings from '../cases/model-settings';
import * as presentation from '../../lib/param-presentation';
import { PFT } from './scope-labels';
import type { Overlay } from '../../lib/types';
import type { UiAction, UiField } from '../../lib/guide';
import type { ModelField, Descriptor } from './model';
type Contract = { fields: UiField[]; actions: UiAction[]; status: string };
function harness() {
  const overlay: Overlay = {
    physics: {
      DEF_USE_BGC: '.true.',
      DEF_USE_TRACER: '.true.',
      DEF_USE_GridRiverLakeFlow: '.true.',
      DEF_USE_WUEST: '.true.',
    },
    parameters: [],
    process_files: [],
  };
  const fields: ModelField[] = [
    ['DEF_USE_BGC', '生态与生地化'],
    ['DEF_USE_MEDLYNST', '生态与生地化'],
    ['DEF_USE_WUEST', '生态与生地化'],
    ['DEF_USE_NITRIF_DENITRIF', '生态与生地化'],
    ['DEF_USE_GridRiverLakeFlow', '河道与水库'],
    ['DEF_USE_RESERVOIR', '河道与水库'],
    ['DEF_USE_TRACER', '示踪剂'],
    ['DEF_URBAN_RUN', '城市'],
    ['DEF_URBAN_type_scheme', '城市'],
    ...helpers.tracerKeys.map((k) => [k, '示踪剂']),
  ].map(([name, section]) => ({
    name,
    section,
    kind: helpers.tracerKeys.includes(name)
      ? name === 'DEF_TRACER_NUM'
        ? 'integer'
        : 'character'
      : 'logical',
    default: '.false.',
    values: [],
    derived: false,
  }));
  const props = {
    onCaseChange: undefined as
      undefined | ((patch: helpers.CaseOptionPatch) => void | Promise<void>),
    caseValues: undefined as Record<string, string> | undefined,
    subgrid: undefined as string | undefined,
    source: { kind: 'draft', id: 'd1', generation: 0, overlay } as helpers.ConfigurationSource,
    overlay,
    fields,
    catalog: [] as Descriptor[],
    disabled: false,
    onChange: (next: Overlay) => {
      props.overlay = next;
      changes.push(next);
    },
  };
  const changes: Overlay[] = [],
    requests: {
      command: string;
      args: Record<string, unknown>;
      resolve: (value: helpers.ConfigurationResult) => void;
    }[] = [],
    slots: unknown[] = [],
    effects: { deps: unknown[]; cleanup?: () => void }[] = [],
    pending: (() => void)[] = [];
  let cursor = 0,
    effectCursor = 0,
    contract: Contract;
  const nodes: { type: string; props: Record<string, unknown>; children?: unknown[] }[] = [];
  const context = {
    PFT,
    ...helpers,
    ...settings,
    ...presentation,
    React: {
      createElement: (type: string, p: Record<string, unknown>, ...children: unknown[]) => {
        const result = { type, props: p ?? {}, children };
        nodes.push(result);
        return result;
      },
    },
    ...Object.fromEntries(
      ['Advanced', 'Button', 'Card', 'Field', 'Input', 'Notice', 'Select'].map((k) => [k, k]),
    ),
    useApp: () => ({ state: { language: 'zh' } }),
    usePageContract: (value: Contract) => {
      contract = value;
    },
    useState: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = initial;
      return [
        slots[i],
        (value: unknown) => {
          slots[i] = typeof value === 'function' ? value(slots[i]) : value;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const i = cursor++;
      if (!(i in slots)) slots[i] = { current: initial };
      return slots[i];
    },
    useEffect: (fn: () => void | (() => void), deps: unknown[]) => {
      const i = effectCursor++;
      if (!effects[i] || deps.some((d, j) => d !== effects[i].deps[j])) {
        effects[i]?.cleanup?.();
        effects[i] = { deps };
        pending.push(() => {
          const cleanup = fn();
          effects[i].cleanup = cleanup || undefined;
        });
      }
    },
    invokeJson: (command: string, args: Record<string, unknown>) =>
      new Promise<helpers.ConfigurationResult>((resolve) =>
        requests.push({ command, args, resolve }),
      ),
  };
  const source = readFileSync(new URL('./ConfigurationOptions.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const Component = new Function(...Object.keys(context), `${code};return ConfigurationOptions;`)(
    ...Object.values(context),
  );
  function render() {
    cursor = 0;
    effectCursor = 0;
    nodes.length = 0;
    Component(props);
    pending.splice(0).forEach((f) => f());
    return contract;
  }
  const response = (): helpers.ConfigurationResult => ({
    fields: fields.map((f) => ({
      name: f.name,
      mode: f.name === 'DEF_USE_NITRIF_DENITRIF' ? 'disabled' : 'editable',
      reason: f.name === 'DEF_USE_NITRIF_DENITRIF' ? 'disabled evidence' : 'pending',
      configured_value: props.overlay.physics[f.name] ?? null,
      allowed_values: [],
      pending: true,
    })),
    process_files: [],
    pft_parameters: [],
    warnings: ['pending frozen source'],
    context_complete: false,
  });
  async function settle() {
    requests.at(-1)!.resolve(response());
    for (let i = 0; i < 5; i++) await Promise.resolve();
    return render();
  }
  return { render, settle, props, requests, changes, response, nodes };
}
test('actual editor and guide expose conditional groups, retain inactive fields, and lock frozen/busy writes', async () => {
  const p = harness();
  let c = p.render();
  assert.equal(c.status, 'loading');
  assert.equal(p.requests[0].command, 'model_configuration');
  assert.equal(p.requests[0].args.caseValues, null);
  c = await p.settle();
  const medlyn = () => c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!;
  assert.ok(medlyn());
  assert.equal(medlyn().disabled, false);
  const inactive = c.fields.find((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF')!;
  assert.equal(inactive.disabled, true);
  assert.throws(() => inactive.set('.true.'));
  assert.equal(p.changes.length, 0);
  medlyn().set('MEDLYN');
  assert.equal(p.props.overlay.physics.DEF_USE_WUEST, '.false.');
  assert.equal(p.props.overlay.physics.DEF_USE_BGC, '.true.');
  c = p.render();
  assert.equal(c.status, 'loading');
  c = await p.settle();
  assert.equal(medlyn().value, 'MEDLYN');
  p.props.disabled = true;
  c = p.render();
  assert.equal(medlyn().disabled, true);
  assert.throws(() => medlyn().set('BALL_BERRY'));
  p.props.disabled = false;
  p.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  c = p.render();
  c = await p.settle();
  assert.equal(medlyn().disabled, true);
  assert.throws(() => medlyn().set('BALL_BERRY'));
  assert.equal(p.changes.length, 1);
});
test('late configuration responses cannot replace current version and guide only contains expanded controls', async () => {
  const p = harness();
  p.render();
  const old = p.requests[0];
  p.props.source = { kind: 'draft', id: 'd2', generation: 0, overlay: p.props.overlay };
  p.render();
  let c: Contract;
  const latest = p.requests[1];
  old.resolve(p.response());
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = p.render();
  assert.equal(c.status, 'loading');
  latest.resolve(p.response());
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = p.render();
  assert.notEqual(c.status, 'loading');
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  assert.ok(!c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(!c.actions.some((a) => a.id === 'configuration-stomatal-reset'));
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(c.actions.some((a) => a.id === 'configuration-stomatal-reset'));
  assert.equal(p.requests.at(-1)!.args.pftSelection, null);
  assert.ok(!c.fields.some((f) => f.id === 'configuration-pft-index'));
  c.actions.find((a) => a.id === 'configuration-pft-toggle')!.run();
  c = p.render();
  const index = c.fields.find((f) => f.id === 'configuration-pft-index')!;
  index.set('');
  p.render();
  assert.equal(p.requests.at(-1)!.args.pftSelection, null);
});
test('tracer table accumulates incomplete rows locally, then applies valid aligned definitions atomically', async () => {
  const p = harness();
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-tracers-add')!.run();
  c = p.render();
  assert.equal(p.changes.length, 0);
  assert.throws(() => c.actions.find((a) => a.id === 'configuration-tracers-apply')!.run());
  assert.equal(p.changes.length, 0);
  c.fields.find((f) => f.id === 'configuration-tracer-0-NAMES')!.set('CH4');
  c = p.render();
  c.fields.find((f) => f.id === 'configuration-tracer-0-TYPES')!.set('reactive');
  c = p.render();
  c.actions.find((a) => a.id === 'configuration-tracers-apply')!.run();
  assert.equal(p.changes.length, 1);
  assert.equal(p.props.overlay.physics.DEF_TRACER_NUM, '1');
  assert.equal(p.props.overlay.physics.DEF_TRACER_NAMES, "'CH4'");
});
test('enabling BGC, rivers or tracer reopens previously collapsed children only on a new activation', async () => {
  for (const [group, key] of [
    ['ecology', 'DEF_USE_BGC'],
    ['river', 'DEF_USE_GridRiverLakeFlow'],
    ['tracer', 'DEF_USE_TRACER'],
  ]) {
    const p = harness();
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.false.' },
    };
    p.render();
    let c = await p.settle();
    const groupNode = () =>
      p.nodes.find(
        (n) =>
          n.type === 'Advanced' &&
          String(n.props.title).startsWith(
            settings.modelSettingGroups.find((g) => g[0] === group)![1],
          ),
      )!;
    assert.equal(groupNode().props.open, false);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    c = p.render();
    assert.equal(groupNode().props.open, true);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.true.' },
    };
    p.render();
    assert.equal(groupNode().props.open, true);
    c = await p.settle();
    assert.equal(groupNode().props.open, true);
    c.actions.find((a) => a.id === `configuration-group-${group}`)!.run();
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, DEF_USE_WUEST: '.false.' },
    };
    p.render();
    assert.equal(groupNode().props.open, false);
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.false.' },
    };
    p.render();
    p.props.overlay = {
      ...p.props.overlay,
      physics: { ...p.props.overlay.physics, [key]: '.true.' },
    };
    p.render();
    assert.equal(groupNode().props.open, true);
  }
});

test('unknown parent activation opens its previously collapsed group without waiting for metadata', async () => {
  const p = harness();
  delete p.props.overlay.physics.DEF_USE_BGC;
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  c = p.render();
  c.actions.find((a) => a.id === 'configuration-group-ecology')!.run();
  p.render();
  p.props.overlay = {
    ...p.props.overlay,
    physics: { ...p.props.overlay.physics, DEF_USE_BGC: '.true.' },
  };
  c = p.render();
  assert.equal(c.status, 'loading');
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
});
test('actual process-list editor preserves quoted list display and submits per-element typed values', async () => {
  const p = harness();
  p.props.overlay.process_files = [{ relative_path: 'forcing.nml', sha256: 'a' }];
  p.props.catalog = [
    {
      id: 'forcing:role',
      raw_key: 'DEF_forcing%forcing_role',
      label_zh: '强迫场角色',
      label_en: 'Forcing roles',
      scope: 'process-file',
      storage: 'process-parameter-file',
      section: '',
      value_kind: 'character',
      unit: null,
      visibility: 'editable',
      activation: [],
      default: null,
      doc_zh: '',
      doc_en: '',
      validation: { min: null, max: null, allowed_values: [], finite: false },
    },
  ];
  p.render();
  const response = p.response();
  response.process_files = [
    {
      file: 'forcing.nml',
      title: 'forcing',
      section: '',
      verified: true,
      active_methane_asset: false,
      entries: [
        {
          path: 'DEF_forcing%forcing_role',
          group: 'DEF_forcing',
          kind: 'list',
          value: "'TAIR', 'WIND'",
          asset_value: "'TAIR', 'WIND'",
          default: '',
          unset: false,
          options: [],
          editable: true,
          descriptor_id: 'forcing:role',
          pending: true,
        },
      ],
    },
  ];
  p.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  const c = p.render(),
    field = c.fields.find(
      (f) => f.id === 'configuration-process-forcing.nml-DEF_forcing%forcing_role',
    )!;
  assert.equal(field.value, "'TAIR', 'WIND'");
  field.set("'TAIR', 'RAIN'");
  assert.equal(p.changes[0].parameters[0].value, "'TAIR', 'RAIN'");
});

test('actual generic groups and guide omit land-class fields that require indexed parameter overrides', async () => {
  const p = harness();
  p.props.fields.push({
    name: 'DEF_LC_VMAX25',
    section: '生态与生地化',
    kind: 'real',
    default: '50.',
    values: [],
    derived: false,
  });
  p.props.catalog.push({
    id: 'land:IGBP:vmax',
    raw_key: 'def_lc_vmax25',
    label_zh: '光合能力',
    label_en: 'Photosynthesis capacity',
    scope: 'land-cover-class',
    storage: 'case-nml',
    section: '',
    value_kind: 'real',
    unit: null,
    visibility: 'editable',
    activation: [],
    default: null,
    doc_zh: '',
    doc_en: '',
    validation: { min: null, max: null, allowed_values: [], finite: true },
  });
  p.render();
  const c = await p.settle();
  assert.ok(c.fields.some((f) => f.id === 'configuration-stomatal-scheme'));
  assert.ok(!c.fields.some((f) => f.id.includes('DEF_LC_VMAX25')));
  assert.ok(!c.actions.some((a) => a.id.includes('DEF_LC_VMAX25')));
  assert.ok(
    !p.nodes.some(
      (n) => n.type === 'Field' && n.props.label === presentation.fieldLabel('DEF_LC_VMAX25', 'zh'),
    ),
  );
  assert.equal(p.changes.length, 0);
});

test('family cards stage only missing names and types, preserve custom rows and never save on click', async () => {
  const p = harness();
  p.props.overlay.physics = {
    ...p.props.overlay.physics,
    DEF_USE_Campbell_SOIL_MODEL: '.false.',
    DEF_URBAN_RUN: '.false.',
    DEF_USE_PFT: '.true.',
    DEF_USE_LCT: '.false.',
    DEF_USE_PC: '.false.',
    DEF_USE_LULCC: '.false.',
  };
  p.render();
  let c = await p.settle();
  const isotope = c.actions.find((a) => a.id === 'configuration-family-isotope')!;
  assert.equal(isotope.enabled, true);
  isotope.run();
  c = p.render();
  assert.equal(p.changes.length, 0);
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-0-NAMES')?.value, 'H2_18O');
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-0-MRAT')?.value, '');
  assert.equal(c.fields.find((f) => f.id === 'configuration-tracer-1-NAMES')?.value, 'HDO');
  assert.equal(c.actions.find((a) => a.id === 'configuration-family-isotope')?.enabled, false);
  c.actions.find((a) => a.id === 'configuration-tracers-remove')!.run();
  c = p.render();
  assert.equal(
    c.fields.some((f) => f.id === 'configuration-tracer-1-NAMES'),
    false,
  );
  assert.equal(p.changes.length, 0);
  p.props.overlay = {
    ...p.props.overlay,
    physics: { ...p.props.overlay.physics, DEF_USE_LULCC: '.true.' },
  };
  p.render();
  c = await p.settle();
  assert.equal(c.actions.find((a) => a.id === 'configuration-family-isotope')?.enabled, false);
  p.props.disabled = true;
  c = p.render();
  assert.ok(
    c.actions
      .filter((a) => a.id.startsWith('configuration-family-'))
      .every((a) => a.enabled === false),
  );
});
test('core runoff enum appears as choice cards while the visible guide shares the same atomic setter', async () => {
  const p = harness();
  p.props.fields.push({
    name: 'DEF_Runoff_SCHEME',
    section: '水热过程',
    kind: 'integer',
    default: '0',
    values: ['0', '1', '2', '3'],
    derived: false,
  });
  p.render();
  let c = await p.settle();
  c.actions.find((a) => a.id === 'configuration-group-water')!.run();
  c = p.render();
  const field = c.fields.find((f) => f.id === 'configuration-DEF_Runoff_SCHEME')!;
  assert.equal(field.value, '');
  assert.deepEqual(
    field.options?.map((o) => o.value),
    ['0', '1', '2', '3'],
  );
  const choice = p.nodes.find(
    (n) =>
      n.type === 'Button' &&
      n.props['aria-pressed'] === false &&
      n.props.className === 'simulation-choice-card' &&
      n.children?.flat().includes(presentation.optionLabel('DEF_Runoff_SCHEME', '2', 'zh')),
  )!;
  assert.ok(choice);
  field.set('2');
  assert.equal(p.changes[0].physics.DEF_Runoff_SCHEME, '2');
  assert.throws(() => field.set('99'));
});

test('same-kernel scheme changes reload BGC tracer and PFT context and invalidate prior replies', async () => {
  const h = harness();
  h.props.subgrid = 'IGBP';
  h.render();
  await h.settle();
  const old = h.requests.at(-1)!;
  h.props.subgrid = 'PFT';
  h.render();
  const pft = h.requests.at(-1)!;
  assert.notEqual(pft, old);
  assert.equal((pft.args.caseValues as Record<string, string>).DEF_USE_PFT, '.true.');
  h.props.subgrid = 'PC';
  h.render();
  const pc = h.requests.at(-1)!;
  assert.notEqual(pc, pft);
  assert.equal((pc.args.caseValues as Record<string, string>).DEF_USE_PC, '.true.');
  assert.deepEqual(pc.args.source, pft.args.source);
  assert.equal(pc.args.kernelDir, pft.args.kernelDir);
  pft.resolve({ ...h.response(), warnings: ['STALE PFT RESULT'] });
  for (let i = 0; i < 5; i++) await Promise.resolve();
  h.render();
  assert.ok(!JSON.stringify(h.nodes).includes('STALE PFT RESULT'));
  await h.settle();
  assert.ok(!JSON.stringify(h.nodes).includes('STALE PFT RESULT'));
});

test('frozen model process options persist an atomic case patch without editing the revision', async () => {
  const h = harness(),
    patches: helpers.CaseOptionPatch[] = [];
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  h.props.caseValues = {
    DEF_USE_BGC: '.true.',
    DEF_USE_MEDLYNST: '.false.',
    DEF_USE_WUEST: '.true.',
  };
  h.props.onCaseChange = (patch) => {
    patches.push(patch);
  };
  const original = structuredClone(h.props.overlay);
  h.render();
  const c = await h.settle();
  c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.set('MEDLYN');
  for (let i = 0; i < 8; i++) await Promise.resolve();
  assert.deepEqual(patches, [{ DEF_USE_MEDLYNST: '.true.', DEF_USE_WUEST: '.false.' }]);
  assert.equal(h.changes.length, 0);
  assert.deepEqual(h.props.overlay, original);
});
test('case option failure and readonly source gates preserve the selected revision', async () => {
  const h = harness();
  h.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  h.props.caseValues = { DEF_USE_BGC: '.true.' };
  h.props.onCaseChange = async () => {
    throw new Error('case save rejected');
  };
  const original = structuredClone(h.props.overlay);
  h.render();
  let c = await h.settle();
  c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.set('MEDLYN');
  for (let i = 0; i < 12; i++) await Promise.resolve();
  c = h.render();
  assert.match(c.status, /case save rejected/);
  assert.deepEqual(h.props.overlay, original);
  assert.equal(h.changes.length, 0);
  h.props.subgrid = 'PC';
  h.render();
  const response = h.response();
  response.fields.find((f) => f.name === 'DEF_USE_MEDLYNST')!.mode = 'readonly';
  h.requests.at(-1)!.resolve(response);
  for (let i = 0; i < 5; i++) await Promise.resolve();
  c = h.render();
  assert.equal(c.fields.find((f) => f.id === 'configuration-stomatal-scheme')!.disabled, true);
});

test('simulation hides inactive process details and files without erasing retained settings', async () => {
  const p = harness();
  p.props.source = { kind: 'frozen', version: { id: 'official', revision: 1 } };
  p.props.onCaseChange = () => {};
  p.props.subgrid = 'IGBP';
  const original = JSON.stringify(p.props.overlay);
  for (const value of [undefined, '.false.', '.true.', '.false.']) {
    p.props.caseValues = Object.fromEntries(
      ['DEF_USE_BGC', 'DEF_USE_TRACER', 'DEF_USE_GridRiverLakeFlow', 'DEF_URBAN_RUN'].flatMap(
        (name) => (value ? [[name, value]] : []),
      ),
    );
    p.render();
    const response = p.response();
    response.fields = response.fields.map((field) => ({ ...field, configured_value: null }));
    p.requests.at(-1)!.resolve(response);
    for (let i = 0; i < 5; i++) await Promise.resolve();
    const c = p.render();
    for (const group of ['river', 'urban', 'tracer']) {
      const available = value === '.true.';
      assert.equal(
        c.actions.some((a) => a.id === `configuration-group-${group}`),
        available,
      );
    }
    assert.equal(
      p.nodes.some((n) => n.type === 'Card' && n.props.title === '示踪过程文件'),
      value === '.true.',
    );
    assert.equal(
      c.fields.some((f) => f.id === 'configuration-DEF_USE_NITRIF_DENITRIF'),
      value === '.true.',
    );
    assert.ok(!c.actions.some((a) => a.id === 'configuration-pft-toggle'));
  }
  assert.equal(JSON.stringify(p.props.overlay), original);
  assert.equal(p.changes.length, 0);
});
