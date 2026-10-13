import { simulationFieldStep } from './simulation-model';
import type { Overlay } from '../../lib/types';
import type { SchemaField } from './case-model';
import type { Descriptor } from '../versions/model';
export type ModelParameter = Descriptor & {
  subgroup?: string | null;
  subgroup_zh?: string;
  subgroup_en?: string;
  default_provider?: string;
  source_location?: string;
  write_strategy?: string;
};
export const modelSettingGroups = [
  ['subgrid', '次网格与植被', 'Subgrid and vegetation'],
  ['soil', '土壤方案', 'Soil schemes'],
  ['water', '水热过程', 'Hydrothermal processes'],
  ['ecology', '生态与生地化', 'Ecology and biogeochemistry'],
  ['tracer', '示踪剂', 'Tracers'],
  ['urban', '城市', 'Urban'],
  ['river', '河湖流与水库', 'River–lake flow and reservoirs'],
  ['assimilation', '数据同化', 'Data assimilation'],
  ['other', '其他模型选项', 'Other model options'],
] as const;
export const basicModelFields = [
  [
    'DEF_USE_Campbell_SOIL_MODEL',
    'Campbell 土壤模型（关闭为 vG）',
    'Campbell soil model (off means vG)',
  ],
  ['DEF_USE_VariablySaturatedFlow', '变饱和流', 'Variably saturated flow'],
  ['DEF_USE_GridRiverLakeFlow', '河湖汇流', 'River-lake routing'],
  ['DEF_USE_BGC', '碳氮生地化 BGC', 'Biogeochemistry BGC'],
  ['DEF_URBAN_RUN', '城市过程', 'Urban processes'],
  ['DEF_USE_TRACER', '示踪过程', 'Tracer processes'],
  ['DEF_TRACER_NAMES', '示踪物名称（含 CH4）', 'Tracer names (including CH4)'],
  ['DEF_TRACER_TYPES', '示踪物类型', 'Tracer types'],
] as const;
export function modelSettingGroup(field: Pick<SchemaField, 'name' | 'section'>): string {
  const name = field.name.toUpperCase();
  if (
    [
      'DEF_USE_LCT',
      'DEF_USE_PFT',
      'DEF_USE_PC',
      'DEF_SOLO_PFT',
      'DEF_FAST_PC',
      'DEF_PC_CROP_SPLIT',
      'DEF_SUBGRID_SCHEME',
      'DEF_USE_LULCC',
      'DEF_LULCC_SCHEME',
    ].includes(name)
  )
    return 'subgrid';
  if (
    [
      'DEF_USE_CAMPBELL_SOIL_MODEL',
      'DEF_USE_VARIABLYSATURATEDFLOW',
      'DEF_SOILPAR_UPS_FIT',
      'DEF_SOIL_REFL_SCHEME',
    ].includes(name)
  )
    return 'soil';
  return (
    (
      {
        水热过程: 'water',
        生态与生地化: 'ecology',
        示踪剂: 'tracer',
        城市: 'urban',
        河道与水库: 'river',
        数据同化: 'assimilation',
      } as Record<string, string>
    )[field.section] ?? 'other'
  );
}
export function modelSettingValue(overlay: Overlay, name: string): string | null {
  const scalar = overlay.parameters.find(
    (p) =>
      p.scope_instance.kind === 'case-scalar' && p.raw_key.toLowerCase() === name.toLowerCase(),
  );
  if (scalar) return scalar.value;
  return (
    Object.entries(overlay.physics).find(
      ([key]) => key.toLowerCase() === name.toLowerCase(),
    )?.[1] ?? null
  );
}
export function modelParameterEditable(
  field: ModelParameter,
  modelFields: readonly Pick<SchemaField, 'name'>[],
): boolean {
  return (
    field.raw_key.toUpperCase() !== 'DEF_SUBGRID_SCHEME' &&
    field.storage !== 'read-only' &&
    !['read-only-context', 'blocked-pending-hook', 'excluded-internal'].includes(
      field.visibility,
    ) &&
    (field.storage !== 'case-nml' ||
      modelFields.some((item) => item.name.toLowerCase() === field.raw_key.toLowerCase()))
  );
}
export function filterModelParameters(
  fields: ModelParameter[],
  query: string,
  scope: string,
): ModelParameter[] {
  const q = query.trim().toLowerCase();
  return fields.filter(
    (field) =>
      (!scope || field.scope === scope) &&
      (!q ||
        [
          field.id,
          field.raw_key,
          field.label_zh,
          field.label_en,
          field.section,
          field.subgroup_zh,
          field.subgroup_en,
        ]
          .join(' ')
          .toLowerCase()
          .includes(q)),
  );
}

export type SimulationMode = 'IGBP' | 'USGS' | 'PFT' | 'PC';
export function simulationMode(
  _overlay?: Overlay,
  classification?: 'IGBP' | 'USGS',
  subgrid?: string,
): SimulationMode | undefined {
  const actual = subgrid?.toUpperCase();
  return actual === 'PFT' || actual === 'PC'
    ? actual
    : (classification ?? (actual === 'IGBP' || actual === 'USGS' ? actual : undefined));
}
export function caseSchemeFields(mode: SimulationMode): Record<string, string> {
  return {
    DEF_USE_LCT: mode === 'IGBP' || mode === 'USGS' ? '.true.' : '.false.',
    DEF_USE_PFT: mode === 'PFT' ? '.true.' : '.false.',
    DEF_USE_PC: mode === 'PC' ? '.true.' : '.false.',
    DEF_SUBGRID_SCHEME: mode,
  };
}

// Follow the backend's parent metadata before falling back to existing schema sections.
export function processSettingOwner(
  field: Pick<SchemaField, 'name' | 'section'>,
  context: readonly { name: string; parent_gate?: string | null }[] = [],
): string | null {
  if (simulationFieldStep(field) === 4) return null;
  const roots: Record<string, string> = {
    DEF_USE_BGC: 'DEF_USE_BGC',
    DEF_USE_TRACER: 'DEF_USE_TRACER',
    DEF_URBAN_RUN: 'DEF_URBAN_RUN',
    DEF_USE_CROP: 'crop',
    DEF_USE_GRIDRIVERLAKEFLOW: 'DEF_USE_GridRiverLakeFlow',
    DEF_UNITCATCHMENT_REGIONAL: 'DEF_UnitCatchment_regional',
    DEF_USE_LULCC: 'DEF_USE_LULCC',
    DEF_USE_VARIABLYSATURATEDFLOW: 'DEF_USE_VariablySaturatedFlow',
  };
  let name = field.name.toUpperCase();
  const seen = new Set<string>();
  while (!seen.has(name)) {
    if (roots[name]) return roots[name];
    seen.add(name);
    const parent = context.find((entry) => entry.name.toUpperCase() === name)?.parent_gate;
    if (!parent) break;
    name = parent.toUpperCase();
  }
  if (/CROP|FERT|IRRIGATION|CNSOYFIXN/.test(field.name.toUpperCase())) return 'crop';
  if (/^DEF_TRACER_|^DEF_FILE_GIEMS$/.test(field.name.toUpperCase())) return 'DEF_USE_TRACER';
  if (field.name.toUpperCase().includes('URBAN')) return 'DEF_URBAN_RUN';
  if (
    [
      'DEF_USE_CN_INIT',
      'DEF_FILE_CN_INIT',
      'DEF_USE_LAIFEEDBACK',
      'DEF_USE_NOSTRESSNITROGEN',
      'DEF_USE_SASU',
      'DEF_USE_DIAGMATRIX',
      'DEF_USE_PN',
      'DEF_USE_NITRIF',
      'DEF_USE_FIRE',
    ].includes(field.name.toUpperCase()) ||
    /NDEP|^DEF_USE_NITRIF/.test(field.name.toUpperCase())
  )
    return 'DEF_USE_BGC';
  if (field.name === 'DEF_LULCC_SCHEME') return 'DEF_USE_LULCC';
  return (
    (
      {
        tracer: 'DEF_USE_TRACER',
        urban: 'DEF_URBAN_RUN',
        river: 'DEF_USE_GridRiverLakeFlow',
        soil: 'soil',
      } as Record<string, string>
    )[modelSettingGroup(field)] ??
    (/NDEP|DEF_USE_CN_INIT|DEF_file_cn_init/.test(field.name) ? 'DEF_USE_BGC' : null)
  );
}

export function simulationOptionField(
  field: Pick<SchemaField, 'name' | 'section'>,
  context: readonly { name: string; parent_gate?: string | null }[] = [],
): boolean {
  if (simulationFieldStep(field) === 4) return false;
  if (processSettingOwner(field, context)) return true;
  if (['DEF_LC_YEAR', 'DEF_CO2SCENARIO'].includes(field.name)) return false;
  return ['soil', 'water', 'ecology', 'subgrid', 'assimilation'].includes(modelSettingGroup(field));
}
