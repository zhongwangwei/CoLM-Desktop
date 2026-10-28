import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createElement as h } from 'react';
import { renderToStaticMarkup } from 'react-dom/server';
import {
  ASSISTANT_DRAWER_BELOW,
  ASSISTANT_MAX_WIDTH,
  ASSISTANT_MIN_WIDTH,
  MIN_MAIN_WIDTH,
  NAV_WIDTH_NARROW,
  NAV_WIDTH_RAIL,
  NAV_WIDTH_WIDE,
  drawerEscapeCloses,
  shellLayout,
  type ShellLayoutInput,
} from './shell-layout';

// App 的共享 UI 模块在导入时读已保存偏好；测试没有 localStorage，只让它读到"没存过"。
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: { getItem: () => null, setItem: () => {}, removeItem: () => {} },
});
const { AppShell, ShellNavigation } = await import('../App');

const layout = (input: ShellLayoutInput) => shellLayout(input);
const at = (width: number, options: Partial<ShellLayoutInput> = {}) =>
  layout({ viewportWidth: width, assistantOpen: false, ...options });

test('960 / 1240 / 1440 wide windows decide assistant column, drawer and main width', () => {
  // 审查实测：960 展开助手时主区曾被挤到约 432px；这里给出修复后的结果（含 640/820/900 高度；
  // 高度只影响纵向滚动，横向列宽由下面的数值决定，工具栏固定 44px 见 styles.css）。
  for (const { label, width } of [
    { label: '960x640', width: 960 },
    { label: '1240x820', width: 1240 },
    { label: '1440x900', width: 1440 },
  ]) {
    const closed = at(width, { assistantOpen: false });
    const narrow = width <= 1240;
    assert.equal(closed.nav, 'full', label);
    assert.equal(closed.sidebarWidth, narrow ? NAV_WIDTH_NARROW : NAV_WIDTH_WIDE, label);
    assert.equal(closed.assistant, 'hidden', label);
    assert.equal(closed.mainWidth, width - closed.sidebarWidth, label);
    assert.equal(closed.splitter, false, label);

    const open = at(width, { assistantOpen: true, assistantWidth: 320 });
    assert.equal(open.splitter, width > ASSISTANT_DRAWER_BELOW, `${label} splitter only in column`);
    if (width <= ASSISTANT_DRAWER_BELOW) {
      assert.equal(open.assistant, 'drawer', `${label} narrow windows use the overlay drawer`);
      assert.equal(open.assistantWidth, 320, label);
      assert.equal(
        open.mainWidth,
        width - open.sidebarWidth,
        `${label} drawer must not squeeze main`,
      );
    } else {
      assert.equal(open.assistant, 'column', `${label} wide windows keep the split column`);
      assert.equal(open.assistantWidth, 320, label);
      assert.equal(open.mainWidth, width - open.sidebarWidth - 320, label);
      assert.ok(open.mainWidth >= MIN_MAIN_WIDTH, `${label} main keeps its minimum width`);
    }
  }
  // 三个基准点原样写出来，方便和审查数字对照：1240 展开助手时主区 712，1440 时 888。
  assert.equal(at(1240, { assistantOpen: true }).mainWidth, 712);
  assert.equal(at(1440, { assistantOpen: true }).mainWidth, 888);
});

test('the assistant drawer starts at 1100px and a column never leaves the main area below 560px', () => {
  assert.equal(at(1100, { assistantOpen: true }).assistant, 'drawer');
  assert.equal(at(1100, { assistantOpen: true }).mainWidth, 1100 - NAV_WIDTH_NARROW);
  const column = at(1101, { assistantOpen: true, assistantWidth: 320 });
  assert.equal(column.assistant, 'column');
  assert.equal(column.mainWidth, 1101 - NAV_WIDTH_NARROW - 320);
  assert.ok(column.mainWidth >= MIN_MAIN_WIDTH);
  // 拖宽助手时夹住列宽，而不是把主区压到 560 以下；拖动条还在，能再拖回去。
  for (const requested of [320, 480, 600, 900]) {
    for (const width of [1101, 1240, 1440, 1920]) {
      const result = at(width, { assistantOpen: true, assistantWidth: requested });
      assert.equal(result.assistant, 'column');
      assert.ok(result.mainWidth >= MIN_MAIN_WIDTH, `${width}px window, ${requested}px assistant`);
      assert.ok(
        result.assistantWidth >= ASSISTANT_MIN_WIDTH &&
          result.assistantWidth <= ASSISTANT_MAX_WIDTH,
      );
      assert.equal(
        result.sidebarWidth + result.assistantWidth + result.mainWidth,
        width,
        'column widths add up exactly to the viewport; no horizontal overflow',
      );
    }
  }
  assert.equal(at(1240, { assistantOpen: true, assistantWidth: 600 }).assistantWidth, 472);
  assert.equal(at(1240, { assistantOpen: true, assistantWidth: 600 }).mainWidth, MIN_MAIN_WIDTH);
});

test('navigation rail keeps the main area on both narrow and wide windows', () => {
  const rail = at(960, { assistantOpen: true, navCollapsed: true });
  assert.equal(rail.nav, 'rail');
  assert.equal(rail.sidebarWidth, NAV_WIDTH_RAIL);
  assert.equal(rail.mainWidth, 960 - NAV_WIDTH_RAIL, 'collapsing the navigation returns 144px');
  const railColumn = at(1240, { assistantOpen: true, navCollapsed: true });
  assert.equal(railColumn.nav, 'rail');
  assert.equal(railColumn.mainWidth, 1240 - NAV_WIDTH_RAIL - 320);
});

test('assistant width inputs are clamped and invalid sizes do not produce negative layout', () => {
  assert.equal(at(1440, { assistantOpen: true, assistantWidth: 1000 }).assistantWidth, 600);
  assert.equal(at(1440, { assistantOpen: true, assistantWidth: 10 }).assistantWidth, 280);
  assert.equal(at(1440, { assistantOpen: true, assistantWidth: Number.NaN }).assistantWidth, 320);
  assert.equal(at(1440, { assistantOpen: true, assistantWidth: -50 }).assistantWidth, 280);
  // 960px 抽屉里最宽也只到"窗口减一个 rail"，不会把右侧内容推出屏幕。
  assert.ok(
    at(960, { assistantOpen: true, assistantWidth: 600 }).assistantWidth <= 960 - NAV_WIDTH_RAIL,
  );
  const tiny = at(480, { assistantOpen: true, assistantWidth: 600 });
  assert.equal(tiny.assistant, 'drawer');
  assert.ok(tiny.mainWidth >= 0);
  assert.ok(tiny.assistantWidth <= 480);
  const broken = at(Number.NaN, { assistantOpen: true });
  assert.ok(broken.mainWidth >= 0);
  assert.equal(broken.sidebarWidth, NAV_WIDTH_NARROW);
});

test('below 232/208px the full navigation stays selected while the column math still adds up', () => {
  for (const width of [960, 1024, 1100, 1101, 1240, 1280, 1440, 1920]) {
    const closed = at(width);
    assert.equal(closed.sidebarWidth + closed.mainWidth, width);
    const rail = at(width, { navCollapsed: true });
    assert.equal(rail.sidebarWidth, NAV_WIDTH_RAIL);
    assert.equal(rail.sidebarWidth + rail.mainWidth, width);
  }
});

test('Escape only closes the drawer when no dialog already handled the key', () => {
  assert.equal(drawerEscapeCloses({ key: 'Escape' }, true), true);
  assert.equal(drawerEscapeCloses({ key: 'Escape' }, false), false);
  assert.equal(drawerEscapeCloses({ key: 'Enter' }, true), false);
  // Radix 对话框在 document 捕获阶段处理 Escape 时会 preventDefault：抽屉不能跟着关。
  assert.equal(drawerEscapeCloses({ key: 'Escape', defaultPrevented: true }, true), false);
});

test('styles.css exposes the U-06 classes without shrinking fonts', () => {
  const css = readFileSync(new URL('../styles.css', import.meta.url), 'utf8');
  // 列宽由 App 写入的 CSS 变量决定，两个响应式旧规则必须让位，否则会与计算值打架。
  assert.doesNotMatch(css, /assistant-visible/);
  assert.match(css, /\.desktop-shell\s*\{[^}]*grid-template-columns:\s*var\(--sidebar-width/);
  assert.match(css, /\.desktop-shell\.assistant-column\s*\{[^}]*var\(--assistant-width/);
  assert.match(
    css,
    /\.desktop-shell\.assistant-drawer\s+\.assistant-pane\s*\{[^}]*position:\s*fixed/,
  );
  assert.match(css, /\.assistant-drawer-backdrop\s*\{[^}]*position:\s*fixed/);
  assert.match(css, /\.desktop-shell\.shell-nav-rail\s+\.nav-label\s*\{[^}]*display:\s*none/);
  // 没有独立的"专注视图"：布局只由导航收起与助手开关两个状态决定。
  assert.doesNotMatch(css, /shell-focus/);
  // 主区自身必须允许内部滚动/收缩，否则固定宽列的页面内容会让整个窗口横向溢出。
  assert.match(css, /\.main-pane\s*\{[^}]*min-width:\s*0/);
  assert.match(css, /\.main-pane\s*\{[^}]*overflow:\s*auto/);
  // 深色主题下遮罩/抽屉的阴影与蒙层要换一套颜色，沿用现有 data-theme 机制。
  assert.match(css, /\[data-theme='dark'\] \.desktop-shell\.assistant-drawer \.assistant-pane/);
  assert.match(css, /\[data-theme='dark'\] \.assistant-drawer-backdrop/);
  // U-06 只改列宽与显隐：新增规则里不得靠缩小字号适应窗口。
  const start = css.indexOf('/* U-06 桌面壳变体');
  const end = css.indexOf('/* --- U-06 桌面壳变体结束 --- */');
  assert.ok(start >= 0 && end > start, 'U-06 style block is marked for review');
  const block = css.slice(start, end);
  assert.doesNotMatch(block, /font-size|font:\s/);
});

function renderShell(
  options: {
    width?: number;
    assistantOpen?: boolean;
    navCollapsed?: boolean;
    language?: 'zh' | 'en';
    assistantWidth?: number;
  } = {},
) {
  const viewportWidth = options.width ?? 1240;
  const computed = shellLayout({
    viewportWidth,
    assistantOpen: options.assistantOpen ?? false,
    navCollapsed: options.navCollapsed ?? false,
    assistantWidth: options.assistantWidth ?? 320,
  });
  return renderToStaticMarkup(
    h(AppShell, {
      layout: computed,
      language: options.language ?? 'zh',
      assistantOpen: options.assistantOpen ?? false,
      navCollapsed: options.navCollapsed ?? false,
      sidebar: h('div', { className: 'sidebar-content' }, 'sidebar'),
      main: h('div', { className: 'main-content' }, 'main'),
      splitter: h('div', { className: 'assistant-splitter', role: 'separator' }),
      assistant: h('div', { className: 'assistant-content' }, 'assistant'),
      onToggleNav: () => {},
      onToggleAssistant: () => {},
      onToggleLanguage: () => {},
      onCloseAssistant: () => {},
    }),
  );
}

test('AppShell renders the computed mode classes, CSS variables and toolbar toggles', () => {
  const wide = renderShell({ width: 1440, assistantOpen: true });
  assert.match(wide, /class="desktop-shell shell-nav-full assistant-column"/);
  assert.match(wide, /--sidebar-width:232px/);
  assert.match(wide, /--assistant-width:320px/);
  assert.match(wide, /aria-label="主导航"/);
  assert.match(wide, /aria-label="AI 助手"/);
  assert.doesNotMatch(wide, /aria-label="Main navigation"/, 'Chinese shell by default');
  // 工具按钮是原生 button，键盘可达。
  assert.match(
    wide,
    /<button[^>]*class="button button-ghost toolbar-toggle"[^>]*>收起导航<\/button>/,
  );
  assert.doesNotMatch(wide, />专注视图<\/button>/, 'only one workspace-width switch');
  assert.match(wide, /aria-pressed="true"[^>]*>AI 助手<\/button>/);

  const narrow = renderShell({ width: 960, assistantOpen: true });
  assert.match(narrow, /class="desktop-shell shell-nav-full assistant-drawer"/);
  assert.match(narrow, /class="assistant-drawer-backdrop"/);
  assert.doesNotMatch(narrow, /assistant-splitter/, 'drawer mode has no splitter');
  assert.match(narrow, /aria-label="关闭助手"/);
  // 抽屉打开时助手不是 hidden；主区自己仍在 DOM 里。
  assert.match(narrow, /id="assistant-pane"[^>]*aria-label="AI 助手"/);
  assert.doesNotMatch(narrow, /id="assistant-pane"[^>]*hidden/);
});

test('AppShell hides the splitter and shows the drawer only for overlay layouts', () => {
  const narrowOpen = renderShell({ width: 960, assistantOpen: true });
  assert.match(narrowOpen, /assistant-drawer-backdrop/);
  const narrowClosed = renderShell({ width: 960, assistantOpen: false });
  assert.doesNotMatch(narrowClosed, /assistant-drawer-backdrop/);
  assert.match(narrowClosed, /id="assistant-pane"[^>]*hidden/);
  const wideColumn = renderShell({ width: 1440, assistantOpen: true });
  assert.match(wideColumn, /class="assistant-splitter"/);
  assert.doesNotMatch(wideColumn, /assistant-drawer-backdrop/);
});

test('navigation rail marks the shell, keeps keyboard names and exposes bilingual labels', () => {
  const zh = renderShell({ width: 960, assistantOpen: true, navCollapsed: true });
  assert.match(zh, /class="desktop-shell shell-nav-rail assistant-drawer"/);
  assert.match(zh, /aria-pressed="true"[^>]*>展开导航<\/button>/);
  assert.match(zh, /aria-controls="app-sidebar"[^>]*aria-expanded="false"/);
  const en = renderShell({ width: 1240, language: 'en', navCollapsed: true });
  assert.match(en, />Expand navigation<\/button>/);
  assert.match(en, />AI assistant<\/button>/);
  assert.match(en, /aria-label="Main navigation"/);
});

test('ShellNavigation keeps each navigation entry named for the icon rail', () => {
  const markup = renderToStaticMarkup(
    h(ShellNavigation, {
      language: 'zh',
      active: 'simulation',
      currentProject: { name: 'CN-Cng', active: false, title: '/cases/CN-Cng' },
      onOpenNavigation: () => {},
      onOpenCurrentProject: () => {},
    }),
  );
  // rail 模式用 CSS 隐藏 .nav-label，名称必须仍在 aria-label/title 上，读屏和悬停都能用。
  assert.match(markup, /<span class="nav-label">模拟<\/span>/);
  assert.match(markup, /aria-label="模拟"/);
  assert.match(markup, /title="模拟"/);
  assert.match(markup, /aria-current="page"/);
  // 用户没选项目时仍是原生按钮，点一下回项目首页。
  const empty = renderToStaticMarkup(
    h(ShellNavigation, {
      language: 'en',
      active: 'project',
      currentProject: null,
      onOpenNavigation: () => {},
      onOpenCurrentProject: () => {},
    }),
  );
  assert.match(empty, /class="current-project is-empty"/);
  assert.match(empty, /None selected/);
});
