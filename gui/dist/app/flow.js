//! 总流程图页：方块可点，跳到对应的一步（不满足前提时由 shell.js 的 go() 说明原因）。

import { go } from './shell.js';

for (const node of document.querySelectorAll?.('[data-step="flow"] [data-go]') ?? []) {
  node.addEventListener('click', () => go(node.dataset.go));
}
