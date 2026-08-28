import { mkdtemp, readFile, writeFile, cp, readdir } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-i18n-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const { translateZh } = await import(pathToFileURL(join(temp, 'app', 'i18n.js')).href);

const CHINESE = /[\u3400-\u9fff]/;
const JS_STRING = /(?:'([^'\\]*(?:\\.[^'\\]*)*)'|"([^"\\]*(?:\\.[^"\\]*)*)"|`([^`\\]*(?:\\.[^`\\]*)*)`)/gs;
const SCAN_EXEMPT_FILES = new Map([
  // i18n.js is the dictionary/regex implementation itself, not source UI copy.
  ['i18n.js', 'translation table and translation regexes'],
  // param-presentation.js is a bilingual field catalog; its public API selects
  // English directly via fieldLabel/optionLabel and is covered by params.mjs.
  ['param-presentation.js', 'bilingual catalog with its own locale-aware accessors'],
]);

function decodeJsLiteral(text) {
  return text
    .replace(/\\n/g, '\n')
    .replace(/\\t/g, '\t')
    .replace(/\\'/g, "'")
    .replace(/\\"/g, '"')
    .replace(/\\`/g, '`')
    .replace(/\\\\/g, '\\');
}

function interpolationPlaceholder(expr) {
  const e = expr.toLowerCase();
  if (/城市.*自然|自然.*城市/.test(expr)) return '自然';
  if (/confidence/.test(e)) return '高';
  if (/solar_noon|solarnoon/.test(e)) return '3';
  if (/solar/.test(e)) return '12.00 UTC';
  if (/format/.test(e)) return 'PDF';
  if (/count|length|len|size|total|index|step|row|point|cpu|job|member|candidate|repeat|page|year|month|day|width|finished|failed|unresolved|completed|written|bad|files|variables|sites|tasks|missing|runs|code|n\b|\bi\b|\d/.test(e)) return '3';
  if (e.trim() === 'stage') return 'colm';
  if (/date|time|from|to|start|end/.test(e)) return '2008-01-01';
  return 'NAME';
}

function substituteInterpolations(text) {
  return text
    .replace(/\$\{([^{}]*)\}/g, (_, expr) => interpolationPlaceholder(expr))
    .replace(/\$\{([^}]*)$/g, (_, expr) => interpolationPlaceholder(expr));
}

function splitVisibleChunks(text) {
  if (!text.trim()) return [];
  text = substituteInterpolations(text);
  const chunks = [];
  if (text.includes('<')) {
    const attrs = text.matchAll(/(?:title|placeholder|aria-label|value)=["']([^"']*?[\u3400-\u9fff][^"']*)["']/g);
    for (const match of attrs) chunks.push(match[1].replace(/\s+/g, ' ').trim());
    text = text
      .replace(/<!--[\s\S]*?-->/g, '')
      .replace(/<script[\s\S]*?<\/script>/g, '')
      .replace(/<style[\s\S]*?<\/style>/g, '')
      .replace(/<[^>]*>/g, '\n');
  }
  chunks.push(...text.split(/\n+/).map(part => part.replace(/\s+/g, ' ').trim()));
  return chunks.filter(Boolean);
}

async function assertAppStringsTranslated() {
  const appDir = join(root, 'dist', 'app');
  const files = (await readdir(appDir))
    .filter(file => file.endsWith('.js') && !SCAN_EXEMPT_FILES.has(file))
    .sort();
  const missing = [];
  for (const file of files) {
    const source = await readFile(join(appDir, file), 'utf8');
    for (const match of source.matchAll(JS_STRING)) {
      const literal = decodeJsLiteral(match[1] ?? match[2] ?? match[3] ?? '');
      for (const chunk of splitVisibleChunks(literal)) {
        if (!CHINESE.test(chunk)) continue;
        const translated = translateZh(chunk);
        if (CHINESE.test(translated)) missing.push(`${file}: ${chunk}`);
      }
    }
  }
  if (missing.length) {
    throw new Error(`app/*.js UI strings have no complete English translation:\n${missing.join('\n')}`);
  }
}

if (translateZh('这次要跑什么？') !== 'What would you like to run?') {
  throw new Error('wizard title is not translated');
}
if (translateZh('选择运行方式') !== 'Choose run mode'
    || translateZh('服务器运行') !== 'Server run') {
  throw new Error('launch-mode cards are not translated');
}
if (translateZh('版权所有：CoLM陆面模式开发团队，中山大学大气科学学院')
    !== 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU') {
  throw new Error('copyright attribution is not translated');
}
if (translateZh('开发与维护团队') !== 'Development and maintenance team'
    || translateZh('联系人') !== 'Contact') {
  throw new Error('about attribution is not translated');
}
if (translateZh('下一步：站点信息 →') !== 'Next: Site information →') {
  throw new Error('dynamic workflow navigation is not translated');
}
if (translateZh('第 12/48 步 · 2008-01-01') !== 'Step 12/48 · 2008-01-01') {
  throw new Error('dynamic per-site progress is not translated');
}
if (translateZh('第 2 步') !== 'Step 2') {
  throw new Error('workflow step labels are not translated');
}
if (translateZh('站点、路径、预热和建例所需的数据设置都收在这里；')
    !== 'Site, path, spin-up, and case-creation data settings are collected here;'
    || translateZh('过程参数留在下一步，避免同一个字段出现两次。')
      !== 'process parameters stay in the next step so each field appears only once.') {
  throw new Error('basic-setup prose must not become mixed Chinese and English');
}
if (translateZh('检测到 8 个逻辑 CPU；单个站点仍使用 1 核。')
    !== '8 logical CPUs detected; each site still uses one core.') {
  throw new Error('dynamic CPU guidance is not translated');
}
if (translateZh('参数调优任务已生成。') !== 'Parameter-tuning task prepared.'
    || translateZh('Qle 的权重必须是正数。') !== 'Qle weight must be positive.'
    || translateZh('请先生成分析任务。') !== 'Prepare an analysis task first.'
    || translateZh('没有符合筛选条件的日志。') !== 'No log entries match the filters.') {
  throw new Error('dynamic Study workflow content is not fully translated');
}
if (translateZh('这 4 个算例的预热设置不一致')
      !== '4 cases have different spin-up settings') {
  throw new Error('multi-site spin-up guidance is not translated');
}
if (translateZh('预热：每轮 2 年，共重复 3 轮')
    !== 'Spin-up: 2 years per cycle, 3 cycles') {
  throw new Error('spin-up status must describe both values without a multiplication symbol');
}
if (translateZh('16 个 history 文件 · 105120 步 · 2003/08/01 至 2004/11/30 · 245 个变量')
      !== '16 history files · 105120 steps · 2003/08/01 to 2004/11/30 · 245 variables'
    || translateZh('AU-Preston · 净辐射 Rnet · 2400/105120 点')
      !== 'AU-Preston · Net radiation Rnet · 2400/105120 points'
    || translateZh('评估分析范围内的 4 个站点')
      !== 'Evaluate 4 sites in the analysis scope'
    || translateZh('批量评估完成：3/4 个站点有结果')
      !== 'Batch evaluation complete: 3/4 sites produced results') {
  throw new Error('dynamic results-workbench content is not fully translated');
}
if (translateZh('用户自定义站点说明') !== '用户自定义站点说明') {
  throw new Error('unknown user text must stay intact rather than become half translated');
}
if (translateZh('先确认槽位映射；缺少观测高度：V、T、Q；请选择站点数据产物目录')
    !== 'Confirm the slot mapping first; Missing observation heights: V、T、Q; Choose the site-data output directory'
    || translateZh('多个站点必须各自提供纬度列和经度列，不能共用一个回退坐标')
      !== 'Each site in a multi-site table must provide latitude and longitude columns; one fallback coordinate cannot be shared') {
  throw new Error('dynamic table-import readiness guidance is not fully translated');
}

for (const text of ['可独立运行', '结构字段', '有依据的查表值']) {
  if (/[㐀-鿿]/.test(translateZh(text))) {
    throw new Error(`known dynamic site-data label is not translated: ${text}`);
  }
}
await assertAppStringsTranslated();

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
if ((html.match(/data-lang="zh"/g) ?? []).length !== 3
    || (html.match(/data-lang="en"/g) ?? []).length !== 3) {
  throw new Error('language switch must exist in the header, launch page, and model wizard');
}
const serverCard = html.match(/<button class="domain-card launch-mode-card" id="serverRunCard"[\s\S]*?<\/button>/)?.[0] ?? '';
if (!serverCard.includes('disabled') || !serverCard.includes('aria-disabled="true"')
    || !serverCard.includes('暂未开放')) {
  throw new Error('server run must stay visible, disabled, and marked unavailable');
}
if (!html.includes('assets/colm-icon.png') || !html.includes('id="modeSeg"')) {
  throw new Error('project icon or expert-mode entry is missing');
}
const statusBar = html.match(/<footer class="status">([\s\S]*?)<\/footer>/)?.[1] ?? '';
const topBar = html.match(/<header class="top">([\s\S]*?)<\/header>/)?.[1] ?? '';
if (!statusBar.includes('id="copyright"')
    || !statusBar.includes('版权所有：CoLM陆面模式开发团队，中山大学大气科学学院')
    || topBar.includes('id="copyright"')) {
  throw new Error('the confirmed copyright attribution must be centered in the bottom status bar');
}

// Static pages are the stable contract of the language toggle. Split on tags so
// inline code/bold elements are checked as the same text nodes the browser sees.
const visible = html
  .replace(/<!--[\s\S]*?-->/g, '')
  .replace(/<script[\s\S]*?<\/script>/g, '')
  .replace(/<style[\s\S]*?<\/style>/g, '')
  .replace(/<[^>]*>/g, '\n')
  .split(/\n+/)
  .map(text => text.replace(/\s+/g, ' ').trim())
  .filter(Boolean);
for (const text of visible) {
  if (text === '中文') continue;
  if (/[\u3400-\u9fff]/.test(translateZh(text))) {
    throw new Error(`static UI text has no complete English translation: ${text}`);
  }
}
for (const match of html.matchAll(/(?:title|placeholder|aria-label|value)="([^"]*[\u3400-\u9fff][^"]*)"/g)) {
  if (match[1] !== '中文' && /[\u3400-\u9fff]/.test(translateZh(match[1]))) {
    throw new Error(`static UI attribute has no complete English translation: ${match[1]}`);
  }
}
const domain = await readFile(join(root, 'dist', 'app', 'domain.js'), 'utf8');
if (domain.includes('PLUMBER2 / Urban-PLUMBER 单点模拟') || !domain.includes('单点站点模拟')) {
  throw new Error('the first site card must use a dataset-neutral description');
}

console.log('i18n: header and first-page switches, project icon, and expert entry are present');
