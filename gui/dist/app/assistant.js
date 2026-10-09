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

/** 随消息附上的“当前页面”说明：页面与具体步骤、选中的算例、内核、项目目录，以及这一页的关键信息（`details`）。 */
export function viewContext({ step, flow, caseDir, kernel, root, details = [] }) {
  return [
    step && `page: ${step}`,
    flow && flow !== step && `workflow step: ${flow}`,
    caseDir && `selected case: ${caseDir}`,
    kernel && `kernel: ${kernel}`,
    root && `project directory: ${root}`,
    ...details,
  ].filter(Boolean).join('\n');
}

const fixed = (value, digits) => (Number.isFinite(value) ? Number(value).toFixed(digits) : 'n/a');

/**
 * 这一页的关键信息（英文，给模型看，不显示）：评估页附上已算出的指标，研究页附上 Study 状态与 AI 模式。
 * `metrics` 是 `state.resultMetrics` 里属于选中算例的行；`badges` 是 `state.studyBadges`。
 */
export function pageDetails(flow, { metrics = [], badges = {}, batch = 0 } = {}) {
  const lines = [];
  if (batch > 1) lines.push(`cases in this batch: ${batch}`);
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
      P('为什么失败', '上一次运行有没有失败或异常？请看运行状态与日志，找出原因。'),
    ],
    'result-overview': [P('总结这次结果', '总结当前算例的结果：模拟时段、主要输出，以及有没有明显异常。')],
    'result-series': [P('偏差在哪个季节', '时间序列上，模型与观测的偏差主要出现在哪个季节或时段？')],
    'result-evaluation': [
      P('解释这些指标', '解释当前算例的评估指标：哪些变量模拟得好、哪些差，可能的原因是什么？'),
      P('偏差在哪个季节', '偏差主要出现在哪个季节或时段？请结合时间序列与指标说明。'),
    ],
    'result-comparison': [P('哪个站点最差', '多站点比较里哪个站点模拟得最差？它和其他站点有什么不同？')],
    'result-diagnostics': [P('检查闭合', '检查水量与能量闭合诊断（f_xerr、f_zerr）有没有异常，异常出在哪个时段。')],
    research: [P('我该做哪种研究', '根据当前算例的评估结果，我应该先做参数率定、不确定性分析，还是 AI 混合建模？为什么？')],
    'result-tuning': [
      P('该率定哪些参数', '针对当前算例的主要偏差，参数率定应该选哪些参数、范围怎么定？'),
      P('解读率定结果', '解读当前参数率定的结果：最优参数有没有压在范围边界？验证期有没有变好？'),
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
  running: false,
  answer: null,
  answerText: '',
  reasoning: null,
  tools: new Map(),
  /** 面板上显示的这段对话的会话号；新对话在第一条消息前为 null。 */
  conversation: null,
  /** 打开面板时是否已经接上过最近一次对话。 */
  restored: false,
  /** 本机 Codex 的模型清单（`model/list`），第一次用到时取。 */
  codexModels: null,
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
  $('assistant-approval').value = settings.approval || 'ask';
  $('assistant-web').value = settings.web_search === false ? 'off' : 'on';
  $('assistant-backend').value = settings.backend || 'builtin';
  await renderChoices(settings, settings.backend || 'builtin');
  refreshBackendStatus().catch(e => status(e));
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

/** 外部后端的模型选项：[值, 显示名]；第一项是“默认”。 */
export function modelOptions(backend, codexModels) {
  if (backend === 'claude_code') {
    return [['', '默认'], ['fable', 'Fable'], ['opus', 'Opus'], ['sonnet', 'Sonnet'], ['haiku', 'Haiku']];
  }
  const models = codexModels || [];
  const fallback = models.find(m => m.default);
  return [
    ['', fallback ? `默认（${fallback.name || fallback.id}）` : '默认'],
    ...models.map(m => [m.id, m.name || m.id]),
  ];
}

/** 输入框下“思考”选框的选项：内置后端是 DeepSeek 的强度；Claude Code 是 --effort 的五档；
 *  Codex 随所选模型（没选用默认模型）而定，取不到清单时给常见的四档。 */
export function thinkOptions(backend, model, codexModels) {
  if (backend === 'builtin') {
    return [['', '思考：默认'], ['low', '思考：low'], ['high', '思考：high'], ['max', '思考：max'], ['off', '不思考']];
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

/** 按后端填设置里的模型选框与输入框下的思考选框。 */
async function renderChoices(settings, backend) {
  const models = backend === 'codex' ? await codexModels() : null;
  const model = settings.external?.[backend]?.model || '';
  if (backend !== 'builtin') fillSelect($('assistant-ext-model'), modelOptions(backend, models), model);
  fillSelect($('assistant-think'), thinkOptions(backend, model, models), currentThink(settings, backend));
  $('assistant-think').title = t(backend === 'builtin'
    ? '思考强度：随时可改，下一条消息生效'
    : '思考强度：随时可改，下一条消息生效（Codex / Claude Code）');
}

async function changeThink() {
  const saved = await invoke('assistant_settings');
  const backend = saved.backend || 'builtin';
  const value = $('assistant-think').value;
  const settings = backend === 'builtin'
    ? { ...saved, ...thinkSettings(value) }
    : withChoice(saved, backend, { effort: value });
  await invoke('assistant_save_settings', { settings });
  ui.started = false; // 下一条消息发出前重新配置
}

/** 设置里换了模型：Codex 的强度选项随模型变。 */
async function changeExternalModel() {
  const backend = $('assistant-backend').value;
  const models = backend === 'codex' ? await codexModels() : null;
  const model = $('assistant-ext-model').value;
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
  if (!info.logged_in) return t(external.loggedOut);
  return null;
}

/** 设置里“后端”下面的一行状态；返回那个后端的状态（内置后端返回 null）。 */
async function refreshBackendStatus(backend = $('assistant-backend').value) {
  const line = $('assistant-backend-status');
  // 服务地址、模型、API Key 只属于内置后端；Codex / Claude Code 用各自的登录，不显示这些。
  for (const el of document.querySelectorAll('#assistant-settings [data-backend-only]')) {
    el.hidden = !el.dataset.backendOnly.split(' ').includes(backend);
  }
  if (backend === 'builtin') {
    line.textContent = t('使用下面的服务地址、模型与 API Key。');
    line.className = 'mini muted';
    return null;
  }
  line.textContent = t('正在检查…');
  const status = await invoke('assistant_backend_status').catch(e => ({ error: String(e?.message || e) }));
  const info = status?.[backend];
  const problem = status?.error ?? backendProblem(backend, info);
  if (problem) {
    line.textContent = problem;
    line.className = 'mini assistant-fail';
  } else {
    const plan = info.subscription ? `${info.subscription} ${t('订阅')}` : (info.auth_method || '');
    line.textContent = `✓ ${t('已登录')}${plan ? ` · ${plan}` : ''} · ${info.version || ''}`;
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
  const backend = $('assistant-backend').value || 'builtin';
  const think = $('assistant-think').value;
  const base = {
    ...previous,
    base_url: $('assistant-base').value.trim(),
    model: $('assistant-model-name').value.trim(),
    approval: $('assistant-approval').value || 'ask',
    web_search: $('assistant-web').value !== 'off',
    backend,
    egress_acknowledged: previous?.egress_acknowledged ?? null,
  };
  // 思考选框显示的是当前后端的强度：内置后端存进 DeepSeek 的设置，外部后端存进它自己的那一份。
  return backend === 'builtin'
    ? { ...base, ...thinkSettings(think) }
    : withChoice(base, backend, { model: $('assistant-ext-model').value, effort: think });
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
  const caseDir = state.selected?.dir;
  return {
    step,
    flow: state.step,
    caseDir,
    kernel: $('kernel')?.value,
    root: $('root')?.value?.trim(),
    details: pageDetails(state.step, {
      metrics: (state.resultMetrics ?? []).filter(row => !caseDir || row.case_dir === caseDir),
      badges: state.studyBadges ?? {},
      batch: state.batch?.length ?? 0,
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
      $('assistant-text').value = language() === 'en' ? translateZh(prompt) : prompt;
      $('assistant-text').focus();
    };
    return chip;
  }));
}

/** 项目目录：开着的项目根，否则选中算例的上一级。 */
function projectRoot(view) {
  if (view.root) return view.root;
  const dir = view.caseDir || '';
  return dir.includes('/') ? dir.slice(0, dir.lastIndexOf('/')) : dir;
}

/** 第一次向某个服务发送前，在面板里问一次数据外发（不用 window.confirm：桌面窗口里弹不出来）。 */
/** 外部后端的说明：数据发给谁、用量算在哪。 */
const EXTERNAL = {
  codex: {
    consent: '发送后，你的问题、算例配置、指标与日志片段会经你本机的 Codex 发给 OpenAI，用量计入你的 ChatGPT 订阅。',
    missing: '本机没有找到 Codex：请先安装它，然后在终端运行 codex login 登录。',
    loggedOut: 'Codex 还没有登录：请在终端运行 codex login 登录。',
  },
  claude_code: {
    consent: '发送后，你的问题、算例配置、指标与日志片段会经你本机的 Claude Code 发给 Anthropic，用量计入你的 Claude 订阅。',
    missing: '本机没有找到 Claude Code：请先安装它，然后在终端运行 claude 并登录。',
    loggedOut: 'Claude Code 还没有登录：请在终端运行 claude 并登录。',
  },
};

function askConsent(target) {
  return new Promise(resolve => {
    const card = element('div', 'assistant-consent');
    const external = EXTERNAL[target];
    card.append(
      element('div', 'assistant-approval-title', '发送前请确认'),
      ...(external
        ? [element('p', 'mini', external.consent)]
        : [
          element('p', 'mini', '发送后，你的问题、算例配置、指标与日志片段会发给这个模型服务：'),
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
  }
  // 外发确认按“发给谁”记：API 服务按地址，外部后端按后端名。
  const target = backend === 'builtin' ? settings.base_url : backend;
  if (settings.egress_acknowledged !== target) {
    if (!(await askConsent(target))) throw new Error(t('已取消发送'));
    settings = { ...settings, egress_acknowledged: target };
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
  renderPageChips();
  addEventListener('colm:step', renderPageChips);
  $('assistant-new').onclick = () => { $('assistant-history').hidden = true; startNewConversation(); };
  $('assistant-history-btn').onclick = () => showHistory().catch(e => notice(String(e?.message || e)));
  // 回车发送，Shift + 回车换行；输入法选词时的回车（isComposing / keyCode 229）不发送。
  $('assistant-think').addEventListener('change', () => changeThink().catch(e => status(e?.message || e)));
  $('assistant-backend').addEventListener('change', async () => {
    const backend = $('assistant-backend').value;
    await renderChoices(await invoke('assistant_settings'), backend).catch(e => status(e?.message || e));
    refreshBackendStatus().catch(e => status(e));
  });
  $('assistant-ext-model').addEventListener('change', () => changeExternalModel().catch(e => status(e?.message || e)));
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
