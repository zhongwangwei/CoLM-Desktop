/**
 * 有效评估配置：把页面实际要提交的 OpenBench YAML（`buildOpenbenchConfig` 生成、用户可以在文本框里
 * 直接改）解析回结构化配置，供"运行前条件摘要 / 配置指纹 / 运行冻结 / 完成提示失效"使用。
 *
 * 为什么要解析而不是继续读表单：页面允许手改 YAML，提交给后端的也是文本；如果摘要与指纹仍按表单
 * 状态算，用户看到的评估条件就不是这次真正计算的条件（审查报告 P1）。这里只接受本页面生成的那一套
 * 键（`project/evaluation/reference/simulation/metrics/scores`）；出现解释不了的字段就明确拒绝，
 * 不静默忽略——宁可让用户重新生成，也不生成一份"摘要与实际不一致"的配置。
 *
 * 解析器只实现生成配置用到的 YAML 子集（缩进映射、标量、行内 `[a, b]`、引号字符串、`- ` 列表），
 * 不引入第三方 YAML 依赖：比较页只认自己写出来的格式，手改超出这个子集时给可操作的错误。
 */

import { bilingual } from '../../lib/bilingual';
import {
  designFingerprint,
  layerVariableProblems,
  simulationKey,
  VALID_COMPARISON_ITEMS,
  VALID_STATISTICS_ITEMS,
  type BenchSite,
  type EvaluationDesign,
  type EvaluationSummaryItem,
} from './compare';
import { fingerprintOf } from './fingerprint';

/** 一条双语问题：页面按当前语言取一半。 */
export type Problem = [string, string];

/**
 * OpenBench 3.0.8 的合法 tim_res（v3.0.8 `config/loader.py` 的 VALID_TIM_RES_VALUES）。
 * 比较页只直接生成常规档位，但手改/探测可能给出 6Hour/8Day/climatology-*，解析器要能接受。
 */
export const TIM_RES_VALUES = [
  'Hour',
  '3Hour',
  '6Hour',
  '8Day',
  'Day',
  'Month',
  'Year',
  'climatology-month',
  'climatology-year',
] as const;
/** `data_groupby`：history 文件按什么切分。 */
export const DATA_GROUPBY_VALUES = ['Year', 'Month', 'Day', 'single'] as const;

export interface ConfigSimulationVariable {
  varname: string;
  varunit: string;
}

export interface ConfigSimulation {
  /** `simulation:` 下的键；单点模拟同时是站点 ID / 站点文件名。 */
  key: string;
  model: string;
  rootDir: string;
  /** 格点模拟的 history 文件前缀；单点模拟的配置里没有这一项（数据源是后端并出来的站点文件）。 */
  prefix: string | null;
  dataGroupby: string;
  dataType: 'grid' | 'stn';
  timRes: string;
  gridRes: number | null;
  /** 单点模拟：合并后的站点清单 CSV（绝对路径）。 */
  fulllist: string | null;
  variables: Record<string, ConfigSimulationVariable>;
}

export interface ConfigProject {
  name: string;
  outputDir: string | null;
  years: [number, number];
  timRes: string;
  gridRes: number | null;
  latRange: [number, number] | null;
  lonRange: [number, number] | null;
  weight: string | null;
  minYearThreshold: number | null;
  timeAlignment: string | null;
  numCores: number | null;
}

export interface EffectiveConfig {
  project: ConfigProject;
  /** `evaluation.variables`：这次要评估的标准变量。 */
  variables: string[];
  /** `reference`：标准变量 → 参考数据集名（一个或多个）。 */
  reference: Record<string, string[]>;
  simulations: ConfigSimulation[];
  /** 勾选的指标/评分；`null` 表示配置里没写（OpenBench 用默认值）。 */
  metrics: string[] | null;
  scores: string[] | null;
  /** 跨模拟比较（`comparison` 段）。配置没写这一段时 enabled=false、items=null（OpenBench 默认关闭）。 */
  comparison: { enabled: boolean; items: string[] | null };
  /** 统计检验（`statistics` 段）。未写时 enabled=false、items=null（OpenBench 默认关闭）。 */
  statistics: { enabled: boolean; items: string[] | null };
}

export interface ParsedConfig {
  config: EffectiveConfig | null;
  problems: Problem[];
}

type Yaml = string | number | boolean | null | Yaml[] | { [key: string]: Yaml };

interface SourceLine {
  indent: number;
  text: string;
  number: number;
}

const TOP_LEVEL_KEYS = new Set([
  'project',
  'evaluation',
  'reference',
  'simulation',
  'metrics',
  'scores',
  'comparison',
  'statistics',
]);

function isMap(value: Yaml | undefined): value is { [key: string]: Yaml } {
  return !!value && typeof value === 'object' && !Array.isArray(value);
}

/**
 * YAML 的 `#` 注释：只有引号外、前面是空白（或行首）的 `#` 才是注释。单引号和双引号都要认，
 * 否则 `'/history/old # keep'` 会被读成 "# 之前" 的残路径——OpenBench 读到的是完整路径，
 * 摘要/提交就会和它不一致（审查报告 R10）。
 *
 * 引号只在 YAML 允许开始标量的位置生效（行首、`- ` 之后、`键: ` 之后、`[`/`,` 之后）：
 * 普通标量中间的撇号（don't）不是引号开始，否则后面的真注释会被吞掉。
 */
function stripComment(line: string): string {
  let double = false;
  let single = false;
  // 当前是否处在"可以开始一个标量"的位置（下一个引号才是引号字符串的开始）。
  let nodeStart = true;
  // 行内列表/映射的嵌套层数：只有流内逗号才是分隔符。
  let flow = 0;
  for (let index = 0; index < line.length; index++) {
    const char = line[index]!;
    if (double) {
      if (char === '\\') index += 1;
      else if (char === '"') {
        double = false;
        nodeStart = false;
      }
      continue;
    }
    if (single) {
      if (char === "'") {
        if (line[index + 1] === "'") index += 1;
        else {
          single = false;
          nodeStart = false;
        }
      }
      continue;
    }
    if (char === '#' && (index === 0 || /\s/.test(line[index - 1]!))) return line.slice(0, index);
    if (nodeStart) {
      if (char === ' ' || char === '\t') continue;
      // `- ` 序列标记（`-1` 是负数标量，不能当标记吃掉）。
      if (char === '-' && (index + 1 >= line.length || /\s/.test(line[index + 1]!))) continue;
      if (char === '"') {
        double = true;
        continue;
      }
      if (char === "'") {
        single = true;
        continue;
      }
      if (char === '[' || char === '{') {
        flow += 1;
        continue;
      }
      if (char === ']' || char === '}') {
        flow = Math.max(0, flow - 1);
        nodeStart = false;
        continue;
      }
      // 普通标量的第一个字符：这个标量里出现的引号都只是字符。
      nodeStart = false;
      continue;
    }
    // `键: 值` 的映射分隔只在冒号后跟空白/行尾时成立（YAML 的 plain scalar 里可以有 `a:b`）。
    if (char === ':' && (index + 1 >= line.length || /\s/.test(line[index + 1]!))) {
      nodeStart = true;
      continue;
    }
    if (flow > 0 && char === ',') {
      nodeStart = true;
      continue;
    }
    if (flow > 0 && (char === '[' || char === '{')) {
      flow += 1;
      nodeStart = true;
      continue;
    }
    if (flow > 0 && (char === ']' || char === '}')) {
      flow = Math.max(0, flow - 1);
      nodeStart = false;
    }
  }
  return line;
}

/** 去掉注释、空行，记下缩进与行号；缩进只允许空格（制表符直接报错，避免"看起来对齐"的假配置）。 */
function sourceLines(text: string, problems: Problem[]): SourceLine[] {
  const out: SourceLine[] = [];
  text.split(/\r?\n/).forEach((raw, index) => {
    const number = index + 1;
    const leading = /^\s*/.exec(raw)![0];
    if (leading.includes('\t'))
      problems.push([
        `第 ${number} 行用了制表符缩进；YAML 只用空格。`,
        `Line ${number} is indented with a tab; YAML uses spaces only.`,
      ]);
    const stripped = stripComment(raw).trimEnd();
    if (!stripped.trim()) return;
    out.push({ indent: leading.length, text: stripped.trimStart(), number });
  });
  return out;
}

function unquoteKey(text: string, problems: Problem[], line: SourceLine): string | null {
  if (!text.startsWith('"')) return text;
  try {
    const value: unknown = JSON.parse(text);
    return typeof value === 'string' ? value : null;
  } catch {
    problems.push([
      `第 ${line.number} 行的键读不懂：${text}`,
      `Line ${line.number} has an unreadable key: ${text}`,
    ]);
    return null;
  }
}

/** 把 `键: 值` 拆成两半；冒号必须在引号外，且键不能为空。 */
function splitKey(
  text: string,
  problems: Problem[],
  line: SourceLine,
): { key: string; rest: string } | null {
  let quoted = false;
  let escaped = false;
  for (let index = 0; index < text.length; index++) {
    const char = text[index]!;
    if (escaped) {
      escaped = false;
      continue;
    }
    if (quoted && char === '\\') {
      escaped = true;
      continue;
    }
    if (char === '"') {
      quoted = !quoted;
      continue;
    }
    if (quoted || char !== ':') continue;
    const rawKey = text.slice(0, index).trim();
    const rest = text.slice(index + 1).trim();
    const key = unquoteKey(rawKey, problems, line);
    if (!key) {
      problems.push([
        `第 ${line.number} 行的配置项没有名字。`,
        `Line ${line.number} has an entry without a name.`,
      ]);
      return null;
    }
    return { key, rest };
  }
  problems.push([
    `第 ${line.number} 行不是 \`键: 值\`（缺冒号）：${line.text}`,
    `Line ${line.number} is not a \`key: value\` entry (no colon): ${line.text}`,
  ]);
  return null;
}

/**
 * YAML 单引号标量：`''` 表示一个单引号，其余字符原样（反斜杠不是转义）。标量必须在闭合引号处结束；
 * 没闭合或引号后还有内容时明确报错——静默读成残路径正是审查报告 R10 的问题。
 */
function parseSingleQuoted(text: string, problems: Problem[], line: SourceLine): string | null {
  let value = '';
  for (let index = 1; index < text.length; index++) {
    const char = text[index]!;
    if (char !== "'") {
      value += char;
      continue;
    }
    if (text[index + 1] === "'") {
      value += "'";
      index += 1;
      continue;
    }
    if (index !== text.length - 1) {
      problems.push([
        `第 ${line.number} 行的单引号字符串后面还有内容：${text}`,
        `Line ${line.number} has content after the closing single quote: ${text}`,
      ]);
      return null;
    }
    return value;
  }
  problems.push([
    `第 ${line.number} 行的单引号字符串没有结束：${text}`,
    `Line ${line.number} has an unclosed single-quoted string: ${text}`,
  ]);
  return null;
}

function parseScalar(text: string, problems: Problem[], line: SourceLine): Yaml {
  const trimmed = text.trim();
  if (!trimmed) return null;
  if (trimmed.startsWith('"')) {
    try {
      const value: unknown = JSON.parse(trimmed);
      if (typeof value !== 'string') throw new Error('not a string');
      return value;
    } catch {
      problems.push([
        `第 ${line.number} 行的引号字符串读不懂（注意是 JSON 风格的双引号，内部引号要转义）：${trimmed}`,
        `Line ${line.number} has an unreadable quoted string (JSON-style double quotes; escape inner quotes): ${trimmed}`,
      ]);
      return null;
    }
  }
  if (trimmed.startsWith("'")) return parseSingleQuoted(trimmed, problems, line);
  if (trimmed.startsWith('[')) {
    if (!trimmed.endsWith(']')) {
      problems.push([
        `第 ${line.number} 行的行内列表少了右方括号：${trimmed}`,
        `Line ${line.number} has an unclosed inline list: ${trimmed}`,
      ]);
      return null;
    }
    const inner = trimmed.slice(1, -1).trim();
    if (!inner) return [];
    const parts: string[] = [];
    let current = '';
    // 单引号和双引号都保护内部的逗号（YAML 的流式列表按引号外的逗号切分）。
    let doubleQuoted = false;
    let singleQuoted = false;
    let escaped = false;
    for (let index = 0; index < inner.length; index++) {
      const char = inner[index]!;
      if (doubleQuoted) {
        current += char;
        if (escaped) escaped = false;
        else if (char === '\\') escaped = true;
        else if (char === '"') doubleQuoted = false;
        continue;
      }
      if (singleQuoted) {
        current += char;
        if (char === "'") {
          if (inner[index + 1] === "'") {
            current += "'";
            index += 1;
          } else singleQuoted = false;
        }
        continue;
      }
      if (char === '"') {
        doubleQuoted = true;
        current += char;
        continue;
      }
      if (char === "'") {
        singleQuoted = true;
        current += char;
        continue;
      }
      if (char === ',') {
        parts.push(current);
        current = '';
        continue;
      }
      current += char;
    }
    parts.push(current);
    return parts.map((part) => parseScalar(part, problems, line));
  }
  if (trimmed.startsWith('{')) {
    problems.push([
      `第 ${line.number} 行的行内映射暂不支持；请用缩进写法或重新生成配置。`,
      `Line ${line.number} uses an inline map, which is not supported here; use block style or regenerate the configuration.`,
    ]);
    return null;
  }
  if (trimmed === 'true') return true;
  if (trimmed === 'false') return false;
  if (trimmed === 'null' || trimmed === '~') return null;
  if (/^-?(?:0|[1-9]\d*)(?:\.\d+)?(?:[eE][+-]?\d+)?$/.test(trimmed)) return Number(trimmed);
  // 冒号后带空格在 YAML 里会被当成嵌套映射；生成器给字符串加引号，手改时漏引号要明确提示。
  if (/:\s/.test(trimmed))
    problems.push([
      `第 ${line.number} 行的值里有冒号（会被 YAML 当成嵌套映射）；请把整个值用双引号包起来。`,
      `Line ${line.number} has a colon in its value (YAML would read it as a nested map); wrap the whole value in double quotes.`,
    ]);
  return trimmed;
}

/** 配置里的字符串列表字段（变量、指标、评分）：路径写进错误里，用户知道改哪一段。 */
function stringListField(
  value: Yaml | undefined,
  path: string,
  problems: Problem[],
): string[] | null {
  if (value == null) {
    problems.push([`配置缺少 ${path}。`, `The configuration is missing ${path}.`]);
    return null;
  }
  if (!Array.isArray(value)) {
    problems.push([`${path} 应该是列表。`, `${path} should be a list.`]);
    return null;
  }
  const out: string[] = [];
  for (const item of value) {
    if (typeof item === 'string') out.push(item);
    else
      problems.push([
        `${path} 里的列表项应为字符串。`,
        `${path} has a list item that should be a string.`,
      ]);
  }
  return problems.length ? null : out;
}

function parseBlock(
  lines: SourceLine[],
  start: number,
  indent: number,
  problems: Problem[],
): { value: Yaml; next: number } {
  if (start >= lines.length) return { value: null, next: start };
  if (lines[start]!.text.startsWith('-')) return parseSequence(lines, start, indent, problems);
  return parseMapping(lines, start, indent, problems);
}

function parseSequence(
  lines: SourceLine[],
  start: number,
  indent: number,
  problems: Problem[],
): { value: Yaml[]; next: number } {
  const out: Yaml[] = [];
  let index = start;
  while (index < lines.length) {
    const line = lines[index]!;
    if (line.indent !== indent || !line.text.startsWith('-')) break;
    const rest = line.text.slice(1).trim();
    index++;
    if (!rest) {
      if (index < lines.length && lines[index]!.indent > indent) {
        const nested = parseBlock(lines, index, lines[index]!.indent, problems);
        out.push(nested.value);
        index = nested.next;
      } else out.push(null);
      continue;
    }
    out.push(parseScalar(rest, problems, line));
  }
  return { value: out, next: index };
}

function parseMapping(
  lines: SourceLine[],
  start: number,
  indent: number,
  problems: Problem[],
): { value: { [key: string]: Yaml }; next: number } {
  const out: { [key: string]: Yaml } = {};
  let index = start;
  while (index < lines.length) {
    const line = lines[index]!;
    if (line.indent < indent) break;
    if (line.indent > indent) {
      problems.push([
        `第 ${line.number} 行缩进比同级多；YAML 里这会变成上一项的子项。`,
        `Line ${line.number} is indented deeper than its siblings; YAML would make it a child of the previous entry.`,
      ]);
      index++;
      continue;
    }
    if (line.text.startsWith('-')) break;
    const keyed = splitKey(line.text, problems, line);
    index++;
    if (!keyed) continue;
    if (keyed.key in out) {
      problems.push([
        `第 ${line.number} 行的配置项重复了：${keyed.key}`,
        `Line ${line.number} repeats the key ${keyed.key}.`,
      ]);
      continue;
    }
    if (keyed.rest !== '') {
      out[keyed.key] = parseScalar(keyed.rest, problems, line);
      continue;
    }
    if (index < lines.length && lines[index]!.indent > indent) {
      const nested = parseBlock(lines, index, lines[index]!.indent, problems);
      out[keyed.key] = nested.value;
      index = nested.next;
    } else out[keyed.key] = null;
  }
  return { value: out, next: index };
}

function parseYamlSubset(text: string, problems: Problem[]): Yaml {
  const lines = sourceLines(text, problems);
  if (!lines.length) {
    problems.push([
      '配置是空的。先点"保存评估配置"生成一份，再改。',
      'The configuration is empty. Generate one first with “Save evaluation configuration”, then edit it.',
    ]);
    return null;
  }
  if (lines[0]!.indent !== 0) {
    problems.push([
      '配置第一行不能缩进。',
      'The first line of the configuration must not be indented.',
    ]);
    return null;
  }
  const parsed = parseBlock(lines, 0, 0, problems);
  if (parsed.next < lines.length)
    problems.push([
      `第 ${lines[parsed.next]!.number} 行之后的缩进对不上，读不全整份配置。`,
      `The indentation after line ${lines[parsed.next]!.number} does not fit; the configuration cannot be read completely.`,
    ]);
  return parsed.value;
}

// ---- 校验：只接受本页面生成的那一套键，改用了不认识的字段就明确拒绝 ----

function stringField(value: Yaml | undefined, path: string, problems: Problem[]): string | null {
  if (typeof value === 'string') return value;
  problems.push([`${path} 应该是字符串。`, `${path} should be a string.`]);
  return null;
}

function numberField(
  value: Yaml | undefined,
  path: string,
  problems: Problem[],
  options: { integer?: boolean; positive?: boolean } = {},
): number | null {
  if (typeof value === 'number' && Number.isFinite(value)) {
    if (options.integer && !Number.isInteger(value)) {
      problems.push([`${path} 应该是整数。`, `${path} should be a whole number.`]);
      return null;
    }
    if (options.positive && value <= 0) {
      problems.push([`${path} 应该是正数。`, `${path} should be positive.`]);
      return null;
    }
    return value;
  }
  problems.push([`${path} 应该是数字。`, `${path} should be a number.`]);
  return null;
}

function rangeField(
  value: Yaml | undefined,
  path: string,
  problems: Problem[],
  limit: [number, number],
): [number, number] | null {
  if (value == null) return null;
  if (!Array.isArray(value) || value.length !== 2) {
    problems.push([`${path} 应该是两个数。`, `${path} should be two numbers.`]);
    return null;
  }
  const [first, second] = value;
  if (
    typeof first !== 'number' ||
    typeof second !== 'number' ||
    !Number.isFinite(first) ||
    !Number.isFinite(second)
  ) {
    problems.push([`${path} 的两个值都应该是数字。`, `Both values of ${path} should be numbers.`]);
    return null;
  }
  if (first >= second) {
    problems.push([
      `${path} 要先小后大（现在是 ${first} 到 ${second}）。`,
      `${path} must run small to large (currently ${first} to ${second}).`,
    ]);
    return null;
  }
  if (first < limit[0] || second > limit[1]) {
    problems.push([
      `${path} 超出允许范围 ${limit[0]}–${limit[1]}。`,
      `${path} is outside the allowed range ${limit[0]}–${limit[1]}.`,
    ]);
    return null;
  }
  return [first, second];
}

function rejectUnknownKeys(
  map: { [key: string]: Yaml },
  allowed: Set<string>,
  path: string,
  problems: Problem[],
) {
  for (const key of Object.keys(map))
    if (!allowed.has(key))
      problems.push([
        `${path} 里有页面不认识的配置项「${key}」；手改超出生成格式时请重新生成评估配置后再改。`,
        `${path} has an entry this page does not understand (“${key}”). If your edit goes beyond the generated format, regenerate the evaluation configuration first.`,
      ]);
}

function validateSimulation(
  key: string,
  value: Yaml,
  problems: Problem[],
): ConfigSimulation | null {
  const path = `simulation.${key}`;
  if (!isMap(value)) {
    problems.push([`${path} 应该是一个配置块。`, `${path} should be a block.`]);
    return null;
  }
  if (!/^[A-Za-z0-9_-]{1,60}$/.test(key))
    problems.push([
      `${path}：模拟的键只能用字母、数字、- 和 _（它会变成站点 ID 和文件名）。`,
      `${path}: a simulation key may only use letters, digits, - and _ (it becomes the station ID and a file name).`,
    ]);
  rejectUnknownKeys(
    value,
    new Set([
      'model',
      'root_dir',
      'prefix',
      'data_groupby',
      'data_type',
      'tim_res',
      'grid_res',
      'fulllist',
      'variables',
    ]),
    path,
    problems,
  );
  const model = stringField(value.model, `${path}.model`, problems);
  const rootDir = stringField(value.root_dir, `${path}.root_dir`, problems);
  const groupby = stringField(value.data_groupby, `${path}.data_groupby`, problems);
  const dataType = stringField(value.data_type, `${path}.data_type`, problems);
  const timRes = stringField(value.tim_res, `${path}.tim_res`, problems);
  if (dataType !== 'grid' && dataType !== 'stn') {
    problems.push([
      `${path}.data_type 只能是 grid 或 stn。`,
      `${path}.data_type must be grid or stn.`,
    ]);
    return null;
  }
  // 旧版本生成器给格点模拟的兜底写过小写 `month`；大小写变体都认，统一成 OpenBench 的枚举写法。
  const canonicalGroupby = groupby
    ? DATA_GROUPBY_VALUES.find((value) => value.toLowerCase() === groupby.toLowerCase())
    : undefined;
  if (!canonicalGroupby) {
    problems.push([
      `${path}.data_groupby 只能是 ${DATA_GROUPBY_VALUES.join('、')}。`,
      `${path}.data_groupby must be one of ${DATA_GROUPBY_VALUES.join(', ')}.`,
    ]);
    return null;
  }
  if (!timRes || !(TIM_RES_VALUES as readonly string[]).includes(timRes)) {
    problems.push([
      `${path}.tim_res 只能是 ${TIM_RES_VALUES.join('、')}。`,
      `${path}.tim_res must be one of ${TIM_RES_VALUES.join(', ')}.`,
    ]);
    return null;
  }
  const gridRes =
    value.grid_res == null
      ? null
      : numberField(value.grid_res, `${path}.grid_res`, problems, { positive: true });
  // 两类模拟写不同的键：生成器给格点写 prefix/grid_res，给单点写 fulllist。混进对方的键说明
  // 手改已经超出页面的格式，摘要无法诚实解释，直接拒绝而不是猜用户想干什么。
  if (dataType === 'stn' && (value.prefix != null || value.grid_res != null))
    problems.push([
      `${path} 是单点模拟，不能有 prefix/grid_res（数据源是后端并出来的站点文件）。`,
      `${path} is a station simulation and cannot have prefix/grid_res (its source is the station file prepared by the backend).`,
    ]);
  if (dataType === 'grid' && value.fulllist != null)
    problems.push([
      `${path} 是格点模拟，不能有 fulllist。`,
      `${path} is a grid simulation and cannot have fulllist.`,
    ]);
  // 生成器只给格点模拟写 prefix（单点模拟的 history 由后端并成站点文件，没有可配的前缀）。
  const prefix = dataType === 'grid' ? stringField(value.prefix, `${path}.prefix`, problems) : null;
  const fulllist =
    dataType === 'stn' ? stringField(value.fulllist, `${path}.fulllist`, problems) : null;
  const variables: Record<string, ConfigSimulationVariable> = {};
  if (value.variables != null) {
    if (!isMap(value.variables)) {
      problems.push([
        `${path}.variables 应该是一个配置块。`,
        `${path}.variables should be a block.`,
      ]);
    } else
      for (const [variable, mapping] of Object.entries(value.variables)) {
        const mappingPath = `${path}.variables.${variable}`;
        if (!isMap(mapping)) {
          problems.push([`${mappingPath} 应该是一个配置块。`, `${mappingPath} should be a block.`]);
          continue;
        }
        // 变量映射只支持 varname/varunit：像 OpenBench 的 `convert: "value * 2"` 这类字段会改实际计算，
        // 但页面既写不进有效配置也不算进指纹，接收后忽略就会"计算变了、指纹没变"（审查报告 R02）。
        // 与顶层/模拟层一样，不认识的字段明确拒绝，不静默丢。
        for (const field of Object.keys(mapping))
          if (field !== 'varname' && field !== 'varunit')
            problems.push([
              `${mappingPath} 里的「${field}」暂不支持：变量映射只会写 varname/varunit，${field} 不会进入有效配置与指纹，但 OpenBench 仍可能按它改变计算。请删掉它，或等页面支持后再用。`,
              `${mappingPath} has “${field}”, which this page does not support: a variable mapping only carries varname/varunit, so ${field} would not enter the effective configuration or its fingerprint while OpenBench could still compute differently. Remove it or wait until the page supports it.`,
            ]);
        const varname = stringField(mapping.varname, `${mappingPath}.varname`, problems);
        const varunit = stringField(mapping.varunit, `${mappingPath}.varunit`, problems);
        if (varname && varunit) variables[variable] = { varname, varunit };
      }
  }
  if (!model || !rootDir || (dataType === 'grid' && !prefix) || (dataType === 'stn' && !fulllist))
    return null;
  return {
    key,
    model,
    rootDir,
    prefix,
    dataGroupby: canonicalGroupby,
    dataType,
    timRes,
    gridRes,
    fulllist,
    variables,
  };
}

/**
 * `project.output_dir` 必须是运行目录内的相对路径（复查 R03）。OpenBench 在本次 run 的工作目录里解析它，
 * 绝对路径/`../shared` 会把不同 run 的结果写到同一处（互相覆盖、报告归属错乱），甚至写到运行目录之外。
 * 允许 `output`、`./custom-output`、`a/../out` 这类归一后仍在内部的写法；拒绝绝对路径、盘符、`~`、
 * 归一后逃到上级的路径，以及归一后正好是运行目录本身（`.`、`sim/..`：OpenBench 会把结果散进 run 根）。
 */
function unsafeOutputDir(value: string): Problem | null {
  const normalized = value.replace(/\\/g, '/');
  if (normalized.startsWith('/') || /^[A-Za-z]:/.test(normalized) || normalized.startsWith('~'))
    return [
      `project.output_dir 必须是运行目录内的相对路径，不能是绝对路径或 ~（现在是「${value}」）。`,
      `project.output_dir must be a relative path inside the run directory, not an absolute path or ~ (currently “${value}”).`,
    ];
  let depth = 0;
  for (const part of normalized.split('/')) {
    if (part === '' || part === '.') continue;
    if (part === '..') {
      depth -= 1;
      if (depth < 0)
        return [
          `project.output_dir 不能跳出运行目录（现在是「${value}」）；多个运行共用输出目录会让结果互相覆盖。`,
          `project.output_dir must not escape the run directory (currently “${value}”); a shared output folder lets runs overwrite each other.`,
        ];
    } else depth += 1;
  }
  if (depth === 0)
    return [
      `project.output_dir 不能指向运行目录本身（现在是「${value}」）；请给它一个子目录，例如 output。`,
      `project.output_dir must name a subdirectory of the run directory (currently “${value}”); use e.g. output.`,
    ];
  return null;
}

function validateRoot(root: { [key: string]: Yaml }, problems: Problem[]): EffectiveConfig | null {
  rejectUnknownKeys(root, TOP_LEVEL_KEYS, '顶层', problems);
  const project = root.project;
  if (!isMap(project)) {
    problems.push(['配置缺少 project 段。', 'The configuration has no project section.']);
    return null;
  }
  rejectUnknownKeys(
    project,
    new Set([
      'name',
      'output_dir',
      'years',
      'tim_res',
      'grid_res',
      'lat_range',
      'lon_range',
      'weight',
      'min_year_threshold',
      'time_alignment',
      'num_cores',
    ]),
    'project',
    problems,
  );
  const name = stringField(project.name, 'project.name', problems);
  const outputDir =
    project.output_dir == null
      ? null
      : stringField(project.output_dir, 'project.output_dir', problems);
  if (outputDir != null) {
    // 输出目录会写进 OpenBench 配置：绝对路径/../ 会让两个 run 共用同一成果目录、或写到运行目录之外，
    // 报告发现（按本次 run 的记录找）就找不到本次结果（复查 R03）。这里只接受"归一到运行目录内部"的
    // 相对路径；后端在运行前还会按同一规则再校验一次，两边口径一致才不会被手改绕过。
    const unsafe = unsafeOutputDir(outputDir);
    if (unsafe) problems.push(unsafe);
  }
  const timRes = stringField(project.tim_res, 'project.tim_res', problems);
  if (!timRes || !(TIM_RES_VALUES as readonly string[]).includes(timRes))
    problems.push([
      `project.tim_res 只能是 ${TIM_RES_VALUES.join('、')}。`,
      `project.tim_res must be one of ${TIM_RES_VALUES.join(', ')}.`,
    ]);
  let years: [number, number] | null = null;
  if (!Array.isArray(project.years) || project.years.length !== 2) {
    problems.push(['project.years 应该是两个整数。', 'project.years should be two whole numbers.']);
  } else {
    const [from, to] = project.years;
    if (
      typeof from !== 'number' ||
      typeof to !== 'number' ||
      !Number.isInteger(from) ||
      !Number.isInteger(to)
    )
      problems.push([
        'project.years 应该是两个整数。',
        'project.years should be two whole numbers.',
      ]);
    else if (from > to)
      problems.push([
        `project.years 的起始年份不能晚于结束年份（现在是 ${from} 到 ${to}）。`,
        `project.years starts after it ends (currently ${from} to ${to}).`,
      ]);
    else years = [from, to];
  }
  const gridRes =
    project.grid_res == null
      ? null
      : numberField(project.grid_res, 'project.grid_res', problems, { positive: true });
  const latRange = rangeField(project.lat_range, 'project.lat_range', problems, [-90, 90]);
  const lonRange = rangeField(project.lon_range, 'project.lon_range', problems, [-180, 180]);
  const weight =
    project.weight == null ? null : stringField(project.weight, 'project.weight', problems);
  const minYearThreshold =
    project.min_year_threshold == null
      ? null
      : numberField(project.min_year_threshold, 'project.min_year_threshold', problems, {
          integer: true,
        });
  const timeAlignment =
    project.time_alignment == null
      ? null
      : stringField(project.time_alignment, 'project.time_alignment', problems);
  const numCores =
    project.num_cores == null
      ? null
      : numberField(project.num_cores, 'project.num_cores', problems, {
          integer: true,
          positive: true,
        });

  const evaluation = root.evaluation;
  if (!isMap(evaluation)) {
    problems.push(['配置缺少 evaluation 段。', 'The configuration has no evaluation section.']);
    return null;
  }
  rejectUnknownKeys(evaluation, new Set(['variables']), 'evaluation', problems);
  const variables = stringListField(evaluation.variables, 'evaluation.variables', problems);
  if (variables && !variables.length)
    problems.push([
      'evaluation.variables 至少要有 1 个变量。',
      'evaluation.variables needs at least one variable.',
    ]);
  if (variables)
    for (const variable of variables)
      if (!/^[A-Za-z0-9_]+$/.test(variable))
        problems.push([
          `变量名「${variable}」只能用字母、数字和 _。`,
          `The variable name “${variable}” may only use letters, digits and _.`,
        ]);

  const reference: Record<string, string[]> = {};
  if (!isMap(root.reference)) {
    problems.push(['配置缺少 reference 段。', 'The configuration has no reference section.']);
    return null;
  }
  for (const [variable, refs] of Object.entries(root.reference)) {
    if (typeof refs === 'string') reference[variable] = [refs];
    else if (Array.isArray(refs)) {
      const names = refs.filter(
        (item): item is string => typeof item === 'string' && item.length > 0,
      );
      if (names.length !== refs.length)
        problems.push([
          `reference.${variable} 里有不是字符串的参考数据名。`,
          `reference.${variable} contains a reference name that is not a string.`,
        ]);
      reference[variable] = names;
    } else
      problems.push([
        `reference.${variable} 应该是参考数据名或名字列表。`,
        `reference.${variable} should be a reference name or a list of names.`,
      ]);
  }
  if (variables)
    for (const variable of variables)
      if (!reference[variable]?.length)
        problems.push([
          `变量 ${variable} 没有参考数据（reference.${variable}）。`,
          `Variable ${variable} has no reference data (reference.${variable}).`,
        ]);

  const simulations: ConfigSimulation[] = [];
  if (!isMap(root.simulation) || !Object.keys(root.simulation).length) {
    problems.push([
      '配置缺少 simulation 段（至少 1 个模拟）。',
      'The configuration has no simulation section (at least one simulation is required).',
    ]);
    return null;
  }
  for (const [key, value] of Object.entries(root.simulation)) {
    const simulation = validateSimulation(key, value, problems);
    if (simulation) simulations.push(simulation);
  }

  const metrics = root.metrics == null ? null : stringListField(root.metrics, 'metrics', problems);
  const scores = root.scores == null ? null : stringListField(root.scores, 'scores', problems);
  // comparison 段：不写 = 关闭（OpenBench 默认 enabled=false）。只接受生成器的
  // enabled/items 两个键与 3.0.8 的合法图表名；写错就明确拒绝，摘要才不会与实际配置不符。
  let comparison: EffectiveConfig['comparison'] = { enabled: false, items: null };
  if (root.comparison != null) {
    if (!isMap(root.comparison))
      problems.push(['comparison 应该是一个配置块。', 'comparison should be a block.']);
    else {
      rejectUnknownKeys(root.comparison, new Set(['enabled', 'items']), 'comparison', problems);
      if (root.comparison.enabled != null && typeof root.comparison.enabled !== 'boolean')
        problems.push([
          'comparison.enabled 应该是 true 或 false。',
          'comparison.enabled should be true or false.',
        ]);
      const items =
        root.comparison.items == null
          ? null
          : stringListField(root.comparison.items, 'comparison.items', problems);
      if (root.comparison.enabled === true && items && items.length === 0)
        problems.push([
          'comparison.enabled 为 true 但 items 是空的；一个图表都不选就把整段删掉。',
          'comparison.enabled is true but comparison.items is empty; drop the whole block when no chart is selected.',
        ]);
      for (const item of items ?? [])
        if (!VALID_COMPARISON_ITEMS.has(item))
          problems.push([
            `不是 OpenBench 3.0.8 支持的比较图表：${item}`,
            `OpenBench 3.0.8 does not support the comparison item ${item}`,
          ]);
      comparison = { enabled: root.comparison.enabled === true, items };
    }
  }
  // statistics 段：与 comparison 同型的 opt-in 段；方法名按 OpenBench 3.0.8 statistic_nml +
  // _BASIC_ANALYSIS_ITEMS + False_Discovery_Rate 校验，写错就拒绝而不是等 OpenBench check。
  let statistics: EffectiveConfig['statistics'] = { enabled: false, items: null };
  if (root.statistics != null) {
    if (!isMap(root.statistics))
      problems.push(['statistics 应该是一个配置块。', 'statistics should be a block.']);
    else {
      rejectUnknownKeys(root.statistics, new Set(['enabled', 'items']), 'statistics', problems);
      if (root.statistics.enabled != null && typeof root.statistics.enabled !== 'boolean')
        problems.push([
          'statistics.enabled 应该是 true 或 false。',
          'statistics.enabled should be true or false.',
        ]);
      const items =
        root.statistics.items == null
          ? null
          : stringListField(root.statistics.items, 'statistics.items', problems);
      if (root.statistics.enabled === true && items && items.length === 0)
        problems.push([
          'statistics.enabled 为 true 但 items 是空的；一个方法都不选就把整段删掉。',
          'statistics.enabled is true but statistics.items is empty; drop the whole block when no method is selected.',
        ]);
      for (const item of items ?? [])
        if (!VALID_STATISTICS_ITEMS.has(item))
          problems.push([
            `不是 OpenBench 3.0.8 支持的统计方法：${item}`,
            `OpenBench 3.0.8 does not support the statistics item ${item}`,
          ]);
      statistics = { enabled: root.statistics.enabled === true, items };
    }
  }
  if (problems.length) return null;
  if (!name || !timRes || !years || !variables) return null;
  return {
    project: {
      name,
      outputDir,
      years,
      timRes,
      gridRes,
      latRange,
      lonRange,
      weight,
      minYearThreshold,
      timeAlignment,
      numCores,
    },
    variables,
    reference,
    simulations,
    metrics,
    scores,
    comparison,
    statistics,
  };
}

/** 解析页面要提交的 OpenBench YAML；`config` 为空时 `problems` 说明哪里不能接受。 */
export function parseOpenbenchConfig(text: string): ParsedConfig {
  const problems: Problem[] = [];
  const yaml = parseYamlSubset(text, problems);
  if (problems.length || !isMap(yaml))
    return {
      config: null,
      problems: problems.length
        ? problems
        : [['配置读不懂。', 'The configuration cannot be read.']],
    };
  const config = validateRoot(yaml, problems);
  return { config: problems.length ? null : config, problems };
}

const separatorFor = (path: string) => (path.includes('\\') && !path.includes('/') ? '\\' : '/');

/**
 * 有效配置与页面结构是否对得上：手改可以改年份、时间分辨率、变量、指标、格点数据源，
 * 但不能增删模拟或改单点模拟的站点清单路径——那些文件是后端按页面选中的目录准备出来的，
 * 文本与页面不一致时运行只会读到错的数据，所以运行前明确拒绝（报告：不支持的输入要在运行前阻断）。
 */
export function structuralProblems(design: EvaluationDesign, config: EffectiveConfig): Problem[] {
  const problems: Problem[] = [];
  // 页面选的变量若映射到制备会跳过的层/类别维度变量，运行前明确拒绝（C05）。
  for (const [zh, en] of layerVariableProblems(design)) problems.push([zh, en]);
  const expected = design.simulations.map((sim) => simulationKey(sim.label));
  const actual = config.simulations.map((sim) => sim.key);
  const missing = expected.filter((key) => !actual.includes(key));
  const extra = actual.filter((key) => !expected.includes(key));
  if (missing.length || extra.length) {
    const parts: string[] = [];
    if (missing.length) parts.push(`缺 ${missing.join('、')}`);
    if (extra.length) parts.push(`多 ${extra.join('、')}`);
    problems.push([
      `配置里的模拟与页面选中的不一致（${parts.join('；')}）。手改不能增删模拟；请重新生成评估配置。`,
      `The simulations in the configuration do not match the page (${[missing.length ? `missing ${missing.join(', ')}` : '', extra.length ? `extra ${extra.join(', ')}` : ''].filter(Boolean).join('; ')}). Hand edits cannot add or remove simulations; regenerate the evaluation configuration.`,
    ]);
    return problems;
  }
  // R07 剩余缺口：上面的 `layerVariableProblems` 只看表单 `design.simulations[].variables`，
  // 手改 YAML 把 varname 换成层/类别维度变量仍能启动，制备阶段只会把它跳过。这里按有效配置再查一遍：
  // 被评估变量的 varname 命中同名模拟的 `unsupported_variables` 就在运行前拒绝（生成路径的检查保持不变）。
  const designByKey = new Map(design.simulations.map((sim) => [simulationKey(sim.label), sim]));
  for (const simulation of config.simulations) {
    const unsupported = designByKey.get(simulation.key)?.info.unsupported_variables;
    if (!unsupported?.length) continue;
    for (const [variable, mapping] of Object.entries(simulation.variables)) {
      if (!config.variables.includes(variable)) continue;
      const hit = unsupported.find((item) => item.name === mapping.varname);
      if (!hit) continue;
      const dims = hit.dims.join(', ');
      problems.push([
        `配置里模拟 ${simulation.key} 把变量 ${variable} 映射到 ${mapping.varname}（维度 ${dims}），超出站点/地表评估维度，制备站点文件时会被跳过。手改 YAML 不能绕过生成前的检查；请把 varname 改回去，或重新生成评估配置。`,
        `The configuration maps variable ${variable} in simulation ${simulation.key} to ${mapping.varname} (dims ${dims}), beyond a station/surface evaluation; preparing the station file would skip it. A hand edit cannot bypass the check done at generation; restore the varname or regenerate the evaluation configuration.`,
      ]);
    }
  }
  const sep = separatorFor(design.projectDir);
  const simDir = `${design.projectDir}${sep}sim`;
  for (const simulation of config.simulations) {
    if (simulation.dataType !== 'stn') continue;
    const expectedList = `${simDir}${sep}${simulation.key}.csv`;
    if (simulation.rootDir !== simDir || simulation.fulllist !== expectedList)
      problems.push([
        `模拟 ${simulation.key} 的站点清单路径被改了（应为 ${expectedList}）。站点文件由后端按页面选中的目录准备，手改路径会读到旧数据；请把 root_dir/fulllist 改回去或重新生成评估配置。`,
        `The station list path of simulation ${simulation.key} was edited (expected ${expectedList}). The backend prepares that file from the folder picked on the page, so a hand-edited path reads stale data; restore root_dir/fulllist or regenerate the evaluation configuration.`,
      ]);
  }
  return problems;
}

/**
 * 有效输入指纹：机器/项目目录身份 + 解析后的全部配置字段 + 实际站点清单。
 *
 * 站点清单（单点模拟的 history 目录、文件前缀、页面确认的坐标）不在生成的 YAML 里，但实际计算读的
 * 就是它；旧的指纹只算配置，改了站点目录/坐标指纹不变，摘要还读当前表单 → 摘要/指纹/提交三者分叉
 * （审查报告 R01）。站点按 id 排序后进指纹：清单顺序只是生成时的模拟顺序，不改变实际输入。
 */
export function configFingerprint(
  design: EvaluationDesign,
  config: EffectiveConfig,
  sites: BenchSite[] = [],
): string {
  // 用码点比较而不是 localeCompare：指纹不能随运行环境的语言/ICU 变化。
  const sortedSites = [...sites].sort((left, right) =>
    left.id < right.id ? -1 : left.id > right.id ? 1 : 0,
  );
  return fingerprintOf({
    machine: design.machine,
    projectDir: design.projectDir,
    config,
    sites: sortedSites,
  });
}

/**
 * 生成配置时冻结的输入快照（审查报告 R01）：配置文本本身不冻结（用户可以手改、运行前重新解析，
 * 解析失败/结构校验不过就明确阻断），但表单设计（每个单点模拟的目录/前缀/坐标/年份与模拟集合）
 * 和实际站点清单必须冻结成同一份。摘要的站点部分、有效指纹和运行提交都读它，表单之后再改就过期，
 * 不会再出现"摘要读当前表单、提交读生成时站点"的分叉。
 *
 * 手改文本不能改变冻结的站点输入：生成的 YAML 里没有 sites 段，单点模拟的 history 目录/前缀/坐标
 * 只在这份清单里（后端按它合并站点文件）；手改 stn 的 root_dir/fulllist 会被 `structuralProblems`
 * 按"必须等于页面准备的路径"拒绝。格点模拟的 root_dir/prefix 属于配置输入（不是站点清单），
 * 手改后随有效配置正常进摘要、指纹与提交。这就是"手改 sites 不生效"的明确处理方式。
 */
export interface CompareSnapshot {
  /** 生成时的完整设计：摘要里的模拟名、年份、机器身份和结构校验都读它。 */
  design: EvaluationDesign;
  /** 生成时的设计指纹：与当前表单比对，判断快照是否过期。 */
  designFingerprint: string;
  /** 生成时的站点清单：单点模拟的 history 目录/前缀/坐标，运行原样提交给后端。 */
  sites: BenchSite[];
  /** 生成时的配置文本：只用来判断文本是否被手改（保存徽标/提示），运行时不用它。 */
  generatedText: string;
  /** 生成时的有效输入指纹（配置 + 站点清单）。 */
  fingerprint: string;
}

/** 生成配置后把设计、站点清单、文本和指纹绑成同一个快照；生成的文本必须自己能解析（否则是程序错误）。 */
export function freezeCompareSnapshot(
  design: EvaluationDesign,
  built: { config: string; sites: BenchSite[] },
): CompareSnapshot {
  const parsed = parseOpenbenchConfig(built.config);
  if (!parsed.config)
    throw new Error(
      bilingual(
        `生成的配置无法解析（这是程序错误，不是用户输入问题）：${parsed.problems.map(([zh]) => zh).join('；')}`,
        `The generated configuration cannot be parsed (a program bug, not user input): ${parsed.problems.map(([, en]) => en).join('; ')}`,
      ),
    );
  return {
    design,
    designFingerprint: designFingerprint(design),
    sites: built.sites,
    generatedText: built.config,
    fingerprint: configFingerprint(design, parsed.config, built.sites),
  };
}

/** 快照是否过期：生成配置之后页面表单（站点目录/前缀/坐标、模拟集合、年份等）又改过。 */
export function snapshotStale(snapshot: CompareSnapshot, design: EvaluationDesign): boolean {
  return snapshot.designFingerprint !== designFingerprint(design);
}

/**
 * 运行前条件摘要：与 `evaluationSummary` 同样的行，但两类输入各读各的快照来源——
 * - 配置部分（年份/时间尺度/数据源/变量/指标/高级项）来自当前文本解析出的有效配置，手改 YAML 立即跟着变；
 * - 单点模拟的站点输入（history 目录、前缀、页面确认的坐标）来自生成配置时冻结的站点清单 `sites`。
 * 运行时提交的正是"当前配置 + 冻结站点清单"，摘要读同样两部分，才不会出现摘要=实际提交的分叉（R01）。
 * 不传 `sites` 时退回页面记录（没有快照可读的旧路径）。
 */
export function effectiveSummary(
  design: EvaluationDesign,
  config: EffectiveConfig,
  sites: BenchSite[] = [],
): EvaluationSummaryItem[] {
  const { machine } = design;
  const sep = separatorFor(design.projectDir);
  const simDir = `${design.projectDir}${sep}sim`;
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
    {
      label: ['比较项目目录', 'Comparison project folder'],
      value: `${design.projectDir} · ${bilingual('OpenBench 项目名', 'OpenBench project')} ${config.project.name}`,
    },
    {
      label: ['评估年份', 'Evaluation years'],
      value: `${config.project.years[0]}–${config.project.years[1]}`,
    },
    { label: ['时间尺度', 'Time scale'], value: config.project.timRes },
  ];
  const labels = new Map(design.simulations.map((sim) => [simulationKey(sim.label), sim]));
  // 单点模拟的站点输入以冻结清单为准：那才是提交给后端准备站点文件的 history 目录/前缀/坐标。
  const siteById = new Map(sites.map((site) => [site.id, site]));
  config.simulations.forEach((simulation, index) => {
    const recorded = labels.get(simulation.key);
    const site = simulation.dataType === 'stn' ? siteById.get(simulation.key) : undefined;
    const lon = site ? site.lon : recorded?.info.lon;
    const lat = site ? site.lat : recorded?.info.lat;
    const kind =
      simulation.dataType === 'grid' ? bilingual('格点', 'gridded') : bilingual('单点', 'station');
    items.push({
      label: [`模拟 ${index + 1}`, `Simulation ${index + 1}`],
      value: [
        recorded?.label ?? simulation.key,
        kind,
        simulation.model,
        site?.prefix ?? simulation.prefix ?? recorded?.info.prefix ?? '',
        ...(recorded ? [`${recorded.info.syear}–${recorded.info.eyear}`] : []),
        simulation.dataGroupby,
        simulation.timRes,
        ...(simulation.gridRes != null ? [`${simulation.gridRes}°`] : []),
        ...(lat != null && lon != null && (site || (recorded && !recorded.info.spatial))
          ? [`${lat}°, ${lon}°`]
          : []),
        // 单点的数据源是页面确认的那份 history 目录（后端按它合并站点文件）；配置里的 fulllist 是合并结果。
        ...(site
          ? [site.history_dir]
          : recorded && simulation.dataType === 'stn'
            ? [recorded.info.history_dir]
            : []),
        simulation.dataType === 'stn' ? (simulation.fulllist ?? '') : simulation.rootDir,
      ]
        .filter(Boolean)
        .join(' · '),
    });
  });
  const grid = config.simulations.filter((simulation) => simulation.dataType === 'grid');
  if (grid.length) {
    const target =
      config.project.gridRes != null ? `${config.project.gridRes}°` : bilingual('默认', 'default');
    const region =
      config.project.latRange && config.project.lonRange
        ? `lat [${config.project.latRange[0]}, ${config.project.latRange[1]}] · lon [${config.project.lonRange[0]}, ${config.project.lonRange[1]}]`
        : bilingual(
            '区域未知（OpenBench 按默认全球范围比较）',
            'region unknown (OpenBench compares on the global default grid)',
          );
    items.push({
      label: ['区域与目标分辨率', 'Region and target resolution'],
      value: `${bilingual('目标分辨率', 'target')} ${target} · ${region}`,
    });
  } else {
    items.push({
      label: ['区域与目标分辨率', 'Region and target resolution'],
      value: bilingual('只有单点模拟，不裁区域', 'stations only; no region crop'),
    });
  }
  items.push({
    label: ['变量与参考数据', 'Variables and reference data'],
    value: config.variables
      .map((variable) => `${variable}: ${(config.reference[variable] ?? []).join(', ')}`)
      .join('; '),
  });
  items.push({
    label: ['指标与评分', 'Metrics and scores'],
    value: `${bilingual('指标', 'metrics')} [${(config.metrics ?? []).join(', ')}]${config.metrics ? '' : bilingual('（默认）', ' (defaults)')} · ${bilingual('评分', 'scores')} [${(config.scores ?? []).join(', ')}]${config.scores ? '' : bilingual('（默认）', ' (defaults)')}`,
  });
  items.push({
    label: ['跨模拟比较', 'Cross-simulation comparison'],
    value: config.comparison.enabled
      ? `${bilingual('已启用', 'enabled')} · ${bilingual('图表', 'items')} [${
          config.comparison.items
            ? config.comparison.items.join(', ')
            : bilingual('默认 Taylor_Diagram, HeatMap', 'defaults Taylor_Diagram, HeatMap')
        }]`
      : bilingual('未启用（comparison.enabled=false）', 'disabled (comparison.enabled=false)'),
  });
  items.push({
    label: ['统计检验', 'Statistical tests'],
    value: config.statistics.enabled
      ? `${bilingual('已启用', 'enabled')} · ${bilingual('方法', 'items')} [${
          config.statistics.items
            ? config.statistics.items.join(', ')
            : bilingual('默认', 'defaults')
        }]`
      : bilingual('未启用（statistics.enabled=false）', 'disabled (statistics.enabled=false)'),
  });
  const advanced = [
    config.project.weight ? `weight=${config.project.weight}` : '',
    config.project.minYearThreshold != null ? `min_year=${config.project.minYearThreshold}` : '',
    config.project.timeAlignment ? `time_alignment=${config.project.timeAlignment}` : '',
    config.project.numCores != null ? `num_cores=${config.project.numCores}` : '',
  ].filter(Boolean);
  items.push({
    label: ['高级选项', 'Advanced options'],
    value: advanced.length ? advanced.join(' · ') : bilingual('默认', 'defaults'),
  });
  // 单点模拟的站点清单来自后端准备（sim/<key>.csv）；把目录也列出来，摘要不暗示手改 YAML 能换数据源。
  const stations = config.simulations.filter((simulation) => simulation.dataType === 'stn');
  if (stations.length)
    items.push({
      label: ['单点站点文件目录', 'Station files folder'],
      value: simDir,
    });
  return items;
}
