import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';
import ts from 'typescript';
import { chartTimeRange, sameSeries } from './chart-data';

const text = readFileSync(new URL('./ResultsPage.tsx', import.meta.url), 'utf8');
const source = ts.createSourceFile(
  'ResultsPage.tsx',
  text,
  ts.ScriptTarget.Latest,
  true,
  ts.ScriptKind.TSX,
);
const chart = source.statements.find(
  (node): node is ts.FunctionDeclaration =>
    ts.isFunctionDeclaration(node) && node.name?.text === 'ScienceChart',
);
assert.ok(chart);
const code = ts.transpileModule(chart.getText(source).replace('export function', 'function'), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
}).outputText;

test('the compact paired overview draws its only actual valid pair', () => {
  let expression = '';
  function visit(node: ts.Node) {
    if (
      ts.isJsxAttribute(node) &&
      node.name.getText(source) === 'showPoints' &&
      node.initializer &&
      ts.isJsxExpression(node.initializer) &&
      node.initializer.expression?.getText(source).includes('selectedMetric')
    ) {
      expression = node.initializer.expression.getText(source);
    }
    ts.forEachChild(node, visit);
  }
  visit(source);
  assert.ok(expression);
  const points = new Function('selectedMetric', `return ${expression};`) as (metric: {
    n: number;
    pair_gaps_truncated: boolean;
  }) => boolean;
  assert.equal(points({ n: 1, pair_gaps_truncated: false }), true);
  assert.equal(points({ n: 2, pair_gaps_truncated: false }), false);
  assert.equal(points({ n: 0, pair_gaps_truncated: false }), false);
  assert.equal(points({ n: 20, pair_gaps_truncated: true }), true);
});

test('the actual chart bounds a single clock value and keeps truncated-gap samples visible', () => {
  for (const showPoints of [false, true]) {
    let refs = 0;
    let options:
      | {
          scales: {
            x: { time: boolean; range: (plot: unknown, min: number, max: number) => number[] };
          };
          series: { points?: { show: boolean } }[];
        }
      | undefined;
    class Plot {
      constructor(value: typeof options) {
        options = value;
      }
      setSize() {}
      setData() {}
      destroy() {}
    }
    const environment = {
      chartTimeRange,
      sameSeries,
      timeTicks: () => [],
      uPlot: Plot,
      useRef: (value: unknown) => ({ current: refs++ === 0 ? { clientWidth: 600 } : value }),
      useEffect: (effect: () => unknown) => effect(),
      ResizeObserver: class {
        observe() {}
        disconnect() {}
      },
      React: { createElement: () => null },
    };
    const render = new Function(...Object.keys(environment), `${code};return ScienceChart;`)(
      ...Object.values(environment),
    );
    render({
      data: [[1704067200], [2]],
      labels: ['Simulation'],
      theme: 'light',
      compact: true,
      showPoints,
    });
    assert.ok(options);
    assert.equal(options.scales.x.time, true);
    assert.deepEqual(
      options.scales.x.range(null, 1704067200, 1790467200),
      [1704024000, 1704110400],
    );
    assert.equal(options.series[1].points?.show, showPoints);
  }
});
