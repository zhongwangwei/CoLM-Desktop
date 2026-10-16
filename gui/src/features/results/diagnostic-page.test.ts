import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import * as seriesHelpers from './series';
import * as overviewHelpers from './result-overview';
import type { PageContract } from '../../lib/guide';

type Element = { type: string | { name: string }; props: Record<string, unknown> };
const normal = {
  time: [1, 2],
  vars: { f_lfevpa: [100, 200] },
  source_n: 2,
  n: 2,
  downsampled: false,
};
const residual = {
  time: [1, 2],
  vars: { energy_residual: [3, 5] },
  source_n: 2,
  n: 2,
  downsampled: false,
};
const diagnostic = {
  schema: 1,
  expression: 'Rnet-Qh-Qle-Qg',
  closure_assessed: false,
  full_stats: {
    basis: 'all_records_in_selected_window',
    mean_kind: 'arithmetic_mean_of_common_finite_records',
    status: 'computed',
    source_n: 2,
    n: 2,
    missing: 0,
    mean: 4,
    units: 'W/m²',
  },
  series: residual,
};
function harness(
  stations = false,
  delayedSeries?: Promise<unknown>,
  openError = '',
  initialCase = '/case',
) {
  let selectedCase = initialCase;
  const opened: string[] = [];
  const requests: { command: string; args: Record<string, unknown> }[] = [];
  const source = readFileSync(new URL('./ResultsPage.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;\n/gm, '')
    .replace('export default function', 'function')
    .replace('export function ScienceChart', 'function ScienceChart');
  const code = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  let cursor = 0,
    dirty = false,
    effects: (() => void)[] = [];
  const slots: unknown[] = [],
    exports: string[] = [];
  let contract: PageContract, tree: Element;
  const same = (a: unknown[] | undefined, b: unknown[]) =>
    a?.length === b.length && a.every((x, i) => Object.is(x, b[i]));
  const invokeJson = async (command: string, args: Record<string, unknown>) => {
    requests.push({ command, args });
    if (command === 'model_case_runs' && args.dir === '/empty') return [];
    if (command === 'model_case_runs')
      return [
        {
          run_case: '/case/.colm/runs/a',
          version: { id: 'official', revision: 1 },
          status: 'succeeded',
        },
      ];
    if (command === 'history_catalog')
      return {
        files: 1,
        steps: 2,
        start: 1,
        end: 2,
        variables: [{ name: 'f_lfevpa', kind: 'series', units: 'W/m²', dimensions: [] }],
      };
    if (command === 'series') return delayedSeries ?? normal;
    if (command === 'energy_summary') return diagnostic;
    throw Error(command);
  };
  const env: Record<string, unknown> = {
    ...seriesHelpers,
    ...overviewHelpers,
    invokeJson,
    invoke: invokeJson,
    hasBackend: () => true,
    defaultObservation: async () => '',
    timeTicks: () => [],
    uPlot: () => {},
    download: async (_name: string, text: string) => {
      exports.push(text);
      return { status: 'saved', path: '/saved.json' };
    },
    React: {
      createElement: (type: Element['type'], props: Element['props'], ...children: unknown[]) => ({
        type,
        props: { ...props, children },
      }),
    },
    useApp: () => ({
      state: {
        language: 'en',
        theme: 'light',
        versionRef: { id: 'official', revision: 1 },
        cases: stations
          ? [
              { dir: '/case', name: 'First' },
              { dir: '/empty', name: 'Empty' },
            ]
          : [{ dir: '/case' }],
        running: {},
      },
      navigate() {},
      async openCase(dir: string) {
        opened.push(dir);
        if (openError === 'blocked') return null;
        if (openError) throw new Error(openError);
        selectedCase = dir;
        return { dir };
      },
    }),
    usePageContract: (value: PageContract) => {
      contract = value;
    },
    useState(initial: unknown) {
      const i = cursor++;
      if (!(i in slots)) slots[i] = typeof initial === 'function' ? initial() : initial;
      return [
        slots[i],
        (value: unknown) => {
          const next = typeof value === 'function' ? value(slots[i]) : value;
          if (!Object.is(next, slots[i])) {
            slots[i] = next;
            dirty = true;
          }
        },
      ];
    },
    useRef(initial: unknown) {
      const i = cursor++;
      return slots[i] ?? (slots[i] = { current: initial });
    },
    useCallback(fn: unknown, deps: unknown[]) {
      const i = cursor++,
        old = slots[i] as { deps: unknown[]; fn: unknown } | undefined;
      if (!same(old?.deps, deps)) slots[i] = { fn, deps };
      return (slots[i] as { fn: unknown }).fn;
    },
    useEffect(fn: () => (() => void) | void, deps: unknown[]) {
      const i = cursor++,
        old = slots[i] as { deps: unknown[]; cleanup?: () => void } | undefined;
      if (!same(old?.deps, deps)) {
        const slot = { deps, cleanup: undefined as (() => void) | undefined };
        slots[i] = slot;
        effects.push(() => {
          old?.cleanup?.();
          slot.cleanup = fn() || undefined;
        });
      }
    },
  };
  for (const name of [
    'Button',
    'Card',
    'Field',
    'Input',
    'Select',
    'Notice',
    'Advanced',
    'Dialog',
    'NavIcon',
  ])
    env[name] = name;
  const Page = new Function(...Object.keys(env), `${code};return ResultsPage;`)(
    ...Object.values(env),
  );
  async function render() {
    for (let n = 0; n < 15; n++) {
      dirty = false;
      cursor = 0;
      tree = Page({ casePath: selectedCase });
      const pending = effects;
      effects = [];
      pending.forEach((fn) => fn());
      await new Promise((resolve) => setImmediate(resolve));
      if (!dirty && !effects.length) return;
    }
    throw Error('Render did not settle');
  }
  function charts(node: unknown): unknown[] {
    if (!node || typeof node !== 'object') return [];
    if (Array.isArray(node)) return node.flatMap(charts);
    const element = node as Element;
    return [
      ...(typeof element.type === 'function' && element.type.name === 'ScienceChart'
        ? [{ labels: element.props.labels, data: element.props.data }]
        : []),
      ...charts(element.props?.children),
    ];
  }
  return {
    render,
    exports,
    opened,
    requests,
    station: () => contract.fields!.find((field) => field.id === 'result-station'),
    async chooseStation(dir: string) {
      contract.fields!.find((field) => field.id === 'result-station')!.set!(dir);
      await new Promise((resolve) => setImmediate(resolve));
      await render();
    },
    charts: () => charts(tree),
    tree: () => tree,
    allowSwitch: () => {
      openError = '';
    },
    async tab(value: string) {
      contract.fields!.find((f) => f.id === 'result-tab')!.set!(value);
      await render();
    },
    async action(id: string) {
      await contract.actions!.find((a) => a.id === id)!.run();
      await render();
    },
  };
}
test('energy diagnostics keep residual charts and exports after visiting the overview', async () => {
  const h = harness();
  await h.render();
  await h.tab('diagnostics');
  await h.action('diagnose-closure');
  const expected = [
    {
      labels: ['energy_residual'],
      data: [
        [1, 2],
        [3, 5],
      ],
    },
  ];
  assert.deepEqual(h.charts(), expected);
  await h.tab('overview');
  await h.tab('diagnostics');
  assert.deepEqual(h.charts(), expected);
  await h.action('export-series-data');
  assert.deepEqual(JSON.parse(h.exports[0]).vars, residual.vars);
  await h.tab('series');
  await h.action('export-series-data');
  assert.deepEqual(JSON.parse(h.exports[1]).vars, normal.vars);
});

test('project station selector switches cases and removes the previous station charts', async () => {
  const h = harness(true);
  await h.render();
  assert.deepEqual(h.station()?.options, [
    { value: '/case', label: 'First' },
    { value: '/empty', label: 'Empty' },
  ]);
  assert.ok(h.charts().length);
  await h.chooseStation('/empty');
  assert.deepEqual(h.opened, ['/empty']);
  assert.equal(h.station()?.value, '/empty');
  assert.deepEqual(h.charts(), []);
  assert.ok(
    h.requests.some(
      (request) => request.command === 'model_case_runs' && request.args.dir === '/empty',
    ),
  );
});

test('delayed series from the previous station cannot populate the new station', async () => {
  let finish!: (value: unknown) => void;
  const h = harness(
    true,
    new Promise((resolve) => {
      finish = resolve;
    }),
  );
  await h.render();
  assert.ok(h.requests.some((request) => request.command === 'series'));
  await h.chooseStation('/empty');
  finish(normal);
  await h.render();
  assert.deepEqual(h.charts(), []);
  assert.equal(h.station()?.value, '/empty');
});

test('a failed station switch reloads the original station instead of leaving a blank page', async () => {
  const h = harness(true, undefined, 'Cannot read station');
  await h.render();
  assert.ok(h.charts().length);
  await h.chooseStation('/empty');
  assert.equal(h.station()?.value, '/case');
  assert.ok(h.charts().length, 'original station data reloads after the failure');
  assert.ok(
    h.requests.filter(({ command, args }) => command === 'model_case_runs' && args.dir === '/case')
      .length >= 2,
  );
  assert.ok(JSON.stringify(h.tree()).includes('Cannot read station'));
  h.allowSwitch();
  await h.chooseStation('/empty');
  assert.equal(h.station()?.value, '/empty');
  assert.ok(!JSON.stringify(h.tree()).includes('Cannot read station'));
});

test('blocked navigation restores the original station and an unselected single station has a selectable placeholder', async () => {
  const blocked = harness(true, undefined, 'blocked');
  await blocked.render();
  await blocked.chooseStation('/empty');
  assert.equal(blocked.station()?.value, '/case');
  assert.ok(blocked.charts().length);
  const empty = harness(false, undefined, '', '');
  await empty.render();
  assert.equal(empty.station()?.value, '');
  function placeholders(value: unknown): Element[] {
    if (Array.isArray(value)) return value.flatMap(placeholders);
    if (!value || typeof value !== 'object') return [];
    const node = value as Element;
    return [
      ...(node.type === 'option' && node.props.value === '' && node.props.disabled ? [node] : []),
      ...placeholders(node.props?.children),
    ];
  }
  assert.equal(placeholders(empty.tree()).length, 1);
  await empty.chooseStation('/case');
  assert.deepEqual(empty.opened, ['/case']);
  assert.ok(empty.charts().length);
});
