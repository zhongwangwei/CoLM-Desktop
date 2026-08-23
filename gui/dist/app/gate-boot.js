//! 只画首页；完整工作台留到首帧之后加载。

import { showDomainGate } from './domain.js';

showDomainGate();
requestAnimationFrame(() => setTimeout(() => import('./main.js'), 0));
