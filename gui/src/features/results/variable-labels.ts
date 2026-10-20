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
