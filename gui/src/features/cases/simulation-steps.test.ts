import test from 'node:test';
import assert from 'node:assert/strict';
import { createElement } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';

// 向导步骤导航是纯展示组件；这里用静态渲染验证重命名与"需重检"标记确实出现在界面上。
const { default: SimulationSteps } = await import('./SimulationSteps');

const render = (props: Record<string, unknown>) =>
  renderToStaticMarkup(createElement(SimulationSteps, props as never));

test('the wizard navigation shows the clearer first step name and recheck markers', () => {
  const markup = render({
    step: 3,
    en: false,
    actionPrefix: 'new',
    onChange: () => {},
    recheck: { 2: '计算引擎改变，需要重新检查并行布局。' },
  });
  assert.match(markup, /运行环境与保存位置/, 'the first step uses its clearer name');
  assert.match(markup, /需重检/);
  assert.match(markup, /title="计算引擎改变，需要重新检查并行布局。"/);
  assert.match(markup, /aria-label="范围与网格 — 计算引擎改变，需要重新检查并行布局。"/);
  // 只有失效的那一步带标记。
  assert.equal((markup.match(/需重检/g) ?? []).length, 1);
  // 旧的名称不再出现。
  assert.doesNotMatch(markup, /总的设定/);
});

test('the recheck hint follows the interface language', () => {
  const markup = render({
    step: 2,
    en: true,
    actionPrefix: 'new',
    onChange: () => {},
    recheck: { 1: 'The engine changed.' },
  });
  assert.match(markup, /Run environment and save location/);
  assert.match(markup, /recheck/);
  assert.match(markup, /The engine changed\./);
  assert.doesNotMatch(markup, /总的设定/);
});

test('a step that still needs checking is marked with an exclamation marker', () => {
  const markup = render({
    step: 2,
    en: false,
    actionPrefix: 'new',
    onChange: () => {},
    recheck: { 2: '换成城市方案后站点要重新核对。' },
  });
  // 当前步骤的圆形序号被 "!" 取代，提示用户先重新检查而不是继续。
  const markers = [...markup.matchAll(/<span class="step-marker"[^>]*>([^<]*)<\/span>/g)].map(
    (match) => match[1],
  );
  // 前面的步骤是 ✓；失效的当前步骤是 "!"；后面的步骤仍是序号。
  assert.deepEqual(markers, ['✓', '!', '3', '4', '5']);
});
