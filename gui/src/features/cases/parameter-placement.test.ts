import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import type { ModelParameter } from './model-settings';
import { parameterLocation } from './model-settings';

test('the full catalog covers every current schema variable and gives each record a settings location', () => {
  const catalog = JSON.parse(
    readFileSync(
      new URL('../../../../artifacts/parameter-audit/catalog.json', import.meta.url),
      'utf8',
    ),
  ) as (ModelParameter & { calibration_eligible: boolean })[];
  const schema = readFileSync(
    new URL('../../../../crates/colm-schema/src/generated.rs', import.meta.url),
    'utf8',
  );
  const names = [...schema.matchAll(/Field \{ name: "([^"]+)"/g)].map((match) => match[1]);
  const declared = [
    ...new Set(catalog.filter((p) => p.storage === 'case-nml').map((p) => p.raw_key)),
  ];
  assert.deepEqual(
    declared.sort(),
    names.sort(),
    'refresh the catalog inventory when schema changes',
  );
  const modelFields = catalog
    .filter(
      (p) =>
        p.storage === 'case-nml' &&
        (p.scope === 'land-cover-class' ||
          p.calibration_eligible ||
          (p.raw_key.startsWith('DEF_MT_') && p.value_kind === 'real') ||
          p.raw_key === 'DEF_GridRiverLake_FloodInfiltMax'),
    )
    .map((p) => ({ name: p.raw_key }));
  for (const parameter of catalog) {
    const location = parameterLocation(parameter, modelFields);
    assert.match(
      location,
      /^(运行环境与保存位置|范围与网格|数据与时段|模型结构与方案|检查与运行|模型版本|只读上下文)/,
      parameter.id,
    );
    if (parameter.scope !== 'case-scalar') assert.match(location, /模型版本/, parameter.id);
    if (
      [
        'DEF_nx_blocks',
        'DEF_ny_blocks',
        'DEF_PIO_groupsize',
        'DEF_nIO_eq_nBlock',
        'DEF_BlockInfoFile',
        'DEF_simulation_time%spinup_sec',
      ].includes(parameter.raw_key)
    )
      assert.match(location, /不单独编辑/, parameter.id);
    if (parameter.raw_key === 'DEF_dir_output') assert.match(location, /自动生成输出路径/);
    if (parameter.visibility === 'read-only-context')
      assert.match(location, /只读上下文/, parameter.id);
  }
});
