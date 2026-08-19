//! 「时间与预热」卡片。
//!
//! 这两样都在 737 个字段的表里躺着（`DEF_simulation_time%*`），但**躺在
//! 表里等于不存在** —— 实测：用户翻完参数页，说"我没有看到 spin-up 的选项"。
//! 一个决定输出从哪天开始的开关，不该和 `DEF_USE_SNICAR` 长得一样。
//!
//! 值仍然写进同一份 case.nml，改完在表里也看得见 —— 这里只是给它一个
//! 说得出后果的入口，不是另一套配置。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { editTarget } from './batch.js';

/** 画卡片。`box` 是 `#timing`。 */
export async function renderTiming() {
  const box = $('timing');
  box.textContent = '';
  const dirs = editTarget();
  if (!dirs.length) return;
  let t;
  try { t = await invoke('read_timing', { dirs }); }
  catch (e) { box.textContent = String(e); return; }

  const card = document.createElement('div');
  card.className = 'card';
  card.innerHTML = `
    <h3>时间与预热（spin-up）</h3>
    <div class="ch">时间范围默认就是强迫场覆盖的<b>全部</b>范围，由文件说了算，不用填。</div>
    <table>
      <tr><th>模拟</th><td>${window_(t)}</td></tr>
      <tr><th>输出</th><td>${t.window_varies ? '各算例从自己的预热结束处开始'
        : t.output_start + ' → ' + t.end}</td></tr>
    </table>
    <div class="pill-row" style="margin-top:12px">
      <span class="mini">spin-up：重复开头</span>
      <label class="check"><input class="input" id="tm-years" type="number" min="0" step="1"
               value="${t.spinup_years}" style="width:4.5em"> 年</label>
      <label class="check">×<input class="input" id="tm-repeat" type="number" min="0" step="1"
               value="${t.spinup_repeat}" style="width:4.5em"> 遍</label>
      <span class="muted mini">任一格填 0 就是不预热</span>
    </div>
    <p class="muted mini" id="tm-note" style="margin-top:8px"></p>`;
  box.appendChild(card);

  note(t);
  const apply = async () => {
    const years = Math.max(0, +$('tm-years').value | 0);
    const repeat = Math.max(0, +$('tm-repeat').value | 0);
    try {
      const r = await invoke('set_spinup', { dirs, years, repeat });
      state.text = r.text;
      const what = repeat > 1 && years > 0
        ? `预热：重复开头 ${years} 年 ×${repeat} 遍`
        : '已关闭预热';
      status(r.written > 1 ? `${what}（${r.written} 个算例）` : what);
      // 重画自己：输出起始日跟着变，而那正是这个开关的代价所在。
      await renderTiming();
    } catch (e) { status(e); }
  };
  $('tm-years').onchange = apply;
  $('tm-repeat').onchange = apply;
}

/** 模拟窗口那一格的字。
 *
 *  **多站点时窗口本来就各不相同** —— 每个算例的窗口是它自己那份强迫场的
 *  完整覆盖范围，而各站点的记录长短不同。这里显示一个统一的日期区间会是
 *  假的，所以不显示。 */
function window_(t) {
  if (!t.window_varies) return `${t.start} → ${t.end}`;
  return `${t.count} 个算例各跑各自强迫场的全部范围（第一个是 ${t.start} → ${t.end}）`;
}

/** 一句话说清楚代价。**必须说** —— 预热期不写 history，开着预热就等于
 *  从输出里扣掉开头那几年，而扣掉的那段在界面上什么痕迹都不留。 */
function note(t) {
  const el = $('tm-note');
  if (t.spinup_varies) {
    el.innerHTML = `<b>这 ${t.count} 个算例的预热设置不一致</b>，上面显示的是第一个的。`
      + '改一下就会把全部统一成同一套。';
    return;
  }
  if (!t.spinup_repeat) {
    el.textContent = '没有预热：从初始场直接开跑。土壤温湿与碳库是慢变量，'
      + '头一段结果不代表这个站点的气候态。';
    return;
  }
  el.innerHTML = `预热期<b>不写输出</b>（MOD_Hist.F90:235 在 itstamp &lt;= ptstamp 时直接 RETURN），`
    + `所以 ${t.start} 到 ${t.output_start} 这段不在结果里 —— 预热是从窗口头上扣的，`
    + `不是加在前面的。总模拟量约为 ${t.spinup_years * t.spinup_repeat} 年预热 + 正式那段。`;
}
