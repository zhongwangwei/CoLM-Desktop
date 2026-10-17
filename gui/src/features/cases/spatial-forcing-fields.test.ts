import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null },
});
const { default: SpatialForcingFields } = await import('./SpatialForcingFields');

function render(currentDataset: string | undefined, failed = false) {
  return renderToStaticMarkup(
    createElement(SpatialForcingFields, {
      directory: '/forcing',
      dataset: '',
      datasets: ['MSWX'],
      currentDataset,
      probe: {
        status: 'done',
        candidates: ['MSWX'],
        ...(failed
          ? { coverage_error: 'time inspection failed' }
          : { coverage: { start: '2003-01-02', end: '2003-01-30' } }),
      },
      catalogError: '',
      disabled: false,
      en: false,
      onDirectoryChange: () => {},
      onDirectoryBlur: () => {},
      onPick: () => {},
      onDatasetChange: () => {},
      onDetect: () => {},
    }),
  );
}

test('template coverage suggestions appear only for new cases, including unknown existing datasets', () => {
  assert.match(render(undefined), /2003-01-02 — 2003-01-30/);
  for (const dataset of ['', 'CUSTOM']) {
    const markup = render(dataset);
    assert.doesNotMatch(markup, /2003-01-02|2003-01-30/);
    assert.match(markup, /已识别文件布局/);
  }
});

test('existing cases do not receive template-only manual-date errors', () => {
  assert.match(render(undefined, true), /time inspection failed/);
  for (const dataset of ['', 'CUSTOM']) {
    assert.doesNotMatch(render(dataset, true), /time inspection failed|请手动设置模拟日期/);
  }
});
