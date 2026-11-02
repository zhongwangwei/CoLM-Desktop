import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { askConfirm, setConfirmHost } from '../../lib/confirm';
import * as model from './model';
import type { Overlay } from '../../lib/types';
import { parameterType, recordType, type Descriptor } from './model';
import { IGBP, PFT, USGS } from './scope-labels';
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { docOf, literalNumber, rangeText, valueProblem, typesOf } =
  await import('./ParameterDialog');

const descriptor = (patch: Partial<Descriptor>): Descriptor => ({
  id: 'case:DEF_X',
  raw_key: 'DEF_X',
  label_zh: 'x',
  label_en: 'x',
  scope: 'case-scalar',
  storage: 'case-nml',
  section: '',
  value_kind: 'real',
  unit: null,
  visibility: 'editable-expert',
  activation: [],
  default: null,
  doc_zh: '',
  doc_en: '',
  validation: { min: '0', max: '1', allowed_values: [], finite: true },
  ...patch,
});

test('each parameter goes to the editor of its kind and records map back to the same kind', () => {
  assert.equal(parameterType(descriptor({})), 'global');
  assert.equal(
    parameterType(descriptor({ id: 'lct:USGS:DEF_LC_VMAX25', scope: 'land-cover-class' })),
    'usgs',
  );
  assert.equal(
    parameterType(descriptor({ id: 'lct:IGBP:DEF_LC_VMAX25', scope: 'land-cover-class' })),
    'igbp',
  );
  assert.equal(parameterType(descriptor({ scope: 'pft-type', storage: 'pft-override' })), 'pft');
  assert.equal(
    parameterType(descriptor({ scope: 'pc-pft-component', storage: 'pc-pft-override' })),
    'pc',
  );
  assert.equal(
    parameterType(descriptor({ scope: 'process-file', storage: 'process-parameter-file' })),
    'process',
  );
  const record = (kind: string, scheme?: string) =>
    ({
      parameter_id: 'x',
      raw_key: 'x',
      value: '1',
      unit: null,
      scope_instance: { kind, scheme },
    }) as Parameters<typeof recordType>[0];
  assert.equal(recordType(record('land-cover-class', 'USGS')), 'usgs');
  assert.equal(recordType(record('land-cover-class', 'IGBP')), 'igbp');
  assert.equal(recordType(record('pc-pft')), 'pc');
  assert.equal(recordType(record('case-scalar')), 'global');
});

test('namelist numbers are read in Fortran spellings and checked against the catalog range', () => {
  assert.equal(literalNumber('1.5d-3'), 0.0015);
  assert.equal(literalNumber('0.01_r8'), 0.01);
  assert.equal(literalNumber('.5'), 0.5);
  assert.equal(literalNumber('abc'), null);
  assert.equal(literalNumber(''), null);
  const d = descriptor({});
  assert.equal(valueProblem(d, '0.3', false), '');
  assert.equal(valueProblem(d, '1.5', false), '不能大于 1');
  assert.equal(valueProblem(d, '-1', true), 'Below 0');
  assert.equal(valueProblem(d, 'x', false), '不是数');
  assert.equal(valueProblem(descriptor({ value_kind: 'integer' }), '0.5', false), '须为整数');
  assert.equal(valueProblem(descriptor({ value_kind: 'logical' }), '.TRUE.', false), '');
  assert.equal(
    valueProblem(descriptor({ value_kind: 'logical' }), 'yes', false),
    '只能是 .true. 或 .false.',
  );
});

test('ranges read as > 0 or < 0 instead of the spelled-out smallest double', () => {
  // The catalog writes "must be > 0" as the smallest positive double, 300-odd digits long.
  const tiny = `0.${'0'.repeat(307)}22250738585072014`;
  const sqrtdi = descriptor({
    validation: { min: tiny, max: null, allowed_values: [], finite: true },
  });
  const psi50 = descriptor({
    validation: { min: null, max: `-${tiny}`, allowed_values: [], finite: true },
  });
  assert.equal(rangeText(sqrtdi, false), '> 0');
  assert.equal(rangeText(psi50, false), '< 0');
  assert.equal(rangeText(descriptor({}), false), '0 … 1');
  assert.equal(
    rangeText(
      descriptor({ validation: { min: '1.e3_r8', max: null, allowed_values: [], finite: true } }),
      true,
    ),
    '≥ 1000',
  );
  assert.equal(
    rangeText(
      descriptor({ validation: { min: null, max: null, allowed_values: [], finite: true } }),
      false,
    ),
    '',
  );
  assert.equal(valueProblem(sqrtdi, '0', false), '必须大于 0');
  assert.equal(valueProblem(psi50, '0', true), 'Must be below 0');
  assert.equal(valueProblem(sqrtdi, '5', false), '');
});

test('placeholder docs that only repeat the key are not shown', () => {
  const generic = descriptor({
    doc_zh: 'CoLM 配置字段 DEF_LC_PSI50_ROOT。',
    doc_en: 'CoLM configuration field DEF_LC_PSI50_ROOT.',
  });
  assert.equal(docOf(generic, false), '');
  assert.equal(docOf(generic, true), '');
  const real = descriptor({ doc_zh: '根50%失导水势；按所选 PFT/CFT 槽位稀疏覆盖。' });
  assert.equal(docOf(real, false), '根50%失导水势；按所选 PFT/CFT 槽位稀疏覆盖。');
});

test('a parameter holding several numbers is checked number by number', () => {
  // DEF_SEDIMENT%grain_diameter: one diameter per grain-size class.
  const diameter = descriptor({
    validation: { min: '0', max: null, allowed_values: [], finite: true },
  });
  assert.equal(valueProblem(diameter, '2.0e-5, 2.0e-4, 2.0e-3', false), '');
  assert.equal(valueProblem(diameter, '2.0e-5, x', false), '不是数');
  assert.equal(valueProblem(diameter, '2.0e-5, -1', false), '不能小于 0');
  assert.equal(valueProblem(diameter, '2.0e-5 2.0e-4 2.0e-3', false), '');
});

// 0 起编号里 16（灌溉 C3）已经是作物：不显示作物时最多到 15，和后端、ConfigurationOptions 一致。
test('plant types without crops stop before the first crop type', () => {
  const ids = typesOf('pft', false).map(([id]) => id);
  assert.equal(Math.max(...ids), 15);
  assert.ok(typesOf('pft', true).some(([id]) => id === 16));
});

// 内核要 GRADM > 1.6：开区间端点显示成 "> 1.6"，1.6 本身不能填。
test('open bounds from the kernel rules are shown and enforced', () => {
  const gradm = {
    value_kind: 'real',
    validation: {
      min: '1.6',
      max: null,
      min_exclusive: true,
      allowed_values: [],
      finite: true,
    },
  } as unknown as Descriptor;
  assert.equal(rangeText(gradm, false), '> 1.6');
  assert.match(valueProblem(gradm, '1.6', false), /必须大于 1.6/);
  assert.equal(valueProblem(gradm, '1.7', false), '');
});

/**
 * 对话框里的按钮由它们自己登记（第 737 轮）。这里把源码拿出来执行一次，收集返回元素树上的
 * ActionButton 与 Dialog 的 closeAction；不可折叠、不是动作的列表项按钮不应该出现在里面。
 */
type DialogNode = { type: unknown; props: Record<string, unknown>; children: unknown[] };
function dialogActions(
  editable: boolean,
  withImport: boolean,
  initialType = 'igbp',
  overrides: Record<string, unknown> = {},
  inspect: (
    nodes: DialogNode[],
    rerender: (props?: Record<string, unknown>) => DialogNode[],
  ) => void = () => {},
  withEffects = false,
) {
  const source = readFileSync(new URL('./ParameterDialog.tsx', import.meta.url), 'utf8')
    .replace(/^import\b[\s\S]*?;$/gm, '')
    .replace('export default function', 'function')
    .replace(/^export /gm, '');
  const js = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const slots: unknown[] = [];
  let cursor = 0;
  let effectCursor = 0;
  const effectDeps: unknown[][] = [];
  const pendingEffects: (() => void)[] = [];
  const descriptor = (id: string, rawKey: string): Descriptor =>
    ({
      id,
      raw_key: rawKey,
      label_zh: rawKey,
      label_en: rawKey,
      scope: 'land-cover-class',
      storage: 'land-cover-override',
      section: '水热过程',
      value_kind: 'real',
      unit: null,
      visibility: 'editable-scientific',
      activation: [],
      default: '1',
      doc_zh: '',
      doc_en: '',
      validation: { min: '0', max: '1', allowed_values: [], finite: true },
    }) as unknown as Descriptor;
  const env: Record<string, unknown> = {
    ...model,
    askConfirm,
    IGBP,
    USGS,
    PFT,
    React: {
      createElement: (
        type: unknown,
        props: Record<string, unknown> | null,
        ...children: unknown[]
      ) => {
        const merged = props ?? {};
        if (type === 'ActionButton')
          return {
            type: 'Button',
            props: { ...merged, onClick: merged.run },
            children: children.length ? children : [String(merged.label)],
          };
        return { type, props: merged, children };
      },
      Fragment: 'Fragment',
    },
    ActionButton: 'ActionButton',
    Dialog: 'Dialog',
    Input: 'Input',
    Select: 'Select',
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = initial;
      return [
        slots[index],
        (next: unknown) => {
          slots[index] = typeof next === 'function' ? next(slots[index]) : next;
        },
      ];
    },
    useRef: (initial: unknown) => {
      const index = cursor++;
      if (!(index in slots)) slots[index] = { current: initial };
      return slots[index];
    },
    useEffect: (effect: () => void, deps: unknown[]) => {
      if (!withEffects) return;
      const index = effectCursor++;
      if (!effectDeps[index] || deps.some((value, i) => !Object.is(value, effectDeps[index][i]))) {
        effectDeps[index] = deps;
        pendingEffects.push(effect);
      }
    },
    useMemo: (fn: () => unknown) => fn(),
    invokeJson: async () => [],
    useRenderedAction: () => {},
  };
  const factory = new Function(...Object.keys(env), `${js};return ParameterDialog;`)(
    ...Object.values(env),
  ) as (props: Record<string, unknown>) => {
    type: unknown;
    props: Record<string, unknown>;
    children: unknown[];
  };
  const props = {
    open: true,
    onOpenChange: () => {},
    catalog: [
      descriptor('lct:IGBP:DEF_LC_X', 'DEF_LC_X'),
      descriptor('lct:IGBP:DEF_LC_Y', 'DEF_LC_Y'),
    ],
    overlay: { physics: {}, parameters: [], process_files: [], hybrid: null },
    editable,
    en: false,
    onChange: () => {},
    initialType,
    onImportProcessFile: withImport ? () => {} : undefined,
    ...overrides,
  };
  const ids: string[] = [];
  const nodes: DialogNode[] = [];
  const walk = (node: unknown) => {
    if (Array.isArray(node)) return node.forEach(walk);
    if (!node || typeof node !== 'object') return;
    const element = node as { type: unknown; props: Record<string, unknown>; children: unknown[] };
    nodes.push(element);
    if (element.type === 'Dialog' && element.props.closeAction)
      ids.push(String(element.props.closeAction));
    if (element.type === 'Button' && typeof element.props.action === 'string')
      ids.push(element.props.action);
    walk(element.children);
  };
  const rerender = (next: Record<string, unknown> = {}) => {
    Object.assign(props, next);
    cursor = 0;
    effectCursor = 0;
    nodes.length = 0;
    ids.length = 0;
    walk(factory(props));
    if (pendingEffects.length) {
      pendingEffects.splice(0).forEach((effect) => effect());
      return rerender();
    }
    return nodes;
  };
  rerender();
  inspect(nodes, rerender);
  return ids;
}

test('the parameter dialog registers its own controls and closes through closeAction', () => {
  const ids = dialogActions(true, true);
  for (const id of [
    'version-parameters-close',
    'version-parameters-tab-global',
    'version-parameters-tab-process',
    'version-parameters-mode-parameter',
    'version-parameters-mode-class',
  ])
    assert.ok(ids.includes(id), id);
  assert.equal(new Set(ids).size, ids.length, '同时显示的动作 id 不能重复');
  // 材料与泥沙页签可以把专家自己的参数文件整份带进版本（写盘动作，按 commit 登记）。
  const process = dialogActions(true, true, 'process');
  assert.ok(process.includes('version-import-process'));
  assert.equal(new Set(process).size, process.length);
  // 冻结版本只读：不能把自己的参数文件导进来。
  assert.equal(dialogActions(false, false, 'process').includes('version-import-process'), false);
});

test('typing commits valid parameter values before close, while invalid text blocks close', () => {
  const changes: { parameters: { value: string }[] }[] = [];
  const closed: boolean[] = [];
  const pending: boolean[] = [];
  dialogActions(
    true,
    false,
    'igbp',
    {
      onChange: (next: (typeof changes)[number]) => changes.push(next),
      onOpenChange: (open: boolean) => closed.push(open),
      onPendingChange: (value: boolean) => pending.push(value),
    },
    (nodes) => {
      const input = nodes.find(
        (node) => node.type === 'input' && node.props.className === 'param-input',
      )!;
      const change = input.props.onChange as (event: { target: { value: string } }) => void;
      const close = nodes.find((node) => node.type === 'Dialog')!.props.onOpenChange as (
        open: boolean,
      ) => void;
      change({ target: { value: '0.5' } });
      assert.equal(changes.at(-1)?.parameters[0].value, '0.5');
      close(false);
      assert.deepEqual(closed, [false], 'closing does not need a blur to commit');
      change({ target: { value: '0.5e' } });
      close(false);
      assert.deepEqual(
        closed,
        [false],
        'incomplete input is retained instead of silently discarded',
      );
      assert.equal(pending.at(-1), true);
      change({ target: { value: '0.5e0' } });
      close(false);
      assert.equal(pending.at(-1), false);
      assert.deepEqual(closed, [false, false]);
      assert.equal(changes.at(-1)?.parameters[0].value, '0.5e0');
    },
  );
});

test('changed-only global rows stay mounted while clearing and replacing the focused value', () => {
  const x = descriptor({});
  const y = descriptor({ id: 'case:DEF_Y', raw_key: 'DEF_Y' });
  let overlay: Overlay = {
    physics: { DEF_X: '0.5', DEF_Y: '0.6' },
    parameters: [],
    process_files: [],
    hybrid: null,
  };
  dialogActions(
    true,
    false,
    'global',
    {
      catalog: [x, y],
      overlay,
      onChange: (next: Overlay) => {
        overlay = next;
      },
    },
    (initial, rerender) => {
      const fire = (node: DialogNode, name: string, value?: string) => {
        (
          node.props[name] as
            ((event: { target: { value?: string; checked: boolean } }) => void) | undefined
        )?.({ target: { value, checked: true } });
      };
      fire(
        initial.find((node) => node.type === 'input' && node.props.type === 'checkbox')!,
        'onChange',
      );
      const render = () => rerender({ overlay });
      const input = (nodes: DialogNode[], key: string) =>
        nodes.find((node) => node.type === 'input' && node.props['aria-label'] === `${key} 0`);
      let nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onFocus');
      nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onChange', '');
      nodes = render();
      assert.ok(
        input(nodes, 'DEF_X'),
        'clearing a focused changed value must not unmount its input',
      );
      assert.equal(input(nodes, 'DEF_X')!.props.value, '');
      assert.equal(overlay.physics.DEF_X, undefined, 'clearing still writes through immediately');
      fire(input(nodes, 'DEF_X')!, 'onChange', '0.7');
      nodes = render();
      assert.equal(overlay.parameters[0]?.value, '0.7', 'replacement writes through before blur');
      fire(input(nodes, 'DEF_X')!, 'onBlur');
      nodes = render();
      assert.ok(input(nodes, 'DEF_X'), 'replacement stays in changed-only results');

      fire(input(nodes, 'DEF_X')!, 'onFocus');
      nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onChange', '');
      nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onChange', '-');
      nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onBlur');
      nodes = render();
      assert.ok(input(nodes, 'DEF_X'), 'invalid input stays visible after blur under changed-only');
      assert.equal(input(nodes, 'DEF_X')!.props['aria-invalid'], true);
      assert.equal(input(nodes, 'DEF_X')!.props.value, '-');
      fire(input(nodes, 'DEF_X')!, 'onFocus');
      nodes = render();
      fire(input(nodes, 'DEF_X')!, 'onChange', '');
      nodes = render();
      const second = input(nodes, 'DEF_Y')!;
      fire(input(nodes, 'DEF_X')!, 'onBlur');
      fire(second, 'onFocus');
      nodes = render();
      assert.equal(input(nodes, 'DEF_X'), undefined, 'a reset row leaves results after blur');
      fire(input(nodes, 'DEF_Y')!, 'onChange', '');
      nodes = render();
      assert.ok(input(nodes, 'DEF_Y'), 'the next focused row is retained independently');
      fire(input(nodes, 'DEF_Y')!, 'onBlur');
      nodes = render();
      assert.equal(input(nodes, 'DEF_Y'), undefined);
    },
  );
});

test('discarding invalid text confirms close and preserves the last valid overlay', async () => {
  let answer: ((ok: boolean) => void) | undefined;
  setConfirmHost((request) => {
    answer = request.resolve;
  });
  let close!: (open: boolean) => Promise<void>;
  let change!: (event: { target: { value: string } }) => void;
  let render!: (props?: Record<string, unknown>) => DialogNode[];
  const closed: boolean[] = [];
  const pending: boolean[] = [];
  const changes: Overlay[] = [];
  try {
    dialogActions(
      true,
      false,
      'igbp',
      {
        onChange: (next: Overlay) => changes.push(next),
        onOpenChange: (open: boolean) => closed.push(open),
        onPendingChange: (value: boolean) => pending.push(value),
      },
      (nodes, rerender) => {
        change = nodes.find(
          (n) => n.type === 'input' && String(n.props.className).startsWith('param-input'),
        )!.props.onChange as typeof change;
        close = nodes.find((n) => n.type === 'Dialog')!.props.onOpenChange as typeof close;
        render = rerender;
      },
    );
    change({ target: { value: '0.5' } });
    const saved = changes[0];
    change({ target: { value: '0.5e' } });
    const cancelled = close(false);
    assert.equal(typeof answer, 'function', 'invalid text offers a discard confirmation');
    answer!(false);
    await cancelled;
    assert.deepEqual(closed, []);
    assert.equal(pending.at(-1), true);
    const nodes = render({ overlay: saved });
    assert.equal(
      nodes.find((n) => n.type === 'input' && String(n.props.className).startsWith('param-input'))!
        .props.value,
      '0.5e',
    );
    const discarded = close(false);
    answer!(true);
    await discarded;
    assert.deepEqual(closed, [false]);
    assert.equal(pending.at(-1), false);
    assert.equal(changes.length, 1, 'discard must not replace the valid overlay');
    assert.equal(saved.parameters[0].value, '0.5');
  } finally {
    setConfirmHost(null);
  }
});

test('switching parameter tabs keeps invalid input visible in the problem notice', () => {
  dialogActions(
    true,
    false,
    'igbp',
    {},
    (initial, rerender) => {
      const input = (nodes: DialogNode[]) =>
        nodes.find((node) => node.type === 'input' && node.props.className === 'param-input')!;
      const notice = (nodes: DialogNode[]) =>
        nodes.find((node) => node.props.className === 'param-problem')?.children.join('');
      (input(initial).props.onChange as (event: { target: { value: string } }) => void)({
        target: { value: '0.5e' },
      });
      let nodes = rerender();
      const error = notice(nodes);
      assert.match(error!, /DEF_LC_X/);
      (
        nodes.find((node) => node.props.action === 'version-parameters-tab-global')!.props
          .onClick as () => void
      )();
      nodes = rerender();
      assert.equal(notice(nodes), error, 'an invalid cell on another tab still needs attention');
      (
        nodes.find((node) => node.props.action === 'version-parameters-tab-igbp')!.props
          .onClick as () => void
      )();
      nodes = rerender();
      assert.equal(input(nodes).props.value, '0.5e');
      assert.equal(input(nodes).props['aria-invalid'], true);
      (input(nodes).props.onChange as (event: { target: { value: string } }) => void)({
        target: { value: '0.5' },
      });
      assert.equal(notice(rerender()), undefined, 'correcting the input removes the notice');
      (input(rerender()).props.onChange as (event: { target: { value: string } }) => void)({
        target: { value: '-' },
      });
      rerender({ open: false });
      nodes = rerender({ open: true });
      assert.equal(notice(nodes), undefined, 'reopening resets the discarded input and its notice');
      assert.notEqual(input(nodes).props.value, '-');
    },
    true,
  );
});
