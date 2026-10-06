// 参数目录里会出现在设定页（case.nml，不含输出变量开关）与专家页（过程参数文件）的字段都要有
// 中英文名。没有名字时界面会退回显示 CoLM 原名——能用，但不该是常态，新字段要在
// param-presentation.js 的 LABELS 里补上。
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';

globalThis.document = {
  getElementById: () => null, querySelectorAll: () => [], addEventListener() {},
  documentElement: { lang: 'zh' },
};
globalThis.window = globalThis;
globalThis.localStorage = { getItem: () => null, setItem() {} };

const { fieldLabel } = await import('../dist/app/param-presentation.js');
const catalog = JSON.parse(readFileSync(
  new URL('../../artifacts/parameter-audit/catalog.json', import.meta.url), 'utf8'));
const items = Array.isArray(catalog) ? catalog : catalog.parameters;
const keys = [...new Set(items
  .filter(item => (item.storage === 'case-nml' && item.section !== '输出变量')
    || item.storage === 'process-parameter-file')
  .map(item => item.raw_key))];
const missing = keys.filter(key => {
  const zh = fieldLabel(key, 'zh');
  const en = fieldLabel(key, 'en');
  return zh === key || en === key || /[一-鿿]/.test(en) || zh.includes(' · ');
});
assert.deepEqual(missing, [], `fields without real labels:\n${missing.join('\n')}`);
console.log(`labels: ${keys.length} catalog fields have Chinese and English names`);
