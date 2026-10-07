//! AI 助手面板（docs/design-ai-assistant.md 第 8 节）。对话循环与工具都在 `colm-agent` 里，
//! 这里只负责：启动与配置、发消息、渲染事件流（回答、思考过程、工具卡片、审批卡片、用量），
//! 以及历史对话（列出、打开后接着聊、删除；打开面板时自动接上最近一次）。
//!
//! 只依赖 ipc/state/ui/i18n：shell、runner、results 都可能导入它，反过来导入会成环。
//! 模型的回答一律按纯文本建 DOM（不用 innerHTML），表格与代码块由 `renderAnswer` 安全地转成元素。

import { invoke, listen, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, appConfirm } from './ui.js';
import { language, translateZh } from './i18n.js';

// ---- 纯函数（tests/assistant.mjs）----------------------------------------------------------

/** 随消息附上的“当前页面”说明：页面、选中的算例、内核、项目目录。 */
export function viewContext({ step, caseDir, kernel, root }) {
  return [
    step && `page: ${step}`,
    caseDir && `selected case: ${caseDir}`,
    kernel && `kernel: ${kernel}`,
    root && `project directory: ${root}`,
  ].filter(Boolean).join('\n');
}

/** 一行协议事件；读不懂的返回 null。 */
export function parseEvent(line) {
  try {
    const event = typeof line === 'string' ? JSON.parse(line) : line;
    return event && typeof event.type === 'string' ? event : null;
  } catch {
    return null;
  }
}

/** 建算例、运行之后，结果里那个算例的目录（工具卡片上给“在工作台打开”按钮）。 */
export function caseFromResult(tool, resultText) {
  if (!['create_case', 'run_case', 'set_case_fields'].includes(tool)) return null;
  try {
    const result = JSON.parse(resultText);
    if (tool === 'create_case') return result.created ? result.case : null;
    if (tool === 'set_case_fields') return result.case ?? null;
    return null;
  } catch {
    return null;
  }
}

/** 把回答切成块：段落、代码块、表格（`|` 开头的连续行）。 */
export function answerBlocks(text) {
  const blocks = [];
  const lines = String(text ?? '').split('\n');
  let i = 0;
  while (i < lines.length) {
    const line = lines[i];
    if (line.trim().startsWith('```')) {
      const body = [];
      i++;
      while (i < lines.length && !lines[i].trim().startsWith('```')) body.push(lines[i++]);
      i++;
      blocks.push({ kind: 'code', text: body.join('\n') });
      continue;
    }
    if (line.trim().startsWith('|')) {
      const rows = [];
      while (i < lines.length && lines[i].trim().startsWith('|')) {
        const cells = lines[i].trim().replace(/^\||\|$/g, '').split('|').map(c => c.trim());
        // 分隔行 |---|---| 不是数据。
        if (!cells.every(c => /^:?-{2,}:?$/.test(c))) rows.push(cells);
        i++;
      }
      blocks.push({ kind: 'table', rows });
      continue;
    }
    const para = [];
    while (i < lines.length && lines[i].trim() !== '' && !lines[i].trim().startsWith('```') && !lines[i].trim().startsWith('|')) {
      para.push(lines[i++]);
    }
    if (para.length) blocks.push({ kind: 'text', text: para.join('\n') });
    else i++;
  }
  return blocks;
}

// ---- 渲染 -----------------------------------------------------------------------------

const t = text => (language() === 'en' ? translateZh(text) : text);

function element(tag, cls = '', text = '') {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text !== '') el.textContent = String(text);
  return el;
}

/** 用 `answerBlocks` 安全地建回答的 DOM。 */
function renderAnswer(host, text) {
  host.replaceChildren();
  for (const block of answerBlocks(text)) {
    if (block.kind === 'code') {
      host.appendChild(element('pre', 'assistant-code', block.text));
    } else if (block.kind === 'table') {
      const table = element('table', 'assistant-table');
      block.rows.forEach((cells, index) => {
        const row = element('tr');
        for (const cell of cells) row.appendChild(element(index === 0 ? 'th' : 'td', '', cell));
        table.appendChild(row);
      });
      const wrap = element('div', 'result-table-wrap');
      wrap.appendChild(table);
      host.appendChild(wrap);
    } else {
      host.appendChild(element('p', 'assistant-text', block.text));
    }
  }
}

const ui = {
  started: false,
  running: false,
  answer: null,
  answerText: '',
  reasoning: null,
  tools: new Map(),
  /** 面板上显示的这段对话的会话号；新对话在第一条消息前为 null。 */
  conversation: null,
  /** 打开面板时是否已经接上过最近一次对话。 */
  restored: false,
};

function log() {
  return $('assistant-log');
}

function scrollDown() {
  const host = log();
  if (host) host.scrollTop = host.scrollHeight;
}

function setRunning(running) {
  ui.running = running;
  $('assistant-send').disabled = running;
  $('assistant-stop').disabled = !running;
}

function finishAnswer() {
  if (ui.answer) renderAnswer(ui.answer, ui.answerText);
  ui.answer = null;
  ui.answerText = '';
  ui.reasoning = null;
}

function bubble(kind) {
  log().querySelector('.assistant-empty')?.remove();
  const node = element('div', `assistant-msg ${kind}`);
  log().appendChild(node);
  return node;
}

function notice(text) {
  const box = $('assistant-notice');
  box.textContent = text ? t(text) : '';
  box.hidden = !text;
}

function toolCard(event) {
  finishAnswer();
  const card = element('details', `assistant-tool tier-${event.tier}`);
  card.dataset.state = 'running';
  const summary = element('summary');
  summary.append(element('span', 'assistant-tool-dot'), element('span', 'assistant-tool-name', event.name), element('span', 'muted mini assistant-tool-state', '运行中…'));
  if (event.preapproved) summary.appendChild(element('span', 'assistant-tool-auto mini', '已自动批准'));
  const body = element('div', 'assistant-tool-body');
  body.append(element('div', 'muted mini', '参数'), element('pre', 'assistant-code', event.arguments));
  card.append(summary, body);
  bubble('tool').appendChild(card);
  ui.tools.set(event.id, card);
}

function toolResult(event) {
  const card = ui.tools.get(event.id);
  if (!card) return;
  card.dataset.state = event.ok ? 'ok' : 'failed';
  card.classList.remove('awaiting');
  const state = card.querySelector('.assistant-tool-state');
  const timing = event.elapsed_ms == null ? [] : [element('span', '', ` · ${event.elapsed_ms} ms`)];
  state.replaceChildren(...(event.ok ? [element('span', '', '完成'), ...timing] : [element('span', '', '失败')]));
  state.className = `mini assistant-tool-state ${event.ok ? 'muted' : 'assistant-fail'}`;
  const preview = event.result.length > 4000 ? `${event.result.slice(0, 4000)}…` : event.result;
  card.querySelector('.assistant-tool-body').append(element('div', 'muted mini', '结果'), element('pre', 'assistant-code', preview));
  const dir = event.ok ? caseFromResult(event.name, event.result) : null;
  if (dir) {
    const actions = element('div', 'assistant-tool-actions');
    const open = element('button', 'btn-ghost', '在工作台打开这个算例');
    open.type = 'button';
    // 建好的算例直接停在“运行”这一步。
    open.onclick = () => dispatchEvent(new CustomEvent('colm:open-case-dir', { detail: { dir, step: 'run' } }));
    actions.appendChild(open);
    card.appendChild(actions);
    card.open = true;
    // 新建的算例直接在工作台打开（引导模式下窗口已经在它上面，不用再开）。
    if (event.name === 'create_case' && state.selected?.dir !== dir) open.click();
  }
}

function approvalCard(event) {
  finishAnswer();
  // 审批放进对应的工具卡片：一件事一张卡，批准后收成一行。
  const card = ui.tools.get(event.id);
  const box = element('div', 'assistant-approval');
  box.append(element('div', 'assistant-approval-title', '需要你的批准'), element('p', 'mini', event.summary));
  const approve = element('button', 'run-btn', '批准');
  const always = element('button', 'btn-ghost', '本会话都允许');
  always.title = t('批准这次，并在本次会话里不再询问同一类操作');
  const deny = element('button', 'btn-ghost', '拒绝');
  for (const button of [approve, always, deny]) button.type = 'button';
  const decide = (ok, remember = false) => {
    const label = !ok ? '已拒绝' : remember ? '已批准，本会话不再询问这类操作' : '已批准';
    box.replaceChildren(element('div', `mini assistant-decided ${ok ? 'ok' : 'no'}`, label));
    box.className = 'assistant-approval decided';
    const state = card?.querySelector('.assistant-tool-state');
    if (state) state.textContent = ok ? '运行中…' : '已拒绝';
    invoke('assistant_approve', { id: event.id, approve: ok, note: null, remember }).catch(e => status(e));
  };
  approve.onclick = () => decide(true);
  always.onclick = () => decide(true, true);
  deny.onclick = () => decide(false);
  const row = element('div', 'assistant-approval-actions');
  row.append(deny, always, approve);
  box.appendChild(row);
  if (card) {
    card.classList.add('awaiting');
    card.open = true; // 审批前要能看到完整参数
    card.querySelector('.assistant-tool-state').textContent = '等待批准';
    card.appendChild(box);
  } else {
    bubble('approval').appendChild(box);
  }
}

/** 引导模式：把 agent 的界面请求交给 guide.js（它能导入 shell/sites，本模块不能），再把结果回给 agent。 */
function uiRequest(event) {
  const respond = (ok, result) => {
    invoke('assistant_ui_result', { id: event.id, ok, result }).catch(e => status(e));
  };
  const detail = { action: event.action, args: event.args, respond, handled: false };
  dispatchEvent(new CustomEvent('colm:ui-request', { detail }));
  // 没有窗口模块接（例如测试页）时立刻回绝，免得 agent 干等两分钟。
  if (!detail.handled) respond(false, 'the application window cannot be driven here');
}

function handle(event) {
  switch (event.type) {
    case 'ready':
      $('assistant-model').textContent = event.model || '';
      ui.conversation = event.session || ui.conversation;
      break;
    case 'reasoning_delta':
      if (!ui.reasoning) {
        const holder = bubble('reply');
        const details = element('details', 'assistant-reasoning');
        details.appendChild(element('summary', 'mini', '思考过程'));
        ui.reasoning = element('div', 'assistant-reasoning-body');
        details.appendChild(ui.reasoning);
        holder.appendChild(details);
      }
      ui.reasoning.textContent += event.text;
      break;
    case 'assistant_delta':
      if (!ui.answer) ui.answer = bubble('reply');
      ui.answerText += event.text;
      ui.answer.textContent = ui.answerText;
      break;
    case 'tool_call':
      toolCard(event);
      break;
    case 'tool_result':
      toolResult(event);
      break;
    case 'approval_request':
      approvalCard(event);
      break;
    case 'ui_request':
      uiRequest(event);
      return; // 不滚动：窗口在动，面板保持原位
    case 'usage':
      $('assistant-usage').replaceChildren(
        element('span', '', '本轮用量'),
        element('span', '', ` ${event.prompt_tokens}+${event.completion_tokens} tokens · `),
        element('span', '', '本会话'),
        element('span', '', ` ${event.session_prompt_tokens}+${event.session_completion_tokens} tokens`),
      );
      break;
    case 'turn_done':
      finishAnswer();
      setRunning(false);
      break;
    case 'error':
      finishAnswer();
      if (event.message === 'cancelled') bubble('note').textContent = t('已停止。');
      else bubble('error').textContent = event.message;
      setRunning(false);
      break;
    case 'exited':
      ui.started = false;
      setRunning(false);
      notice('助手进程已退出；下次发送时会重新启动。');
      break;
    default:
      break;
  }
  scrollDown();
}

// ---- 设置 ----------------------------------------------------------------------------

async function loadSettings() {
  const settings = await invoke('assistant_settings');
  $('assistant-base').value = settings.base_url;
  $('assistant-model-name').value = settings.model;
  $('assistant-think').value = thinkValue(settings);
  $('assistant-approval').value = settings.approval || 'ask';
  $('assistant-web').value = settings.web_search === false ? 'off' : 'on';
  await refreshKeyStatus(settings.base_url);
  return settings;
}

/** 输入框下的“思考”选框：不思考 = 关闭思考模式；其余是思考强度（默认交给服务端，DeepSeek 为 high）。 */
export function thinkValue(settings) {
  if (settings.thinking === false) return 'off';
  return settings.reasoning_effort || '';
}

export function thinkSettings(value) {
  return value === 'off'
    ? { thinking: false, reasoning_effort: null }
    : { thinking: null, reasoning_effort: value || null };
}

async function changeThink() {
  const settings = { ...(await invoke('assistant_settings')), ...thinkSettings($('assistant-think').value) };
  await invoke('assistant_save_settings', { settings });
  ui.started = false; // 下一条消息发出前重新配置
}

async function refreshKeyStatus(baseUrl) {
  const has = await invoke('assistant_has_key', { baseUrl }).catch(() => false);
  const line = $('assistant-key-status');
  line.textContent = has ? t('✓ 已保存这个服务的 Key') : t('还没有保存这个服务的 Key。');
  line.className = `mini ${has ? 'assistant-key-ok' : 'muted'}`;
  // Key 保存后不回显：输入框空着，用提示文字说明它已经存好了。
  $('assistant-key').placeholder = t(has ? '已保存（为安全起见不显示）；要更换就粘贴新的 Key' : '粘贴后点“保存 Key”');
  $('assistant-key-save').textContent = t(has ? '更换 Key' : '保存 Key');
  return has;
}

function formSettings(previous) {
  return {
    base_url: $('assistant-base').value.trim(),
    model: $('assistant-model-name').value.trim(),
    ...thinkSettings($('assistant-think').value),
    approval: $('assistant-approval').value || 'ask',
    web_search: $('assistant-web').value !== 'off',
    egress_acknowledged: previous?.egress_acknowledged ?? null,
  };
}

async function saveSettings() {
  const previous = await invoke('assistant_settings');
  const settings = formSettings(previous);
  await invoke('assistant_save_settings', { settings });
  ui.started = false;
  await refreshKeyStatus(settings.base_url);
  $('assistant-settings').hidden = true;
  status(t('已保存助手设置'));
}

// ---- 发送 ----------------------------------------------------------------------------

function currentView() {
  const step = document.querySelector('.page:not([hidden])')?.dataset.step;
  return {
    step,
    caseDir: state.selected?.dir,
    kernel: $('kernel')?.value,
    root: $('root')?.value?.trim(),
  };
}

/** 项目目录：开着的项目根，否则选中算例的上一级。 */
function projectRoot(view) {
  if (view.root) return view.root;
  const dir = view.caseDir || '';
  return dir.includes('/') ? dir.slice(0, dir.lastIndexOf('/')) : dir;
}

/** 第一次向某个服务发送前，在面板里问一次数据外发（不用 window.confirm：桌面窗口里弹不出来）。 */
function askConsent(baseUrl) {
  return new Promise(resolve => {
    const card = element('div', 'assistant-consent');
    card.append(
      element('div', 'assistant-approval-title', '发送前请确认'),
      element('p', 'mini', '发送后，你的问题、算例配置、指标与日志片段会发给这个模型服务：'),
      element('p', 'mini assistant-consent-url', baseUrl),
      element('p', 'muted mini', '换用本机的模型（例如 Ollama）可以避免数据外发。只需确认一次。'),
    );
    const ok = element('button', 'run-btn', '同意并发送');
    const no = element('button', 'btn-ghost', '取消');
    for (const button of [ok, no]) button.type = 'button';
    const done = value => { card.remove(); resolve(value); };
    ok.onclick = () => done(true);
    no.onclick = () => done(false);
    const row = element('div', 'assistant-approval-actions');
    row.append(no, ok);
    card.appendChild(row);
    log().querySelector('.assistant-empty')?.remove();
    log().appendChild(card);
    scrollDown();
    ok.focus();
  });
}

async function ensureStarted() {
  let settings = await invoke('assistant_settings');
  if (!(await refreshKeyStatus(settings.base_url))) {
    $('assistant-settings').hidden = false;
    throw new Error(t('请先在设置里保存 API Key。'));
  }
  if (settings.egress_acknowledged !== settings.base_url) {
    if (!(await askConsent(settings.base_url))) throw new Error(t('已取消发送'));
    settings = { ...settings, egress_acknowledged: settings.base_url };
    await invoke('assistant_save_settings', { settings });
  }
  if (!ui.started) {
    const view = currentView();
    // 接上面板上正显示的那段对话（进程重启后也接得上）。
    await invoke('assistant_start', { projectRoot: projectRoot(view), kernelDir: view.kernel || null, resume: ui.conversation });
    ui.started = true;
  }
}

async function send() {
  const text = $('assistant-text').value.trim();
  if (!text || ui.running) return;
  await ensureStarted();
  bubble('user').textContent = text;
  $('assistant-text').value = '';
  setRunning(true);
  notice('');
  await invoke('assistant_send', { text, context: viewContext(currentView()) || null }).catch(e => {
    setRunning(false);
    throw e;
  });
  scrollDown();
}

// ---- 历史对话 ------------------------------------------------------------------------

/** 列表里的时间：今天只写时分，否则写月日。 */
export function sessionTime(ms, now = Date.now()) {
  const date = new Date(ms);
  const today = new Date(now);
  const pad = n => String(n).padStart(2, '0');
  const clock = `${pad(date.getHours())}:${pad(date.getMinutes())}`;
  if (date.toDateString() === today.toDateString()) return clock;
  const day = `${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  return date.getFullYear() === today.getFullYear() ? `${day} ${clock}` : `${date.getFullYear()}-${day}`;
}

function clearLog() {
  const empty = ui.emptyState;
  log().replaceChildren(...(empty ? [empty] : []));
  ui.tools.clear();
  ui.answer = null;
  ui.answerText = '';
  ui.reasoning = null;
  $('assistant-usage').textContent = '';
}

/** 把历史记录画回面板：文字照常排版，工具卡片收起（结果在卡片里）。 */
function renderTranscript(items) {
  clearLog();
  for (const item of items) {
    if (item.kind === 'user') {
      bubble('user').textContent = item.text;
    } else if (item.kind === 'assistant') {
      renderAnswer(bubble('reply'), item.text);
    } else if (item.kind === 'tool') {
      toolCard({ id: item.id, name: item.name, arguments: item.arguments, tier: 'history' });
      const card = ui.tools.get(item.id);
      if (item.result == null) {
        card.dataset.state = 'failed';
        card.querySelector('.assistant-tool-state').textContent = t('未完成');
      } else {
        toolResult({ id: item.id, name: item.name, ok: item.ok, result: item.result });
      }
      card.open = false;
    }
  }
  if (items.length) bubble('note').textContent = t('以上是之前的对话，可以直接接着问。');
  scrollDown();
}

async function openConversation(id) {
  if (ui.running) {
    notice('请先停止当前回答，再切换对话。');
    return;
  }
  const items = await invoke('assistant_transcript', { id });
  renderTranscript(items);
  ui.conversation = id;
  $('assistant-history').hidden = true;
  notice('');
  if (ui.started) await invoke('assistant_resume', { id });
}

async function showHistory() {
  const box = $('assistant-history');
  if (!box.hidden) {
    box.hidden = true;
    return;
  }
  $('assistant-settings').hidden = true;
  const sessions = await invoke('assistant_sessions');
  const list = element('div', 'assistant-history-list');
  if (!sessions.length) list.appendChild(element('p', 'muted mini', '还没有保存的对话。'));
  for (const session of sessions) {
    const row = element('div', `assistant-history-item${session.id === ui.conversation ? ' current' : ''}`);
    const open = element('button', 'assistant-history-open');
    open.type = 'button';
    open.append(
      element('span', 'assistant-history-title', session.title),
      element('span', 'muted mini', `${sessionTime(session.updated_ms)} · ${session.turns} ${t('问')}`),
    );
    open.onclick = () => openConversation(session.id).catch(e => notice(String(e?.message || e)));
    const remove = element('button', 'icon-btn', '×');
    remove.type = 'button';
    remove.title = t('删除这段对话');
    remove.onclick = async () => {
      if (!(await appConfirm(t('删除这段对话？删除后无法恢复。'), { okText: t('删除') }))) return;
      try {
        await invoke('assistant_delete_session', { id: session.id });
        if (session.id === ui.conversation) startNewConversation();
        box.hidden = true;
        await showHistory();
      } catch (e) {
        notice(String(e?.message || e));
      }
    };
    row.append(open, remove);
    list.appendChild(row);
  }
  box.replaceChildren(element('div', 'assistant-history-head', '历史对话'), list);
  box.hidden = false;
}

function startNewConversation() {
  if (ui.running) return;
  clearLog();
  ui.conversation = null;
  if (ui.started) invoke('assistant_new_session').catch(e => status(e));
}

/** 第一次打开面板：面板还空着时接上最近一次对话。 */
async function restoreLatest() {
  if (ui.restored) return;
  ui.restored = true;
  if (ui.conversation || log().querySelector('.assistant-msg')) return;
  const sessions = await invoke('assistant_sessions');
  if (sessions.length) await openConversation(sessions[0].id);
}

/** 助手栏宽度：至少 320，并给主页面留至少 480。 */
export function clampAssistantWidth(width, viewport, railWidth) {
  const max = Math.max(320, viewport - railWidth - 480);
  return Math.round(Math.min(max, Math.max(320, width)));
}

const WIDTH_KEY = 'colm.assistant.width';
const DEFAULT_WIDTH = 440;

function setAssistantWidth(width) {
  const rail = document.querySelector('.rail')?.getBoundingClientRect().width || 250;
  const size = clampAssistantWidth(width, innerWidth, rail);
  document.documentElement.style.setProperty('--assist-w', `${size}px`);
  return size;
}

function restoreWidth() {
  let saved = null;
  try { saved = Number(localStorage.getItem(WIDTH_KEY)); } catch { /* 存储不可用就用默认 */ }
  setAssistantWidth(Number.isFinite(saved) && saved > 0 ? saved : DEFAULT_WIDTH);
}

function saveWidth(width) {
  try { localStorage.setItem(WIDTH_KEY, String(width)); } catch { /* 记不住也不影响使用 */ }
}

function beginResize(event) {
  const panel = $('assistant-panel');
  if (!panel || panel.hidden) return;
  event.preventDefault();
  event.currentTarget?.setPointerCapture?.(event.pointerId);
  document.body.classList.add('resizing-assistant');
  const right = panel.getBoundingClientRect().right;
  let width = panel.getBoundingClientRect().width;
  const move = ev => { width = setAssistantWidth(right - ev.clientX); };
  const up = () => {
    document.body.classList.remove('resizing-assistant');
    saveWidth(width);
    window.removeEventListener('pointermove', move);
    window.removeEventListener('pointerup', up);
    window.removeEventListener('pointercancel', up);
  };
  window.addEventListener('pointermove', move);
  window.addEventListener('pointerup', up);
  window.addEventListener('pointercancel', up);
}

function togglePanel(open = $('assistant-panel').hidden) {
  $('assistant-panel').hidden = !open;
  document.querySelector('.app')?.classList.toggle('assistant-open', open);
  document.body.classList.toggle('assistant-open', open);
  for (const button of document.querySelectorAll('[data-assistant-toggle]')) button.setAttribute('aria-pressed', String(open));
  if (open) restoreWidth();
  if (open) {
    loadSettings().catch(e => status(e));
    restoreLatest().catch(e => notice(String(e?.message || e)));
    $('assistant-text').focus();
  }
}

function wire() {
  if (!$('assistant-panel') || !hasBackend) {
    for (const button of document.querySelectorAll('[data-assistant-toggle]')) button.hidden = !hasBackend;
    return;
  }
  for (const button of document.querySelectorAll('[data-assistant-toggle]')) button.onclick = () => togglePanel();
  $('assistant-close').onclick = () => togglePanel(false);
  $('assistant-resizer').addEventListener('pointerdown', beginResize);
  $('assistant-resizer').addEventListener('dblclick', () => saveWidth(setAssistantWidth(DEFAULT_WIDTH)));
  // 窗口变窄时把助手栏收回到不挤掉主页面的宽度。
  addEventListener('resize', () => {
    if (!$('assistant-panel').hidden) setAssistantWidth($('assistant-panel').getBoundingClientRect().width);
  });
  $('assistant-settings-btn').onclick = () => {
    $('assistant-history').hidden = true;
    $('assistant-settings').hidden = !$('assistant-settings').hidden;
  };
  $('assistant-settings-save').onclick = () => saveSettings().catch(e => status(e?.message || e));
  $('assistant-key-save').onclick = async () => {
    const key = $('assistant-key').value;
    $('assistant-key').value = '';
    try {
      await invoke('assistant_set_key', { baseUrl: $('assistant-base').value.trim(), key });
      ui.started = false; // 助手进程已结束，下次发送时用新 Key 重启
      await refreshKeyStatus($('assistant-base').value.trim());
      status(t('已保存 API Key'));
    } catch (e) {
      status(e?.message || e);
    }
  };
  $('assistant-key-delete').onclick = async () => {
    try {
      await invoke('assistant_delete_key', { baseUrl: $('assistant-base').value.trim() });
      ui.started = false;
      await refreshKeyStatus($('assistant-base').value.trim());
    } catch (e) {
      status(e?.message || e);
    }
  };
  $('assistant-send').onclick = () => send().catch(e => { notice(String(e?.message || e)); });
  $('assistant-stop').onclick = () => invoke('assistant_cancel').catch(e => status(e));
  for (const chip of document.querySelectorAll('.assistant-chip')) {
    chip.onclick = () => {
      $('assistant-text').value = language() === 'en' ? translateZh(chip.dataset.prompt) : chip.dataset.prompt;
      $('assistant-text').focus();
    };
  }
  ui.emptyState = log().querySelector('.assistant-empty');
  $('assistant-new').onclick = () => { $('assistant-history').hidden = true; startNewConversation(); };
  $('assistant-history-btn').onclick = () => showHistory().catch(e => notice(String(e?.message || e)));
  // 回车发送，Shift + 回车换行；输入法选词时的回车（isComposing / keyCode 229）不发送。
  $('assistant-think').addEventListener('change', () => changeThink().catch(e => status(e?.message || e)));
  $('assistant-text').addEventListener('keydown', event => {
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    send().catch(e => notice(String(e?.message || e)));
  });
  listen('assistant://event', event => {
    const parsed = parseEvent(event.payload);
    if (parsed) handle(parsed);
  });
}

wire();
