//! 结果页是汇总点：所有流程（工作流、研究、修改版、服务器）的结果都汇到这里，再从这里分流。
//!
//! - 这个算例的运行：每条标明出处（版本 · 参数 · 位置与引擎），以这个算例为基础的研究也列出来；修改版的源码之后
//!   又改过的标出来（`colm-cli case-runs`）。
//! - 评估摘要：把模型评估的指标写成白话（哪个变量吻合、系统偏高偏低、还是过程对不上）。
//! - 下一步：问题在哪？四个选项（参数不对 / 过程或结构不对 / 想知道稳不稳 / 公平比较），推荐项附理由。
//!
//! 纯函数见 tests/results-hub.mjs。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $ } from './ui.js';
import { language, translateZh } from './i18n.js';
import { go } from './shell.js';
import { versionLabel, engineLabel, modelVersion, provenanceMismatch } from './run-mode.js';
import { modelEngine } from './engine.js';

const t = text => (language() === 'en' ? translateZh(text) : text);

// ---- 纯函数 ----------------------------------------------------------------------------------

const KIND = { tuning: '参数率定', uncertainty: '不确定性分析' };
const STUDY_STATUS = {
  ready: '还没开始', running: '运行中', paused: '已暂停', completed: '已完成',
  completed_with_failures: '完成（有失败）', cancelled: '已停止', needs_review: '需要确认',
};
const fixed = (value, digits = 2) => (Number.isFinite(value) ? Number(value).toFixed(digits) : '—');

function dateText(unix) {
  if (!Number.isFinite(unix)) return '';
  return new Date(unix * 1000).toLocaleDateString(language() === 'en' ? 'en' : 'zh-CN', { month: 'short', day: 'numeric' });
}

/** 这次运行用的参数从哪来。 */
export function paramsLabel(derived) {
  if (!derived?.study) return '本算例的参数';
  const kind = derived.kind === 'tuning' ? '率定参数' : '研究参数';
  return `${kind} · 研究 ${derived.study}${derived.member ? ` 成员 ${derived.member}` : ''}`;
}

/** “这个算例的运行”的一行：版本、参数、位置与引擎，以及标记与来源。 */
export function runRows(summary) {
  const derived = summary?.derived_from;
  return (summary?.runs ?? []).map((run, index) => {
    const version = run.version ?? {};
    const name = version.kind === 'workspace' ? version.name : 'official';
    const where = run.location?.kind === 'server' ? (run.location.host ?? '服务器') : '本机';
    const flags = [];
    if (run.ok === false) flags.push('这次运行失败');
    if (run.source_changed) flags.push('源码之后又改过');
    if (Number.isFinite(run.ranks) && run.ranks > 1) flags.push(`${run.ranks} 个 MPI 进程`);
    return {
      current: index === 0,
      modified: version.kind === 'workspace',
      chips: [versionLabel(name), paramsLabel(derived), `${where} · ${engineLabel(run.engine)}`],
      flags,
      source: [run.location?.kind === 'server' ? '服务器' : '工作流', dateText(run.finished ?? run.started)].filter(Boolean).join(' · '),
    };
  });
}

/** 以这个算例为基础的研究，一行一个。 */
export function studyRows(summary) {
  return (summary?.studies ?? []).map(study => ({
    study: study.study,
    dir: study.dir,
    kind: study.kind,
    chips: [
      versionLabel(study.version ?? 'official'),
      `${KIND[study.kind] ?? '研究'} ${study.study}`,
      study.location ? `服务器 ${study.location}` : '本机',
    ],
    status: STUDY_STATUS[study.status ?? 'ready'] ?? String(study.status),
    best: study.best_member ? `最优 ${study.best_member} · 目标 ${fixed(study.best_objective, 4)}` : '',
  }));
}

/** 一个变量的白话结论。偏差超过观测标准差的两成算“系统偏高/偏低”。 */
export function metricSentence(row) {
  const label = row.label_zh || row.name;
  const nse = fixed(row.nse);
  if (!Number.isFinite(row.nse)) return { level: 'unknown', text: `${label}：平均偏差 ${fixed(row.bias)}${row.units ? ` ${row.units}` : ''}，NSE 无法计算。` };
  const threshold = 0.2 * (Number.isFinite(row.obs_sd) && row.obs_sd > 0 ? row.obs_sd : Math.abs(row.obs_mean) || 1);
  const units = row.units ? ` ${row.units}` : '';
  if (Number.isFinite(row.bias) && row.bias > threshold) return { level: 'bad', text: `${label}：系统偏高（平均偏差 +${fixed(row.bias)}${units}，NSE ${nse}）` };
  if (Number.isFinite(row.bias) && row.bias < -threshold) return { level: 'bad', text: `${label}：系统偏低（平均偏差 ${fixed(row.bias)}${units}，NSE ${nse}）` };
  if (Number.isFinite(row.nse) && row.nse >= 0.5) return { level: 'ok', text: `${label}：与观测基本吻合（NSE ${nse}）` };
  return { level: 'bad', text: `${label}：变化过程对不上（NSE ${nse}），不是整体偏高或偏低` };
}

export const ROUTES = [
  {
    id: 'params', title: '检查参数', sub: '差得远，但过程看起来是对的',
    detail: '在参数范围内找最贴近观测的一组；多个站点率定出的参数差别大时，可以让网络按地点特征给参数。产出：带新参数的算例，回到运行方式里重跑。',
    actions: [['参数率定', 'result-tuning'], ['AI 学习参数', 'hybrid-learn']],
  },
  {
    id: 'process', title: '检查过程', sub: '率定之后偏差还在，或某个过程系统性地偏',
    detail: '改代码产出一个新的修改版（只改 Rust 引擎、只改 Fortran，或两边都改）；也可以先用网络替换一个过程量试试。改完在运行方式里选它重跑。',
    actions: [['新建修改版', 'code'], ['AI 替换过程', 'hybrid-process']],
  },
  {
    id: 'robust', title: '检查稳健性', sub: '结果对哪些参数敏感、误差有多大',
    detail: '在参数范围内抽样，看输出怎么变。产出：敏感参数排序与输出区间；敏感参数可以带进参数率定。',
    actions: [['不确定性分析', 'result-uncertainty']],
  },
  {
    id: 'compare', title: '公平比较', sub: '改了代码，新版本真的更好吗',
    detail: '两个版本在同样的率定预算下各自率定，再在验证期上比较最优结果，排除“只是参数没调”的影响。',
    actions: [['比较两个版本', 'result-tuning']],
  },
];

/**
 * 推荐哪一条、为什么。`rows` 是当前算例的评估指标，`summary` 是 `case-runs`。
 * 修改版跑的结果先推荐公平比较；指标差而已经率定过（这次用的就是率定参数，或有完成的率定研究）推荐查过程；
 * 指标差而没率定过推荐调参数；指标尚可推荐看稳不稳。没有指标时不推荐。
 */
export function recommendRoute(rows, summary) {
  const latest = summary?.runs?.[0];
  if (latest?.version?.kind === 'workspace') {
    return { route: 'compare', reason: `这批结果来自修改版 ${latest.version.name}；要下“新版本更好”的结论，先在同样的率定预算下和官方版公平比较。` };
  }
  const scored = (rows ?? []).filter(row => Number.isFinite(row?.nse));
  if (!scored.length) return null;
  const worst = scored.reduce((a, b) => (b.nse < a.nse ? b : a));
  const label = worst.label_zh || worst.name;
  if (worst.nse >= 0.5) return { route: 'robust', reason: `指标尚可（最差的是 ${label}，NSE ${fixed(worst.nse)}）；看看结果对参数稳不稳。` };
  const calibrated = summary?.derived_from?.kind === 'tuning'
    || (summary?.studies ?? []).some(s => s.kind === 'tuning' && /^completed/.test(String(s.status ?? '')));
  if (calibrated) {
    return { route: 'process', reason: `${label} 的 NSE 在率定之后仍只有 ${fixed(worst.nse)}，建议设计过程对照实验；指标本身不能确认原因。` };
  }
  return { route: 'params', reason: `${label} 的 NSE 只有 ${fixed(worst.nse)}，还没率定过；建议检查参数范围与敏感性，原因仍需验证。` };
}

// ---- 页面 ------------------------------------------------------------------------------------

let picked = null;
let lastSummary = null;
let summarySeq = 0;

function el(tag, className = '', text = '', children = []) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text) node.textContent = t(text);
  node.append(...children);
  return node;
}

function chip(text, extra = '') {
  return el('span', `hub-chip${extra ? ` ${extra}` : ''}`, text);
}

export function currentRows() {
  const dir = state.resultCaseDir;
  if (!dir) return [];
  if (state.step === 'result-overview') {
    const row = state.resultOverviewMetric;
    return row?.case_dir === dir ? [row] : [];
  }
  return (state.resultMetrics ?? []).filter(row => row.case_dir === dir);
}

function renderRuns(summary) {
  const box = $('hub-runs');
  if (!box) return;
  const runs = runRows(summary);
  const provenance = $('hub-provenance');
  if (provenance) provenance.replaceChildren(...(runs[0] ? [
    ...runs[0].chips.map((text, i) => chip(text, i === 0 ? 'hub-modified' : '')),
    ...runs[0].flags.map(text => chip(text, 'hub-flag')),
  ] : [el('span', 'muted mini', '暂无运行出处记录')]));
  const studies = studyRows(summary);
  if (!runs.length && !studies.length) {
    box.replaceChildren(el('p', 'muted mini', '这个算例还没有运行记录。'));
    return;
  }
  const mismatch = provenanceMismatch(summary?.runs?.[0], { version: modelVersion(), engine: modelEngine() });
  box.replaceChildren(
    ...(mismatch ? [el('p', 'warn mini', `${mismatch}；要对比请按当前运行方式重跑。`)] : []),
    ...runs.map(row => el('div', `hub-run${row.current ? ' hub-current' : ''}`, '', [
      el('div', 'hub-chips', '', [
        ...row.chips.map((text, i) => chip(text, i === 0 && row.modified ? 'hub-modified' : '')),
        ...row.flags.map(text => chip(text, 'hub-flag')),
        ...(row.current ? [chip('当前结果', 'hub-now')] : []),
      ]),
      el('span', 'muted mini', row.source),
    ])),
    ...studies.map(row => {
      const line = el('div', 'hub-run hub-study', '', [
        el('div', 'hub-chips', '', [...row.chips.map(text => chip(text)), chip(row.status, 'hub-status')]),
        el('span', 'muted mini', row.best),
      ]);
      line.title = row.dir;
      return line;
    }),
  );
}

function renderSummary() {
  const box = $('hub-summary');
  if (!box) return;
  const rows = currentRows().filter(row => Number.isFinite(row.nse) || Number.isFinite(row.bias));
  if (!rows.length) {
    const button = el('button', 'btn-ghost', '去模型评估');
    button.type = 'button';
    button.onclick = () => go('result-evaluation');
    box.replaceChildren(el('p', 'muted mini', '还没有和观测比过。先做模型评估，这里会写出哪个变量吻合、哪里偏。'), button);
    return;
  }
  box.replaceChildren(el('ul', 'hub-bullets', '', rows.map(row => {
    const sentence = metricSentence(row);
    return el('li', `hub-${sentence.level}`, sentence.text);
  })));
}

/** 四个选项与选中项的说明。`container` 可以是结果总览，也可以是模型评估页的“下一步”。 */
export function renderRoutes(container, summary = lastSummary) {
  if (!container) return;
  const recommended = recommendRoute(currentRows(), summary);
  const active = picked ?? recommended?.route ?? 'params';
  const grid = el('div', 'hub-routes', '', ROUTES.map(route => {
    const button = el('button', `hub-route${route.id === active ? ' on' : ''}`, '', [
      el('span', 'hub-route-title', '', [
        el('span', '', route.title),
        ...(recommended?.route === route.id ? [el('span', 'research-badge', '推荐')] : []),
      ]),
      el('span', 'muted mini', route.sub),
    ]);
    button.type = 'button';
    button.setAttribute('aria-pressed', String(route.id === active));
    button.onclick = () => { picked = route.id; renderAll(); };
    return button;
  }));
  const route = ROUTES.find(r => r.id === active);
  const detail = el('div', 'hub-detail', '', [
    el('details', '', '', [el('summary', '', '查看方法说明与依据'),
      ...(recommended?.route === active ? [el('p', 'hub-reason', '', [el('span', '', '推荐理由：'), el('span', '', recommended.reason)])] : []),
      el('p', 'mini', route.detail),
    ]),
    el('div', 'pill-row', '', route.actions.map(([label, step], i) => {
      const button = el('button', i === 0 ? 'run-btn' : 'btn-ghost', `${label} →`);
      button.type = 'button';
      button.onclick = () => go(step);
      return button;
    })),
  ]);
  container.replaceChildren(grid, detail);
}

function renderAll() {
  renderRuns(lastSummary);
  renderSummary();
  renderRoutes($('hub-routes'));
  const next = $('result-next-list');
  if (next) {
    renderRoutes(next);
    $('result-next').hidden = false;
  }
}

/** 读这个算例的运行与研究，然后重画。 */
export async function refreshHub() {
  const seq = ++summarySeq;
  const dir = state.resultCaseDir;
  lastSummary = null;
  renderAll();
  if (!dir || !hasBackend) { lastSummary = null; renderAll(); return; }
  try {
    const summary = await invoke('case_runs', { case: dir });
    if (seq !== summarySeq || state.resultCaseDir !== dir) return;
    lastSummary = summary;
    // 助手读出处用最近一次运行。
    state.provenance = { ...(state.provenance ?? {}), [dir]: summary?.runs?.[0] ?? null };
  } catch {
    if (seq !== summarySeq || state.resultCaseDir !== dir) return;
    lastSummary = null;
  }
  renderAll();
}

function wire() {
  if (!$('hub-runs')) return;
  addEventListener('colm:step', () => {
    if (state.step === 'result-overview' || state.step === 'result-evaluation') refreshHub();
  });
  addEventListener('colm:result-case', () => { picked = null; refreshHub(); });
  addEventListener('colm:metrics', renderAll);
  addEventListener('colm:language', renderAll);
  for (const [id, step] of [['hub-open-evaluation', 'result-evaluation'], ['hub-open-diagnostics', 'result-diagnostics']]) {
    if ($(id)) $(id).onclick = () => go(step);
  }
  if ($('hub-open-records')) $('hub-open-records').onclick = () => {
    const records = $('hub-records');
    records.open = true;
    records.scrollIntoView({ block: 'nearest', behavior: 'auto' });
  };
}

wire();
