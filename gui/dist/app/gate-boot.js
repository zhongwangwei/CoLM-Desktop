//! 先画启动页；完整工作台就绪后再显示首页卡片。

import { showDomainGate } from './domain.js';
import { $ } from './ui.js';

$('domaingate').hidden = true;
$('loadinggate').hidden = false;
$('launchgate').hidden = true;
$('localRunCard').onclick = () => {
  $('launchgate').hidden = true;
  showDomainGate();
};

export function showLaunchGate() {
  $('loadinggate').hidden = true;
  $('launchgate').hidden = false;
}

requestAnimationFrame(() => setTimeout(async () => {
  try {
    const { ready } = await import('./main.js');
    await ready;
    showLaunchGate();
  } catch (e) {
    $('loading-status').textContent = `Loading failed: ${e}`;
  }
}, 0));
