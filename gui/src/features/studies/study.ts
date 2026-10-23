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
    throw new Error(
      bilingual(
        `${row.name}：下限和上限都要填数，且下限小于上限`,
        `${row.name}: finite min < max required`,
      ),
    );
  if (scale === 'log' && min <= 0)
    throw new Error(
      bilingual(`${row.name}：对数尺度的范围要大于 0`, `${row.name}: log range must be positive`),
    );
  if (
    (row.min != null && (min < row.min || (!row.min_inclusive && min === row.min))) ||
    (row.max != null && (max > row.max || (!row.max_inclusive && max === row.max))) ||
    min === row.sentinel ||
    max === row.sentinel
  )
    throw new Error(
      bilingual(
        `${row.name}：超出了物理允许的范围（或用了表示缺省的特殊值）`,
        `${row.name}: outside physical bounds or sentinel`,
      ),
    );
  // 参数目录的作用域来自 colm-model（多一个 process_file，读出来恒为 null），
  // 研究规格的作用域没有这个字段且拒绝未知字段：原样转发时预检报
  // `unknown field process_file`（实测：GUI 参数率定预检全部失败）。只带研究规格认识的字段。
  const { kind, scheme, index, type_name, process_file } = row.scope_instance;
  if (process_file != null)
    throw new Error(
      bilingual(
        `${row.name}：过程文件里的参数不能用研究抽样`,
        `${row.name}: process-file parameters cannot be sampled by a study`,
      ),
    );
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
/**
 * 勾上参数时预填的研究范围：和版本页 `defaultRange` 同一条规则（默认值的 0.5–1.5 倍，夹进目录的上下界），
 * 但这里还拿得到开闭与尺度。夹到开区间的端点上时 `parameterSpec` 会拒，所以往里收 1%；
 * 对数尺度的下限必须大于 0。给不出合法范围就留空，让人自己填。
 */
export function suggestedRange(row: ParameterContext, scale: string): [string, string] {
  const trim = (value: number) => String(Number(value.toPrecision(6)));
  // 目录用 ±最小正规数表示"严格大于/小于 0"。
  const clean = (value: number | undefined) =>
    value == null ? null : value !== 0 && Math.abs(value) < 1e-300 ? 0 : value;
  const lowBound = clean(row.min),
    highBound = clean(row.max);
  const lowOpen = lowBound != null && (row.min_inclusive === false || lowBound !== row.min),
    highOpen = highBound != null && (row.max_inclusive === false || highBound !== row.max);
  let low: number, high: number;
  if (row.default !== 0 && Number.isFinite(row.default)) {
    low = Math.min(row.default * 0.5, row.default * 1.5);
    high = Math.max(row.default * 0.5, row.default * 1.5);
  } else if (lowBound != null && highBound != null) {
    [low, high] = [lowBound, highBound];
  } else return ['', ''];
  if (lowBound != null && low <= lowBound) low = lowBound;
  if (highBound != null && high >= highBound) high = highBound;
  const span = high - low;
  if (lowOpen && low === lowBound) low += span * 0.01;
  if (highOpen && high === highBound) high -= span * 0.01;
  if (scale === 'log' && low <= 0) return ['', ''];
  if (!(low < high) || low === row.sentinel || high === row.sentinel) return ['', ''];
  return [trim(low), trim(high)];
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
  // Morris 的样本数是"轨迹数 × (参数数 + 1)"：每条轨迹 k+1 个点。
  const candidates =
    kind === 'tuning'
      ? population * (generations + 1)
      : method === 'oat'
        ? 2 * parameters
        : method === 'morris'
          ? count * (parameters + 1)
          : count;
  if (kind !== 'tuning' && method === 'morris' && !(Number.isInteger(count) && count >= 2))
    throw new Error(
      bilingual('Morris 至少要 2 条轨迹', 'Morris screening needs at least two trajectories'),
    );
  if (!Number.isInteger(candidates) || candidates < 1 || candidates > 1000)
    throw new Error(bilingual('候选成员总数要在 1–1000 之间', 'Candidate budget must be 1–1000'));
  if (
    kind === 'tuning' &&
    (!Number.isInteger(population) ||
      population < 4 ||
      !Number.isInteger(generations) ||
      generations < 1)
  )
    throw new Error(
      bilingual('种群至少 4、迭代至少 1 代', 'Population ≥4 and generations ≥1 required'),
    );
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
  if (!['linear', '4', '8'].includes(hidden))
    throw new Error(bilingual('不支持这个网络结构', 'Unsupported network size'));
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
    throw new Error(
      bilingual(
        '网络的输入特征或输出范围不对：特征不能为空或重复，下限要小于上限',
        'Invalid network features or output range',
      ),
    );
  output = slot === 'soil_stress' ? output.trim().toLowerCase() : output.trim().toUpperCase();
  if (slot !== 'soil_stress' && !output.startsWith(slot === 'land_class' ? 'DEF_LC_' : 'DEF_PFT_'))
    throw new Error(
      bilingual(
        '输出参数和网络作用位置不配：每个植物功能型用 DEF_PFT_ 参数，每个地类用 DEF_LC_ 参数',
        'Output parameter does not match network slot',
      ),
    );
  if (slot === 'soil_stress' && output !== 'beta')
    throw new Error(bilingual('土壤水分胁迫网络的输出只能是 beta', 'soil_stress outputs beta'));
  if (slot === 'soil_stress' && (min < 0 || (!relative && max > 1 + 1e-10)))
    throw new Error(
      bilingual(
        'β 取绝对值时要在 [0,1] 内；取相对值时是非负的倍数',
        'Absolute beta must stay in [0,1]; relative beta is a non-negative multiplier',
      ),
    );
  if (!['pft', 'land_class', 'soil_stress'].includes(slot))
    throw new Error(bilingual('网络作用位置不对', 'Invalid slot'));
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
  if (!['pft', 'land_class', 'soil_stress'].includes(slot))
    throw new Error(bilingual('网络作用位置不对', 'Invalid network slot'));
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
  /** `station`：观测时间比世界时早几小时（PLUMBER2 是当地标准时，中国的站填 8）；空着按 0。 */
  utcOffset: string;
  period: '' | 'daily' | 'monthly';
  /** 这个目标单独用的比较指标；空着沿用研究的总体指标。 */
  metric?: string;
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
    utcOffset: '',
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
    utcOffset: '',
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
    utcOffset: '',
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
    const offset = draft.utcOffset.trim()
      ? number(draft.utcOffset, -14, 14, ['时差', 'UTC offset'])
      : 0;
    source =
      draft.kind === 'station'
        ? { kind: 'station', path, lon, lat, ...(offset ? { utc_offset: offset } : {}) }
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
    metric: draft.metric || metric,
    weight: 1,
    min_pairs: minPairs,
    ...windows,
    source,
  };
}

/** 一个算例的时间设置（`read_timing` 读出来的那几项）。日期都是 `YYYY-MM-DD`。 */
export interface SiteTiming {
  start: string;
  end: string;
  /** history 的正式起点：旧预热模式是预热截止日，回卷模式和没预热时是起始日。 */
  outputStart: string;
  spinupYears: number;
}

const YEAR_MS = 365.25 * 86_400_000;
const utcDay = (text: string) => {
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(text);
  return m ? Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : NaN;
};
const isoDay = (ms: number) => new Date(ms).toISOString().slice(0, 10);
const addYears = (ms: number, years: number) => {
  const d = new Date(ms);
  return Date.UTC(d.getUTCFullYear() + years, d.getUTCMonth(), d.getUTCDate());
};

/**
 * 率定期与验证期的预填。原来率定期取整个模拟期、验证期空着，带来两个问题：
 * 每个成员都从 mkinidata 冷启动，算例没设预热时前一年的土壤水、温度还在往平衡态走，
 * 率定会拿参数去补偿初值偏差；没有验证期，过拟合警告永远不会出现。
 * 所以：没预热且有两年以上时扣掉第一年；剩下满三年时把末尾约 30%（整年，至少一年）留作验证期。
 * 几个算例取共同时段，有一个没预热就按没预热算。
 */
export function suggestedWindows(sites: SiteTiming[]) {
  if (!sites.length) return null;
  const starts = sites.map((s) => utcDay(s.outputStart || s.start));
  const ends = sites.map((s) => utcDay(s.end));
  if (![...starts, ...ends].every(Number.isFinite)) return null;
  let start = Math.max(...starts);
  const end = Math.min(...ends);
  if (start >= end) return null;
  const warmup = !sites.every((s) => s.spinupYears > 0) && Math.round((end - start) / YEAR_MS) >= 2;
  if (warmup) start = addYears(start, 1);
  const years = Math.round((end - start) / YEAR_MS);
  const validationYears = years >= 3 ? Math.max(1, Math.round(0.3 * years)) : 0;
  const split = validationYears ? addYears(start, years - validationYears) : end;
  const valid = validationYears > 0 && split < end;
  return {
    from: isoDay(start),
    to: isoDay(valid ? split : end),
    validationFrom: valid ? isoDay(split) : '',
    validationTo: valid ? isoDay(end) : '',
    warmup,
    validationYears: valid ? validationYears : 0,
  };
}

/** 差分进化的预算偏小时的提示（中、英）。种群默认已是参数个数的 5 倍。 */
export function budgetAdvice(
  parameters: number,
  population: number,
  generations: number,
): [string, string][] {
  if (!(parameters > 0) || !(population > 0) || !(generations > 0)) return [];
  const advice: [string, string][] = [];
  if (parameters > 8)
    advice.push([
      `一次率定 ${parameters} 个参数，差分进化很难在有限预算里收敛；建议先做不确定性分析，只率定其中最敏感的几个。`,
      `Calibrating ${parameters} parameters at once rarely converges on a limited budget; run an uncertainty analysis first and calibrate only the most sensitive few.`,
    ]);
  if (population < 5 * parameters)
    advice.push([
      `种群少于参数个数的 5 倍（${5 * parameters}），容易过早收敛到局部最优。`,
      `The population is below five times the parameter count (${5 * parameters}); it may converge prematurely.`,
    ]);
  const perParameter = (population * (generations + 1)) / parameters;
  if (perParameter < 20)
    advice.push([
      `平均每个参数不到 20 次模拟（现在约 ${Math.floor(perParameter)} 次），结果接近在随机样本里挑最好的；加代数或减少参数。`,
      `Fewer than 20 simulations per parameter (about ${Math.floor(perParameter)}): the result is close to picking the best random sample; add generations or drop parameters.`,
    ]);
  return advice;
}

type CandidateScore = { feasible?: boolean; calibration?: number; validation?: number };

/**
 * 某个成员相对默认参数（基准 m000000）的判定。写回草稿前看：率定期不比基准好、或验证期变差，
 * 都说明这组参数不值得进新版本，默认不让写（用户可以明确选择仍要写）。基准本身、没有基准成绩时不判。
 */
export function memberVerdict(
  candidates: Record<string, CandidateScore> | undefined,
  member: string,
): { blocked: boolean; zh: string; en: string } | null {
  const base = candidates?.m000000;
  const chosen = candidates?.[member];
  if (member === 'm000000' || base?.calibration == null || !chosen) return null;
  const fmt = (value: number) => String(Number(value.toPrecision(4)));
  if (chosen.feasible === false || chosen.calibration == null)
    return {
      blocked: true,
      zh: `${member} 没有可用的率定期成绩，不能和默认参数比。`,
      en: `${member} has no usable calibration score to compare with the default parameters.`,
    };
  const cal = `${fmt(base.calibration)} → ${fmt(chosen.calibration)}`;
  if (chosen.calibration >= base.calibration)
    return {
      blocked: true,
      zh: `${member} 的率定期目标函数（${cal}）不比默认参数好：这次研究没找到更好的参数。可以放宽范围、加代数或换参数再试。`,
      en: `${member} does not beat the default parameters in the calibration period (${cal}): this study found nothing better. Widen the ranges, add generations or pick other parameters.`,
    };
  const gain = Math.round((1 - chosen.calibration / base.calibration) * 100);
  if (base.validation != null && chosen.validation != null) {
    const val = `${fmt(base.validation)} → ${fmt(chosen.validation)}`;
    if (chosen.validation > base.validation)
      return {
        blocked: true,
        zh: `${member} 率定期变好（${cal}），验证期却变差（${val}）：过拟合了率定期，换到别的时段未必更好。`,
        en: `${member} improves the calibration period (${cal}) but worsens validation (${val}): it overfits and may not hold in other periods.`,
      };
    return {
      blocked: false,
      zh: `${member} 比默认参数好：率定期 ${cal}（降低 ${gain}%），验证期 ${val}。`,
      en: `${member} beats the default parameters: calibration ${cal} (${gain}% lower), validation ${val}.`,
    };
  }
  return {
    blocked: false,
    zh: `${member} 率定期比默认参数好（${cal}，降低 ${gain}%）；没有验证期，换到别的时段是否更好没有检验。`,
    en: `${member} beats the default parameters in the calibration period (${cal}, ${gain}% lower); with no validation period, other periods are untested.`,
  };
}

/** 后端研究状态里的一条提示换成中英两种说法；不认得的原样返回（后端错误原样显示）。 */
export function studyWarningText(warning: string): [string, string] {
  if (warning.startsWith('overfitting:'))
    return [
      '最优候选在率定期比默认参数好、验证期却更差：可能过拟合。',
      'The best candidate beats the default in calibration but is worse in validation: possible overfitting.',
    ];
  const bound = /^bound: (\S+) (lower|upper) (\S+)$/.exec(warning);
  if (bound) {
    const [, key, side, value] = bound;
    return side === 'lower'
      ? [
          `${key} 的最优值 ${value} 贴着范围下限：真正的最优可能更小，建议把下限放宽后重跑。`,
          `The best ${key} (${value}) sits on the lower bound: the optimum may lie below; widen the lower bound and rerun.`,
        ]
      : [
          `${key} 的最优值 ${value} 贴着范围上限：真正的最优可能更大，建议把上限放宽后重跑。`,
          `The best ${key} (${value}) sits on the upper bound: the optimum may lie above; widen the upper bound and rerun.`,
        ];
  }
  if (warning.startsWith('tuning baseline failed'))
    return [
      '默认参数（基准）没有跑成，其余参数组都没有运行。先看基准的运行日志修好问题，再点"重试失败任务"。',
      'The baseline (default parameters) failed, so no other parameter set ran. Fix it using the baseline log, then retry failed tasks.',
    ];
  if (warning === 'uncertainty results are unavailable until every baseline task succeeds')
    return [
      '基准还没有在每个站点都跑成，不确定性结果暂时算不出来。',
      'Uncertainty results need the baseline to succeed at every site first.',
    ];
  const support =
    /^(.+) \/ (.+): (\d+) timestamps have insufficient ensemble support \(need at least (\d+)\)$/.exec(
      warning,
    );
  if (support) {
    const [, site, variable, count, need] = support;
    return [
      `${site} 的 ${variable} 有 ${count} 个时刻成功的成员不足 ${need} 个，这些时刻的 P05/P50/P95 留空、不画分位带。`,
      `${site} ${variable}: ${count} timestamps have fewer than ${need} successful members, so their P05/P50/P95 are left empty.`,
    ];
  }
  const axis = /^(.+) \/ (.+) \/ (.+) has a different time axis and was excluded$/.exec(warning);
  if (axis)
    return [
      `成员 ${axis[1]} 在 ${axis[2]} 的 ${axis[3]} 时间轴和基准不同，没有计入集合。`,
      `Member ${axis[1]} at ${axis[2]}: ${axis[3]} has a different time axis from the baseline and was left out.`,
    ];
  const excluded = /^(.+) \/ (.+) \/ (.+) was excluded: (.*)$/.exec(warning);
  if (excluded)
    return [
      `成员 ${excluded[1]} 在 ${excluded[2]} 的 ${excluded[3]} 读不出来，没有计入集合：${excluded[4]}`,
      `Member ${excluded[1]} at ${excluded[2]}: ${excluded[3]} could not be read and was left out: ${excluded[4]}`,
    ];
  for (const [prefix, zh, en] of [
    [
      'uncertainty result aggregation failed: ',
      '不确定性结果汇总失败：',
      'Uncertainty aggregation failed: ',
    ],
    [
      'Study summary tables could not be written: ',
      '研究汇总表没写成：',
      'Study summary tables could not be written: ',
    ],
  ] as const)
    if (warning.startsWith(prefix)) {
      const rest = warning.slice(prefix.length);
      return [zh + rest, en + rest];
    }
  return [warning, warning];
}

/**
 * 研究状态里的全部提示，给诊断卡片用。逐成员的"验证期算不出来"一个研究可能有几十条，合成一条计数；
 * 原文仍在导出报告里。
 */
export function studyDiagnostics(warnings: string[] | undefined): [string, string][] {
  const perMember = /^\S+ \/ .+ validation (target .+ )?unavailable: /;
  const members = new Set(
    (warnings ?? []).filter((w) => perMember.test(w)).map((w) => w.split(' / ')[0]!),
  );
  const notes = (warnings ?? []).filter((w) => !perMember.test(w)).map(studyWarningText);
  if (members.size)
    notes.push([
      `${members.size} 组参数的验证期成绩算不出来（多是验证期的配对时刻和基准对不上），它们的验证期一栏留空；原因见导出报告。`,
      `${members.size} parameter set${members.size === 1 ? '' : 's'} have no validation score (usually their validation pairs differ from the baseline); see the exported report.`,
    ]);
  return notes;
}

/**
 * `study_run` 失败时的错误：取消和失败都是一段 JSON（`{"type":"study_cancelled"|"study_failed","reason":…}`），
 * 其余是普通报错。取消不算错误；失败只取原因，后端英文原因原样跟在中文说明后面。
 */
export function studyRunOutcome(error: unknown): { cancelled: boolean; zh: string; en: string } {
  const text = String(error).replace(/^Error: /, '');
  type Payload = { type?: unknown; reason?: unknown };
  let payload: Payload | null = null;
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed && typeof parsed === 'object') payload = parsed as Payload;
  } catch {
    payload = null;
  }
  const reason = typeof payload?.reason === 'string' ? payload.reason : '';
  if (payload?.type === 'study_cancelled')
    return {
      cancelled: true,
      zh: '研究已停止：正在运行的成员会跑完，排队中的不再运行。',
      en: 'The study was stopped: running members finish, queued ones will not run.',
    };
  if (payload?.type === 'study_failed')
    return {
      cancelled: false,
      zh: `研究运行失败：${reason || '没有给出原因'}`,
      en: `The study run failed: ${reason || 'no reason given'}`,
    };
  return { cancelled: false, zh: text, en: text };
}

/**
 * 冻结规格里要核对的东西：每个参数的范围与尺度、预算。第 4 步要求勾"已核对冻结的范围和预算"，
 * 原来只显示参数个数、站点数，等于盲签（GUI 审查 C-06）。
 */
export function frozenDesign(spec: Record<string, unknown> | null) {
  const fmt = (value: unknown) =>
    typeof value === 'number' && Number.isFinite(value)
      ? String(Number(value.toPrecision(6)))
      : '—';
  const parameters = (Array.isArray(spec?.parameters) ? spec.parameters : []).map((raw) => {
    const p = (raw ?? {}) as Record<string, unknown>;
    const scope = (p.scope_instance ?? {}) as Record<string, unknown>;
    const where = [scope.scheme, scope.index].filter((v) => v != null && v !== '').join(' ');
    return {
      name: `${String(p.name ?? '')}${where ? ` (${where})` : ''}`,
      range: `${fmt(p.sample_min)} – ${fmt(p.sample_max)}`,
      log: p.scale === 'log',
    };
  });
  const budget = (spec?.budget ?? {}) as Record<string, unknown>;
  const lines: [string, string][] = [];
  const n = (value: unknown) => (typeof value === 'number' ? value : null);
  if (n(budget.population) != null && n(budget.generations) != null)
    lines.push([
      `差分进化：种群 ${budget.population}、迭代 ${budget.generations} 代${budget.include_baseline ? '、默认参数可进初代' : ''}`,
      `Differential evolution: population ${budget.population}, ${budget.generations} generations${budget.include_baseline ? ', baseline may join the first generation' : ''}`,
    ]);
  const total = plannedCandidates(spec);
  if (spec?.method === 'morris')
    lines.push([
      `Morris ${morrisTrajectories(spec)} 条轨迹，共 ${total} 组`,
      `Morris: ${morrisTrajectories(spec)} trajectories, ${total} samples`,
    ]);
  else if (spec?.method === 'oat')
    lines.push([`一次改一个，共 ${total} 组`, `One at a time: ${total} samples`]);
  else if (n(budget.candidate_count) != null)
    lines.push([`样本 ${budget.candidate_count} 组`, `${budget.candidate_count} samples`]);
  if (n(budget.patience) != null && n(budget.generations) != null)
    lines.push([
      `连续 ${budget.patience} 代没有改善就提前停止`,
      `stops early after ${budget.patience} generations without improvement`,
    ]);
  if (n(budget.jobs) != null)
    lines.push([`每个研究同时 ${budget.jobs} 个模拟`, `${budget.jobs} simulations at a time`]);
  if (n(spec?.seed) != null) lines.push([`随机种子 ${spec!.seed}`, `seed ${spec!.seed}`]);
  return { parameters, budget: lines };
}

type FrozenParameter = {
  name?: unknown;
  parameter_id?: unknown;
  scope_instance?: { kind?: unknown; scheme?: unknown; index?: unknown } | null;
  sample_min?: unknown;
  sample_max?: unknown;
  scale?: unknown;
};
const frozenParameters = (spec: Record<string, unknown> | null) =>
  (Array.isArray(spec?.parameters) ? spec.parameters : []) as FrozenParameter[];

/** 和后端 `ParameterSpec::member_key` 一致：逐 PFT 的参数带 1 起的下标，其余就是参数名。 */
export function frozenParameterKey(p: FrozenParameter): string {
  const scope = p.scope_instance;
  const pft = scope?.kind === 'pft-type' || scope?.kind === 'pc-pft-component';
  return pft && typeof scope?.index === 'number'
    ? `${String(p.name)}(${scope.index + 1})`
    : String(p.name ?? '');
}

type ImportanceRow = {
  site?: unknown;
  variable?: unknown;
  parameter?: unknown;
  method?: unknown;
  value?: unknown;
  mu?: unknown;
  sigma?: unknown;
  n?: unknown;
};
const finite = (value: unknown) =>
  typeof value === 'number' && Number.isFinite(value) ? value : null;

/** `importance.json` 里有哪些响应量：目标函数排第一，其余是"站点 / 输出变量"。 */
export function sensitivityResponses(
  rows: unknown,
): { key: string; site: string | null; variable: string }[] {
  const seen = new Map<string, { key: string; site: string | null; variable: string }>();
  for (const row of (Array.isArray(rows) ? rows : []) as ImportanceRow[]) {
    const site = typeof row.site === 'string' ? row.site : null;
    const variable = String(row.variable ?? '');
    const key = site == null ? variable : `${site} / ${variable}`;
    if (!seen.has(key)) seen.set(key, { key, site, variable });
  }
  return [...seen.values()].sort(
    (a, b) => Number(b.variable === 'objective') - Number(a.variable === 'objective'),
  );
}

/**
 * 一个响应量下各参数的敏感性，按影响大小排。三种方法的数不在同一个量纲上，换成可比的"影响"：
 * Morris 取 μ*（归一化尺度上的平均绝对效应）；LHS 取 |Spearman ρ|；OAT 取 |斜率 × 抽样范围|，
 * 即参数从下限扫到上限时响应量的变化。`direction` 是增大参数时响应量变大（1）还是变小（-1）。
 */
export function sensitivityRanking(
  rows: unknown,
  responseKey: string,
  spec: Record<string, unknown> | null,
) {
  const ranges = new Map(
    frozenParameters(spec).map((p) => [
      frozenParameterKey(p),
      (finite(p.sample_max) ?? NaN) - (finite(p.sample_min) ?? NaN),
    ]),
  );
  return ((Array.isArray(rows) ? rows : []) as ImportanceRow[])
    .filter((row) => {
      const site = typeof row.site === 'string' ? row.site : null;
      const variable = String(row.variable ?? '');
      return (site == null ? variable : `${site} / ${variable}`) === responseKey;
    })
    .map((row) => {
      const parameter = String(row.parameter ?? '');
      const value = finite(row.value);
      const signed =
        row.method === 'morris'
          ? finite(row.mu)
          : row.method === 'oat_finite_difference_slope' && value != null
            ? value * (ranges.get(parameter) ?? NaN)
            : value;
      const impact =
        row.method === 'morris'
          ? value
          : signed != null && Number.isFinite(signed)
            ? Math.abs(signed)
            : null;
      return {
        parameter,
        method: String(row.method ?? ''),
        impact,
        direction:
          signed == null || !Number.isFinite(signed) || signed === 0 ? 0 : Math.sign(signed),
        sigma: finite(row.sigma),
        n: finite(row.n) ?? 0,
      };
    })
    .sort((a, b) => (b.impact ?? -1) - (a.impact ?? -1));
}

/**
 * 敏感性结果"带入率定"：选中的参数带上不确定性研究冻结的范围与尺度，交给研究页的参数种子
 * （和版本页带参数进研究是同一条路，读出参数后自动勾上）。
 */
export function calibrationSeed(
  spec: Record<string, unknown> | null,
  keys: string[],
): import('../../lib/types').StudyPick[] {
  return frozenParameters(spec)
    .filter((p) => keys.includes(frozenParameterKey(p)))
    .map((p) => ({
      parameter_id: String(p.parameter_id ?? ''),
      raw_key: String(p.name ?? ''),
      scheme: typeof p.scope_instance?.scheme === 'string' ? p.scope_instance.scheme : null,
      index: typeof p.scope_instance?.index === 'number' ? p.scope_instance.index : null,
      min: String(finite(p.sample_min) ?? ''),
      max: String(finite(p.sample_max) ?? ''),
      scale: p.scale === 'log' ? 'log' : 'linear',
    }));
}

/** 后端 `MORRIS_DEFAULT_TRAJECTORIES`：规格没写轨迹数时用它。 */
const MORRIS_DEFAULT_TRAJECTORIES = 10;
const morrisTrajectories = (spec: Record<string, unknown> | null) => {
  const count = Number((spec?.budget as Record<string, unknown> | undefined)?.candidate_count);
  return count > 0 ? count : MORRIS_DEFAULT_TRAJECTORIES;
};

/**
 * 冻结规格计划跑的参数组数（不含基准），和后端 `default_candidate_count` 同一规则。原来一律取
 * `candidate_count`：Morris 的它是轨迹数，进度显示成"已完成 40 / 8"；OAT 没写它，进度没有分母
 *（2026-10-23 真实窗口实测）。
 */
export function plannedCandidates(spec: Record<string, unknown> | null): number | null {
  const budget = spec?.budget as Record<string, unknown> | undefined;
  if (!spec || !budget) return null;
  const k = Array.isArray(spec.parameters) ? spec.parameters.length : 0;
  if (spec.method === 'morris') return morrisTrajectories(spec) * (k + 1);
  if (spec.method === 'oat') return 2 * k;
  const count = Number(budget.candidate_count);
  if (count > 0) return count;
  const population = Number(budget.population),
    generations = Number(budget.generations);
  return population > 0 && generations >= 0 ? population * (generations + 1) : null;
}
