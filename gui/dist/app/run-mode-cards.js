import { OFFICIAL, touchesText } from './run-mode.js';
import { cachedProbe } from './remote-probe-cache.js';
import { language, translateZh } from './i18n.js';

const t = text => language() === 'en' ? translateZh(text) : text;

function element(tag, className, text) {
  const node = document.createElement(tag);
  node.className = className;
  if (text) node.textContent = text;
  return node;
}

function versionBadge(lights) {
  if (Object.values(lights ?? {}).includes('fail')) return '✕ 检查没通过';
  return lights?.compile === 'pass' && lights?.tests === 'pass' ? '✓ 检查已通过' : '! 还没测试';
}

function serverChoice(server) {
  const cached = cachedProbe(server);
  const probe = cached?.probe;
  const facts = [];
  if (Number(probe?.cpus) > 0) facts.push(`${probe.cpus} ${t('个逻辑 CPU')}`);
  if (probe?.schedulers?.length) facts.push(probe.schedulers.join(' / '));
  const badge = !cached ? '没测过' : cached.error ? '连接失败（上次测试）' : '在线（上次测试）';
  return { value: server.host, title: server.host, detail: facts.join(' · '), badge: t(badge) };
}

/** Selection belongs to the caller; clicks follow its existing change path. */
export function renderRunModeCards(container, { versions = [], servers = [], selection, onChange, lockEngine = null }) {
  const active = document.activeElement;
  const focusedId = active && container.contains(active) ? active.id : null;
  const cpus = Number(globalThis.navigator?.hardwareConcurrency);
  const groups = [
    ['version', '用哪个模型版本', [
      { value: OFFICIAL, title: t('官方版'), detail: t('CoLM 2024 · 随软件发布，不可修改'), badge: t('默认') },
      ...versions.map(ws => ({ value: ws.name, title: ws.name, detail: t(touchesText(ws.touches)), badge: t(versionBadge(ws.lights)) })),
    ]],
    ['target', '在哪里跑', [
      { value: 'local', title: t('本机'), detail: cpus > 0 ? `${cpus} ${t('个逻辑 CPU')}` : '' },
      ...servers.map(serverChoice),
    ]],
    ['engine', '用哪个引擎', [
      { value: 'rust', title: t('Rust 引擎'), detail: t('单进程多线程；站点、网格、流域都能跑，与 Fortran 逐位一致'), badge: t('推荐') },
      { value: 'fortran', title: t('Fortran 内核'), detail: t('需要多进程 MPI 时选；装了 AI 网络的算例不能用它跑') },
    ]],
  ];
  for (const [key, title, choices] of groups) {
    let group = container.querySelector(`[data-run-choice="${key}"]`);
    if (!group) {
      const section = element('section', 'run-mode-group');
      section.append(element('h3', '', t(title)));
      group = element('div', 'run-choice-grid');
      group.dataset.runChoice = key;
      section.append(group);
      container.append(section);
    }
    group.replaceChildren();
    choices.forEach((choice, index) => {
      const button = element('button', 'run-mode-card');
      button.type = 'button';
      button.id = `${container.id}-choice-${key}-${index}`;
      button.dataset.value = choice.value;
      button.setAttribute('aria-pressed', String(selection[key] === choice.value));
      button.disabled = key === 'engine' && !!lockEngine && choice.value !== lockEngine;
      button.append(element('strong', 'run-mode-card-title', choice.title));
      if (choice.detail) button.append(element('span', 'run-mode-card-detail', choice.detail));
      if (choice.badge) button.append(element('span', 'run-mode-card-badge', choice.badge));
      if (button.disabled) button.append(element('span', 'run-mode-card-detail', t('此研究已锁定引擎，不能切换')));
      button.onclick = () => { if (!button.disabled) onChange(key, choice.value); };
      group.append(button);
      if (button.id === focusedId && !button.disabled) button.focus({ preventScroll: true });
    });
  }
}
