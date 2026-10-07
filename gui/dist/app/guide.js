//! 引导模式（docs/design-ai-assistant.md 第 8.1 节）：AI 助手像用户一样操作窗口。
//!
//! assistant.js 收到 agent 的 `ui_request` 后派发 `colm:ui-request`，这里执行并用 `respond(ok, result)`
//! 回话。走的都是用户操作的路径（改值后派发 input/change、按按钮、`go()`），所以校验、默认值与
//! 落盘和手动操作完全一样。按后果把关，越级的请求在这里拒绝：
//! - `fill` 只填草稿字段（按按钮才生效），`click` 只按不落盘的按钮；
//! - `set` 只改“改了就存”的参数表字段，`commit` 只按会落盘或开跑的按钮（这两个在 agent 那边要审批）。
//! 本模块可以导入 shell/sites（assistant.js 不行，会成环），由 main.js 加载。

import { state } from './state.js';
import { $ } from './ui.js';
import { STEPS, go } from './shell.js';
import { scanPreparedSites, pickSite } from './sites.js';

const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
const text = el => (el?.textContent ?? '').replace(/\s+/g, ' ').trim();
const shown = el => !!el && !el.closest('[hidden]') && el.getClientRects().length > 0;

/** 按按钮才生效的草稿字段（各页的元素 id）。 */
const DRAFT_FIELDS = {
  'basic-files': [
    'sitedir', 'forcingdir', 'rawdata', 'runtime', 'root',
    'spatial-rawdata', 'spatial-runtime', 'spatial-forcing-dataset', 'spatial-forcing-dir',
    'spatial-forcing', 'spatial-start', 'spatial-end', 'spatial-timestep', 'spatial-root', 'spatial-name',
  ],
  'basic-timing': ['tm-years', 'tm-repeat'],
  run: ['model-engine', 'cpu-workers', 'mpi-ranks', 'case-threads', 'force'],
};
/** 配置向导里的输入（空间范围、同位素、溶质）。 */
const GATE_FIELDS = [
  'spatial-west', 'spatial-east', 'spatial-south', 'spatial-north', 'spatial-dlon', 'spatial-dlat',
  'spatial-shapefile', 'spatial-meshFile', 'spatial-catchmentFile',
  'isotope-mixing', 'solute-init', 'solute-precip',
];

/** 不落盘的按钮：名字 → 元素与说明。 */
const CLICKS = {
  'basic-files': { scan: () => $('scan') },
};
/** 会落盘或开跑的按钮。 */
const COMMITS = {
  'basic-files': {
    'create-case': () => document.querySelector('#makecase .btn-next'),
    'make-spatial-case': () => $('make-spatial-case'),
    'use-example': () => $('use-example'),
  },
  'basic-timing': { 'apply-spinup': () => $('tm-apply') },
  run: {
    'run-all': () => $('runall'),
    'run-mksrfdata': () => $('run-mksrfdata'),
    'run-mkinidata': () => $('run-mkinidata'),
    'run-colm': () => $('run-colm'),
    'cancel-run': () => $('cancel-run'),
  },
};

function screen() {
  if (shown($('loadinggate'))) return 'loading';
  if (!$('launchgate')?.hidden) return 'launch';
  if (!$('domaingate')?.hidden) return 'setup-wizard';
  return 'workbench';
}

function labelFor(el) {
  const label = el.id ? document.querySelector(`label[for="${el.id}"]`) : null;
  return text(label) || el.getAttribute('aria-label') || el.placeholder || el.id;
}

function describeControl(name, el, saves) {
  const field = { name, label: '', value: '', saves };
  if (el.tagName === 'SELECT') {
    field.kind = 'choice';
    field.value = el.value;
    field.options = [...el.options].map(o => (o.value === text(o) ? o.value : `${o.value} (${text(o)})`));
  } else if (el.type === 'checkbox') {
    field.kind = 'switch';
    field.value = String(el.checked);
  } else {
    field.kind = el.type === 'number' ? 'number' : 'text';
    field.value = el.value;
  }
  if (el.disabled) field.disabled = true;
  if (el.readOnly) field.read_only = true;
  return field;
}

function draftFields(names) {
  return names
    .map(id => $(id))
    .filter(shown)
    .map(el => ({ ...describeControl(el.id, el, false), label: labelFor(el) }));
}

/** 当前页参数表里“改了就存”的字段：`tr[data-parameter-key]`。 */
function tableRows() {
  return [...document.querySelectorAll('.page:not([hidden]) tr[data-parameter-key]')].filter(shown);
}

function tableFields() {
  return tableRows().map(row => {
    const control = row.querySelector('select, input');
    const label = row.querySelector('td');
    const field = control
      ? describeControl(row.dataset.parameterKey, control, true)
      : { name: row.dataset.parameterKey, value: text(row.querySelectorAll('td')[1]), saves: true, read_only: true };
    field.label = text(label);
    const reason = label?.title?.trim();
    if (reason) field.note = reason;
    return field;
  });
}

function buttons(table, kind) {
  return Object.entries(table[state.step] ?? {})
    .map(([name, find]) => [name, find()])
    .filter(([, el]) => shown(el))
    .map(([name, el]) => ({ name, label: text(el), kind, enabled: !el.disabled }));
}

function siteList() {
  if (state.step !== 'basic-files' || !shown($('sites'))) return undefined;
  return (state.sites ?? []).map(s => ({
    name: `site:${s.name}`,
    label: s.name,
    highlighted: state.pickedSite === s,
    checked: state.picked.has(s.site_file),
  }));
}

function steps() {
  return STEPS.filter(s => !s.show || s.show()).map(s => {
    const blocked = s.need();
    return { name: s.id, title: s.t, ...(blocked ? { blocked } : {}), ...(s.id === state.step ? { current: true } : {}) };
  });
}

function wizardState() {
  const gate = $('domaingate');
  const choices = [...gate.querySelectorAll('#gatecards [data-choice]')].map(card => {
    const choice = { name: `choice:${card.dataset.choice}`, label: text(card.querySelector('.dt')) };
    if (card.getAttribute('aria-selected') === 'true' || card.getAttribute('aria-pressed') === 'true') choice.selected = true;
    const why = text(card.querySelector('.dwhy'));
    if (why) choice.blocked = why;
    return choice;
  });
  const controls = [...gate.querySelectorAll('[data-gate]')].map(b => ({
    name: `wizard-${b.dataset.gate}`,
    label: text(b),
    kind: 'click',
    enabled: !b.disabled,
  }));
  return {
    page: gate.dataset.page,
    title: text($('gatetitle')),
    subtitle: text($('gatesub')),
    info: text($('gateinfo')) || undefined,
    choices,
    fields: draftFields(GATE_FIELDS),
    buttons: controls,
    hint: 'Pick choices with ui_click("choice:<id>"), then ui_click("wizard-next"); "wizard-finish" on the last page opens the workbench.',
  };
}

/** 窗口现在的样子：给模型看的 JSON。 */
export function uiState() {
  const where = screen();
  const status = text($('status')) || undefined;
  if (where === 'loading') return { screen: where, hint: 'The application is still starting; try again shortly.' };
  if (where === 'launch') {
    return {
      screen: where,
      buttons: [
        { name: 'local-run', label: text($('localRunCard')?.querySelector('.dt')), kind: 'click', enabled: true },
        { name: 'server-run', label: text($('serverRunCard')?.querySelector('.dt')), kind: 'click', enabled: !$('serverRunCard')?.disabled },
      ],
    };
  }
  if (where === 'setup-wizard') return { screen: where, wizard: wizardState(), status };
  return {
    screen: where,
    step: state.step,
    case: state.selected?.dir ?? null,
    model: state.wizard ? { domain: state.domain, subgrid: state.subgrid, physics: state.wizard.physics } : null,
    steps: steps(),
    fields: [...draftFields(DRAFT_FIELDS[state.step] ?? []), ...tableFields()],
    sites: siteList(),
    buttons: [...buttons(CLICKS, 'click'), ...buttons(COMMITS, 'commit')],
    status,
  };
}

// ---- 动作 --------------------------------------------------------------------------------

/** 高亮助手动过的控件，用户一眼看到改了哪里。 */
function mark(el) {
  el.classList.add('ai-filled');
  el.addEventListener('focus', () => el.classList.remove('ai-filled'), { once: true });
}

/** 把文字值落到控件上：选项可以写值或显示文字；逻辑量接受 true/false。 */
function assign(el, raw) {
  const value = String(raw ?? '').trim();
  if (el.tagName === 'SELECT') {
    const want = value.toLowerCase();
    const aliases = { true: '.true.', false: '.false.', '.true.': '.true.', '.false.': '.false.' };
    const option = [...el.options].find(o => o.value === value)
      ?? [...el.options].find(o => o.value.toLowerCase() === (aliases[want] ?? want))
      ?? [...el.options].find(o => text(o).toLowerCase() === want);
    if (!option) throw new Error(`${value} is not an option; choose one of: ${[...el.options].map(o => o.value).join(', ')}`);
    el.value = option.value;
  } else if (el.type === 'checkbox') {
    el.checked = /^(true|1|yes|on|是|开)$/i.test(value);
  } else {
    el.value = value;
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  mark(el);
}

function fill(fields) {
  const allowed = new Set(screen() === 'setup-wizard' ? GATE_FIELDS : DRAFT_FIELDS[state.step] ?? []);
  const done = [];
  for (const { field, value } of fields ?? []) {
    const el = allowed.has(field) ? $(field) : null;
    if (!el || !shown(el)) {
      const saves = tableRows().some(row => row.dataset.parameterKey === field);
      throw new Error(saves
        ? `${field} is saved as soon as it changes; use ui_set for it`
        : `${field} is not a draft field on this page; read ui_state for the fields here`);
    }
    if (el.disabled) throw new Error(`${field} is disabled here`);
    assign(el, value);
    done.push({ field, value: el.type === 'checkbox' ? String(el.checked) : el.value });
  }
  return done;
}

/** 改参数表字段：每改一个就等它自动保存、表格重画完，再改下一个。 */
async function set(fields) {
  const done = [];
  for (const { field, value } of fields ?? []) {
    const row = tableRows().find(r => r.dataset.parameterKey === field);
    const control = row?.querySelector('select, input');
    if (!control) throw new Error(`${field} is not an editable parameter on this page; read ui_state`);
    if (control.disabled) throw new Error(`${field} cannot be changed here: ${row.querySelector('td')?.title || 'disabled'}`);
    const before = text($('status'));
    assign(control, value);
    // 自动保存后会重画表格；等状态栏变化或表格换新，最多 15 秒。
    for (let i = 0; i < 75; i += 1) {
      await sleep(200);
      if (!control.isConnected || text($('status')) !== before) break;
    }
    const now = tableRows().find(r => r.dataset.parameterKey === field)?.querySelector('select, input');
    if (now) mark(now);
    done.push({ field, value: now ? now.value : null, status: text($('status')) });
  }
  return done;
}

/** 打开已有算例并停在某一步（默认“运行”）：读算例，不写文件。 */
async function openCase(dir, step = 'run') {
  const before = state.selected?.dir ?? null;
  dispatchEvent(new CustomEvent('colm:open-case-dir', { detail: { dir, step } }));
  for (let i = 0; i < 150; i += 1) {
    await sleep(200);
    if (state.selected?.dir === dir && state.step === step) return;
    if (state.selected?.dir === dir && i > 10) return;
  }
  if ((state.selected?.dir ?? null) === before) throw new Error(text($('status')) || `cannot open ${dir}`);
}

async function click(target) {
  if (target.startsWith('open-case:')) {
    await openCase(target.slice('open-case:'.length).trim());
    return;
  }
  const where = screen();
  if (where === 'launch') {
    if (target === 'local-run') { $('localRunCard').click(); await sleep(100); return; }
    if (target === 'server-run') throw new Error('server runs are not available yet');
  }
  if (where === 'setup-wizard') {
    const gate = $('domaingate');
    const choice = target.startsWith('choice:') ? target.slice(7) : null;
    const el = choice
      ? gate.querySelector(`#gatecards [data-choice="${CSS.escape(choice)}"]`)
      : gate.querySelector(`[data-gate="${CSS.escape(target.replace(/^wizard-/, ''))}"]`);
    if (!el) throw new Error(`${target} is not on this wizard page; read ui_state`);
    if (el.disabled) throw new Error(`${target} is not available: ${text(el.querySelector('.dwhy')) || 'disabled'}`);
    el.click();
    await sleep(100);
    return;
  }
  if (target.startsWith('site:') && state.step === 'basic-files') {
    const site = (state.sites ?? []).find(s => s.name === target.slice(5));
    if (!site) throw new Error(`${target.slice(5)} is not in the scanned site list`);
    pickSite(site);
    return;
  }
  if (target === 'scan' && state.step === 'basic-files') {
    await scanPreparedSites();
    return;
  }
  const el = CLICKS[state.step]?.[target]?.();
  if (el && shown(el)) { el.click(); await sleep(200); return; }
  if (COMMITS[state.step]?.[target]) throw new Error(`${target} saves or runs something; use ui_commit`);
  throw new Error(`${target} is not a button on this page; read ui_state`);
}

async function commit(target) {
  const el = COMMITS[state.step]?.[target]?.();
  if (!el || !shown(el)) {
    throw new Error(CLICKS[state.step]?.[target] || target.startsWith('site:')
      ? `${target} does not save anything; use ui_click`
      : `${target} is not a commit button on this page; read ui_state`);
  }
  if (el.disabled) throw new Error(`${target} is disabled: ${text($('status')) || 'the page is not complete'}`);
  const before = state.selected?.dir ?? null;
  const status = text($('status'));
  el.click();
  if (target === 'create-case' || target === 'make-spatial-case') {
    // 建算例要读站点、写文件：等到选中的算例换成新的，或状态栏报错，最多两分钟。建好就跳到“运行”。
    for (let i = 0; i < 600; i += 1) {
      await sleep(200);
      if ((state.selected?.dir ?? null) !== before) {
        const created = state.selected.dir;
        await sleep(300);
        go('run');
        return { created, step: state.step };
      }
      const now = text($('status'));
      if (now !== status && /失败|错误|error|failed|cannot|不能|缺/i.test(now)) throw new Error(now);
    }
    throw new Error('the case was not created within two minutes; check the status bar');
  }
  await sleep(500);
  return { pressed: target, status: text($('status')) };
}

async function perform(action, args) {
  switch (action) {
    case 'state':
      return uiState();
    case 'go': {
      if (screen() !== 'workbench') throw new Error(`the ${screen()} screen is showing; finish it first`);
      go(args.step);
      if (state.step !== args.step) throw new Error(text($('status')) || `cannot open ${args.step}`);
      await sleep(150);
      return uiState();
    }
    case 'fill':
      return { filled: fill(args.fields), page: uiState() };
    case 'set': {
      if (screen() !== 'workbench') throw new Error('no parameter page is showing');
      return { saved: await set(args.fields), page: uiState() };
    }
    case 'click':
      await click(String(args.target ?? ''));
      return uiState();
    case 'commit': {
      if (screen() !== 'workbench') throw new Error('nothing to commit on this screen');
      const outcome = await commit(String(args.target ?? ''));
      return { ...outcome, page: uiState() };
    }
    default:
      throw new Error(`unknown window action ${action}`);
  }
}

addEventListener('colm:ui-request', event => {
  const { action, args, respond } = event.detail ?? {};
  if (!respond) return;
  event.detail.handled = true;
  perform(action, args ?? {})
    .then(result => respond(true, result))
    .catch(error => respond(false, String(error?.message || error)));
});
