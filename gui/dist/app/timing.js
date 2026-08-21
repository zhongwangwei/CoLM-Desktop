//! 「预热」卡片。
//!
//! 这两样都在 737 个字段的表里躺着（`DEF_simulation_time%*`），但**躺在
//! 表里等于不存在** —— 实测：用户翻完参数页，说"我没有看到 spin-up 的选项"。
//! 一个决定输出从哪天开始的开关，不该和 `DEF_USE_SNICAR` 长得一样。
//! 所以它有自己的卡片，摆在“基本设定 / 预热”。建算例之前该分页
//! 为空；建好之后复用同一份 case.nml 与批量编辑路径。
//!
//! **刷新时机挂在 `renderFields()` 上**（`params.js` 里 `await renderTiming()`）。
//! 那是选中算例之后必经的一次渲染。

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
    <h3>预热（spin-up）</h3>
    <div class="ch">让土壤温湿与碳库等慢变量先趋于平衡；预热期不写输出。</div>
    <div class="pill-row" style="margin-top:12px">
      <label class="check">每轮预热年数
        <input class="input" id="tm-years" type="number" min="0" step="1"
               value="${t.spinup_years}" style="width:4.5em"> 年</label>
      <label class="check">重复轮数
        <input class="input" id="tm-repeat" type="number" min="0" step="1"
               value="${t.spinup_repeat}" style="width:4.5em"> 轮</label>
      <button class="btn-ghost" id="tm-apply" type="button">应用</button>
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
      const what = repeat > 0 && years > 0
        ? `预热：每轮 ${years} 年，共重复 ${repeat} 轮`
        : '已关闭预热';
      status(r.written > 1 ? `${what}（${r.written} 个算例）` : what);
      // 重画自己：输出起始日跟着变，而那正是这个开关的代价所在。
      await renderTiming();
    } catch (e) { status(e); }
  };
  // 两个数是一项配置，必须成组提交。逐格 onchange 会在第一格改完、第二格
  // 仍是 0 时把后端关掉并重绘，两个输入就会立刻一起跳回 0。
  $('tm-apply').onclick = apply;
}

/** 一句话说清楚代价：预热期不写 history，正式输出从预热结束后开始。 */
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
  el.textContent = `每轮使用开头 ${t.spinup_years} 年，共重复 ${t.spinup_repeat} 轮。`
    + '预热期不写输出（MOD_Hist.F90:235 在 itstamp <= ptstamp 时直接 RETURN）；'
    + '完成全部预热轮次后才开始正式输出。';
}
