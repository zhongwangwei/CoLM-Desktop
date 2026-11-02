import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import type { StudyPick } from '../../lib/types';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import { parameterType, type Descriptor } from './model';
import { Button, Input, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { applyResearchSeed, type ParameterContext } from '../studies/study';
const store = new Map<string, string>();
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => store.set(k, v),
  },
});
const { literalNumber, typesOf } = await import('./ParameterDialog');
const { default: StudyPicks, defaultRange, loadPicks } = await import('./StudyPicks');

const descriptor = (min: string | null, max: string | null) =>
  ({ validation: { min, max, allowed_values: [], finite: true } }) as unknown as Descriptor;

test('a study range starts at half to one and a half times the default, inside the catalog bounds', () => {
  assert.deepEqual(defaultRange(descriptor('0', null), 52), ['26', '78']);
  assert.deepEqual(defaultRange(descriptor('0', '1'), 0.8), ['0.4', '1']);
  assert.deepEqual(defaultRange(descriptor('-1', '1'), -0.3), ['-0.45', '-0.15']);
  assert.deepEqual(defaultRange(descriptor('0', '1'), 0), ['0', '1'], 'no default: catalog bounds');
  assert.deepEqual(defaultRange(descriptor(null, null), null), ['', '']);
});

test('picks remembered per version survive, and unreadable storage just means none', () => {
  store.set('colm.versions.studyPicks.model-1', JSON.stringify([{ parameter_id: 'x' }]));
  assert.equal(loadPicks('model-1').length, 1);
  store.set('colm.versions.studyPicks.model-2', '{broken');
  assert.deepEqual(loadPicks('model-2'), []);
  assert.deepEqual(loadPicks('model-3'), []);
});

test('picks from the version page tick the matching research rows only', () => {
  const row = (id: string, index: number | null, scheme: string | null = null) =>
    ({ id, scope_instance: { kind: 'x', index, scheme } }) as unknown as ParameterContext;
  const rows = [
    row('pft:DEF_PFT_VMAX25', 13),
    row('pft:DEF_PFT_VMAX25', 14),
    row('lct:IGBP:DEF_LC_VMAX25', 10, 'IGBP'),
    row('case:DEF_TUNING_CNFAC', null),
  ];
  const pick = (parameter_id: string, index: number | null, scheme: string | null = null) => ({
    parameter_id,
    raw_key: '',
    scheme,
    index,
    min: '1',
    max: '2',
    scale: 'linear',
  });
  const picked = applyResearchSeed(rows, [
    pick('pft:DEF_PFT_VMAX25', 14),
    pick('lct:IGBP:DEF_LC_VMAX25', 10, 'IGBP'),
    pick('case:DEF_TUNING_CNFAC', null),
    pick('pft:DEF_PFT_VMAX25', 7),
  ]);
  assert.deepEqual(Object.keys(picked).sort(), ['1', '2', '3'], 'PFT 7 is not in these cases');
  assert.deepEqual(picked['1'], { enabled: true, min: '1', max: '2', scale: 'linear' });
});

test('the add button registers itself while removing a pick stays manual', () => {
  const catalog = [
    {
      id: 'case:DEF_X',
      raw_key: 'DEF_X',
      label_zh: '参数 X',
      label_en: 'Parameter X',
      scope: 'case-scalar',
      storage: 'case-nml',
      section: '',
      value_kind: 'real',
      unit: null,
      calibration_eligible: true,
      default: '1',
      recommended_scale: 'linear',
      validation: { min: '0', max: '1', allowed_values: [], finite: true },
      visibility: 'editable-scientific',
      activation: [],
    },
  ] as unknown as Descriptor[];
  const render = (owner: string) =>
    renderToStaticMarkup(
      createElement(StudyPicks, { owner, catalog, en: true, disabled: false, onChange: () => {} }),
    );
  // 还没选参数时"加入"是灰的，但助手能看到它（不可用），不会误以为没有这个按钮。
  const empty = render('owner-empty');
  assert.ok(empty.includes('data-action="version-study-pick-add"'));
  assert.match(empty, /data-action="version-study-pick-add"[^>]*disabled/);
  // 已经选了参数时"移除"在界面上，但移除属于破坏性动作，不登记。
  store.set(
    'colm.versions.studyPicks.owner-picked',
    JSON.stringify([
      {
        parameter_id: 'case:DEF_X',
        raw_key: 'DEF_X',
        scheme: null,
        index: null,
        min: '0.5',
        max: '1.5',
        scale: 'linear',
      },
    ]),
  );
  const picked = render('owner-picked');
  assert.ok(picked.includes('>Remove<'));
  assert.doesNotMatch(picked, /data-action="[^"]*remove/i);
});

function pickFunctions<T>(names: string[], context: Record<string, unknown>): T {
  const source = ts.createSourceFile(
    'StudyPicks.tsx',
    readFileSync(new URL('./StudyPicks.tsx', import.meta.url), 'utf8'),
    ts.ScriptTarget.Latest,
    true,
    ts.ScriptKind.TSX,
  );
  const found = new Map<string, string>();
  const visit = (node: ts.Node) => {
    if (ts.isFunctionDeclaration(node) && node.name)
      found.set(node.name.text, node.getText(source).replace(/^export default /, ''));
    if (ts.isVariableDeclaration(node) && ts.isIdentifier(node.name) && node.initializer)
      found.set(node.name.text, `const ${node.name.text} = ${node.initializer.getText(source)};`);
    ts.forEachChild(node, visit);
  };
  visit(source);
  const code = ts.transpileModule(names.map((name) => found.get(name)).join('\n'), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  return new Function(...Object.keys(context), `${code}; return { ${names.join(', ')} };`)(
    ...Object.values(context),
  );
}

test('study plant types follow crop support and exclude type 16 without it', () => {
  for (const crops of [false, true]) {
    const { types } = pickFunctions<{ types: [number, string, string][] }>(['types'], {
      kind: 'pft',
      crops,
      typesOf,
    });
    assert.equal(
      types.some(([id]) => id === 16),
      crops,
    );
    assert.ok(types.some(([id]) => id === 15));
  }
});

test('late default-value responses merge current picks and cannot write into a different owner', async () => {
  const picksRef: { current: StudyPick[] } = { current: [] };
  const ownerEpoch = { current: 0 };
  const pending: (() => void)[] = [];
  const d = { ...descriptor(null, null), id: 'pft:x', raw_key: 'x', default: '2' };
  const context = {
    descriptor: d,
    disabled: false,
    typeProblem: '',
    ownerEpoch,
    kind: 'pft',
    typeIndex: 1,
    types: typesOf('pft', false),
    picksRef,
    literalNumber,
    defaultRange,
    setProblem: () => {},
    setPicks: () => {},
    savePicks: () => {},
    onChange: () => {},
    owner: 'A',
    t: (_zh: string, en: string) => en,
    invokeJson: () =>
      new Promise((resolve) => pending.push(() => resolve([{ index: 1, value: 2 }]))),
  };
  const { add } = pickFunctions<{ add: () => Promise<void> }>(['commit', 'add'], context);
  const first = add();
  const other: StudyPick = {
    parameter_id: 'other',
    raw_key: 'other',
    scheme: null,
    index: null,
    min: '1',
    max: '2',
    scale: 'linear',
  };
  picksRef.current = [other];
  pending.shift()!();
  await first;
  assert.deepEqual(
    picksRef.current.map((p) => p.parameter_id),
    ['other', 'pft:x'],
  );
  picksRef.current = [];
  const oldOwner = add();
  ownerEpoch.current++;
  pending.shift()!();
  await oldOwner;
  assert.deepEqual(
    picksRef.current,
    [],
    'a response for A cannot modify the newly selected version',
  );
});

test('disabling crops blocks the stale crop selection and choosing an available type restores Add', () => {
  const catalog = [
    { ...descriptor('0', '10'), id: 'pft:x', scope: 'pft-type', calibration_eligible: true },
  ];
  for (const [crops, index, blocked] of [
    [false, 16, true],
    [false, 15, false],
    [true, 16, false],
  ] as const) {
    let cursor = 0;
    const state = [[], 'pft:x', index, ''];
    const { StudyPicks: render } = pickFunctions<{
      StudyPicks: (props: unknown) => ReturnType<typeof createElement>;
    }>(['StudyPicks'], {
      React: { createElement },
      useState: () => [state[cursor++], () => {}],
      useRef: (value: unknown) => ({ current: value }),
      useEffect: () => {},
      parameterType,
      typesOf,
      Button,
      Input,
      Select,
      ActionButton,
    });
    const html = renderToStaticMarkup(
      render({
        owner: 'crop-switch',
        catalog,
        crops,
        en: true,
        disabled: false,
        onChange: () => {},
      }),
    );
    assert.equal(/data-action="version-study-pick-add"[^>]*disabled/.test(html), blocked);
    assert.equal(html.includes('This type is no longer available.'), blocked);
  }
});
