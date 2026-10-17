import { caseDirectory, versionMode, type Site } from './case-model';
import { caseSchemeFields, simulationMode } from './model-settings';
import { caseDatePatch, newCaseProblem, spinupProblem, validDate } from './simulation-model';

export interface MultiSiteDraft {
  site: string;
  name: string;
  start: string;
  end: string;
  spinup_end: string;
  spinup_repeat: string;
}
export type MultiSiteMode = 'first-year' | 'per-site';

export function siteDraft(site: Site): MultiSiteDraft {
  return {
    site: site.site_file,
    name: site.name,
    start: site.start?.slice(0, 10) ?? '',
    end: site.end?.slice(0, 10) ?? '',
    spinup_end: '',
    spinup_repeat: '1',
  };
}

export function firstYearCutoff(start: string): string {
  const cutoff = `${String(Number(start.slice(0, 4)) + 1).padStart(4, '0')}${start.slice(4)}`;
  if (!validDate(start) || !validDate(cutoff))
    throw new Error(
      '第一年预热需要有效的周年日期；请改用分站点配置。 / First-year spinup requires a valid anniversary; use per-site settings.',
    );
  return cutoff;
}

export function planMultiSites({
  form,
  sites,
  drafts,
  mode,
  repeat,
  physics,
  cropDefaults,
  en = false,
  step,
}: {
  form: Record<string, string>;
  sites: Site[];
  drafts: MultiSiteDraft[];
  mode: MultiSiteMode;
  repeat: string;
  physics: Record<string, string>;
  cropDefaults: Record<string, string>;
  en?: boolean;
  step: 3 | 5;
}): { site: Site; form: Record<string, string>; fields: Record<string, string>; mode: string }[] {
  const fail = (zh: string, english: string, name?: string): never => {
    const error = new Error(`${name ? `${name}: ` : ''}${en ? english : zh}`);
    error.name = 'MultiSiteValidationError';
    throw error;
  };
  if (mode === 'first-year' && form.engine && form.engine !== 'rust')
    fail('首年重复预热需要 Rust 计算引擎。', 'First-year replay spinup requires the Rust engine.');
  if (!drafts.length) fail('请至少选择一个站点。', 'Select at least one site.');
  let root: string;
  try {
    root = caseDirectory(form.root, form.name);
  } catch {
    fail('请填写有效的项目目录和模拟名称。', 'Enter a valid project folder and simulation name.');
  }
  const paths = new Set<string>();
  const names = new Set<string>();
  return drafts.map((draft) => {
    if (paths.has(draft.site))
      fail('站点不能重复选择。', 'A site cannot be selected twice.', draft.name);
    paths.add(draft.site);
    const nameKey = draft.name.toLowerCase();
    if (names.has(nameKey)) fail('算例名称不能重复。', 'Case names must be unique.', draft.name);
    names.add(nameKey);
    const site = sites.find((item) => item.site_file === draft.site);
    if (
      !site ||
      !site.met_file ||
      !site.start ||
      !site.end ||
      site.problem ||
      !validDate(site.start.slice(0, 10)) ||
      !validDate(site.end.slice(0, 10)) ||
      site.start.slice(0, 10) > site.end.slice(0, 10)
    )
      fail(
        site?.problem || '请重新扫描完整的站点与强迫场数据。',
        site?.problem || 'Rescan complete site and forcing data.',
        draft.name,
      );
    const scanned = site!;
    let cutoff = draft.spinup_end;
    if (mode === 'first-year') {
      try {
        cutoff = firstYearCutoff(draft.start);
      } catch (error) {
        fail(String((error as Error).message), String((error as Error).message), draft.name);
      }
    }
    const repetitions = mode === 'first-year' ? repeat : draft.spinup_repeat;
    if (
      cutoff &&
      (!/^\d+$/.test(repetitions) || Number(repetitions) < 1 || Number(repetitions) > 2147483647)
    )
      fail(
        '预热重复次数须为 1 至 2147483647 的整数。',
        'Spinup repetitions must be an integer from 1 to 2147483647.',
        draft.name,
      );
    const spinupIssue = spinupProblem(
      draft.start,
      draft.end,
      cutoff,
      repetitions,
      en,
      mode === 'first-year',
    );
    if (spinupIssue) fail(spinupIssue, spinupIssue, draft.name);
    const classification =
      scanned.urban || scanned.crop ? 'IGBP' : form.landcover || (step === 3 ? 'IGBP' : '');
    const sharedMode = simulationMode(
      undefined,
      classification === 'IGBP' || classification === 'USGS' ? classification : undefined,
      form.subgrid,
    );
    const structure = scanned.urban
      ? 'IGBP'
      : scanned.crop
        ? sharedMode === 'PC'
          ? 'PC'
          : 'PFT'
        : sharedMode;
    if (!structure)
      fail(
        '请选择地表分类与植被结构。',
        'Choose land-cover classification and vegetation structure.',
        draft.name,
      );
    const fields: Record<string, string> = {
      ...physics,
      ...caseSchemeFields(structure),
      DEF_URBAN_RUN: scanned.urban ? '.true.' : '.false.',
    };
    if (scanned.crop) {
      if (!cropDefaults.DEF_TUNING_CROP_PLANTING_DAY)
        fail('作物默认配置尚未加载。', 'Crop defaults have not loaded.', draft.name);
      Object.assign(
        fields,
        Object.fromEntries(
          Object.entries(cropDefaults).map(([key, value]) => [key, physics[key] ?? value]),
        ),
        {
          ...caseSchemeFields(structure),
          DEF_USE_BGC: '.true.',
          DEF_USE_LULCC: '.false.',
          DEF_URBAN_RUN: '.false.',
        },
      );
    } else {
      delete fields.DEF_TUNING_CROP_PLANTING_DAY;
      if (scanned.urban)
        Object.assign(fields, { DEF_USE_BGC: '.false.', DEF_USE_TRACER: '.false.' });
    }
    if (structure !== 'PFT') fields.DEF_USE_IRRIGATION = '.false.';
    Object.assign(
      fields,
      cutoff
        ? {
            ...caseDatePatch(cutoff, 'spinup'),
            'DEF_simulation_time%spinup_sec': '0',
            'DEF_simulation_time%spinup_repeat': repetitions,
          }
        : { 'DEF_simulation_time%spinup_year': '0' },
    );
    const caseForm = {
      ...form,
      root: root!,
      name: draft.name,
      site: scanned.site_file,
      met: scanned.met_file!,
      start: draft.start,
      end: draft.end,
      spinup_end: cutoff,
      landcover: classification,
      subgrid: structure!,
    };
    const effectivePhysics = fields;
    const issue = newCaseProblem(
      caseForm,
      step,
      { ...scanned, start: scanned.start!.slice(0, 10), end: scanned.end!.slice(0, 10) },
      effectivePhysics,
      en,
      mode === 'first-year',
    );
    if (issue) fail(issue, issue, draft.name);
    return {
      site: scanned,
      form: caseForm,
      fields,
      mode: versionMode(effectivePhysics, classification as 'IGBP' | 'USGS'),
    };
  });
}
