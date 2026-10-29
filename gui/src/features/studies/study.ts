import { bilingual } from '../../lib/bilingual';
import { utcSeconds } from '../../lib/dates';
import type { VersionRef } from '../../lib/types';

/**
 * 一项研究“用哪个模型修订”的基线（U-01）。
 *
 * 从"新建修改版"启动研究时，用户选的那一版只是研究基线，不能顺手写成模拟正在用的版本，否则别的
 * 算例会被静默换掉。研究首页没有显式基线（用户没选版本），退回当前运行修订，行为与改动前一致。
 * 已建好的研究不用这里的结果：研究页以自己记录（manifest 的 model_version）的冻结修订为准。
 */
export function researchBaselineOf(
  baseline: VersionRef | null | undefined,
  runVersion: VersionRef,
): VersionRef {
  return baseline ?? runVersion;
}
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
  // 相对输出是乘在查表值上的乘数，不能为负（后端同样拒绝；绝对输出的参数硬边界由后端按参数目录查）。
  if (relative && min < 0)
    throw new Error(
      bilingual(
        '相对输出是乘在默认值上的倍数，下限不能小于 0',
        'A relative output multiplies the default value, so its lower bound cannot be negative',
      ),
    );
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

/** 报告里一个程序身份（`feature_program` / `frozen_feature_program`）；缺字段时返回 null。 */
function programIdentity(value: unknown): { path: string | null; sha256: string | null } | null {
  if (!value || typeof value !== 'object') return null;
  const program = value as { path?: unknown; sha256?: unknown };
  const path = typeof program.path === 'string' && program.path ? program.path : null;
  const sha256 = typeof program.sha256 === 'string' && program.sha256 ? program.sha256 : null;
  return path || sha256 ? { path, sha256 } : null;
}

/** 报告里的冻结修订身份，形如 `official#2`；没有就返回空串。 */
function revisionIdentity(value: unknown): string {
  if (!value || typeof value !== 'object') return '';
  const version = (value as { version?: unknown }).version;
  if (!version || typeof version !== 'object') return '';
  const { id, revision } = version as { id?: unknown; revision?: unknown };
  return typeof id === 'string' && typeof revision === 'number' ? `${id}#${revision}` : '';
}

/** 提示里 hash 只留前 12 位：完整值在展开的拟合报告里，摘要里太长反而读不出来。 */
function shortHash(value: string | null): string {
  return value ? value.slice(0, 12) : '—';
}

/**
 * 后端 `inputs.warnings` 的英文原文是报告的稳定字段（脱离界面也要能读），已知的几类在界面上
 * 译成中文；以后后端新加的 warning 不吞，原文照出。固定前缀与 `hybrid_fit.rs` 的
 * `check_compatibility` 一一对应，改那边措辞要一起改这里（测试会红）。
 */
export function fitInputWarningText(warning: string): [string, string] {
  const revisions =
    /^the Studies come from different frozen model revisions \((.*)\); the fit combines their calibrated parameters as they are$/.exec(
      warning,
    );
  if (revisions)
    return [
      `这些研究来自不同的冻结模型修订（${revisions[1]}）；拟合会把它们的率定参数原样合并。`,
      `The Studies come from different frozen model revisions (${revisions[1]}); the fit combines their calibrated parameters as they are.`,
    ];
  const programs =
    /^the Studies do not all have the same frozen colm-rs \((.*)\); the rows are all extracted by the fit's own program, recorded as feature_program$/.exec(
      warning,
    );
  if (programs)
    return [
      `这些研究冻结的 colm-rs 不完全相同（${programs[1]}）；所有行都由拟合自己的程序抽取，报告里记为 feature_program。`,
      `The Studies do not all have the same frozen colm-rs (${programs[1]}); the rows are all extracted by the fit's own program, recorded as feature_program.`,
    ];
  const extracting =
    /^the fit extracts features with a different colm-rs than Study (.+) was calibrated with \((.*)\); a rebuild of the same source also changes the bytes, so confirm this is intended$/.exec(
      warning,
    );
  if (extracting)
    return [
      `本次提特征用的 colm-rs 与研究 ${extracting[1]} 率定时用的不是同一个（${extracting[2]}）；同一份源码重编也会改变字节，确认这是有意的切换。`,
      `The fit extracts features with a different colm-rs than Study ${extracting[1]} was calibrated with (${extracting[2]}); a rebuild of the same source also changes the bytes, so confirm this is intended.`,
    ];
  return [warning, warning];
}

/** 拟合结果区的一条可见提示：`zh`/`en` 两版，`key` 供 React 列表用。 */
export interface FitInputNote {
  key: string;
  zh: string;
  en: string;
}

/**
 * `hybrid-fit` 报告 `inputs` 段里使用者该看到、但以前只能展开 Advanced JSON 才看得到的信息：
 * 后端逐条兼容性 warning，以及“本次提特征的程序”和“研究冻结的程序”之间的内容差异。
 *
 * 程序差异要前端自己再算一遍：旧后端写的报告没有这条 warning，但字段齐全时仍能看出来。
 * 缺字段（老报告没有 `frozen_feature_program`、程序读不出 sha256、只有路径没有内容）一律
 * 不报——没有证据时宁可不说，也不要把“无法比较”说成“不一致”。后端已经为同一条差异写过
 * warning 时不再按字段重复一遍。
 */
export function fitInputNotes(report: unknown): FitInputNote[] {
  const inputs =
    report && typeof report === 'object' ? (report as { inputs?: unknown }).inputs : null;
  if (!inputs || typeof inputs !== 'object') return [];
  const data = inputs as {
    warnings?: unknown;
    feature_program?: unknown;
    studies?: unknown;
  };
  const warnings = Array.isArray(data.warnings)
    ? data.warnings.filter((item): item is string => typeof item === 'string' && !!item.trim())
    : [];
  const notes: FitInputNote[] = warnings.map((warning, index) => {
    const [zh, en] = fitInputWarningText(warning);
    return { key: `warning-${index}`, zh, en };
  });
  if (
    warnings.some((warning) =>
      warning.startsWith('the fit extracts features with a different colm-rs'),
    )
  )
    return notes;
  const current = programIdentity(data.feature_program);
  if (!current?.sha256) return notes;
  const studies = Array.isArray(data.studies) ? data.studies : [];
  studies.forEach((entry, index) => {
    if (!entry || typeof entry !== 'object') return;
    const study = entry as {
      id?: unknown;
      study?: unknown;
      frozen_feature_program?: unknown;
      model_revision?: unknown;
    };
    const frozen = programIdentity(study.frozen_feature_program);
    // 两端都有内容 hash 且不同才报；路径不同（快照副本）不算差异。
    if (!frozen?.sha256 || frozen.sha256 === current.sha256) return;
    const label =
      (typeof study.id === 'string' && study.id) ||
      (typeof study.study === 'string' && study.study) ||
      `#${index + 1}`;
    const revision = revisionIdentity(study.model_revision);
    notes.push({
      key: `program-${index}`,
      zh: `研究 ${label}${revision ? `（冻结修订 ${revision}）` : ''} 率定时用的特征提取程序与这次不同：冻结 ${frozen.path ?? '未知'}（sha256 ${shortHash(frozen.sha256)}），本次 ${current.path ?? '未知'}（sha256 ${shortHash(current.sha256)}）；同一份源码重编也会改变字节，确认这是有意的切换。`,
      en: `Study ${label}${revision ? ` (frozen revision ${revision})` : ''} was calibrated with a different feature-extraction program: frozen ${frozen.path ?? 'unknown'} (sha256 ${shortHash(frozen.sha256)}), extracting ${current.path ?? 'unknown'} (sha256 ${shortHash(current.sha256)}); a rebuild of the same source also changes the bytes, so confirm this is intended.`,
    });
  });
  return notes;
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
  /**
   * 后端 `weight`。界面上没有单独的权重输入框（单点目标按变量给权重），但从已有研究派生设计时
   * 必须原样继承：丢了权重就等于换了一个目标函数，而界面看不出来。缺省 1。
   */
  weight?: string;
  /** 后端 `key`。派生时继承来源研究的键，结果表与证据支持的列名才对得上。空着按变量+序号生成。 */
  key?: string;
  /** `grid` 目标的观测偏移（后端 `offset`，默认 0）。界面没有输入框；派生时继承，丢了会整体平移观测。 */
  offset?: string;
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
    const offset = draft.offset?.trim() ? Number(draft.offset) : 0;
    if (!Number.isFinite(offset))
      throw new Error(
        bilingual(
          `第 ${index + 1} 个目标的偏移量不是数`,
          `Target ${index + 1} has a non-numeric offset`,
        ),
      );
    source = {
      kind: 'grid',
      path,
      variable: draft.obsVariable.trim(),
      scale,
      ...(offset ? { offset } : {}),
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
  // 权重与键在校验里也要拦住坏值：它们直接进目标函数与结果列名，坏了比缺参数更难发现。
  const weight = draft.weight?.trim() ? Number(draft.weight) : 1;
  if (!Number.isFinite(weight) || weight <= 0)
    throw new Error(
      bilingual(
        `第 ${index + 1} 个目标的权重要是正数`,
        `Target ${index + 1} needs a positive weight`,
      ),
    );
  const key = draft.key?.trim() || `${variable}@${draft.kind}${index + 1}`;
  return {
    key,
    variable,
    metric: draft.metric || metric,
    weight,
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
      `一次要搜 ${parameters} 维（参数加网络权重），差分进化很难在有限预算里收敛；参数先用不确定性分析筛出最敏感的几个，网络可以减少输入特征或用线性网络。`,
      `Searching ${parameters} dimensions (parameters plus network weights) rarely converges on a limited budget; screen parameters with an uncertainty analysis first, and use fewer features or a linear network.`,
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

/**
 * 预检里最常见的拒绝：观测不够。后端用英文报，按固定文字识别后给中英两种说法；不认得的原样返回。
 * 按日、按月比时说清数的是日均、月均个数（复验 A2-03：默认 20 对，14 天的算例只有 14 个日均）。
 */
export function studyErrorText(error: string): [string, string] {
  const period = (p: string): [string, string] =>
    p === 'validation' ? ['验证期', 'validation period'] : ['率定期', 'calibration period'];
  const points =
    /target (\S+) \((\S+)\) has only (\d+) usable observation points in the (calibration|validation) window for (\S+); need at least (\d+)/.exec(
      error,
    );
  if (points) {
    const [, key, variable, count, which, site, need] = points;
    const [zh, en] = period(which!);
    return [
      `${site} 的 ${key}（${variable}）在${zh}里只有 ${count} 个可用观测，至少要 ${need} 个：放宽时段，或调低"每个站点至少多少对观测"。`,
      `${site} ${key} (${variable}) has only ${count} usable observations in the ${en}; at least ${need} are needed. Widen the period or lower the minimum number of pairs.`,
    ];
  }
  const means =
    /target (\S+) \((\S+)\) has only (\d+) (daily|monthly) means with enough observations in the (calibration|validation) window for (\S+); need at least (\d+)/.exec(
      error,
    );
  if (means) {
    const [, key, variable, count, unit, which, site, need] = means;
    const [zh, en] = period(which!);
    const [zhUnit, enUnit] = unit === 'daily' ? ['天', 'daily'] : ['个月', 'monthly'];
    return [
      `${site} 的 ${key}（${variable}）在${zh}里观测够用的只有 ${count} ${zhUnit}，至少要 ${need}：按${unit === 'daily' ? '日' : '月'}比时"每个站点至少多少对观测"数的是${unit === 'daily' ? '天数' : '月数'}，调低它或放宽时段。`,
      `${site} ${key} (${variable}) has only ${count} ${enUnit} means with enough observations in the ${en}; at least ${need} are needed. With ${enUnit} comparison the minimum counts ${unit === 'daily' ? 'days' : 'months'}: lower it or widen the period.`,
    ];
  }
  return [error, error];
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
  if (warning === 'no independent validation window was configured')
    return [
      '没有设独立的验证期：率定期上的改善有没有过拟合，这次检验不了。',
      'No independent validation period was set, so overfitting to the calibration period cannot be checked.',
    ];
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
    lines.push(
      budget.patience === 0
        ? ['跑满全部代数，不提前停止', 'runs every generation without stopping early']
        : [
            `连续 ${budget.patience} 代没有改善就提前停止`,
            `stops early after ${budget.patience} generations without improvement`,
          ],
    );
  if (n(budget.jobs) != null)
    lines.push([`每个研究同时 ${budget.jobs} 个模拟`, `${budget.jobs} simulations at a time`]);
  if (n(spec?.seed) != null) lines.push([`随机种子 ${spec!.seed}`, `seed ${spec!.seed}`]);
  // 派生研究（审计 P1-4）：第 4 步核对冻结设计时要能看见"这份设计从哪来"，
  // 否则用户核对的是预算，却不知道目标与观测是按另一项研究继承的。
  const parent = spec?.parent_study as
    { study?: unknown; model_revision?: { id?: unknown; revision?: unknown } | null } | undefined;
  if (parent && typeof parent.study === 'string' && parent.study) {
    const name = parent.study.split(/[/\\]/).filter(Boolean).pop() ?? parent.study;
    const revision = parent.model_revision;
    const revisionText =
      revision && typeof revision.id === 'string' && typeof revision.revision === 'number'
        ? [
            `（模型修订 ${revision.id} r${revision.revision}）`,
            ` (model revision ${revision.id} r${revision.revision})`,
          ]
        : ['', ''];
    lines.push([
      `派生自研究 ${name}${revisionText[0]}：目标、观测与时间窗按来源研究继承`,
      `Derived from study ${name}${revisionText[1]}: targets, observations and time windows inherited from the source`,
    ]);
  }
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

/** 网络要训练的权重个数：逐层 输入×输出 + 偏置，与后端 `HybridStudySpec::weight_count` 一致。 */
export function networkWeightCount(network: {
  features: unknown[];
  hidden: number[];
  outputs: unknown[];
}): number {
  const widths = [network.features.length, ...network.hidden, network.outputs.length];
  let count = 0;
  for (let i = 0; i + 1 < widths.length; i++) count += widths[i]! * widths[i + 1]! + widths[i + 1]!;
  return count;
}

/**
 * 进度条与文字。差分进化的计划总数只是上限：连续 `patience` 代没有改善就提前结束（后端缺省 3，
 * 0 表示跑满）。原来按上限画进度，提前收敛的研究停在"42 / 70"，看起来像被中断（GUI 审查 R9/C-07）。
 */
export function studyProgress(spec: Record<string, unknown> | null, done: number, status: string) {
  const total = plannedCandidates(spec);
  const finished = ['completed', 'completedwithfailures'].includes(
    status.replace(/_/g, '').toLowerCase(),
  );
  const budget = (spec?.budget ?? {}) as Record<string, unknown>;
  const de = spec?.method === 'differential-evolution';
  const patience = typeof budget.patience === 'number' ? budget.patience : 3;
  if (!total)
    return {
      percent: finished ? 100 : 0,
      zh: `已完成 ${done} 组参数`,
      en: `${done} parameter sets done`,
      note: null as [string, string] | null,
    };
  // 只有干净结束、确有成员跑完、且开了早停，少于上限才是收敛。带失败结束时少于上限多半是失败把研究
  // 挡住了（实测：基准算指标失败，0 组完成也写成"收敛了"，复验 A2-01/B2-01）。
  const clean = status.replace(/_/g, '').toLowerCase() === 'completed';
  if (de && finished && done < total)
    return {
      percent: 100,
      zh: `已完成 ${done} 组参数（最多 ${total} 组）`,
      en: `${done} parameter sets done (at most ${total})`,
      note: (clean && done > 0 && patience > 0
        ? [
            `连续 ${patience} 代没有明显改善，提前结束：这是收敛了，不是中断。`,
            `Stopped early after ${patience} generations without clear improvement: it converged, it was not interrupted.`,
          ]
        : [
            '有参数组失败，研究没有跑到上限就结束了：这不是收敛，先看下面的失败原因。',
            'Some parameter sets failed and the study ended before its limit: this is not convergence; check the failures below.',
          ]) as [string, string],
    };
  return {
    percent: Math.min(100, Math.round((done / total) * 100)),
    zh: de ? `已完成 ${done} / 最多 ${total} 组参数` : `已完成 ${done} / ${total} 组参数`,
    en: de
      ? `${done} of at most ${total} parameter sets done`
      : `${done} of ${total} parameter sets done`,
    note:
      de && !finished && patience > 0
        ? ([
            `连续 ${patience} 代没有改善会提前结束，实际可能少于上限。`,
            `It stops early after ${patience} generations without improvement, so it may run fewer.`,
          ] as [string, string])
        : null,
  };
}

/**
 * 每代的最优与"到这一代为止的最优"（不含基准、只算可行的）。到此为止的最优几代不动，再加代数也没用；
 * 最后几代还在降，说明预算给少了。
 */
export function convergenceRows(
  candidates:
    Record<string, { generation?: number; feasible?: boolean; calibration?: number }> | undefined,
) {
  const byGeneration = new Map<number, number>();
  for (const [id, c] of Object.entries(candidates ?? {})) {
    if (id === 'm000000' || c.feasible === false || typeof c.calibration !== 'number') continue;
    const g = c.generation ?? 0;
    byGeneration.set(g, Math.min(byGeneration.get(g) ?? Infinity, c.calibration));
  }
  let soFar = Infinity;
  return [...byGeneration.keys()]
    .sort((a, b) => a - b)
    .map((generation) => {
      const best = byGeneration.get(generation)!;
      soFar = Math.min(soFar, best);
      return { generation, best, bestSoFar: soFar };
    });
}

/**
 * 近优集合至少要这么多组才判读"定没定住"。只有一两组时范围天然是 0，会把没被观测约束的参数
 * 说成"定得住"——与这张表要回答的问题正相反（复验 B2-02：1 组时每个参数都是"0% · 定得住"）。
 */
export const NEAR_OPTIMAL_MIN_MEMBERS = 5;

/**
 * `near_optimal.json` → 每个参数在近优集合里的取值范围与它占抽样范围的比例。占得越宽，观测越定不住它。
 */
export function nearOptimalRows(value: unknown) {
  const data = (value ?? {}) as { members?: unknown; margin?: unknown; parameters?: unknown };
  const rows = (Array.isArray(data.parameters) ? data.parameters : []) as Record<string, unknown>[];
  const finiteNumber = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  const count = Array.isArray(data.members) ? data.members.length : 0;
  const tooFew = count < NEAR_OPTIMAL_MIN_MEMBERS;
  return {
    count,
    tooFew,
    margin: finiteNumber(data.margin) ?? 0.05,
    parameters: rows.map((row) => {
      const spread = finiteNumber(row.spread) ?? 0;
      return {
        parameter: String(row.parameter ?? ''),
        min: finiteNumber(row.min),
        max: finiteNumber(row.max),
        spread,
        pinned: tooFew ? 'few' : spread < 0.2 ? 'tight' : spread < 0.5 ? 'loose' : 'free',
      } as const;
    }),
  };
}

/** `cross_validation.json` → 每个拿掉的站：只按其余站挑出的成员，在这个站上比基准好还是差。 */
export function crossValidationRows(value: unknown) {
  const folds = ((value ?? {}) as { folds?: unknown }).folds;
  const num = (v: unknown) => (typeof v === 'number' && Number.isFinite(v) ? v : null);
  return (Array.isArray(folds) ? folds : []).map((raw) => {
    const fold = (raw ?? {}) as Record<string, unknown>;
    const heldOut = num(fold.held_out),
      baseline = num(fold.baseline);
    return {
      site: String(fold.site ?? ''),
      chosen: String(fold.chosen ?? ''),
      heldOut,
      baseline,
      better: heldOut != null && baseline != null ? heldOut < baseline : null,
    };
  });
}

/** 派生研究记录的来源，与后端 `study::spec::ParentStudy` 同形，写进新研究的 `spec.parent_study`。 */
export interface ParentStudyRef {
  study: string;
  spec_sha256: string;
  model_revision: VersionRef | null;
}

/** 从来源研究继承下来、要套回研究页控件上的设计。 */
export interface DerivedStudyUi {
  sites: string[];
  variables: string;
  metric: string;
  minPairs: string;
  corrected: boolean;
  aggregate: '' | 'daily' | 'monthly';
  weights: Record<string, string>;
  observations: string;
  from: string;
  to: string;
  valFrom: string;
  valTo: string;
  /** 格点算例的目标；单点研究为 null（目标由站点×变量重新生成）。 */
  gridTargets: GridTargetDraft[] | null;
  seed: string;
  kernelDir: string | null;
}

export interface DerivedStudyDesign {
  parent: ParentStudyRef;
  ui: DerivedStudyUi;
  /** 从来源研究继承了什么（中英各一句），调用方显示给用户核对。 */
  notes: [string, string][];
}

/** 后端目标规格的宽松读取；这里只认派生要用的字段，别的原样留在来源研究里。 */
type RawTarget = {
  key?: unknown;
  site?: unknown;
  variable?: unknown;
  metric?: unknown;
  weight?: unknown;
  from?: unknown;
  to?: unknown;
  validation_from?: unknown;
  validation_to?: unknown;
  min_pairs?: unknown;
  corrected?: unknown;
  aggregate?: unknown;
  source?: unknown;
};

const refusal = (zh: string, en: string): Error => new Error(bilingual(zh, en));

const inputWindow = (seconds: number) => new Date(seconds * 1000).toISOString().slice(0, 16);

/**
 * 来源研究所有目标共用的率定期/验证期。
 *
 * 研究页的时间窗是研究级设置（一对率定期、一对验证期），按目标逐条存储的来源设计只有全部一致
 * 才能无损继承；不一致就拒绝，不能挑第一条、也不能留空让界面自己重填。单点与空间目标共用这
 * 一条校验：空间目标的 from/to 原来派生时写成空字符串，界面上时间框空着，等于换了比较时段。
 */
function carriedWindow(targets: RawTarget[]) {
  const windows = new Set(
    targets.map(
      (t) =>
        `${String(t.from)}|${String(t.to)}|${String(t.validation_from ?? '')}|${String(t.validation_to ?? '')}`,
    ),
  );
  if (windows.size > 1)
    throw refusal(
      '来源研究各目标的率定期/验证期不一致，研究页按整项研究给时间窗，无法无损继承；请取消派生后手工新建。',
      'The source study uses different calibration/validation windows per target; this page sets one window for the study, so the design cannot be carried without loss. Start a fresh study instead.',
    );
  const first = targets[0];
  const seconds = (value: unknown) => (typeof value === 'number' ? value : null);
  return {
    from: seconds(first?.from),
    to: seconds(first?.to),
    valFrom: seconds(first?.validation_from),
    valTo: seconds(first?.validation_to),
  };
}

/**
 * 单点算例里目标按变量给权重、按研究给指标；同一变量在不同站点权重不同就无法用界面无损表达。
 *
 * 默认权重 1 也是目标函数的一部分，必须一起记下来比较：原来只记录非 1 的权重，`[1, 2]` 会被
 * 静默当成 `[2, 2]`（界面上按变量只有一个权重框）；输出 UI 时才丢掉 1，因为界面缺省就是 1。
 */
function singleSiteWeightMap(targets: RawTarget[]): Record<string, string> {
  const seen = new Map<string, number>();
  for (const target of targets) {
    const variable = String(target.variable ?? '');
    const weight =
      typeof target.weight === 'number' && Number.isFinite(target.weight) ? target.weight : 1;
    const known = seen.get(variable);
    if (known != null && known !== weight)
      throw refusal(
        `来源研究里 ${variable} 在不同站点的权重不一样（${known} / ${weight}），研究页按变量给权重，无法无损继承；请取消派生后手工新建。`,
        `The source study weights ${variable} differently per site (${known} / ${weight}); this page sets weights per variable, so the design cannot be carried without loss. Start a fresh study instead.`,
      );
    seen.set(variable, weight);
  }
  return Object.fromEntries(
    [...seen]
      .filter(([, weight]) => weight !== 1)
      .map(([variable, weight]) => [variable, String(weight)]),
  );
}

function singleSiteTargets(targets: RawTarget[], baseCases: string[]) {
  const minPairs = new Set(targets.map((t) => String(t.min_pairs ?? 30)));
  if (minPairs.size > 1)
    throw refusal(
      '来源研究各目标的最少观测配对数不一致，研究页只有一个设置，无法无损继承；请取消派生后手工新建。',
      'The source study uses different minimum pair counts per target; this page has one setting, so the design cannot be carried without loss. Start a fresh study instead.',
    );
  const metrics = new Set(targets.map((t) => String(t.metric ?? 'nrmse')));
  if (metrics.size > 1)
    throw refusal(
      '来源研究各目标用了不同的比较指标，研究页只有一个指标设置，无法无损继承；请取消派生后手工新建。',
      'The source study uses different metrics per target; this page has one setting, so the design cannot be carried without loss. Start a fresh study instead.',
    );
  const aggregates = new Set(targets.map((t) => String(t.aggregate ?? '')));
  if (aggregates.size > 1)
    throw refusal(
      '来源研究各目标的时间聚合不一致，研究页只有一个设置，无法无损继承；请取消派生后手工新建。',
      'The source study aggregates targets differently in time; this page has one setting, so the design cannot be carried without loss. Start a fresh study instead.',
    );
  const sites = new Set(targets.map((t) => String(t.site ?? '')).filter(Boolean));
  for (const site of sites)
    if (!baseCases.includes(site))
      throw refusal(
        `来源研究的目标引用了不在算例列表里的站点 ${site}，无法无损继承；请取消派生后手工新建。`,
        `A source-study target references ${site}, which is not one of its cases; the design cannot be carried without loss. Start a fresh study instead.`,
      );
  return {
    observed: [...sites],
    metric: [...metrics][0] ?? 'nrmse',
    minPairs: [...minPairs][0] ?? '20',
    aggregate: ([...aggregates][0] ?? '') as '' | 'daily' | 'monthly',
  };
}

/** 派生设计里"按变量做能量闭合订正"只有一个开关；来源研究逐目标不一致时拒绝而不是悄悄改。 */
function carriedCorrected(targets: RawTarget[], spatial: boolean) {
  const flux = (variable: string) => ['qle', 'qh'].includes(variable.trim().toLowerCase());
  const any = targets.some((t) => t.corrected === true);
  if (!any) return false;
  const consistent = targets.every((target) => {
    const expected = flux(String(target.variable ?? ''));
    // 格点产品/水文站目标没有订正版观测；只有站点含义的目标（单点或区域通量站）才可比。
    const stationLike = !target.source || (target.source as { kind?: unknown }).kind === 'station';
    return Boolean(target.corrected) === (stationLike && expected);
  });
  if (!consistent)
    throw refusal(
      '来源研究只给部分目标用了能量闭合订正（或订正了非湍流通量目标），研究页只有整项研究的开关，无法无损继承；请取消派生后手工新建。',
      'The source study corrects only some targets with the energy-balance closure (or corrects non-flux targets); this page has one study-wide switch, so the design cannot be carried without loss. Start a fresh study instead.',
    );
  if (spatial) return true;
  return true;
}

function gridTargetDraftFromTarget(target: RawTarget): GridTargetDraft {
  const source = (target.source ?? {}) as Record<string, unknown>;
  const kind = String(source.kind ?? '');
  if (kind !== 'grid' && kind !== 'station' && kind !== 'gauge')
    throw refusal(
      `来源研究的目标 ${String(target.key ?? '')} 种类不认识（${kind || '没有种类'}），无法无损继承；请取消派生后手工新建。`,
      `The source-study target ${String(target.key ?? '')} has an unknown kind (${kind || 'none'}), so the design cannot be carried without loss. Start a fresh study instead.`,
    );
  const path = String(source.path ?? '');
  const number = (value: unknown, fallback = '') =>
    typeof value === 'number' && Number.isFinite(value) ? String(value) : fallback;
  const draft: GridTargetDraft = {
    ...gridTargetDefaults[kind as GridTargetDraft['kind']],
    variable: String(target.variable ?? ''),
    path,
    metric: typeof target.metric === 'string' ? target.metric : undefined,
    ...(typeof target.weight === 'number' && target.weight !== 1
      ? { weight: String(target.weight) }
      : {}),
    ...(typeof target.key === 'string' ? { key: target.key } : {}),
  };
  if (kind === 'grid') {
    draft.obsVariable = String(source.variable ?? '');
    draft.scale = number(source.scale, '1') || '1';
    // 界面没有偏移输入框：非零偏移必须继承，否则派生研究的观测整体平移。
    if (typeof source.offset === 'number' && source.offset !== 0)
      draft.offset = String(source.offset);
    if (source.period === 'daily' || source.period === 'monthly') draft.period = source.period;
  } else {
    draft.lon = number(source.lon);
    draft.lat = number(source.lat);
    if (kind === 'station') {
      const offset = typeof source.utc_offset === 'number' ? source.utc_offset : 0;
      draft.utcOffset = offset ? String(offset) : '';
    } else {
      draft.obsVariable = typeof source.variable === 'string' ? source.variable : '';
      if (source.period === 'daily' || source.period === 'monthly') draft.period = source.period;
    }
  }
  return draft;
}

/**
 * 从一项已完成的研究派生新研究的设计（审计 P1-4）。
 *
 * 派生只换方法、参数子集与预算；算例、观测、目标（含权重、时间聚合、逐目标指标）、
 * 时间窗和模型修订都按来源研究继承。界面表达不了的组合（逐目标权重/指标/时段不同等）
 * 明确拒绝而不是悄悄丢掉——这正是审计复现的"月均目标变成逐时"那类静默变化。
 */
export function deriveStudyDesign(
  parentSpec: Record<string, unknown> | null,
  options: {
    study: string;
    specSha256: string;
    modelRevision: VersionRef | null;
    projectRoot: string;
  },
): DerivedStudyDesign {
  if (!parentSpec)
    throw refusal(
      '这个研究没有可读的冻结设计，不能派生。',
      'The study has no readable frozen design to derive from.',
    );
  if (!options.modelRevision)
    throw refusal(
      '这个研究没有记录所用的模型修订，派生会悄悄换模型；请从有模型修订的研究派生，或手工新建。',
      'This study did not record its model revision; deriving would silently switch models. Derive from a study with a recorded revision, or start fresh.',
    );
  if (parentSpec.hybrid)
    throw refusal(
      '来源研究训练了 AI 网络；当前"带入率定"只派生纯参数率定，网络会丢。请取消派生后手工新建。',
      'The source study trained an AI network; this carry only derives plain parameter calibration and would drop the network. Start a fresh study instead.',
    );
  if (String(parentSpec.site_mode ?? 'shared') !== 'shared')
    throw refusal(
      '来源研究是独立站点模式，研究页派生只支持共享参数。请取消派生后手工新建。',
      'The source study uses independent sites; derivation on this page only supports shared parameters. Start a fresh study instead.',
    );
  const baseCases = (Array.isArray(parentSpec.base_cases) ? parentSpec.base_cases : []).map(String);
  if (!baseCases.length)
    throw refusal('来源研究没有算例，不能派生。', 'The source study has no cases to derive from.');
  const rawTargets = Array.isArray(parentSpec.targets) ? (parentSpec.targets as RawTarget[]) : [];
  const spatial = rawTargets.some((target) => target.source);
  const separator = options.projectRoot.includes('/') ? '/' : '\\';
  const sites = baseCases.map((name) => `${options.projectRoot}${separator}${name}`);
  // 写进研究规格的键名是后端的 snake_case（`spec_sha256` / `model_revision`），不能把选项名原样透传。
  const parent: ParentStudyRef = {
    study: options.study,
    spec_sha256: options.specSha256,
    model_revision: options.modelRevision,
  };
  const corrected = carriedCorrected(rawTargets, spatial);
  const notes: [string, string][] = [];

  if (spatial) {
    const drafts = rawTargets.map(gridTargetDraftFromTarget);
    const stationAggregates = new Set(
      rawTargets
        .filter((target) => (target.source as { kind?: unknown } | undefined)?.kind === 'station')
        .map((target) => String(target.aggregate ?? '')),
    );
    if (stationAggregates.size > 1)
      throw refusal(
        '来源研究各通量站目标的时间聚合不一致，研究页只有一个设置，无法无损继承；请取消派生后手工新建。',
        'The source study aggregates flux-station targets differently in time; this page has one setting, so the design cannot be carried without loss. Start a fresh study instead.',
      );
    if (
      rawTargets.some(
        (target) =>
          target.aggregate && (target.source as { kind?: unknown } | undefined)?.kind !== 'station',
      )
    )
      throw refusal(
        '来源研究给格点产品或水文站目标写了时间聚合，研究页把它放在观测自身的时间分辨率里，无法无损继承；请取消派生后手工新建。',
        'The source study sets a time aggregation on gridded or gauge targets, which this page stores in the observation period instead; it cannot be carried without loss. Start a fresh study instead.',
      );
    const minPairs = new Set(rawTargets.map((t) => String(t.min_pairs ?? 30)));
    if (minPairs.size > 1)
      throw refusal(
        '来源研究各目标的最少观测配对数不一致，研究页只有一个设置，无法无损继承；请取消派生后手工新建。',
        'The source study uses different minimum pair counts per target; this page has one setting, so the design cannot be carried without loss. Start a fresh study instead.',
      );
    const aggregate = ([...stationAggregates][0] ?? '') as '' | 'daily' | 'monthly';
    // 空间目标的时间窗同样是研究级设置（spec() 把它套到每个格点目标上），从来源目标统一提取；
    // 各目标不一致由 carriedWindow 明确拒绝，不再写空字符串让界面自己重填。
    const window = carriedWindow(rawTargets);
    notes.push([
      `派生自研究 ${options.study.split(/[/\\]/).pop()}：${sites.length} 个格点算例、${drafts.length} 个目标按来源研究继承（含权重、逐目标指标与聚合）。`,
      `Derived from study ${options.study.split(/[/\\]/).pop()}: ${sites.length} gridded cases and ${drafts.length} targets are inherited from the source (weights, per-target metrics and aggregation included).`,
    ]);
    return {
      parent,
      notes,
      ui: {
        sites,
        variables: '',
        metric: typeof rawTargets[0]?.metric === 'string' ? rawTargets[0].metric : 'nrmse',
        minPairs: [...minPairs][0] ?? '20',
        corrected,
        aggregate,
        weights: {},
        observations: '',
        from: window.from != null && window.to != null ? inputWindow(window.from) : '',
        to: window.from != null && window.to != null ? inputWindow(window.to) : '',
        valFrom: window.valFrom != null ? inputWindow(window.valFrom) : '',
        valTo: window.valTo != null ? inputWindow(window.valTo) : '',
        gridTargets: drafts,
        seed: String(parentSpec.seed ?? 1),
        kernelDir:
          typeof parentSpec.kernel_dir === 'string' && parentSpec.kernel_dir
            ? parentSpec.kernel_dir
            : null,
      },
    };
  }

  if (
    rawTargets.length &&
    rawTargets.some((target) => !target.site || typeof target.site !== 'string')
  )
    throw refusal(
      '来源研究有些目标没有站点归属，无法无损继承；请取消派生后手工新建。',
      'The source study has gridded targets but some lack a site; the design cannot be carried without loss. Start a fresh study instead.',
    );
  const plan = singleSiteTargets(rawTargets, baseCases);
  const targetSites = plan.observed.length ? plan.observed : baseCases;
  // 单点目标要能按站点×变量重新生成：每个目标站点都得有观测文件，否则派生后会缺观测。
  const observations = (parentSpec.observations ?? {}) as Record<string, unknown>;
  const missing = targetSites.filter((site) => typeof observations[site] !== 'string');
  if (rawTargets.length && missing.length)
    throw refusal(
      `来源研究的站点 ${missing.join('、')} 没有记下观测文件，无法无损继承；请取消派生后手工新建。`,
      `The source study did not record observation files for ${missing.join(', ')}; the design cannot be carried without loss. Start a fresh study instead.`,
    );
  const variables = [
    ...new Set(rawTargets.map((target) => String(target.variable ?? '')).filter(Boolean)),
  ];
  // 界面上的单点目标只有"站点 × 变量"的完整组合（spec() 按 sites.flatMap(variables) 重建）。
  // 来源研究若是稀疏组合（Qle@site1 + GPP@site2），派生会凭空造出四个目标、悄悄换掉目标函数；
  // 界面表达不了就明确拒绝，而不是按并集重建。
  const pairs = new Set(rawTargets.map((t) => `${String(t.site)}|${String(t.variable ?? '')}`));
  const missingPairs = targetSites.flatMap((site) =>
    variables.filter((variable) => !pairs.has(`${site}|${variable}`)).map((v) => `${v}@${site}`),
  );
  if (rawTargets.length && missingPairs.length)
    throw refusal(
      `来源研究的目标不是完整的站点×变量组合（缺 ${missingPairs.slice(0, 4).join('、')}${missingPairs.length > 4 ? ' 等' : ''}），研究页只能按站点×变量重建全部组合，无法无损继承；请取消派生后手工新建。`,
      `The source study's targets are not the complete site × variable grid (missing ${missingPairs.slice(0, 4).join(', ')}${missingPairs.length > 4 ? ', …' : ''}); this page rebuilds every site × variable combination, so the design cannot be carried without loss. Start a fresh study instead.`,
    );
  const observationsText = targetSites
    .filter((site) => typeof observations[site] === 'string')
    .map((site) => `${site} | ${String(observations[site])}`)
    .join('\n');
  const { from, to, valFrom, valTo } = carriedWindow(rawTargets);
  const weights = singleSiteWeightMap(rawTargets);
  if (from != null && to != null)
    notes.push([
      `率定期按来源研究继承：${inputWindow(from)} 至 ${inputWindow(to)}${valFrom != null && valTo != null ? `；验证期 ${inputWindow(valFrom)} 至 ${inputWindow(valTo)}` : '（没有独立验证期）'}。`,
      `Calibration window inherited from the source: ${inputWindow(from)} to ${inputWindow(to)}${valFrom != null && valTo != null ? `; validation ${inputWindow(valFrom)} to ${inputWindow(valTo)}` : ' (no separate validation period)'}.`,
    ]);
  if (Object.keys(weights).length)
    notes.push([
      `目标权重按来源研究继承：${Object.entries(weights)
        .map(([variable, weight]) => `${variable} × ${weight}`)
        .join('、')}。`,
      `Target weights inherited from the source: ${Object.entries(weights)
        .map(([variable, weight]) => `${variable} × ${weight}`)
        .join(', ')}.`,
    ]);
  return {
    parent,
    notes,
    ui: {
      sites,
      variables: variables.join(', '),
      metric: plan.metric,
      minPairs: plan.minPairs,
      corrected,
      aggregate: plan.aggregate,
      weights,
      observations: observationsText,
      from: from != null && to != null ? inputWindow(from) : '',
      to: from != null && to != null ? inputWindow(to) : '',
      valFrom: valFrom != null ? inputWindow(valFrom) : '',
      valTo: valTo != null ? inputWindow(valTo) : '',
      gridTargets: null,
      seed: String(parentSpec.seed ?? 1),
      kernelDir:
        typeof parentSpec.kernel_dir === 'string' && parentSpec.kernel_dir
          ? parentSpec.kernel_dir
          : null,
    },
  };
}
