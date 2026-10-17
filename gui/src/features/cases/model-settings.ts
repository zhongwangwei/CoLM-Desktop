import {
  simulationFieldStep,
  isSiteConfiguration,
  isForcingScheme,
  legacyMpiFields,
} from './simulation-model';
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
  ['site', '单点数据来源', 'Single-point data sources'],
  ['forcing', '强迫场处理', 'Forcing processing'],
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
  if (isSiteConfiguration(field.name)) return 'site';
  if (isForcingScheme(field.name)) return 'forcing';
  if (name === 'DEF_VEG_SNOW') return 'water';
  if (
    /^DEF_LAI_/.test(name) ||
    ['DEF_LANDONLY', 'DEF_USE_DOMINANT_PATCHTYPE', 'DEF_HIGHRESVEG'].includes(name)
  )
    return 'subgrid';
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
      'DEF_HIGHRESSOIL',
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
  if (
    [1, 2, 5].includes(simulationFieldStep(field)) ||
    field.name.toUpperCase() === 'DEF_HIGHRESURBAN_ALBEDO'
  )
    return null;
  if (isSiteConfiguration(field.name)) return null;
  if (isForcingScheme(field.name)) return null;
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
  if (isSiteConfiguration(field.name) || isForcingScheme(field.name)) return false;
  const step = simulationFieldStep(field);
  if (step === 4) return true;
  if (step !== 3) return false;
  if (processSettingOwner(field, context)) return true;
  if (field.name === 'DEF_LC_YEAR') return false;
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

export function caseSimulationDomain(
  profile?: {
    grid?: string | null;
    domain_kind?: string | null;
    domain?: readonly number[] | null;
  } | null,
): string | null {
  if (!profile) return null;
  if (['region', 'watershed', 'global'].includes(profile.domain_kind ?? ''))
    return profile.domain_kind!;
  if (!profile.grid) return 'site';
  const bounds = profile.domain;
  return bounds?.length === 4 && bounds[1] - bounds[0] >= 360 && bounds[2] <= -90 && bounds[3] >= 90
    ? 'global'
    : 'region';
}
// case.nml 没写的开关按上游默认取值：DEF_USE_GridRiverLakeFlow 默认 .true.
// （MOD_Namelist.F90），缺省当成关会让流域算例退回全球单元汇水网络（round 696）。
const upstreamOnByDefault = ['DEF_USE_GridRiverLakeFlow'];
function flagOn(value: string | null | undefined, key: string): boolean {
  const text = value?.trim().toLowerCase();
  return text
    ? text === '.true.'
    : upstreamOnByDefault.some((name) => name.toLowerCase() === key.toLowerCase());
}
export function regionalRoutingPatch(
  values: Readonly<Record<string, string>>,
  domain?: string | null,
  grid?: string | null,
): Record<string, string> {
  if (!domain || !['site', 'single', 'region', 'regional', 'watershed', 'global'].includes(domain))
    return {};
  if (!grid && !['site', 'single'].includes(domain)) return {};
  const on = (key: string) =>
    flagOn(
      Object.entries(values).find(([name]) => name.toLowerCase() === key.toLowerCase())?.[1],
      key,
    );
  return {
    DEF_UnitCatchment_regional:
      !['site', 'single', 'global'].includes(domain) &&
      grid !== 'catchment' &&
      on('DEF_USE_GridRiverLakeFlow') &&
      !on('DEF_USE_LULCC')
        ? '.true.'
        : '.false.',
  };
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
      (site || !flagOn(read('DEF_USE_GridRiverLakeFlow'), 'DEF_USE_GridRiverLakeFlow')) &&
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
  const routing = regionalRoutingPatch(
    {
      DEF_USE_GridRiverLakeFlow: read('DEF_USE_GridRiverLakeFlow') ?? '',
      DEF_USE_LULCC: read('DEF_USE_LULCC') ?? '',
    },
    context.domain,
    context.grid,
  );
  for (const [key, value] of Object.entries(routing)) {
    if (read(key) !== value && (value === '.true.' || read(key) !== undefined)) out[key] = value;
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

export function parameterLocation(
  parameter: Descriptor,
  modelFields: readonly Pick<SchemaField, 'name'>[],
  en = false,
): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  if (parameter.visibility === 'read-only-context')
    return t('只读上下文（自动管理）', 'Read-only context (managed automatically)');
  if (parameter.scope === 'land-cover-class')
    return t('模型版本 → 地类参数', 'Model version → land-cover parameters');
  if (['pft-type', 'pc-pft-component'].includes(parameter.scope))
    return t('模型版本 → 按植物类型调整参数', 'Model version → parameters by plant type');
  if (parameter.storage === 'process-parameter-file')
    return t('模型版本 → 过程文件参数', 'Model version → process-file parameters');
  if (modelParameterEditable(parameter, modelFields))
    return t('模型版本 → 模型系数', 'Model version → model coefficients');
  if (legacyMpiFields.includes(parameter.raw_key))
    return t(
      '范围与网格 → 并行布局（向导不单独编辑）',
      'Range and grid → parallel layout (not edited in the wizard)',
    );
  if (parameter.raw_key === 'DEF_simulation_time%spinup_sec')
    return t(
      '数据与时段 → 随预热日期管理（不单独编辑）',
      'Data and period → managed with spin-up dates (not edited separately)',
    );
  if (parameter.raw_key === 'DEF_dir_output')
    return t('总的设定 → 自动生成输出路径', 'General setup → generated output directory');
  const field = { name: parameter.raw_key, section: parameter.section };
  const step = simulationOptionField(field) ? 4 : simulationFieldStep(field);
  const labels = [
    t('总的设定', 'General setup'),
    t('范围与网格', 'Range and grid'),
    t('数据与时段', 'Data and period'),
    t('模型结构与方案', 'Model structure and schemes'),
    t('检查与运行', 'Review and run'),
  ];
  const process = processSettingOwner(field);
  const owners: Record<string, string> = {
    DEF_USE_BGC: 'BGC',
    crop: t('作物设置', 'Crop settings'),
    DEF_URBAN_RUN: t('城市设置', 'Urban settings'),
    DEF_USE_TRACER: t('示踪设置', 'Tracer settings'),
    DEF_USE_LULCC: t('土地覆盖变化设置', 'Land-cover change settings'),
    DEF_USE_GridRiverLakeFlow: t('汇流设置（按网格类型）', 'Routing settings (by grid type)'),
    PFT: t('PFT 专属设置', 'PFT settings'),
    PC: t('PC 专属设置', 'PC settings'),
    DEF_USE_VariablySaturatedFlow: t('变饱和流设置', 'Variably saturated flow settings'),
  };
  if (isForcingScheme(field.name))
    return labels[2] + t(' → 更多配置 → 强迫场处理', ' → More settings → Forcing processing');
  if (isSiteConfiguration(field.name))
    return labels[2] + t(' → 更多配置 → 单点专属设置', ' → More settings → Single-point settings');
  return labels[step - 1] + (owners[process ?? ''] ? ' → ' + owners[process!] : '');
}

export function configurationReason(reason?: string | null): string | undefined {
  return reason === '适用条件待按所选源码、算例和内核核对；目录默认值仅供参考'
    ? undefined
    : (reason ?? undefined);
}
