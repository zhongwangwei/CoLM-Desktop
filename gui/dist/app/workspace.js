//! 改代码（模型版本）页：每个修改版是一份 CoLM 源码副本（开发工作区，docs/design-ai-assistant.md 第 5、8 节）。
//! 实际工作都由 `colm-cli ws-*` 做；这里提供新建（从官方版或另一个修改版）、版本卡、本机检查、远程检查，
//! 以及导出补丁、撤回、删除。修改版不用登记或“启用”：建好、编好之后在“运行方式”里选它就用它自己的程序跑。
//!
//! 导出、撤回、删除是 **D 级**：只在这里有入口，助手的工具里没有对应的工具，模型调不到。
//!
//! 只依赖 ipc/state/ui/i18n/shell。和 run-mode-view.js 之间用事件联系（`colm:use-version`、`colm:versions`），不互相导入。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, appConfirm, appPrompt, baseName } from './ui.js';
import { language, translateZh } from './i18n.js';
import { go } from './shell.js';
import { configuredServers } from './remote.js';
import { cachedProbe } from './remote-probe-cache.js';
import { touchesText } from './run-mode.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
/** 旧版“启用实验内核”的记录：迁移成默认的模型版本后删掉。 */
const LEGACY_ADOPTED_KEY = 'colm.adoptedKernels';
const VERSION_KEY = 'colm.modelVersion';

// ---- 纯函数（tests/workspace.mjs）--------------------------------------------------------------

const LIGHT_TEXT = {
  pass: '通过',
  fail: '不通过',
  stale: '需要重测（之后又改过代码）',
  unknown: '还没测',
  not_needed: '不需要（没改会影响计算结果的代码）',
};

/** 四项检查，按做的先后排：先能编译，再测试，再看 Fortran 与 Rust 两版一致，最后和原版比。 */
const LIGHT_NAMES = { compile: '编译', tests: '测试', parity: '两版一致', regression: '与原版对比' };

/** 每项检查在查什么（面板开头的说明与灯的悬停提示）。 */
export const LIGHT_HELP = {
  compile: '改过的代码能不能编出 Rust 引擎和 Fortran 内核',
  tests: '自动测试是否通过，包括 Fortran 和 Rust 里的参数表是否同步',
  parity: '同一个算例，改后的 Fortran 和改后的 Rust 算出的结果是否完全一样。只在改了会影响计算结果的代码（Fortran 上游或 Rust 引擎的计算部分）时需要',
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

/** 改动记录里一行：编号 · 说明（去掉工作区自动加的 `ws: ` 前缀）。 */
export function commitLine(commit) {
  return `${commit.short} · ${String(commit.subject ?? '').replace(/^ws: /, '')}`;
}

/** 版本卡第二行：从哪来、改了几次、改了哪边、有没有没提交的改动。 */
export function versionFacts(ws) {
  const touches = ws.touches ?? {};
  const where = touches.engine && touches.kernel ? '改了 Rust 引擎和 Fortran 内核'
    : touches.engine ? '改了 Rust 引擎'
      : touches.kernel ? '改了 Fortran 内核'
        : '没改会影响计算结果的代码';
  return [
    ws.based_on ? `${t('基于修改版')} ${ws.based_on}` : `${t('基于')} ${baseName(ws.origin)}`,
    `${t('改了')} ${ws.commits} ${t('次')}`,
    t(where),
    ws.dirty ? t('有未保存的改动') : null,
  ].filter(Boolean).join(' · ');
}

/** 旧版“启用实验内核”迁移成默认的模型版本：只在还没选过版本时生效，返回要设成的版本名或 null。 */
export function migratedVersion(adopted, chosen) {
  if (chosen) return null;
  const name = (adopted ?? []).find(k => k?.experimental?.workspace)?.experimental.workspace;
  return name ?? null;
}

/** 本机检查按钮：动作、按钮文字、说明。 */
export const LOCAL_CHECKS = [
  ['build-engine', '编译 Rust 引擎', '编出这个版本自己的 colm-cli、colm-rs 与两个前处理；运行这个版本前必须做'],
  ['build-kernel', '编译 Fortran 内核', '改了 Fortran 时要做；用 Fortran 内核跑也要做'],
  ['test-drift', '测试：参数表同步', 'Fortran 与 Rust 里的参数表、字段表有没有漂移'],
  ['test-cargo', '测试：Rust 单元测试', '跑一个包的 cargo 测试（默认 colm-core）'],
  ['parity', '两版一致', '用当前算例分别以改后的 Fortran 与 Rust 跑，逐位对比'],
];

function storageGet(key) {
  try { return localStorage.getItem(key); } catch { return null; }
}

function storageSet(key, value) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // 存不了不影响本次使用。
  }
}

// ---- 页面 ------------------------------------------------------------------------------------

let creating = false;
let createStep = 0;
let createMethod = 'ai';
let versions = [];
let detailName = null;
let detailTab = 'modify';
let detailRequest = 0;

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function versionsChanged() {
  globalThis.dispatchEvent?.(new Event('colm:versions'));
}

function renderBaseChoices() {
  const select = $('workspace-base');
  if (!select?.replaceChildren) return;
  const keep = select.value;
  select.replaceChildren(new Option(t('官方版（应用自带的 CoLM）'), ''));
  for (const ws of versions) select.appendChild(new Option(`${t('修改版')}：${ws.name}`, ws.name));
  if (keep && !versions.some(ws => ws.name === keep)) select.appendChild(new Option(keep, keep));
  select.value = keep;
  renderCreateWizard();
}

function renderCreateWizard() {
  if (!$('workspace-base-cards')) return;
  for (let index = 0; index < 3; index++) {
    $(`workspace-create-step-${index}`).hidden = index !== createStep;
    $('workspace-create-progress').children[index].setAttribute('aria-current', index === createStep ? 'step' : 'false');
  }
  $('workspace-create-prev').disabled = creating || createStep === 0;
  $('workspace-create-next').hidden = createStep === 2;
  $('workspace-create').hidden = createStep !== 2;
  $('workspace-other-source').hidden = !!$('workspace-base').value;
  const choices = (container, items, selected, choose) => {
    const focused = document.activeElement?.id;
    container.replaceChildren(...items.map(([value, title, help], index) => {
      const control = button(title, () => { if (!creating) choose(value); }, creating);
      control.id = `${container.id}-${index}`;
      control.className = 'run-mode-card';
      control.setAttribute('aria-pressed', String(value === selected));
      control.append(el('span', { className: 'run-mode-card-detail', textContent: t(help) }));
      return control;
    }));
    if (focused) [...container.children].find(control => control.id === focused)?.focus();
  };
  choices($('workspace-base-cards'), [
    ['', '官方版', '从发布的 CoLM 2024 开始'],
    ...versions.map(ws => [ws.name, ws.name,
      ws.lights?.compile === 'pass' && ws.lights?.tests === 'pass' && !Object.values(ws.lights ?? {}).includes('fail')
        ? '在已有修改上继续改 · 测试已通过' : '复制一份，分出另一个方向']),
  ], $('workspace-base').value, value => { $('workspace-base').value = value; renderCreateWizard(); });
  choices($('workspace-method-cards'), [
    ['ai', '让 AI 助手改', '助手会知道当前版本；每次改代码前都会请你确认'],
    ['editor', '用我的编辑器打开', '创建后显示源码路径，用自己的编辑器修改'],
    ['patch', '导入补丁文件', '创建后选择 .diff 或 .patch 文件'],
  ], createMethod, value => { createMethod = value; renderCreateWizard(); });
}

function validateCreateStep() {
  const name = $('workspace-name').value.trim();
  const note = $('workspace-note')?.value.trim() || '';
  let error = '';
  if ($('workspace-base').value && !versions.some(ws => ws.name === $('workspace-base').value)) error = '起点版本已不存在，请重新选择';
  if (createStep >= 1 && !/^[A-Za-z0-9][A-Za-z0-9_-]{0,39}$/.test(name)) error = '名称须为 1–40 个字母、数字、下划线或短横线，并以字母或数字开头';
  if (createStep >= 1 && note.length > 500) error = '目的不能超过 500 字';
  $('workspace-create-status').textContent = t(error);
  $('workspace-create-status').className = error ? 'mini assistant-fail' : 'mini';
  return !error;
}

function moveCreateStep(delta) {
  if (creating || (delta > 0 && !validateCreateStep())) return;
  createStep = Math.max(0, Math.min(2, createStep + delta));
  renderCreateWizard();
}

function openCreateWizard() {
  if (creating) return;
  closeDetail();
  detailVisibility(true);
  $('workspace-create-form').hidden = false;
  createStep = 0;
  $('workspace-create-status').textContent = '';
  renderCreateWizard();
}

async function createWorkspace(event) {
  event.preventDefault();
  if (creating) return;
  if (createStep < 2) { moveCreateStep(1); return; }
  if (!validateCreateStep()) return;
  const name = $('workspace-name').value.trim();
  const base = $('workspace-base')?.value || null;
  const note = $('workspace-note')?.value.trim() || null;
  const source = base ? null : ($('workspace-source').value.trim() || null);
  const rev = base ? null : ($('workspace-rev').value.trim() || null);
  const message = $('workspace-create-status');
  const submit = $('workspace-create');
  const fields = ['workspace-name', 'workspace-note', 'workspace-base', 'workspace-source', 'workspace-rev', 'workspace-source-browse', 'workspace-create', 'workspace-create-next', 'workspace-create-prev', 'workspace-create-back'];
  creating = true;
  for (const id of fields) if ($(id)) $(id).disabled = true;
  renderCreateWizard();
  submit.textContent = t('正在创建…');
  message.className = 'mini';
  message.textContent = t('正在复制源码，创建完成后进入版本详情。');
  try {
    const result = await invoke('workspace_create', { name, source, rev, base, note });
    message.textContent = `${t('已创建修改版')} ${name}：${result?.dir ?? ''}。${t('下一步：改代码，然后编译 Rust 引擎。')}`;
    $('workspace-name').value = '';
    if ($('workspace-note')) $('workspace-note').value = '';
    await refresh();
    versionsChanged();
    const opened = await showDetail(name);
    if (opened) {
      if (createMethod === 'ai') askAssistant(name);
      else if (createMethod === 'editor') $('workspace-source-path').hidden = false;
      else await importPatch(name);
    }
    createStep = 0;
  } catch (error) {
    message.className = 'mini assistant-fail';
    message.textContent = String(error?.message || error);
  } finally {
    creating = false;
    for (const id of fields) if ($(id)) $(id).disabled = false;
    submit.textContent = t('建好并开始修改 →');
    renderCreateWizard();
  }
}

async function refresh() {
  const list = $('workspace-list');
  list.replaceChildren(Object.assign(document.createElement('p'), { className: 'muted mini', textContent: t('正在读取…') }));
  try {
    const workspaces = await invoke('workspace_list');
    versions = workspaces?.workspaces ?? [];
    renderBaseChoices();
    render(versions);
    if (detailName) await showDetail(detailName, detailTab);
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

/** 在“运行方式”里选这个版本并过去。 */
function useVersion(name) {
  globalThis.dispatchEvent?.(new CustomEvent('colm:use-version', { detail: name }));
}

/** 打开助手，把一句关于这个版本的请求填进输入框（不自动发送）。 */
function askAssistant(name) {
  globalThis.dispatchEvent?.(new CustomEvent('colm:assistant-prompt', {
    detail: `请在修改版 ${name} 里改代码：先读相关源码，说明打算怎么改、影响 Rust 引擎还是 Fortran 内核，等我确认后再打补丁。我想改的是：`,
  }));
}

async function editNote(ws) {
  const note = await appPrompt(t('这个版本想改什么、为什么？（一句话，会写进它跑出的每个结果）'), ws.note ?? '');
  if (note === null || note === undefined) return;
  try {
    await invoke('workspace_note', { name: ws.name, note });
    await refresh();
  } catch (error) {
    status(error);
  }
}

async function importPatch(name) {
  try {
    const file = await invoke('pick_file', { key: 'workspace-patch' });
    if (!file) return;
    const message = await appPrompt(t('这次改动的说明（会记进改动记录）'), baseName(file));
    if (!message) return;
    await invoke('workspace_import_patch', { name, file, message });
    status(`${t('已导入补丁')}：${baseName(file)}`);
    await refresh();
    versionsChanged();
  } catch (error) {
    status(error);
  }
}

/** 本机检查（编译、测试、两版一致）：一个版本同时只跑一项，结果写在下面。 */
function checksPanel(ws) {
  const panel = el('details', { className: 'ws-detail ws-checks' }, [el('summary', { textContent: t('在本机编译和检查') })]);
  const preset = el('input', { className: 'input', value: 'default', required: true, size: 12 });
  const pkg = el('input', { className: 'input', value: 'colm-core', size: 14 });
  const out = el('p', { className: 'mini', role: 'status' });
  const options = el('div', { className: 'ws-check-options mini' }, [
    el('label', { className: 'muted', textContent: t('内核预设') }), preset,
    el('label', { className: 'muted', textContent: t('测试包') }), pkg,
  ]);
  const rows = el('div', { className: 'ws-check-list' });
  let busy = false;
  for (const [id, label, help] of LOCAL_CHECKS) {
    rows.append(action(label, help, async () => {
      if (busy) return;
      const caseDir = state.selected?.dir;
      if (id === 'parity' && !caseDir) { out.className = 'mini assistant-fail'; out.textContent = t('两版一致要先选一个算例'); return; }
      busy = true;
      for (const b of rows.querySelectorAll?.('button') ?? []) b.disabled = true;
      out.className = 'mini';
      out.textContent = `${t(label)}：${t('进行中…编译可能要几分钟')}`;
      try {
        const result = await invoke('workspace_check', {
          name: ws.name, action: id, preset: preset.value.trim() || 'default', package: pkg.value.trim() || null, case: id === 'parity' ? caseDir : null,
        });
        const ok = result?.ok ?? result?.outcome?.ok ?? true;
        out.className = ok ? 'mini' : 'mini assistant-fail';
        out.textContent = `${t(label)}：${t(ok ? '通过' : '不通过')}${result?.first_difference ? ` · ${result.first_difference}` : ''}${result?.tail && !ok ? `\n${String(result.tail).slice(-1200)}` : ''}`;
      } catch (error) {
        out.className = 'mini assistant-fail';
        out.textContent = `${t(label)}：${String(error?.message || error)}`;
      } finally {
        busy = false;
        for (const b of rows.querySelectorAll?.('button') ?? []) b.disabled = false;
        await refresh();
        versionsChanged();
      }
    }));
  }
  panel.append(
    el('p', { className: 'mini muted', textContent: t('在受限环境里运行（不能联网、只能写这个版本的文件夹）。编译与测试都记在当前提交名下；之后再改代码，灯会变成“需要重测”。') }),
    options, rows, out,
  );
  return panel;
}

function render(workspaces) {
  const list = $('workspace-list');
  list.replaceChildren();
  if (!workspaces.length) {
    list.append(Object.assign(document.createElement('p'), {
      className: 'muted',
      textContent: t('还没有修改版。点击“新建修改版”，选择起点并填写名称和目的。'),
    }));
    return;
  }
  for (const ws of workspaces) {
    const card = document.createElement('div');
    card.className = 'ws-card';
    const head = document.createElement('div');
    head.className = 'ws-head';
    const inUse = (state.modelVersion ?? storageGet(VERSION_KEY)) === ws.name;
    head.append(
      button(ws.name, () => showDetail(ws.name)),
      Object.assign(document.createElement('span'), { className: 'muted mini', textContent: versionFacts(ws) }),
    );
    if (inUse) head.append(Object.assign(document.createElement('span'), { className: 'ws-in-use', textContent: t('运行方式里选的就是它') }));
    const note = el('p', { className: 'ws-note mini', textContent: ws.note ? `“${ws.note}”` : t('还没写目的') });
    const lights = document.createElement('div');
    lights.className = 'ws-lights';
    for (const [key, name] of Object.entries(LIGHT_NAMES)) lights.append(lightDot(key, name, ws.lights?.[key]));
    const primary = el('div', { className: 'pill-row ws-primary' }, [
      Object.assign(button('用它运行 →', () => useVersion(ws.name)), { className: 'run-btn' }),
      button('查看详情', () => showDetail(ws.name)),
    ]);
    card.append(head, note, lights, primary);
    list.append(card);
  }
}

export function remoteRequest(action, preset, casePath, packageName, changeKind = 'refactor') {
  const request = { action, preset: preset.trim() || 'default' };
  if (action === 'verify' || action === 'test') {
    if (!/^[A-Za-z0-9_-]+$/.test(packageName.trim())) throw new Error(t('请输入测试包名称'));
    Object.assign(request, { kind: action === 'verify' ? changeKind : 'cargo', package: packageName.trim() });
  }
  if (action === 'verify') {
    if (!casePath.trim().startsWith('/') || /[\0\r\n]/.test(casePath)) throw new Error(t('请输入服务器上算例的绝对路径'));
    request.case = casePath.trim();
  }
  return request;
}

export function remoteJobText(job, live) {
  const stateText = { preparing: '准备中', submitting: '正在提交', queued: '等待运行', running: '正在运行', finished: '已完成', failed: '失败', lost: '作业已丢失', unknown: '状态未知' };
  const value = live?.state === 'finished' && live.exit_code !== 0 ? 'failed' : live?.state ?? job.state;
  return `${job.id} · ${job.host} · ${t(stateText[value] ?? '状态未知')} · ${String(job.commit ?? '').slice(0, 8)}${job.stale ? ` · ${t('需要重测（之后又改过代码）')}` : ''}`;
}

export function remoteEvidenceText(evidence) {
  const gateLight = (runs, commit) => {
    if (!runs.length || runs.some(run => !run)) return 'unknown';
    if (evidence.stale || runs.some(run => run.commit !== commit)) return 'stale';
    return runs.some(run => run.ok === false) ? 'fail' : runs.every(run => run.ok === true) ? 'pass' : 'unknown';
  };
  const lines = [];
  for (const [key, label, commit] of [['candidate', '修改后的源码', evidence.commit], ['baseline', '原版源码', evidence.base_commit]]) {
    const gates = evidence.gates?.[key] ?? {};
    const kernels = Object.values(gates.kernels ?? {});
    lines.push(`${t(label)} · ${String(commit ?? '').slice(0, 8)}`);
    lines.push(lightsSummary({
      compile: gateLight([gates.engine, ...(kernels.length ? kernels : [null])], commit),
      tests: gateLight(Object.values(gates.tests ?? {}), commit),
      parity: gateLight([gates.parity], commit),
      regression: gateLight([gates.regression], commit),
    }));
  }
  for (const outcome of (evidence.outcomes ?? []).slice(0, 20)) {
    const detail = [outcome.command, outcome.verdict].filter(Boolean).join(' · ');
    if (detail) lines.push(detail.slice(0, 1200));
    if (outcome.first_difference != null) lines.push(`${t('首个差异')}：${JSON.stringify(outcome.first_difference).slice(0, 1200)}`);
    const compare = Object.entries(outcome.compare ?? {}).filter(([, value]) => value != null);
    if (compare.length) lines.push(`${t('对比统计')}：${compare.map(([key, value]) => `${key}=${value}`).join(', ').slice(0, 1200)}`);
  }
  return lines.join('\n');
}

function remotePanel(name, host = null) {
  const panel = document.createElement('details');
  panel.className = 'ws-detail';
  panel.append(Object.assign(document.createElement('summary'), { textContent: t('在服务器上编译和验证') }));
  const form = document.createElement('form');
  const field = (text, input) => {
    const label = Object.assign(document.createElement('label'), { className: 'field', textContent: t(text) });
    label.append(input);
    form.append(label);
    return input;
  };
  const server = field('服务器', document.createElement('select'));
  server.required = true;
  const preset = field('预设', Object.assign(document.createElement('input'), { value: 'default', required: true }));
  const casePath = field('服务器算例绝对路径', Object.assign(document.createElement('input'), { placeholder: '/media/data/cases/site' }));
  const packageName = field('测试包', Object.assign(document.createElement('input'), { value: 'colm-core', required: true }));
  const task = field('任务', document.createElement('select'));
  for (const [value, label] of [['verify', '完整验证'], ['build-engine', '编译 Rust 引擎'], ['build-kernel', '编译 Fortran 内核'], ['test', '运行测试']]) {
    task.append(Object.assign(document.createElement('option'), { value, textContent: t(label) }));
  }
  const changeKind = field('改动类型', document.createElement('select'));
  for (const [value, label] of [['refactor', '重构（结果应保持一致）'], ['physics', '物理过程修改（检查闭合）']]) changeKind.append(Object.assign(document.createElement('option'), { value, textContent: t(label) }));
  const help = Object.assign(document.createElement('p'), { className: 'mini muted', textContent: t('上传当前工作区源码快照，在服务器独立目录运行。完整验证包含编译、测试、与原版对比及两版一致检查；算例须已在服务器上。远程结果不改变本机内核。') });
  const submit = Object.assign(document.createElement('button'), { type: 'submit', className: 'btn-ghost', textContent: t('提交远程任务'), disabled: true });
  const message = Object.assign(document.createElement('p'), { className: 'mini' });
  message.setAttribute('role', 'status');
  message.setAttribute('aria-live', 'polite');
  const jobs = document.createElement('div');
  const refreshButton = button('刷新远程任务', () => loadJobs(true));
  form.append(help, submit);
  const evidenceBox = Object.assign(document.createElement('pre'), { className: 'mini', hidden: true });
  const submissionWarnings = Object.assign(document.createElement('p'), { className: 'mini warn', hidden: true });
  panel.append(form, message, refreshButton, jobs, evidenceBox, submissionWarnings);
  let loaded = false;
  let busy = false;
  let timer;
  const errorText = error => { message.className = 'mini assistant-fail'; message.textContent = String(error?.message || error); };
  const call = (operation, extra = {}) => invoke('workspace_remote', { name, operation, host: null, job: null, request: null, ...extra });
  async function jobAction(operation, job) {
    if (busy) return;
    busy = true;
    refreshButton.disabled = true;
    message.className = 'mini';
    message.textContent = t('正在处理远程任务…');
    try {
      const result = await call(operation, { job: job.id });
      if (operation === 'fetch') {
        message.textContent = `${t('已取回远程报告')}：${result.job?.report_dir ?? ''} · ${remoteJobText(result.job ?? job, result.status)}`;
        evidenceBox.hidden = !result.evidence;
        evidenceBox.textContent = result.evidence ? `${t('远程验证证据')} · ${job.id}\n${remoteEvidenceText(result.evidence)}` : '';
      }
      else message.textContent = '';
      await loadJobs(false);
    } catch (error) { errorText(error); }
    finally { busy = false; refreshButton.disabled = false; }
  }
  async function loadJobs(updateStatus) {
    clearTimeout(timer);
    try {
      const result = await call('list');
      const entries = [];
      let pending = false;
      for (const record of result.jobs ?? []) {
        if (host && record.host !== host) continue;
        let job = record;
        let live;
        let failure;
        if (updateStatus) {
          try { const result = await call('status', { job: job.id }); job = result.job; live = result.status; }
          catch (error) { failure = String(error?.message || error); }
        }
        const active = ['preparing', 'submitting', 'queued', 'running', 'unknown', 'lost'].includes(live?.state ?? job.state);
        pending ||= active && !failure;
        const row = Object.assign(document.createElement('div'), { className: 'ws-action' });
        row.append(Object.assign(document.createElement('p'), { className: 'mini', textContent: remoteJobText(job, live) }));
        row.append(Object.assign(document.createElement('p'), { className: 'mini muted', textContent: `${t('源码哈希')}：${job.source_sha256 ?? ''}` }));
        if (live?.phase) row.append(Object.assign(document.createElement('p'), { textContent: live.phase }));
        if (failure || live?.log_tail) row.append(Object.assign(document.createElement('pre'), { textContent: failure || live.log_tail }));
        if (active) row.append(button('取消远程任务', () => jobAction('cancel', job)));
        else row.append(button('取回报告和日志', () => jobAction('fetch', job)));
        if (job.report_dir) row.append(Object.assign(document.createElement('p'), { className: 'mini', textContent: `${t('报告位置')}：${job.report_dir}` }));
        entries.push(row);
      }
      jobs.replaceChildren(...entries);
      if (!entries.length) jobs.append(Object.assign(document.createElement('p'), { className: 'mini muted', textContent: t('还没有远程任务') }));
      if (pending && panel.open && panel.isConnected) timer = setTimeout(() => { if (panel.open && panel.isConnected && state.step === 'code' && !busy) loadJobs(true); }, 5000);
    } catch (error) { errorText(error); }
  }
  panel.addEventListener('toggle', async () => {
    if (!panel.open) { clearTimeout(timer); return; }
    if (!loaded) {
      try {
        const config = host ? { servers: configuredServers() } : await invoke('remote_config');
        for (const entry of config.servers ?? []) server.append(Object.assign(document.createElement('option'), { value: entry.host, textContent: entry.host }));
        if (host) { server.value = host; server.disabled = true; }
        submit.disabled = !server.value;
        if (submit.disabled) message.textContent = t('请先在服务器设置中添加服务器');
        loaded = true;
      } catch (error) { errorText(error); }
    }
    await loadJobs(true);
  });
  form.onsubmit = async event => {
    event.preventDefault();
    if (busy || !form.reportValidity()) return;
    busy = true;
    submit.disabled = true;
    try {
      const request = remoteRequest(task.value, preset.value, casePath.value, packageName.value, changeKind.value);
      message.className = 'mini';
      message.textContent = t('正在上传源码并提交任务…');
      const result = await call('submit', { host: server.value, request });
      submissionWarnings.hidden = !result.warnings?.length;
      submissionWarnings.textContent = (result.warnings ?? []).map(warning => `${t('任务已提交，请注意')}：${warning}`).join('\n');
      message.textContent = remoteJobText(result.job, result.status);
      if (result.error) errorText(result.error);
      await loadJobs(true);
    } catch (error) { errorText(error); }
    finally { busy = false; submit.disabled = !server.value; }
  };
  return panel;
}

function detailVisibility(open) {
  $('workspace-list').hidden = open;
  if ($('workspace-create-form')) $('workspace-create-form').hidden = true;
  if ($('workspace-new')) $('workspace-new').hidden = open;
  const page = $('workspace-list').parentElement;
  const title = page?.querySelector('h1');
  const intro = page?.querySelector('.sub');
  if (title) title.hidden = open;
  if (intro) intro.hidden = open;
  const official = page?.querySelector('.ws-official');
  if (official) official.hidden = open;
  const legend = page?.querySelector('.ws-legend');
  if (legend) legend.hidden = open;
  if ($('workspace-refresh')?.parentElement) $('workspace-refresh').parentElement.hidden = open;
}

function closeDetail() {
  detailName = null;
  detailRequest++;
  $('workspace-detail')?.remove();
  detailVisibility(false);
}

function studyRows(studies) {
  if (!studies?.length) return [el('p', { className: 'muted mini', textContent: t('还没有研究用过这个版本') })];
  return studies.map(study => {
    const time = Number(study.pinned_unix);
    const when = study.pinned_unix != null && Number.isFinite(time) ? new Date(time * 1000).toLocaleString(language() === 'en' ? 'en-US' : 'zh-CN') : '';
    return el('div', { className: 'ws-study-row mini' }, [
      el('b', { textContent: study.id || baseName(study.dir) }),
      el('p', { textContent: [study.location === 'local' ? t('本机') : study.location, when, String(study.head ?? '').slice(0, 8)].filter(Boolean).join(' · ') }),
      el('p', { textContent: t(study.current ? '✓ 快照仍是当前代码' : '! 之后又改过') }),
      el('p', { className: 'muted', textContent: study.dir }),
    ]);
  });
}

async function showDetail(name, tab = 'modify') {
  const request = ++detailRequest;
  detailName = name;
  detailTab = typeof tab === 'string' ? tab : 'modify';
  let box = $('workspace-detail');
  if (!box) {
    box = el('div', { id: 'workspace-detail', className: 'ws-detail' });
    $('workspace-list').after(box);
  }
  detailVisibility(true);
  box.replaceChildren(el('p', { role: 'status', textContent: t('正在读取…') }));
  try {
    const d = await invoke('workspace_status', { name });
    if (request !== detailRequest) return;
    const listed = versions.find(ws => ws.name === name) ?? {};
    const ws = { ...listed, ...d.workspace, name, commits: d.commits?.length ?? listed.commits,
      lights: d.lights ?? listed.lights, touches: d.touches ?? { engine: (d.changed_files ?? []).some(file => file.touches?.engine), kernel: (d.changed_files ?? []).some(file => file.touches?.kernel) } };
    box.replaceChildren(el('div', { className: 'ws-detail-header' }, [
      button('← 改代码', closeDetail), el('h1', { textContent: name }),
      el('p', { className: 'mini', textContent: `${ws.note || t('还没写目的')} · ${ws.based_on ? `${t('基于修改版')} ${ws.based_on}` : t('基于官方版')}` }),
      button('用它运行 →', () => useVersion(name)),
    ]));
    const tabs = el('div', { className: 'ws-detail-tabs', role: 'tablist' });
    const panels = [];
    const choose = key => {
      detailTab = key;
      for (const [id, control, panel] of panels) {
        control.setAttribute('aria-selected', String(id === key));
        control.tabIndex = id === key ? 0 : -1;
        panel.hidden = id !== key;
      }
    };
    for (const [key, label] of [['modify','修改'], ['checks','检查'], ['compare','对比'], ['studies','用到它的研究']]) {
      const control = button(label, () => choose(key));
      control.id = `workspace-tab-${key}`;
      control.setAttribute('role', 'tab');
      control.setAttribute('aria-controls', `workspace-panel-${key}`);
      const panel = el('section', { id: `workspace-panel-${key}`, className: 'ws-detail-panel', role: 'tabpanel' });
      panel.setAttribute('aria-labelledby', control.id);
      control.onkeydown = event => {
        const index = panels.findIndex(([id]) => id === key);
        const next = event.key === 'Home' ? 0 : event.key === 'End' ? panels.length - 1
          : event.key === 'ArrowRight' ? (index + 1) % panels.length : event.key === 'ArrowLeft' ? (index + panels.length - 1) % panels.length : -1;
        if (next < 0) return;
        event.preventDefault(); choose(panels[next][0]); panels[next][1].focus();
      };
      tabs.append(control); panels.push([key, control, panel]);
    }
    box.append(tabs, ...panels.map(([, , panel]) => panel));
    const [modify, checks, compare, studies] = panels.map(([, , panel]) => panel);
    const source = el('p', { id: 'workspace-source-path', className: 'mini', hidden: true, textContent: d.dir ? `${d.dir}/src` : t('源码路径不可用') });
    modify.append(el('div', { className: 'pill-row' }, [
      button('在右侧让助手改', () => askAssistant(name)),
      button('用我的编辑器打开', () => { source.hidden = false; }),
      button('导入补丁文件…', () => importPatch(name)), button('写目的…', () => editNote(ws)),
    ]), source, el('h3', { textContent: `${t('相对官方版的改动')} · ${(d.changed_files ?? []).length} ${t('个文件')}` }));
    for (const file of d.changed_files ?? []) {
      const output = el('pre', { className: 'mini ws-file-diff', hidden: true, role: 'status' });
      const row = el('div', { className: 'ws-file-row' });
      const kind = file.touches?.kernel ? 'Fortran 内核' : file.touches?.engine ? 'Rust 引擎' : '其他改动';
      const diff = button('看差异', async () => {
        diff.disabled = true; output.hidden = false; output.textContent = t('正在读取…');
        try {
          const result = await invoke('workspace_diff', { name, path: file.path });
          output.textContent = `${result.diff}${result.truncated ? `\n${t('差异过长，已截断')}` : ''}`;
        } catch (error) { output.textContent = String(error?.message || error); }
        finally { diff.disabled = false; }
      });
      const undo = button('撤销这处', async () => {
        if (undo.disabled) return;
        undo.disabled = true;
        try {
          if (!(await appConfirm(`${t('恢复这个文件到官方基线并记录一次提交？')}\n${file.path}`, { okText: t('撤销这处') }))) return;
          await invoke('workspace_revert_file', { name, path: file.path, message: 'Restore file to official baseline' });
          await refresh(); versionsChanged();
        } catch (error) { output.hidden = false; output.textContent = String(error?.message || error); }
        finally { undo.disabled = false; }
      });
      row.append(el('code', { textContent: file.path }), el('span', { className: 'mini muted', textContent: `${t(kind)} · +${file.added ?? '—'} −${file.removed ?? '—'}` }), diff, undo, output);
      modify.append(row);
    }
    modify.append(el('p', { className: 'mini muted', textContent: t('用编辑器修改后，请先提交或撤销改动，再检查和运行。研究首次运行时会固定所用版本。') }));
    const more = el('details', { className: 'ws-detail' }, [el('summary', { textContent: t('更多') })]);
    more.append(
      action('导出改动…', '把全部改动存成一个 .patch 文本文件：可以发给别人，或在你自己的仓库里用 git apply 应用后再提交 PR。不会自动上传到 GitHub。', () => exportPatch(name), ws.commits === 0, '还没有改动'),
      action('撤回改动…', '退回到之前的某一次改动，或最初的状态；之后的改动被丢弃，需要重新编译和测试。', () => revert(name), ws.commits === 0, '还没有改动'),
      action('删除这个修改版…', '删除整个副本，包括编出的程序和测试报告，不能恢复；运行方式里选着它的会改回官方版。', () => remove(name)),
    );
    modify.append(more);
    checks.append(el('h3', { textContent: t('在哪里检查') }));
    const locations = el('div', { className: 'run-choice-grid ws-check-targets' });
    const checkBody = el('div');
    const servers = configuredServers();
    const targetButtons = [];
    const checkPanels = new Map();
    const localEvidence = el('div');
    const selectTarget = host => {
      for (const [target, control] of targetButtons) control.setAttribute('aria-pressed', String(target === host));
      if (!checkPanels.has(host)) {
        const panel = host === 'local' ? checksPanel(ws) : remotePanel(name, host);
        if (host === 'local') panel.open = true;
        checkPanels.set(host, panel);
      }
      localEvidence.hidden = host !== 'local';
      checkBody.replaceChildren(checkPanels.get(host));
    };
    for (const [host, label] of [['local',t('本机')], ...servers.map(server => {
      const cached = cachedProbe(server);
      return [server.host, `${server.host} · ${t(!cached ? '没测过' : cached.error ? '连接失败（上次测试）' : '在线（上次测试）')}`];
    })]) {
      const control = button(label, () => selectTarget(host)); control.className = 'run-mode-card';
      targetButtons.push([host, control]); locations.append(control);
    }
    checks.append(locations, el('p', { className: 'mini', textContent: t(ws.touches?.engine && ws.touches?.kernel
      ? 'Rust 引擎与 Fortran 内核都改了，两边都要编译' : touchesText(ws.touches)) }));
    for (const [key, label] of Object.entries(LIGHT_NAMES)) localEvidence.append(el('div', { className: 'ws-action' }, [
      lightDot(key, label, ws.lights?.[key]), el('p', { className: 'mini muted', textContent: t(LIGHT_HELP[key]) }),
    ]));
    checks.append(localEvidence, checkBody); selectTarget('local');
    compare.append(el('h3', { textContent: t('快速对比') }), el('p', { textContent: t('同一个算例、同一组参数，只换模型版本各跑一次。先保留已有结果，再到运行方式选择父版本或官方版。') }),
      button('去运行方式', () => go('run-mode')),
      el('h3', { textContent: t('公平比较（研究）') }), el('p', { textContent: t('按相同参数范围、预算和站点分别率定两个版本，再比较验证期表现。') }),
      button('比较两个版本', () => { go('result-tuning'); $('version-compare-card')?.scrollIntoView({ block: 'center' }); }));
    studies.append(...studyRows(d.studies));
    if (ws.based_on) {
      const parent = el('div', {}, [el('h3', { textContent: `${t('父版本的研究')} · ${ws.based_on}` })]);
      studies.append(parent);
      invoke('workspace_status', { name: ws.based_on }).then(result => parent.append(...studyRows(result.studies)))
        .catch(error => parent.append(el('p', { className: 'assistant-fail', textContent: String(error?.message || error) })));
    }
    choose(detailTab);
    return true;
  } catch (error) {
    if (request !== detailRequest) return;
    box.replaceChildren(button('← 改代码', closeDetail), el('p', { role: 'status', className: 'assistant-fail', textContent: String(error?.message || error) }));
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
    const choices = [...(d.commits ?? []).map(commitLine), `${(d.workspace?.base_commit ?? '').slice(0, 8)} · ${t('最初的状态（丢弃全部改动）')}`];
    const picked = await appPrompt(`${t('退回到哪一次改动？填它前面的编号；这之后的改动都会被丢弃：')}\n${choices.join('\n')}`, (d.workspace?.base_commit ?? '').slice(0, 8));
    if (!picked) return;
    await invoke('workspace_revert', { name, commit: picked.trim().split(/\s+/)[0] });
    status(`${t('已撤回工作区的改动')}：${name}`);
    await refresh();
    versionsChanged();
  } catch (error) {
    status(error);
  }
}

async function remove(name) {
  if (!(await appConfirm(`${t('删除修改版')} ${name}？${t('它的源码副本、编译产物和报告都会被删掉，无法恢复。')}`, { okText: t('删除') }))) return;
  try {
    await invoke('workspace_delete', { name });
    if (detailName === name) closeDetail();
    await refresh();
    versionsChanged();
  } catch (error) {
    status(error);
  }
}


/** 去“改代码”页（左栏“改进模型”下）。 */
export function openWorkspaces() {
  go('code');
}

function wire() {
  // 旧版“启用实验内核”迁移成默认的模型版本（只在还没选过版本时），之后不再参与内核匹配。
  let legacy = [];
  try { legacy = JSON.parse(storageGet(LEGACY_ADOPTED_KEY) ?? '[]'); } catch { legacy = []; }
  const migrated = migratedVersion(legacy, storageGet(VERSION_KEY));
  if (migrated) storageSet(VERSION_KEY, migrated);
  storageSet(LEGACY_ADOPTED_KEY, null);
  if (!hasBackend || !$('workspace-list')) return;
  $('research-workspaces-btn')?.addEventListener('click', openWorkspaces);
  $('workspace-create-form').onsubmit = createWorkspace;
  $('workspace-new').onclick = openCreateWizard;
  $('workspace-create-back').onclick = () => { if (!creating) closeDetail(); };
  $('workspace-create-prev').onclick = () => moveCreateStep(-1);
  $('workspace-create-next').onclick = () => moveCreateStep(1);
  $('workspace-base')?.addEventListener('change', renderCreateWizard);
  $('workspace-source-browse').onclick = async () => {
    try {
      const path = await invoke('pick_folder', { key: 'workspace-source' });
      if (path && !creating) $('workspace-source').value = path;
    } catch (error) {
      $('workspace-create-status').textContent = String(error?.message || error);
    }
  };
  $('workspace-refresh').onclick = () => refresh();
  addEventListener('colm:step', () => { if (state.step === 'code') refresh(); });
}

wire();
