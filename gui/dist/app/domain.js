//! 进门第一道：这次要跑什么。
//!
//! **不是欢迎页，是分流点。** 站点、区域、全球三种域各自要的前处理、
//! 地表数据与并行设置都不一样，将来它们会各自展开自己的步骤链。
//! 三档现在就摆出来，任何一档落地时不用再改这一层，且用户一眼看得到路线图。
//!
//! **没实现的那两档是 disabled，不是「点了报错」** —— 一个能点但必然失败的
//! 入口比一个灰着的更糟。
//!
//! **每次启动都弹，不记忆。** 它在区域与全球落地后是真正的分流点，
//! 不是一次性的欢迎页。
//!
//! 依赖方向只出不进：`main.js` import 它，它不被任何业务模块 import ——
//! `check-gui` 禁止模块成环，而 `sites ↔ results` 有前科。

import { state } from './state.js';
import { $ } from './ui.js';
import { go } from './shell.js';

const DOMAINS = [
  { id: 'site',   t: '站点', d: 'PLUMBER2 / Urban-PLUMBER 单点模拟', ready: true },
  { id: 'region', t: '区域', d: '有限范围网格', ready: false },
  { id: 'global', t: '全球', d: '全球网格', ready: false },
];

/** 立起门。后台初始化在它后面照常跑 —— 门只是视觉遮挡。 */
export function showDomainGate() {
  const box = $('domaincards');
  box.textContent = '';
  for (const d of DOMAINS) {
    const b = document.createElement('button');
    b.className = 'domain-card';
    b.disabled = !d.ready;
    const t = document.createElement('span');
    t.className = 'dt';
    t.textContent = d.t;
    b.appendChild(t);
    const sub = document.createElement('span');
    sub.className = 'dd';
    sub.textContent = d.d;
    b.appendChild(sub);
    if (d.ready) {
      b.onclick = () => pick(d.id);
    } else {
      const soon = document.createElement('span');
      soon.className = 'dsoon';
      soon.textContent = '暂不支持';
      b.appendChild(soon);
    }
    box.appendChild(b);
  }
  $('domaingate').hidden = false;
}

function pick(id) {
  state.domain = id;
  $('domaingate').hidden = true;
  go('prep');
}
