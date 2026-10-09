//! 研究：总览页（你想回答什么问题、基于哪个算例、准备情况）与结果页评估之后的“下一步”。
//!
//! 研究都从一个算例出发、产出新的算例：参数率定写出率定后的算例，AI 混合建模把网络装进算例；
//! 新算例回到工作流里运行，在结果页与原算例比较。这里只负责“去哪一条研究、条件够不够”，
//! 研究本身的向导仍在 results.js（参数率定、不确定性分析）与 hybrid.js（AI 混合建模）。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';
import { batchTarget, sourceSite } from './batch.js';
import { modelEngine } from './engine.js';
import { language, translateZh } from './i18n.js';
import { processSlotBlocked } from './hybrid.js';

const t = text => (language() === 'en' ? translateZh(text) : text);

// ---- 纯函数（tests/research.mjs）----------------------------------------------------------

/** 四条研究：两组（经典方法、AI 混合建模），每条一个要回答的问题。 */
export const STUDIES = [
  { id: 'tuning', step: 'result-tuning', group: 'classic', title: '参数率定', question: '让模拟更贴近观测',
    note: '在参数范围内搜索一组最优值（全区一个，或每个地类一个），产出率定后的新算例。' },
  { id: 'uq', step: 'result-uncertainty', group: 'classic', title: '不确定性分析', question: '结果对哪些参数敏感、误差有多大',
    note: '在参数范围内抽样，统计输出怎么变；敏感参数可以带进参数率定。' },
  { id: 'learn', step: 'hybrid-learn', group: 'ai', title: 'AI 学习参数', question: '参数能不能随地点特征变化',
    note: '参数率定的进阶版：网络按气候、土壤等特征给出每个 patch 的参数。' },
  { id: 'process', step: 'hybrid-process', group: 'ai', title: 'AI 替换过程', question: '某个物理公式能不能用网络代替',
    note: '网络每步替换一个过程量（目前是土壤水分胁迫 β）。' },
];

/** NSE 低于它就推荐参数率定。 */
export const NSE_THRESHOLD = 0.5;

/**
 * 按评估指标推荐一条研究：最差的 NSE 低于门槛时推荐参数率定（并给出是哪个变量），
 * 否则推荐不确定性分析（指标尚可，看稳不稳）。没有可用的 NSE 时返回 `null`。
 */
export function recommend(rows, threshold = NSE_THRESHOLD) {
  const scored = (rows ?? []).filter(row => Number.isFinite(row?.nse));
  if (!scored.length) return null;
  const worst = scored.reduce((a, b) => (b.nse < a.nse ? b : a));
  return { study: worst.nse < threshold ? 'tuning' : 'uq', row: worst };
}

/**
 * 一条研究的准备情况：每项 `{ level: 'ok' | 'warn' | 'missing', text, step? }`，`step` 是去补的那一步。
 * `ctx`：`caseName`（没有算例为空）、`spatial`、`missingObs`（没有观测的站点数）、`engine`、`processBlock`（β 插槽不能用的原因）。
 */
export function readiness(study, ctx) {
  const items = [];
  const hasCase = !!ctx.caseName;
  const caseItem = level => (hasCase
    ? { level: 'ok', text: `基于算例 ${ctx.caseName}` }
    : { level, text: '还没有算例', step: 'basic-files' });
  if (study === 'tuning' || study === 'uq') {
    items.push(caseItem('missing'));
    if (ctx.spatial) items.push({ level: 'missing', text: '空间算例暂不支持参数率定和不确定性分析。' });
    if (hasCase && !ctx.spatial) {
      if (!ctx.missingObs) items.push({ level: 'ok', text: '每个站点都有观测数据' });
      else if (study === 'tuning') items.push({ level: 'missing', text: `${ctx.missingObs} 个站点没有观测数据，参数率定要按观测打分`, step: 'basic-site' });
      else items.push({ level: 'warn', text: `${ctx.missingObs} 个站点没有观测数据；不确定性分析仍可做，但不能检查真实配对数`, step: 'basic-site' });
    }
    items.push({ level: 'ok', text: '会自己运行基准成员，不要求先跑完算例' });
  } else {
    items.push(study === 'learn'
      ? (hasCase ? caseItem('ok') : { level: 'warn', text: '还没有算例；两步法只需要项目目录，差分进化与导入要先建算例', step: 'basic-files' })
      : caseItem('missing'));
    if (study === 'process' && hasCase) {
      items.push(ctx.processBlock
        ? { level: 'missing', text: ctx.processBlock, step: 'params-water' }
        : { level: 'ok', text: '土壤水分胁迫 β 插槽可用' });
    }
    items.push(ctx.engine === 'fortran'
      ? { level: 'warn', text: '当前是 Fortran 引擎；装了网络的算例要换成 Rust 引擎运行', step: 'run' }
      : { level: 'ok', text: '当前是 Rust 引擎，能运行装了网络的算例' });
  }
  return items;
}

/** 准备情况里有没有挡路的（`missing`）。 */
export function blocked(items) {
  return items.some(item => item.level === 'missing');
}

// ---- 页面 ----------------------------------------------------------------------------

function element(tag, cls = '', text = '') {
  const el = document.createElement(tag);
  if (cls) el.className = cls;
  if (text !== '') el.textContent = String(text);
  return el;
}

let picked = null;
let processBlock = '';

function selectedRows() {
  const dir = state.selected?.dir;
  return (state.resultMetrics ?? []).filter(row => !dir || row.case_dir === dir);
}

function rowLabel(row) {
  return (language() === 'en' ? row.label_en : row.label_zh) || row.name;
}

function nseValue(row) {
  return Number(row.nse).toFixed(2);
}

function context() {
  const cases = batchTarget();
  return {
    caseName: state.selected?.name ?? cases[0]?.name ?? '',
    spatial: !!(state.domain && state.domain !== 'site') || cases.some(c => c?.spatial === true),
    missingObs: cases.filter(c => !sourceSite(c)?.obs_file).length,
    engine: modelEngine(),
    processBlock,
  };
}

function studyCard(study, recommended) {
  const card = element('button', 'domain-card research-card');
  card.type = 'button';
  card.setAttribute('aria-selected', String(picked === study.id));
  const head = element('span', 'research-card-head');
  head.append(element('span', 'dt', t(study.title)));
  if (recommended?.study === study.id) {
    const nse = nseValue(recommended.row);
    head.append(element('span', 'research-badge', `${t('推荐')} · NSE ${nse}`));
  }
  card.append(head, element('span', 'dq', t(study.question)), element('span', 'dd', t(study.note)));
  const badge = state.studyBadges?.[study.id];
  if (badge) card.append(element('span', 'research-status', t(badge)));
  card.onclick = () => { picked = study.id; renderOverview(); };
  return card;
}

const LEVEL_ICON = { ok: '✓', warn: '!', missing: '✕' };

function renderReadiness(study) {
  const items = readiness(study.id, context());
  $('research-readiness-title').textContent = `${t(study.title)} · ${t('准备情况')}`;
  const list = $('research-readiness-list');
  list.replaceChildren(...items.map(item => {
    const row = element('div', `readiness-item ${item.level}`);
    row.append(element('span', 'readiness-icon', LEVEL_ICON[item.level]), element('span', '', t(item.text)));
    if (item.step) {
      const fix = element('button', 'link-btn', t('去处理 →'));
      fix.type = 'button';
      fix.onclick = () => go(item.step);
      row.append(fix);
    }
    return row;
  }));
  const start = element('button', 'run-btn', `${t('开始')}：${t(study.title)} →`);
  start.type = 'button';
  start.disabled = blocked(items);
  start.onclick = () => go(study.step);
  const back = element('button', 'btn-ghost', t('先看评估结果'));
  back.type = 'button';
  back.onclick = () => go('result-evaluation');
  $('research-readiness-actions').replaceChildren(start, back);
}

export function renderOverview() {
  if (!$('research-classic')) return;
  const ctx = context();
  const recommended = recommend(selectedRows());
  if (!picked) picked = recommended?.study ?? 'tuning';
  const count = batchTarget().length;
  const scope = count > 1 ? t(`本次共 ${count} 个算例`) : '';
  $('research-case').textContent = ctx.caseName
    ? (scope ? `${ctx.caseName} · ${scope}` : ctx.caseName)
    : t('还没有算例：先在工作流里建一个并运行，研究会以它为基础。');
  $('research-classic').replaceChildren(...STUDIES.filter(s => s.group === 'classic').map(s => studyCard(s, recommended)));
  $('research-ai').replaceChildren(...STUDIES.filter(s => s.group === 'ai').map(s => studyCard(s, recommended)));
  renderReadiness(STUDIES.find(s => s.id === picked));
}

/** 结果页评估之后的“下一步”：推荐的那条放最前面并标出原因，其余三条作为可选的去向。 */
export function renderNextSteps() {
  const card = $('result-next');
  if (!card) return;
  const recommended = recommend(selectedRows());
  if (!recommended) { card.hidden = true; return; }
  card.hidden = false;
  const reason = {
    tuning: () => `${rowLabel(recommended.row)} 的 NSE 为 ${nseValue(recommended.row)}，偏低`,
    uq: () => `最差的 NSE 为 ${nseValue(recommended.row)}，指标尚可；看看结果稳不稳`,
  };
  const others = {
    tuning: '想让模拟更贴近观测',
    uq: '想知道结果对哪些参数敏感',
    learn: '多个站点率定出的参数差别大',
    process: '某个过程有系统偏差（如干旱期蒸散偏低），像是公式问题',
  };
  const line = (study, text, top) => {
    const row = element('div', 'next-step' + (top ? ' recommended' : ''));
    if (top) row.append(element('span', 'research-badge', t('推荐')));
    row.append(element('span', '', `${t(text)} →`));
    const link = element('button', 'link-btn', t(STUDIES.find(s => s.id === study).title));
    link.type = 'button';
    link.onclick = () => go(STUDIES.find(s => s.id === study).step);
    row.append(link);
    return row;
  };
  const rows = [line(recommended.study, reason[recommended.study](), true)];
  for (const study of ['tuning', 'uq', 'process', 'learn']) {
    if (study !== recommended.study) rows.push(line(study, others[study], false));
  }
  $('result-next-list').replaceChildren(...rows);
}

async function refreshProcessBlock() {
  try {
    processBlock = await processSlotBlocked();
  } catch {
    processBlock = '';
  }
  if (state.step === 'research') renderOverview();
}

addEventListener('colm:step', () => {
  if (state.step === 'research') {
    renderOverview();
    refreshProcessBlock();
  }
  if (state.step === 'result-evaluation') renderNextSteps();
});
addEventListener('colm:metrics', () => {
  renderNextSteps();
  if (state.step === 'research') renderOverview();
});
