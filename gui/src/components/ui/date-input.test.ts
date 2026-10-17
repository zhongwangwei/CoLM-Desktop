import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import ts from 'typescript';
import { validDate } from '../../lib/dates';

function control(
  value: string,
  options: { disabled?: boolean; readOnly?: boolean; picker?: () => void } = {},
) {
  const elements: { type: string; props: Record<string, unknown> }[] = [];
  const changes: string[] = [],
    calls: string[] = [];
  let nativeVisible = false;
  const input = { focus: () => calls.push('focus'), showPicker: options.picker };
  const source = readFileSync(new URL('./index.tsx', import.meta.url), 'utf8');
  const start = source.indexOf('export function DateInput(');
  const end = source.indexOf('export function Select(', start);
  const code = ts.transpileModule(source.slice(start, end).replace('export ', ''), {
    compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
  }).outputText;
  const DateInput = new Function(
    'React',
    'Input',
    'Button',
    'useRef',
    'useState',
    'validDate',
    `${code}; return DateInput;`,
  )(
    {
      createElement: (type: string, props: Record<string, unknown>) => {
        const element = { type, props: props ?? {} };
        elements.push(element);
        return element;
      },
    },
    'Input',
    'Button',
    () => ({ current: input }),
    () => [
      nativeVisible,
      (value: boolean) => {
        nativeVisible = value;
      },
    ],
    validDate,
  );
  const render = () => {
    elements.length = 0;
    DateInput({
      id: 'start',
      value,
      calendarLabel: 'Choose start date',
      ...options,
      onChange: (event: { target: { value: string } }) => changes.push(event.target.value),
    });
  };
  render();
  return {
    render,
    changes,
    calls,
    text: () => elements.find((element) => element.type === 'Input')!.props,
    native: () => elements.find((element) => element.type === 'input')!.props,
    button: () => elements.find((element) => element.type === 'Button')!.props,
  };
}

test('date text remains intact while only valid ISO dates reach the native picker', () => {
  for (const value of ['', '2', '2001-', '2001-02-29', '0000-01-01']) {
    const input = control(value);
    assert.equal(input.text().value, value);
    assert.equal(input.text().type, 'text');
    assert.equal(input.text().id, 'start');
    assert.equal(input.native().value, '');
  }
  for (const value of ['0001-01-01', '2000-02-29', '2026-10-17', '9999-12-31'])
    assert.equal(control(value).native().value, value);
});

test('calendar opens during button activation and selected ISO dates use the normal setter', () => {
  let opened = 0;
  const input = control('2001-', {
    picker: () => {
      opened++;
    },
  });
  (input.button().onClick as () => void)();
  assert.equal(opened, 1);
  assert.deepEqual(input.calls, ['focus']);
  assert.equal(input.button()['aria-label'], 'Choose start date');
  (input.native().onChange as (event: unknown) => void)({ target: { value: '2001-03-04' } });
  (input.text().onChange as (event: unknown) => void)({ target: { value: '200' } });
  assert.deepEqual(input.changes, ['2001-03-04', '200']);
});

test('unavailable, blocked or silently ineffective picker APIs leave a reachable native control', () => {
  for (const picker of [
    undefined,
    () => {},
    () => {
      throw new Error('unsupported webview');
    },
  ]) {
    const input = control('2001-', { picker });
    (input.button().onClick as () => void)();
    input.render();
    assert.deepEqual(input.native().style, { flexBasis: '100%' });
    assert.equal(input.native().tabIndex, 0);
    assert.equal(input.text().value, '2001-');
    assert.deepEqual(input.changes, []);
    (input.native().onChange as (event: unknown) => void)({ target: { value: '2001-03-04' } });
    input.render();
    assert.equal(input.native().tabIndex, -1);
    assert.deepEqual(input.changes, ['2001-03-04']);
  }
});

test('disabled and read-only dates cannot open or expose a writable calendar', () => {
  for (const options of [{ disabled: true }, { readOnly: true }]) {
    let opened = 0;
    const input = control('2001-01-01', {
      ...options,
      picker: () => {
        opened++;
      },
    });
    assert.equal(input.button().disabled, true);
    (input.button().onClick as () => void)();
    assert.equal(opened, 0);
    assert.deepEqual(input.calls, []);
    assert.ok(input.native().disabled || input.native().readOnly);
  }
});
