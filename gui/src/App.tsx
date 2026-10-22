import { Suspense, lazy, useEffect, useRef, useState, type CSSProperties } from 'react';
import { assistantPanelWidth } from './lib/utils';
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
import appManifest from '../package.json';
import { GuideBinding, usePageContract } from './lib/guide';
import { Button, Notice } from './components/ui';
import AboutDialog, { betaNote } from './components/AboutDialog';
import ConfirmHost from './components/ConfirmHost';
import StartCover from './components/StartCover';
const ProjectPage = lazy(() => import('./features/projects/ProjectPage'));
const CasesPage = lazy(() => import('./features/cases/CasesPage'));
const NewCasePage = lazy(() => import('./features/cases/NewCasePage'));
const PreparationPage = lazy(() => import('./features/cases/PreparationPage'));
const VersionsPage = lazy(() => import('./features/versions/VersionsPage'));
const ResourcesPage = lazy(() => import('./features/resources/ResourcesPage'));
const HistoryPage = lazy(() => import('./features/history/HistoryPage'));
const ResearchPage = lazy(() => import('./features/studies/ResearchPage'));
const SettingsPage = lazy(() => import('./features/settings/SettingsPage'));
const AssistantPanel = lazy(() => import('./features/assistant/AssistantPanel'));
function Help() {
  const { state, navigate } = useApp();
  const en = state.language === 'en';
  usePageContract({
    title: en ? 'Help' : '帮助',
    step: 'help',
    fields: [],
    actions: [
      {
        id: 'help-example',
        label: en ? 'Try an example' : '试用示例',
        kind: 'click',
        run: () => navigate('new-case'),
      },
    ],
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
        <Button onClick={() => navigate('new-case')}>{en ? 'Try an example' : '试用示例'}</Button>
      </div>
    </div>
  );
}
export default function App() {
  const [assistantWidth, setAssistantWidth] = useState(() =>
    assistantPanelWidth(
      Number(localStorage.getItem('colm-ui-assistant-width')) || 320,
      window.innerWidth,
    ),
  );
  const drag = useRef<{ id: number; x: number; width: number } | null>(null);
  useEffect(() => {
    localStorage.setItem('colm-ui-assistant-width', String(assistantWidth));
  }, [assistantWidth]);
  useEffect(() => {
    const resize = () =>
      setAssistantWidth((width) => assistantPanelWidth(width, window.innerWidth));
    window.addEventListener('resize', resize);
    return () => window.removeEventListener('resize', resize);
  }, []);
  const { state, update, navigate } = useApp();
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
  const navButton = (item: (typeof navigation)[number]) => (
    <Button
      key={item.id}
      className={`nav-button ${active === item.id ? 'active' : ''}`}
      variant="ghost"
      aria-current={active === item.id ? 'page' : undefined}
      onClick={() => openNavigation(item)}
    >
      <NavIcon name={item.id} />
      {tr(item.zh, item.en)}
    </Button>
  );
  usePageContract({
    title: 'CoLM Desktop',
    step: state.route,
    scope: 'shell',
    fields: [],
    actions: [
      ...(state.root
        ? [
            {
              id: 'nav-current-project',
              label: tr('回到当前项目', 'Back to current project'),
              kind: 'click' as const,
              run: openCurrentProject,
            },
          ]
        : []),
      ...navigation.map((item) => ({
        id: `nav-${item.id}`,
        label: tr(item.zh, item.en),
        kind: 'click' as const,
        run: () => openNavigation(item),
      })),
    ],
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
    ) : state.route === 'research' ? null : state.route === 'settings' ? (
      <SettingsPage />
    ) : (
      <Help />
    );
  return (
    <>
      <StartCover />
      <GuideBinding />
      <AboutDialog fallbackVersion={appManifest.version} />
      <ConfirmHost />
      <div
        className={`desktop-shell ${state.assistantOpen ? 'assistant-visible' : ''}`}
        style={{ '--assistant-width': `${assistantWidth}px` } as CSSProperties}
      >
        <header className="app-toolbar" aria-label={tr('窗口操作', 'Window actions')}>
          <div className="brand">
            <img src="assets/colm-icon.png" alt="" />
            <strong>CoLM Desktop</strong>
          </div>
          <div className="toolbar-actions">
            <Button
              variant="ghost"
              aria-pressed={state.assistantOpen}
              onClick={() => update({ assistantOpen: !state.assistantOpen })}
            >
              {tr('AI 助手', 'AI assistant')}
            </Button>
            <Button
              variant="ghost"
              aria-label={tr('切换语言', 'Change language')}
              onClick={() => update({ language: en ? 'zh' : 'en' })}
            >
              {en ? '中文' : 'English'}
            </Button>
          </div>
        </header>
        <aside className="sidebar" aria-label={tr('主导航', 'Main navigation')}>
          {state.root ? (
            <button
              type="button"
              className={`current-project ${onCurrentProject ? 'active' : ''}`}
              title={state.root}
              aria-current={onCurrentProject ? 'page' : undefined}
              onClick={() => openCurrentProject()}
            >
              <span className="current-project-label">{tr('当前项目', 'Current project')}</span>
              <span className="current-project-name">{projectName(state.root, state.cases)}</span>
            </button>
          ) : (
            // 启动时不自动打开上次的项目：这里写明还没选，点一下去首页的"最近的项目"里选。
            <button
              type="button"
              className="current-project is-empty"
              onClick={() => openNavigation(navigation[0])}
            >
              <span className="current-project-label">{tr('当前项目', 'Current project')}</span>
              <span className="current-project-name">{tr('未选项目', 'None selected')}</span>
            </button>
          )}
          <nav className="sidebar-links">{navigation.slice(0, 5).map(navButton)}</nav>
          <div className="sidebar-bottom">
            <nav className="sidebar-links">{navigation.slice(5).map(navButton)}</nav>
            <div className="app-version" title={betaNote(en)}>
              <span>CoLM Desktop</span>
              <span>v{appManifest.version}</span>
              <span className="beta-badge">{tr('测试版', 'Beta')}</span>
            </div>
          </div>
        </aside>
        <main className="main-pane">
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
        </main>
        {state.assistantOpen && (
          <>
            <div
              className="assistant-splitter"
              role="separator"
              aria-label={tr('调整助手宽度', 'Resize assistant')}
              aria-orientation="vertical"
              aria-controls="assistant-pane"
              aria-valuemin={280}
              aria-valuemax={assistantPanelWidth(600, window.innerWidth)}
              aria-valuenow={assistantWidth}
              tabIndex={0}
              onPointerDown={(event) => {
                event.preventDefault();
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
                        ? 280
                        : event.key === 'End'
                          ? 600
                          : assistantWidth + (event.key === 'ArrowLeft' ? 16 : -16),
                      window.innerWidth,
                    ),
                  );
                }
              }}
            />
            <aside
              id="assistant-pane"
              className="assistant-pane"
              aria-label={tr('AI 助手', 'AI assistant')}
            >
              <Suspense fallback={<Notice>{tr('正在读取助手…', 'Loading assistant…')}</Notice>}>
                <AssistantPanel />
              </Suspense>
            </aside>
          </>
        )}
      </div>
    </>
  );
}
