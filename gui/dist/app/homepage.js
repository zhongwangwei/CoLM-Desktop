//! 任务首页复用已有算例恢复与示例准备路径，不创建算例或开始计算。
import { invoke, hasBackend } from './ipc.js';
import { state } from './state.js';
import { $, status } from './ui.js';
import { language, translateZh } from './i18n.js';
import { showDomainGate, startSession } from './domain.js';
import { openExistingCase } from './opencase.js';
import { loadExample, sitesForWizard, pickSite } from './sites.js';
import { kernelForSubgrid } from './kernel.js';
import { go } from './shell.js';

const t = text => language() === 'en' ? translateZh(text) : text;
const EXAMPLES = [
  { name: 'CN-Cng', title: '草地 · CN-Cng', detail: '基础水热过程', config: { subgrid: 'IGBP', physics: {} } },
  { name: 'AT-Neu', title: '甲烷 · AT-Neu', detail: '草地甲烷过程；运行前需补充过程数据', config: { subgrid: 'PFT', physics: { bgc: true, tracer: true }, tracer: 'methane', methaneMode: 'wetwat' } },
  { name: 'AU-Preston', title: '城市 · AU-Preston', detail: '城市冠层与人为热', config: { subgrid: 'IGBP', physics: { urban: true } } },
  { name: 'US-Ne3', title: '农田 · US-Ne3', detail: '作物生长与碳氮过程', config: { subgrid: 'PFT', physics: { bgc: true, crop: true } } },
];
const configFor = example => ({ domain: 'site', grid: null, spatial: null, soil: 'vg', ...example.config });
let pending = false;
const recentButtons = new Map();

function report(error) {
  const message = String(error?.message || error);
  $('home-status').textContent = message;
  status(message);
}

function refreshControls() {
  $('launchgate').setAttribute('aria-busy', String(pending));
  for (const id of ['home-new-case', 'home-show-examples', 'home-resume']) $(id).disabled = pending;
  $('home-open-case').disabled = pending || !hasBackend;
  $('home-resume').hidden = !state.wizard;
  for (const button of recentButtons.values()) button.disabled = pending || !hasBackend;
  for (const example of EXAMPLES) {
    const button = $(`home-example-${example.name}`);
    if (button) button.disabled = pending || !hasBackend || !kernelForSubgrid(example.config.subgrid, configFor(example));
  }
}

async function homeOperation(message, work) {
  if (pending) throw new Error(t('正在处理首页操作，请等待完成'));
  pending = true;
  $('home-status').textContent = t(message);
  refreshControls();
  try {
    const result = await work();
    $('home-status').textContent = '';
    return result;
  } catch (error) {
    report(error);
    throw error;
  } finally {
    pending = false;
    refreshControls();
  }
}

function newCase() {
  if (pending) throw new Error(t('正在处理首页操作，请等待完成'));
  $('launchgate').hidden = true;
  showDomainGate();
}

function resume() {
  if (pending || !state.wizard) throw new Error(t('当前没有可继续的任务'));
  $('launchgate').hidden = true;
}

function showExamples() {
  if (pending) throw new Error(t('正在处理首页操作，请等待完成'));
  $('home-examples').hidden = !$('home-examples').hidden;
  $('home-show-examples').setAttribute('aria-expanded', String(!$('home-examples').hidden));
}

function openCase(chosen) {
  return homeOperation('正在打开算例…', async () => {
    const dir = typeof chosen === 'string' ? chosen : await invoke('pick_folder', { key: 'open-case' });
    if (!dir) return null;
    return openExistingCase(dir, 'basic-files', { strict: true });
  });
}

function prepareExample(example) {
  return homeOperation('正在准备示例…', async () => {
    const config = configFor(example);
    if (!kernelForSubgrid(config.subgrid, config)) throw new Error(t('缺少所需内核，暂时无法准备此示例'));
    startSession(config);
    const wizard = state.wizard;
    go('basic-files');
    await loadExample();
    if (state.wizard !== wizard) throw new Error(t('示例加载期间工作流已变化，请重新选择示例'));
    const site = sitesForWizard().find(item => item.name === example.name);
    if (!site) throw new Error(t('示例目录没有找到所选站点'));
    pickSite(site);
    $('launchgate').hidden = true;
    status(t('示例已准备，可核对输入后创建算例；尚未开始计算。'));
    return { site: site.name, step: state.step, started: false };
  });
}

function node(tag, className, text) {
  const element = document.createElement(tag);
  element.className = className;
  if (text) element.textContent = t(text);
  return element;
}

function renderHome() {
  const examples = $('home-example-list');
  examples.replaceChildren();
  for (const example of EXAMPLES) {
    const button = node('button', 'domain-card home-example');
    button.type = 'button';
    button.id = `home-example-${example.name}`;
    button.append(node('b', 'dt', example.title), node('span', 'dd', example.detail));
    if (!kernelForSubgrid(example.config.subgrid, configFor(example))) button.append(node('span', 'dwhy', '缺少所需内核，暂时无法准备此示例'));
    button.onclick = () => prepareExample(example).catch(() => {});
    examples.append(button);
  }
  const recent = $('home-recent-list');
  recent.replaceChildren();
  recentButtons.clear();
  // shortcut: 只展示上次项目的前六个算例；全局最近历史另行扩展。
  for (const entry of state.cases.slice(0, 6)) {
    const button = node('button', 'home-recent-case');
    button.type = 'button';
    button.title = entry.dir;
    const name = node('b', 'no-i18n'); name.textContent = entry.name;
    const path = node('span', 'mini muted no-i18n'); path.textContent = entry.dir;
    button.append(name, path);
    button.onclick = () => openCase(entry.dir).catch(() => {});
    recentButtons.set(entry.dir, button);
    recent.append(button);
  }
  $('home-recent-root').textContent = $('root').value.trim();
  $('home-recent-empty').hidden = recentButtons.size > 0;
  refreshControls();
}

export function showHome() {
  $('loadinggate').hidden = true;
  $('domaingate').hidden = true;
  $('launchgate').hidden = false;
  renderHome();
  if (!hasBackend) $('home-status').textContent = t('请在桌面软件中打开算例或加载示例。');
}

export const homeGuide = {
  openCase,
  fields: () => [],
  state: () => ({ busy: pending, examples_visible: !$('home-examples').hidden }),
  actions() {
    const action = (element, run, kind = 'click') => ({ element, run, kind });
    return {
      'new-case': action($('home-new-case'), newCase),
      'open-existing-case': action($('home-open-case'), () => openCase()),
      'show-examples': action($('home-show-examples'), showExamples),
      'resume-task': action($('home-resume'), resume),
      ...Object.fromEntries(EXAMPLES.map(example => [`example:${example.name}`, action($(`home-example-${example.name}`), () => prepareExample(example), 'commit')])),
      ...Object.fromEntries([...recentButtons].map(([dir, element]) => [`recent:${dir}`, action(element, () => openCase(dir))])),
    };
  },
};

$('home-new-case').onclick = newCase;
$('home-open-case').onclick = () => openCase().catch(() => {});
$('home-show-examples').onclick = showExamples;
$('home-resume').onclick = resume;
$('homeBtn').onclick = showHome;
addEventListener('colm:home', showHome);
for (const event of ['colm:kernels', 'colm:language']) addEventListener(event, () => {
  if (!$('launchgate').hidden) renderHome();
});
