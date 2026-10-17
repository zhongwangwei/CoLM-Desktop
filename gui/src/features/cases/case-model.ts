import { bilingual } from '../../lib/bilingual';
import type { Site } from '../../lib/types';
export type { Site } from '../../lib/types';
export interface SchemaField {
  name: string;
  kind: string;
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
export function requiredNumber(value: string, label: string): number {
  if (!value.trim() || !Number.isFinite(Number(value)))
    throw new Error(`${label}: 请输入有效数字 / enter a number`);
  return Number(value);
}
export function caseDirectory(root: string, name: string): string {
  if (
    !root.trim() ||
    !name ||
    name !== name.trim() ||
    /[\\/:]|^\.{1,2}$|\p{Cc}/u.test(name) ||
    new TextEncoder().encode(name).length > 256
  )
    throw new Error(bilingual('请填写根目录和有效算例名', 'root and valid case name required'));
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
export function preparedDestination(directory: string, source: string): string {
  const stem = (source.split(/[\\/]/).pop() ?? 'forcing').replace(/\.[^.]+$/, '');
  return `${directory.replace(/[\\/]$/, '')}/${stem}_colm.nc`;
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
