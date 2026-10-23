import { bilingual } from '../../lib/bilingual';

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
}

/** 比较里的一个模拟：一个算例（或别的 history 目录）的一次输出。 */
export interface BenchSimulation {
  /** 报告里显示的名字，也是 `simulation` 下的键。 */
  label: string;
  info: HistoryInfo;
}

/** OpenBench 登记表里的一个参考数据集（`ref list --format json` 的一项，只取用得上的字段）。 */
export interface BenchReference {
  name: string;
  data_type?: string;
  tim_res?: string;
  variables: string[];
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

/** YAML 里的字符串一律用 JSON 的双引号写法：路径里有空格、冒号、`#` 也不会被读错。 */
const quote = (value: string) => JSON.stringify(value);

/**
 * 生成 `openbench.yaml`。格点模拟直接指向 history 目录（按月一个文件）；单点模拟先由后端并成一个站点文件，
 * 这里写站点清单 `<比较目录>/sim/<键>.csv`（绝对路径，原因见下）。
 * 返回配置文本和要先并成站点文件的单点模拟。
 */
export function buildOpenbenchConfig(design: {
  name: string;
  years: [number, number];
  timRes: string;
  variables: Record<string, string>;
  simulations: BenchSimulation[];
  /** 这个比较的目录（绝对路径）：单点模拟的站点文件与清单放在它的 `sim/` 下。 */
  projectDir: string;
}) {
  const [from, to] = design.years;
  if (!Number.isInteger(from) || !Number.isInteger(to) || from > to)
    throw new Error(bilingual('评估年份要先小后大', 'The evaluation years must run forward'));
  const variables = Object.entries(design.variables).filter(([, ref]) => ref);
  if (!variables.length)
    throw new Error(
      bilingual('至少选一个变量和它的参考数据', 'Choose at least one variable and its reference'),
    );
  if (!design.simulations.length)
    throw new Error(bilingual('至少选一个模拟', 'Choose at least one simulation'));
  const keys = design.simulations.map((sim) => simulationKey(sim.label));
  if (new Set(keys).size !== keys.length)
    throw new Error(bilingual('模拟的名字不能重复', 'Simulation names must be unique'));
  const lines = [
    '# CoLM Desktop 生成；可以先改再运行。',
    'project:',
    `  name: ${quote(simulationKey(design.name))}`,
    '  output_dir: ./output',
    `  years: [${from}, ${to}]`,
    `  tim_res: ${design.timRes}`,
    'evaluation:',
    `  variables: [${variables.map(([variable]) => variable).join(', ')}]`,
    'reference:',
    ...variables.map(([variable, ref]) => `  ${variable}: ${quote(ref)}`),
    'simulation:',
  ];
  const sites: { id: string; station: string; history_dir: string; prefix: string }[] = [];
  design.simulations.forEach((sim, i) => {
    const key = keys[i]!;
    lines.push(`  ${key}:`, '    model: CoLM2024');
    if (sim.info.spatial) {
      lines.push(
        `    root_dir: ${quote(sim.info.history_dir)}`,
        `    prefix: ${quote(sim.info.prefix)}`,
        '    data_groupby: month',
        '    data_type: grid',
        `    tim_res: ${sim.info.tim_res}`,
      );
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
        `    tim_res: ${sim.info.tim_res}`,
        `    fulllist: ${quote(`${simDir}${sep}${key}.csv`)}`,
      );
    }
  });
  return { config: `${lines.join('\n')}\n`, sites };
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
  };
  if (text in fixed) return [text, fixed[text]!];
  const patterns: [RegExp, (m: RegExpExecArray) => string][] = [
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
 * 能评估的变量：只列参考数据与所选模拟同种（站点 / 格点）的变量，再按输入筛选。原来把 152 个参考
 * 数据集的变量并集一次列成 82 个下拉，选了单点算例也列格点产品的变量（复验 C2-05）。
 * 没写种类的参考数据两种都算。还没选模拟时不列。
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
  for (const reference of references) {
    if (reference.data_type && !kinds.has(reference.data_type)) continue;
    for (const variable of reference.variables) {
      if (needle && !variable.toLowerCase().includes(needle)) continue;
      byVariable.set(variable, [...(byVariable.get(variable) ?? []), reference]);
    }
  }
  return [...byVariable.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([variable, refs]) => ({ variable, references: refs }));
}
