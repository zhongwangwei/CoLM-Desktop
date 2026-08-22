import { mkdtemp, readFile, writeFile, cp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const temp = await mkdtemp(join(tmpdir(), 'colm-i18n-'));
await cp(join(root, 'dist', 'app'), join(temp, 'app'), { recursive: true });
await writeFile(join(temp, 'package.json'), '{"type":"module"}\n');
const { translateZh } = await import(pathToFileURL(join(temp, 'app', 'i18n.js')).href);

if (translateZh('这次要跑什么？') !== 'What would you like to run?') {
  throw new Error('wizard title is not translated');
}
if (translateZh('版权所有：CoLM陆面模式开发团队，中山大学大气科学学院')
    !== 'Copyright: CoLM LSM Development Team, School of Atmospheric Sciences, SYSU') {
  throw new Error('copyright attribution is not translated');
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

const html = await readFile(join(root, 'dist', 'index.html'), 'utf8');
if ((html.match(/data-lang="zh"/g) ?? []).length !== 2
    || (html.match(/data-lang="en"/g) ?? []).length !== 2) {
  throw new Error('language switch must exist in both the header and first-card overlay');
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
