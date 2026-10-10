//! AI 助手面板（docs/design-ai-assistant.md 第 8 节）。对话循环与工具都在 `colm-agent` 里，
//! 这里只负责：启动与配置、发消息、渲染事件流（回答、思考过程、工具卡片、审批卡片、用量），
//! 以及历史对话（列出、打开后接着聊、删除；重启后从新对话开始）。
//!
//! 只依赖 ipc/state/ui/i18n：shell、runner、results 都可能导入它，反过来导入会成环。
//! 模型的回答一律按纯文本建 DOM（不用 innerHTML），表格与代码块由 `renderAnswer` 安全地转成元素。

import { invoke, listen, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status, appConfirm } from './ui.js';
import { language, translateZh } from './i18n.js';
import { API_PROVIDERS, providerId, rememberProfile, selectProvider, apiEfforts, parseApiOptions, settingsForSave } from './assistant-providers.js';

// ---- 纯函数（tests/assistant.mjs）----------------------------------------------------------

/** 随消息附上的“当前页面”说明：页面与具体步骤、选中的算例、内核、项目目录，以及这一页的关键信息（`details`）。 */
export function viewContext({ step, flow, caseDir, kernel, root, variable, details = [] }) {
  return [
    step && `page: ${step}`,
    flow && flow !== step && `workflow step: ${flow}`,
    caseDir && `selected case: ${caseDir}`,
    kernel && `kernel: ${kernel}`,
    root && `file operations directory: ${root}`,
    variable && `selected history variable: ${variable}`,
    ...details,
  ].filter(Boolean).join('\n');
}

const fixed = (value, digits) => (Number.isFinite(value) ? Number(value).toFixed(digits) : 'n/a');

/**
 * 这一页的关键信息（英文，给模型看，不显示）：评估页附上已算出的指标，研究页附上 Study 状态与 AI 模式。
 * `metrics` 是 `state.resultMetrics` 里属于选中算例的行；`badges` 是 `state.studyBadges`。
 */
export function pageDetails(flow, { metrics = [], badges = {}, batch = 0, adopted = [] } = {}) {
  const lines = [];
  if (batch > 1) lines.push(`cases in this batch: ${batch}`);
  // 用户在开发工作区面板里启用的实验内核：之后用 Fortran 内核跑这些预设时用的不是正式内核。
  for (const kernel of adopted) {
    const from = kernel?.experimental;
    if (from?.workspace) {
      lines.push(`experimental Fortran kernel in use for preset ${kernel.preset}: built in development workspace ${from.workspace} (commit ${String(from.head ?? '').slice(0, 8)}), not the official kernel`);
    }
  }
  if (flow?.startsWith('result-') || flow === 'research') {
    const rows = metrics.filter(row => row && row.name).slice(0, 8);
    if (rows.length) {
      lines.push('evaluation already computed in the window (model vs observations):');
      for (const row of rows) {
        lines.push(`- ${row.name}: n=${row.n ?? 'n/a'}, NSE=${fixed(row.nse, 3)}, KGE=${fixed(row.kge, 3)}, RMSE=${fixed(row.rmse, 3)}, bias=${fixed(row.bias, 3)}`);
      }
    }
  }
  if (flow === 'result-tuning' && badges.tuning) lines.push(`calibration Study status: ${badges.tuning}`);
  if (flow === 'result-uncertainty' && badges.uq) lines.push(`uncertainty Study status: ${badges.uq}`);
  if (flow === 'hybrid-learn') lines.push('AI hybrid modeling mode: learned parameters (network sets parameters per patch from features)');
  if (flow === 'hybrid-process') lines.push('AI hybrid modeling mode: process replacement (network replaces soil-moisture stress beta)');
  return lines;
}

/** 每一步给出的提问建议（`label` 显示在按钮上，`prompt` 填进输入框）。没有建议的步骤返回空数组。 */
export function pagePrompts(flow) {
  const P = (label, prompt) => ({ label, prompt });
  if (!flow) return [];
  if (flow.startsWith('basic-')) {
    return [P('帮我建算例', '帮我用这个目录里的站点建一个算例，需要我拿主意的地方再问我。')];
  }
  if (flow.startsWith('params-')) {
    return [P('解释这一页的参数', '解释当前页面这些过程参数的含义、常用取值，以及改动它们会影响哪些输出。')];
  }
  const table = {
    run: [
      P('检查运行设置', '检查当前算例的运行设置（时段、预热、输出）是否合理。'),
      P('为什么失败', '请用固定启动失败流程诊断当前算例，先调用 diagnostic_plan（workflow=startup）；按已确认事实、待验证原因、下一步实验报告。'),
    ],
    'result-overview': [P('总结这次结果', '总结当前算例的结果：模拟时段、主要输出，以及有没有明显异常。')],
    'result-series': [
      P('诊断选中变量', '请用固定通量偏差流程分析当前选中的变量，先调用 diagnostic_plan（workflow=flux）；核对单位、时段和观测覆盖，再区分已确认事实、待验证原因和下一步实验。'),
      P('偏差在哪个季节', '时间序列上，模型与观测的偏差主要出现在哪个季节或时段？'),
    ],
    'result-evaluation': [
      P('分析潜热偏差', '请用固定通量偏差流程分析当前算例的潜热，先调用 diagnostic_plan（workflow=flux，variable=f_lfevpa）；缺少观测或对齐信息时明确说明，不凭聚合指标认定原因。'),
      P('解释这些指标', '解释当前算例的评估指标：哪些变量模拟得好、哪些差，可能的原因是什么？'),
      P('偏差在哪个季节', '偏差主要出现在哪个季节或时段？请结合时间序列与指标说明。'),
    ],
    'result-comparison': [P('哪个站点最差', '多站点比较里哪个站点模拟得最差？它和其他站点有什么不同？')],
    'result-diagnostics': [P('检查闭合', '请用固定闭合诊断流程检查当前算例，先调用 diagnostic_plan（workflow=closure）；同时核对 f_xerr 和 f_zerr，缺失时不能宣称闭合通过。')],
    research: [
      P('我该做哪种研究', '根据当前算例的评估结果，我应该先做参数率定、不确定性分析，还是 AI 混合建模？为什么？'),
      P('定位 Rust/Fortran 差异', '请用固定对齐诊断流程检查当前算例，先调用 diagnostic_plan（workflow=parity）；先核对工作区、版本、平台和构建状态，运行验证前说明所需操作。'),
    ],
    'result-tuning': [
      P('该率定哪些参数', '针对当前算例的主要偏差，参数率定应该选哪些参数、范围怎么定？'),
      P('解读率定结果', '请用固定率定诊断流程解读当前结果，先调用 diagnostic_plan（workflow=calibration）；同时检查参数范围、可辨识性、训练与验证期表现，不把压边界直接当作结构偏差证据。'),
    ],
    'result-uncertainty': [P('哪些参数最敏感', '当前算例的输出对哪些参数最敏感？不确定性分析该选哪些参数和范围？')],
    'hybrid-learn': [P('适合学哪些参数', '在当前算例上，哪些参数适合让网络按地点特征去学？输入特征怎么选？')],
    'hybrid-process': [P('能不能替换 β', '当前算例能不能用 AI 替换土壤水分胁迫 β？前提条件（植物水力、地表模式、引擎）满足吗？')],
  };
  return table[flow] ?? [];
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

export function taskPhase(tool) {
  if (['run_status', 'read_case_config', 'path_info', 'list_cases', 'environment_doctor'].includes(tool)) return 'checkdata';
  if (['run_case', 'run_case_with', 'parity_check', 'regression_check', 'compare_outputs', 'run_tests'].includes(tool)) return 'validate';
  return tool === 'write_text_file' ? 'report' : 'localise';
}

export function taskProgressText(task) {
  const phases = { checkdata: '检查数据', localise: '定位问题', validate: '验证假设', report: '生成报告' };
  const states = { running: '进行中', interrupted: '已中断，等待续接', failed: '遇到问题，等待核对', answered: '已回答，验证以工具证据为准' };
  const actions = task.actions ?? [];
  const returned = actions.filter(a => a.state !== 'unconfirmed').length;
  return [t('任务进度'), t(phases[task.phase] ?? '检查数据'), t(states[task.state] ?? '等待核对'),
    `${returned}/${actions.length} ${t('工具已返回')}`,
    task.requires_reconciliation && t('先核对已有结果，不自动重放操作')].filter(Boolean).join(' · ');
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

/** 工具结果出来后要不要自动在工作台打开新算例：只对当场建成的，不对回放的历史。 */
export function shouldOpenCreatedCase(event, dir, selectedDir) {
  return !event.replay && event.name === 'create_case' && !!dir && dir !== selectedDir;
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
  configKey: null,
  task: null,
  progress: null,
  running: false,
  answer: null,
  answerText: '',
  reasoning: null,
  tools: new Map(),
  /** 面板上显示的这段对话的会话号；新对话在第一条消息前为 null。 */
  conversation: null,
  pendingNewConversation: true,
  inputHistory: [],
  inputHistoryLoading: null,
  pendingInputs: [],
  historyIndex: null,
  historyDraft: '',
  /** 本机 Codex 的模型清单（`model/list`），第一次用到时取。 */
  codexModels: null,
  opencodeModels: null,
  opencodeModelError: null,
  choicesRequest: 0,
  statusRequest: 0,
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
    // 当场新建的算例直接在工作台打开（引导模式下窗口已经在它上面，不用再开）。
    // 回放历史对话时不开：那是以前建的，打开会把用户正在看的页面换掉（第 633 轮）。
    if (shouldOpenCreatedCase(event, dir, state.selected?.dir)) open.click();
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
  always.hidden = event.explicit_only === true;
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
    case 'task_state':
      if (!ui.running) renderTask(event.task);
      break;
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
      if (ui.task) {
        ui.task.phase = taskPhase(event.name);
        ui.task.actions.push({ id: event.id, state: 'unconfirmed' });
        renderTask(ui.task);
      }
      break;
    case 'tool_result':
      toolResult(event);
      if (ui.task) {
        const action = ui.task.actions.find(a => a.id === event.id);
        if (action) action.state = event.ok ? 'success' : 'failed';
        renderTask(ui.task);
      }
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
      if (ui.task) renderTask({ ...ui.task, phase: 'report', state: 'answered' });
      break;
    case 'error':
      finishAnswer();
      if (event.message === 'cancelled') bubble('note').textContent = t('已停止。');
      else bubble('error').textContent = event.message;
      setRunning(false);
      if (ui.task) renderTask({ ...ui.task, state: 'failed' });
      break;
    case 'exited':
      ui.started = false;
      setRunning(false);
      if (ui.task?.state === 'running') renderTask({ ...ui.task, state: 'interrupted', requires_reconciliation: true });
      notice('助手进程已退出；下次发送时会重新启动。');
      break;
    default:
      break;
  }
  scrollDown();
}

function renderTask(task) {
  ui.task = task;
  if (!ui.progress) {
    ui.progress = element('p', 'mini assistant-task-progress');
    ui.progress.setAttribute('role', 'status');
    log().prepend(ui.progress);
  }
  ui.progress.textContent = taskProgressText(task);
  ui.progress.title = task.goal || '';
}

// ---- 设置 ----------------------------------------------------------------------------

async function loadSettings() {
  const settings = await invoke('assistant_settings');
  ui.settingsDraft = settings;
  ui.settingsBackend = settings.backend || 'builtin';
  renderApiProfile(settings);
  $('assistant-approval').value = settings.approval || 'ask';
  $('assistant-web').value = settings.web_search === false ? 'off' : 'on';
  $('assistant-backend').value = settings.backend || 'builtin';
  await renderChoices(settings, settings.backend || 'builtin');
  refreshBackendStatus().catch(e => status(e));
  await refreshKeyStatus(settings.base_url);
  return settings;
}

function renderApiProfile(settings) {
  $('assistant-settings').dataset.provider = providerId(settings);
  fillSelect($('assistant-provider'), API_PROVIDERS.map(p => [p.id, p.name]), providerId(settings));
  $('assistant-base').value = settings.base_url || '';
  $('assistant-model-name').value = settings.model || '';
  $('assistant-api-format').value = settings.api_format || 'chat_completions';
  $('assistant-max-output').value = settings.max_output_tokens ?? 16384;
  $('assistant-timeout').value = settings.timeout_seconds ?? 600;
  $('assistant-strict').checked = settings.strict === true;
  $('assistant-api-options').value = JSON.stringify(settings.api_options || {}, null, 2);
  const preset = API_PROVIDERS.find(p => p.id === providerId(settings));
  $('assistant-models').replaceChildren(...(preset?.models || (preset?.model ? [preset.model] : [])).map(model => {
    const option = document.createElement('option'); option.value = model; return option;
  }));
  $('assistant-model-status').textContent = '';
  renderEndpointNote();
}

function renderEndpointNote() {
  const preset = API_PROVIDERS.find(p => p.id === $('assistant-provider').value);
  const address = $('assistant-base').value.trim();
  const normalize = url => url.replace(/\/+$/, '');
  $('assistant-endpoint-note').textContent = preset?.base_url && normalize(address) !== normalize(preset.base_url)
    ? `${t('自定义地址')}：${address}` : '';
}

function readApiProfile(settings) {
  const positive = id => {
    const value = Number($(id).value);
    if (!Number.isSafeInteger(value) || value < 1) throw new Error(t('输出上限与超时必须是正整数。'));
    return value;
  };
  return rememberProfile({ ...settings,
    provider_id: $('assistant-provider').value,
    base_url: $('assistant-base').value.trim(), model: $('assistant-model-name').value.trim(),
    api_format: $('assistant-api-format').value,
    api_options: parseApiOptions($('assistant-api-options').value),
    strict: $('assistant-strict').checked,
    max_output_tokens: positive('assistant-max-output'), timeout_seconds: positive('assistant-timeout'),
  });
}

function captureSettingsDraft() {
  let draft = readApiProfile(ui.settingsDraft || {});
  const backend = ui.settingsBackend || 'builtin';
  draft = backend === 'builtin'
    ? rememberProfile({ ...draft, ...thinkSettings($('assistant-think').value) })
    : withChoice(draft, backend, { model: $('assistant-ext-model').value, effort: $('assistant-think').value });
  return draft;
}

async function changeProvider() {
  const next = $('assistant-provider').value;
  const old = providerId(ui.settingsDraft);
  try {
    $('assistant-provider').value = old;
    if ($('assistant-key').value.trim()) throw new Error(t('请先保存或清空当前服务的 Key，再切换服务商。'));
    const draft = captureSettingsDraft();
    ui.settingsDraft = selectProvider(draft, next);
    renderApiProfile(ui.settingsDraft);
    await renderChoices(ui.settingsDraft, $('assistant-backend').value);
    await refreshKeyStatus($('assistant-base').value.trim());
  } catch (error) { $('assistant-provider').value = old; throw error; }
}

async function refreshApiModels() {
  const button = $('assistant-model-refresh');
  const baseUrl = $('assistant-base').value.trim();
  const apiFormat = $('assistant-api-format').value;
  button.disabled = true;
  $('assistant-model-status').textContent = t('正在检查…');
  try {
    const models = await invoke('assistant_api_models', { baseUrl, apiFormat });
    if ($('assistant-base').value.trim() !== baseUrl || $('assistant-api-format').value !== apiFormat) return;
    $('assistant-models').replaceChildren(...models.map(model => {
      const option = document.createElement('option'); option.value = model; return option;
    }));
    $('assistant-model-status').textContent = models.length
      ? `${t('可用模型')}：${models.length}` : t('服务未返回模型，可手动输入。');
  } catch (error) {
    $('assistant-model-status').textContent = `${t('模型刷新失败，当前输入已保留。')} ${error?.message || error}`;
  } finally { button.disabled = false; }
}

/** 输入框下的“思考”选框：不思考 = 关闭思考模式；其余是思考强度（默认交给服务端，DeepSeek 为 high）。 */
export function thinkValue(settings) {
  if (settings.thinking === false) return 'off';
  if (settings.thinking === true && !settings.reasoning_effort) return 'on';
  return settings.reasoning_effort || '';
}

export function thinkSettings(value) {
  return ['off', 'on'].includes(value)
    ? { thinking: value === 'on', reasoning_effort: null }
    : { thinking: null, reasoning_effort: value || null };
}

/** 外部后端的模型选项：[值, 显示名]；第一项是“默认”。 */
export function modelOptions(backend, codexModels) {
  if (backend === 'claude_code') {
    return [['', '默认'], ['fable', 'Fable'], ['opus', 'Opus'], ['sonnet', 'Sonnet'], ['haiku', 'Haiku']];
  }
  const models = codexModels || [];
  if (backend === 'opencode') {
    return [['', '请选择模型'], ...models.map(m => [m.id, `${m.name || m.id} · ${m.id}`])];
  }
  const fallback = models.find(m => m.default);
  return [
    ['', fallback ? `默认（${fallback.name || fallback.id}）` : '默认'],
    ...models.map(m => [m.id, m.name || m.id]),
  ];
}

/** 输入框下“思考”选框的选项：内置后端是 DeepSeek 的强度；Claude Code 是 --effort 的五档；
 *  Codex 随所选模型（没选用默认模型）而定，取不到清单时给常见的四档。 */
export function thinkOptions(backend, model, codexModels, provider = 'deepseek') {
  if (backend === 'builtin') {
    return apiEfforts(provider, model).map(value => [value, value === '' ? '思考：默认' : value === 'off' ? '不思考' : value === 'on' ? '开启思考' : `思考：${value}`]);
  }
  if (backend === 'opencode') {
    const entry = (codexModels || []).find(m => m.id === model);
    return [['', '思考：默认'], ...(entry?.efforts || []).map(e => [e, `思考：${e}`])];
  }
  let efforts = ['low', 'medium', 'high', 'xhigh', 'max'];
  let fallback = '';
  if (backend === 'codex') {
    const models = codexModels || [];
    const entry = models.find(m => m.id === model) || models.find(m => m.default);
    efforts = entry?.efforts?.length ? entry.efforts : ['low', 'medium', 'high', 'xhigh'];
    fallback = entry?.default_effort ? `（${entry.default_effort}）` : '';
  }
  return [['', `思考：默认${fallback}`], ...efforts.map(e => [e, `思考：${e}`])];
}

/** 当前后端在设置里存的思考强度。 */
export function currentThink(settings, backend) {
  if (backend === 'builtin') return thinkValue(settings);
  return settings.external?.[backend]?.effort || '';
}

/** 把一个外部后端的模型或强度改进设置（空值表示用默认）。 */
export function withChoice(settings, backend, patch) {
  const previous = settings.external?.[backend] || {};
  const choice = { model: previous.model ?? null, effort: previous.effort ?? null, ...patch };
  for (const key of ['model', 'effort']) if (!choice[key]) choice[key] = null;
  return { ...settings, external: { ...(settings.external || {}), [backend]: choice } };
}

function fillSelect(select, options, value) {
  select.replaceChildren(...options.map(([v, label]) => {
    const option = document.createElement('option');
    option.value = v;
    option.textContent = t(label);
    return option;
  }));
  // 存的值不在清单里（例如 Codex 下线了那个模型）时也留着，免得悄悄改掉用户的选择。
  if (value && !options.some(([v]) => v === value)) {
    const option = document.createElement('option');
    option.value = value;
    option.textContent = value;
    select.appendChild(option);
  }
  select.value = value || '';
}

async function codexModels() {
  if (!ui.codexModels) ui.codexModels = await invoke('assistant_codex_models').catch(() => []);
  return ui.codexModels;
}

async function externalModels(backend) {
  if (backend === 'codex') return codexModels();
  if (backend === 'opencode') {
    if (!ui.opencodeModels) {
      ui.opencodeModels = await invoke('assistant_opencode_models');
      ui.opencodeModelError = null;
    }
    return ui.opencodeModels;
  }
  return null;
}

async function refreshExternalModels() {
  const button = $('assistant-ext-refresh');
  button.disabled = true;
  try {
    ui.opencodeModels = await invoke('assistant_opencode_models');
    ui.opencodeModelError = null;
    if ($('assistant-backend').value !== 'opencode') return;
    ui.settingsDraft = captureSettingsDraft();
    await renderChoices(ui.settingsDraft, 'opencode');
    await refreshBackendStatus();
  } catch (error) {
    ui.opencodeModelError = String(error?.message || error);
    status(`${t('模型刷新失败，当前输入已保留。')} ${ui.opencodeModelError}`);
  }
  finally { button.disabled = false; }
}

/** 按后端填设置里的模型选框与输入框下的思考选框。 */
async function renderChoices(settings, backend) {
  const request = ++ui.choicesRequest;
  const model = backend === 'builtin' ? settings.model : settings.external?.[backend]?.model || '';
  const render = models => {
    if (backend !== 'builtin') fillSelect($('assistant-ext-model'), modelOptions(backend, models), model);
    fillSelect($('assistant-think'), thinkOptions(backend, model, models, providerId(settings)), currentThink(settings, backend));
  };
  // Show the destination draft before discovery can yield to another switch.
  render(backend === 'opencode' ? ui.opencodeModels : ui.codexModels);
  const controls = ['assistant-ext-model', 'assistant-think', 'assistant-settings-save'];
  controls.forEach(id => { $(id).disabled = true; });
  try {
    const models = await externalModels(backend);
    if (request !== ui.choicesRequest) return;
    render(models);
  } catch (error) {
    if (request !== ui.choicesRequest) return;
    if (backend === 'opencode') ui.opencodeModelError = String(error?.message || error);
    status(`${t('模型刷新失败，当前输入已保留。')} ${error?.message || error}`);
  } finally {
    if (request === ui.choicesRequest) controls.forEach(id => { $(id).disabled = false; });
  }
  if (request !== ui.choicesRequest) return;
  $('assistant-think').title = t(backend === 'builtin'
    ? '思考强度：随时可改，下一条消息生效'
    : '思考强度：随时可改，下一条消息生效（外部后端）');
}

async function changeThink() {
  if (!$('assistant-settings').hidden) { ui.settingsDraft = captureSettingsDraft(); return; }
  const saved = await invoke('assistant_settings');
  const backend = saved.backend || 'builtin';
  const value = $('assistant-think').value;
  const settings = backend === 'builtin'
    ? rememberProfile({ ...saved, ...thinkSettings(value) })
    : withChoice(saved, backend, { effort: value });
  await invoke('assistant_save_settings', { settings });
  ui.started = false; // 下一条消息发出前重新配置
}

/** 设置里换了模型：Codex 的强度选项随模型变。 */
async function changeExternalModel() {
  const backend = $('assistant-backend').value;
  const request = ui.choicesRequest;
  const model = $('assistant-ext-model').value;
  const models = await externalModels(backend);
  if (request !== ui.choicesRequest || backend !== $('assistant-backend').value || model !== $('assistant-ext-model').value) return;
  const think = $('assistant-think');
  const keep = think.value;
  const options = thinkOptions(backend, model, models);
  fillSelect(think, options, options.some(([v]) => v === keep) ? keep : '');
}

/** 外部后端缺了什么（没装、没登录）；齐了返回 null。 */
export function backendProblem(backend, info) {
  const external = EXTERNAL[backend];
  if (!external) return null;
  if (!info?.installed) return t(external.missing);
  if (info.error) return String(info.error);
  if (backend === 'opencode') return info.configured ? null : t(external.loggedOut);
  if (!info.logged_in) return t(external.loggedOut);
  return null;
}

/** 设置里“后端”下面的一行状态；返回那个后端的状态（内置后端返回 null）。 */
async function refreshBackendStatus(backend = $('assistant-backend').value) {
  const request = ++ui.statusRequest;
  const line = $('assistant-backend-status');
  // 服务地址、模型、API Key 只属于内置后端；Codex / Claude Code 用各自的登录，不显示这些。
  for (const el of document.querySelectorAll('#assistant-settings [data-backend-only]')) {
    el.hidden = !el.dataset.backendOnly.split(' ').includes(backend);
  }
  if (backend === 'builtin') {
    line.textContent = '';
    line.className = 'mini muted';
    return null;
  }
  line.textContent = t('正在检查…');
  const status = await invoke('assistant_backend_status').catch(e => ({ error: String(e?.message || e) }));
  const info = status?.[backend];
  if (request !== ui.statusRequest) return info;
  const setup = $('assistant-opencode-setup');
  if (setup) {
    setup.hidden = backend !== 'opencode' || info?.configured || !info?.setup_command;
    $('assistant-opencode-command').textContent = info?.setup_command || '';
  }
  const problem = status?.error ?? (backend === 'opencode' ? ui.opencodeModelError : null) ?? backendProblem(backend, info);
  if (problem) {
    line.textContent = problem;
    line.className = 'mini assistant-fail';
  } else {
    const plan = info.subscription ? `${info.subscription} ${t('订阅')}` : (info.auth_method || '');
    line.textContent = `✓ ${t(backend === 'opencode' ? '已配置' : '已登录')}${plan ? ` · ${plan}` : ''} · ${info.version || ''}`;
    line.className = 'mini assistant-key-ok';
  }
  return info;
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
  return { ...previous, ...captureSettingsDraft(),
    approval: $('assistant-approval').value || 'ask',
    web_search: $('assistant-web').value !== 'off',
    backend: $('assistant-backend').value || 'builtin',
    egress_acknowledged: previous?.egress_acknowledged ?? null,
  };
}

async function saveSettings() {
  const previous = await invoke('assistant_settings');
  const draft = formSettings(previous);
  const settings = settingsForSave(draft, previous);
  await invoke('assistant_save_settings', { settings });
  ui.started = false;
  ui.settingsDraft = draft;
  await refreshKeyStatus(settings.base_url);
  $('assistant-settings').hidden = true;
  status(t(Object.keys(draft.api_profiles || {}).length > Object.keys(settings.api_profiles).length
    ? '已保存助手设置；未填写完整的服务仅保留在当前表单。' : '已保存助手设置'));
}

// ---- 发送 ----------------------------------------------------------------------------

function currentView() {
  const step = document.querySelector('.page:not([hidden])')?.dataset.step;
  const caseDir = state.selected?.dir;
  return {
    step,
    flow: state.step,
    caseDir,
    variable: (state.step === 'result-series' || state.step === 'result-diagnostics') ? $('var')?.value : null,
    kernel: $('kernel')?.value,
    root: $('root')?.value?.trim(),
    details: pageDetails(state.step, {
      metrics: (state.resultMetrics ?? []).filter(row => !caseDir || row.case_dir === caseDir),
      badges: state.studyBadges ?? {},
      batch: state.batch?.length ?? 0,
      adopted: state.adoptedKernels ?? [],
    }),
  };
}

/** 输入框上方的提问建议：跟着当前步骤变；点一下填进输入框，可以改了再发。 */
function renderPageChips() {
  const host = $('assistant-page-chips');
  if (!host) return;
  const prompts = pagePrompts(state.step);
  host.hidden = !prompts.length;
  host.replaceChildren(...prompts.map(({ label, prompt }) => {
    const chip = document.createElement('button');
    chip.type = 'button';
    chip.className = 'assistant-chip';
    chip.textContent = language() === 'en' ? translateZh(label) : label;
    chip.onclick = () => {
      fillPrompt(prompt);
    };
    return chip;
  }));
}

/** 项目目录：开着的项目根，否则选中算例的上一级。 */
export function projectRoot(view) {
  const root = view.root?.trim();
  let dir = root || view.caseDir || '';
  if (/^(?:[A-Za-z]:[\\/]|\\\\)/.test(dir)) dir = dir.replace(/\\/g, '/');
  if (root) return dir;
  dir = dir.replace(/\/+$/, '');
  if (!dir) return view.caseDir ? '/' : '';
  if (/^[A-Za-z]:$/.test(dir)) return `${dir}/`;
  if (/^\/\/[^/]+\/[^/]+$/.test(dir)) return dir;
  const slash = dir.lastIndexOf('/');
  if (slash < 0) return dir;
  const parent = dir.slice(0, slash);
  return /^[A-Za-z]:$/.test(parent) ? `${parent}/` : parent || '/';
}

/** 第一次向某个服务发送前，在面板里问一次数据外发（不用 window.confirm：桌面窗口里弹不出来）。 */
/** 外部后端的说明：数据发给谁、用量算在哪。 */
const EXTERNAL = {
  codex: {
    consent: '发送后，你的问题、算例配置、指标、日志片段与授权目录中的文本文件片段会经你本机的 Codex 发给 OpenAI，用量计入你的 ChatGPT 订阅。',
    missing: '本机没有找到 Codex：请先安装它，然后在终端运行 codex login 登录。',
    loggedOut: 'Codex 还没有登录：请在终端运行 codex login 登录。',
  },
  claude_code: {
    consent: '发送后，你的问题、算例配置、指标、日志片段与授权目录中的文本文件片段会经你本机的 Claude Code 发给 Anthropic，用量计入你的 Claude 订阅。',
    missing: '本机没有找到 Claude Code：请先安装它，然后在终端运行 claude 并登录。',
    loggedOut: 'Claude Code 还没有登录：请在终端运行 claude 并登录。',
  },
  opencode: {
    consent: '发送后，你的问题、算例配置、指标、日志片段与授权目录中的文本文件片段会经本机 OpenCode 发给所选模型服务商，用量按该服务商的账户配置计费。',
    missing: '本机没有找到 OpenCode：请先安装它，并配置模型服务商。',
    loggedOut: 'CoLM 的 OpenCode 配置尚未连接模型：展开“首次连接模型”完成连接，再刷新模型。',
  },
};

export function egressTarget(settings) {
  const backend = settings.backend || 'builtin';
  if (backend === 'builtin') return settings.base_url;
  return backend === 'opencode' ? `opencode:${settings.external?.opencode?.model || ''}` : backend;
}

function askConsent(target, backend = target) {
  return new Promise(resolve => {
    const card = element('div', 'assistant-consent');
    const external = EXTERNAL[backend];
    card.append(
      element('div', 'assistant-approval-title', '发送前请确认'),
      ...(external
        ? [element('p', 'mini', external.consent), ...(backend === 'opencode' ? [element('p', 'mini assistant-consent-url', target.slice('opencode:'.length))] : [])]
        : [
          element('p', 'mini', '发送后，你的问题、算例配置、指标、日志片段与授权目录中的文本文件片段会发给这个模型服务：'),
          element('p', 'mini assistant-consent-url', target),
        ]),
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
  const backend = settings.backend || 'builtin';
  if (backend === 'builtin') {
    if (!(await refreshKeyStatus(settings.base_url))) {
      $('assistant-settings').hidden = false;
      throw new Error(t('请先在设置里保存 API Key。'));
    }
  } else {
    const info = await refreshBackendStatus(backend);
    const problem = backendProblem(backend, info);
    if (problem) {
      $('assistant-settings').hidden = false;
      throw new Error(problem);
    }
    if (backend === 'opencode' && !settings.external?.opencode?.model) {
      $('assistant-settings').hidden = false;
      throw new Error(t('请先选择 OpenCode 模型。'));
    }
  }
  // OpenCode 可跨服务商切换，确认绑定到所选 provider/model。
  const target = egressTarget(settings);
  if (settings.egress_acknowledged !== target) {
    if (!(await askConsent(target, backend))) throw new Error(t('已取消发送'));
    settings = { ...settings, egress_acknowledged: target };
    await invoke('assistant_save_settings', { settings });
  }
  while (true) {
    const view = currentView();
    view.root = projectRoot(view);
    const kernelDir = view.kernel || null;
    const configKey = JSON.stringify([view.root, kernelDir]);
    if (!ui.started || ui.configKey !== configKey) {
      // 接上面板上正显示的那段对话（进程重启后也接得上）。
      await invoke('assistant_start', { projectRoot: view.root, kernelDir, resume: ui.pendingNewConversation ? null : ui.conversation });
      ui.started = true;
      ui.configKey = configKey;
      // 启动期间用户也可能切换目录；发送前确认授权仍然对应当前页面。
      continue;
    }
    if (ui.pendingNewConversation) {
      await invoke('assistant_new_session');
      ui.pendingNewConversation = false;
    }
    return view;
  }
}

async function send() {
  const text = $('assistant-text').value.trim();
  if (!text || ui.running) return;
  setRunning(true);
  let view;
  try { view = await ensureStarted(); }
  catch (error) { setRunning(false); throw error; }
  bubble('user').textContent = text;
  renderTask({ goal: text, state: 'running', phase: 'checkdata', actions: [] });
  $('assistant-text').value = '';
  setRunning(true);
  notice('');
  await invoke('assistant_send', { text, context: viewContext(view) || null }).catch(e => {
    setRunning(false);
    throw e;
  });
  ui.inputHistory = inputHistory([...ui.inputHistory, text]);
  if (ui.inputHistoryLoading) ui.pendingInputs.push(text);
  resetInputHistory();
  scrollDown();
}

/** Keep the same bounded, chronological view as the stored user transcripts. */
export function inputHistory(items) {
  const texts = items.filter(text => typeof text === 'string' && text.trim() && text.length <= 32000);
  return texts.filter((text, i) => text !== texts[i - 1]).slice(-100);
}

function resetInputHistory() {
  ui.historyIndex = null;
  ui.historyDraft = '';
}

function fillPrompt(prompt) {
  $('assistant-text').value = language() === 'en' ? translateZh(prompt) : prompt;
  resetInputHistory();
  $('assistant-text').focus();
}

async function refreshInputHistory(force = false) {
  if (ui.inputHistoryLoading) {
    if (!force) return ui.inputHistoryLoading;
    await ui.inputHistoryLoading.catch(() => {});
    return refreshInputHistory(true);
  }
  ui.pendingInputs = [];
  ui.inputHistoryLoading = invoke('assistant_input_history').then(items => {
    const recalled = ui.historyIndex === null ? null : ui.inputHistory[ui.historyIndex];
    ui.inputHistory = inputHistory([...items, ...ui.pendingInputs]);
    if (recalled !== null) {
      const index = ui.inputHistory.lastIndexOf(recalled);
      if (index < 0) { $('assistant-text').value = ui.historyDraft; resetInputHistory(); }
      else ui.historyIndex = index;
    }
  }).finally(() => { ui.inputHistoryLoading = null; ui.pendingInputs = []; });
  return ui.inputHistoryLoading;
}

/** Multiline editing and IME selection retain their normal arrow-key behavior. */
export function recallInput(event, textarea, history = ui.inputHistory, cursor = ui) {
  if (!['ArrowUp', 'ArrowDown'].includes(event.key) || event.isComposing || event.keyCode === 229
      || event.shiftKey || event.ctrlKey || event.altKey || event.metaKey || !history.length
      || textarea.selectionStart !== textarea.selectionEnd) return false;
  const browsing = cursor.historyIndex !== null;
  if (!browsing && (event.key === 'ArrowDown' || textarea.value.slice(0, textarea.selectionStart).includes('\n'))) return false;
  if (!browsing) {
    cursor.historyDraft = textarea.value;
    cursor.historyIndex = history.length;
  }
  cursor.historyIndex = Math.max(0, Math.min(history.length, cursor.historyIndex + (event.key === 'ArrowUp' ? -1 : 1)));
  textarea.value = cursor.historyIndex === history.length ? cursor.historyDraft : history[cursor.historyIndex];
  if (cursor.historyIndex === history.length) cursor.historyIndex = null;
  textarea.setSelectionRange(textarea.value.length, textarea.value.length);
  event.preventDefault();
  return true;
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
  log().scrollTop = 0;
  ui.tools.clear();
  ui.task = null;
  ui.progress = null;
  ui.answer = null;
  ui.answerText = '';
  ui.reasoning = null;
  $('assistant-usage').textContent = '';
}

/** 把历史记录画回面板：文字照常排版，工具卡片收起（结果在卡片里）。 */
function renderTranscript(items) {
  clearLog();
  for (const item of items) {
    if (item.kind === 'task') {
      renderTask(item.task);
    } else if (item.kind === 'user') {
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
        toolResult({ id: item.id, name: item.name, ok: item.ok, result: item.result, replay: true });
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
  ui.pendingNewConversation = false;
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
        if (session.id === ui.conversation) await startNewConversation();
        await refreshInputHistory(true);
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

async function startNewConversation() {
  if (ui.running) return;
  clearLog();
  ui.conversation = null;
  ui.pendingNewConversation = true;
  $('assistant-text').value = '';
  resetInputHistory();
  notice('');
  $('assistant-text').focus();
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
    refreshInputHistory().catch(e => status(e?.message || e));
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
    const settingsPanel = $('assistant-settings');
    if (!settingsPanel.hidden) {
      try { ui.settingsDraft = captureSettingsDraft(); }
      catch (error) { status(error?.message || error); return; }
    }
    settingsPanel.hidden = !settingsPanel.hidden;
    if (settingsPanel.hidden) {
      invoke('assistant_settings').then(saved => renderChoices(saved, saved.backend || 'builtin')).catch(e => status(e));
    } else {
      renderChoices(ui.settingsDraft, $('assistant-backend').value).catch(e => status(e));
    }
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
      fillPrompt(chip.dataset.prompt);
    };
  }
  ui.emptyState = log().querySelector('.assistant-empty');
  renderPageChips();
  addEventListener('colm:step', renderPageChips);
  $('assistant-new').onclick = () => {
    $('assistant-history').hidden = true;
    startNewConversation().catch(e => notice(String(e?.message || e)));
  };
  $('assistant-history-btn').onclick = () => showHistory().catch(e => notice(String(e?.message || e)));
  // 回车发送，Shift + 回车换行；输入法选词时的回车（isComposing / keyCode 229）不发送。
  $('assistant-think').addEventListener('change', () => changeThink().catch(e => status(e?.message || e)));
  $('assistant-backend').addEventListener('change', async () => {
    const backend = $('assistant-backend').value;
    try {
      ui.settingsDraft = captureSettingsDraft();
      ui.settingsBackend = backend;
      await renderChoices(ui.settingsDraft, backend);
      refreshBackendStatus().catch(e => status(e));
    } catch (e) { $('assistant-backend').value = ui.settingsBackend; status(e?.message || e); }
  });
  $('assistant-provider').addEventListener('change', () => changeProvider().catch(e => status(e?.message || e)));
  $('assistant-model-refresh').onclick = () => refreshApiModels();
  $('assistant-base').addEventListener('change', () => {
    renderEndpointNote();
    refreshKeyStatus($('assistant-base').value.trim());
  });
  $('assistant-model-name').addEventListener('change', () => {
    ui.settingsDraft = captureSettingsDraft();
    renderChoices(ui.settingsDraft, $('assistant-backend').value).catch(e => status(e?.message || e));
  });
  $('assistant-ext-model').addEventListener('change', () => changeExternalModel().catch(e => status(e?.message || e)));
  $('assistant-ext-refresh').onclick = () => refreshExternalModels();
  $('assistant-text').addEventListener('keydown', event => {
    if (recallInput(event, $('assistant-text'))) return;
    if (event.key !== 'Enter' || event.shiftKey || event.isComposing || event.keyCode === 229) return;
    event.preventDefault();
    send().catch(e => notice(String(e?.message || e)));
  });
  $('assistant-text').addEventListener('input', resetInputHistory);
  listen('assistant://event', event => {
    const parsed = parseEvent(event.payload);
    if (parsed) handle(parsed);
  });
}

wire();
