//! 只画首页；完整工作台留到首帧之后加载。

import { showDomainGate } from './domain.js';
import { $ } from './ui.js';

$('domaingate').hidden = true;
$('launchgate').hidden = false;
$('localRunCard').onclick = () => {
  $('launchgate').hidden = true;
  showDomainGate();
};
requestAnimationFrame(() => setTimeout(() => import('./main.js'), 0));
