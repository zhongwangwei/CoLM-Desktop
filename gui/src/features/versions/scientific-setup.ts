import { bilingual } from '../../lib/bilingual';
import type { KernelEntry, Overlay } from '../../lib/types';
import { foundationControls, foundationValue, setFoundation, type ModelField } from './model';
import { modelSettingValue } from '../cases/model-settings';
import { cropEnabled, effectiveModelFields } from '../cases/case-model';
import {
  resetModelOption,
  setModelOption,
  unquote,
  type ConfigurationField,
  type ConfigurationResult,
} from './configuration-options';
export type SetupContext = Pick<ConfigurationResult, 'fields' | 'process_files' | 'crop_defaults'>;
export type SetupRequest = { id: string; value: string };
export type SetupEnvironment = {
  engine?: string;
  subgrid?: string;
  caseValues?: Record<string, string>;
  domain?: string | null;
  grid?: string | null;
  classification?: 'IGBP' | 'USGS';
  kernel?: KernelEntry;
  cropKernel?: KernelEntry;
  // 算例页面会把被上游选择否定的过程自动关闭（见 exclusionPatch），这些选择因此不必先置灰。
  autoClose?: boolean;
};
const planting = 'DEF_TUNING_CROP_PLANTING_DAY';
const isSite = (environment: SetupEnvironment) =>
  ['single', 'site'].includes(environment.domain ?? '');
const extra = [
  { id: 'DEF_USE_VariablySaturatedFlow', zh: '变饱和流', en: 'Variably saturated flow' },
  { id: 'DEF_UnitCatchment_regional', zh: '区域单元汇流', en: 'Regional unit-catchment routing' },
  { id: 'DEF_USE_LULCC', zh: '土地覆盖变化', en: 'Land-cover change' },
  { id: 'crop', zh: '作物过程', en: 'Crop processes' },
].map((c) => ({
  ...c,
  keys: [c.id === 'crop' ? planting : c.id],
  choices: [
    ['.true.', '开启', 'On'],
    ['.false.', '关闭', 'Off'],
  ],
}));
// 上游把 DEF_USE_TRACER 拆成 isotope / methane / sediment 三个开关（溶质由名单非空启用），
// 卡片仍保留一个聚合入口：显示值 = 任一 family 开启；点击开只会打开示踪设置面板让人选 family。
const tracerKeys = ['DEF_USE_ISOTOPE', 'DEF_USE_METHANE', 'DEF_USE_SEDIMENT', 'DEF_SOLUTE_NAMES'];
const tracerControl = {
  id: 'tracer',
  zh: '示踪过程',
  en: 'Tracer processes',
  keys: tracerKeys,
  choices: [
    ['.true.', '选择示踪物', 'Choose tracers'],
    ['.false.', '关闭', 'Off'],
  ] as [string, string, string][],
};
const order = [
  'soil',
  'DEF_USE_VariablySaturatedFlow',
  'DEF_USE_GridRiverLakeFlow',
  'DEF_UnitCatchment_regional',
  'DEF_USE_LULCC',
  'DEF_USE_BGC',
  'DEF_URBAN_RUN',
  'crop',
  'tracer',
];
const controls = order.map((id) =>
  [...foundationControls, ...extra, tracerControl].find((c) => c.id === id)!,
);
export const setupHandledKeys = controls.flatMap((c) => c.keys);
export function setupChoices(
  overlay: Overlay,
  fields: ModelField[],
  environment: SetupEnvironment,
  queryFields: ConfigurationField[] = [],
  context?: SetupContext,
) {
  queryFields = context?.fields ?? queryFields;
  const value = (key: string) =>
    (
      modelSettingValue(overlay, key) ??
      queryFields.find((f) => f.name.toLowerCase() === key.toLowerCase())?.configured_value
    )
      ?.trim()
      .toLowerCase();
  const displayValue = (key: string) =>
    value(key) ??
    (
      queryFields.find((f) => f.name.toLowerCase() === key.toLowerCase())?.built_in_default ??
      fields.find((f) => f.name.toLowerCase() === key.toLowerCase())?.default
    )
      ?.trim()
      .toLowerCase();
  const on = (key: string) => value(key) === '.true.';
  const subgrid = environment.subgrid?.toUpperCase() ?? '',
    displayedSoil =
      displayValue('DEF_USE_Campbell_SOIL_MODEL') === '.true.'
        ? 'Campbell'
        : displayValue('DEF_USE_Campbell_SOIL_MODEL') === '.false.'
          ? 'vG'
          : '';
  const list = (key: string) =>
    unquote(value(key) ?? '')
      .split(',')
      .map((v) => unquote(v.trim()).toLowerCase())
      .filter(Boolean);
  // 聚合示踪状态：任一 family 开关开启，或溶质名单非空。
  const methane = on('DEF_USE_METHANE'),
    sediment = on('DEF_USE_SEDIMENT'),
    isotope = on('DEF_USE_ISOTOPE'),
    tracerOn = () => methane || sediment || isotope || list('DEF_SOLUTE_NAMES').length > 0;
  const activeFiles =
    context?.process_files.filter((f) => f.verified && f.active_methane_asset === true) ?? [];
  const methaneModes = activeFiles
    .map(
      (f) =>
        f.entries.find((e) => e.path.toLowerCase() === 'def_methane%inundation_mode' && !e.unset)
          ?.value,
    )
    .map((v) => (v ? unquote(v).toLowerCase() : null));
  const riverModes = ['routing', 'hybrid', 'dh_all_thr05', 'dyn_routing_hybrid'],
    nonRiverModes = ['wetwat', 'satellite', 'giems', 'dynamic_wtd', 'dynamic-wtd'];
  const methaneRiver = methaneModes.some((m) => m !== null && riverModes.includes(m));
  const methaneKnown =
    methaneModes.length > 0 &&
    methaneModes.every((m) => m !== null && [...riverModes, ...nonRiverModes].includes(m));
  const crop = cropEnabled(effectiveModelFields(overlay)),
    compiledCrop = environment.kernel?.macros.includes('CROP') === true;
  function reason(id: string, v: string): string {
    if (
      !controls
        .find((c) => c.id === id)!
        .keys.every((k) => fields.some((f) => f.name === k && !f.derived))
    )
      return bilingual('当前版本没有可写字段', 'No writable field in this version');
    if (tracerOn() && id !== 'tracer') {
      if (methane && id === 'DEF_USE_BGC' && v === '.false.')
        return bilingual(
          '请先关闭甲烷示踪或修改 CH4 定义',
          'Disable methane tracers or change the CH4 definition first',
        );
      if (isotope && id === 'DEF_USE_LULCC' && v === '.true.')
        return bilingual(
          '请先关闭同位素示踪或修改 isotope 类型',
          'Disable isotope tracers or change the isotope type first',
        );
      if (id === 'DEF_USE_GridRiverLakeFlow' && v === '.false.') {
        if (sediment)
          return bilingual(
            '请先关闭河道泥沙示踪或修改 SEDIMENT 定义',
            'Disable sediment tracers or change SEDIMENT first',
          );
        if (methane && methaneRiver)
          return bilingual(
            '请先修改甲烷 routing/hybrid 淹没方案或关闭示踪',
            'Change methane routing/hybrid inundation or disable tracers first',
          );
        if (methane && !methaneKnown)
          return bilingual(
            '待核对：当前活动甲烷文件及淹没方案尚未确认；可先关闭示踪',
            'Pending: active methane asset and inundation mode are unverified; disable tracers first',
          );
      }
    }
    if (
      on('DEF_GridRiverLake_FloodFeedback') &&
      ((id === 'DEF_URBAN_RUN' && v === '.true.') ||
        (id === 'DEF_USE_LULCC' && v === '.true.') ||
        (['DEF_USE_GridRiverLakeFlow', 'DEF_USE_VariablySaturatedFlow'].includes(id) &&
          v === '.false.'))
    )
      return bilingual('请先关闭漫滩回馈', 'Disable floodplain feedback first');
    if (v === '.false.' && id === 'DEF_USE_VariablySaturatedFlow' && displayedSoil === 'vG')
      return bilingual(
        'van Genuchten 必须开启变饱和流；请先改用 Campbell',
        'van Genuchten requires variably saturated flow; choose Campbell first',
      );
    if (
      v === '.false.' &&
      id === 'DEF_USE_BGC' &&
      (compiledCrop || (crop && !environment.autoClose))
    )
      return compiledCrop
        ? bilingual(
            '当前作物编译配置要求 BGC；请先切换非作物编译配置',
            'The crop build configuration requires BGC; switch to a non-crop build configuration first',
          )
        : bilingual('请先关闭作物过程', 'Disable crop first');
    if (
      v === '.false.' &&
      id === 'DEF_USE_VariablySaturatedFlow' &&
      tracerOn() &&
      !environment.autoClose
    )
      return bilingual('请先关闭示踪过程', 'Disable tracers first');
    if (v === '.false.') return '';
    if (id !== 'crop') {
      const blocked = (
        id === 'soil' ? ['DEF_USE_Campbell_SOIL_MODEL'] : controls.find((c) => c.id === id)!.keys
      )
        .map((k) => queryFields.find((f) => f.name === k))
        .find((f) => f && ['disabled', 'readonly', 'read-only'].includes(f.mode));
      if (blocked)
        return (
          blocked.reason ??
          bilingual(
            '当前源码或编译配置不支持修改',
            'Current source or build configuration disallows this option',
          )
        );
    }
    if (id === 'soil' && v === 'Campbell' && tracerOn() && !environment.autoClose)
      return bilingual('请先关闭示踪过程', 'Disable tracers first');
    if (
      isSite(environment) &&
      ((id === 'DEF_USE_BGC' && on('DEF_URBAN_RUN')) ||
        (id === 'DEF_URBAN_RUN' && on('DEF_USE_BGC')))
    )
      return bilingual(
        '单点城市过程与 BGC 不能同开；请先关闭另一项',
        'Single-point urban and BGC cannot coexist; disable the other first',
      );
    if (id === 'DEF_USE_BGC' || id === 'crop') {
      if (!['PFT', 'PC'].includes(subgrid))
        return bilingual('请先明确选择 PFT 或 PC', 'Select PFT or PC first');
      if (id === 'DEF_USE_BGC' && on('DEF_USE_LULCC'))
        return bilingual('请先关闭土地覆盖变化', 'Disable land-cover change first');
      if (id === 'crop') {
        if (!on('DEF_USE_BGC')) return bilingual('请先开启 BGC', 'Enable BGC first');
        if (!environment.cropKernel)
          return bilingual('待核对：缺少作物编译支持', 'Pending: crop build support unavailable');
        if (!environment.cropKernel.macros.includes('CROP'))
          return bilingual('当前编译配置不支持作物过程', 'Build configuration lacks crop support');
        if (!environment.domain)
          return bilingual('待核对：请先确定模拟范围', 'Pending: choose simulation domain');
      }
    }
    if (id === 'DEF_URBAN_RUN') {
      if (environment.grid === 'catchment' || environment.kernel?.macros.includes('CATCHMENT'))
        return bilingual(
          '流域网格不支持城市过程',
          'Catchment grid does not support urban processes',
        );
      if (
        !isSite(environment) &&
        !environment.grid &&
        !environment.kernel?.macros.some((m) =>
          ['GRIDBASED', 'UNSTRUCTURED', 'SinglePoint'].includes(m),
        )
      )
        return bilingual(
          '待核对：请先确定网格或编译配置',
          'Pending: choose a grid or build configuration',
        );
    }
    if (id === 'DEF_URBAN_RUN' && tracerOn())
      return bilingual('请先关闭示踪过程', 'Disable tracers first');
    if (id === 'tracer') {
      if (methane && (displayValue('DEF_USE_BGC') !== '.true.' || !['PFT', 'PC'].includes(subgrid)))
        return bilingual(
          '甲烷示踪需要先开启 BGC 并选择 PFT/PC',
          'Methane requires BGC and PFT/PC first',
        );
      if (isotope && displayValue('DEF_USE_LULCC') === '.true.')
        return bilingual(
          '请先关闭 LULCC 或修改 isotope 类型',
          'Disable LULCC or change isotope type first',
        );
      if (
        sediment &&
        (isSite(environment) ||
          displayValue('DEF_USE_GridRiverLakeFlow') !== '.true.' ||
          !environment.kernel?.macros.includes('GridRiverLakeFlow'))
      )
        return bilingual(
          'SEDIMENT 需要空间模式、河湖汇流编译支持并开启汇流',
          'SEDIMENT requires a spatial build configuration with routing support and routing enabled',
        );
      if (methane && methaneRiver && displayValue('DEF_USE_GridRiverLakeFlow') !== '.true.')
        return bilingual(
          '甲烷 routing/hybrid 需要先开启河湖汇流',
          'Methane routing/hybrid requires river-lake routing first',
        );
      if (displayedSoil !== 'vG')
        return bilingual(
          '请先选择 van Genuchten 和变饱和流',
          'Select van Genuchten and variably saturated flow',
        );
      if (displayValue('DEF_URBAN_RUN') !== '.false.')
        return bilingual('请先明确关闭城市过程', 'Explicitly disable urban processes first');
      if (!environment.grid && !isSite(environment))
        return bilingual('待核对：请先确定网格', 'Pending: choose grid');
      if (
        environment.engine === 'fortran' &&
        environment.kernel?.macros.some((m) => m.toUpperCase() === 'OPENMP')
      )
        return bilingual(
          'OpenMP 编译配置不支持示踪',
          'OpenMP build configurations do not support tracers',
        );
      if (environment.grid === 'catchment' || environment.kernel?.macros.includes('CATCHMENT'))
        return bilingual('流域网格不支持示踪', 'Catchment grid does not support tracers');
    }
    if (['DEF_USE_LULCC', 'DEF_USE_GridRiverLakeFlow', 'DEF_UnitCatchment_regional'].includes(id)) {
      if (isSite(environment))
        return id === 'DEF_USE_LULCC'
          ? bilingual(
              'SinglePoint 模型不支持土地覆盖变化，请使用空间模拟',
              'SinglePoint does not support land-cover change; use a spatial simulation',
            )
          : bilingual('需要空间模拟', 'Requires spatial simulation');
      if (!environment.domain || !environment.grid)
        return bilingual('待核对：请先确定模拟范围和网格', 'Pending: choose domain and grid');
      if (id === 'DEF_USE_GridRiverLakeFlow' || id === 'DEF_UnitCatchment_regional') {
        if (id === 'DEF_UnitCatchment_regional') {
          if (!on('DEF_USE_GridRiverLakeFlow'))
            return bilingual('请先开启河湖汇流', 'Enable river-lake routing first');
          if (value('DEF_USE_LULCC') !== '.false.')
            return bilingual(
              '请先明确关闭土地覆盖变化',
              'Explicitly disable land-cover change first',
            );
        }
        if (environment.grid === 'catchment')
          return bilingual(
            '流域网格不能启用网格河湖汇流',
            'Grid routing is unavailable for catchments',
          );
        if (!environment.kernel)
          return bilingual(
            '待核对：缺少匹配的编译配置',
            'Pending: matching build configuration unavailable',
          );
        if (!environment.kernel.macros.includes('GridRiverLakeFlow'))
          return bilingual(
            '当前编译配置不支持河湖汇流',
            'Build configuration lacks river-lake routing',
          );
      } else {
        if (!environment.classification)
          return bilingual('待核对：请先确定地表分类', 'Pending: choose classification');
        if (environment.grid === 'catchment')
          return bilingual(
            '流域网格不支持土地覆盖变化',
            'Catchment grid does not support land-cover change',
          );
        if (environment.classification === 'USGS')
          return bilingual('USGS 不支持土地覆盖变化', 'USGS does not support land-cover change');
        if (value('DEF_USE_BGC') !== '.false.')
          return bilingual('请先明确关闭 BGC', 'Explicitly disable BGC first');
      }
    }
    return '';
  }
  return controls
    .filter((c) => c.id !== 'DEF_UnitCatchment_regional')
    .map((c) => ({
      ...c,
      // 作物没有 namelist 开关（开不开由内核与种植日决定），未开启就是实际生效的“关闭”。
      // 原先返回空串，卡片永远停在“未指定”，点“关闭”也无处记录（第 692 轮原生界面实测）。
      value:
        c.id === 'crop'
          ? crop
            ? '.true.'
            : '.false.'
          : c.id === 'soil'
            ? displayedSoil
            : c.id === 'DEF_USE_VariablySaturatedFlow' && displayedSoil === 'vG'
              ? '.true.'
              : c.id === 'tracer'
                ? tracerOn()
                  ? '.true.'
                  : '.false.'
                : foundationValue({ [c.id]: displayValue(c.id) ?? '' }, c.id),
      inherited: c.id !== 'crop' && c.keys.every((key) => value(key) === undefined),
      pending: c.keys.some(
        (k) =>
          !queryFields.find((f) => f.name === k) || queryFields.find((f) => f.name === k)?.pending,
      ),
      choices: c.choices.map(([value, zh, en]) => ({ value, zh, en, reason: reason(c.id, value) })),
    }));
}
export function applySetupRequest(
  overlay: Overlay,
  fields: ModelField[],
  request: SetupRequest,
  environment: SetupEnvironment,
  context?: SetupContext,
): Overlay {
  const choice = setupChoices(overlay, fields, environment, context?.fields, context).find(
    (c) => c.id === request.id,
  );
  if (!choice) throw new Error('Unknown setup control');
  const value =
    request.id === 'crop' && !['.true.', '.false.'].includes(request.value)
      ? '.true.'
      : request.value;
  const candidate = choice.choices.find((c) => c.value === value);
  if (!candidate) throw new Error('Invalid setup choice');
  if (candidate.reason) throw new Error(candidate.reason);
  if (request.id === 'crop') {
    if (value === '.false.') return resetModelOption(overlay, planting);
    if (request.value === '.true.') {
      if (!context?.crop_defaults?.[planting])
        throw new Error(bilingual('作物默认配置尚未读取', 'Crop defaults have not loaded'));
      let next = overlay;
      for (const [key, defaultValue] of Object.entries(context.crop_defaults)) {
        const current = modelSettingValue(overlay, key);
        next = setModelOption(
          next,
          fields,
          key,
          key === planting && !isSite(environment) ? '0' : (current ?? defaultValue),
        );
      }
      return next;
    }
    if (
      !/^\d+$/.test(request.value) ||
      Number(request.value) > 366 ||
      (!isSite(environment) && Number(request.value) !== 0)
    )
      throw new Error(
        bilingual('单点种植日为 0–366；空间模拟只能为 0', 'Planting day: site 0–366, spatial 0'),
      );
    return setModelOption(overlay, fields, planting, request.value);
  }
  const foundation = foundationControls.find((c) => c.id === request.id);
  if (request.id === 'tracer' && !foundation) {
    // 聚合入口：关闭时三个 family 开关都写 .false.，并清空溶质名单；
    // 打开时由界面转到示踪设置面板选择 family，这里只在什么都没选时给同位素一个默认起点。
    if (value === '.false.') {
      let next = overlay;
      for (const key of ['DEF_USE_ISOTOPE', 'DEF_USE_METHANE', 'DEF_USE_SEDIMENT'])
        if (fields.some((field) => field.name === key && !field.derived))
          next = setModelOption(next, fields, key, '.false.');
      return resetModelOption(next, 'DEF_SOLUTE_NAMES');
    }
    const anyOn = ['DEF_USE_ISOTOPE', 'DEF_USE_METHANE', 'DEF_USE_SEDIMENT'].some(
      (key) => (modelSettingValue(overlay, key) ?? '').trim().toLowerCase() === '.true.',
    );
    return anyOn ? overlay : setModelOption(overlay, fields, 'DEF_USE_ISOTOPE', '.true.');
  }
  if (!foundation) {
    let next = setModelOption(overlay, fields, request.id, request.value);
    if (request.id === 'DEF_USE_LULCC' && request.value === '.true.') {
      for (const [key, value] of Object.entries({
        DEF_UnitCatchment_regional: '.false.',
        DEF_LAI_MONTHLY: '.true.',
        DEF_LAI_CHANGE_YEARLY: '.true.',
      }))
        if (fields.some((field) => field.name === key && !field.derived))
          next = setModelOption(next, fields, key, value);
    }
    return next;
  }
  // Migrate only the keys changed by this atomic control; unrelated scoped values stay intact.
  const keys = (
    request.id === 'soil' && request.value === 'Campbell'
      ? ['DEF_USE_Campbell_SOIL_MODEL']
      : foundation.keys
  ).map((k) => k.toLowerCase());
  const parameters = overlay.parameters.filter(
    (p) => p.scope_instance.kind !== 'case-scalar' || !keys.includes(p.raw_key.toLowerCase()),
  );
  const physics = { ...overlay.physics };
  for (const p of overlay.parameters)
    if (p.scope_instance.kind === 'case-scalar' && keys.includes(p.raw_key.toLowerCase()))
      physics[p.raw_key] = p.value;
  return {
    ...overlay,
    parameters,
    physics: setFoundation(
      physics,
      parameters,
      fields,
      request.id,
      request.value,
      environment.subgrid,
    ),
  };
}
