//! 运行方式页（工作流第 3 步的第一小步）、每页顶部的上下文条，以及结果页的出处标签。
//! 判断逻辑都在 run-mode.js；这里只接线与画界面。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { language, translateZh } from './i18n.js';
import { modelEngine } from './engine.js';
import { kernelForSubgrid } from './kernel.js';
import { go, renderSteps } from './shell.js';
import { openWorkspaces } from './workspace.js';
import { configuredServers } from './remote.js';
import { renderRunModeCards } from './run-mode-cards.js';
import {
  OFFICIAL, modelVersion, versionLabel, targetLabel, engineLabel, touchesText, readinessRows, contextChips,
  runModeBlocker,
} from './run-mode.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
const STORE_KEY = 'colm.modelVersion';

let workspaces = [];
let checkSeq = 0;

const runTarget = () => $('run-target')?.value || 'local';

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function storeVersion() {
  try { localStorage.setItem(STORE_KEY, modelVersion()); } catch { /* 存不了不影响本次使用 */ }
}

function storedVersion() {
  try { return localStorage.getItem(STORE_KEY) || OFFICIAL; } catch { return OFFICIAL; }
}

/** 选了修改版时交给 `version_check` 的正式内核（决定预设）。 */
export function officialKernelDir() {
  return kernelForSubgrid()?.dir ?? '';
}

// ---- 版本列表与检查 ---------------------------------------------------------------------------

export async function refreshVersions() {
  const select = $('model-version');
  if (!select) return;
  if (hasBackend) {
    try {
      workspaces = (await invoke('workspace_list'))?.workspaces ?? [];
    } catch (error) {
      workspaces = [];
      $('model-version-note').textContent = `${t('读不到修改版列表')}：${error?.message || error}`;
    }
  }
  const want = state.modelVersion ?? storedVersion();
  select.replaceChildren(new Option(t('官方版'), OFFICIAL));
  for (const ws of workspaces) select.appendChild(new Option(`${t('修改版')}：${ws.name}`, ws.name));
  const kept = [...select.options].some(o => o.value === want);
  select.value = kept ? want : OFFICIAL;
  if (!kept && want !== OFFICIAL) status(`${t('上次用的修改版已不存在，改回官方版')}：${want}`);
  state.modelVersion = select.value;
  await checkVersion();
}

/** 重新检查当前版本能不能按选定的引擎跑。官方版不用检查。 */
export async function checkVersion() {
  const version = modelVersion();
  const seq = ++checkSeq;
  if (version === OFFICIAL || !hasBackend) {
    state.versionReadiness = null;
    renderAll();
    return;
  }
  const kernel = officialKernelDir();
  if (!kernel) {
    state.versionReadiness = { name: version, ready: false, items: [{ code: 'no_kernel', ok: false, blocking: true, message: '还没有匹配这个算例配置的正式内核' }] };
    renderAll();
    return;
  }
  state.versionReadiness = null;
  renderAll();
  try {
    const readiness = await invoke('version_check', { name: version, kernel, engine: modelEngine() });
    if (seq !== checkSeq) return;
    state.versionReadiness = readiness;
  } catch (error) {
    if (seq !== checkSeq) return;
    state.versionReadiness = { name: version, ready: false, items: [{ code: 'check_failed', ok: false, blocking: true, message: `${t('检查失败')}：${error?.message || error}` }] };
  }
  renderAll();
}

// ---- 画界面 -----------------------------------------------------------------------------------

function renderVersionCard() {
  const card = $('version-card');
  const list = $('version-readiness');
  if (!card || !list) return;
  const version = modelVersion();
  if (version === OFFICIAL) {
    card.hidden = true;
    list.hidden = false;
    list.replaceChildren(el('li', { className: 'readiness-ok', textContent: t('✓ 使用随软件发布的官方版') }));
    return;
  }
  const ws = workspaces.find(w => w.name === version);
  const readiness = state.versionReadiness?.name === version ? state.versionReadiness : null;
  const facts = [
    ws && `${t('基于')} ${baseName(ws.origin)}`,
    ws && `${t('改了')} ${ws.commits} ${t('次')}`,
    readiness?.head && `${t('当前提交')} ${readiness.head.slice(0, 8)}`,
    readiness && t(touchesText(readiness.touches)),
  ].filter(Boolean);
  card.hidden = false;
  card.replaceChildren(
    el('b', { textContent: `${t('修改版')}：${version}` }),
    el('span', { className: 'muted mini', textContent: facts.join(' · ') || t('正在检查…') }),
  );
  list.hidden = false;
  if (!readiness) {
    list.replaceChildren(el('li', { className: 'readiness-wait', textContent: t('正在检查这个版本能不能运行…') }));
    return;
  }
  list.replaceChildren(...readinessRows(readiness, runTarget(), modelEngine()).map(row => {
    const li = el('li', { className: `readiness-${row.kind}` }, [
      el('span', { className: 'readiness-mark', textContent: row.kind === 'ok' ? '✓' : row.kind === 'warn' ? '!' : '✗' }),
      el('span', { textContent: t(row.text) }),
    ]);
    if (row.kind === 'block' && ['clean', 'engine_built', 'kernel_built'].includes(row.code)) {
      li.append(el('button', { className: 'btn-ghost mini', type: 'button', textContent: t('去改代码'), onclick: openWorkspaces }));
    }
    return li;
  }));
}

function blocker() {
  return runModeBlocker({ version: modelVersion(), target: runTarget(), readiness: state.versionReadiness, engine: modelEngine() });
}

function renderRunLine() {
  const line = $('run-mode-line');
  if (!line) return;
  const why = blocker();
  line.replaceChildren(
    el('span', { textContent: `${t('运行方式')}：${[versionLabel(modelVersion()), targetLabel(runTarget()), engineLabel(modelEngine())].map(t).join(' · ')}` }),
    el('button', { className: 'btn-ghost mini', type: 'button', textContent: t('改运行方式'), onclick: () => go('run-mode') }),
  );
  if (why) line.append(el('p', { className: 'warn mini', textContent: t(why) }));
}

export function renderContextBar() {
  const bar = $('context-bar');
  if (!bar) return;
  const focused = document.activeElement?.id;
  const cases = state.batch?.length ?? 0;
  const chips = contextChips({
    caseName: state.selected?.name,
    cases,
    version: modelVersion(),
    target: runTarget(),
    engine: modelEngine(),
  });
  const why = blocker();
  bar.replaceChildren(...chips.map(chip => {
    const label = el('span', { className: 'context-label', textContent: t(chip.label) });
    if (chip.key === 'case') return el('button', {
      type: 'button', className: 'context-chip context-case',
      title: state.selected?.dir || $('casename')?.value || t('当前算例'),
      onclick: () => go('basic-files'),
    }, [label, el('span', { className: 'context-value', textContent: t(chip.value) })]);
    const source = $({ version: 'model-version', target: 'run-target', engine: 'model-engine' }[chip.key]);
    const select = el('select', { id: `context-select-${chip.key}`, className: 'context-select', value: source?.value || '', title: t(chip.label) });
    select.setAttribute('aria-label', t(chip.label));
    for (const option of source?.options ?? []) select.append(new Option(option.textContent, option.value, false, option.selected));
    select.value = source?.value || '';
    select.disabled = !!source?.disabled;
    select.onchange = () => {
      source.value = select.value;
      source.dispatchEvent(new Event('change', { bubbles: true }));
    };
    return el('label', {
      className: `context-chip context-${chip.key}${chip.key === 'version' && modelVersion() !== OFFICIAL ? ' context-modified' : ''}`,
    }, [label, select]);
  }));
  if (why) bar.append(el('span', { className: 'context-warn', title: t(why), textContent: t('还不能运行') }));
  if (focused?.startsWith('context-select-')) $(focused)?.focus({ preventScroll: true });
}

function renderAll() {
  const cards = $('run-mode-choices');
  if (cards) renderRunModeCards(cards, {
    versions: workspaces, servers: configuredServers(),
    selection: { version: modelVersion(), target: runTarget(), engine: modelEngine() },
    onChange(key, value) {
      const source = $({ version: 'model-version', target: 'run-target', engine: 'model-engine' }[key]);
      if (!source || source.disabled || ![...source.options].some(option => option.value === value)) return;
      source.value = value;
      source.dispatchEvent(new Event('change', { bubbles: true }));
    },
  });
  const next = $('run-mode-next');
  if (next) {
    const why = blocker();
    next.disabled = !!why;
    next.title = why ? t(why) : '';
    next.textContent = t(why ? '先处理上面的 ✕ 项' : '下一步：输出与运行 →');
  }
  renderVersionCard();
  renderRunLine();
  renderContextBar();
  renderSteps(false);
}

// ---- 接线 -------------------------------------------------------------------------------------

function wire() {
  const select = $('model-version');
  if (!select) return;
  state.modelVersion = storedVersion();
  select.onchange = () => {
    state.modelVersion = select.value;
    storeVersion();
    checkVersion();
  };
  $('manage-versions')?.addEventListener('click', openWorkspaces);
  $('run-mode-next')?.addEventListener('click', () => { if (!blocker()) go('run'); });
  $('model-engine')?.addEventListener('change', () => checkVersion());
  $('run-target')?.addEventListener('change', renderAll);
  // 改代码页里新建、删除、重新编译或改了代码：版本列表与检查结果都要重来。
  addEventListener('colm:versions', () => refreshVersions().catch(status));
  // 改代码页的“用它运行”：选中这个版本，去运行方式。
  addEventListener('colm:use-version', event => {
    state.modelVersion = event.detail || OFFICIAL;
    storeVersion();
    go('run-mode');
  });
  addEventListener('colm:kernels', () => checkVersion());
  addEventListener('colm:wizard', () => checkVersion());
  addEventListener('colm:context', renderAll);
  addEventListener('colm:probe', renderAll);
  addEventListener('colm:step', () => {
    if (state.step === 'run-mode') refreshVersions().catch(status);
    else renderAll();
  });
  addEventListener('colm:language', renderAll);
  refreshVersions().catch(status);
}

wire();
