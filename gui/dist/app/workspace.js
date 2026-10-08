//! 开发工作区面板（docs/design-ai-assistant.md 第 5、8 节）：助手改 CoLM 源码、编译、测试、对照的地方。
//! 实际工作都由 `colm-cli ws-*` 做；这里显示各工作区的状态灯，并提供采纳、导出补丁、回滚、删除。
//!
//! 这四个按钮是 **D 级**：只在这里有入口，助手的工具里没有对应的工具，模型调不到。
//!
//! 只依赖 ipc/state/ui/i18n。kernel.js 读 `state.adoptedKernels`（采纳过的实验内核），反过来不依赖本模块。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, appConfirm, appPrompt, baseName } from './ui.js';
import { language, translateZh } from './i18n.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
const STORE_KEY = 'colm.adoptedKernels';

// ---- 纯函数（tests/workspace.mjs）--------------------------------------------------------------

const LIGHT_TEXT = {
  pass: '通过',
  fail: '不通过',
  stale: '过期（之后又有新提交）',
  unknown: '还没测过',
};

const LIGHT_NAMES = { compile: '编译', tests: '测试', regression: '回归', parity: '对齐' };

/** 一盏灯的文字。 */
export function lightText(light) {
  return t(LIGHT_TEXT[light] ?? LIGHT_TEXT.unknown);
}

/** 四盏灯的摘要，每盏灯一段，用点号隔开。 */
export function lightsSummary(lights) {
  return Object.entries(LIGHT_NAMES)
    .map(([key, name]) => `${t(name)} ${lightText(lights?.[key])}`)
    .join(' · ');
}

/** 这个工作区有没有可采纳的实验内核。 */
export function adoptableKernels(kernels, workspace) {
  return (kernels ?? []).filter(k => k.workspace === workspace);
}

/** 实验内核在匹配用的内核表里的样子（和 `list_kernels` 的条目同形，另带 `experimental` 标记）。 */
export function kernelEntry(experimental) {
  return {
    preset: experimental.preset,
    dir: experimental.dir,
    generator_args: experimental.generator_args ?? '',
    macros: experimental.macros ?? [],
    colm_git_sha: experimental.colm_git_sha ?? '',
    platform: experimental.platform ?? '',
    experimental: { workspace: experimental.workspace, label: experimental.label, head: experimental.head, reviewed: false },
  };
}

/** 采纳过的内核里，只留下仍然登记着的（工作区又有了新提交、门槛作废了的要去掉）。 */
export function stillRegistered(adopted, registered) {
  return (adopted ?? []).filter(a =>
    (registered ?? []).some(k => k.workspace === a.experimental?.workspace && k.preset === a.preset && k.head === a.experimental?.head));
}

/** 提交列表里一行。 */
export function commitLine(commit) {
  return `${commit.short} ${commit.subject}`;
}

function loadStored() {
  try {
    return JSON.parse(localStorage.getItem(STORE_KEY) ?? '[]');
  } catch {
    return [];
  }
}

function storeAdopted() {
  try {
    localStorage.setItem(STORE_KEY, JSON.stringify(state.adoptedKernels ?? []));
  } catch {
    // 存不了不影响本次使用。
  }
}

// ---- 面板 ------------------------------------------------------------------------------------

let registered = [];

async function refresh() {
  const list = $('workspace-list');
  list.replaceChildren(Object.assign(document.createElement('p'), { className: 'muted mini', textContent: t('正在读取…') }));
  try {
    const [workspaces, kernels] = await Promise.all([invoke('workspace_list'), invoke('workspace_kernels')]);
    registered = kernels?.kernels ?? [];
    // 门槛作废了的采纳过的内核不再参与匹配。
    state.adoptedKernels = stillRegistered(state.adoptedKernels, registered);
    storeAdopted();
    render(workspaces?.workspaces ?? []);
  } catch (error) {
    list.replaceChildren(Object.assign(document.createElement('p'), { className: 'assistant-fail', textContent: String(error?.message || error) }));
  }
}

function lightDot(name, light) {
  const dot = Object.assign(document.createElement('span'), {
    className: `ws-light ws-${light ?? 'unknown'}`,
    title: `${t(name)}：${lightText(light)}`,
  });
  dot.append(Object.assign(document.createElement('span'), { className: 'ws-light-dot', textContent: '●' }), document.createTextNode(` ${t(name)}`));
  return dot;
}

function button(text, onclick, disabled = false, title = '') {
  return Object.assign(document.createElement('button'), { className: 'btn-ghost', type: 'button', textContent: t(text), onclick, disabled, title: title ? t(title) : '' });
}

function render(workspaces) {
  const list = $('workspace-list');
  list.replaceChildren();
  if (!workspaces.length) {
    list.append(Object.assign(document.createElement('p'), {
      className: 'muted',
      textContent: t('还没有开发工作区。让助手建一个（例如“新建工作区 emis，来自当前仓库”），它会在里面改代码、编译和对照。'),
    }));
    return;
  }
  for (const ws of workspaces) {
    const card = document.createElement('div');
    card.className = 'ws-card';
    const head = document.createElement('div');
    head.className = 'ws-head';
    head.append(
      Object.assign(document.createElement('b'), { textContent: ws.name }),
      Object.assign(document.createElement('span'), {
        className: 'muted mini',
        textContent: `${ws.commits} ${t('个提交')} · ${baseName(ws.origin)}${ws.dirty ? ` · ${t('有未提交的改动')}` : ''}`,
      }),
    );
    const lights = document.createElement('div');
    lights.className = 'ws-lights';
    for (const [key, name] of Object.entries(LIGHT_NAMES)) lights.append(lightDot(name, ws.lights?.[key]));
    const actions = document.createElement('div');
    actions.className = 'ws-actions';
    const kernels = adoptableKernels(registered, ws.name);
    actions.append(
      button('详情', () => showDetail(ws.name, card)),
      button('采纳为默认内核', () => adopt(ws.name, kernels), !kernels.length,
        kernels.length ? '' : '编译与测试要在当前提交上通过，才能登记实验内核'),
      button('导出补丁…', () => exportPatch(ws.name), ws.commits === 0),
      button('回滚…', () => revert(ws.name), ws.commits === 0),
      button('删除…', () => remove(ws.name)),
    );
    card.append(head, lights, actions);
    list.append(card);
  }
}

async function showDetail(name, card) {
  card.querySelector('.ws-detail')?.remove();
  const box = Object.assign(document.createElement('div'), { className: 'ws-detail mini' });
  try {
    const d = await invoke('workspace_status', { name });
    const lines = [
      `${t('沙箱')}：${d.sandbox?.kind ?? '?'}（${d.sandbox?.note ?? ''}）`,
      `${t('已编好的内核')}：${(d.built_kernels ?? []).join(', ') || t('无')}`,
      lightsSummary(d.lights),
      ...(d.commits ?? []).map(commitLine),
      ...(d.changed_files ?? []).map(f => `${f.path}  +${f.added} −${f.removed}`),
    ];
    box.append(...lines.map(text => Object.assign(document.createElement('div'), { textContent: text })));
  } catch (error) {
    box.textContent = String(error?.message || error);
  }
  card.append(box);
}

async function adopt(name, kernels) {
  const entry = kernels[0];
  if (!entry) return;
  const ok = await appConfirm(
    `${t('把工作区')} ${name} ${t('编出的内核')} ${entry.preset} ${t('设为默认？它是实验内核，没有经过审阅；用它跑出的结果会在阶段指纹里记下内核身份，不会和正式结果混在一起。')}`,
    { okText: t('采纳') },
  );
  if (!ok) return;
  try {
    await invoke('workspace_adopt', { name, preset: entry.preset });
    const adopted = (state.adoptedKernels ?? []).filter(k => !(k.experimental?.workspace === name && k.preset === entry.preset));
    adopted.push(kernelEntry(entry));
    state.adoptedKernels = adopted;
    storeAdopted();
    status(`${t('已采纳')} ${entry.label}`);
  } catch (error) {
    status(error);
  }
}

async function exportPatch(name) {
  const out = await appPrompt(t('导出到哪个文件？（绝对路径，扩展名 .patch）'), `${(state.projectRoot || '').replace(/\/$/, '')}/${name}.patch`);
  if (!out) return;
  try {
    const answer = await invoke('workspace_export', { name, out });
    status(`${t('已导出补丁')}（${answer.bytes} ${t('字节')}）：${out}`);
  } catch (error) {
    status(error);
  }
}

async function revert(name) {
  try {
    const d = await invoke('workspace_status', { name });
    const choices = [...(d.commits ?? []).map(commitLine), `${(d.workspace?.base_commit ?? '').slice(0, 8)} ${t('基线（丢弃全部改动）')}`];
    const picked = await appPrompt(`${t('回滚到哪个提交？写提交号（之后的提交会被丢弃）')}\n${choices.join('\n')}`, (d.workspace?.base_commit ?? '').slice(0, 8));
    if (!picked) return;
    await invoke('workspace_revert', { name, commit: picked.trim().split(/\s+/)[0] });
    status(`${t('已回滚工作区')} ${name}`);
    refresh();
  } catch (error) {
    status(error);
  }
}

async function remove(name) {
  if (!(await appConfirm(`${t('删除工作区')} ${name}？${t('它的源码副本、编译产物和报告都会被删掉，无法恢复。')}`, { okText: t('删除') }))) return;
  try {
    await invoke('workspace_delete', { name });
    state.adoptedKernels = (state.adoptedKernels ?? []).filter(k => k.experimental?.workspace !== name);
    storeAdopted();
    refresh();
  } catch (error) {
    status(error);
  }
}

export function openWorkspaces() {
  $('workspace-dialog').showModal();
  refresh();
}

function wire() {
  state.adoptedKernels = loadStored();
  if (!hasBackend || !$('workspace-dialog')) return;
  $('assistant-workspaces-btn')?.addEventListener('click', openWorkspaces);
  $('workspace-close').onclick = () => $('workspace-dialog').close();
  $('workspace-refresh').onclick = () => refresh();
  // 启动时核一遍：采纳过的内核若门槛作废了，就不再参与匹配。
  invoke('workspace_kernels').then(k => {
    registered = k?.kernels ?? [];
    state.adoptedKernels = stillRegistered(state.adoptedKernels, registered);
    storeAdopted();
  }).catch(() => {});
}

wire();
