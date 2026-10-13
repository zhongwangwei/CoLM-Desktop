import { bilingual } from '../../lib/bilingual';
import type { Overlay, VersionRef } from '../../lib/types';
import {
  foundationControls,
  makeRecord,
  parameterKey,
  upsertParameter,
  type Descriptor,
  type ModelField,
} from './model';
import { modelSettingValue } from '../cases/model-settings';
import { fortranNumberInputValue } from '../../lib/param-presentation';
export type ConfigurationSource =
  | { kind: 'frozen'; version: VersionRef }
  | { kind: 'draft'; id: string; generation: number; overlay: Overlay };
export type ConfigurationField = {
  name: string;
  mode: string;
  reason?: string | null;
  configured_value: string | null;
  allowed_values: string[];
  pending: boolean;
  built_in_default?: string | null;
};
export type ProcessEntry = {
  path: string;
  group: string;
  kind: string;
  value: string;
  asset_value: string | null;
  default: string;
  unset: boolean;
  doc?: string | null;
  options: string[];
  editable: boolean;
  descriptor_id: string | null;
  pending: boolean;
};
export type ConfigurationResult = {
  fields: ConfigurationField[];
  process_files: {
    file: string;
    title: string;
    section: string;
    verified: boolean;
    active_methane_asset: boolean | null;
    entries: ProcessEntry[];
  }[];
  pft_parameters: {
    name: string;
    parameter_id: string;
    label_zh: string;
    label_en: string;
    unit: string | null;
    kind: string;
    default: string | null;
    value: string | null;
    configured_value: string | null;
    scope_kind: string;
    scope_label: string;
    allowed_values: string[];
    mode: string;
    reason?: string | null;
    pending: boolean;
  }[];
  warnings: string[];
  context_complete: boolean;
};
export const tracerColumns = [
  'NAMES',
  'TYPES',
  'MRAT',
  'REF_RATIO',
  'INIT_DELTA',
  'REACTIVE_DECAY_RATE',
  'PARAM_FILES',
] as const;
export const tracerTypes = [
  ['isotope', '同位素', 'Isotope'],
  ['solute', '溶质', 'Solute'],
  ['particle', '颗粒 / 泥沙', 'Particle / sediment'],
  ['gas', '气体', 'Gas'],
  ['reactive', '反应性物质', 'Reactive'],
] as const;
export const tracerLabels: Record<(typeof tracerColumns)[number], [string, string]> = {
  NAMES: ['名称', 'Name'],
  TYPES: ['类型', 'Type'],
  MRAT: ['分子质量', 'Molecular mass'],
  REF_RATIO: ['参考比率', 'Reference ratio'],
  INIT_DELTA: ['初始δ值', 'Initial delta'],
  REACTIVE_DECAY_RATE: ['反应衰减率', 'Reactive decay rate'],
  PARAM_FILES: ['过程参数文件（当前行对应的路径）', 'Process file (path for this tracer)'],
};
export const tracerKeys = ['DEF_TRACER_NUM', ...tracerColumns.map((c) => `DEF_TRACER_${c}`)];
export const groupedField = (field: ModelField, catalog: readonly Descriptor[]) =>
  !field.derived &&
  !foundationControls.some((c) => c.keys.includes(field.name)) &&
  field.name !== 'DEF_SUBGRID_SCHEME' &&
  !tracerKeys.includes(field.name) &&
  !catalog.some(
    (d) =>
      d.raw_key.toLowerCase() === field.name.toLowerCase() &&
      ['land-cover-class', 'pft-type', 'pc-pft-component'].includes(d.scope),
  );
export const unquote = (value: string) => {
  const text = value.trim();
  return /^(['"]).*\1$/s.test(text)
    ? text.slice(1, -1).replace(text[0] === "'" ? /''/g : /""/g, text[0])
    : text;
};
export function fieldLiteral(field: Pick<ModelField, 'kind' | 'values'>, input: string) {
  const kind = field.kind.toLowerCase(),
    raw = input.trim();
  if (/[\r\n]/.test(input))
    throw new Error(bilingual('值不能包含换行', 'Newlines are not allowed'));
  if (kind === 'logical' && !['.true.', '.false.'].includes(raw.toLowerCase()))
    throw new Error(bilingual('请选择开启或关闭', 'Choose on or off'));
  if (kind === 'integer' && !/^[+-]?\d+$/.test(raw))
    throw new Error(bilingual('请输入整数', 'Enter an integer'));
  if (kind === 'real' && (raw === '' || fortranNumberInputValue(raw) === null))
    throw new Error(bilingual('请输入有限数值', 'Enter a finite number'));
  if (field.values.length && !field.values.some((v) => unquote(v) === unquote(raw)))
    throw new Error(bilingual('请选择目录支持的值', 'Choose a supported value'));
  return kind.startsWith('character') || kind === 'string'
    ? `'${unquote(input).replace(/'/g, "''")}'`
    : raw;
}
export function patchPhysics(overlay: Overlay, patch: Record<string, string>): Overlay {
  const keys = Object.keys(patch).map((k) => k.toLowerCase());
  const parameters = overlay.parameters.map((record) => {
    const key =
      record.scope_instance.kind === 'case-scalar'
        ? Object.keys(patch).find((name) => name.toLowerCase() === record.raw_key.toLowerCase())
        : undefined;
    return key ? { ...record, value: patch[key] } : record;
  });
  const physics = Object.fromEntries(
    Object.entries(overlay.physics).filter(([key]) => !keys.includes(key.toLowerCase())),
  );
  for (const [key, value] of Object.entries(patch))
    if (
      !parameters.some(
        (r) =>
          r.scope_instance.kind === 'case-scalar' && r.raw_key.toLowerCase() === key.toLowerCase(),
      )
    )
      physics[key] = value;
  return { ...overlay, physics, parameters };
}
export function setModelOption(
  overlay: Overlay,
  fields: ModelField[],
  name: string,
  input: string,
): Overlay {
  const field = fields.find((f) => f.name === name && !f.derived);
  if (!field) throw new Error(bilingual('字段不可编辑', 'Field is not writable'));
  const value = fieldLiteral(field, input),
    patch: Record<string, string> = { [name]: value };
  const pairs: Record<string, string> = {
    DEF_USE_MEDLYNST: 'DEF_USE_WUEST',
    DEF_USE_WUEST: 'DEF_USE_MEDLYNST',
    DEF_USE_Forcing_Downscaling: 'DEF_USE_Forcing_Downscaling_Simple',
    DEF_USE_Forcing_Downscaling_Simple: 'DEF_USE_Forcing_Downscaling',
  };
  if (value.toLowerCase() === '.true.' && pairs[name]) {
    if (!fields.some((f) => f.name === pairs[name] && !f.derived))
      throw new Error(bilingual('配对开关不可编辑', 'Paired switch is not writable'));
    patch[pairs[name]] = '.false.';
  }
  return patchPhysics(overlay, patch);
}
export function resetModelOption(overlay: Overlay, name: string): Overlay {
  return {
    ...overlay,
    physics: Object.fromEntries(
      Object.entries(overlay.physics).filter(([key]) => key.toLowerCase() !== name.toLowerCase()),
    ),
    parameters: overlay.parameters.filter(
      (record) =>
        record.scope_instance.kind !== 'case-scalar' ||
        record.raw_key.toLowerCase() !== name.toLowerCase(),
    ),
  };
}
export function tracerFileFor(raw: string, name: string, index: number): string {
  let positional = 0;
  for (const token of unquote(raw)
    .split(/[,;]/)
    .map((s) => s.trim())
    .filter(Boolean)) {
    const colon = token.indexOf(':'),
      keyed = colon >= 0 && !/^[a-z]:\\/i.test(token);
    if (keyed) {
      if (!token.slice(0, colon).trim() || !token.slice(colon + 1).trim())
        throw new Error(bilingual('示踪文件映射无效', 'Invalid tracer file mapping'));
      if (token.slice(0, colon).trim().toLowerCase() === name.toLowerCase())
        return token.slice(colon + 1).trim();
    } else if (positional++ === index) return token;
  }
  return '';
}
export function tracerRows(overlay: Overlay) {
  const arrays = tracerColumns.map((c) =>
    unquote(modelSettingValue(overlay, `DEF_TRACER_${c}`) ?? '')
      .split(',')
      .map((s) => s.trim()),
  );
  const countText = modelSettingValue(overlay, 'DEF_TRACER_NUM');
  const count =
    countText && /^\d+$/.test(countText)
      ? Math.min(Number(countText), 100)
      : Math.max(0, ...arrays.slice(0, -1).map((a) => (a.length === 1 && !a[0] ? 0 : a.length)));
  const files = modelSettingValue(overlay, 'DEF_TRACER_PARAM_FILES') ?? '';
  return Array.from(
    { length: count },
    (_, i) =>
      Object.fromEntries(
        tracerColumns.map((c, j) => [
          c,
          c === 'PARAM_FILES' ? tracerFileFor(files, arrays[0][i] ?? '', i) : (arrays[j][i] ?? ''),
        ]),
      ) as Record<(typeof tracerColumns)[number], string>,
  );
}
export function setTracerRows(
  overlay: Overlay,
  fields: ModelField[],
  rows: ReturnType<typeof tracerRows>,
): Overlay {
  if (rows.length > 100) throw new Error(bilingual('示踪物数量过多', 'Too many tracers'));
  const existing = tracerRows(resetModelOption(overlay, 'DEF_TRACER_PARAM_FILES'));
  const required = ['NAMES', 'TYPES'] as const;
  for (const row of rows) {
    if (
      !tracerTypes.some(([type]) => type === row.TYPES) &&
      !existing.some(
        (previous) =>
          previous.NAMES.toLowerCase() === row.NAMES.toLowerCase() && previous.TYPES === row.TYPES,
      )
    )
      throw new Error(bilingual('请选择有效示踪物类型', 'Choose a valid tracer type'));
    for (const key of required)
      if (!row[key].trim())
        throw new Error(bilingual('每行需要名称和类型', 'Each row needs a name and type'));
    for (const value of Object.values(row))
      if (/[,;\r\n]/.test(value))
        throw new Error(bilingual('每个单元格只能有一个值', 'One value per cell'));
  }
  for (const column of tracerColumns.slice(2, -1)) {
    const populated = rows.filter((row) => row[column].trim()).length;
    if (populated && populated !== rows.length)
      throw new Error(`${column}: 请填写整列或留空 / Fill the whole column or leave it empty`);
  }
  for (const row of rows)
    for (const column of ['MRAT', 'REF_RATIO', 'INIT_DELTA', 'REACTIVE_DECAY_RATE'] as const)
      if (row[column].trim() && fortranNumberInputValue(row[column]) === null)
        throw new Error(`${column}: 请输入有限 Fortran 数值 / Enter a finite Fortran number`);
  if (new Set(rows.map((row) => row.NAMES.toLowerCase())).size !== rows.length)
    throw new Error(bilingual('示踪物名称不能重复', 'Tracer names must be unique'));
  let next = overlay;
  let sameFiles = false;
  try {
    const previous = tracerRows(overlay);
    sameFiles =
      rows.length === previous.length &&
      rows.every(
        (row, i) => row.NAMES === previous[i].NAMES && row.PARAM_FILES === previous[i].PARAM_FILES,
      );
  } catch {
    /* A corrected table can replace an invalid existing mapping. */
  }
  for (const key of tracerKeys) {
    if (key === 'DEF_TRACER_PARAM_FILES') {
      if (!sameFiles)
        next = setModelOption(
          next,
          fields,
          key,
          rows.map((row) => `${row.NAMES}:${row.PARAM_FILES.trim() || 'null'}`).join(','),
        );
      continue;
    }
    if (!fields.some((f) => f.name === key && !f.derived))
      throw new Error(`${key}: 字段不可编辑 / Field is not writable`);
    const text =
      key === 'DEF_TRACER_NUM'
        ? String(rows.length)
        : rows.some((r) => r[key.slice(11) as (typeof tracerColumns)[number]].trim())
          ? rows.map((r) => r[key.slice(11) as (typeof tracerColumns)[number]]).join(',')
          : '';
    next = setModelOption(next, fields, key, text);
  }
  return next;
}
export function processListValues(raw: string): string[] {
  const values: string[] = [];
  let value = '',
    quote = '',
    closedQuote = false;
  for (let i = 0; i < raw.length; i++) {
    const char = raw[i];
    if (quote) {
      value += char;
      if (char === quote) {
        if (raw[i + 1] === quote) {
          value += raw[++i];
        } else {
          quote = '';
          closedQuote = true;
        }
      }
    } else if ((char === "'" || char === '"') && !value.trim()) {
      quote = char;
      value += char;
    } else if (char === ',') {
      if (!value.trim()) throw new Error(bilingual('列表元素不能为空', 'Empty list element'));
      values.push(value.trim());
      value = '';
      closedQuote = false;
    } else {
      if (closedQuote && char.trim())
        throw new Error(bilingual('引号后有无效字符', 'Unexpected text after quoted value'));
      value += char;
    }
  }
  if (quote) throw new Error(bilingual('引号未闭合', 'Unterminated quote'));
  if (!value.trim()) throw new Error(bilingual('列表元素不能为空', 'Empty list element'));
  values.push(value.trim());
  return values;
}
export function setScopedOption(
  overlay: Overlay,
  descriptor: Descriptor,
  value: string,
  index: number,
  file: string,
  shape?: string,
): Overlay {
  if (descriptor.storage === 'read-only')
    throw new Error(bilingual('此参数只读', 'Read-only parameter'));
  if (
    descriptor.storage === 'process-parameter-file' &&
    !overlay.process_files.some((asset) => asset.relative_path === file)
  )
    throw new Error(bilingual('过程资产不存在', 'Process asset missing'));
  const items = (shape === 'list' ? processListValues(value) : [value]).map((item) =>
    fieldLiteral(
      { kind: descriptor.value_kind, values: descriptor.validation.allowed_values },
      item,
    ),
  );
  for (const item of items) {
    const numeric = fortranNumberInputValue(item);
    if (numeric !== null) {
      const n = Number(numeric),
        minimum = descriptor.validation.min,
        maximum = descriptor.validation.max;
      if (minimum !== null && n < Number(fortranNumberInputValue(minimum)))
        throw new Error(bilingual('低于参数下限', 'Below parameter minimum'));
      if (maximum !== null && n > Number(fortranNumberInputValue(maximum)))
        throw new Error(bilingual('高于参数上限', 'Above parameter maximum'));
    }
  }
  const record = makeRecord(descriptor, items.join(', '), index, file);
  const previous = overlay.parameters.find((item) => parameterKey(item) === parameterKey(record));
  if (previous?.expected_old_value !== undefined)
    record.expected_old_value = previous.expected_old_value;
  return { ...overlay, parameters: upsertParameter(overlay.parameters, record) };
}
export function resetScopedOption(
  overlay: Overlay,
  descriptor: Descriptor,
  index: number,
  file: string,
): Overlay {
  const key = parameterKey(makeRecord(descriptor, '0', index, file));
  return { ...overlay, parameters: overlay.parameters.filter((r) => parameterKey(r) !== key) };
}
export function setProcessOption(
  overlay: Overlay,
  descriptor: Descriptor,
  value: string,
  file: string,
  context: ConfigurationField[],
  shape?: string,
  activeMethaneAsset?: boolean | null,
): Overlay {
  let next = setScopedOption(overlay, descriptor, value, 0, file, shape);
  if (
    activeMethaneAsset === true &&
    descriptor.raw_key.toLowerCase() === 'def_methane%inundation_mode'
  ) {
    const rawMode = unquote(value).toLowerCase();
    const mode =
      (
        {
          'dynamic-wtd': 'dynamic_wtd',
          dh_all_thr05: 'hybrid',
          dyn_routing_hybrid: 'hybrid',
          giems: 'satellite',
        } as Record<string, string>
      )[rawMode] ?? rawMode;
    if (['routing', 'hybrid'].includes(mode))
      for (const key of ['DEF_USE_GridRiverLakeFlow', 'DEF_USE_SinglePoint']) {
        const v = context.find((f) => f.name === key)?.configured_value?.toLowerCase();
        if (
          (key === 'DEF_USE_GridRiverLakeFlow' && v === '.false.') ||
          (key === 'DEF_USE_SinglePoint' && v === '.true.')
        )
          throw new Error(
            bilingual('河道淹水需要空间模式和河湖过程', 'Routing needs a spatial case and rivers'),
          );
      }
    next = patchPhysics(next, {
      DEF_USE_Dynamic_Wetland: ['dynamic_wtd', 'hybrid'].includes(mode) ? '.true.' : '.false.',
    });
  }
  return next;
}

export function resetProcessOption(
  overlay: Overlay,
  descriptor: Descriptor,
  file: string,
  original: string | null,
  context: ConfigurationField[],
  shape?: string,
  activeMethaneAsset?: boolean | null,
): Overlay {
  const reset = resetScopedOption(overlay, descriptor, 0, file);
  if (
    activeMethaneAsset !== true ||
    descriptor.raw_key.toLowerCase() !== 'def_methane%inundation_mode'
  )
    return reset;
  if (original === null)
    throw new Error(
      bilingual(
        '资产未明确淹水模式，无法安全恢复联动开关',
        'Asset has no explicit inundation mode',
      ),
    );
  const linked = setProcessOption(
    reset,
    descriptor,
    original,
    file,
    context,
    shape,
    activeMethaneAsset,
  );
  return resetScopedOption(linked, descriptor, 0, file);
}

export const tracerFamilies = [
  {
    id: 'isotope',
    zh: '水同位素',
    en: 'Water isotopes',
    rows: [
      ['H2_18O', 'isotope'],
      ['HDO', 'isotope'],
    ],
  },
  { id: 'methane', zh: '甲烷 CH4', en: 'Methane CH4', rows: [['CH4', 'gas']] },
  { id: 'chloride', zh: '氯离子 Cl', en: 'Chloride Cl', rows: [['Cl', 'solute']] },
  { id: 'sediment', zh: '河道泥沙', en: 'River sediment', rows: [['SEDIMENT', 'particle']] },
] as const;
export type TracerFamily = (typeof tracerFamilies)[number]['id'];
export type FamilyContext = {
  kernelMacros?: readonly string[] | null;
  caseScope?: 'site' | 'spatial' | 'unknown' | null;
};
export function appendTracerFamily(
  rows: ReturnType<typeof tracerRows>,
  family: TracerFamily,
): ReturnType<typeof tracerRows> {
  const chosen = tracerFamilies.find((item) => item.id === family);
  if (!chosen) throw new Error('Unknown tracer family');
  const aliases: Record<string, string[]> = {
    CH4: ['CH4', 'METHANE'],
    H2_18O: ['H2_18O', 'O18'],
    SEDIMENT: ['SEDIMENT', 'SED'],
  };
  return [
    ...rows,
    ...chosen.rows
      .filter(
        ([name]) =>
          !rows.some((row) =>
            (aliases[name] ?? [name]).some(
              (alias) => alias.toLowerCase() === row.NAMES.toLowerCase(),
            ),
          ),
      )
      .map(([NAMES, TYPES]) => ({
        NAMES,
        TYPES,
        MRAT: '',
        REF_RATIO: '',
        INIT_DELTA: '',
        REACTIVE_DECAY_RATE: '',
        PARAM_FILES: '',
      })),
  ];
}
export function tracerFamilyState(
  overlay: Overlay,
  family: TracerFamily,
  context: FamilyContext,
): { status: 'blocked' | 'pending' | 'ready'; reasons: [string, string][] } {
  const reasons: [string, string][] = [],
    pending: [string, string][] = [];
  const flag = (key: string) => {
    const v = modelSettingValue(overlay, key)?.trim().toLowerCase();
    return v === '.true.' ? true : v === '.false.' ? false : null;
  };
  const need = (key: string, value: boolean, zh: string, en: string) => {
    const actual = flag(key);
    if (actual === null) pending.push([zh, en]);
    else if (actual !== value) reasons.push([zh, en]);
  };
  need('DEF_USE_TRACER', true, '先开启示踪过程', 'Enable tracer processes first');
  need(
    'DEF_USE_Campbell_SOIL_MODEL',
    false,
    '需要 van Genuchten 土壤方案',
    'Requires van Genuchten soil',
  );
  need('DEF_URBAN_RUN', false, '不支持城市模式', 'Urban mode is unsupported');
  const macros = context.kernelMacros?.map((name) => name.toLowerCase()),
    catchment = macros?.includes('catchment');
  if (catchment) reasons.push(['不支持 Catchment 模式', 'Catchment mode is unsupported']);
  else if (!macros) pending.push(['算例范围与内核待核对', 'Case scope and kernel pending']);
  if (family === 'methane') {
    need('DEF_USE_BGC', true, '甲烷需要 BGC', 'Methane requires BGC');
    if (
      flag('DEF_USE_LCT') === true ||
      (flag('DEF_USE_PFT') === true && flag('DEF_USE_PC') === true) ||
      (flag('DEF_USE_PFT') === false && flag('DEF_USE_PC') === false)
    )
      reasons.push([bilingual('甲烷需要 PFT', 'PC'), 'Methane requires PFT / PC']);
    else if (['DEF_USE_LCT', 'DEF_USE_PFT', 'DEF_USE_PC'].some((key) => flag(key) === null))
      pending.push(['PFT / PC 结构待确认', 'PFT / PC structure pending']);
  }
  if (family === 'isotope')
    need(
      'DEF_USE_LULCC',
      false,
      '水同位素不能与 LULCC 同开',
      'Water isotopes cannot run with LULCC',
    );
  if (family === 'sediment') {
    need(
      'DEF_USE_GridRiverLakeFlow',
      true,
      '河道泥沙需要河湖汇流',
      'Sediment requires river/lake routing',
    );
    if (context.caseScope === 'site' || macros?.includes('singlepoint'))
      reasons.push(['河道泥沙需要空间算例', 'Sediment requires a spatial case']);
    else if (context.caseScope !== 'spatial' && !macros)
      pending.push(['需要确认空间算例', 'Confirm spatial scope']);
    if (macros && !macros.includes('gridriverlakeflow'))
      reasons.push(['所选内核没有河湖汇流能力', 'Selected kernel lacks river/lake routing']);
    else if (!macros) pending.push(['河湖内核能力待核对', 'River/lake kernel capability pending']);
  }
  return {
    status: reasons.length ? 'blocked' : pending.length ? 'pending' : 'ready',
    reasons: reasons.length ? reasons : pending,
  };
}
export function setStomatalScheme(overlay: Overlay, fields: ModelField[], scheme: string): Overlay {
  if (!['BALL_BERRY', 'MEDLYN', 'WUE'].includes(scheme)) throw new Error('Invalid stomatal scheme');
  if (
    !['DEF_USE_MEDLYNST', 'DEF_USE_WUEST'].every((name) =>
      fields.some((f) => f.name === name && !f.derived),
    )
  )
    throw new Error('Stomatal fields are not writable');
  return patchPhysics(overlay, {
    DEF_USE_MEDLYNST: scheme === 'MEDLYN' ? '.true.' : '.false.',
    DEF_USE_WUEST: scheme === 'WUE' ? '.true.' : '.false.',
  });
}
