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
import { fieldInputOptions, fortranNumberInputValue } from '../../lib/param-presentation';
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
  parent_gate?: string | null;
  parent_gate_enabled?: boolean;
  built_in_default?: string | null;
};
const fractionationChildren = new Set([
  'DEF_ISO_KINETIC_SCHEME',
  'DEF_ISO_OPEN_WATER_KINETIC',
  'DEF_ISO_SOIL_KINETIC',
]);
// 上游 MaterialTransport 把示踪剂拆成 family：这些字段只对水同位素有意义。
const isotopeFields = new Set([
  ...fractionationChildren,
  'DEF_USE_ISOTOPE',
  'DEF_ISOTOPE_SPECIES',
  'DEF_ISOTOPE_PARAM_FILE',
  'DEF_ISO_USE_FRACTIONATION',
  'DEF_ISO_ICE_SUPERSAT_SLOPE',
  'DEF_ISO_CG_RELHUM_MAX',
  'DEF_ISO_SUBL_SKIN_MM',
  'DEF_ISO_SOIL_DIFFUSION',
  'DEF_ISO_SOIL_VAPOR_DIFFUSION',
  'DEF_ISO_CANOPY_EQUILIBRATION',
  'DEF_ISO_SNOWMELT_EQUILIBRATION',
  'DEF_ISO_NSS_LEAF_WATER_PER_LAI',
  'DEF_ISO_NSS_LEAF_PATH_LENGTH',
  'DEF_ISO_NSS_LEAF_RB',
  'DEF_ISO_REF_RATIO_H2_18O',
  'DEF_ISO_INIT_DELTA_H2_18O',
  'DEF_ISO_REF_RATIO_HDO',
  'DEF_ISO_INIT_DELTA_HDO',
  'DEF_MT_AQUIFER_MIXING_WATER_MM',
  'DEF_MT_USE_SOIL_INIT',
  'DEF_MT_SOIL_INIT_FILE',
  'DEF_MT_SOIL_INIT_VARS',
]);
/// 同位素字段是否可见：只有明确启用同位素时才显示；undefined 表示尚未核对，按不可见处理。
export function tracerFieldVisible(name: string, isotope?: boolean): boolean {
  return !isotopeFields.has(name.toUpperCase()) || isotope === true;
}
export function caseFieldVisible(
  name: string,
  fields: readonly ConfigurationField[],
  currentValues?: Readonly<Record<string, string>>,
): boolean {
  const currentValue = (key: string) =>
    Object.entries(currentValues ?? {}).find(
      ([name]) => name.toLowerCase() === key.toLowerCase(),
    )?.[1];
  const enabled = (value: string | undefined) => /^(true|\.true\.)$/i.test(value?.trim() ?? '');
  const isotopeValue =
    currentValue('DEF_USE_ISOTOPE') ??
    fields.find((field) => field.name.toUpperCase() === 'DEF_USE_ISOTOPE')?.configured_value ??
    fields.find((field) => field.name.toUpperCase() === 'DEF_USE_ISOTOPE')?.built_in_default ??
    undefined;
  const isotope = isotopeValue === undefined ? undefined : enabled(isotopeValue);
  if (!tracerFieldVisible(name, isotope)) return false;
  // These controls must not flash back while the next applicability query is loading.
  if (
    currentValues &&
    fractionationChildren.has(name.toUpperCase()) &&
    !enabled(currentValue('DEF_ISO_USE_FRACTIONATION'))
  )
    return false;
  if (['DEF_VIC_OPT', 'DEF_FILE_VIC_PARA', 'DEF_FILE_VIC_OPT'].includes(name.toUpperCase())) {
    const effective = (key: string) =>
      currentValue(key) ??
      fields.find((field) => field.name.toLowerCase() === key.toLowerCase())?.built_in_default;
    if (effective('DEF_Runoff_SCHEME')?.trim() !== '1') return false;
    const grid = enabled(effective('DEF_VIC_OPT') ?? '.false.');
    if (name.toUpperCase() === 'DEF_FILE_VIC_PARA' && grid) return false;
    if (name.toUpperCase() === 'DEF_FILE_VIC_OPT' && !grid) return false;
  }
  let field = fields.find((item) => item.name.toLowerCase() === name.toLowerCase());
  if (field?.mode === 'disabled' && field.pending === false) return false;
  const visited = new Set<string>([name.toLowerCase()]);
  while (field?.parent_gate) {
    if (field.parent_gate_enabled !== true) return false;
    const parent = field.parent_gate.toLowerCase();
    if (visited.has(parent)) return false;
    visited.add(parent);
    field = fields.find((item) => item.name.toLowerCase() === parent);
  }
  return true;
}

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
  crop_defaults?: Record<string, string>;
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
export const tracerColumns = ['NAMES', 'TYPES', 'PARAM_FILES'] as const;
export const tracerTypes = [
  ['isotope', '同位素', 'Isotope'],
  ['solute', '溶质', 'Solute'],
  ['particle', '颗粒 / 泥沙', 'Particle / sediment'],
  ['gas', '气体', 'Gas'],
] as const;
export const tracerLabels: Record<(typeof tracerColumns)[number], [string, string]> = {
  NAMES: ['名称', 'Name'],
  TYPES: ['类型', 'Type'],
  PARAM_FILES: ['过程参数文件（当前行对应的路径）', 'Process file (path for this tracer)'],
};
// 上游把旧的平铺 `DEF_TRACER_*` 表拆成按 family 的字段；表格只是这些字段的视图。
export const tracerKeys = [
  'DEF_USE_ISOTOPE',
  'DEF_ISOTOPE_SPECIES',
  'DEF_ISOTOPE_PARAM_FILE',
  'DEF_USE_METHANE',
  'DEF_METHANE_PARAM_FILE',
  'DEF_SOLUTE_NAMES',
  'DEF_SOLUTE_PARAM_FILES',
  'DEF_USE_SEDIMENT',
  'DEF_SEDIMENT_PARAM_FILE',
];
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
export function fieldLiteral(
  field: Pick<ModelField, 'kind' | 'values'> & Partial<Pick<ModelField, 'name'>>,
  input: string,
) {
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
  const choices = fieldInputOptions(field.name ?? '', field.kind, field.values);
  if (choices.length && !choices.some((v) => unquote(v) === unquote(raw)))
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
const tracerSetting = (overlay: Overlay, key: string) =>
  unquote(modelSettingValue(overlay, key) ?? '').trim();
const tracerFlag = (overlay: Overlay, key: string) =>
  tracerSetting(overlay, key).toLowerCase() === '.true.';
const tracerList = (raw: string) =>
  unquote(raw)
    .split(',')
    .map((s) => s.trim())
    .filter(Boolean);
const tracerFile = (value: string) => (/^(null|none)?$/i.test(value) ? '' : value);
/// 聚合判定：上游没有总开关，任一 family 开着（或溶质名单非空）即“示踪过程开启”。
export function tracerProcessEnabled(overlay: Overlay): boolean {
  return (
    tracerFlag(overlay, 'DEF_USE_ISOTOPE') ||
    tracerFlag(overlay, 'DEF_USE_METHANE') ||
    tracerFlag(overlay, 'DEF_USE_SEDIMENT') ||
    tracerSetting(overlay, 'DEF_SOLUTE_NAMES') !== ''
  );
}

/// 把新的 family 字段还原成表格行：同位素、甲烷、溶质、泥沙各占若干行。
export function tracerRows(overlay: Overlay) {
  const rows: Record<(typeof tracerColumns)[number], string>[] = [];
  if (tracerFlag(overlay, 'DEF_USE_ISOTOPE'))
    for (const name of tracerList(tracerSetting(overlay, 'DEF_ISOTOPE_SPECIES')))
      rows.push({
        NAMES: name,
        TYPES: 'isotope',
        PARAM_FILES: tracerFile(tracerSetting(overlay, 'DEF_ISOTOPE_PARAM_FILE')),
      });
  if (tracerFlag(overlay, 'DEF_USE_METHANE'))
    rows.push({
      NAMES: 'CH4',
      TYPES: 'gas',
      PARAM_FILES: tracerFile(tracerSetting(overlay, 'DEF_METHANE_PARAM_FILE')),
    });
  const soluteFiles = tracerSetting(overlay, 'DEF_SOLUTE_PARAM_FILES');
  tracerList(tracerSetting(overlay, 'DEF_SOLUTE_NAMES')).forEach((name, index) =>
    rows.push({
      NAMES: name,
      TYPES: 'solute',
      PARAM_FILES: tracerFile(tracerFileFor(soluteFiles, name, index)),
    }),
  );
  if (tracerFlag(overlay, 'DEF_USE_SEDIMENT'))
    rows.push({
      NAMES: 'SEDIMENT',
      TYPES: 'particle',
      PARAM_FILES: tracerFile(tracerSetting(overlay, 'DEF_SEDIMENT_PARAM_FILE')),
    });
  return rows;
}
export function setTracerRows(
  overlay: Overlay,
  fields: ModelField[],
  rows: ReturnType<typeof tracerRows>,
): Overlay {
  if (rows.length > 100) throw new Error(bilingual('示踪物数量过多', 'Too many tracers'));
  for (const row of rows) {
    if (!tracerTypes.some(([type]) => type === row.TYPES.toLowerCase()))
      throw new Error(
        bilingual(
          `请选择有效示踪物类型：${row.TYPES || '（空）'}`,
          `Choose a valid tracer type: ${row.TYPES || '(empty)'}`,
        ),
      );
    for (const key of ['NAMES', 'TYPES'] as const)
      if (!row[key].trim())
        throw new Error(bilingual('每行需要名称和类型', 'Each row needs a name and type'));
    for (const value of Object.values(row))
      if (/[,;\r\n]/.test(value))
        throw new Error(bilingual('每个单元格只能有一个值', 'One value per cell'));
  }
  if (new Set(rows.map((row) => row.NAMES.trim().toLowerCase())).size !== rows.length)
    throw new Error(bilingual('示踪物名称不能重复', 'Tracer names must be unique'));
  const namesOf = (type: string) =>
    rows.filter((row) => row.TYPES.toLowerCase() === type).map((row) => row.NAMES.trim());
  // 上游只编了两种水同位素、甲烷和河道泥沙各一种 provider；其余名字写进去引擎不认。
  for (const name of namesOf('isotope'))
    if (!['H2_18O', 'HDO'].includes(name.toUpperCase()))
      throw new Error(
        bilingual(
          `水同位素只支持 H2_18O / HDO：${name}`,
          `Isotopes support only H2_18O / HDO: ${name}`,
        ),
      );
  for (const name of namesOf('gas'))
    if (!['CH4', 'METHANE'].includes(name.toUpperCase()))
      throw new Error(bilingual(`甲烷只支持 CH4：${name}`, `Methane supports only CH4: ${name}`));
  for (const name of namesOf('particle'))
    if (!['SEDIMENT', 'SED'].includes(name.toUpperCase()))
      throw new Error(
        bilingual(`颗粒物只支持 SEDIMENT：${name}`, `Particles support only SEDIMENT: ${name}`),
      );
  const firstFile = (type: string) =>
    rows
      .filter((row) => row.TYPES.toLowerCase() === type)
      .map((row) => tracerFile(row.PARAM_FILES.trim()))
      .find(Boolean) ?? '';
  const isotopeNames = namesOf('isotope');
  const soluteNames = namesOf('solute');
  const soluteFiles = rows
    .filter((row) => row.TYPES.toLowerCase() === 'solute')
    .map((row) => `${row.NAMES.trim()}:${tracerFile(row.PARAM_FILES.trim()) || 'null'}`)
    .join(',');
  let next = overlay;
  const write = (key: string, value: string) => {
    if (!fields.some((field) => field.name === key && !field.derived))
      throw new Error(`${key}: 字段不可编辑 / Field is not writable`);
    next = setModelOption(next, fields, key, value);
  };
  write('DEF_USE_ISOTOPE', isotopeNames.length ? '.true.' : '.false.');
  write('DEF_ISOTOPE_SPECIES', isotopeNames.join(','));
  write('DEF_ISOTOPE_PARAM_FILE', firstFile('isotope') || 'null');
  write('DEF_USE_METHANE', namesOf('gas').length ? '.true.' : '.false.');
  write('DEF_METHANE_PARAM_FILE', firstFile('gas') || 'null');
  write('DEF_SOLUTE_NAMES', soluteNames.join(','));
  write('DEF_SOLUTE_PARAM_FILES', soluteFiles || 'null');
  write('DEF_USE_SEDIMENT', namesOf('particle').length ? '.true.' : '.false.');
  write('DEF_SEDIMENT_PARAM_FILE', firstFile('particle') || 'null');
  return next;
}
export function applyTracerSelection(
  overlay: Overlay,
  fields: ModelField[],
  rows: ReturnType<typeof tracerRows>,
): Overlay {
  const next = setTracerRows(overlay, fields, rows);
  return rows.some(
    (row) =>
      ['ch4', 'methane'].includes(row.NAMES.trim().toLowerCase()) &&
      row.TYPES.trim().toLowerCase() === 'gas',
  )
    ? setModelOption(next, fields, 'DEF_USE_BGC', '.true.')
    : next;
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
  shape?: string,
): Overlay {
  return setScopedOption(overlay, descriptor, value, 0, file, shape);
}
export function resetProcessOption(
  overlay: Overlay,
  descriptor: Descriptor,
  file: string,
): Overlay {
  return resetScopedOption(overlay, descriptor, 0, file);
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
  processFiles?: ConfigurationResult['process_files'];
  defaults?: Readonly<Record<string, string>>;
  engine?: string;
  subgrid?: string;
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
        PARAM_FILES: {
          CH4: 'standard_methane_parameter.nml',
          H2_18O: 'standard_isotope_parameter.nml',
          HDO: 'standard_isotope_parameter.nml',
          Cl: 'standard_chloride_parameter.nml',
          SEDIMENT: 'standard_sediment_parameter.nml',
        }[NAMES],
      })),
  ];
}
export function tracerFamilyState(
  overlay: Overlay,
  family: TracerFamily | null,
  context: FamilyContext,
): { status: 'blocked' | 'pending' | 'ready'; reasons: [string, string][] } {
  const reasons: [string, string][] = [],
    pending: [string, string][] = [];
  const flag = (key: string) => {
    const v = (
      modelSettingValue(overlay, key) ??
      Object.entries(context.defaults ?? {}).find(
        ([name]) => name.toLowerCase() === key.toLowerCase(),
      )?.[1]
    )
      ?.trim()
      .toLowerCase();
    return v === '.true.' ? true : v === '.false.' ? false : null;
  };
  const need = (key: string, value: boolean, zh: string, en: string) => {
    const actual = flag(key);
    if (actual === null) pending.push([zh, en]);
    else if (actual !== value) reasons.push([zh, en]);
  };
  // 上游没有总开关：任一家族开启（或溶质名单非空）才算“示踪过程已开启”。
  const familyFlags = ['DEF_USE_ISOTOPE', 'DEF_USE_METHANE', 'DEF_USE_SEDIMENT'].map(flag);
  const soluteNames = (
    modelSettingValue(overlay, 'DEF_SOLUTE_NAMES') ??
    Object.entries(context.defaults ?? {}).find(
      ([name]) => name.toLowerCase() === 'def_solute_names',
    )?.[1] ??
    ''
  ).trim();
  if (!familyFlags.includes(true) && !soluteNames) {
    if (familyFlags.includes(null))
      pending.push(['先开启示踪过程', 'Enable tracer processes first']);
    else reasons.push(['先开启示踪过程', 'Enable tracer processes first']);
  }
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
  if (context.engine === 'fortran' && macros?.includes('openmp'))
    reasons.push([
      'OpenMP 编译配置不支持示踪',
      'OpenMP build configurations do not support tracers',
    ]);
  if (family === 'methane') {
    const riverModes = ['routing', 'hybrid', 'dh_all_thr05', 'dyn_routing_hybrid'];
    if (
      context.processFiles?.some(
        (file) =>
          file.verified &&
          file.active_methane_asset === true &&
          file.entries.some(
            (entry) =>
              entry.path.toLowerCase() === 'def_methane%inundation_mode' &&
              !entry.unset &&
              riverModes.includes(
                unquote(entry.value ?? '')
                  .trim()
                  .toLowerCase(),
              ),
          ),
      )
    )
      need(
        'DEF_USE_GridRiverLakeFlow',
        true,
        '甲烷 routing/hybrid 淹没方案需要河湖汇流',
        'Methane routing/hybrid inundation requires river/lake routing',
      );
    need('DEF_USE_BGC', true, '甲烷需要 BGC', 'Methane requires BGC');
    if (context.subgrid && !['PFT', 'PC'].includes(context.subgrid.toUpperCase()))
      reasons.push(['甲烷需要 PFT / PC', 'Methane requires PFT / PC']);
    else if (!context.subgrid) pending.push(['PFT / PC 结构待确认', 'PFT / PC structure pending']);
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
      reasons.push([
        '所选编译配置不支持河湖汇流',
        'Selected build configuration lacks river/lake routing',
      ]);
    else if (!macros)
      pending.push(['河湖汇流编译支持待核对', 'River/lake routing build support pending']);
  }
  return {
    status: reasons.length ? 'blocked' : pending.length ? 'pending' : 'ready',
    reasons: reasons.length ? reasons : pending,
  };
}
export function tracerSetupIssues(overlay: Overlay, context: FamilyContext): [string, string][] {
  const rows = tracerRows(overlay);
  if (!rows.length) return [['请选择至少一种示踪物', 'Select at least one tracer']];
  const issues: [string, string][] = [];
  const families = new Set<TracerFamily | null>([null]);
  for (const row of rows) {
    const name = row.NAMES.trim().toLowerCase(),
      type = row.TYPES.trim().toLowerCase();
    if (type === 'isotope') families.add('isotope');
    else if (type === 'solute') families.add('chloride');
    else if (type === 'gas' && ['ch4', 'methane'].includes(name)) {
      families.add('methane');
      if (
        !unquote(row.PARAM_FILES).trim() ||
        /^(null|none)$/i.test(unquote(row.PARAM_FILES).trim())
      )
        issues.push([
          '甲烷需要参数文件，可选择内置预设或指定路径',
          'Methane requires a parameter file: choose a built-in preset or a path',
        ]);
    } else if (type === 'particle' && ['sediment', 'sed'].includes(name)) families.add('sediment');
    else if (context.engine === 'rust' && ['gas', 'particle'].includes(type))
      issues.push([
        `${row.NAMES}：当前 Rust 引擎不支持此示踪物`,
        `${row.NAMES}: this tracer is unsupported by the Rust engine`,
      ]);
  }
  for (const family of families) {
    const state = tracerFamilyState(overlay, family, context);
    if (state.status === 'blocked') issues.push(...state.reasons);
  }
  if (families.has('isotope')) {
    const key = 'DEF_MT_AQUIFER_MIXING_WATER_MM';
    const raw =
      modelSettingValue(overlay, key) ??
      Object.entries(context.defaults ?? {}).find(
        ([name]) => name.toLowerCase() === key.toLowerCase(),
      )?.[1] ??
      '';
    const mixing = fortranNumberInputValue(raw);
    if (mixing === null || !(Number(mixing) > 0))
      issues.push([
        '水同位素需要设置大于 0 的地下水混合水量（mm）',
        'Water isotopes require positive aquifer mixing water (mm)',
      ]);
  }
  return issues.filter(([zh], index) => issues.findIndex(([other]) => zh === other) === index);
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

export type CaseOptionPatch = Record<string, string | null>;

export function vicParameterDefault(name: string, runtime: string): string {
  const directory = unquote(runtime).trim();
  if (/^(null|none)?$/i.test(directory)) return '';
  // Preserve the upstream filename concatenation used by the initializer.
  return name === 'DEF_file_VIC_OPT'
    ? directory + 'vic/vic_para.nc'
    : directory.replace(/\/+$/, '') + '/vic/vic_para.txt';
}

export function cnInitialStatePatch(
  overlay: Overlay,
  caseValues: Readonly<Record<string, string>>,
  patch: CaseOptionPatch,
): CaseOptionPatch {
  const value = (name: string) => {
    const changed = Object.entries(patch).find(([key]) => key.toLowerCase() === name.toLowerCase());
    if (changed) return changed[1] ?? '';
    return (
      Object.entries(caseValues).find(([key]) => key.toLowerCase() === name.toLowerCase())?.[1] ??
      modelSettingValue(overlay, name) ??
      ''
    );
  };
  const missing = (text: string) => /^(null|none)?$/i.test(unquote(text).trim());
  if (!/^\.true\.$/i.test(value('DEF_USE_CN_INIT').trim()) || !missing(value('DEF_file_cn_init')))
    return {};
  const runtime = unquote(value('DEF_dir_runtime')).trim();
  if (missing(runtime)) return {};
  const separator = runtime.includes('\\') && !runtime.includes('/') ? '\\' : '/';
  const path = runtime.replace(/[\\/]+$/, '') + separator + 'cnsteadystate.nc';
  return { DEF_file_cn_init: fieldLiteral({ kind: 'character', values: [] }, path) };
}

export function caseOptionOverlay(
  overlay: Overlay,
  fields: readonly ModelField[],
  caseValues: Record<string, string> | null | undefined,
  context: readonly ConfigurationField[] = [],
): Overlay {
  const names = new Set(fields.map((field) => field.name.toLowerCase()));
  const explicit = Object.fromEntries(
    context
      .filter((field) => names.has(field.name.toLowerCase()) && field.configured_value !== null)
      .map((field) => [field.name, field.configured_value!]),
  );
  const physics = {
    ...Object.fromEntries(
      Object.entries(overlay.physics).filter(([key]) => !names.has(key.toLowerCase())),
    ),
    ...explicit,
    ...Object.fromEntries(
      Object.entries(caseValues ?? {}).filter(([key]) => names.has(key.toLowerCase())),
    ),
  };
  return {
    ...overlay,
    physics,
    parameters: overlay.parameters.filter(
      (record) =>
        record.scope_instance.kind !== 'case-scalar' || !names.has(record.raw_key.toLowerCase()),
    ),
  };
}
export function caseOptionPatch(
  before: Overlay,
  after: Overlay,
  fields: readonly ModelField[],
): CaseOptionPatch {
  return Object.fromEntries(
    fields
      .filter((field) => before.physics[field.name] !== after.physics[field.name])
      .map((field) => [field.name, after.physics[field.name] ?? null]),
  );
}
