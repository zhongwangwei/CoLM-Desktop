import { bilingual } from '../../lib/bilingual';
import type { SaveState } from '../../lib/save-status';
import { remoteEndStatus } from '../runs/run-model';
import { fingerprintOf } from './fingerprint';

/** `bench_inspect` 读出的一个 history 目录。 */
export interface HistoryInfo {
  history_dir: string;
  prefix: string;
  tim_res: string;
  syear: number;
  eyear: number;
  spatial: boolean;
  lon: number | null;
  lat: number | null;
  files: number;
  /** 格点：[西, 东, 南, 北]（格子边界，度）与分辨率（度）；单点或读不出时为空。 */
  extent?: [number, number, number, number] | null;
  resolution?: number | null;
  /** history 文件按年/月/日切，还是 single；OpenBench 用它找文件。 */
  data_groupby?: string | null;
  /** 目录里的文件族（主 history 之外还有 tracer/unitcat 等时不参与探测，只是报告出来）。 */
  families?: {
    family: string;
    prefix: string;
    files: number;
    syear?: number | null;
    eyear?: number | null;
  }[];
  /** 显式空间类型：station/patch/grid/unstructured/unknown（不能只看维度长度是否大于 1）。 */
  spatial_type?: 'station' | 'patch' | 'grid' | 'unstructured' | 'unknown' | null;
  /** 每格 patch 数；>1 时单点比较不支持。 */
  patch?: number | null;
  /**
   * tim_res 从哪来：time=实测时间轴；metadata=单时刻时用文件里的频率元数据；
   * filename=单时刻且没有元数据时按文件名推断（界面要说明后两种不是实测）。
   */
  tim_res_source?: 'time' | 'metadata' | 'filename' | null;
  /** 文件里的频率元数据解析出的 tim_res（与实测冲突时也照实给出，界面提示以实测为准）。 */
  tim_res_metadata?: string | null;
  /** 格点跨日界线：OpenBench 的 [西, 东] 表示不了，探测会拒绝给范围。 */
  crosses_dateline?: boolean;
  /** 这个目录能不能进比较；不能时 blockers/reason 说明原因。 */
  comparable?: boolean;
  reason?: string | null;
  blockers?: { code: string; message: string }[];
  variables?: { name: string; dims: string[] }[];
  /** 带土层/积雪层/湖泊层维度（soil/soilsnow/lake）的变量：制备会保留维度，OpenBench profile 的
   * `compute` 可直接消费；页面只提示"映射到它能否用取决于 profile"，不再运行前拒绝。 */
  layered_variables?: { name: string; dims: string[] }[];
  /** 超出站点评估维度、且不属于层维度的变量（band/rtyp/vegnodes 等）：制备时会被跳过，运行前要能看到。 */
  unsupported_variables?: { name: string; dims: string[]; reason: string }[];
  warnings?: string[];
}

/** 比较里的一个模拟：一个算例（或别的 history 目录）的一次输出。 */
export interface BenchSimulation {
  /** 报告里显示的名字，也是 `simulation` 下的键。 */
  label: string;
  info: HistoryInfo;
  /** 这个模拟的 per-simulation 变量覆盖（新模块的新模拟变量）。 */
  variables?: Record<string, { varname: string; varunit: string }>;
  /** 手动登记时覆盖自动推断的分组/原生分辨率。 */
  dataGroupby?: string | null;
  gridRes?: number | null;
  /** OpenBench 模型 profile；缺省/空 = CoLM2024。 */
  profile?: string | null;
  /** 单点模拟的站点 ID（与参考数据里的站点比对）；缺省 = 从 history 前缀去掉 `_hist_` 推导。 */
  stationId?: string | null;
}

/**
 * 单点模拟要先并成站点文件的一条输入（与后端 `bench_run` 的 `sites` 参数一致）。
 * 站点目录、前缀和页面确认的坐标只在这份清单里，不在生成的 YAML 中：实际读哪份 history、
 * 用哪个坐标合并站点文件由它决定，所以它必须和配置一起冻结、一起进有效指纹（审查报告 R01）。
 */
export interface BenchSite {
  id: string;
  station: string;
  history_dir: string;
  prefix: string;
  lon: number | null;
  lat: number | null;
}

/** OpenBench 登记表里的一个参考数据集（`ref list --format json` 的一项，只取用得上的字段）。 */
export interface BenchReference {
  name: string;
  data_type?: string;
  tim_res?: string;
  variables: string[];
}

/**
 * 登记/编辑模拟弹窗里与保存有关的字段。字符串保持输入框里的原文：空白有"沿用探测到的值"的含义，
 * 由 `simulationInfoForSave` 统一解释，不能在输入时就变成 0/null。
 */
export interface SimulationFormValues {
  dir: string;
  dataType: 'auto' | 'grid' | 'stn';
  prefix: string;
  timRes: string;
  from: string;
  to: string;
  dataGroupby: string;
  gridRes: string;
  /** OpenBench 模型 profile；默认 CoLM2024（页面按需覆盖）。 */
  profile: string;
  /** 单点模拟的站点 ID；空 = 从 history 前缀推导。 */
  stationId: string;
  /** 单点坐标（度）：读取概况没给出或需要手填时用。 */
  lon: string;
  lat: string;
  /** 格点区域范围，按「西,东,南,北」四个数（度）；读不出时留空。 */
  extent: string;
  variables: { variable: string; varname: string; unit: string }[];
}

/** 表单里会出错、需要提示并聚焦的位置。 */
export type SimulationField =
  | 'dir'
  | 'dataType'
  | 'prefix'
  | 'timRes'
  | 'from'
  | 'to'
  | 'dataGroupby'
  | 'gridRes'
  | 'profile'
  | 'stationId'
  | 'lon'
  | 'lat'
  | 'extent'
  | 'variables';

/** 一条表单错误：页面按当前语言取其中一半，并把焦点放到对应字段（GUI-11）。 */
export interface SimulationIssue {
  field: SimulationField;
  message: [string, string];
}

/** 表单里的数：空白表示"沿用探测到的值"，不是 0；不是数时给 invalid。 */
function numberInput(text: string): 'blank' | 'invalid' | number {
  const trimmed = text.trim();
  if (!trimmed) return 'blank';
  const value = Number(trimmed);
  return Number.isFinite(value) ? value : 'invalid';
}

function integerInput(text: string): 'blank' | 'invalid' | number {
  const value = numberInput(text);
  if (typeof value !== 'number') return value;
  return Number.isInteger(value) ? value : 'invalid';
}

/**
 * 区域范围「西,东,南,北」→ 四元组。OpenBench 的 lat_range/lon_range 要按先小后大写；
 * 读不出范围时 `gridLines` 只写目标分辨率，不拿默认全球范围冒充已确认的区域（GUI-10）。
 */
export function parseExtent(text: string): [number, number, number, number] | null {
  const parts = text.split(/[,，\s]+/).filter(Boolean);
  if (parts.length !== 4) return null;
  const values = parts.map(Number);
  if (values.some((value) => !Number.isFinite(value))) return null;
  const [west, east, south, north] = values as [number, number, number, number];
  if (west < -180 || east > 180 || south < -90 || north > 90 || west > east || south > north)
    return null;
  return [west, east, south, north];
}

/** 探测到的范围写回表单；小数保留 6 位，免得 0.0083333333333 这类浮点尾巴占满输入框。 */
export function formatExtent(extent: [number, number, number, number] | null | undefined): string {
  if (!extent) return '';
  return extent.map((value) => String(Number(value.toFixed(6)))).join(',');
}

/**
 * 保存模拟前的校验。原来空目录、未确认类型、缺年份这些分支直接抛异常，按钮事件没人接，
 * 错误只进控制台（GUI-11）：现在页面把这里返回的第一个问题放进弹窗并聚焦字段。
 * 返回双语两半，因为纯函数拿不到 React 里的语言状态。
 */
export function validateSimulation(
  probe: HistoryInfo | null,
  values: SimulationFormValues,
): SimulationIssue | null {
  if (!values.dir.trim())
    return { field: 'dir', message: ['请先填模拟目录。', 'Enter the simulation folder first.'] };
  if (probe && probe.comparable === false) {
    const blocker = inspectBlockerText(probe);
    if (blocker) return { field: 'dir', message: blocker };
  }
  const auto = values.dataType === 'auto';
  if (auto && !probe)
    return {
      field: 'dataType',
      message: [
        '先读取目录概况，或明确选择格点/站点。',
        'Inspect the folder or choose grid/station first.',
      ],
    };
  const from = integerInput(values.from);
  if (from === 'invalid')
    return {
      field: 'from',
      message: ['起始年份要填整数。', 'The start year must be a whole number.'],
    };
  const to = integerInput(values.to);
  if (to === 'invalid')
    return { field: 'to', message: ['结束年份要填整数。', 'The end year must be a whole number.'] };
  const startYear = from === 'blank' ? (probe?.syear ?? null) : from;
  const endYear = to === 'blank' ? (probe?.eyear ?? null) : to;
  if (startYear == null || endYear == null)
    return { field: 'from', message: ['请填起止年份。', 'Enter start and end years.'] };
  if (startYear > endYear)
    return {
      field: 'from',
      message: ['起始年份不能晚于结束年份。', 'The start year cannot be after the end year.'],
    };
  if (!values.timRes.trim())
    return {
      field: 'timRes',
      message: ['时间分辨率不能为空。', 'The time resolution cannot be empty.'],
    };
  // 模型 profile：OpenBench 的 `model:` 字段；空值会让配置缺这个键，必须显式给。
  const profile = values.profile.trim();
  if (!profile)
    return {
      field: 'profile',
      message: [
        '模型 profile 不能为空（默认 CoLM2024）。',
        'The model profile cannot be empty (default CoLM2024).',
      ],
    };
  if (!/^[A-Za-z0-9_.-]+$/.test(profile))
    return {
      field: 'profile',
      message: [
        '模型 profile 只能用字母、数字、_、. 和 -。',
        'The model profile may only use letters, digits, _, . and -.',
      ],
    };
  const gridRes = numberInput(values.gridRes);
  if (gridRes === 'invalid' || (typeof gridRes === 'number' && gridRes <= 0))
    return {
      field: 'gridRes',
      message: [
        '原生网格分辨率要填正数（度）；留空表示沿用读到的值。',
        'The native grid resolution must be a positive number (degrees); leave it empty to use the inspected one.',
      ],
    };
  if (values.dataGroupby && !['Year', 'Month', 'Day', 'single'].includes(values.dataGroupby))
    return {
      field: 'dataGroupby',
      message: [
        '文件分组只能是 Year、Month、Day 或 single。',
        'File grouping must be Year, Month, Day or single.',
      ],
    };
  const spatial = auto ? (probe?.spatial ?? false) : values.dataType === 'grid';
  // 站点 ID：后端 valid_station 的约束（≤80 字符，不能有逗号/引号/换行）；空 = 从前缀推导。
  if (!spatial && values.stationId.trim()) {
    const station = values.stationId.trim();
    if (station.length > 80 || /[,"\n\r]/.test(station))
      return {
        field: 'stationId',
        message: [
          '站点 ID 不能超过 80 字符，也不能含逗号、引号或换行。',
          'The station ID must be at most 80 characters and cannot contain commas, quotes or line breaks.',
        ],
      };
  }
  if (spatial) {
    if (values.extent.trim() && !parseExtent(values.extent))
      return {
        field: 'extent',
        message: [
          '区域范围要按「西,东,南,北」填四个数（度），先小后大。',
          'Enter the region as four numbers (degrees): west, east, south, north, smaller first.',
        ],
      };
  } else {
    const lon = numberInput(values.lon);
    if (lon === 'invalid' || (typeof lon === 'number' && (lon < -180 || lon > 180)))
      return {
        field: 'lon',
        message: ['经度要填 -180 到 180 之间的数。', 'Longitude must be between -180 and 180.'],
      };
    const lat = numberInput(values.lat);
    if (lat === 'invalid' || (typeof lat === 'number' && (lat < -90 || lat > 90)))
      return {
        field: 'lat',
        message: ['纬度要填 -90 到 90 之间的数。', 'Latitude must be between -90 and 90.'],
      };
  }
  for (const row of values.variables) {
    const variable = row.variable.trim();
    const varname = row.varname.trim();
    // 只填了一半的行既不报错也不该静默丢掉：用户以为登记了变量覆盖，实际没进配置。
    if (!variable !== !varname)
      return {
        field: 'variables',
        message: [
          '变量覆盖要同时填标准变量和 NetCDF 变量名。',
          'A variable override needs both the standard variable and the NetCDF variable name.',
        ],
      };
  }
  return null;
}

/**
 * 把表单里明确编辑过的字段合并回**完整的探测对象**：没动的 extent/lon/lat/files/history_dir
 * 原样保留（GUI-10 的读取→保存丢失问题），而不是用表单里能拿到的字段重新拼一个 info。
 * 已有模拟与新增模拟走同一条合并逻辑（GUI-09），类型以表单里的 spatial 为准。
 */
export function simulationInfoForSave(
  probe: HistoryInfo | null,
  values: SimulationFormValues,
): HistoryInfo {
  const from = integerInput(values.from);
  const to = integerInput(values.to);
  const startYear = typeof from === 'number' ? from : probe?.syear;
  const endYear = typeof to === 'number' ? to : probe?.eyear;
  if (startYear == null || endYear == null)
    throw new Error(bilingual('请填起止年份。', 'Enter start and end years.'));
  const spatial =
    values.dataType === 'auto' ? (probe?.spatial ?? false) : values.dataType === 'grid';
  const gridRes = numberInput(values.gridRes);
  const lon = numberInput(values.lon);
  const lat = numberInput(values.lat);
  // 类型变了就把另一类才有的空间字段清掉：站点没有格点范围，格点也不该带上一段单点范围。
  const extent = spatial ? (parseExtent(values.extent) ?? probe?.extent ?? null) : null;
  return {
    // 探测结论（文件族/空间类型/patch/层维度/警告）要原样带过去：保存模拟时丢掉它们，
    // 运行前就无法再提示"哪些变量会被跳过""为什么不可比较"（审查报告第 2 条）。
    ...(probe ?? {}),
    history_dir: probe?.history_dir ?? values.dir.trim(),
    prefix: values.prefix.trim() || probe?.prefix || '',
    tim_res: values.timRes.trim() || probe?.tim_res || 'Day',
    syear: startYear,
    eyear: endYear,
    spatial,
    lon: typeof lon === 'number' ? lon : (probe?.lon ?? null),
    lat: typeof lat === 'number' ? lat : (probe?.lat ?? null),
    files: probe?.files ?? 0,
    extent,
    resolution: typeof gridRes === 'number' ? gridRes : (probe?.resolution ?? null),
    data_groupby: values.dataGroupby.trim() || probe?.data_groupby || null,
    // 用户显式改过类型时，探测里的 spatial_type 也要跟着改；否则界面一边说格点、
    // 一边保留"单点"的空间分类，摘要/阻断判断会自相矛盾。旧后端/旧夹具没有 spatial_type 时不要凭空补，
    // 保持"保存再打开不变"（GUI-09）。
    ...(probe?.spatial_type != null
      ? {
          spatial_type: spatial
            ? ('grid' as const)
            : probe.spatial_type === 'patch'
              ? ('patch' as const)
              : ('station' as const),
        }
      : {}),
  };
}

/**
 * 探测脚本报告"这个目录不能进比较"时的双语原因。按固定 code 翻译（后端 message 是英文诊断），
 * 不认得的 code 至少给出可操作的中文说明 + 原诊断，避免中文界面只看到英文。
 */
export function inspectBlockerText(info: {
  blockers?: { code: string; message: string }[];
  reason?: string | null;
  patch?: number | null;
}): [string, string] | null {
  const blocker = info.blockers?.[0];
  if (!blocker) return info.reason ? [info.reason, info.reason] : null;
  switch (blocker.code) {
    case 'multi_patch':
      return [
        `这个 history 每格有 ${info.patch ?? '多个'} 个 patch；比较页目前只支持单 patch，不能按单点比较。`,
        `This history has ${info.patch ?? 'multiple'} patches per grid cell; the comparison page supports a single patch and cannot treat it as one station.`,
      ];
    case 'unstructured':
      return [
        '这个 history 是非结构网格（element/ncol 向量），不能按单点或规则网格比较。',
        'This history is an unstructured (element/ncol) grid; it cannot be compared as a station or a regular grid.',
      ];
    case 'unknown_spatial_type':
      return [
        '从文件里看不出这是单点还是格点（没有经纬度维度/坐标，也没有 patch 维度）。',
        'The files do not say whether this is a station or a grid (no lat/lon dimensions or coordinates, and no patch dimension).',
      ];
    case 'dateline_crossing':
      return [
        '这个格点区域跨日界线，OpenBench 的 [西, 东] 范围写法表示不了；请先用不跨日界线的范围。',
        'This grid crosses the antimeridian, which OpenBench’s [west, east] range cannot express; use a region that does not cross it.',
      ];
    default:
      return [`这个目录不能进比较：${blocker.message}`, blocker.message];
  }
}

/** 探测脚本的警告（tim_res 来自文件名、缺经纬度、格点范围读不出）的双语说法。 */
export function inspectWarningText(info: { warnings?: string[] }): [string, string] | null {
  const warning = info.warnings?.[0];
  if (!warning) return null;
  if (warning.includes('finer than 1 h'))
    return [
      '这个 history 的输出比 1 小时更细，OpenBench 没有亚小时档位；按支持的最细档 Hour 比较，重采样到小时。',
      'This history is finer than 1 hour and OpenBench has no sub-hourly resolution; it is compared at Hour, the finest supported resolution, after resampling.',
    ];
  if (warning.includes('ignored frequency metadata'))
    return [
      '文件里的频率元数据不是 OpenBench 支持的档位，已忽略；时间分辨率按实测时间轴/文件名判断。',
      'The frequency metadata in the files is not an OpenBench tim_res and was ignored; the resolution follows the sampled time axis or the file names.',
    ];
  if (warning.includes('frequency metadata says'))
    return [
      '频率元数据与实测时间轴不一致；以实测时间轴为准（OpenBench 实际读文件，不读元数据）。',
      'The frequency metadata disagrees with the sampled time axis; the sampled axis wins because OpenBench reads the files, not the metadata.',
    ];
  if (warning.includes('single time step') && warning.includes('frequency metadata'))
    return [
      '这个 history 只有一个时刻，时间分辨率取自文件里的频率元数据。',
      'This history has a single time step; the time resolution was taken from the frequency metadata.',
    ];
  if (warning.includes('single time step'))
    return [
      '这个 history 只有一个时刻，时间分辨率是按文件名推断的。',
      'This history has a single time step; the time resolution was inferred from the file names.',
    ];
  if (warning.includes('no coordinates'))
    return [
      'history 文件里没有经纬度；保存模拟时请在编辑弹窗里填站点经纬度，制备站点文件会用它。',
      'The history files have no coordinates; fill in the station longitude/latitude in the simulation dialog, and preparing the station file will use them.',
    ];
  if (warning.includes('grid extent'))
    return [
      '读不出格点范围；请在编辑模拟里手动填区域范围，否则配置不会写区域。',
      'Could not read the grid extent; fill in the region in the simulation dialog, otherwise the configuration has no region.',
    ];
  return [warning, warning];
}

/**
 * 主 history 之外的文件族（tracer/unitcat/…）：不参与比较，但要在界面里说明存在，
 * 避免用户以为这些时刻也进了站点文件。
 */
export function inspectFamilyNote(info: {
  families?: { family: string; prefix: string; files: number }[];
}): [string, string] | null {
  const others = (info.families ?? []).filter((family) => family.family !== 'main');
  if (!others.length) return null;
  const text = others.map((family) => `${family.family}（${family.files} 个文件）`).join('、');
  const english = others.map((family) => `${family.family} (${family.files} files)`).join(', ');
  return [
    `同目录另有不参与比较的文件族：${text}（只合并主 history）。`,
    `Other file families in the same folder are not compared: ${english} (only the main history is merged).`,
  ];
}

/**
 * 打开编辑弹窗时用：把已保存的模拟（含完整探测结果）摊回表单。类型按**保存后**的 `spatial`
 * 显示，所以把格点改成站点、保存、再打开表单看到的是站点（GUI-09 的验收）。
 */
export function simulationFormForPick(pick: {
  dir: string;
  label: string;
  info: HistoryInfo | null;
  variables?: Record<string, { varname: string; varunit: string }>;
  dataGroupby?: string | null;
  gridRes?: number | null;
  profile?: string | null;
  stationId?: string | null;
}): {
  dir: string;
  label: string;
  dataType: 'auto' | 'grid' | 'stn';
  prefix: string;
  suffix: string;
  timRes: string;
  from: string;
  to: string;
  dataGroupby: string;
  gridRes: string;
  profile: string;
  stationId: string;
  lon: string;
  lat: string;
  extent: string;
  variables: SimulationFormValues['variables'];
} {
  const info = pick.info;
  return {
    dir: pick.dir,
    label: pick.label,
    dataType: info ? (info.spatial ? 'grid' : 'stn') : 'auto',
    prefix: info?.prefix ?? '',
    suffix: '',
    timRes: info?.tim_res ?? 'Day',
    from: info ? String(info.syear) : '',
    to: info ? String(info.eyear) : '',
    dataGroupby: pick.dataGroupby ?? info?.data_groupby ?? '',
    profile: pick.profile?.trim() || 'CoLM2024',
    stationId: pick.stationId ?? '',
    gridRes:
      pick.gridRes != null
        ? String(pick.gridRes)
        : info?.resolution != null
          ? String(info.resolution)
          : '',
    lon: info?.lon != null ? String(info.lon) : '',
    lat: info?.lat != null ? String(info.lat) : '',
    extent: formatExtent(info?.extent),
    variables: Object.entries(pick.variables ?? {}).map(([variable, mapping]) => ({
      variable,
      varname: mapping.varname,
      unit: mapping.varunit,
    })),
  };
}

/**
 * 名字里有没有非 ASCII 字符（中文等）。ESLint 的 no-control-regex 不接受 [\x00-\x7F] 这种
 * 控制字符范围；`\p{ASCII}` 的属性转义等价且不触发规则。
 */
const hasNonAscii = (text: string) => !/^[\p{ASCII}]*$/u.test(text);

/**
 * 名字只留字母、数字和下划线：它要当 YAML 的键、站点 ID 和文件名。
 * 显示名里有非 ASCII（中文等）时，按字符清洗会塌成同一个 `sim_run`，不同模拟被报重名、共用站点文件；
 * 追加显示名指纹保持稳定且互不相同（审查报告：中文名称身份碰撞）。纯 ASCII 名的键保持原样，
 * 已有比较的目录与配置不迁移。
 */
export function simulationKey(label: string): string {
  const key = label.replace(/[^A-Za-z0-9_]+/g, '_').replace(/^_+|_+$/g, '');
  const base = /^[A-Za-z]/.test(key) ? key : `sim_${key || 'run'}`;
  return hasNonAscii(label) ? `${base}_${fingerprintOf(label).slice(0, 6)}` : base;
}

/** `ref list --format json` 的输出各版本略有不同（数组，或 `{datasets: [...]}`）；统一成 [`BenchReference`]。 */
export function parseReferences(value: unknown): BenchReference[] {
  const rows = Array.isArray(value)
    ? value
    : value && typeof value === 'object'
      ? (Object.values(value as Record<string, unknown>).find(Array.isArray) ?? [])
      : [];
  return (rows as Record<string, unknown>[])
    .map((row) => {
      const variables = row.variables;
      const names = Array.isArray(variables)
        ? variables.map((v) =>
            typeof v === 'string' ? v : String((v as { name?: unknown })?.name ?? ''),
          )
        : variables && typeof variables === 'object'
          ? Object.keys(variables as Record<string, unknown>)
          : [];
      return {
        name: String(row.name ?? ''),
        data_type: typeof row.data_type === 'string' ? row.data_type : undefined,
        tim_res: typeof row.tim_res === 'string' ? row.tim_res : undefined,
        variables: names.filter(Boolean),
      };
    })
    .filter((row) => row.name);
}

/**
 * 一组格点模拟覆盖范围的并集（西, 东, 南, 北；向外取到 0.01 度，免得边上的格子被截掉）。
 * 没有可读范围时 null。配置里的 lat_range/lon_range 和运行前条件摘要共用这一个结果（U-04）。
 */
export function unionRegion(
  simulations: BenchSimulation[],
): [number, number, number, number] | null {
  const extents = simulations
    .filter((s) => s.info.spatial && s.info.extent)
    .map((s) => s.info.extent!)
    // 只并"能当范围用"的 extent。西>东或越界的值不再钳到 ±180（审查报告：0–360 经度会生成
    // lon_range [199.5, 180] 这种西界大于东界的配置）；不可用的范围由页面按"缺范围"提示。
    .filter(
      (extent) =>
        extent.every((value) => Number.isFinite(value)) &&
        extent[0] >= -180 &&
        extent[1] <= 180 &&
        extent[0] < extent[1] &&
        extent[2] >= -90 &&
        extent[3] <= 90 &&
        extent[2] < extent[3],
    );
  if (!extents.length) return null;
  const round = (v: number, up: boolean) => (up ? Math.ceil(v * 100) : Math.floor(v * 100)) / 100;
  return [
    round(Math.min(...extents.map((e) => e[0])), false),
    round(Math.max(...extents.map((e) => e[1])), true),
    round(Math.min(...extents.map((e) => e[2])), false),
    round(Math.max(...extents.map((e) => e[3])), true),
  ];
}

/**
 * 有格点模拟时按它们覆盖的范围（并集）裁，按给定分辨率比。不写时 OpenBench 按全球 0.5 度比：2026-10-23 在 7920
 * 上实测 GreaterBay 1 km 对 ERA5-Land，全球格网里只有 4 个格子有值；裁到模拟范围、按 0.25 度比是 39 个陆地格子。
 * 范围向外取到 0.01 度，免得边上的格子被截掉。
 *
 * 读不出范围时只写目标分辨率（GUI-10）：OpenBench 会按默认全球范围比，这比把默认范围写成"已确认区域"诚实；
 * 页面另有提示说明哪些模拟缺范围。
 */
function gridLines(simulations: BenchSimulation[], gridRes?: number | null): string[] {
  const region = unionRegion(simulations);
  const lines: string[] = [];
  if (region)
    lines.push(
      `  lat_range: [${region[2]}, ${region[3]}]`,
      `  lon_range: [${region[0]}, ${region[1]}]`,
    );
  // 目标分辨率不依赖范围：只有单点模拟时才有意不写 grid_res。
  if (gridRes && gridRes > 0 && simulations.some((s) => s.info.spatial))
    lines.push(`  grid_res: ${gridRes}`);
  return lines;
}

/**
 * 比较网格的缺省分辨率：所选格点模拟里最粗的那个，但不细于 0.25 度。参考产品大多是 0.25 度或更粗，
 * 把它们插到 1 km 上比只是放大误差、拖慢计算。
 */
export function defaultGridRes(simulations: { spatial: boolean; resolution?: number | null }[]) {
  const resolutions = simulations
    .filter((s) => s.spatial && s.resolution)
    .map((s) => s.resolution!);
  if (!resolutions.length) return null;
  return Math.max(0.25, Math.round(Math.max(...resolutions) * 1000) / 1000);
}

/** YAML 里的字符串一律用 JSON 的双引号写法：路径里有空格、冒号、`#` 也不会被读错。 */
const quote = (value: string) => JSON.stringify(value);

/**
 * OpenBench 3.0.8 的 `comparison.items` 合法图表名（v3.0.8 源码 figlib.yaml 的 comparison_nml
 * 加 check.py 的 _BASIC_ANALYSIS_ITEMS）。写进配置前在本地校验名字，避免把拼错的名字交给
 * OpenBench check 才发现。`needs` 只标注能确定输出依赖的两类：Taylor 用 metrics、HeatMap 用 scores
 * （v3.0.8 `_comparison_heatmap.py` 有显式校验）；其余图表缺数据时由 OpenBench check 明确报错。
 */
export interface ComparisonItemChoice {
  name: string;
  needs: 'metrics' | 'scores' | null;
}
export const COMPARISON_ITEMS: ComparisonItemChoice[] = [
  { name: 'Taylor_Diagram', needs: 'metrics' },
  { name: 'HeatMap', needs: 'scores' },
  { name: 'Target_Diagram', needs: null },
  { name: 'Kernel_Density_Estimate', needs: null },
  { name: 'Whisker_Plot', needs: null },
  { name: 'Parallel_Coordinates', needs: null },
  { name: 'Portrait_Plot_seasonal', needs: null },
  { name: 'Single_Model_Performance_Index', needs: null },
  { name: 'Relative_Score', needs: null },
  { name: 'Ridgeline_Plot', needs: null },
  { name: 'Diff_Plot', needs: null },
  { name: 'RadarMap', needs: null },
  { name: 'Mann_Kendall_Trend_Test', needs: null },
  { name: 'Correlation', needs: null },
  { name: 'Standard_Deviation', needs: null },
  { name: 'Functional_Response', needs: null },
  { name: 'Basic', needs: null },
  { name: 'Mean', needs: null },
  { name: 'Median', needs: null },
  { name: 'Min', needs: null },
  { name: 'Max', needs: null },
  { name: 'Sum', needs: null },
];
export const VALID_COMPARISON_ITEMS = new Set(COMPARISON_ITEMS.map((item) => item.name));
/** 页面默认勾选的两个（OpenBench 自己的默认值）；一个都不勾 = 不写 comparison 段。 */
export const DEFAULT_COMPARISON_ITEMS = ['Taylor_Diagram', 'HeatMap'];
/** 界面上实际给出的选项：只放图表类，分析量（Mean/Median/…）不让用户误以为是图。 */
export const COMPARISON_ITEM_CHOICES = COMPARISON_ITEMS.filter(
  (item) => !['Basic', 'Mean', 'Median', 'Min', 'Max', 'Sum'].includes(item.name),
);

/**
 * OpenBench 3.0.8 `statistics.items` 的合法方法名：figlib.yaml 的 statistic_nml（名字去掉
 * `_source`）+ check.py 的 `_BASIC_ANALYSIS_ITEMS` {Basic,Mean,Median,Max,Min,Sum} +
 * 单独放行的 False_Discovery_Rate。统计阶段是独立分析，页面默认不启用（opt-in）。
 */
export const STATISTICS_ITEMS = [
  'Basic',
  'Mann_Kendall_Trend_Test',
  'Correlation',
  'Standard_Deviation',
  'Functional_Response',
  'Z_Score',
  'Three_Cornered_Hat',
  'Hellinger_Distance',
  'Partial_Least_Squares_Regression',
  'False_Discovery_Rate',
  'ANOVA',
  'Mean',
  'Median',
  'Max',
  'Min',
  'Sum',
] as const;
export const VALID_STATISTICS_ITEMS = new Set<string>(STATISTICS_ITEMS);

/**
 * 类别/结构维度变量预检：模拟的变量覆盖把标准变量映射到了 `unsupported_variables` 里的 NetCDF 变量时，
 * 制备站点文件会跳过它（不是静默丢，但运行到 check 才发现太晚）。运行/生成前明确拒绝，并指出是哪个
 * 模拟的哪个变量、什么维度（审查报告 C05：预检时明确拒绝，而不是制备成功但变量没了）。
 */
export function layerVariableProblems(design: ComparisonDesign): [string, string][] {
  const problems: [string, string][] = [];
  for (const simulation of design.simulations) {
    const unsupported = new Map(
      (simulation.info.unsupported_variables ?? []).map((variable) => [
        variable.name,
        variable.dims,
      ]),
    );
    if (!unsupported.size) continue;
    for (const [variable, mapping] of Object.entries(simulation.variables ?? {})) {
      if (!(variable in design.variables)) continue;
      const varname = mapping.varname?.trim();
      if (!varname || !unsupported.has(varname)) continue;
      const dims = (unsupported.get(varname) ?? []).join(', ');
      problems.push([
        `变量 ${variable} 在模拟「${simulation.label}」里是 ${varname}（维度 ${dims}），超出站点/地表评估维度，制备时会被跳过。请先换一个变量，或等支持层维度选择后再比较。`,
        `Variable ${variable} maps to ${varname} in simulation “${simulation.label}” (dims ${dims}), beyond a station/surface evaluation; preparing the station file would skip it. Pick another variable or wait for layer selection support.`,
      ]);
    }
  }
  return problems;
}

/**
 * 层维度变量提示（不是阻断）：用户显式把标准变量映射到带 soilsnow/soil/lake 维的 NetCDF 变量时，
 * 站点合并文件会原样保留这些维度；能不能用取决于 OpenBench 模型 profile 里有没有对应的 compute/层处理
 * （3.0.8 的 CoLM2024 profile 对 Root_Zone_Soil_Moisture 等确实有）。错误的 varname 到运行阶段才会报
 * "找不到变量"，所以这里保留一条可读提示，但不再像 R07 那样把整次比较提前拒绝。
 */
export function layerVariableWarnings(design: ComparisonDesign): [string, string][] {
  const warnings: [string, string][] = [];
  for (const simulation of design.simulations) {
    const layered = new Map(
      (simulation.info.layered_variables ?? []).map((variable) => [variable.name, variable.dims]),
    );
    if (!layered.size) continue;
    for (const [variable, mapping] of Object.entries(simulation.variables ?? {})) {
      if (!(variable in design.variables)) continue;
      const varname = mapping.varname?.trim();
      if (!varname || !layered.has(varname)) continue;
      const dims = (layered.get(varname) ?? []).join(', ');
      warnings.push([
        `变量 ${variable} 在模拟「${simulation.label}」里映射到带层维度的 ${varname}（维度 ${dims}）：站点文件会保留这些维度，只有 OpenBench 模型 profile 对该变量有 compute/层处理时才可用；否则 OpenBench check 会报找不到变量。`,
        `Variable ${variable} maps to the layered ${varname} (dims ${dims}) in simulation “${simulation.label}”: the station file keeps these dimensions, so it only works when the OpenBench model profile computes this variable from them; otherwise openbench check reports the variable is missing.`,
      ]);
    }
  }
  return warnings;
}

/**
 * statistics 阶段的前置提示：OpenBench 3.0.8 的统计检验在格点场上算，只有单点模拟时该阶段
 * 实测不产出任何结果、也不报错（本仓库真实链路验证过）。页面不阻止用户选，只提前说明会白选。
 */
export function statisticsStationWarning(simulations: BenchSimulation[]): [string, string] | null {
  const usable = simulations.filter((sim) => sim.info && sim.info.comparable !== false);
  if (!usable.length || usable.some((sim) => sim.info.spatial)) return null;
  return [
    'OpenBench 3.0.8 的统计检验阶段基于格点场：当前只有单点模拟，不会产出统计结果（实测无输出、也不报错）。需要统计结果的话，请至少加入一个格点模拟。',
    'OpenBench 3.0.8 computes statistics on gridded fields: with station simulations only, this phase produces no output (observed silently). Add at least one gridded simulation to get statistics.',
  ];
}

/**
 * 生成 `openbench.yaml` 与站点清单所需的全部输入。`MachineIdentity` 之外的字段全进配置文本；
 * 同一份比较输入换机器/换参考数据根后就是另一次评估，指纹（`designFingerprint`）因此分开算。
 */
export interface ComparisonDesign {
  name: string;
  years: [number, number];
  timRes: string;
  variables: Record<string, string | string[]>;
  simulations: BenchSimulation[];
  /** 这个比较的目录（绝对路径）：单点模拟的站点文件与清单放在它的 `sim/` 下。 */
  projectDir: string;
  /** 比较用的网格分辨率（度）；有格点模拟时写进 `grid_res`。 */
  gridRes?: number | null;
  /** OpenBench 项目级选项；空表示用 OpenBench 默认值。 */
  weight?: string | null;
  minYear?: number | null;
  timeAlignment?: string | null;
  numCores?: number | null;
  metrics?: string[];
  scores?: string[];
  /** 跨模拟比较（OpenBench comparison 阶段）要生成的图表；空/不传 = 不写这一段（默认关闭）。 */
  comparisonItems?: string[];
  /** OpenBench 统计检验阶段（statistics）的方法；空/不传 = 不写这一段（默认关闭）。 */
  statisticsItems?: string[];
}

/** 评估发生在哪台机器、哪个 OpenBench 安装、哪个参考数据根下。 */
export interface MachineIdentity {
  host: string;
  openbenchRoot: string;
  refRoot: string;
}

/** 一次评估的完整有效设计：比较输入 + 机器/路径身份。GUI-04 要求全部进去指纹。 */
export interface EvaluationDesign extends ComparisonDesign {
  machine: MachineIdentity;
}

/**
 * 生成 `openbench.yaml`。格点模拟直接指向 history 目录（按月一个文件）；单点模拟先由后端并成一个站点文件，
 * 这里写站点清单 `<比较目录>/sim/<键>.csv`（绝对路径，原因见下）。
 * 返回配置文本、要先并成站点文件的单点模拟，以及读不出区域范围的格点模拟（页面据此说明原因，GUI-10）。
 */
export function buildOpenbenchConfig(design: ComparisonDesign) {
  const [from, to] = design.years;
  if (!Number.isInteger(from) || !Number.isInteger(to) || from > to)
    throw new Error(bilingual('评估年份要先小后大', 'The evaluation years must run forward'));
  // 变量与每模拟覆盖都按名字排序：输出顺序不随勾选先后变化，指纹（不区分对象键顺序）才和配置文本一一对应。
  const variables = Object.entries(design.variables)
    .sort(([left], [right]) => left.localeCompare(right))
    .map(
      ([variable, refs]) =>
        [
          variable,
          (Array.isArray(refs) ? refs : [refs]).filter((ref) => typeof ref === 'string' && ref),
        ] as const,
    )
    .filter(([, refs]) => refs.length > 0);
  if (!variables.length)
    throw new Error(
      bilingual('至少选一个变量和它的参考数据', 'Choose at least one variable and its reference'),
    );
  if (!design.simulations.length)
    throw new Error(bilingual('至少选一个模拟', 'Choose at least one simulation'));
  // OpenBench 把缺省和空列表分开：不写用默认，写 [] 表示一个都不要，两者都 [] 直接报错。
  if (design.metrics?.length === 0 && design.scores?.length === 0)
    throw new Error(bilingual('指标和评分至少勾一个', 'Tick at least one metric or score'));
  const comparisonItems = [...new Set((design.comparisonItems ?? []).filter(Boolean))];
  for (const name of comparisonItems)
    if (!VALID_COMPARISON_ITEMS.has(name))
      throw new Error(
        bilingual(
          `不是 OpenBench 3.0.8 支持的比较图表：${name}`,
          `OpenBench 3.0.8 does not support the comparison item ${name}`,
        ),
      );
  for (const item of design.simulations.length >= 2 ? COMPARISON_ITEMS : []) {
    if (!comparisonItems.includes(item.name)) continue;
    if (item.needs === 'metrics' && !design.metrics?.length)
      throw new Error(
        bilingual(
          `${item.name} 至少需要一个指标；先在指标里勾选。`,
          `${item.name} needs at least one metric; tick a metric first.`,
        ),
      );
    if (item.needs === 'scores' && !design.scores?.length)
      throw new Error(
        bilingual(
          `${item.name} 至少需要一个评分；先在评分里勾选。`,
          `${item.name} needs at least one score; tick a score first.`,
        ),
      );
  }
  const statisticsItems = [...new Set((design.statisticsItems ?? []).filter(Boolean))];
  for (const name of statisticsItems)
    if (!VALID_STATISTICS_ITEMS.has(name))
      throw new Error(
        bilingual(
          `不是 OpenBench 3.0.8 支持的统计方法：${name}`,
          `OpenBench 3.0.8 does not support the statistics item ${name}`,
        ),
      );
  // 层维度变量在生成配置前就拒绝：制备阶段只会跳过它们，等到 check 失败太晚（C05）。
  const layerIssues = layerVariableProblems(design);
  if (layerIssues.length)
    throw new Error(
      bilingual(
        layerIssues.map(([zh]) => zh).join('；'),
        layerIssues.map(([, en]) => en).join('; '),
      ),
    );
  const keys = design.simulations.map((sim) => simulationKey(sim.label));
  if (new Set(keys).size !== keys.length)
    throw new Error(bilingual('模拟的名字不能重复', 'Simulation names must be unique'));
  const missingExtent = design.simulations
    .filter((sim) => sim.info.spatial && !sim.info.extent)
    .map((sim) => sim.label);
  const lines = [
    '# CoLM Desktop 生成；可以先改再运行。',
    'project:',
    // OpenBench 的 project.name 决定输出子目录；它必须和后端比较目录（comparisonName）一致，
    // 否则页面/报告里的"项目"与磁盘上的 projects/<名字> 对不上（R11 遗留的命名差异）。
    `  name: ${quote(comparisonName(design.name))}`,
    '  output_dir: ./output',
    `  years: [${from}, ${to}]`,
    `  tim_res: ${design.timRes}`,
    ...gridLines(design.simulations, design.gridRes),
    ...(design.weight ? [`  weight: ${design.weight}`] : []),
    ...(design.minYear != null && design.minYear >= 0
      ? [`  min_year_threshold: ${design.minYear}`]
      : []),
    ...(design.timeAlignment ? [`  time_alignment: ${design.timeAlignment}`] : []),
    ...(design.numCores != null && design.numCores >= 0 ? [`  num_cores: ${design.numCores}`] : []),
    'evaluation:',
    `  variables: [${variables.map(([variable]) => variable).join(', ')}]`,
    'reference:',
  ];
  for (const [variable, refs] of variables) {
    if (refs.length === 1) lines.push(`  ${variable}: ${quote(refs[0]!)}`);
    else {
      lines.push(`  ${variable}:`);
      for (const ref of refs) lines.push(`    - ${quote(ref)}`);
    }
  }
  lines.push('simulation:');
  const sites: BenchSite[] = [];
  design.simulations.forEach((sim, i) => {
    const key = keys[i]!;
    // 模型 profile 默认 CoLM2024；用户可换（OpenBench check 会校验 profile 是否存在）。
    const profile = sim.profile?.trim() || 'CoLM2024';
    const model = /^[A-Za-z0-9_.-]+$/.test(profile) ? profile : quote(profile);
    lines.push(`  ${key}:`, `    model: ${model}`);
    if (sim.info.spatial) {
      const groupby =
        sim.dataGroupby || sim.info.data_groupby || (sim.info.files === 1 ? 'single' : 'Month');
      const native = sim.gridRes ?? sim.info.resolution;
      lines.push(
        `    root_dir: ${quote(sim.info.history_dir)}`,
        `    prefix: ${quote(sim.info.prefix)}`,
        `    data_groupby: ${groupby}`,
        '    data_type: grid',
        `    tim_res: ${sim.info.tim_res}`,
      );
      if (native != null && native > 0) lines.push(`    grid_res: ${native}`);
    } else {
      // 站点 ID 用站点自己的名字（history 前缀去掉 `_hist_`，如 AT-Neu）：OpenBench 按 ID 和参考数据里的站点
      // 对上，整理过的键（AT_Neu）对不上。
      // 站点 ID：页面可显式覆盖（模型版本/前缀与观测站点 ID 不必同名）；空则沿用旧规则从前缀推导。
      const station = sim.stationId?.trim() || sim.info.prefix.replace(/_hist_$/, '') || key;
      // 编辑模拟弹窗允许修正经纬度：界面显示的是这组值，实际计算也必须用它，而不是再读文件里的旧值。
      sites.push({
        id: key,
        station,
        history_dir: sim.info.history_dir,
        prefix: sim.info.prefix,
        lon: Number.isFinite(sim.info.lon) ? sim.info.lon : null,
        lat: Number.isFinite(sim.info.lat) ? sim.info.lat : null,
      });
      // 用绝对路径：OpenBench 3.0.8 的 `check` 按 root_dir 解析 fulllist，`run` 却按工作目录打开它，
      // 相对路径总有一边找不到（2026-10-23 窗口实测：sim/sim/x.csv 与 x.csv 各错一次）。
      const sep = design.projectDir.includes('\\') && !design.projectDir.includes('/') ? '\\' : '/';
      const simDir = `${design.projectDir}${sep}sim`;
      lines.push(
        `    root_dir: ${quote(simDir)}`,
        '    data_type: stn',
        '    data_groupby: single',
        `    tim_res: ${sim.info.tim_res}`,
        `    fulllist: ${quote(`${simDir}${sep}${key}.csv`)}`,
      );
    }
    const overrides = Object.entries(sim.variables ?? {})
      .filter(([, mapping]) => mapping.varname.trim())
      .sort(([left], [right]) => left.localeCompare(right));
    if (overrides.length) {
      lines.push('    variables:');
      for (const [variable, mapping] of overrides) {
        lines.push(
          `      ${quote(variable)}:`,
          `        varname: ${quote(mapping.varname)}`,
          `        varunit: ${quote(mapping.varunit)}`,
        );
      }
    }
  });
  // 勾选结果原样写出：用户全部取消勾选指标、只要评分时，省掉这一行会让 OpenBench 改跑默认指标。
  if (design.metrics) lines.push(`metrics: [${design.metrics.map(quote).join(', ')}]`);
  if (design.scores) lines.push(`scores: [${design.scores.map(quote).join(', ')}]`);
  // 跨模拟比较：至少两个模拟才有意义；一个图表都不勾就不写这一段（OpenBench 默认 enabled=false）。
  // 写 enabled: true 而不是省略 items：页面明确选中的图表要进配置，不能靠 OpenBench 的缺省值。
  if (design.simulations.length >= 2 && comparisonItems.length)
    lines.push(
      'comparison:',
      '  enabled: true',
      `  items: [${comparisonItems.map(quote).join(', ')}]`,
    );
  // 统计检验阶段：页面明确勾选的方法才写；一个都不勾就不写这一段（OpenBench 默认关闭）。
  if (statisticsItems.length)
    lines.push(
      'statistics:',
      '  enabled: true',
      `  items: [${statisticsItems.map(quote).join(', ')}]`,
    );
  return { config: `${lines.join('\n')}\n`, sites, missingExtent };
}

/**
 * 一次评估设计的稳定指纹：把**生成配置与运行所依赖的全部输入**（目录、类型、前缀、时间、分组、
 * 空间范围与分辨率、变量映射、参考数据、指标评分，以及机器/OpenBench/参考数据路径身份）压成一个
 * 短串。生成配置时记下它，运行前再算一遍；只是目录和显示名不变、内部输入变了也能发现（GUI-04）。
 * 算法本体在 `fingerprint.ts`：手改后的有效配置（`config.ts`）必须用同一套规范化/哈希，判断才一致。
 */
export function designFingerprint(design: EvaluationDesign): string {
  return fingerprintOf(design);
}

/**
 * 生成配置之后设置又变了时怎么处理：没手改过的自动配置直接作废（重新生成即可）；手改过的保留文本，
 * 由页面显示"已过期、不能直接运行"并给重新生成入口，绝不静默覆盖用户的修改（GUI-04）。
 */
export function configInvalidation(args: {
  currentFingerprint: string;
  generated: { fingerprint: string; config: string } | null;
  yaml: string;
}): 'none' | 'clear' | 'keep-edited' {
  const { currentFingerprint, generated, yaml } = args;
  if (!generated || !yaml || generated.fingerprint === currentFingerprint) return 'none';
  return yaml === generated.config ? 'clear' : 'keep-edited';
}

/** 运行前条件摘要的一行：双语标签 + 数据值（路径、名字、数字；不带需要翻译的整句）。 */
export interface EvaluationSummaryItem {
  label: [string, string];
  value: string;
}

/**
 * U-04：运行前条件摘要。入参必须就是要提交的完整设计（页面用同一个 `comparisonDesign()` 加机器身份），
 * 这里不读页面状态，摘要与实际生成的配置、站点清单因此不会各算一份。
 */
export function evaluationSummary(design: EvaluationDesign): EvaluationSummaryItem[] {
  const { machine } = design;
  const items: EvaluationSummaryItem[] = [
    {
      label: ['运行位置', 'Machine'],
      value: machine.host
        ? bilingual(`服务器 ${machine.host}`, `server ${machine.host}`)
        : bilingual('本机', 'this computer'),
    },
    {
      label: ['OpenBench 安装位置', 'OpenBench installed in'],
      value: machine.openbenchRoot || bilingual('未安装', 'not installed'),
    },
    {
      label: ['参考数据根目录', 'Reference data root'],
      value: machine.refRoot || bilingual('未设置', 'not set'),
    },
    { label: ['比较项目目录', 'Comparison project folder'], value: design.projectDir || '—' },
    { label: ['评估年份', 'Evaluation years'], value: `${design.years[0]}–${design.years[1]}` },
    { label: ['时间尺度', 'Time scale'], value: design.timRes },
  ];
  design.simulations.forEach((sim, index) => {
    const kind = sim.info.spatial ? bilingual('格点', 'gridded') : bilingual('单点', 'station');
    const groupby =
      sim.dataGroupby || sim.info.data_groupby || (sim.info.files === 1 ? 'single' : 'Month');
    const native = sim.gridRes ?? sim.info.resolution;
    items.push({
      label: [`模拟 ${index + 1}`, `Simulation ${index + 1}`],
      value: [
        sim.label,
        kind,
        sim.profile?.trim() || 'CoLM2024',
        ...(sim.stationId?.trim()
          ? [`${bilingual('站点 ID', 'station ID')} ${sim.stationId.trim()}`]
          : []),
        sim.info.prefix,
        `${sim.info.syear}–${sim.info.eyear}`,
        groupby,
        ...(native != null && native > 0 ? [`${native}°`] : []),
        ...(sim.info.lon != null && sim.info.lat != null
          ? [`${sim.info.lat}°, ${sim.info.lon}°`]
          : []),
        sim.info.history_dir,
      ].join(' · '),
    });
  });
  const grid = design.simulations.filter((sim) => sim.info.spatial);
  if (grid.length) {
    const region = unionRegion(design.simulations);
    const missing = grid.filter((sim) => !sim.info.extent).map((sim) => sim.label);
    const target =
      design.gridRes != null && design.gridRes > 0
        ? `${design.gridRes}°`
        : bilingual('默认', 'default');
    items.push({
      label: ['区域与目标分辨率', 'Region and target resolution'],
      value: [
        `${bilingual('目标分辨率', 'target')} ${target}`,
        region
          ? `lat [${region[2]}, ${region[3]}] · lon [${region[0]}, ${region[1]}]`
          : bilingual('区域未知', 'region unknown'),
        ...(missing.length ? [`${bilingual('缺范围', 'no region')}: ${missing.join(', ')}`] : []),
      ].join(' · '),
    });
  } else {
    items.push({
      label: ['区域与目标分辨率', 'Region and target resolution'],
      value: bilingual('只有单点模拟，不裁区域', 'stations only; no region crop'),
    });
  }
  const variables = Object.entries(design.variables)
    .map(
      ([variable, refs]) =>
        [variable, (Array.isArray(refs) ? refs : [refs]).filter(Boolean)] as const,
    )
    .filter(([, refs]) => refs.length)
    .sort(([left], [right]) => left.localeCompare(right));
  items.push({
    label: ['变量与参考数据', 'Variables and reference data'],
    value: variables.length
      ? variables.map(([variable, refs]) => `${variable}: ${refs.join(', ')}`).join('; ')
      : bilingual('未选', 'none'),
  });
  items.push({
    label: ['指标与评分', 'Metrics and scores'],
    value: `${bilingual('指标', 'metrics')} [${(design.metrics ?? []).join(', ')}] · ${bilingual('评分', 'scores')} [${(design.scores ?? []).join(', ')}]`,
  });
  // 跨模拟比较（comparison 阶段）：页面把选中的图表写进配置；一个都不选就是关闭。
  const comparisonChosen = [...new Set((design.comparisonItems ?? []).filter(Boolean))];
  items.push({
    label: ['跨模拟比较', 'Cross-simulation comparison'],
    value:
      design.simulations.length >= 2 && comparisonChosen.length
        ? `${bilingual('已启用', 'enabled')} [${comparisonChosen.join(', ')}]`
        : bilingual(
            '未启用（一个图表都没选，或模拟少于两个）',
            'disabled (no chart selected, or fewer than two simulations)',
          ),
  });
  const statisticsChosen = [...new Set((design.statisticsItems ?? []).filter(Boolean))];
  const statisticsStationsOnly =
    design.simulations.length > 0 && design.simulations.every((sim) => !sim.info.spatial);
  items.push({
    label: ['统计检验', 'Statistical tests'],
    value: statisticsChosen.length
      ? `${bilingual('已启用', 'enabled')} [${statisticsChosen.join(', ')}]${
          statisticsStationsOnly
            ? ` · ${bilingual(
                'OpenBench 3.0.8 对纯单点模拟不产出统计结果',
                'OpenBench 3.0.8 produces no statistics for station-only simulations',
              )}`
            : ''
        }`
      : bilingual('未启用（一项都没勾）', 'disabled (no method selected)'),
  });
  const advanced = [
    design.weight ? `weight=${design.weight}` : '',
    design.minYear != null ? `min_year=${design.minYear}` : '',
    design.timeAlignment ? `time_alignment=${design.timeAlignment}` : '',
    design.numCores != null ? `num_cores=${design.numCores}` : '',
  ].filter(Boolean);
  items.push({
    label: ['高级选项', 'Advanced options'],
    value: advanced.length ? advanced.join(' · ') : bilingual('默认', 'defaults'),
  });
  return items;
}

/**
 * U-05：评估配置的保存状态。配置先只在页面上，直到「保存评估配置」生成文本、运行开始时由后端写进
 * 比较项目目录：还没保存或设置变了 = dirty；正在提交（写配置给 OpenBench）= saving；与当前设计一致 = saved。
 */
export function configSaveState(args: {
  yaml: string;
  currentFingerprint: string;
  generated: { fingerprint: string; config: string } | null;
  submitting: boolean;
}): SaveState {
  if (args.submitting && args.yaml) return 'saving';
  if (!args.yaml || !args.generated || args.generated.fingerprint !== args.currentFingerprint)
    return 'dirty';
  // U-05 的补充：手改过的文本还没有写进比较项目目录（运行时才写），不能显示"已保存"
  // （审查报告：只改 YAML 后徽标仍是 saved）。
  if (args.yaml !== args.generated.config) return 'dirty';
  return 'saved';
}

/**
 * 比较页草稿：只保存页面输入（模拟、站点、变量、指标/评分、comparison/statistics 选项、手改 YAML），
 * 不包含运行状态与报告。离开页面再回来时由用户显式选择是否恢复，避免自动覆盖当前页面。
 */
export interface CompareDraftPick {
  dir: string;
  label: string;
  info: HistoryInfo | null;
  variables?: Record<string, { varname: string; varunit: string }>;
  dataGroupby?: string | null;
  gridRes?: number | null;
  profile?: string | null;
  stationId?: string | null;
}

export interface CompareDraft {
  version: 1;
  savedAt: string;
  host: string;
  name: string;
  from: string;
  to: string;
  timRes: string;
  gridRes: string;
  chosen: Record<string, string[]>;
  weight: string;
  minYear: string;
  timeAlignment: string;
  numCores: string;
  metrics: string[];
  scores: string[];
  comparisonItems: string[];
  statisticsItems: string[];
  yaml: string;
  picks: CompareDraftPick[];
}

/** 草稿按机器分开存：同一套节点/参考数据在不同服务器上不是同一份输入。 */
export function compareDraftKey(host: string): string {
  return `colm.compareDraft@${host}`;
}

function stringArray(value: unknown): string[] | null {
  if (!Array.isArray(value) || value.some((item) => typeof item !== 'string')) return null;
  return value as string[];
}

/** 读 localStorage 里的草稿；坏记录当没有（宁可不恢复，也不用一条读不懂的数据挡住页面）。 */
export function parseCompareDraft(text: string): CompareDraft | null {
  if (!text.trim()) return null;
  let raw: unknown;
  try {
    raw = JSON.parse(text);
  } catch {
    return null;
  }
  if (!raw || typeof raw !== 'object') return null;
  const draft = raw as Record<string, unknown>;
  if (draft.version !== 1 || !Array.isArray(draft.picks)) return null;
  const pickOk = draft.picks.every(
    (pick) =>
      !!pick &&
      typeof pick === 'object' &&
      typeof (pick as Record<string, unknown>).dir === 'string' &&
      typeof (pick as Record<string, unknown>).label === 'string',
  );
  const arrays: Record<string, string[]> = {};
  for (const key of ['metrics', 'scores', 'comparisonItems', 'statisticsItems']) {
    const value = stringArray(draft[key]);
    if (!value) return null;
    arrays[key] = value;
  }
  const chosen = draft.chosen;
  if (!chosen || typeof chosen !== 'object' || Array.isArray(chosen)) return null;
  const chosenOk = Object.values(chosen as Record<string, unknown>).every(
    (refs) => stringArray(refs) !== null,
  );
  if (!pickOk || !chosenOk) return null;
  return {
    version: 1,
    savedAt: typeof draft.savedAt === 'string' ? draft.savedAt : '',
    host: typeof draft.host === 'string' ? draft.host : '',
    name: typeof draft.name === 'string' ? draft.name : 'comparison',
    from: typeof draft.from === 'string' ? draft.from : '',
    to: typeof draft.to === 'string' ? draft.to : '',
    timRes: typeof draft.timRes === 'string' ? draft.timRes : 'Day',
    gridRes: typeof draft.gridRes === 'string' ? draft.gridRes : '',
    chosen: chosen as Record<string, string[]>,
    weight: typeof draft.weight === 'string' ? draft.weight : '',
    minYear: typeof draft.minYear === 'string' ? draft.minYear : '1',
    timeAlignment: typeof draft.timeAlignment === 'string' ? draft.timeAlignment : '',
    numCores: typeof draft.numCores === 'string' ? draft.numCores : '',
    metrics: arrays.metrics!,
    scores: arrays.scores!,
    comparisonItems: arrays.comparisonItems!,
    statisticsItems: arrays.statisticsItems!,
    yaml: typeof draft.yaml === 'string' ? draft.yaml : '',
    picks: draft.picks as CompareDraftPick[],
  };
}

/** 草稿里有没有值得提示恢复的内容：空页面/只有默认值时不打扰。 */
export function compareDraftHasContent(draft: CompareDraft): boolean {
  if (draft.picks.length > 0) return true;
  if (draft.yaml.trim()) return true;
  if (Object.values(draft.chosen).some((refs) => refs.length > 0)) return true;
  return draft.name.trim() !== '' && draft.name !== 'comparison';
}

/** 选中的模拟共同覆盖的年份（评估年份的缺省）；没有交集时为 null。 */
export function commonYears(simulations: BenchSimulation[]): [number, number] | null {
  if (!simulations.length) return null;
  const from = Math.max(...simulations.map((sim) => sim.info.syear));
  const to = Math.min(...simulations.map((sim) => sim.info.eyear));
  return from <= to ? [from, to] : null;
}

/** 一个比较的目录（与后端 `bench_run` 的 `<root>/projects/<名字>` 一致），沿用根目录的路径分隔符。 */
export function comparisonDir(root: string, name: string): string {
  const sep = root.includes('\\') && !root.includes('/') ? '\\' : '/';
  return `${root}${sep}projects${sep}${comparisonName(name)}`;
}

/** 后端 `valid_name` 的规则：字母、数字、`-`、`_`，1–60 个字符。 */
const VALID_COMPARISON_NAME = /^[A-Za-z0-9_-]{1,60}$/;

/**
 * 比较名在后端只认字母、数字、- 和 _（`valid_name`），并当目录名用。
 *
 * 已经是合法名字的 ASCII 名原样保留：旧实现把首尾下划线剥掉，`_experiment_` 会和 `experiment`
 * 共用 `projects/experiment` 目录、报告互相覆盖（审查报告 R11）；后端允许首尾下划线，所以不需要清洗，
 * 已有 ASCII 比较目录也不迁移。只有含空格/点号等非法字符的 ASCII 名才按旧规则清洗（映射不变）。
 * 超长名（后端上限 60）截断后接显示名指纹，避免两个长名折成同一个目录或被后端拒绝。
 * 非 ASCII 名保留旧算法（按字符清洗后塌成同一个 `_`，所以追加指纹）：已有中文比较目录不迁移。
 */
export function comparisonName(name: string): string {
  if (!hasNonAscii(name)) {
    if (VALID_COMPARISON_NAME.test(name)) return name;
    const slug = name.replace(/[^A-Za-z0-9_-]+/g, '_').replace(/^_+|_+$/g, '');
    const base = slug || 'comparison';
    if (base.length <= 60) return base;
    const suffix = fingerprintOf(name).slice(0, 8);
    return `${base.slice(0, 60 - suffix.length - 1)}-${suffix}`;
  }
  const slug = name.replace(/[^A-Za-z0-9_-]+/g, '_').replace(/^_+|_+$/g, '');
  const base = slug || 'comparison';
  const suffix = fingerprintOf(name).slice(0, 8);
  return `${base.slice(0, 60 - suffix.length - 1)}-${suffix}`;
}

/**
 * 本机记着的远程比较作业身份（GUI-07）。一次 SSH/网络查询失败不能把它删掉：远端可能还在跑，
 * 恢复连接后要能按同一个作业号继续核对。存进 localStorage 的 `colm.benchJob@<host>`；
 * 确认终态后移到 `colm.benchLastJob@<host>` 保留历史。
 */
export interface RemoteJobIdentity {
  job: string;
  kind: 'install' | 'run';
  /** 比较项目名（kind === 'run'）：失联时说明这是哪次比较，确认成功后据此取回报告。 */
  name?: string;
  /** 作业属于哪台服务器：身份按机器分开存，核对时确认回来的是不是同一台。 */
  host?: string;
  submittedAt?: string;
  /** 最后一次被服务器确认的状态（queued/running/finished/lost…）。 */
  lastState?: string;
  lastConfirmedAt?: string;
  /** 通信失败后置位：终态未知；页面显示"暂时失联/待核对"，重启应用后仍可恢复核对。 */
  uncertain?: boolean;
  /** 最近一次失败原因原文（通信或作业错误），展示前过 `benchErrorText`。 */
  lastError?: string;
  /** 只有确认终态后才写：历史记录里的结果（`unknown` = 确认已终止但没有退出码/结果）。 */
  end?: 'succeeded' | 'failed' | 'cancelled' | 'unknown';
  endedAt?: string;
}

/** 解析 localStorage 里的作业身份；坏记录当没有（宁可重新核对，也不用一条读不懂的数据挡住提交）。 */
export function parseRemoteJob(text: string | null | undefined): RemoteJobIdentity | null {
  if (!text) return null;
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch {
    return null;
  }
  if (!value || typeof value !== 'object') return null;
  const data = value as Record<string, unknown>;
  const job = typeof data.job === 'string' ? data.job : '';
  const kind = data.kind === 'install' || data.kind === 'run' ? data.kind : null;
  if (!job || !kind) return null;
  const textField = (key: string) =>
    typeof data[key] === 'string' ? (data[key] as string) : undefined;
  const end =
    data.end === 'succeeded' ||
    data.end === 'failed' ||
    data.end === 'cancelled' ||
    data.end === 'unknown'
      ? data.end
      : undefined;
  return {
    job,
    kind,
    name: textField('name'),
    host: textField('host'),
    submittedAt: textField('submittedAt'),
    lastState: textField('lastState'),
    lastConfirmedAt: textField('lastConfirmedAt'),
    uncertain: data.uncertain === true,
    lastError: textField('lastError'),
    end,
    endedAt: textField('endedAt'),
  };
}

/**
 * 一次 `bench_remote_job` 查询的终态。只有服务器明确给出 `finished`，或调度系统记下了 `ended`，
 * 才算终态；"还在跑"和"这次查不到"（通信失败）都不是（GUI-07）。判定本身复用普通远程运行
 * 已有的 `remoteEndStatus`，避免比较页与运行面板对同一个状态给出两套结论。
 */
export function remoteJobTerminal(reply: {
  status?: { state?: string; exit_code?: number | null };
  ended?: string | null;
  /** 调度系统明确确认作业不会再写（但退出码/结果经 NFS 可能还没可见）。 */
  terminal?: boolean;
  /** 作业进程已不在、也没有退出码：确认终止但结果未知（`ended:"unknown"` 的布尔别名）。 */
  result_unknown?: boolean;
}): 'succeeded' | 'failed' | 'cancelled' | 'unknown' | null {
  const state = reply.status?.state;
  // 进程/队列已不在且没有退出码：调度器确认"已终止、结果未知"。这是终态（否则页面会永远每 3 秒
  // 轮询、身份一直挡着新提交），但不能当成功或失败，单独返回 `unknown` 让页面给出可行动说明。
  if (reply.result_unknown === true || reply.ended === 'unknown') return 'unknown';
  // 调度系统记下的取消/失败是明确终态（没有退出码也认）。
  if (reply.ended === 'cancelled' || reply.ended === 'failed') return remoteEndStatus(reply);
  if (state !== 'finished') return null;
  // finished 但退出码还没出现（COMPLETED 已写、exit_code 还在 NFS 路上）：继续等，不能当失败。
  // 同理 terminal=true 只是"不再写"，成功/失败未定，也不是本函数要回答的问题（由页面继续轮询）。
  if (reply.status?.exit_code == null) return null;
  return remoteEndStatus(reply);
}

/** 一次成功的状态查询：更新最后确认状态，不动终态字段（终态由 `jobEnded` 写）。 */
export function jobContacted(
  identity: RemoteJobIdentity,
  state: string,
  now: string,
): RemoteJobIdentity {
  return {
    ...identity,
    lastState: state,
    lastConfirmedAt: now,
    uncertain: false,
    lastError: undefined,
  };
}

/** 一次通信失败：保留全部身份，只标记"终态未确认"和原因，不删记录（GUI-07 的核心）。 */
export function jobUncertain(identity: RemoteJobIdentity, error: string): RemoteJobIdentity {
  return { ...identity, uncertain: true, lastError: error };
}

/** 确认终态后的历史记录：保留作业身份，写上结果与时间；活动记录由页面删除。 */
export function jobEnded(
  identity: RemoteJobIdentity,
  end: 'succeeded' | 'failed' | 'cancelled' | 'unknown',
  now: string,
  reason = '',
): RemoteJobIdentity {
  return {
    ...identity,
    uncertain: false,
    end,
    endedAt: now,
    lastState: end,
    lastConfirmedAt: now,
    lastError: reason || undefined,
  };
}

/**
 * 后端（`bench.rs`）报的是固定的中文句子；按固定文字识别后给出中英两种说法，英文界面不再中英混排
 * （复验 C2-06/A2-05）。不认得的（OpenBench 自己的输出）原样返回。
 */
export function benchErrorText(error: string): [string, string] {
  const text = error.replace(/^(?:Error: )+/, '');
  const fixed: Record<string, string> = {
    已停止: 'Stopped',
    '先装 uv': 'Install uv first',
    '先装专用 Python': 'Install the private Python first',
    '先装 OpenBench': 'Install OpenBench first',
    '先装专用 Python 和 OpenBench': 'Install the private Python and OpenBench first',
    子进程没有输出管道: 'The child process has no output pipes',
    '压缩包里没有 uv': 'The archive has no uv in it',
    '类型只能是 grid 或 stn': 'The type must be grid or stn',
    至少给一个变量: 'Give at least one variable',
    '报告应是 .html 文件': 'A report must be an .html file',
    读不了这个目录: 'This folder cannot be read',
    '只能显示这里跑出来的报告；别处的报告请用"载入报告…"':
      'Only reports produced here can be shown directly; use “Load a report…” for others',
    // GUI-08：同一比较已有一份报告在取回中，后端明确拒绝并发取回。
    '这个比较的报告正在取回中，等这次取回结束后再试。':
      'This comparison’s report is being fetched already; try again when that fetch finishes.',
    // 复查 C07/C09：远程 run 级快照、提交意图与取回目标的固定句（Tauri/CLI 侧原文）。
    这个比较的活动作业锁一直被占用:
      "The comparison's active-job lock is still held; try again shortly.",
    这个比较还没有成功过的运行: 'This comparison has no successful run yet.',
    服务器上还没有可显示的评估报告: 'The server has no report to show yet.',
    服务器上的运行记录读不懂: 'The run record on the server cannot be read.',
  };
  if (text in fixed) return [text, fixed[text]!];
  const patterns: [RegExp, (m: RegExpExecArray) => string][] = [
    // 远程 run 级身份：指定了服务器上不存在的 run，或运行记录解析失败。
    [
      /^这次运行没有成功，没有可显示的评估报告（运行 ([^，]+)，状态 ([^）]+)）$/,
      (m) =>
        `That run did not succeed, so there is no report to show (run ${m[1]}, state ${m[2]}).`,
    ],
    [
      /^服务器上没有这次运行的快照：([\s\S]*)$/,
      (m) => `The server has no snapshot for run ${m[1]}.`,
    ],
    [
      /^服务器上的运行记录读不懂：([\s\S]*)$/,
      (m) => `The run record on the server cannot be read: ${m[1]}`,
    ],
    // 审查报告第 6 条：调度器不可用/查询命令执行失败（不是"作业丢了"）。
    [
      /^查询作业状态失败：([\s\S]*)$/,
      (m) => `Cannot query the job status: ${benchErrorText(m[1]!)[1]}`,
    ],
    // 本机运行身份：失败原因带 run_id 与冻结目录（run-identity 定的固定格式）。
    [
      /^运行 (\S+) 失败：([\s\S]*?)\n运行目录：([\s\S]*?)；配置与站点清单已冻结在这次运行里。上次成功的报告未受影响。$/,
      (m) =>
        `Run ${m[1]} failed: ${m[2]!.trim()}. Run folder: ${m[3]}; the configuration and station list are frozen with this run. The last successful report is unaffected.`,
    ],
    [
      /^运行 (\S+) 失败：([\s\S]*?)\n运行目录：([\s\S]*?)；配置与站点清单已冻结在这次运行里。这个比较还没有成功过的报告。$/,
      (m) =>
        `Run ${m[1]} failed: ${m[2]!.trim()}. Run folder: ${m[3]}; the configuration and station list are frozen with this run. This comparison has no successful report yet.`,
    ],
    // GUI-07 服务器侧：同名比较还有未确认终态的作业时，sidecar 拒绝提交/取回，作业号在括号里。
    // 括号后的中文动作说明不逐句翻（服务端措辞会变）：英文固定给出"还有未完成作业（作业号）"这一可操作含义。
    [
      // 作业号的括号可能是全角也可能被日志/包装层换成半角，两种都认。
      // 真实服务端原文括号里是"作业号 <id>"（例如「作业号 bench-run-42」）；英文界面只需要 <id>，
      // 不摘掉这个中文前缀就会输出 "(作业号 bench-run-42)" 的中英混排（交付审计实测）。
      /^这个比较还有未完成的评估作业(?:[（(]([^）)]*)[）)])?([\s\S]*)$/,
      (m) => {
        const job = m[1]?.replace(/^作业号[：:\s]*/, '').trim();
        return `This comparison already has an unfinished evaluation job${job ? ` (${job})` : ''}; confirm its final state or stop it before resubmitting.`;
      },
    ],
    // GUI-08：取回失败时后端保证本机旧镜像没有动过；错误里那半句固定说明要一起翻，页面据此
    // 提示"旧报告仍是上次成功版本"，不能只显示英文界面里夹着的中文原句。
    // 内层错误可能自己带句号（GUI-07 服务器侧固定句子），嵌套前去掉，拼接后才不会出现".."。
    [
      /^取回失败：([\s\S]*)。本机旧报告未改动，仍是上次成功版本。$/,
      (m) =>
        `Fetch failed: ${benchErrorText(m[1]!)[1].replace(/[。.]$/, '')}. The previous report on this computer is unchanged; it is still the last successful version.`,
    ],
    [
      /^取回失败：([\s\S]*)。本机原先没有这个比较的报告。$/,
      (m) =>
        `Fetch failed: ${benchErrorText(m[1]!)[1].replace(/[。.]$/, '')}. There was no previous report of this comparison here.`,
    ],
    [
      /^没有给 (\S+) 准备 uv，请自行安装 Python 与 colm-openbench$/,
      (m) => `No uv is prepared for ${m[1]}; install Python and colm-openbench yourself`,
    ],
    [/^无法启动(?: (\S+))?：([\s\S]*)$/, (m) => `Cannot start${m[1] ? ` ${m[1]}` : ''}: ${m[2]}`],
    [/^(\S+) 失败（([^）]*)）：\n?([\s\S]*)$/, (m) => `${m[1]} failed (${m[2]}):\n${m[3]}`],
    [
      /^([\s\S]+) 不是 OpenBench 源码目录（没有 pyproject\.toml）$/,
      (m) => `${m[1]} is not an OpenBench source folder (no pyproject.toml)`,
    ],
    [/^没有这一步：(.*)$/, (m) => `No such step: ${m[1]}`],
    [/^没有这一项：(.*)$/, (m) => `No such item: ${m[1]}`],
    [
      /^下载的 uv 校验不对（(\w+)，应为 (\w+)），已删除，请重试$/,
      (m) =>
        `The downloaded uv failed its checksum (${m[1]}, expected ${m[2]}); it was deleted, try again`,
    ],
    [/^删不掉 ([\s\S]+)：([\s\S]*)$/, (m) => `Cannot remove ${m[1]}: ${m[2]}`],
    [/^参考数据列表读不懂：([\s\S]*)$/, (m) => `The reference list cannot be read: ${m[1]}`],
    [
      /^比较名只能用字母、数字、- 和 _：(.*)$/,
      (m) => `A comparison name may only use letters, digits, - and _: ${m[1]}`,
    ],
    [/^站点名不合适：(.*)$/, (m) => `Unsuitable station name: ${m[1]}`],
    [
      /^报告有 (\d+) MB，太大不在软件里显示；请用浏览器打开$/,
      (m) => `The report is ${m[1]} MB, too large to show here; open it in a browser`,
    ],
    [/^扫描失败：([\s\S]*)$/, (m) => `Scan failed: ${m[1]}`],
  ];
  for (const [pattern, english] of patterns) {
    const m = pattern.exec(text);
    if (m) return [text, english(m)];
  }
  return [text, text];
}

/** 后端自己写进日志的几行（`say`）的英文；OpenBench、uv 的输出原样。 */
export function benchLogText(line: string): string {
  const patterns: [RegExp, (m: RegExpExecArray) => string][] = [
    [/^下载 (\S+)$/, (m) => `Downloading ${m[1]}`],
    [/^校验通过，解压$/, () => 'Checksum verified; unpacking'],
    [/^uv (\S+) 装好了$/, (m) => `uv ${m[1]} installed`],
    [/^合并 (.+) 的 history$/, (m) => `Merging the history of ${m[1]}`],
  ];
  for (const [pattern, english] of patterns) {
    const m = pattern.exec(line);
    if (m) return english(m);
  }
  return line;
}

/**
 * 能评估的变量与各自可用的参考数据（按名字筛选）。参考数据不按种类排除：站点模拟可以拿格点产品比
 * （在站点处取值），格点模拟也可以拿站点观测比，OpenBench 都支持（2026-10-23 在 7920 上实测站点模拟对
 * ERA5-Land 格点）；同种的排在前面。原来一次给 82 个变量各列一个下拉（复验 C2-05），现在页面只给加进来的
 * 变量列下拉。还没选模拟时不列。
 */
export function comparableVariables(
  references: BenchReference[],
  simulations: { spatial: boolean }[],
  query = '',
): { variable: string; references: BenchReference[] }[] {
  const kinds = new Set<string>(simulations.map((s) => (s.spatial ? 'grid' : 'stn')));
  if (!kinds.size) return [];
  const needle = query.trim().toLowerCase();
  const byVariable = new Map<string, BenchReference[]>();
  for (const reference of references)
    for (const variable of reference.variables) {
      if (needle && !variable.toLowerCase().includes(needle)) continue;
      byVariable.set(variable, [...(byVariable.get(variable) ?? []), reference]);
    }
  const sameKind = (r: BenchReference) => (!r.data_type || kinds.has(r.data_type) ? 0 : 1);
  return [...byVariable.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([variable, refs]) => ({
      variable,
      references: [...refs].sort((a, b) => sameKind(a) - sameKind(b)),
    }));
}
