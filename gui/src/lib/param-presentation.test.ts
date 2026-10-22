import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fieldLabel, withCatalogLabels } from './param-presentation';

interface CatalogItem {
  raw_key: string;
  storage?: string;
  section?: string;
}

test('catalog entries get the named labels instead of the split CoLM key', () => {
  const [global, landCover, pft, context, unknown] = withCatalogLabels([
    {
      raw_key: 'DEF_TUNING_CAPR',
      scope: 'case-scalar',
      label_zh: 'DEF TUNING CAPR',
      label_en: 'DEF TUNING CAPR',
    },
    { raw_key: 'DEF_LC_BETA', scope: 'land-cover-class', label_zh: '根', label_en: 'DEF LC BETA' },
    { raw_key: 'DEF_PFT_VMAX25', scope: 'pft-type', label_zh: '最大羧化', label_en: 'Vmax' },
    // 研究的参数行只有 name，没有 raw_key。
    { name: 'DEF_PH_K_AXS', scope: 'case-scalar', label_zh: 'DEF PH K AXS', label_en: '' },
    { raw_key: 'DEF_NEW_KNOB', scope: 'case-scalar', label_zh: 'DEF NEW KNOB', label_en: '' },
  ]);
  assert.equal(global!.label_zh, '首层土温到地表温度调节因子');
  assert.equal(global!.label_en, 'First-layer-to-surface temperature tuning factor');
  assert.equal(landCover!.label_zh, '根系分布形状参数');
  assert.equal(landCover!.label_en, 'Root-profile shape parameter');
  // PFT 的目录名是 colm-case 里写好的正式名字，保持不变。
  assert.deepEqual([pft!.label_zh, pft!.label_en], ['最大羧化', 'Vmax']);
  assert.equal(context!.label_zh, '根轴向水力导度系数');
  assert.equal(unknown!.label_zh, 'NEW_KNOB');
  assert.equal(unknown!.raw_key, 'DEF_NEW_KNOB');
});

// 原来由旧界面的 gui/tests/labels.mjs 守着；旧界面删掉后搬到这里（第 733 轮）。
test('every catalog field shown on the settings or expert pages has Chinese and English names', () => {
  const catalog = JSON.parse(
    readFileSync(
      new URL('../../../artifacts/parameter-audit/catalog.json', import.meta.url),
      'utf8',
    ),
  ) as { parameters?: CatalogItem[] } | CatalogItem[];
  const items = Array.isArray(catalog) ? catalog : (catalog.parameters ?? []);
  const keys = [
    ...new Set(
      items
        .filter(
          (item) =>
            (item.storage === 'case-nml' && item.section !== '输出变量') ||
            item.storage === 'process-parameter-file',
        )
        .map((item) => item.raw_key),
    ),
  ];
  assert.ok(keys.length > 100, `only ${keys.length} catalog fields found`);
  const missing = keys.filter((key) => {
    const zh = fieldLabel(key, 'zh');
    const en = fieldLabel(key, 'en');
    return zh === key || en === key || /[一-鿿]/.test(en) || zh.includes(' · ');
  });
  assert.deepEqual(missing, [], `fields without real labels:\n${missing.join('\n')}`);
});
