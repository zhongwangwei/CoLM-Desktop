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
  stale: '需要重测（之后又改过代码）',
  unknown: '还没测',
};

/** 四项检查，按做的先后排：先能编译，再测试，再看 Fortran 与 Rust 两版一致，最后和原版比。 */
const LIGHT_NAMES = { compile: '编译', tests: '测试', parity: '两版一致', regression: '与原版对比' };

/** 每项检查在查什么（面板开头的说明与灯的悬停提示）。 */
export const LIGHT_HELP = {
  compile: '改过的代码能不能编出 Rust 引擎和 Fortran 内核',
  tests: '自动测试是否通过，包括 Fortran 和 Rust 里的参数表是否同步',
  parity: '同一个算例，改后的 Fortran 和改后的 Rust 算出的结果是否完全一样',
  regression: '和改之前的正式版本比：结果可以变，但不能出现 NaN，水量和能量闭合不能变差',
};

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

/** 改动记录里一行：编号 · 说明（去掉工作区自动加的 `ws: ` 前缀）。 */
export function commitLine(commit) {
  return `${commit.short} · ${String(commit.subject ?? '').replace(/^ws: /, '')}`;
}

/** 这个工作区的内核是否正在使用（启用过、且仍登记着）。 */
export function adoptedFrom(adopted, workspace) {
  return (adopted ?? []).filter(k => k.experimental?.workspace === workspace);
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

function lightDot(key, name, light) {
  const dot = Object.assign(document.createElement('span'), {
    className: `ws-light ws-${light ?? 'unknown'}`,
    title: `${t(name)}：${t(LIGHT_HELP[key])}`,
  });
  dot.append(
    Object.assign(document.createElement('span'), { className: 'ws-light-dot', textContent: '●' }),
    document.createTextNode(` ${t(name)}：`),
    Object.assign(document.createElement('span'), { className: 'ws-light-state', textContent: lightText(light) }),
  );
  return dot;
}

function button(text, onclick, disabled = false, title = '') {
  return Object.assign(document.createElement('button'), { className: 'btn-ghost', type: 'button', textContent: t(text), onclick, disabled, title: title ? t(title) : '' });
}

/** 一个操作：按钮，旁边一行说明它做什么、会不会影响正式版本、能不能撤销。 */
function action(text, help, onclick, disabled = false, why = '') {
  const row = Object.assign(document.createElement('div'), { className: 'ws-action' });
  row.append(button(text, onclick, disabled, why), Object.assign(document.createElement('span'), {
    className: 'muted mini', textContent: disabled && why ? t(why) : t(help),
  }));
  return row;
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
    const inUse = adoptedFrom(state.adoptedKernels, ws.name);
    head.append(
      Object.assign(document.createElement('b'), { textContent: ws.name }),
      Object.assign(document.createElement('span'), {
        className: 'muted mini',
        textContent: `${t('基于')} ${baseName(ws.origin)} · ${t('改了')} ${ws.commits} ${t('次')}${ws.dirty ? ` · ${t('有未保存的改动')}` : ''}`,
      }),
    );
    if (inUse.length) {
      head.append(Object.assign(document.createElement('span'), {
        className: 'ws-in-use', textContent: `${t('正在使用这个内核')}：${inUse.map(k => k.preset).join(', ')}`,
      }));
    }
    const lights = document.createElement('div');
    lights.className = 'ws-lights';
    for (const [key, name] of Object.entries(LIGHT_NAMES)) lights.append(lightDot(key, name, ws.lights?.[key]));
    const actions = document.createElement('div');
    actions.className = 'ws-actions';
    const kernels = adoptableKernels(registered, ws.name);
    const preset = kernels[0]?.preset ?? 'default';
    actions.append(
      action('查看改动', '改动记录、改了哪些文件、在什么环境里编译和测试', () => showDetail(ws.name, card)),
      inUse.length
        ? action('停用这个内核', '改回正式内核；工作区本身不动，之后还能再启用', () => unadopt(ws.name))
        : action('启用这个内核', `以后用 Fortran 内核跑 ${preset} 预设的算例时，改用这个工作区编出的内核。正式内核不会被删除或覆盖，随时可以停用；应用自带的 Rust 引擎不受影响。`,
          () => adopt(ws.name, kernels), !kernels.length, '编译和测试要在最新的改动上通过、与原版对比没有不通过，才能启用'),
      action('导出改动…', '把全部改动存成一个 .patch 文本文件：可以发给别人，或在你自己的仓库里用 git apply 应用后再提交 PR。不会自动上传到 GitHub。',
        () => exportPatch(ws.name), ws.commits === 0, '还没有改动'),
      action('撤回改动…', '退回到之前的某一次改动，或最初的状态；之后的改动被丢弃，需要重新编译和测试。',
        () => revert(ws.name), ws.commits === 0, '还没有改动'),
      action('删除工作区…', '删除整个副本，包括编出的内核和测试报告，不能恢复；正在使用的内核会一起停用。', () => remove(ws.name)),
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
    const section = (title, lines) => {
      if (!lines.length) return;
      box.append(Object.assign(document.createElement('div'), { className: 'ws-detail-title', textContent: t(title) }));
      box.append(...lines.map(text => Object.assign(document.createElement('div'), { textContent: text })));
    };
    const base = (d.workspace?.base_commit ?? '').slice(0, 8);
    section('改动记录（最新的在最上面）', [
      ...(d.commits ?? []).map(commitLine),
      `${base} · ${t('最初的状态（从正式版本复制来时）')}`,
    ]);
    section('和最初的状态相比，改了哪些文件', (d.changed_files ?? []).map(f => `${f.path}  +${f.added} −${f.removed}`));
    section('已经编好的内核', [(d.built_kernels ?? []).join(', ') || t('还没有')]);
    section('编译和测试的运行环境', [d.sandbox?.network_blocked
      ? t('受限环境：不能联网，只能写这个工作区的文件夹')
      : `${t('没有限制环境')}（${d.sandbox?.note ?? ''}）`]);
  } catch (error) {
    box.textContent = String(error?.message || error);
  }
  card.append(box);
}

async function adopt(name, kernels) {
  const entry = kernels[0];
  if (!entry) return;
  const ok = await appConfirm(
    `${t('启用工作区')} ${name} ${t('编出的内核')}（${entry.preset}）？${t('以后用 Fortran 内核跑这个预设的算例时会用它。它是没经过审阅的实验内核；用它跑出的结果会记下内核身份，不会和正式结果混在一起。正式内核不会被删除，随时可以停用。')}`,
    { okText: t('启用') },
  );
  if (!ok) return;
  try {
    await invoke('workspace_adopt', { name, preset: entry.preset });
    const adopted = (state.adoptedKernels ?? []).filter(k => !(k.experimental?.workspace === name && k.preset === entry.preset));
    adopted.push(kernelEntry(entry));
    state.adoptedKernels = adopted;
    storeAdopted();
    status(`${t('已启用')} ${entry.label}`);
    refresh();
  } catch (error) {
    status(error);
  }
}

function unadopt(name) {
  state.adoptedKernels = (state.adoptedKernels ?? []).filter(k => k.experimental?.workspace !== name);
  storeAdopted();
  status(`${t('已停用工作区的内核，改回正式内核')}：${name}`);
  refresh();
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
    const choices = [...(d.commits ?? []).map(commitLine), `${(d.workspace?.base_commit ?? '').slice(0, 8)} · ${t('最初的状态（丢弃全部改动）')}`];
    const picked = await appPrompt(`${t('退回到哪一次改动？填它前面的编号；这之后的改动都会被丢弃：')}\n${choices.join('\n')}`, (d.workspace?.base_commit ?? '').slice(0, 8));
    if (!picked) return;
    await invoke('workspace_revert', { name, commit: picked.trim().split(/\s+/)[0] });
    status(`${t('已撤回工作区的改动')}：${name}`);
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
