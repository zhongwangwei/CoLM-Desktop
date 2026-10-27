import {
  Suspense,
  lazy,
  useEffect,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { assistantPanelWidth } from './lib/utils';
import {
  ASSISTANT_MAX_WIDTH,
  ASSISTANT_MIN_WIDTH,
  MIN_MAIN_WIDTH,
  drawerEscapeCloses,
  shellLayout,
} from './lib/shell-layout';
import { useApp } from './lib/app-state';
import {
  activeNavigation,
  currentProjectView,
  navigation,
  navigationTarget,
  projectName,
  readSavedProject,
} from './lib/navigation';
import NavIcon from './components/NavIcon';
import { ActionButton } from './components/ActionButton';
import appManifest from '../package.json';
import { GuideBinding, usePageContract } from './lib/guide';
import { Button, Notice } from './components/ui';
import AboutDialog, { betaNote } from './components/AboutDialog';
import ConfirmHost from './components/ConfirmHost';
import StartCover from './components/StartCover';
import { readStored, writeStored } from './lib/storage';
import type { AppState } from './lib/types';
const ProjectPage = lazy(() => import('./features/projects/ProjectPage'));
const CasesPage = lazy(() => import('./features/cases/CasesPage'));
const NewCasePage = lazy(() => import('./features/cases/NewCasePage'));
const PreparationPage = lazy(() => import('./features/cases/PreparationPage'));
const VersionsPage = lazy(() => import('./features/versions/VersionsPage'));
const ResourcesPage = lazy(() => import('./features/resources/ResourcesPage'));
const HistoryPage = lazy(() => import('./features/history/HistoryPage'));
const ResearchPage = lazy(() => import('./features/studies/ResearchPage'));
const SettingsPage = lazy(() => import('./features/settings/SettingsPage'));
const ComparePage = lazy(() => import('./features/compare/ComparePage'));
const AssistantPanel = lazy(() => import('./features/assistant/AssistantPanel'));
/** 侧栏收起成图标 rail 是长期偏好；专注视图只管本次会话。 */
const NAV_RAIL_KEY = 'colm-ui-nav-rail';
type Language = AppState['language'];
const shellText = (language: Language, zh: string, en: string) => (language === 'en' ? en : zh);
function Help() {
  const { state, navigate } = useApp();
  const en = state.language === 'en';
  usePageContract({
    title: en ? 'Help' : '帮助',
    step: 'help',
    fields: [],
    actions: [],
  });
  return (
    <div className="page-stack">
      <h1>{en ? 'Working with CoLM' : '使用 CoLM'}</h1>
      <p>
        {en
          ? 'Start with a bundled example, open a case, or prepare your own input data. The official model is selected by default.'
          : '可以直接试内置示例、打开算例，或准备自己的输入数据。默认使用官方模型。'}
      </p>
      <div className="card">
        <h2>{en ? 'A reproducible research loop' : '可追溯的研究流程'}</h2>
        <p>
          {en
            ? 'A case describes the place, period, input and output data. A model revision fixes its source code, parameters, physics and network. Each run keeps an independent output and its actual origin.'
            : '算例描述地点、时段、输入和输出。模型修订固定代码、参数、物理配置和网络。每次运行保留独立结果和实际出处。'}
        </p>
        <p>
          {en
            ? 'Use Results to evaluate outputs, Research to calibrate or train, Versions to review changes, and History to retrace operations.'
            : '在结果页评估输出，在研究页率定或学习，在版本页核对修改，在历史里追溯操作。'}
        </p>
        <ActionButton
          action="help-example"
          label={en ? 'Try an example' : '试用示例'}
          run={() => navigate('new-case')}
        />
      </div>
    </div>
  );
}

/** 侧栏内容（当前项目 + 主导航 + 版本号）。单独导出，让 U-06 的 rail/双语/aria 有静态渲染证据。 */
export function ShellNavigation({
  language,
  active,
  currentProject,
  onOpenNavigation,
  onOpenCurrentProject,
}: {
  language: Language;
  active: string | null;
  currentProject: { name: string; active: boolean; title?: string } | null;
  onOpenNavigation: (item: (typeof navigation)[number]) => void;
  onOpenCurrentProject: () => void;
}) {
  const tr = (zh: string, en: string) => shellText(language, zh, en);
  const navButton = (item: (typeof navigation)[number]) => (
    <ActionButton
      key={item.id}
      action={`nav-${item.id}`}
      label={tr(item.zh, item.en)}
      // rail 模式隐藏可见文字（.nav-label），所以名称同时给读屏和悬停提示。
      aria-label={tr(item.zh, item.en)}
      title={tr(item.zh, item.en)}
      className={`nav-button ${active === item.id ? 'active' : ''}`}
      variant="ghost"
      aria-current={active === item.id ? 'page' : undefined}
      run={() => onOpenNavigation(item)}
    >
      <NavIcon name={item.id} />
      <span className="nav-label">{tr(item.zh, item.en)}</span>
    </ActionButton>
  );
  return (
    <>
      {currentProject ? (
        <ActionButton
          bare
          action="nav-current-project"
          label={tr('回到当前项目', 'Back to current project')}
          className={`current-project ${currentProject.active ? 'active' : ''}`}
          title={currentProject.title}
          aria-current={currentProject.active ? 'page' : undefined}
          run={onOpenCurrentProject}
        >
          <span className="current-project-label">{tr('当前项目', 'Current project')}</span>
          <span className="current-project-name">{currentProject.name}</span>
        </ActionButton>
      ) : (
        // 启动时不自动打开上次的项目：这里写明还没选，点一下去首页的"最近的项目"里选。
        <button type="button" className="current-project is-empty" onClick={onOpenCurrentProject}>
          <span className="current-project-label">{tr('当前项目', 'Current project')}</span>
          <span className="current-project-name">{tr('未选项目', 'None selected')}</span>
        </button>
      )}
      <nav className="sidebar-links">{navigation.slice(0, 6).map(navButton)}</nav>
      <div className="sidebar-bottom">
        <nav className="sidebar-links">{navigation.slice(6).map(navButton)}</nav>
        <div className="app-version" title={betaNote(language === 'en')}>
          <span>CoLM Desktop</span>
          <span>v{appManifest.version}</span>
          <span className="beta-badge">{tr('测试版', 'Beta')}</span>
        </div>
      </div>
    </>
  );
}

/**
 * 桌面壳：工具栏 + 侧栏/主区/助手三个区域。布局全部来自 `shellLayout()` 的结果，这里只套类名，
 * 方便在无真实浏览器的测试里用 react-dom/server 断言各尺寸下的类名、aria 与显隐。
 */
export function AppShell({
  layout,
  language,
  assistantOpen,
  navCollapsed,
  focusMode,
  sidebar,
  main,
  splitter,
  assistant,
  onToggleNav,
  onToggleFocus,
  onToggleAssistant,
  onToggleLanguage,
  onCloseAssistant,
}: {
  layout: ReturnType<typeof shellLayout>;
  language: Language;
  assistantOpen: boolean;
  navCollapsed: boolean;
  focusMode: boolean;
  sidebar: ReactNode;
  main: ReactNode;
  splitter?: ReactNode;
  assistant?: ReactNode;
  onToggleNav: () => void;
  onToggleFocus: () => void;
  onToggleAssistant: () => void;
  onToggleLanguage: () => void;
  onCloseAssistant: () => void;
}) {
  const tr = (zh: string, en: string) => shellText(language, zh, en);
  const shellClass = [
    'desktop-shell',
    `shell-nav-${layout.nav}`,
    layout.assistant === 'column'
      ? 'assistant-column'
      : layout.assistant === 'drawer'
        ? 'assistant-drawer'
        : '',
    focusMode ? 'shell-focus' : '',
  ]
    .filter(Boolean)
    .join(' ');
  return (
    <>
      <div
        className={shellClass}
        style={
          {
            '--sidebar-width': `${layout.sidebarWidth}px`,
            '--assistant-width': `${layout.assistantWidth}px`,
          } as CSSProperties
        }
      >
        <header className="app-toolbar" aria-label={tr('窗口操作', 'Window actions')}>
          <div className="brand">
            <img src="assets/colm-icon.png" alt="" />
            <strong>CoLM Desktop</strong>
          </div>
          <div className="toolbar-actions">
            <Button
              variant="ghost"
              className="toolbar-toggle"
              aria-pressed={navCollapsed}
              aria-controls={layout.nav === 'hidden' ? undefined : 'app-sidebar'}
              aria-expanded={layout.nav === 'full'}
              disabled={layout.nav === 'hidden'}
              onClick={onToggleNav}
            >
              {navCollapsed
                ? tr('展开导航', 'Expand navigation')
                : tr('收起导航', 'Collapse navigation')}
            </Button>
            <Button
              variant="ghost"
              className="toolbar-toggle"
              aria-pressed={focusMode}
              onClick={onToggleFocus}
            >
              {focusMode ? tr('退出专注视图', 'Exit focus view') : tr('专注视图', 'Focus view')}
            </Button>
            <Button variant="ghost" aria-pressed={assistantOpen} onClick={onToggleAssistant}>
              {tr('AI 助手', 'AI assistant')}
            </Button>
            <Button
              variant="ghost"
              aria-label={tr('切换语言', 'Change language')}
              onClick={onToggleLanguage}
            >
              {language === 'en' ? '中文' : 'English'}
            </Button>
          </div>
        </header>
        {layout.nav !== 'hidden' && (
          <aside id="app-sidebar" className="sidebar" aria-label={tr('主导航', 'Main navigation')}>
            {sidebar}
          </aside>
        )}
        <main className="main-pane">{main}</main>
        {layout.splitter ? splitter : null}
        {assistant != null && (
          <aside
            id="assistant-pane"
            className="assistant-pane"
            aria-label={tr('AI 助手', 'AI assistant')}
            hidden={!assistantOpen || layout.assistant === 'hidden'}
          >
            {assistant}
          </aside>
        )}
      </div>
      {layout.assistant === 'drawer' && assistantOpen && (
        // 覆盖式抽屉没有分栏列可让路：点主区任意处即可收起（键盘另有 Escape 与工具栏开关）。
        <button
          type="button"
          className="assistant-drawer-backdrop"
          aria-label={tr('关闭助手', 'Close assistant')}
          // 键盘收起走 Escape 与工具栏开关，遮罩只是鼠标/触屏的快捷方式，不占 Tab 顺序。
          tabIndex={-1}
          onClick={onCloseAssistant}
        />
      )}
    </>
  );
}

export default function App() {
  const [viewport, setViewport] = useState(() => window.innerWidth);
  const [assistantWidth, setAssistantWidth] = useState(() =>
    assistantPanelWidth(Number(readStored('colm-ui-assistant-width')) || 320, window.innerWidth),
  );
  // 收起导航是长期偏好；专注视图不跨重启，避免下次打开软件时找不到导航。
  const [navCollapsed, setNavCollapsed] = useState(() => readStored(NAV_RAIL_KEY) === 'true');
  const [focusMode, setFocusMode] = useState(false);
  const drag = useRef<{ id: number; x: number; width: number } | null>(null);
  useEffect(() => {
    writeStored('colm-ui-assistant-width', String(assistantWidth));
  }, [assistantWidth]);
  useEffect(() => {
    writeStored(NAV_RAIL_KEY, navCollapsed ? 'true' : '');
  }, [navCollapsed]);
  useEffect(() => {
    const resize = () => {
      setViewport(window.innerWidth);
      setAssistantWidth((width) => assistantPanelWidth(width, window.innerWidth));
    };
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, []);
  const { state, update, navigate } = useApp();
  // 助手面板收起时保持挂载：否则正在跑的回答会被卸载 cleanup 取消，对话也随组件状态丢掉。
  // 第一次打开才加载，之后用 hidden 收起，和 researchKept 同一做法。
  const [assistantMounted, setAssistantMounted] = useState(state.assistantOpen);
  useEffect(() => {
    if (state.assistantOpen) setAssistantMounted(true);
  }, [state.assistantOpen]);
  // U-06：布局完全由纯函数按窗口宽度与三个开关算出（窄窗口助手改抽屉、专注视图隐藏两栏）。
  const layout = shellLayout({
    viewportWidth: viewport,
    assistantOpen: state.assistantOpen,
    focusMode,
    navCollapsed,
    assistantWidth,
  });
  // 抽屉打开时按 Escape 收起；对话框自己处理 Escape 时会 preventDefault，这里不抢。
  useEffect(() => {
    if (layout.assistant !== 'drawer' || !state.assistantOpen) return;
    const onKey = (event: KeyboardEvent) => {
      if (drawerEscapeCloses(event, true)) update({ assistantOpen: false });
    };
    document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [layout.assistant, state.assistantOpen, update]);
  const en = state.language === 'en';
  const tr = (zh: string, english: string) => (en ? english : zh);
  const active = activeNavigation(state);
  const openNavigation = (item: (typeof navigation)[number]) => {
    const target = navigationTarget(item, state);
    if (!navigate(target.route, target.tab)) return false;
    if (target.projectView) update({ projectView: target.projectView, projectCasesOpen: false });
    if (target.researchHome) update({ researchHome: true });
    return true;
  };
  // Wherever the user is, one click returns to the project being worked on, on its last page.
  const onCurrentProject =
    state.route === 'project' && state.projectView?.root === state.root && !state.projectView?.home;
  const openCurrentProject = () => {
    if (!state.root) return false;
    let saved = null;
    try {
      saved = readSavedProject(localStorage);
    } catch {
      /* Storage may be unavailable; the project still opens on its case list. */
    }
    const view = currentProjectView(state, saved);
    if (!navigate('project')) return false;
    update({ projectView: view, projectCasesOpen: false });
    return true;
  };
  // 工具栏开关：专注视图里点助手会先退出专注，再展开助手，免得按了没反应。
  const toggleAssistant = () => {
    if (focusMode) {
      setFocusMode(false);
      update({ assistantOpen: true });
      return;
    }
    update({ assistantOpen: !state.assistantOpen });
  };
  usePageContract({
    title: 'CoLM Desktop',
    step: state.route,
    scope: 'shell',
    fields: [],
    // 侧栏按钮自己登记（第 737 轮）。
    actions: [],
  });
  // 研究页打开过一次就常驻：本机研究一跑几小时，离开研究页不能把它（和它的进度）一起卸掉。
  const [researchKept, setResearchKept] = useState(false);
  if (state.route === 'research' && !researchKept) setResearchKept(true);
  const content =
    state.route === 'project' ? (
      <ProjectPage />
    ) : state.route === 'case' ? (
      <CasesPage />
    ) : state.route === 'new-case' ? (
      <NewCasePage />
    ) : state.route === 'preparation' ? (
      <PreparationPage />
    ) : state.route === 'versions' ? (
      <VersionsPage />
    ) : state.route === 'resources' ? (
      <ResourcesPage />
    ) : state.route === 'history' ? (
      <HistoryPage />
    ) : state.route === 'research' ? null : state.route === 'compare' ? (
      <ComparePage />
    ) : state.route === 'settings' ? (
      <SettingsPage />
    ) : (
      <Help />
    );
  const sidebar = (
    <ShellNavigation
      language={state.language}
      active={active}
      currentProject={
        state.root
          ? {
              name: projectName(state.root, state.cases),
              active: onCurrentProject,
              title: state.root,
            }
          : null
      }
      onOpenNavigation={openNavigation}
      onOpenCurrentProject={() =>
        state.root ? openCurrentProject() : openNavigation(navigation[0])
      }
    />
  );
  const main = (
    <>
      <div className="notifications" aria-live="polite">
        {state.notices.map((message, index) => (
          <Notice
            key={`${index}:${message}`}
            tone={state.errorNotices?.includes(message) ? 'error' : 'info'}
          >
            {message}
          </Notice>
        ))}
        {state.notices.length > 0 && (
          <Button variant="ghost" onClick={() => update({ notices: [], errorNotices: [] })}>
            {tr('收起提示', 'Dismiss')}
          </Button>
        )}
      </div>
      <Suspense fallback={<Notice>{tr('正在打开页面…', 'Opening page…')}</Notice>}>
        {content}
        {researchKept && (
          <div hidden={state.route !== 'research'}>
            <ResearchPage active={state.route === 'research'} />
          </div>
        )}
      </Suspense>
    </>
  );
  const assistant = assistantMounted ? (
    <Suspense fallback={<Notice>{tr('正在读取助手…', 'Loading assistant…')}</Notice>}>
      <AssistantPanel />
    </Suspense>
  ) : null;
  // 拆分条只在分栏模式出现；宽度上限由 shellLayout 夹住，拖到头也不会把主区压到 MIN_MAIN_WIDTH 以下。
  // aria-valuemax 用同一夹法算出来：原来沿用旧的 assistantPanelWidth，读屏报的可调上限会大于实际能到的宽度。
  const columnMax = Math.max(
    ASSISTANT_MIN_WIDTH,
    Math.min(ASSISTANT_MAX_WIDTH, window.innerWidth - layout.sidebarWidth - MIN_MAIN_WIDTH),
  );
  const splitter = layout.splitter ? (
    <div
      className="assistant-splitter"
      role="separator"
      aria-label={tr('调整助手宽度', 'Resize assistant')}
      aria-orientation="vertical"
      aria-controls="assistant-pane"
      aria-valuemin={ASSISTANT_MIN_WIDTH}
      aria-valuemax={columnMax}
      aria-valuenow={layout.assistantWidth}
      aria-valuetext={`${layout.assistantWidth} px`}
      tabIndex={0}
      onPointerDown={(event) => {
        event.preventDefault();
        event.currentTarget.focus({ preventScroll: true });
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { id: event.pointerId, x: event.clientX, width: assistantWidth };
      }}
      onPointerMove={(event) => {
        if (drag.current?.id === event.pointerId)
          setAssistantWidth(
            assistantPanelWidth(
              drag.current.width + drag.current.x - event.clientX,
              window.innerWidth,
            ),
          );
      }}
      onPointerUp={(event) => {
        if (drag.current?.id === event.pointerId) {
          drag.current = null;
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }
      }}
      onPointerCancel={() => {
        drag.current = null;
      }}
      onLostPointerCapture={() => {
        drag.current = null;
      }}
      onKeyDown={(event) => {
        if (['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) {
          event.preventDefault();
          setAssistantWidth(
            assistantPanelWidth(
              event.key === 'Home'
                ? ASSISTANT_MIN_WIDTH
                : event.key === 'End'
                  ? ASSISTANT_MAX_WIDTH
                  : assistantWidth + (event.key === 'ArrowLeft' ? 16 : -16),
              window.innerWidth,
            ),
          );
        }
      }}
    />
  ) : null;
  return (
    <>
      <StartCover />
      <GuideBinding />
      <AboutDialog fallbackVersion={appManifest.version} />
      <ConfirmHost />
      <AppShell
        layout={layout}
        language={state.language}
        assistantOpen={state.assistantOpen}
        navCollapsed={navCollapsed}
        focusMode={focusMode}
        sidebar={sidebar}
        main={main}
        splitter={splitter}
        assistant={assistant}
        onToggleNav={() => setNavCollapsed((value) => !value)}
        onToggleFocus={() => setFocusMode((value) => !value)}
        onToggleAssistant={toggleAssistant}
        onToggleLanguage={() => update({ language: en ? 'zh' : 'en' })}
        onCloseAssistant={() => update({ assistantOpen: false })}
      />
    </>
  );
}
