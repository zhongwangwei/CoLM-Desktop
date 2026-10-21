import { bilingual } from '../../lib/bilingual';
import { utcSeconds } from '../../lib/dates';
export interface ParameterContext {
  name: string;
  id: string;
  scope: string;
  scope_instance: {
    kind: string;
    scheme?: string | null;
    index?: number | null;
    type_name?: string | null;
    process_file?: string | null;
  };
  label_zh: string;
  label_en: string;
  default: number;
  min?: number;
  max?: number;
  min_inclusive?: boolean;
  max_inclusive?: boolean;
  sentinel?: number;
  scale: string;
}
export function parameterSpec(
  row: ParameterContext,
  minText: string,
  maxText: string,
  scale: string,
) {
  const min = Number(minText),
    max = Number(maxText);
  if (
    !minText.trim() ||
    !maxText.trim() ||
    !Number.isFinite(min) ||
    !Number.isFinite(max) ||
    min >= max
  )
    throw new Error(`${row.name}: finite min < max required`);
  if (scale === 'log' && min <= 0) throw new Error(`${row.name}: log range must be positive`);
  if (
    (row.min != null && (min < row.min || (!row.min_inclusive && min === row.min))) ||
    (row.max != null && (max > row.max || (!row.max_inclusive && max === row.max))) ||
    min === row.sentinel ||
    max === row.sentinel
  )
    throw new Error(`${row.name}: outside physical bounds or sentinel`);
  // 参数目录的作用域来自 colm-model（多一个 process_file，读出来恒为 null），
  // 研究规格的作用域没有这个字段且拒绝未知字段：原样转发时预检报
  // `unknown field process_file`（实测：GUI 参数率定预检全部失败）。只带研究规格认识的字段。
  const { kind, scheme, index, type_name, process_file } = row.scope_instance;
  if (process_file != null)
    throw new Error(`${row.name}: process-file parameters cannot be sampled by a study`);
  const scope_instance = Object.fromEntries(
    Object.entries({ kind, scheme, index, type_name }).filter(([, v]) => v != null),
  );
  return {
    name: row.name,
    parameter_id: row.id,
    scope_instance,
    sample_min: min,
    sample_max: max,
    scale,
  };
}
export function calibrationWindows(
  from: string,
  to: string,
  validationFrom: string,
  validationTo: string,
) {
  const [start, end] = [from, to].map(utcSeconds);
  if (!Number.isFinite(start) || !Number.isFinite(end) || start >= end)
    throw new Error(
      bilingual(
        '率定期要填开始和结束，并且开始早于结束',
        'The calibration window needs a start before its end',
      ),
    );
  // 验证期可空：后端两头都不填也接受（研究记录里提示"没有独立验证期"）。原来这里要求四个时间都填，
  // 页面却标着"可空"，空着就被拒（2026-10-21 打包应用里实测）。要填就两头都填，和后端一样。
  if (!validationFrom.trim() && !validationTo.trim())
    return { from: start, to: end, validation_from: null, validation_to: null };
  const [validStart, validEnd] = [validationFrom, validationTo].map(utcSeconds);
  if (!Number.isFinite(validStart) || !Number.isFinite(validEnd) || validStart >= validEnd)
    throw new Error(
      bilingual(
        '验证期要么都空着，要么开始和结束都填，并且开始早于结束',
        'Leave the validation window empty, or give a start before its end',
      ),
    );
  if (!(end <= validStart || validEnd <= start))
    throw new Error(
      bilingual(
        '验证期不能和率定期重叠',
        'The validation window must not overlap the calibration window',
      ),
    );
  return { from: start, to: end, validation_from: validStart, validation_to: validEnd };
}
export function candidateBudget(
  kind: string,
  method: string,
  parameters: number,
  count: number,
  population: number,
  generations: number,
  sites: number,
) {
  const candidates =
    kind === 'tuning' ? population * (generations + 1) : method === 'oat' ? 2 * parameters : count;
  if (!Number.isInteger(candidates) || candidates < 1 || candidates > 1000)
    throw new Error('Candidate budget must be 1–1000');
  if (
    kind === 'tuning' &&
    (!Number.isInteger(population) ||
      population < 4 ||
      !Number.isInteger(generations) ||
      generations < 1)
  )
    throw new Error('Population ≥4 and generations ≥1 required');
  return { candidates, tasks: (candidates + 1) * sites, stageRuns: 3 * (candidates + 1) * sites };
}
export function studyPermissions(status: string) {
  const normalized = status.replace(/_/g, '').toLowerCase();
  return {
    run: ['ready', 'needsreview'].includes(normalized),
    pause: normalized === 'running',
    resume: normalized === 'paused',
    cancel: ['running', 'paused', 'needsreview'].includes(normalized),
    retry: ['completedwithfailures', 'needsreview', 'failed'].includes(normalized),
    results: ['completed', 'completedwithfailures'].includes(normalized),
  };
}
export function networkSpec(
  slot: string,
  features: string,
  output: string,
  lo: string,
  hi: string,
  hidden: string,
  relative: boolean,
) {
  if (!['linear', '4', '8'].includes(hidden)) throw new Error('Unsupported network size');
  const names = features
    .split(/[,，\s]+/)
    .map((v) => v.trim())
    .filter(Boolean);
  const min = Number(lo),
    max = Number(hi);
  if (
    !names.length ||
    new Set(names).size !== names.length ||
    !output.trim() ||
    !lo.trim() ||
    !hi.trim() ||
    !Number.isFinite(min) ||
    !Number.isFinite(max) ||
    min >= max
  )
    throw new Error('Invalid network features or output range');
  output = slot === 'soil_stress' ? output.trim().toLowerCase() : output.trim().toUpperCase();
  if (slot !== 'soil_stress' && !output.startsWith(slot === 'land_class' ? 'DEF_LC_' : 'DEF_PFT_'))
    throw new Error('Output parameter does not match network slot');
  if (slot === 'soil_stress' && output !== 'beta') throw new Error('soil_stress outputs beta');
  if (slot === 'soil_stress' && (min < 0 || (!relative && max > 1 + 1e-10)))
    throw new Error('Absolute beta must stay in [0,1]; relative beta is a non-negative multiplier');
  if (!['pft', 'land_class', 'soil_stress'].includes(slot)) throw new Error('Invalid slot');
  return {
    slot,
    features: names,
    outputs: [
      {
        name: output,
        range: [min, max],
        transform: 'sigmoid',
        ...(relative ? { relative: true } : {}),
      },
    ],
    hidden: hidden === 'linear' ? [] : [Number(hidden)],
  };
}
export function fitValidation(report: unknown): boolean | null {
  if (!report || typeof report !== 'object') return null;
  const validation = (report as { validation?: { passed?: boolean } | null }).validation;
  return validation ? validation.passed === true : null;
}

export function networkDefaults(slot: string) {
  if (!['pft', 'land_class', 'soil_stress'].includes(slot)) throw new Error('Invalid network slot');
  return slot === 'soil_stress'
    ? {
        slot,
        features: 'beta_physics, root_saturation, root_temperature, frozen_root_fraction',
        output: 'beta',
        lower: '0',
        upper: '1',
        relative: false,
      }
    : {
        slot,
        features: 'clim_tair, clim_vpd, clim_prec',
        output: slot === 'land_class' ? 'DEF_LC_VMAX25' : 'DEF_PFT_VMAX25',
        lower: '0.5',
        upper: '2',
        relative: true,
      };
}

/**
 * 版本页带来的要研究的参数，对上研究页按算例读出的参数行：同一个参数、同一个地类或植物类型就勾上，
 * 范围与尺度用版本页定的。返回研究页参数表的那种 `{行号: 设定}`。
 */
export function applyResearchSeed(
  rows: ParameterContext[],
  seed: import('../../lib/types').StudyPick[],
): Record<string, { enabled: boolean; min: string; max: string; scale: string }> {
  const picked: Record<string, { enabled: boolean; min: string; max: string; scale: string }> = {};
  rows.forEach((row, i) => {
    const pick = seed.find(
      (p) =>
        p.parameter_id === row.id &&
        (p.index ?? null) === (row.scope_instance.index ?? null) &&
        (p.scheme ?? null) === (row.scope_instance.scheme ?? null),
    );
    if (pick)
      picked[String(i)] = { enabled: true, min: pick.min, max: pick.max, scale: pick.scale };
  });
  return picked;
}

/** 格点算例（流域、区域、全球）的一个率定目标，研究页上逐行填。 */
export interface GridTargetDraft {
  /** `grid` 格点观测产品；`station` 区域里的通量站；`gauge` 水文站流量。 */
  kind: 'grid' | 'station' | 'gauge';
  /** `grid`、`gauge`：模型 history 变量（f_lfevpa、f_discharge）；`station`：观测标准名（Qle）。 */
  variable: string;
  path: string;
  /** `grid`：观测文件里的变量；`gauge` 的 NetCDF 观测才要填。 */
  obsVariable: string;
  lon: string;
  lat: string;
  /** `grid`：观测乘多少换成模型单位。 */
  scale: string;
  period: '' | 'daily' | 'monthly';
}

export const gridTargetDefaults: Record<GridTargetDraft['kind'], GridTargetDraft> = {
  grid: {
    kind: 'grid',
    variable: 'f_lfevpa',
    path: '',
    obsVariable: '',
    lon: '',
    lat: '',
    scale: '1',
    period: '',
  },
  station: {
    kind: 'station',
    variable: 'Qle',
    path: '',
    obsVariable: '',
    lon: '',
    lat: '',
    scale: '1',
    period: '',
  },
  gauge: {
    kind: 'gauge',
    variable: 'f_discharge',
    path: '',
    obsVariable: '',
    lon: '',
    lat: '',
    scale: '1',
    period: 'daily',
  },
};

/**
 * 格点目标 → 研究规格里的 `targets` 项（带 `source`）。键是"变量@种类序号"，同一变量能有几个站。
 * 填得不对就报错（中英两种说法），不让它到后端才失败。
 */
export function gridTargetSpec(
  draft: GridTargetDraft,
  index: number,
  windows: ReturnType<typeof calibrationWindows>,
  metric: string,
  minPairs: number,
) {
  const variable = draft.variable.trim();
  const path = draft.path.trim();
  if (!variable || !path)
    throw new Error(
      bilingual(
        `第 ${index + 1} 个目标要填变量和观测文件`,
        `Target ${index + 1} needs a variable and an observation file`,
      ),
    );
  const number = (text: string, low: number, high: number, what: [string, string]) => {
    const value = Number(text);
    if (!text.trim() || !Number.isFinite(value) || value < low || value > high)
      throw new Error(
        bilingual(
          `第 ${index + 1} 个目标的${what[0]}不对`,
          `Target ${index + 1} has an invalid ${what[1]}`,
        ),
      );
    return value;
  };
  let source: Record<string, unknown>;
  if (draft.kind === 'grid') {
    if (!draft.obsVariable.trim())
      throw new Error(
        bilingual(
          `第 ${index + 1} 个目标要填观测文件里的变量名`,
          `Target ${index + 1} needs the variable name in the observation file`,
        ),
      );
    const scale = number(draft.scale, -Infinity, Infinity, ['单位换算系数', 'unit scale']);
    if (scale === 0)
      throw new Error(
        bilingual(
          `第 ${index + 1} 个目标的单位换算系数不能是 0`,
          `Target ${index + 1} cannot have a zero unit scale`,
        ),
      );
    source = {
      kind: 'grid',
      path,
      variable: draft.obsVariable.trim(),
      scale,
      ...(draft.period ? { period: draft.period } : {}),
    };
  } else {
    const lon = number(draft.lon, -180, 360, ['经度', 'longitude']);
    const lat = number(draft.lat, -90, 90, ['纬度', 'latitude']);
    source =
      draft.kind === 'station'
        ? { kind: 'station', path, lon, lat }
        : {
            kind: 'gauge',
            path,
            lon,
            lat,
            ...(draft.obsVariable.trim() ? { variable: draft.obsVariable.trim() } : {}),
            ...(draft.period ? { period: draft.period } : {}),
          };
  }
  return {
    key: `${variable}@${draft.kind}${index + 1}`,
    variable,
    metric,
    weight: 1,
    min_pairs: minPairs,
    ...windows,
    source,
  };
}
