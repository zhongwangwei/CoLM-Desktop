//! 比较两个版本（参数率定页最后一步）：新结构是否真的更好？两个模型版本用同一份率定设定（参数、预算、种子、率定期
//! 与验证期）各自率定，再在验证期上比较最优成员——排除“只是参数没调”的影响。
//!
//! 第二个研究由 `study-clone` 按第一个的设定建；两边各自固定自己的版本（`study-run --model-version`，第一次运行时拍
//! 快照）；比较由 `study-compare` 算。纯函数见 tests/version-compare.mjs。

import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { language, translateZh } from './i18n.js';
import { modelEngine } from './engine.js';
import { OFFICIAL, versionLabel, runModeBlocker } from './run-mode.js';
import { activeTuningStudies, refreshTuningStudy, studyVersionArg } from './results.js';
import { awaitServerStudy, studyTarget, submitStudies } from './remote-study.js';

const t = text => (language() === 'en' ? translateZh(text) : text);
const PAIRS_KEY = 'colm.versionComparePairs';

// ---- 纯函数 ----------------------------------------------------------------------------------

const score = value => (Number.isFinite(value) ? Number(value).toFixed(4) : '—');

/** 比较表的行：版本、状态、最优成员、率定期与验证期目标值、基线。 */
export function compareRows(result) {
  return ['a', 'b'].map(key => {
    const side = result?.[key] ?? {};
    return {
      key,
      version: versionLabel(side.version),
      status: side.status ?? '—',
      best: side.best_member ?? '—',
      calibration: score(side.calibration),
      validation: score(side.validation),
      baseline: score(side.baseline_validation ?? side.baseline_calibration),
    };
  });
}

/** 结论一句话（目标值越小越好）。 */
export function compareVerdict(result) {
  if (!result) return '';
  const period = result.period === 'validation' ? '验证期' : '率定期';
  const a = versionLabel(result.a?.version);
  const b = versionLabel(result.b?.version);
  if (!result.same_design) return '两个研究的设定不一样（参数、预算、种子、目标或时段），这不是公平比较。';
  if (result.better === 'a') return `在${period}上，${a}更好（目标值更低）。`;
  if (result.better === 'b') return `在${period}上，${b}更好（目标值更低）。`;
  if (result.better === 'tie') return `在${period}上，两个版本一样好。`;
  return '还有研究没有得到最优成员，暂时比不出来。';
}

/** 这个研究现在固定的版本：固定过用固定的；用官方版开始跑过就是官方版；都没有返回 null（还能选）。 */
export function fixedVersion(envelope) {
  if (envelope?.model_version?.name) return envelope.model_version.name;
  const started = Object.values(envelope?.state?.tasks || {})
    .some(task => !['pending', 'materialized'].includes(String(task.status).toLowerCase()));
  return started ? OFFICIAL : null;
}

const finished = envelope => ['completed', 'completed_with_failures'].includes(String(envelope?.state?.status ?? '').toLowerCase());

function loadPairs() {
  try { return JSON.parse(localStorage.getItem(PAIRS_KEY) ?? '{}') ?? {}; } catch { return {}; }
}

function savePair(a, b) {
  try { localStorage.setItem(PAIRS_KEY, JSON.stringify({ ...loadPairs(), [a]: b })); } catch { /* 存不了只是下次要重建 */ }
}

// ---- 页面 ------------------------------------------------------------------------------------

let busy = false;

function el(tag, props = {}, children = []) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

async function fillVersions() {
  const a = $('compare-version-a');
  const b = $('compare-version-b');
  if (!a || !b) return;
  let versions = [];
  try { versions = (await invoke('workspace_list'))?.workspaces ?? []; } catch { versions = []; }
  for (const select of [a, b]) {
    const keep = select.value;
    select.replaceChildren(new Option(t('官方版'), OFFICIAL), ...versions.map(v => new Option(`${t('修改版')}：${v.name}`, v.name)));
    if ([...select.options].some(o => o.value === keep)) select.value = keep;
  }
  if (!b.value || b.value === a.value) b.value = [...b.options].find(o => o.value !== a.value)?.value ?? OFFICIAL;
}

function show(lines, kind = '') {
  const box = $('compare-versions-result');
  box.replaceChildren(...lines.map(line => (typeof line === 'string' ? el('p', { className: `mini ${kind}`.trim(), textContent: t(line) }) : line)));
}

function renderResult(result) {
  const table = el('table', { className: 'result-table compare-table' });
  const head = el('tr', {}, ['', '版本', '状态', '最优成员', '率定期目标', '验证期目标', '基线'].map(text => el('th', { textContent: t(text) })));
  table.append(el('thead', {}, [head]), el('tbody', {}, compareRows(result).map(row => el('tr', {}, [
    el('td', { textContent: row.key.toUpperCase() }), el('td', { textContent: t(row.version) }), el('td', { textContent: row.status }),
    el('td', { textContent: row.best }), el('td', { textContent: row.calibration }), el('td', { textContent: row.validation }), el('td', { textContent: row.baseline }),
  ]))));
  show([
    table,
    el('p', { className: 'compare-verdict', textContent: t(compareVerdict(result)) }),
    ...(result.notes ?? []).map(note => el('p', { className: 'muted mini', textContent: note })),
  ]);
}

/** 修改版要能在选定的位置跑（本机要先编好；服务器上要源码干净）。 */
async function checkReady(version) {
  if (version === OFFICIAL) return null;
  const readiness = await invoke('version_check', { name: version, kernel: $('kernel').value, engine: modelEngine() });
  return runModeBlocker({ version, target: studyTarget(), readiness, engine: modelEngine() });
}

async function runSide(dir, version, label) {
  const envelope = JSON.parse(await invoke('study_status', { studyDir: dir }));
  if (finished(envelope)) return;
  show([`${t('正在率定')} ${label}（${t(versionLabel(version))}）…`], 'muted');
  const target = studyTarget();
  if (target !== 'local') {
    // 服务器上跑：交过去，等它跑完、取回（研究的状态与结果移植进本机这个研究）。
    if (!envelope.remote?.host) {
      await submitStudies([dir], {
        host: target, kernel: $('kernel').value, engine: modelEngine(), jobs: null, versionFor: d => studyVersionArg(d, version),
      });
    }
    await awaitServerStudy(dir, { onProgress: text => show([`${label}：${t(text)}`], 'muted') });
    return;
  }
  await invoke('study_run', {
    studyDir: dir, kernel: $('kernel').value, stream: true, jobs: null, retryFailed: false,
    engine: modelEngine(), modelVersion: await studyVersionArg(dir, version),
  });
}

async function compareVersions() {
  if (busy) return;
  const dirs = activeTuningStudies();
  if (dirs.length !== 1) {
    show([dirs.length ? '比较两个版本一次只比较一个率定任务；当前有多个（按站点分开的率定），请只保留一个。' : '先在上面生成一个参数率定任务。'], 'warn');
    return;
  }
  busy = true;
  $('compare-versions-run').disabled = true;
  try {
    const studyA = dirs[0];
    const envelopeA = JSON.parse(await invoke('study_status', { studyDir: studyA }));
    const fixedA = fixedVersion(envelopeA);
    let versionA = $('compare-version-a').value || OFFICIAL;
    if (fixedA && fixedA !== versionA) {
      versionA = fixedA;
      $('compare-version-a').value = fixedA;
      status(`${t('这个率定任务已经用')} ${t(versionLabel(fixedA))} ${t('跑过，版本 A 固定为它')}`);
    }
    const versionB = $('compare-version-b').value || OFFICIAL;
    if (versionA === versionB) { show(['两边选了同一个版本；请给版本 B 选另一个。'], 'warn'); return; }
    for (const version of [versionA, versionB]) {
      const blocked = await checkReady(version);
      if (blocked) { show([blocked], 'warn'); return; }
    }
    let studyB = loadPairs()[studyA];
    if (studyB) {
      try {
        const envelopeB = JSON.parse(await invoke('study_status', { studyDir: studyB }));
        const fixedB = fixedVersion(envelopeB);
        if (fixedB && fixedB !== versionB) studyB = null;
      } catch { studyB = null; }
    }
    if (!studyB) {
      show(['正在按同样的设置建第二个率定任务…'], 'muted');
      studyB = await invoke('study_clone', { studyDir: studyA });
      savePair(studyA, studyB);
    }
    await runSide(studyA, versionA, 'A');
    await runSide(studyB, versionB, 'B');
    await refreshTuningStudy().catch(() => {});
    renderResult(await invoke('study_compare', { a: studyA, b: studyB }));
  } catch (error) {
    show([String(error?.message || error)], 'assistant-fail');
  } finally {
    busy = false;
    $('compare-versions-run').disabled = false;
  }
}

function wire() {
  if (!hasBackend || !$('compare-versions-run')) return;
  $('compare-versions-run').onclick = () => compareVersions().catch(status);
  addEventListener('colm:step', () => { if (state.step === 'result-tuning') fillVersions().catch(() => {}); });
  addEventListener('colm:versions', () => fillVersions().catch(() => {}));
}

wire();
