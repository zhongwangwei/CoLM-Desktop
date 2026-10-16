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
      'DEF_USE_SOILPAR_UPS_FIT',
      'DEF_RSS_SCHEME',
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
export function caseSchemeFields(mode?: SimulationMode): Record<string, string> {
  if (!mode) return {};
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
  if (simulationFieldStep(field) === 5) return null;
  const roots: Record<string, string> = {
    DEF_USE_SOILPAR_UPS_FIT: 'soil',
    DEF_RSS_SCHEME: 'soil',
    DEF_USE_BGC: 'DEF_USE_BGC',
    DEF_USE_TRACER: 'DEF_USE_TRACER',
    DEF_URBAN_RUN: 'DEF_URBAN_RUN',
    DEF_USE_CROP: 'crop',
    DEF_USE_GRIDRIVERLAKEFLOW: 'DEF_USE_GridRiverLakeFlow',
    DEF_UNITCATCHMENT_REGIONAL: 'DEF_UnitCatchment_regional',
    DEF_USE_LULCC: 'DEF_USE_LULCC',
    DEF_USE_VARIABLYSATURATEDFLOW: 'DEF_USE_VariablySaturatedFlow',
  };
  // 植被结构的专属子项归各自方案的弹窗，不随名字里的 CROP 落进作物设置。
  const scheme = schemeSettingOwner(field.name);
  if (scheme) return scheme;
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
  // 土壤反照率等通用土壤项仍留在通用设置里。
  return (
    (
      {
        tracer: 'DEF_USE_TRACER',
        urban: 'DEF_URBAN_RUN',
        river: 'DEF_USE_GridRiverLakeFlow',
      } as Record<string, string>
    )[modelSettingGroup(field)] ??
    (/NDEP|DEF_USE_CN_INIT|DEF_file_cn_init/.test(field.name) ? 'DEF_USE_BGC' : null)
  );
}

export function simulationOptionField(
  field: Pick<SchemaField, 'name' | 'section'>,
  context: readonly { name: string; parent_gate?: string | null }[] = [],
): boolean {
  if (simulationFieldStep(field) === 5) return false;
  if (processSettingOwner(field, context)) return true;
  if (['DEF_LC_YEAR', 'DEF_CO2SCENARIO'].includes(field.name)) return false;
  return ['soil', 'water', 'ecology', 'subgrid', 'assimilation'].includes(modelSettingGroup(field));
}

// 四种下垫面方案各自独有的二级输入。IGBP / USGS 只是分类选择，没有专属参数，点选即生效、不弹窗。
export const schemeSettingKeys: Record<'PFT' | 'PC', readonly string[]> = {
  PFT: ['DEF_SOLO_PFT'],
  PC: ['DEF_FAST_PC', 'DEF_PC_CROP_SPLIT'],
};
// SinglePoint 执行路径不读这两个字段（后端同样隐藏），单点算例不应为它们弹窗。
export const spatialOnlySchemeKeys = ['DEF_SOLO_PFT', 'DEF_FAST_PC'];
export const schemeFlagKeys = ['DEF_USE_LCT', 'DEF_USE_PFT', 'DEF_USE_PC', 'DEF_SUBGRID_SCHEME'];
export function schemeSettingOwner(name: string): 'PFT' | 'PC' | null {
  const key = name.toUpperCase();
  for (const scheme of ['PFT', 'PC'] as const)
    if (schemeSettingKeys[scheme].some((item) => item.toUpperCase() === key)) return scheme;
  return null;
}
type FieldState = {
  name: string;
  mode: string;
  pending: boolean;
  parent_gate?: string | null;
};
// 一个字段能否出现在专属弹窗里：查询结果明确判定不可用（且不是因为父开关尚未开启）才排除。
export function settingAvailable(name: string, states?: readonly FieldState[]): boolean {
  const state = states?.find((item) => item.name.toLowerCase() === name.toLowerCase());
  return !state || state.mode !== 'disabled' || state.pending || !!state.parent_gate;
}
export function schemeHasSettings(
  scheme: string | undefined,
  fields: readonly Pick<SchemaField, 'name'>[],
  domain?: string | null,
  states?: readonly FieldState[],
): boolean {
  if (scheme !== 'PFT' && scheme !== 'PC') return false;
  return schemeSettingKeys[scheme].some(
    (key) =>
      fields.some((field) => field.name === key) &&
      !(['site', 'single'].includes(domain ?? '') && spatialOnlySchemeKeys.includes(key)) &&
      settingAvailable(key, states),
  );
}

export type AutoClosed = {
  key: string;
  zh: string;
  en: string;
  reasonZh: string;
  reasonEn: string;
};
export type ExclusionContext = {
  subgrid?: string;
  classification?: string;
  domain?: string | null;
  grid?: string | null;
  // CROP 内核要求 BGC：这时不能靠自动关闭 BGC 来解决冲突，由卡片提示切换内核。
  compiledCrop?: boolean;
};
const processNames: Record<string, [string, string]> = {
  DEF_USE_BGC: ['生物地球化学（BGC）', 'Biogeochemistry (BGC)'],
  DEF_TUNING_CROP_PLANTING_DAY: ['作物过程', 'Crop processes'],
  DEF_USE_LULCC: ['土地覆盖变化', 'Land-cover change'],
  DEF_USE_GridRiverLakeFlow: ['河湖汇流', 'River-lake routing'],
  DEF_UnitCatchment_regional: ['区域单元汇流', 'Regional unit-catchment routing'],
  DEF_URBAN_RUN: ['城市过程', 'Urban processes'],
  DEF_USE_TRACER: ['示踪剂过程', 'Tracer processes'],
};
/**
 * 上游选择（方案、分类、范围、网格、土壤方案、父过程）让某个已开启的过程失去物理前提时，
 * 把它一并关闭，返回补全后的 patch 与被关闭的过程。成对互斥（单点城市↔BGC、BGC↔LULCC、
 * 城市↔示踪、LULCC↔区域汇流）以本次 patch 中的选择为准，关闭另一项。
 * 只处理物理互斥；内核能力、版本字段与待核对条件仍由卡片置灰说明，不擅自改配置。
 */
export function exclusionPatch(
  current: Readonly<Record<string, string>>,
  patch: Readonly<Record<string, string | null>>,
  context: ExclusionContext,
): { patch: Record<string, string | null>; closed: AutoClosed[] } {
  const out: Record<string, string | null> = { ...patch };
  const closed: AutoClosed[] = [];
  const read = (key: string) => {
    const hit = Object.keys(out).find((name) => name.toLowerCase() === key.toLowerCase());
    if (hit) return out[hit];
    const base = Object.keys(current).find((name) => name.toLowerCase() === key.toLowerCase());
    return base ? current[base] : undefined;
  };
  const on = (key: string) => read(key)?.trim().toLowerCase() === '.true.';
  const touched = (key: string) =>
    Object.keys(patch).some((name) => name.toLowerCase() === key.toLowerCase()) &&
    patch[Object.keys(patch).find((name) => name.toLowerCase() === key.toLowerCase())!] !== null;
  const scheme = on('DEF_USE_PFT')
    ? 'PFT'
    : on('DEF_USE_PC')
      ? 'PC'
      : read('DEF_USE_LCT')?.trim().toLowerCase() === '.true.'
        ? (context.classification ?? context.subgrid ?? 'IGBP')
        : context.subgrid?.toUpperCase();
  const classification =
    read('DEF_SUBGRID_SCHEME')?.toUpperCase() === 'USGS' ? 'USGS' : context.classification;
  const site = ['site', 'single'].includes(context.domain ?? '');
  const catchment = context.grid === 'catchment';
  const close = (key: string, zh: string, en: string) => {
    if (key === 'DEF_TUNING_CROP_PLANTING_DAY') {
      if (read(key) === undefined || read(key) === null) return false;
      out[key] = null;
    } else {
      if (!on(key)) return false;
      out[key] = '.false.';
    }
    const [nameZh, nameEn] = processNames[key];
    closed.push({ key, zh: nameZh, en: nameEn, reasonZh: zh, reasonEn: en });
    return true;
  };
  // 成对互斥：本次选择的一方保留，另一方关闭。
  const pair = (a: string, b: string, zh: string, en: string) => {
    if (!on(a) || !on(b)) return false;
    return touched(a) && !touched(b) ? close(b, zh, en) : close(a, zh, en);
  };
  for (let round = 0; round < 8; round++) {
    const changed = [
      !!scheme &&
        scheme !== 'PFT' &&
        scheme !== 'PC' &&
        close('DEF_USE_BGC', 'BGC 需要 PFT 或 PC 植被结构', 'BGC requires a PFT or PC structure'),
      !on('DEF_USE_BGC') &&
        !context.compiledCrop &&
        close('DEF_TUNING_CROP_PLANTING_DAY', '作物过程需要 BGC', 'Crop processes require BGC'),
      site &&
        pair(
          'DEF_URBAN_RUN',
          'DEF_USE_BGC',
          '单点城市过程与 BGC 互斥',
          'Single-point urban processes and BGC are mutually exclusive',
        ),
      pair(
        'DEF_USE_LULCC',
        'DEF_USE_BGC',
        '土地覆盖变化与 BGC 互斥',
        'Land-cover change and BGC are mutually exclusive',
      ),
      (site || catchment || classification === 'USGS') &&
        close(
          'DEF_USE_LULCC',
          site
            ? '土地覆盖变化需要空间模拟'
            : catchment
              ? '汇水区网格不支持土地覆盖变化'
              : 'USGS 分类不支持土地覆盖变化',
          site
            ? 'Land-cover change requires a spatial simulation'
            : catchment
              ? 'Catchment grids do not support land-cover change'
              : 'USGS does not support land-cover change',
        ),
      (site || catchment) &&
        close(
          'DEF_USE_GridRiverLakeFlow',
          site ? '河湖汇流需要空间模拟' : '汇水区网格不能启用网格河湖汇流',
          site
            ? 'River-lake routing requires a spatial simulation'
            : 'Grid routing is unavailable for catchments',
        ),
      (site || !on('DEF_USE_GridRiverLakeFlow')) &&
        close(
          'DEF_UnitCatchment_regional',
          site ? '区域单元汇流需要空间模拟' : '区域单元汇流需要河湖汇流',
          site
            ? 'Regional routing requires a spatial simulation'
            : 'Regional routing requires river-lake routing',
        ),
      pair(
        'DEF_UnitCatchment_regional',
        'DEF_USE_LULCC',
        '区域单元汇流与土地覆盖变化互斥',
        'Regional routing and land-cover change are mutually exclusive',
      ),
      catchment &&
        close(
          'DEF_URBAN_RUN',
          '汇水区网格不支持城市过程',
          'Catchment grids do not support urban processes',
        ),
      pair(
        'DEF_USE_TRACER',
        'DEF_URBAN_RUN',
        '示踪剂过程与城市过程互斥',
        'Tracer and urban processes are mutually exclusive',
      ),
      (catchment ||
        read('DEF_USE_Campbell_SOIL_MODEL')?.trim().toLowerCase() === '.true.' ||
        read('DEF_USE_VariablySaturatedFlow')?.trim().toLowerCase() === '.false.') &&
        close(
          'DEF_USE_TRACER',
          catchment ? '汇水区网格不支持示踪' : '示踪剂过程需要 van Genuchten 土壤方案与变饱和流',
          catchment
            ? 'Catchment grids do not support tracers'
            : 'Tracers require the van Genuchten soil scheme with variably saturated flow',
        ),
    ].some(Boolean);
    if (!changed) break;
  }
  return { patch: out, closed };
}
export function autoClosedText(closed: readonly AutoClosed[], en: boolean): string {
  return closed.length
    ? (en ? 'Automatically disabled: ' : '已自动关闭：') +
        closed
          .map((item) => (en ? `${item.en} (${item.reasonEn})` : `${item.zh}（${item.reasonZh}）`))
          .join(en ? '; ' : '；')
    : '';
}
