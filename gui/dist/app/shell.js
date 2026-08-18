//! 外壳：步骤条、常规/专家、主题、状态栏、右栏页签。
//!
//! 这一层不碰业务，只管「现在在哪一步、界面长什么样」。
//! 把它单独拎出来是因为它是**唯一回答「这几块什么关系」的地方** ——
//! 原来那版把站点库、新建、算例库并排摆着，谁也看不出它们是一条流水线。

import { state } from './state.js';
import { $ } from './ui.js';

/** 五步。`need` 说明这一步要什么才能进 —— 灰着的步骤要能说出为什么。 */
export const STEPS = [
  { id: 'data',   t: '数据',   d: '有哪些站点', need: () => null },
  { id: 'case',   t: '算例',   d: '造一个 / 选一个',
    need: () => (state.sites.length || state.cases.length ? null : '先在第 1 步扫一个站点目录') },
  { id: 'params', t: '参数',   d: '物理与输出',
    need: () => (state.selected ? null : '先选一个算例') },
  { id: 'run',    t: '运行',   d: '三段依次跑',
    need: () => (state.selected ? null : '先选一个算例') },
  { id: 'result', t: '结果',   d: '曲线与指标',
    need: () => (state.selected ? null : '先选一个算例') },
];

export function go(id) {
  const step = STEPS.find(s => s.id === id);
  const why = step?.need();
  if (why) { setStatus(why); return; }
  state.step = id;
  for (const p of document.querySelectorAll('.page')) p.hidden = p.dataset.step !== id;
  renderSteps();
}

export function renderSteps() {
  const box = $('steps');
  box.textContent = '';
  for (const [i, s] of STEPS.entries()) {
    const why = s.need();
    const d = document.createElement('div');
    d.className = 'step' + (state.step === s.id ? ' active' : '');
    if (why) d.setAttribute('aria-disabled', 'true');
    d.innerHTML = `<span class="num">${i + 1}</span><span class="step-copy">
      <span class="t">${s.t}</span><span class="d">${why ?? s.d}</span></span>`;
    d.onclick = () => go(s.id);
    box.appendChild(d);
  }
  // 左下角那两行说的是「当前上下文」—— 切到任何一步都还看得见选了哪个站、
  // 用的哪个内核。那两样决定了另外几步的行为。
  $('estSite').textContent = state.selected?.name ?? '—';
  const k = $('kernel');
  $('estKernel').textContent = k?.selectedIndex >= 0 ? k.options[k.selectedIndex].textContent : '—';
  $('casename').value = state.selected ? state.selected.dir : '还没有算例';
}

export function setStatus(msg) { $('status').textContent = String(msg); }

/** 状态灯：空闲 / 忙 / 成功 / 失败。**运行是这个程序里唯一会让人等的事**，
 *  切到别的步骤时它仍然要看得见 —— 这正是状态栏存在的理由。 */
export function setRunning(kind, text) {
  const dot = $('rdot');
  dot.className = 'rdot' + (kind === 'busy' ? ' busy' : kind === 'ok' ? ' on' : '');
  $('rtext').textContent = text;
}

export function initShell() {
  // 主题：跟随系统，但用户能自己切，切了就记住。
  const saved = localStorage.getItem('theme');
  const sys = matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  setTheme(saved ?? sys);
  $('themeBtn').onclick = () => {
    const next = document.documentElement.dataset.theme === 'dark' ? 'light' : 'dark';
    setTheme(next);
    localStorage.setItem('theme', next);
  };

  for (const b of document.querySelectorAll('#modeSeg button')) {
    b.onclick = () => {
      for (const x of document.querySelectorAll('#modeSeg button')) x.classList.toggle('on', x === b);
      state.expert = b.dataset.mode === 'expert';
      document.body.className = state.expert ? 'expert' : 'normal';
      window.dispatchEvent(new CustomEvent('colm:mode'));
    };
  }

  for (const tabs of ['livetabs', 'ptabs']) {
    const el = $(tabs);
    if (!el) continue;
    for (const b of el.querySelectorAll('button')) {
      b.onclick = () => {
        for (const x of el.querySelectorAll('button')) x.classList.toggle('on', x === b);
        if (tabs === 'livetabs') {
          for (const p of document.querySelectorAll('.live-pane'))
            p.classList.toggle('on', p.dataset.pane === b.dataset.pane);
        } else {
          window.dispatchEvent(new CustomEvent('colm:ptab', { detail: b.dataset.ptab }));
        }
      };
    }
  }
  renderSteps();
}

function setTheme(t) {
  document.documentElement.dataset.theme = t;
  $('themeBtn').textContent = t === 'dark' ? '☀️' : '🌙';
}
