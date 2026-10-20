import { RECOMMENDED_VARIABLES } from '../cases/history-variables';

/** One variable of the history catalog (`colm-cli history-catalog`). */
export type CatalogVariable = {
  name: string;
  units?: string | null;
  long_name?: string | null;
  kind: string;
  dimensions: unknown[];
};

// 输出文件里的名字是 `f_<开关名>`；模拟页的中文名表按开关名登记，所以去掉前缀再查。
const byName = new Map(RECOMMENDED_VARIABLES.map((item, order) => [item.name, { item, order }]));
const known = (name: string) => byName.get(name.replace(/^f_/, ''));

/** 变量的人话名字：模拟页那套中英文名；没登记的用文件里的 `long_name`；都没有才用原名。 */
export function variableTitle(name: string, en: boolean, longName?: string | null): string {
  const entry = known(name)?.item;
  if (entry) return en ? entry.label_en : entry.label_zh;
  return longName?.trim() || name;
}

/** 下拉框里的一项：名字后面带上原始变量名，方便和 NetCDF 对照。 */
export function variableOption(variable: CatalogVariable, en: boolean): string {
  const title = variableTitle(variable.name, en, variable.long_name);
  const units = variable.units?.trim();
  const named = title === variable.name ? title : `${title}（${variable.name}）`;
  return units && units !== '-' ? `${named} · ${units}` : named;
}

/**
 * 能画成时序的变量：单点的 `(time)` 序列，或区域模拟的 `(time, lat, lon)` 格点（画区域平均）。
 * 坐标、常量、剖面不列。模拟页推荐的核心量排在前面，其余按名字。
 */
export function chartVariables(variables: CatalogVariable[]): CatalogVariable[] {
  return variables
    .filter((item) => (item.kind === 'series' || item.kind === 'grid') && item.name !== 'time')
    .sort((a, b) => {
      const ka = known(a.name)?.order ?? Infinity,
        kb = known(b.name)?.order ?? Infinity;
      return ka === kb ? a.name.localeCompare(b.name) : ka - kb;
    });
}

/** 这次结果是不是区域（格点）模拟：有任何格点变量就是。 */
export function isGridCatalog(variables: CatalogVariable[]): boolean {
  return variables.some((item) => item.kind === 'grid');
}

/**
 * 读结果的错误换成人话：服务器上的引擎还是旧版时，格点变量的区域平均和空间分布图都读不了
 * （旧版 `series` 报 "is not a (time, patch) series"，旧版没有 `history-map` 这个命令）。
 */
export function readErrorText(error: unknown, en: boolean): string {
  const text = String(error);
  if (/not a \(time, patch\) series|history-map|single-point scalar time series/.test(text))
    return en
      ? 'The engine on the server is too old to read grid results. Submit one run on this server with this app version (it builds the new engine), then look again.'
      : '服务器上的引擎版本较旧，还读不了格点结果。用新版软件在这台服务器上提交一次运行（会编译新引擎）后再看。';
  return text;
}
