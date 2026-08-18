//! 结果：可画的变量、取序列、画图。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { batchTarget, updateCaseBatchButtons } from './batch.js';

// 能画的只有 (time, patch) 形状的那些 —— 119 个变量里 108 个是。
// 其余 11 个是剖面（(time, patch, soil) 之类），需要另一种画法。
const PLOTTABLE = [
  ['f_rnet', '净辐射 Rnet', 'W/m2'],
  ['f_fsena', '感热 Qh', 'W/m2'],
  ['f_lfevpa', '潜热 Qle', 'W/m2'],
  ['f_fgrnd', '地表热通量 Qg', 'W/m2'],
  ['f_sr', '反射短波 SWup', 'W/m2'],
  ['f_xy_t', '参考高度气温', 'K'],
  ['f_rnof', '总产流', 'mm/s'],
  ['f_zwt', '地下水位', 'm'],
];

export function refreshVars() {
  const s = $('var');
  s.textContent = '';
  for (const [id, label, unit] of PLOTTABLE) {
    const o = document.createElement('option');
    o.value = id; o.textContent = `${label}  [${unit}]`;
    s.appendChild(o);
  }
  $('plot').disabled = !(state.selected && state.selected.has_history);
  // 评估要三样：跑过的算例、观测文件、以及能配对上的时间轴。前两样在这里判。
  const obs = $('obs').value.trim() || autoObs();
  $('obs').value = obs;
  $('evaluate').disabled = !(state.selected && state.selected.has_history && obs);
  // 批量评估只要有一个跑过的算例就能点 —— 观测按站点逐个找，缺的那几个跳过并记名。

}

/** 从站点库里找这个算例对应的观测文件。
 *
 *  §1.4：观测靠命名约定找，不让用户为每个站点手工指路。找不到就留空，
 *  由上面那个输入框兜底 —— 命名不合约定的数据仍然要能用。 */
function autoObs() {
  if (!state.selected) return '';
  const name = state.selected.name;
  return state.sites.find(s => s.name === name)?.obs_file ?? '';
}

$('plot').onclick = async () => {
  if (!state.selected) return;
  const id = $('var').value;
  const meta = PLOTTABLE.find(p => p[0] === id);
  $('plot').disabled = true;
  try {
    const json = await invoke('series', { case: state.selected.dir, vars: id });
    const d = JSON.parse(json);
    draw(d, id, meta);
  } catch (e) { $('status').textContent = String(e); }
  finally { $('plot').disabled = false; }
};


// 建好的图表。**uPlot 的宽度是建图那一刻算死的** —— 容器变窄它不会跟着缩，
// 超出的部分会把整页撑宽。CSS 那边用 `overflow: hidden` 兜住了不撑页，
// 但裁掉一半的图仍然是坏的，所以这里让它真的跟着窗口变。
const CHARTS = new Set();

function track(u, host) {
  CHARTS.add({ u, host });
  // 图被 `while (box.children.length > 4)` 删掉时，它的 host 也就脱离了文档 ——
  // 不清理的话这个集合会一直涨，而每次 resize 都要遍历它。
  for (const c of [...CHARTS]) if (!c.host.isConnected) CHARTS.delete(c);
  return u;
}

let resizeTimer;
addEventListener('resize', () => {
  // 拖窗口时 resize 每秒能触发几十次，而每次都重排一张 264 点的图不划算。
  clearTimeout(resizeTimer);
  resizeTimer = setTimeout(() => {
    for (const c of [...CHARTS]) {
      if (!c.host.isConnected) { CHARTS.delete(c); continue; }
      const w = c.host.parentElement.clientWidth - 30;
      if (w > 0) c.u.setSize({ width: w, height: 190 });
    }
  }, 120);
});

function draw(d, id, meta) {
  const host = document.createElement('div');
  host.className = 'chart';
  const box = $('charts');
  box.prepend(host);
  // 不设上限的话，反复点「画图」会把几十张图堆在一起，每张都还挂着
  // 自己的事件监听 —— 界面会越用越卡。
  while (box.children.length > 4) box.lastElementChild.remove();
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  track(new uPlot({
    width: host.parentElement.clientWidth - 30,
    height: 190,
    title: `${meta[1]}  ·  ${d.n} 点`,
    // **必须按 UTC 格式化。** PLUMBER2 是地方时、模型也按地方时推进，
    // 时间戳是「把地方时当成 UTC」算出来的；按浏览器本地时区格式化
    // 会把整条曲线平移一个时区。
    tzDate: ts => uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'),
    series: [
      { label: '时间' },
      { label: meta[2], stroke: dark ? '#8fd3a6' : '#1e6b3a', width: 1.25 },
    ],
    axes: [{}, { label: meta[2] }],
  }, [d.time, d.vars[id]], host), host);
}

// ---------------------------------------------------------------- 评估

$('evaluate').onclick = async () => {
  if (!state.selected) return;
  const obs = $('obs').value.trim();
  if (!obs) { status('要先给观测文件'); return; }
  $('evaluate').disabled = true;
  try {
    const rows = JSON.parse(await invoke('metrics', {
      case: state.selected.dir, obs, spinup: Number($('spinup').value) || 0,
      corrected: $('corrected').checked,
    }));
    renderMetrics(rows);
  } catch (e) { status(e); }
  finally { $('evaluate').disabled = false; }
};

// 勾选一变就重算。**不能只改勾选** —— 表里的数字与勾选状态对不上，
// 比不给这个开关更糟：看的人会以为看到的是订正后的结果。
$('corrected').onchange = () => {
  if (!$('metrics').textContent.trim()) return;
  $('evaluate').click();
};

function renderMetrics(rows) {
  const box = $('metrics');
  box.textContent = '';
  if (!rows.length) { box.innerHTML = '<p class="muted">没有可配对的变量</p>'; return; }

  const tbl = document.createElement('table');
  const head = document.createElement('tr');
  for (const h of ['变量', 'n', 'RMSE', 'bias', 'R²', 'KGE']) {
    const th = document.createElement('th');
    th.textContent = h;
    if (h !== '变量') th.className = 'n';
    head.appendChild(th);
  }
  tbl.appendChild(head);

  for (const r of rows) {
    const tr = document.createElement('tr');
    const cells = [
      [r.obs_var ?? r.name, ''],
      [r.n, 'n'], [r.rmse.toFixed(1), 'n'],
      [(r.bias >= 0 ? '+' : '') + r.bias.toFixed(2), 'n'],
      [r.r2.toFixed(3), 'n'],
      [(r.kge >= 0 ? '+' : '') + r.kge.toFixed(3), 'n'],
    ];
    for (const [v, cls] of cells) {
      const td = document.createElement('td');
      td.textContent = v;
      if (cls) td.className = cls;
      tr.appendChild(td);
    }
    // **β 警告非空一定要显示。** 藏起来等于给一个假指标 ——
    // Qh 那一行的 KGE 是 -11.6，不说明白会被当成模型烂到离谱。
    if (r.beta_warning) {
      tr.lastChild.className = 'n warn';
      tr.lastChild.title = r.beta_warning;
      tr.title = r.beta_warning;
    }
    tr.style.cursor = 'pointer';
    tr.onclick = () => drawComparison(r);
    tbl.appendChild(tr);
  }
  box.appendChild(tbl);
  const hint = document.createElement('p');
  hint.className = 'muted';
  hint.style.fontSize = '11px';
  hint.textContent = '点一行看模型与观测的对比图';
  box.appendChild(hint);
  const warned = rows.filter(r => r.beta_warning);
  for (const r of warned) {
    const p = document.createElement('p');
    p.className = 'warn';
    p.style.fontSize = '11px';
    p.textContent = `${r.name}：${r.beta_warning}`;
    box.appendChild(p);
  }
}

/** 模型与观测的双线图 + 散点图。两张都从同一批配对点画。 */
function drawComparison(r) {
  const box = $('charts');
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  const host = document.createElement('div');
  host.className = 'chart';
  box.prepend(host);
  while (box.children.length > 4) box.lastElementChild.remove();
  track(new uPlot({
    width: host.parentElement.clientWidth - 30,
    height: 190,
    title: `${r.name}  ·  模型 vs 观测  ·  n=${r.n}`,
    // **必须按 UTC 格式化**，理由同 draw()：时间戳是「把地方时当成 UTC」
    // 算出来的，按浏览器本地时区格式化会把整条曲线平移一个时区。
    tzDate: ts => uPlot.tzDate(new Date(ts * 1000), 'Etc/UTC'),
    series: [
      { label: '时间' },
      { label: '模型', stroke: dark ? '#8fd3a6' : '#1e6b3a', width: 1.25 },
      { label: '观测', stroke: dark ? '#e0a45e' : '#a5610d', width: 1.25 },
    ],
    axes: [{}, {}],
  }, [r.time, r.model, r.obs], host), host);

  // 散点 + 1:1 线。一眼看偏差是系统性的还是散的 —— 那是 bias 与 R²
  // 两个数合起来才说得清的事，而图上一看就明白。
  const s = document.createElement('div');
  s.className = 'chart';
  box.prepend(s);
  const order = r.obs.map((o, i) => [o, r.model[i]]).sort((a, b) => a[0] - b[0]);
  track(new uPlot({
    width: s.parentElement.clientWidth - 30,
    height: 190,
    title: `${r.name}  ·  观测（横）对模型（纵）`,
    scales: { x: { time: false } },
    series: [
      { label: '观测' },
      { label: '模型', stroke: 'transparent',
        points: { show: true, size: 3, stroke: dark ? '#8fd3a6' : '#1e6b3a' } },
      { label: '1:1', stroke: dark ? '#888' : '#bbb', width: 1, dash: [4, 4] },
    ],
  // 第三条 series 是 1:1 线：横坐标就是纵坐标。按观测排过序，
  // 所以它画出来是一条直线而不是折线。
  }, [order.map(p => p[0]), order.map(p => p[1]), order.map(p => p[0])], s), s);
}

// ---------------------------------------------------------------- 批量汇总

/** 把每个跑过的算例各评估一次，汇成一张按站点排的表。
 *
 *  **前端循环，没有新开后端接口。** 实测一次评估 0.03 秒（热缓存），
 *  90 个站点约 3 秒 —— 为此加一个批量命令，省下的时间还不如它带来的
 *  第二份参数处理逻辑值钱。冷缓存首次 1.39 秒，所以照样报进度。
 */
$('eval-all').onclick = async () => {
  const spinup = Number($('spinup').value) || 0;
  const todo = batchTarget().filter(c => c.has_history);
  if (!todo.length) return;
  $('eval-all').disabled = true;
  const rows = [];
  const failed = [];
  try {
    for (let i = 0; i < todo.length; i++) {
      const c = todo[i];
      status(`评估 ${i + 1}/${todo.length}：${c.name}`);
      const obs = state.sites.find(s => s.name === c.name)?.obs_file;
      // 没有观测就跳过，并**记下来** —— 一张少了几行的表，
      // 不说明少了谁的话，看的人会以为那几个站评估结果为零。
      if (!obs) { failed.push([c.name, '没有观测文件']); continue; }
      try {
        const one = JSON.parse(await invoke('metrics', {
          case: c.dir, obs, spinup, corrected: $('corrected').checked,
        }));
        for (const r of one) rows.push({ site: c.name, ...r });
      } catch (e) { failed.push([c.name, String(e)]); }
    }
    renderSummary(rows, failed, todo.length);
    status(`批量评估完成：${todo.length} 个算例`);
  } finally { updateCaseBatchButtons(); }
};

function renderSummary(rows, failed, total) {
  const box = $('summary');
  box.textContent = '';
  if (!rows.length && !failed.length) return;

  // 变量太多会让表没法看。默认只汇总 Rnet，其余可切换 ——
  // 一张 90 行 × 5 变量的表读不出任何东西。
  const vars = [...new Set(rows.map(r => r.name))];
  state.summaryVar ??= vars[0];
  const bar = document.createElement('div');
  bar.className = 'row';
  for (const v of vars) {
    const b = document.createElement('button');
    b.textContent = v;
    b.setAttribute('aria-pressed', String(v === state.summaryVar));
    b.onclick = () => { state.summaryVar = v; renderSummary(rows, failed, total); };
    bar.appendChild(b);
  }
  box.appendChild(bar);

  const shown = rows.filter(r => r.name === state.summaryVar);
  const tbl = document.createElement('table');
  const head = document.createElement('tr');
  const COLS = [['站点', 'site'], ['n', 'n'], ['RMSE', 'rmse'], ['bias', 'bias'], ['R²', 'r2'], ['KGE', 'kge']];
  for (const [label, key] of COLS) {
    const th = document.createElement('th');
    th.textContent = label + (state.summarySort === key ? ' ▾' : '');
    th.style.cursor = 'pointer';
    if (key !== 'site') th.className = 'n';
    // 排序是这张表唯一的用处 —— 90 行里找最差的那几个，靠眼睛扫不出来。
    th.onclick = () => { state.summarySort = key; renderSummary(rows, failed, total); };
    head.appendChild(th);
  }
  tbl.appendChild(head);
  const key = state.summarySort ?? 'site';
  const sorted = [...shown].sort((a, b) =>
    key === 'site' ? String(a.site).localeCompare(b.site) : (b[key] ?? 0) - (a[key] ?? 0));
  for (const r of sorted) {
    const tr = document.createElement('tr');
    for (const [, k] of COLS) {
      const td = document.createElement('td');
      const v = r[k];
      td.textContent = typeof v === 'number' ? (Number.isInteger(v) ? v : v.toFixed(3)) : v;
      if (k !== 'site') td.className = 'n';
      tr.appendChild(td);
    }
    if (r.beta_warning) { tr.title = r.beta_warning; tr.lastChild.className = 'n warn'; }
    tbl.appendChild(tr);
  }
  box.appendChild(tbl);

  const note = document.createElement('p');
  note.className = failed.length ? 'warn' : 'muted';
  note.style.fontSize = '11px';
  note.textContent = failed.length
    ? `${shown.length}/${total} 个算例有结果；${failed.length} 个没有：` +
      failed.map(([n, why]) => `${n}（${why}）`).join('、')
    : `${shown.length}/${total} 个算例`;
  box.appendChild(note);
}
