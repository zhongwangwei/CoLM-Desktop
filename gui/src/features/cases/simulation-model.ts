import { caseDirectory, siteVersionMismatch, type SchemaField, type Site } from './case-model';

export type SimulationStep = 1 | 2 | 3 | 4 | 5;
export const simulationSteps = [
  ['范围与网格', 'Range and grid'],
  ['模型结构与方案', 'Model structure and schemes'],
  ['数据与时段', 'Data and period'],
  ['预热与输出', 'Spinup and output'],
  ['检查与运行', 'Review and run'],
] as const;
export function validDate(value: string): boolean {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || Number(value.slice(0, 4)) < 1) return false;
  const date = new Date(`${value}T00:00:00Z`);
  return Number.isFinite(date.getTime()) && date.toISOString().slice(0, 10) === value;
}
export function caseDate(
  values: Record<string, string>,
  prefix: 'start' | 'end' | 'spinup',
): string {
  const parts = ['year', 'month', 'day'].map((part) =>
    Number(values[`DEF_simulation_time%${prefix}_${part}`]),
  );
  const value = `${String(parts[0]).padStart(4, '0')}-${String(parts[1]).padStart(2, '0')}-${String(parts[2]).padStart(2, '0')}`;
  return validDate(value) ? value : '';
}
export function caseDatePatch(
  value: string,
  prefix: 'start' | 'end' | 'spinup',
): Record<string, string> {
  if (!validDate(value)) throw new Error('请输入有效日期 / Enter a valid date');
  return Object.fromEntries(
    ['year', 'month', 'day'].map((part, i) => [
      `DEF_simulation_time%${prefix}_${part}`,
      String(Number(value.split('-')[i])),
    ]),
  );
}
export function casePeriodValid(values: Record<string, string>): boolean {
  const start = caseDate(values, 'start'),
    end = caseDate(values, 'end');
  const first =
    Date.parse(`${start}T00:00:00Z`) + Number(values['DEF_simulation_time%start_sec']) * 1000;
  const last =
    Date.parse(`${end}T00:00:00Z`) + Number(values['DEF_simulation_time%end_sec']) * 1000;
  return !!start && !!end && Number.isFinite(first) && Number.isFinite(last) && last > first;
}
export function caseSpinupValid(values: Record<string, string>): boolean {
  if (Number(values['DEF_simulation_time%spinup_year']) === 0) return true;
  const stamps = (['start', 'end', 'spinup'] as const).map(
    (prefix) =>
      Date.parse(`${caseDate(values, prefix)}T00:00:00Z`) +
      Number(values[`DEF_simulation_time%${prefix}_sec`]) * 1000,
  );
  return stamps.every(Number.isFinite) && stamps[0] < stamps[2] && stamps[2] < stamps[1];
}
export function casePeriodText(values: Record<string, string>): string {
  const start = caseDate(values, 'start'),
    end = caseDate(values, 'end');
  if (!start || !end) return '';
  if (
    Number(values['DEF_simulation_time%start_sec']) === 0 &&
    Number(values['DEF_simulation_time%end_sec']) === 0
  )
    return `${start} — ${end}`;
  const dates = (['start', 'end'] as const).map(
    (prefix) =>
      new Date(
        Date.parse(`${caseDate(values, prefix)}T00:00:00Z`) +
          Number(values[`DEF_simulation_time%${prefix}_sec`]) * 1000,
      ),
  );
  return dates.every((date) => Number.isFinite(date.getTime()))
    ? dates.map((date) => date.toISOString().replace('T', ' ').slice(0, 19)).join(' — ')
    : '';
}
export function scannedForcing(site: Site | undefined, met: string): boolean {
  return !!site?.met_file && site.met_file === met && !!site.start && !!site.end && !site.problem;
}
export const primaryDataFields = [
  'SITE_fsitedata',
  'DEF_forcing_namelist',
  'DEF_dir_rawdata',
  'DEF_dir_runtime',
  'DEF_dir_existing_srfdata',
];
export const primaryOutputFields = [
  'DEF_simulation_time%timestep',
  'DEF_simulation_time%greenwich',
  'DEF_simulation_time%spinup_repeat',
  'DEF_dir_output',
  'DEF_HIST_FREQ',
  'DEF_HIST_groupby',
  'DEF_WRST_FREQ',
];
export function simulationFieldStep(field: SchemaField): 1 | 3 | 4 {
  if (field.section === '网格与并行' || /^DEF_domain%/.test(field.name)) return 1;
  if (field.name.startsWith('DEF_simulation_time%'))
    return /%(start|end)_(year|month|day|sec)$/.test(field.name) ? 3 : 4;
  return ['输出与重启', '输出变量', '调试与诊断'].includes(field.section) ? 4 : 3;
}
export function simulationFields(
  schema: SchemaField[],
  step: SimulationStep,
  advanced: boolean,
): SchemaField[] {
  if (step === 2 || step === 5) return [];
  return schema.filter(
    (f) =>
      simulationFieldStep(f) === step &&
      !/^DEF_simulation_time%(start|end|spinup)_(year|month|day)$/.test(f.name) &&
      (advanced || (step === 4 ? primaryOutputFields : primaryDataFields).includes(f.name)),
  );
}
export function newCaseRangeProblem(form: Record<string, string>, en = false): string | null {
  const t = (zh: string, english: string) => (en ? english : zh);
  try {
    caseDirectory(form.root || '.', form.name);
  } catch {
    return t('请填写有效的模拟名称。', 'Enter a valid simulation name.');
  }
  if (
    !['site', 'region', 'watershed', 'global'].includes(form.kind) ||
    (form.kind !== 'site' && !['latlon', 'unstructured', 'catchment'].includes(form.grid))
  )
    return t('请选择有效范围与网格类型。', 'Choose a valid range and grid type.');
  if (form.kind === 'region' && form.grid === 'latlon') {
    const coordinates = ['west', 'east', 'south', 'north'].map((key) =>
        form[key]?.trim() ? Number(form[key]) : NaN,
      ),
      [west, east, south, north] = coordinates;
    if (
      coordinates.some((n) => !Number.isFinite(n)) ||
      Math.abs(west) > 180 ||
      Math.abs(east) > 180 ||
      west === east ||
      south < -90 ||
      north > 90 ||
      south >= north
    )
      return t('请填写有效的经纬度范围。', 'Enter valid longitude and latitude bounds.');
  }
  if (form.kind !== 'site') {
    if (
      form.grid === 'latlon' &&
      ['dlon', 'dlat'].some(
        (key) =>
          !form[key]?.trim() || !Number.isFinite(Number(form[key])) || Number(form[key]) <= 0,
      )
    )
      return t('网格分辨率须为正数。', 'Grid resolution must be positive.');
    if (
      (form.grid === 'unstructured' && !form.mesh) ||
      (form.grid === 'catchment' && !form.catchment) ||
      (form.kind === 'watershed' && form.grid === 'latlon' && !form.shapefile)
    )
      return t(
        '请提供当前网格或流域需要的文件。',
        'Provide the files required by the selected grid or watershed.',
      );
  }
  return null;
}
export function spinupProblem(
  start: string,
  end: string,
  cutoff: string,
  repeat: string,
  en = false,
): string | null {
  if (!cutoff) return null;
  if (!validDate(cutoff) || cutoff <= start || cutoff >= end)
    return en
      ? 'Spinup cutoff must be after the start and before the end.'
      : '预热截止日期须晚于开始日期、早于结束日期。';
  if (!/^\d+$/.test(repeat) || Number(repeat) < 1)
    return en ? 'Spinup repetitions must be a positive integer.' : '预热重复次数须为正整数。';
  return null;
}
export function newCaseProblem(
  form: Record<string, string>,
  step: 1 | 2,
  site: Site | undefined,
  physics: Record<string, string>,
  en = false,
): string | null {
  const t = (zh: string, english: string) => (en ? english : zh);
  const rangeProblem = newCaseRangeProblem(form, en);
  if (rangeProblem) return rangeProblem;
  if (!validDate(form.start) || !validDate(form.end) || form.end < form.start)
    return t(
      '请填写有效时段，结束日期不能早于开始日期。',
      'Choose valid dates with the end on or after the start.',
    );
  if (step === 1) return null;
  if (!form.root.trim())
    return t('请选择保存模拟的项目目录。', 'Choose a project folder for this simulation.');
  if (form.kind === 'site') {
    if (!form.site || !form.met)
      return t('请选择站点文件与强迫场文件。', 'Choose the site and forcing files.');
    if (site?.problem && site.met_file === form.met) return site.problem;
    const mismatch = site ? siteVersionMismatch(site, physics) : null;
    if (mismatch) return mismatch;
    if (
      site &&
      scannedForcing(site, form.met) &&
      (form.start < site.start! || form.end > site.end!)
    )
      return t(
        '模拟时段超出已扫描的强迫场覆盖。',
        'The period exceeds the scanned forcing coverage.',
      );
    if (/^(true|\.true\.|1)$/i.test(physics.DEF_USE_BGC ?? '') && !form.runtime)
      return t(
        '当前模型需要 Runtime 数据目录，请展开高级配置。',
        'This model requires runtime data; open Advanced settings.',
      );
  } else {
    if (!form.rawdata || !form.runtime || !form.forcing)
      return t(
        '请提供地表数据、Runtime 数据与强迫场配置。',
        'Provide surface data, runtime data and forcing configuration.',
      );
    if (
      !form.timestep?.trim() ||
      !Number.isFinite(Number(form.timestep)) ||
      Number(form.timestep) <= 0
    )
      return t('时间步长须为正数。', 'The time step must be positive.');
  }
  return null;
}

export const processDataSections = [
  ['bgc', 'BGC 初始化与氮沉降', 'BGC initialization and nitrogen deposition', 'DEF_USE_BGC'],
  ['river', '河网与水库输入', 'River and reservoir inputs', 'DEF_USE_GridRiverLakeFlow'],
  ['tracer', '示踪初始化与驱动数据', 'Tracer initialization and driving data', 'DEF_USE_TRACER'],
  ['urban', '城市输入数据', 'Urban input data', 'DEF_URBAN_RUN'],
] as const;
export function processDataGroupId(field: SchemaField): string | null {
  const name = field.name.toUpperCase();
  if (
    name === 'DEF_USE_CN_INIT' ||
    name === 'DEF_FILE_CN_INIT' ||
    name.includes('NDEP') ||
    field.section === '生态与生地化'
  )
    return 'bgc';
  if (name.startsWith('DEF_TRACER_') || name === 'DEF_FILE_GIEMS' || field.section === '示踪剂')
    return 'tracer';
  if (name.includes('URBAN') || field.section === '城市') return 'urban';
  return field.section === '河道与水库' ? 'river' : null;
}
