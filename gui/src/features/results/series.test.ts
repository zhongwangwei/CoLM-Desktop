import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import {
  boundedMap,
  fullEnergyStats,
  fullSeriesCsv,
  parentRuns,
  printReport,
  reportFile,
  type EnergyDiagnostic,
  type ResultReport,
} from './series';
describe('result scientific integrity', () => {
  it('uses backend full-record statistics rather than extrema-sampled means', () => {
    const report: EnergyDiagnostic = {
      schema: 1,
      expression: 'Rnet-H-LE-G',
      closure_assessed: false,
      full_stats: {
        basis: 'all_records_in_selected_window',
        mean_kind: 'arithmetic_mean_of_common_finite_records',
        status: 'computed',
        source_n: 10001,
        n: 10001,
        missing: 0,
        mean: 20000 / 10001,
        units: 'W/m²',
      },
      series: {
        time: [1, 5000, 10001],
        vars: { residual: [1, 10000, 1] },
        source_n: 10001,
        n: 3,
        downsampled: true,
      },
      window: { from: null, to: null, end_inclusive: true, time_basis: 'model_history_clock' },
      limitation: 'Storage is excluded',
    };
    assert.equal(fullEnergyStats(report).mean, 20000 / 10001);
    assert.notEqual(fullEnergyStats(report).mean, (1 + 10000 + 1) / 3);
    assert.throws(
      () =>
        fullEnergyStats({
          ...report,
          full_stats: { ...report.full_stats, basis: 'sampled' as never },
        }),
      /Full-record/,
    );
    assert.throws(
      () => fullEnergyStats({ ...report, series: { ...report.series, source_n: 3 } }),
      /support/,
    );
  });
  it('limits live readers and keeps source order', async () => {
    let active = 0,
      max = 0;
    const results = await boundedMap([3, 2, 1, 4, 5], 2, async (value) => {
      active++;
      max = Math.max(max, active);
      await new Promise((resolve) => setTimeout(resolve, value));
      active--;
      return value * 2;
    });
    assert.equal(max, 2);
    assert.deepEqual(results, [6, 4, 2, 8, 10]);
  });
  it('rejects a manufactured numeric mean without valid full-record support', () => {
    const invalid = {
      schema: 1,
      closure_assessed: false,
      full_stats: {
        basis: 'all_records_in_selected_window',
        mean_kind: 'arithmetic_mean_of_common_finite_records',
        status: 'computed',
        source_n: 2,
        n: 0,
        missing: 2,
        mean: 0,
        units: 'W/m²',
      },
      series: { source_n: 2 },
    } as EnergyDiagnostic;
    assert.throws(() => fullEnergyStats(invalid), /Invalid/);
  });
});

it('selects only successful exact parent revisions for comparison', () => {
  const runs = [
    { operation_id: 'a', version: { id: 'parent', revision: 1 }, provenance: { ok: true } },
    { operation_id: 'b', version: { id: 'parent', revision: 2 }, provenance: { ok: true } },
    { operation_id: 'c', version: { id: 'parent', revision: 1 }, provenance: { ok: false } },
    { version: { id: 'parent', revision: 1 }, provenance: { ok: true } },
  ];
  assert.deepEqual(parentRuns(runs, { id: 'parent', revision: 1 }), [runs[0]]);
  assert.deepEqual(parentRuns(runs, null), []);
});

it('exports every full-series record while preserving missing and negative numeric values', () => {
  const data = {
    time: [0, 60, 120],
    vars: { '=variable': [1, null, -2] },
    source_n: 3,
    n: 3,
    downsampled: false,
  };
  const text = fullSeriesCsv(data, '=variable');
  assert.equal(text.split('\r\n').filter(Boolean).length, 4);
  assert.ok(text.includes('"1970-01-01T00:01:00.000Z","60","\'=variable",""'));
  assert.ok(text.includes('"120","\'=variable","-2"'));
  assert.throws(() => fullSeriesCsv({ ...data, downsampled: true }, '=variable'), /sampled/);
  assert.throws(() => fullSeriesCsv({ ...data, source_n: 100 }, '=variable'), /every/);
  assert.throws(
    () => fullSeriesCsv({ ...data, vars: { '=variable': [1, NaN, -2] } }, '=variable'),
    /value/,
  );
});

const exportFixture: ResultReport = {
  generated_at: '2026-10-12T00:00:00Z',
  source: '/copy/<img src=x onerror=alert(1)>',
  provenance: { operation_id: 'run-1', note: '```</pre><script>alert(1)</script>' },
  catalog: { steps: 3 },
  evaluation: [],
  metrics: [
    {
      site: ' =HYPERLINK("bad")',
      name: 'flux|line\n<script>alert(1)</script>',
      n: 3,
      rmse: 1,
      mae: 0.5,
      bias: -0.2,
      r2: 0.81,
      correlation: -0.9,
      nse: 0.7,
      kge: 0.6,
      alpha: 1.2,
      beta: 0.8,
    },
  ],
  diagnostic: null,
  baseline: { same_design: false, better: null },
  baselineNote: 'Inputs differ',
  conditions: { obs: '/obs.csv', from: null, to: null, spinup: '0', corrected: false },
  notice: 'No ranking for unequal inputs',
};
it('retains backend metric values in all report formats and escapes active content', () => {
  assert.deepEqual(JSON.parse(reportFile(exportFixture, 'json', true).text), exportFixture);
  const csv = reportFile(exportFixture, 'csv', true).text;
  assert.ok(csv.includes('"correlation"'));
  assert.ok(csv.includes('"alpha","beta"'));
  assert.ok(csv.includes('"\' =HYPERLINK(""bad"")"'));
  assert.ok(csv.includes('"-0.9"'));
  assert.ok(!csv.includes('"\'-0.9"'));
  const markdown = reportFile(exportFixture, 'markdown', true).text;
  assert.ok(markdown.includes('flux\\|line<br>&lt;script&gt;'));
  assert.ok(markdown.includes('-0.9'));
  assert.equal(
    markdown.match(/^```/gm)?.length,
    2,
    'untrusted evidence cannot terminate a JSON code block',
  );
  const html = reportFile(exportFixture, 'html', false).text;
  assert.ok(html.includes('&lt;img'));
  assert.ok(html.includes('&lt;script&gt;'));
  assert.ok(!html.includes('<script>'));
  assert.ok(html.includes('correlation'));
  assert.ok(html.includes('-0.9'));
});

it('requests the real native or browser print path without claiming a PDF was saved', async () => {
  const keys = ['document', 'window', 'requestAnimationFrame'] as const;
  const previous = new Map(
    keys.map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)]),
  );
  type Article = { id: string; innerHTML: string; remove: () => void };
  let articles: Article[] = [];
  const listeners = new Map<string, () => void>();
  let nativeCalls = 0,
    browserCalls = 0;
  const document = {
    querySelector: () => articles.find((article) => article.id === 'print-report'),
    createElement: () => {
      const article: Article = {
        id: '',
        innerHTML: '',
        remove: () => {
          articles = articles.filter((value) => value !== article);
        },
      };
      return article;
    },
    body: { appendChild: (article: Article) => articles.push(article) },
  };
  Reflect.set(globalThis, 'document', document);
  Reflect.set(globalThis, 'window', {
    addEventListener: (name: string, fn: () => void) => listeners.set(name, fn),
    removeEventListener: (name: string) => listeners.delete(name),
    print: () => {
      browserCalls++;
    },
  });
  Reflect.set(globalThis, 'requestAnimationFrame', (fn: FrameRequestCallback) => {
    fn(0);
    return 1;
  });
  try {
    const native = await printReport(exportFixture, true, async () => {
      nativeCalls++;
    });
    assert.deepEqual(native, { status: 'printdialog_requested', format: 'pdf' });
    assert.equal(nativeCalls, 1);
    assert.equal(browserCalls, 0);
    assert.equal(articles.length, 1);
    assert.ok(!articles[0].innerHTML.includes('<script>'));
    listeners.get('afterprint')?.();
    assert.equal(articles.length, 0);
    assert.equal((await printReport(exportFixture, false)).status, 'printdialog_requested');
    assert.equal(browserCalls, 1);
    listeners.get('afterprint')?.();
    await assert.rejects(
      printReport(exportFixture, true, async () => {
        throw new Error('native print unavailable');
      }),
      /native print unavailable/,
    );
    assert.equal(articles.length, 0);
    assert.equal(listeners.size, 0);
  } finally {
    listeners.get('afterprint')?.();
    for (const key of keys) {
      const descriptor = previous.get(key);
      if (descriptor) Object.defineProperty(globalThis, key, descriptor);
      else Reflect.deleteProperty(globalThis, key);
    }
  }
});
