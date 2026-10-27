import { bilingual } from '../../lib/bilingual';
import type { SaveState } from '../../lib/save-status';
import { remoteEndStatus } from '../runs/run-model';

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
  };
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

/** 名字只留字母、数字和下划线：它要当 YAML 的键、站点 ID 和文件名。 */
export function simulationKey(label: string): string {
  const key = label.replace(/[^A-Za-z0-9_]+/g, '_').replace(/^_+|_+$/g, '');
  return /^[A-Za-z]/.test(key) ? key : `sim_${key || 'run'}`;
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
    .map((s) => s.info.extent!);
  if (!extents.length) return null;
  const round = (v: number, up: boolean) => (up ? Math.ceil(v * 100) : Math.floor(v * 100)) / 100;
  return [
    round(Math.max(-180, Math.min(...extents.map((e) => e[0]))), false),
    round(Math.min(180, Math.max(...extents.map((e) => e[1]))), true),
    round(Math.max(-90, Math.min(...extents.map((e) => e[2]))), false),
    round(Math.min(90, Math.max(...extents.map((e) => e[3]))), true),
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
  const keys = design.simulations.map((sim) => simulationKey(sim.label));
  if (new Set(keys).size !== keys.length)
    throw new Error(bilingual('模拟的名字不能重复', 'Simulation names must be unique'));
  const missingExtent = design.simulations
    .filter((sim) => sim.info.spatial && !sim.info.extent)
    .map((sim) => sim.label);
  const lines = [
    '# CoLM Desktop 生成；可以先改再运行。',
    'project:',
    `  name: ${quote(simulationKey(design.name))}`,
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
  const sites: { id: string; station: string; history_dir: string; prefix: string }[] = [];
  design.simulations.forEach((sim, i) => {
    const key = keys[i]!;
    lines.push(`  ${key}:`, '    model: CoLM2024');
    if (sim.info.spatial) {
      const groupby =
        sim.dataGroupby || sim.info.data_groupby || (sim.info.files === 1 ? 'single' : 'month');
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
      const station = sim.info.prefix.replace(/_hist_$/, '') || key;
      sites.push({ id: key, station, history_dir: sim.info.history_dir, prefix: sim.info.prefix });
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
  return { config: `${lines.join('\n')}\n`, sites, missingExtent };
}

/** 指纹里的对象键按名字排序：同一份设计不因勾选/写入顺序不同得到两个指纹。 */
function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (value && typeof value === 'object')
    return `{${Object.keys(value as Record<string, unknown>)
      .sort()
      .map(
        (key) => `${JSON.stringify(key)}:${canonicalJson((value as Record<string, unknown>)[key])}`,
      )
      .join(',')}}`;
  if (typeof value === 'number' && !Number.isFinite(value)) return 'null';
  return JSON.stringify(value) ?? 'null';
}

/**
 * 一次评估设计的稳定指纹：把**生成配置与运行所依赖的全部输入**（目录、类型、前缀、时间、分组、
 * 空间范围与分辨率、变量映射、参考数据、指标评分，以及机器/OpenBench/参考数据路径身份）压成一个
 * 短串。生成配置时记下它，运行前再算一遍；只是目录和显示名不变、内部输入变了也能发现（GUI-04）。
 */
export function designFingerprint(design: EvaluationDesign): string {
  const text = canonicalJson(design);
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, '0');
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
      sim.dataGroupby || sim.info.data_groupby || (sim.info.files === 1 ? 'single' : 'month');
    const native = sim.gridRes ?? sim.info.resolution;
    items.push({
      label: [`模拟 ${index + 1}`, `Simulation ${index + 1}`],
      value: [
        sim.label,
        kind,
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
  generated: { fingerprint: string } | null;
  submitting: boolean;
}): SaveState {
  if (args.submitting && args.yaml) return 'saving';
  if (!args.yaml || !args.generated || args.generated.fingerprint !== args.currentFingerprint)
    return 'dirty';
  return 'saved';
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

/** 比较名在后端只认字母、数字、- 和 _。 */
export function comparisonName(name: string): string {
  return name.replace(/[^A-Za-z0-9_-]+/g, '_') || 'comparison';
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
  /** 只有确认终态后才写：历史记录里的结果。 */
  end?: 'succeeded' | 'failed' | 'cancelled';
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
    data.end === 'succeeded' || data.end === 'failed' || data.end === 'cancelled'
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
}): 'succeeded' | 'failed' | 'cancelled' | null {
  const state = reply.status?.state;
  const ended = reply.ended === 'cancelled' || reply.ended === 'failed';
  if (state !== 'finished' && !ended) return null;
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
  end: 'succeeded' | 'failed' | 'cancelled',
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
  };
  if (text in fixed) return [text, fixed[text]!];
  const patterns: [RegExp, (m: RegExpExecArray) => string][] = [
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
