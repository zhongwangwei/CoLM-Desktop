//! 运行产物匹配、运行控制、进度与日志。

import { invoke, listen } from './ipc.js';
import { state } from './state.js';
import { $, status, baseName } from './ui.js';
import { renderCases, ensureCases, renderSites } from './sites.js';
import { batchTarget, updateCaseBatchButtons } from './batch.js';
import { invalidateResultCase, refreshVars } from './results.js';
import { setRunning, renderSteps, setStatus } from './shell.js';
import { renderFields } from './params.js';
import { kernelForSubgrid, urbanEnabled } from './kernel.js';
import { appendLogText, progressText } from './run-format.js';

// 单点内核不启 MPI；多核的实际用途是并发跑多个独立站点。默认沿用原来的
// 两路并发，但让用户在基本设定里按机器容量调整。
const cpuCapacity = Math.max(1, Number(navigator.hardwareConcurrency) || 1);
$('cpu-workers').max = String(cpuCapacity);
$('cpu-workers').value = String(Math.min(cpuCapacity, Number($('cpu-workers').value) || 2));
$('cpu-capacity').textContent = `检测到 ${cpuCapacity} 个逻辑 CPU；单个站点仍使用 1 核。`;

function requestedWorkers() {
  const n = Math.trunc(Number($('cpu-workers').value));
  const valid = Number.isFinite(n) ? n : 1;
  const clamped = Math.max(1, Math.min(cpuCapacity, valid));
  $('cpu-workers').value = String(clamped);
  return clamped;
}

function runName(dir) {
  return state.cases.find(c => c.dir === dir)?.name ?? baseName(dir);
}

function renderSelectedLog() {
  const dir = $('log-case').value;
  $('log').textContent = state.runLogs[dir] ?? '';
  $('log').scrollTop = $('log').scrollHeight;
  updateLogInfo();
}

function renderLogChoices(dirs, preferred = $('log-case').value) {
  const pick = $('log-case');
  pick.textContent = '';
  for (const dir of dirs) {
    const o = document.createElement('option');
    o.value = dir;
    o.textContent = runName(dir);
    pick.appendChild(o);
  }
  const fallback = state.selected && dirs.includes(state.selected.dir) ? state.selected.dir : dirs[0];
  pick.value = dirs.includes(preferred) ? preferred : (fallback ?? '');
  renderSelectedLog();
}
$('log-case').onchange = renderSelectedLog;

function resetRunView(dirs) {
  state.runTargets = [...dirs];
  state.runningCases = new Set(dirs);
  state.runFailures = new Set();
  for (const dir of dirs) {
    state.runState[dir] = '待运行';
    state.runStages[dir] = {};
    state.runProgress[dir] = { step: 0, total_steps: 0, date: '', stage: '' };
    state.runLogs[dir] = '';
  }
  renderLogChoices(dirs);
  renderCaseProgress();
  renderStages();
  updateOverallProgress();
  setRunning('busy', `运行中（0/${dirs.length}）`);
  setStatus(`开始运行 ${dirs.length} 个算例`);
}

function failPendingRuns(reason) {
  const message = String(reason);
  for (const dir of [...state.runningCases]) {
    state.runningCases.delete(dir);
    state.runFailures.add(dir);
    state.runState[dir] = '失败';
    state.runProgress[dir] = { ...(state.runProgress[dir] ?? {}), reason: message };
    updateCaseProgress(dir);
  }
  updateOverallProgress();
  renderStages();
  renderCases();
}

function ensureRunTarget(dir) {
  if (state.runTargets.includes(dir)) return;
  state.runTargets.push(dir);
  state.runningCases.add(dir);
  state.runStages[dir] ??= {};
  state.runProgress[dir] ??= { step: 0, total_steps: 0, date: '', stage: '' };
  state.runLogs[dir] ??= '';
  renderLogChoices(state.runTargets, dir);
  renderCaseProgress();
}

function renderCaseProgress() {
  const box = $('case-progress');
  box.textContent = '';
  for (const dir of state.runTargets) {
    const row = document.createElement('div');
    row.className = 'case-progress';
    row.dataset.case = dir;
    const head = document.createElement('div');
    head.className = 'case-progress-head';
    const name = document.createElement('span'); name.textContent = runName(dir);
    const stateEl = document.createElement('span'); stateEl.dataset.role = 'state';
    head.appendChild(name); head.appendChild(stateEl);
    const bar = document.createElement('div'); bar.className = 'progress';
    const fill = document.createElement('i'); bar.appendChild(fill);
    const text = document.createElement('p'); text.className = 'muted mini';
    text.dataset.role = 'text';
    row.appendChild(head); row.appendChild(bar); row.appendChild(text);
    box.appendChild(row);
    updateCaseProgress(dir);
  }
}

function progressRow(dir) {
  return [...$('case-progress').children].find(row => row.dataset.case === dir);
}

function updateCaseProgress(dir) {
  const row = progressRow(dir);
  if (!row) return;
  const p = state.runProgress[dir] ?? {};
  const label = state.runState[dir] ?? '待运行';
  const done = label === '已完成';
  const pct = done ? 100 : (p.total_steps ? Math.min(100, 100 * p.step / p.total_steps) : 0);
  row.querySelector('.progress > i').style.width = `${pct}%`;
  row.querySelector('[data-role=state]').textContent = label;
  row.querySelector('[data-role=text]').textContent = progressText(p, label);
}

function updateOverallProgress() {
  const dirs = state.runTargets;
  const known = dirs.map(d => state.runProgress[d]).filter(p => p?.total_steps);
  const total = known.reduce((n, p) => n + p.total_steps, 0);
  const step = known.reduce((n, p) => n + Math.min(p.step, p.total_steps), 0);
  const finished = dirs.filter(d => !state.runningCases.has(d)).length;
  const pct = total ? Math.min(100, 100 * step / total) : 0;
  $('prog').style.width = `${pct}%`;
  $('progtext').textContent = dirs.length
    ? `批量总体：${finished}/${dirs.length} 个站点结束` + (total ? ` · 模型步 ${step}/${total}` : '')
    : '\u00a0';
}

export async function refreshKernels() {
  const s = $('kernel');
  state.kernels = await invoke('list_kernels');
  s.textContent = '';
  if (!state.kernels.length) {
    // 只在开发树里可能发生：装出来的程序自带 IGBP/USGS。
    const o = document.createElement('option');
    o.textContent = '没有找到内核'; o.value = '';
    s.appendChild(o);
    await applyKernel();
    globalThis.dispatchEvent?.(new Event('colm:kernels'));
    return;
  }
  for (const k of state.kernels) {
    const o = document.createElement('option');
    o.value = k.dir; o.textContent = k.preset;
    s.appendChild(o);
  }
  await syncKernel();
  globalThis.dispatchEvent?.(new Event('colm:kernels'));
}

/** 当前内核真正没编进去的字段。向导控制的运行时开关不属于这份名单。 */
async function refreshRelevance() {
  const dir = $('kernel').value;
  if (!dir) { state.irrelevant = new Set(); return; }
  try {
    state.irrelevant = new Set(await invoke('irrelevant_fields', { kernelDir: dir }));
  } catch (e) {
    // 内核校验不过时不该让整个界面失效 —— 那时「哪些字段有用」这个问题
    // 本来也没有答案，全显示是安全的方向。
    state.irrelevant = new Set();
    status(e);
  }
  renderFields();
}

async function syncKernel() {
  $('kernel').value = kernelForSubgrid()?.dir ?? '';
  await applyKernel();
}

/** 向导改变后，更新与编译产物相关的后续状态。 */
async function applyKernel() {
  // 城市栅格目录跟着向导的 URBAN 开关走；到选站点时必须已经可见。
  const ud = $('urbandirs');
  if (ud) ud.hidden = !urbanEnabled();
  // 向导变更后站点的 URBAN 匹配也要立即重画。
  if (state.sites.length) renderSites();
  await refreshRelevance();
  // renderSites() 末尾已经调过一次 renderSteps，不再重复刷。只有
  // state.sites 为空、没走 renderSites 那条分支时，这里才是唯一一次
  // 刷新左栏的机会，所以只在那时补一次。
  if (!state.sites.length) renderSteps();
  // 自带的示例有自然站与城市站，按钮跟着向导选择说出本次该用哪个。
  const ex = $('use-example');
  if (ex) {
    ex.textContent = urbanEnabled()
      ? '用自带的示例站点（城市站 AU-Preston）'
      : '用自带的示例站点（CN-Cng）';
  }
}

addEventListener('colm:wizard', () => { syncKernel(); });

const RUN_STAGES = ['mksrfdata', 'mkinidata', 'colm', null];
const RUN_BUTTONS = ['run-mksrfdata', 'run-mkinidata', 'run-colm', 'runall'];

for (let i = 0; i < RUN_STAGES.length; i++) {
  $(RUN_BUTTONS[i]).onclick = () => runRequested(RUN_STAGES[i]);
}

/** 下方四个按钮共用同一批目标。指定单段是明确的手工重建意图，始终强制
 *  执行该段；“运行全部”才由“强制全部重跑”决定是否忽略阶段指纹。 */
async function runRequested(stage) {
  // 勾了站点却还没建算例的，先建 —— **建算例不再是一道要人按的关**。
  const wanted = state.sites.filter(s => state.picked.has(s.site_file));
  if (wanted.length) {
    const made = await ensureCases(wanted);
    state.batch = [...new Set(made.map(c => c.dir))];
    state.pickedCases.clear();
    for (const c of made) state.pickedCases.add(c.dir);
  }
  const dirs = batchTarget().map(c => c.dir);
  if (!dirs.length) return;
  resetRunView(dirs);
  renderCases();
  const force = stage !== null || $('force').checked;
  try {
    if (dirs.length === 1) {
      const code = await invoke('run_case', {
        case: dirs[0], kernel: $('kernel').value, force, stage,
      });
      status(code === 0
        ? `${stage ?? '全部阶段'}运行完成`
        : `${stage ?? '全部阶段'}运行失败（退出码 ${code}）`);
    } else {
      const summary = await invoke('run_batch', {
        cases: dirs, kernel: $('kernel').value, maxConcurrent: requestedWorkers(),
        force, stage,
      });
      status(summary.failed
        ? `批次结束：${summary.succeeded}/${summary.total} 个成功，${summary.failed} 个失败`
        : `批次结束：${summary.succeeded}/${summary.total} 个算例全部成功`);
    }
  } catch (e) {
    failPendingRuns(e);
    // run://done 只在子进程真的起来之后才会发。起不来的话这里是唯一的收尾点。
    status(e);
    setRunning('fail', '批次启动失败');
  } finally {
    updateCaseBatchButtons();
  }
}

/** 订阅三个运行事件。由 `main.js` 在启动时调一次。 */
export async function watchRun() {
    await listen('run://stage', e => {
      // 三段串行，而**只有 colm.x 打 TIMESTEP** —— 没有这条，前两段跑的时候
      // 界面完全不知道进行到哪。标记由 colm-cli 自己打，不认 CoLM 的措辞。
      const { stage, state: st, case: dir } = e.payload;
      if (!dir) return;
      ensureRunTarget(dir);
      state.runState[dir] = '运行中';
      state.runStages[dir] = { ...(state.runStages[dir] ?? {}), [stage]: st };
      state.runProgress[dir] = {
        ...(state.runProgress[dir] ?? {}), stage, stage_state: st,
      };
      updateCaseProgress(dir);
      renderStages();
      renderCases();
    });
    await listen('run://progress', e => {
      // 后端已从 case.nml 算出总步数；这里直接显示模型步完成比例。
      const p = e.payload;
      ensureRunTarget(p.case);
      state.runState[p.case] = '运行中';
      state.runProgress[p.case] = p;
      updateCaseProgress(p.case);
      updateOverallProgress();
    });
    await listen('run://lines', e => {
      const { case: dir, lines } = e.payload;
      ensureRunTarget(dir);
      state.runLogs[dir] = appendLogText(state.runLogs[dir] ?? '', lines);
      if ($('log-case').value === dir) renderSelectedLog();
    });
    await listen('run://done', e => {
      const d = e.payload;
      if (!d.case) return;
      ensureRunTarget(d.case);
      state.runningCases.delete(d.case);
      state.runState[d.case] = d.code === 0 ? '已完成' : '失败';
      if (d.code !== 0) state.runFailures.add(d.case);
      const p = state.runProgress[d.case] ?? {};
      state.runProgress[d.case] = {
        ...p, reason: d.reason,
        step: d.code === 0 && p.total_steps ? p.total_steps : (p.step ?? 0),
      };
      // 必须按事件里的 case 更新；批量跑时 state.selected 只是代表算例，
      // 把每个完成事件都写给它会让其余站点永远显示“未跑”。
      const c = state.cases.find(c => c.dir === d.case);
      if (c && d.code === 0 && (d.requested_stage == null || d.requested_stage === 'colm')) {
        c.has_history = true;
        invalidateResultCase(d.case);
      }
      updateCaseProgress(d.case);
      updateOverallProgress();
      renderStages();
      renderCases();
      if (state.runningCases.size) {
        const ended = state.runTargets.length - state.runningCases.size;
        setRunning('busy', `运行中（${ended}/${state.runTargets.length}）`);
      } else if (state.runFailures.size) {
        setRunning('fail', `${state.runFailures.size} 个站点失败`);
      } else {
        setRunning('ok', '全部完成');
      }
      updateCaseBatchButtons();
      if (d.code === 0 && (d.requested_stage == null || d.requested_stage === 'colm')) refreshVars();
    });
}


/** 三段各自的状态。**分开显示是必须的** —— 只有 colm.x 打 TIMESTEP，
 *  前两段没有步进度可看，而城市算例里 mksrfdata 恰恰是最慢的那段。 */
function renderStages() {
  const LABEL = { begin: '运行中', ok: '成功', failed: '失败', skipped: '跳过' };
  for (const id of ['stages', 'stages2']) {
    const box = $(id);
    if (!box) continue;
    box.textContent = '';
    for (const s of ['mksrfdata', 'mkinidata', 'colm']) {
      const d = document.createElement('span');
      const states = state.runTargets.map(dir => state.runStages[dir]?.[s]).filter(Boolean);
      if (state.runTargets.length <= 1) {
        const st = states[0];
        d.textContent = `${s}：${LABEL[st] ?? '待运行'}`;
        if (st === 'failed') d.className = 'warn';
        if (st === 'skipped') { d.className = 'muted'; d.title = '产物齐全且输入未变'; }
      } else {
        const count = key => states.filter(st => st === key).length;
        const parts = [];
        if (count('ok')) parts.push(`${count('ok')}成功`);
        if (count('skipped')) parts.push(`${count('skipped')}跳过`);
        if (count('begin')) parts.push(`${count('begin')}运行中`);
        if (count('failed')) parts.push(`${count('failed')}失败`);
        const waiting = state.runTargets.length - states.length;
        if (waiting) parts.push(`${waiting}等待`);
        d.textContent = `${s}：${parts.join(' · ')}`;
        if (count('failed')) d.className = 'warn';
      }
      box.appendChild(d);
    }
  }
}

// ---------------------------------------------------------------- 日志

/** 复制整段日志。**出问题时人要做的第一件事就是把它发给别人**，
 *  而在一个 pre 里手工选中几千行是不可能的 —— 它还在滚。
 *
 *  日志窗只留最后 40000 字符（见 run://lines 那段的截断），所以复制到的
 *  也是那一段。完整的三段日志在算例目录里：`mksrfdata.log` / `mkinidata.log`
 *  / `colm.log` —— 复制成功时把这句一并说出来，免得有人以为剪贴板里就是全部。 */
$('log-copy').onclick = async () => {
  const text = $('log').textContent;
  if (!text.trim()) { setStatus('日志是空的'); return; }
  try {
    await navigator.clipboard.writeText(text);
    const dir = $('log-case').value;
    setStatus(`已复制 ${text.length} 个字符`
      + (dir ? `；完整日志在 ${dir}/{mksrfdata,mkinidata,colm}.log` : ''));
  } catch (e) {
    // 剪贴板可能被拒（无用户手势、无权限）。退回到「全选」，
    // 让人自己按一次复制 —— 比只报一句错强。
    const r = document.createRange();
    r.selectNodeContents($('log'));
    getSelection().removeAllRanges();
    getSelection().addRange(r);
    setStatus(`剪贴板不可用（${e}），已全选，请按 ⌘C`);
  }
};

$('log-clear').onclick = () => {
  const dir = $('log-case').value;
  if (dir) state.runLogs[dir] = '';
  renderSelectedLog();
};

export function updateLogInfo() {
  const n = $('log').textContent.length;
  $('loginfo').textContent = n ? `${n} 个字符` : ' ';
}
