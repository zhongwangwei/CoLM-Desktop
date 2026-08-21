//! 运行产物匹配、运行控制、进度与日志。

import { invoke, listen } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { renderCases, ensureCases, renderSites } from './sites.js';
import { batchTarget, updateCaseBatchButtons } from './batch.js';
import { refreshVars } from './results.js';
import { setRunning, renderSteps, setStatus } from './shell.js';
import { renderFields } from './params.js';
import { kernelForSubgrid, urbanEnabled } from './kernel.js';

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

$('run').onclick = async () => {
  if (!state.selected) return;
  $('run').disabled = true;
  $('log').textContent = '';
  $('prog').style.width = '0';
  $('progtext').textContent = '启动…';
  try {
    renderStages({});   // 三段都回到「待运行」
    setRunning('busy', '运行中');
    await invoke('run_case', {
      case: state.selected.dir, kernel: $('kernel').value, force: $('force').checked,
    });
  } catch (e) {
    // run://done 只在子进程真的起来之后才会发。起不来的话这里是唯一的收尾点，
    // 不写的话进度文字会永远停在「启动…」。
    $('status').textContent = String(e);
    $('progtext').textContent = '没能启动 —— ' + e;
    $('run').disabled = false;
  }
};

/** 订阅三个运行事件。由 `main.js` 在启动时调一次。 */
export async function watchRun() {
    await listen('run://stage', e => {
      // 三段串行，而**只有 colm.x 打 TIMESTEP** —— 没有这条，前两段跑的时候
      // 界面完全不知道进行到哪。标记由 colm-cli 自己打，不认 CoLM 的措辞。
      const { stage, state: st, case: dir } = e.payload;
      if (dir) { state.runState[dir] = '运行中'; renderCases(); }
      state.stages = { ...state.stages, [stage]: st };
      renderStages(state.stages);
      if (st === 'begin') {
        $('progtext').textContent = `${stage} 运行中…`;
        // 前两段没有步数，阶段徽标负责说明状态；不拿猜出来的 2%/4% 冒充进度。
        $('prog').style.width = '0';
      }
    });
    await listen('run://progress', e => {
      // 后端已从 case.nml 算出总步数；这里直接显示模型步完成比例。
      const p = e.payload;
      // 预热与正常推进要分开说。CoLM 在预热期**不写 history**
      // （MOD_Hist.F90:235 在 itstamp <= ptstamp 时直接 RETURN），
      // 混进正常进度会让人以为那段输出被算进了结果。
      const total = p.total_steps || p.step;
      $('progtext').textContent = p.spinup
        ? `预热 ${p.spinup[0]}/${p.spinup[1]} 轮 · 第 ${p.step}/${total} 步 · ${p.date}`
        : `第 ${p.step}/${total} 步 · ${p.date}`;
      $('prog').style.width = Math.min(100, 100 * p.step / total) + '%';
    });
    await listen('run://lines', e => {
      const el = $('log');
      // 事件是**成批**到的（后端每 100 毫秒合并一次），所以这里一次追加一批。
      // payload 从数组变成了 { case, lines } —— 批量跑时要分得清来源。
      el.textContent += e.payload.lines.join('\n') + '\n';
      if (el.textContent.length > 60000) el.textContent = el.textContent.slice(-40000);
      el.scrollTop = el.scrollHeight;
      updateLogInfo();
    });
    await listen('run://done', e => {
      const d = e.payload;
      if (d.case) { state.runState[d.case] = d.code === 0 ? '已完成' : '失败'; renderCases(); }
      // 状态栏是切到别的步骤时**唯一还看得见运行结果**的地方。
      // **退出码说明不了任何事。** 真正的原因在 stderr 上，后端现在把它
      // 一并带过来了 —— 只报「失败（退出码 1）」会逼着人自己去磁盘上翻日志。
      setRunning(d.code === 0 ? 'ok' : 'fail',
        d.code === 0 ? '完成' : `失败：${d.reason ?? '退出码 ' + d.code}`);
      $('prog').style.width = d.code === 0 ? '100%' : '0';
      $('progtext').textContent =
        `${d.code === 0 ? '完成' : '失败（退出码 ' + d.code + '）' + (d.reason ? '：' + d.reason : '')} · ` +
        `子进程打了 ${d.total} 行，丢弃 ${d.dropped} 行噪声`;
      $('run').disabled = false;
      if (d.code === 0 && state.selected) {
        // list_cases 是运行**之前**扫的，这个标记那时还是 false ——
        // 不在这里更新的话，跑完第一次「画图」仍然是灰的，
        // 而用户完全看不出为什么。
        state.selected.has_history = true;
        renderCases();
        refreshVars();
      }
    });
}

// ---------------------------------------------------------------- 批量

$('runall').onclick = async () => {
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
  $('runall').disabled = true;
  $('run').disabled = true;
  // 先把所有算例标成「待运行」。不先标的话，还没轮到的那些在界面上
  // 与「已完成」长得一样，用户看不出批次进行到哪。
  for (const d of dirs) state.runState[d] = '待运行';
  renderCases();
  try {
    const n = await invoke('run_batch', {
      cases: dirs, kernel: $('kernel').value, maxConcurrent: requestedWorkers(),
    });
    status(`批次结束：${n}/${dirs.length} 个算例跑完`);
  } catch (e) { status(e); }
  finally { updateCaseBatchButtons(); $('run').disabled = false; }
};


/** 三段各自的状态。**分开显示是必须的** —— 只有 colm.x 打 TIMESTEP，
 *  前两段没有步进度可看，而城市算例里 mksrfdata 恰恰是最慢的那段。 */
function renderStages(st) {
  const box = $('stages');
  box.textContent = '';
  const LABEL = { begin: '运行中', ok: '成功', failed: '失败', skipped: '跳过' };
  for (const s of ['mksrfdata', 'mkinidata', 'colm']) {
    const d = document.createElement('span');
    const state = st[s];
    d.textContent = `${s}：${LABEL[state] ?? '待运行'}`;
    d.style.cssText = 'font-size:11px;padding:2px 8px;border-radius:10px;background:var(--soft)';
    if (state === 'failed') d.className = 'warn';
    // 「跳过」要看得出来是**有意跳过**而不是没跑 —— 两者在界面上长得太像，
    // 而误以为没跑会让人去按强制重跑，白等一次。
    if (state === 'skipped') { d.className = 'muted'; d.title = '产物齐全且输入未变'; }
    box.appendChild(d);
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
    const dir = state.selected?.dir;
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

$('log-clear').onclick = () => { $('log').textContent = ''; updateLogInfo(); };

export function updateLogInfo() {
  const n = $('log').textContent.length;
  $('loginfo').textContent = n ? `${n} 个字符` : ' ';
}
