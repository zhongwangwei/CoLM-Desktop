import { bilingual } from '../../lib/bilingual';
import type { Site } from '../../lib/types';
export type { Site } from '../../lib/types';
export interface SchemaField {
  name: string;
  kind: string;
  arity?: number | null;
  default: string;
  doc?: string;
  group?: string;
  derived: boolean;
  values: string[];
  section: string;
}
// Case fields are filtered against model_fields by each caller.
export function isCaseField(field: SchemaField): boolean {
  return !field.derived && !!field.group && !/^DEF_USE_(USGS|IGBP)$/.test(field.name);
}
export function requiredNumber(
  value: string,
  label: string,
  t?: (zh: string, en: string) => string,
): number {
  if (!value.trim() || !Number.isFinite(Number(value))) {
    // React 页面传 t 得到当前语言；非 React 调用回退到 bilingual（无 document 时为中文，D-09）。
    const say = t ?? bilingual;
    throw new Error(`${label}: ${say('请输入有效数字', 'enter a number')}`);
  }
  return Number(value);
}

/** 日期接受 `YYYY-MM-DD` 或 `YYYYMMDD`，且必须是真实存在的日期（D-09）。 */
export function prepDateValid(text: string): boolean {
  const compact = text.trim().replace(/-/g, '');
  if (!/^\d{8}$/.test(compact)) return false;
  const year = Number(compact.slice(0, 4));
  const month = Number(compact.slice(4, 6));
  const day = Number(compact.slice(6, 8));
  if (month < 1 || month > 12 || day < 1 || day > 31) return false;
  const date = new Date(Date.UTC(year, month - 1, day));
  return (
    date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day
  );
}

/** 把两种日期写法归一成可比较的 `YYYYMMDD`。 */
export function prepDateKey(text: string): string {
  return text.trim().replace(/-/g, '');
}
export function caseDirectory(root: string, name: string): string {
  if (
    !root.trim() ||
    !name ||
    name !== name.trim() ||
    /^--/.test(name) ||
    /[\\/:]|^\.{1,2}$|\p{Cc}/u.test(name) ||
    new TextEncoder().encode(name).length > 256
  )
    throw new Error(
      name.trim().startsWith('--')
        ? bilingual(
            '算例名不能以 -- 开头：命令行会把它当成参数。',
            'A case name must not start with --: the command line would treat it as a flag.',
          )
        : bilingual('请填写根目录和有效算例名', 'root and valid case name required'),
    );
  return `${root.replace(/[\\/]+$/, '')}/${name.trim()}`;
}
export function physicsFields(physics: Record<string, string> = {}) {
  return Object.entries(physics).map(([path, value]) => ({ path, value }));
}
export function versionMode(
  physics: Record<string, string> = {},
  classification?: 'IGBP' | 'USGS',
): string {
  const enabled = (key: string) => /^(true|\.true\.|1)$/i.test(physics[key] ?? '');
  const sub = enabled('DEF_USE_PFT')
    ? 'pft'
    : enabled('DEF_USE_PC')
      ? 'pc'
      : classification === 'USGS' || (classification === undefined && enabled('DEF_USE_USGS'))
        ? 'usgs'
        : 'igbp';
  return enabled('DEF_URBAN_RUN') ? `urban-${sub}` : sub;
}
export function displayLiteral(value: string): string {
  return /^(['"]).*\1$/.test(value) ? value.slice(1, -1) : value;
}
/** Fortran strings compare case-insensitively: show `'monthly'` as MONTHLY, keep unknown values visible. */
export function selectChoice(value: string, options: readonly string[]) {
  const match = options.find((option) => option.toLowerCase() === value.toLowerCase());
  if (match !== undefined) return { value: match, options: [...options] };
  return { value, options: value ? [...options, value] : [...options] };
}
export function siteVersionMismatch(
  site: Pick<Site, 'crop' | 'urban'>,
  physics: Record<string, string> = {},
): string | null {
  const enabled = (key: string) => /^(true|\.true\.|1)$/i.test(physics[key] ?? '');
  if (site.crop !== cropEnabled(physics))
    return bilingual(
      '站点作物类型与当前算例 CROP 配置不符',
      "The site's crop type does not match this case's CROP configuration",
    );
  if (site.urban !== enabled('DEF_URBAN_RUN'))
    return bilingual('站点城市类型与当前算例配置不符', 'Urban configuration mismatch');
  return null;
}
/**
 * 前处理产物的默认落盘名。必须和 `colm-cli` 扫描器（`main.rs` 的 `LAYOUTS`）使用同一套
 * 命名约定，否则前处理→扫描→建例配对不上（GUI 审查 D-06 / 复核 P2）：
 *
 * | 数据集 | 站点 | 强迫场 | 观测 |
 * |---|---|---|---|
 * | PLUMBER2 | `<X>_site.nc` | `<X>_Met.nc` | `<X>_Flux.nc` |
 * | Urban-PLUMBER | `<X>_site_v1.nc` | `<X>_metforcing_v1.nc` | `<X>_clean_observations_v1.nc` |
 *
 * 源文件已带约定后缀时，**只剥一层完整后缀**再换成同布局的目标后缀：
 * `A_Met_site.nc` 的站名是 `A_Met`，强迫场应为 `A_Met_Met.nc`；
 * 合法的 `A_Met.nc` 作为源时不能再剥成 `A`。源文件没有约定后缀时按 PLUMBER2 追加。
 */
const PREPARED_LAYOUTS: {
  site: string;
  met: string;
  flux: string;
}[] = [
  { site: '_site.nc', met: '_Met.nc', flux: '_Flux.nc' },
  {
    site: '_site_v1.nc',
    met: '_metforcing_v1.nc',
    flux: '_clean_observations_v1.nc',
  },
];

export function preparedDestination(
  directory: string,
  source: string,
  kind: 'site' | 'Met' | 'Flux' = 'Met',
): string {
  const name = source.split(/[\\/]/).pop() ?? 'forcing';
  const target = kind === 'site' ? 'site' : kind === 'Met' ? 'met' : 'flux';
  // 与扫描器同样逐布局、逐源后缀精确匹配（大小写敏感）；只剥一层。
  for (const layout of PREPARED_LAYOUTS) {
    for (const sourceKind of ['site', 'met', 'flux'] as const) {
      const suffix = layout[sourceKind];
      if (!name.endsWith(suffix)) continue;
      return `${directory.replace(/[\\/]$/, '')}/${name.slice(0, -suffix.length)}${layout[target]}`;
    }
  }
  const stem = name.replace(/\.[^.]+$/, '') || 'forcing';
  const suffix = kind === 'site' ? '_site.nc' : kind === 'Met' ? '_Met.nc' : '_Flux.nc';
  return `${directory.replace(/[\\/]$/, '')}/${stem}${suffix}`;
}

export function effectiveModelFields(
  overlay?: {
    physics: Record<string, string>;
    parameters: { raw_key: string; value: string; scope_instance: { kind: string } }[];
  },
  modelFields?: readonly Pick<SchemaField, 'name'>[],
): Record<string, string> {
  if (!overlay) return {};
  const values = {
    ...overlay.physics,
    ...Object.fromEntries(
      overlay.parameters
        .filter((p) => p.scope_instance.kind === 'case-scalar')
        .map((p) => [p.raw_key, p.value]),
    ),
  };
  return modelFields
    ? Object.fromEntries(
        Object.entries(values).filter(([key]) =>
          modelFields.some((field) => field.name.toLowerCase() === key.toLowerCase()),
        ),
      )
    : values;
}
export function cropEnabled(fields: Record<string, string>): boolean {
  return Object.keys(fields).some((key) => key.toUpperCase() === 'DEF_TUNING_CROP_PLANTING_DAY');
}

export function casePreparationOptions(
  profile?: { classification?: 'IGBP' | 'USGS'; subgrid?: string; urban?: boolean; crop?: boolean },
  seed?: { form: Record<string, string>; caseValues?: Record<string, string> } | null,
) {
  const classification = seed
    ? seed.form.landcover === 'USGS'
      ? 'USGS'
      : 'IGBP'
    : (profile?.classification ?? (profile?.subgrid === 'USGS' ? 'USGS' : 'IGBP'));
  const subgrid = (
    seed ? (seed.form.subgrid ?? classification) : (profile?.subgrid ?? classification)
  ).toUpperCase();
  const scheme =
    subgrid === 'PFT' || subgrid === 'PC' ? subgrid.toLowerCase() : classification.toLowerCase();
  const urban = seed
    ? /^(true|\.true\.|1)$/i.test(seed.caseValues?.DEF_URBAN_RUN ?? '')
    : profile?.urban === true;
  return {
    mode: urban ? `urban-${scheme}` : scheme,
    crop: seed ? cropEnabled(seed.caseValues ?? {}) : profile?.crop === true,
    classification,
  };
}

export type SiteDefaults = {
  previous: Record<string, string>;
  values: Record<string, string>;
};

export function withoutSiteDefaults(values: Record<string, string>, defaults: SiteDefaults) {
  const next = { ...values };
  for (const [key, value] of Object.entries(defaults.values)) {
    if (next[key] !== value) continue;
    if (key in defaults.previous) next[key] = defaults.previous[key];
    else delete next[key];
  }
  return next;
}

export function isMethaneSite(site: Site) {
  return (
    !site.urban &&
    !site.crop &&
    (site.name === 'AT-Neu' || /ch4/i.test(site.obs_file ?? '') || /ch4/i.test(site.site_file))
  );
}

/**
 * 第 4 步 `model_configuration` 接受的算例覆盖值。
 *
 * forcing 组（`nl_colm_forcing`）的字段是从 forcing.nml 读进来的"文件实际值"，不是算例标量
 * 覆盖；后端 `typed()` 会拿 schema 旧值表校验 `DEF_forcing%dataset`，而 22 个 GUI 空间模板里
 * 17 个的 dataset 不在那份 8 值白名单里（ERA5/CMFD/CRA40…），整份配置查询会因此失败——
 * 空间模板算例一打开第 4 步就红字锁死（GUI 审查 F-N-07，已真机复现）。这里按字段组过滤；
 * forcing 字段仍由第 3 步控件与 forcing.nml 链路负责保存和回读。
 */
export function configurationCaseValues(
  values: Record<string, string>,
  schema: readonly Pick<SchemaField, 'name' | 'group'>[],
): Record<string, string> {
  const byName = new Map(schema.map((field) => [field.name.toLowerCase(), field] as const));
  const out: Record<string, string> = {};
  for (const [key, value] of Object.entries(values)) {
    const field = byName.get(key.toLowerCase());
    // 只过滤 forcing 组；schema 里没有的键（界面临时 scheme 字段等）保持原样传下去。
    if (field?.group === 'nl_colm_forcing') continue;
    out[field?.name ?? key] = value;
  }
  return out;
}

/** 「制作站点」可选的地表覆盖/植被方案。 */
export const PREPARATION_SITE_MODES = [
  'igbp',
  'usgs',
  'pft',
  'pc',
  'urban-igbp',
  'urban-usgs',
  'urban-pft',
  'urban-pc',
] as const;

/**
 * CROP 内核只接受 PFT/PC 结构：用 IGBP/USGS 建站会在站点审计（site.rs `crop_enabled &&
 * subgrid == Lct`）时才失败，GUI 应提前只给可行选项（GUI 审查 F-D-04）。
 */
export function preparationSiteModes(crop: boolean): readonly string[] {
  return crop ? ['pft', 'pc', 'urban-pft', 'urban-pc'] : PREPARATION_SITE_MODES;
}

/**
 * 同一次保存里"强迫场路径/数据集"与「更多配置」里的 forcing 字段都改会被后端拒绝
 * （两处都写 forcing.nml）；前端先给出可操作的提示（GUI 审查 N-08）。
 */
export function forcingSaveConflict(
  forcingDirty: boolean,
  fields: readonly { path: string }[],
): boolean {
  return (
    forcingDirty && fields.some((field) => field.path.toLowerCase().startsWith('def_forcing%'))
  );
}

/** 站点文件被新算例扫描识别所需的两种布局后缀。 */
export const PREPARED_SITE_SUFFIXES = ['_site.nc', '_site_v1.nc'] as const;

/** 站点输出文件名须以扫描器认识的 `_site.nc`/`_site_v1.nc` 结尾（D-06 残留）。 */
export function preparedSiteNameValid(path: string): boolean {
  const name = path.split(/[\\/]/).pop() ?? '';
  return PREPARED_SITE_SUFFIXES.some((suffix) => name.toLowerCase().endsWith(suffix));
}

/** 选目录后给站点文件补一个扫描器认识的名字；沿用当前文件名词干，没有就用 `site`。 */
export function preparedSiteDestination(directory: string, current = ''): string {
  const base = directory.replace(/[\\/]+$/, '');
  const name = current.split(/[\\/]/).pop() ?? '';
  const stem =
    name
      .replace(/_site(_v1)?\.nc$/i, '')
      .replace(/\.[^.]+$/, '')
      .trim() || 'site';
  return `${base}/${stem}_site.nc`;
}

export interface PreparedDatasetDestinations {
  site?: string;
  met?: string;
  /** 站点与强迫场词干是否一致（扫描按同一词干配对；不一致时安装后扫不到配对的强迫场）。 */
  stemsMatch: boolean;
}

/** 把准备页产物映射到数据集根下的 Sitedata/Forcing 正式路径（D-06 残留）。
 *  PLUMBER2 布局：`Sitedata/<X>_site.nc` + `Forcing/<X>_Met.nc`。 */
export function preparedDatasetDestinations(
  produced: { site?: string; met?: string },
  root: string,
): PreparedDatasetDestinations {
  const base = root.replace(/[\\/]+$/, '');
  const stemOf = (path: string, suffixes: RegExp) =>
    (path.split(/[\\/]/).pop() ?? '').replace(suffixes, '').trim();
  const siteStem = produced.site ? stemOf(produced.site, /_site(_v1)?\.nc$/i) : '';
  const metStem = produced.met ? stemOf(produced.met, /_(Met|metforcing_v1)\.nc$|\.[^.]+$/i) : '';
  return {
    ...(produced.site ? { site: `${base}/Sitedata/${siteStem || 'site'}_site.nc` } : {}),
    ...(produced.met ? { met: `${base}/Forcing/${metStem || 'forcing'}_Met.nc` } : {}),
    stemsMatch: !produced.site || !produced.met || siteStem === metStem,
  };
}
