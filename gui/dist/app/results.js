//! 结果：可画的变量、取序列、画图。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';

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

function draw(d, id, meta) {
  const host = document.createElement('div');
  host.className = 'chart';
  const box = $('charts');
  box.prepend(host);
  // 不设上限的话，反复点「画图」会把几十张图堆在一起，每张都还挂着
  // 自己的事件监听 —— 界面会越用越卡。
  while (box.children.length > 4) box.lastElementChild.remove();
  const dark = matchMedia('(prefers-color-scheme: dark)').matches;
  new uPlot({
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
  }, [d.time, d.vars[id]], host);
}
