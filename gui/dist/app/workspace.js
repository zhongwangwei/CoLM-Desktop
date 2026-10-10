//! 开发工作区面板（docs/design-ai-assistant.md 第 5、8 节）：助手改 CoLM 源码、编译、测试、对照的地方。
//! 实际工作都由 `colm-cli ws-*` 做；这里提供新建、状态和采纳、导出补丁、回滚、删除。
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
let creating = false;

async function createWorkspace(event) {
  event.preventDefault();
  if (creating || !$('workspace-create-form').reportValidity()) return;
  const name = $('workspace-name').value.trim();
  const source = $('workspace-source').value.trim() || null;
  const rev = $('workspace-rev').value.trim() || null;
  const message = $('workspace-create-status');
  const submit = $('workspace-create');
  creating = true;
  for (const id of ['workspace-name', 'workspace-source', 'workspace-rev', 'workspace-source-browse', 'workspace-create']) $(id).disabled = true;
  submit.textContent = t('正在创建…');
  message.className = 'mini';
  message.textContent = t('正在复制源码，创建完成后会显示在下方。');
  try {
    const result = await invoke('workspace_create', { name, source, rev });
    message.textContent = `${t('已创建工作区')} ${name} · ${t('位置')}：${result.dir}。${t('下一步：让助手在这个工作区里修改代码、编译和验证。')}`;
    await refresh();
  } catch (error) {
    message.className = 'mini assistant-fail';
    message.textContent = String(error?.message || error);
  } finally {
    creating = false;
    for (const id of ['workspace-name', 'workspace-source', 'workspace-rev', 'workspace-source-browse', 'workspace-create']) $(id).disabled = false;
    submit.textContent = t('新建工作区');
  }
}

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
      textContent: t('还没有开发工作区。在上方填写名称，点击“新建工作区”。'),
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
    card.append(head, lights, actions, remotePanel(ws.name));
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

function remotePanel(name) {
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
  panel.append(form, message, refreshButton, jobs, evidenceBox);
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
      if (pending && panel.open && panel.isConnected) timer = setTimeout(() => { if (panel.open && panel.isConnected && $('workspace-dialog')?.open && !busy) loadJobs(true); }, 5000);
    } catch (error) { errorText(error); }
  }
  panel.addEventListener('toggle', async () => {
    if (!panel.open) { clearTimeout(timer); return; }
    if (!loaded) {
      try {
        const config = await invoke('remote_config');
        for (const entry of config.servers ?? []) server.append(Object.assign(document.createElement('option'), { value: entry.host, textContent: entry.host }));
        submit.disabled = !(config.servers ?? []).length;
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
      message.textContent = remoteJobText(result.job, result.status);
      if (result.error) errorText(result.error);
      await loadJobs(true);
    } catch (error) { errorText(error); }
    finally { busy = false; submit.disabled = !server.value; }
  };
  return panel;
}

async function showDetail(name, card) {
  card.querySelector('.ws-local-detail')?.remove();
  const box = Object.assign(document.createElement('div'), { className: 'ws-detail ws-local-detail mini' });
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
  $('research-workspaces-btn')?.addEventListener('click', openWorkspaces);
  $('workspace-create-form').onsubmit = createWorkspace;
  $('workspace-source-browse').onclick = async () => {
    try {
      const path = await invoke('pick_folder', { key: 'workspace-source' });
      if (path && !creating) $('workspace-source').value = path;
    } catch (error) {
      $('workspace-create-status').textContent = String(error?.message || error);
    }
  };
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
