import { bilingual } from '../../lib/bilingual';
import type { FrozenRevision, ParameterOverride } from '../../lib/types';
export type Descriptor = {
  id: string;
  raw_key: string;
  label_zh: string;
  label_en: string;
  scope: string;
  storage: string;
  section: string;
  value_kind: string;
  unit: string | null;
  visibility: string;
  activation: string[];
  default: string | null;
  doc_zh: string;
  doc_en: string;
  validation: { min: string | null; max: string | null; allowed_values: string[]; finite: boolean };
};
export type ModelField = {
  name: string;
  kind: string;
  default: string;
  values: string[];
  section: string;
  doc?: string;
  derived: boolean;
};
export function parameterKey(r: ParameterOverride) {
  const s = r.scope_instance;
  return JSON.stringify([
    r.parameter_id,
    s.scheme ?? null,
    s.index ?? null,
    s.process_file ?? null,
  ]);
}
export function upsertParameter(records: ParameterOverride[], record: ParameterOverride) {
  return [...records.filter((r) => parameterKey(r) !== parameterKey(record)), record];
}
export function makeRecord(
  d: Descriptor,
  value: string,
  index: number,
  processFile: string,
): ParameterOverride {
  if (!value.trim() || /[\r\n]/.test(value)) throw new Error('Parameter value required');
  const scope =
    d.storage === 'pft-override'
      ? 'pft'
      : d.storage === 'pc-pft-override'
        ? 'pc-pft'
        : d.scope === 'land-cover-class'
          ? 'land-cover-class'
          : d.storage === 'process-parameter-file'
            ? 'process-file'
            : 'case-scalar';
  const scheme = scope === 'land-cover-class' ? d.id.split(':')[1] : undefined;
  if (
    scope === 'land-cover-class' &&
    (!Number.isInteger(index) || index < 1 || index > (scheme === 'USGS' ? 24 : 17))
  )
    throw new Error('Invalid land class');
  if (
    (scope === 'pft' || scope === 'pc-pft') &&
    (!Number.isInteger(index) || index < 0 || index > 78)
  )
    throw new Error('Invalid PFT');
  if (scope === 'process-file' && (!processFile || processFile.split('/').some((p) => p === '..')))
    throw new Error('Process asset required');
  return {
    parameter_id: d.id,
    raw_key: d.raw_key,
    value,
    unit: d.unit,
    scope_instance: {
      kind: scope,
      ...(scheme ? { scheme } : {}),
      ...(['pft', 'pc-pft', 'land-cover-class'].includes(scope) ? { index } : {}),
      ...(scope === 'process-file' ? { process_file: processFile } : {}),
    },
  };
}
export function importParameters(
  text: string,
  catalog: Descriptor[],
  fields: ModelField[],
  assets: string[],
): ParameterOverride[] {
  const data = JSON.parse(text);
  const rows = Array.isArray(data) ? data : data.records;
  if (!Array.isArray(rows)) throw new Error('Expected records array');
  const seen = new Set<string>();
  return rows.map((raw: ParameterOverride) => {
    const descriptor = catalog.find((d) => d.id === raw.parameter_id);
    if (
      !descriptor ||
      descriptor.raw_key.toLowerCase() !== String(raw.raw_key).toLowerCase() ||
      descriptor.storage === 'read-only' ||
      (descriptor.storage === 'case-nml' &&
        !fields.some((f) => f.name.toLowerCase() === descriptor.raw_key.toLowerCase()))
    )
      throw new Error(`Not a model parameter: ${raw.parameter_id}`);
    const record = makeRecord(
      descriptor,
      raw.value,
      raw.scope_instance?.index ?? 0,
      raw.scope_instance?.process_file ?? '',
    );
    const a = record.scope_instance,
      b = raw.scope_instance;
    if (
      a.kind !== b.kind ||
      (a.scheme ?? null) !== (b.scheme ?? null) ||
      (a.index ?? null) !== (b.index ?? null) ||
      (a.process_file ?? null) !== (b.process_file ?? null)
    )
      throw new Error('Parameter scope mismatch');
    if (record.scope_instance.process_file && !assets.includes(record.scope_instance.process_file))
      throw new Error('Process asset missing');
    const key = parameterKey(record);
    if (seen.has(key)) throw new Error('Duplicate parameter');
    seen.add(key);
    return record;
  });
}
export function versionDepth(v: FrozenRevision, all: FrozenRevision[]) {
  let depth = 0,
    parent = v.parent;
  const seen = new Set([`${v.version.id}@${v.version.revision}`]);
  while (parent) {
    const key = `${parent.id}@${parent.revision}`;
    if (seen.has(key)) break;
    seen.add(key);
    depth++;
    parent =
      all.find((x) => x.version.id === parent!.id && x.version.revision === parent!.revision)
        ?.parent ?? null;
  }
  return depth;
}

export const foundationControls = [
  {
    id: 'soil',
    zh: '土壤水力方案',
    en: 'Soil hydraulic scheme',
    keys: ['DEF_USE_Campbell_SOIL_MODEL', 'DEF_USE_VariablySaturatedFlow'],
    choices: [
      ['Campbell', 'Campbell', 'Campbell'],
      ['vG', 'van Genuchten（启用变饱和流）', 'van Genuchten (enables variably saturated flow)'],
    ],
  },
  ...[
    ['DEF_USE_GridRiverLakeFlow', '河湖汇流', 'River-lake routing'],
    ['DEF_USE_BGC', '生物地球化学（BGC）', 'Biogeochemistry (BGC)'],
    ['DEF_URBAN_RUN', '城市过程', 'Urban processes'],
    ['DEF_USE_TRACER', '示踪剂过程', 'Tracer processes'],
  ].map(([id, zh, en]) => ({
    id,
    zh,
    en,
    keys: [id],
    choices: [
      ['.true.', '开启', 'On'],
      ['.false.', '关闭', 'Off'],
    ],
  })),
];
function physicsValue(physics: Record<string, string>, key: string) {
  return Object.entries(physics)
    .find(([k]) => k.toLowerCase() === key.toLowerCase())?.[1]
    ?.trim()
    .toLowerCase();
}
export function foundationValue(
  physics: Record<string, string>,
  id: string,
  parameters: ParameterOverride[] = [],
) {
  physics = {
    ...physics,
    ...Object.fromEntries(
      parameters
        .filter((record) => record.scope_instance.kind === 'case-scalar')
        .map((record) => [record.raw_key, record.value]),
    ),
  };
  if (id === 'subgrid') {
    const flags = ['LCT', 'PFT', 'PC'].map((name) => physicsValue(physics, `DEF_USE_${name}`));
    return flags.every((v) => v === '.true.' || v === '.false.') &&
      flags.filter((v) => v === '.true.').length === 1
      ? ['LCT', 'PFT', 'PC'][flags.indexOf('.true.')]
      : '';
  }
  if (id === 'soil') {
    const campbell = physicsValue(physics, 'DEF_USE_Campbell_SOIL_MODEL');
    return campbell === '.true.'
      ? 'Campbell'
      : campbell === '.false.' &&
          physicsValue(physics, 'DEF_USE_VariablySaturatedFlow') === '.true.'
        ? 'vG'
        : '';
  }
  const value = physicsValue(physics, id);
  return value === '.true.' || value === '.false.' ? value : '';
}
export function setFoundation(
  physics: Record<string, string>,
  parameters: ParameterOverride[],
  fields: ModelField[],
  id: string,
  value: string,
  subgrid?: string,
) {
  if (id === 'subgrid')
    throw new Error(bilingual('植被方案属于算例', 'Vegetation scheme belongs to the case'));
  const control = foundationControls.find((c) => c.id === id);
  if (
    !control ||
    !control.choices.some((c) => c[0] === value) ||
    !control.keys.every((key) => fields.some((f) => f.name === key && !f.derived))
  )
    throw new Error(bilingual('该基础结构不可修改', 'Foundation control is not writable'));
  const patch: Record<string, string> =
    id === 'soil'
      ? {
          DEF_USE_Campbell_SOIL_MODEL: value === 'Campbell' ? '.true.' : '.false.',
          ...(value === 'vG' ? { DEF_USE_VariablySaturatedFlow: '.true.' } : {}),
        }
      : { [id]: value };
  if (
    parameters.some((r) =>
      Object.keys(patch).some((k) => k.toLowerCase() === r.raw_key.toLowerCase()),
    )
  )
    throw new Error(
      bilingual('请先移除同名参数覆盖', 'Remove the conflicting parameter override first'),
    );
  if (id === 'DEF_USE_BGC' && value === '.true.' && !['PFT', 'PC'].includes(subgrid ?? ''))
    throw new Error(
      bilingual('请先明确选择 PFT 或 PC，再开启 BGC', 'Select PFT or PC before enabling BGC'),
    );
  const next = { ...physics };
  for (const [key, v] of Object.entries(patch)) {
    for (const old of Object.keys(next))
      if (old.toLowerCase() === key.toLowerCase()) delete next[old];
    next[key] = v;
  }
  return next;
}
