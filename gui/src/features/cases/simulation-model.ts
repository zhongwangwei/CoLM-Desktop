import { bilingual } from '../../lib/bilingual';
import { caseDirectory, siteVersionMismatch, type SchemaField, type Site } from './case-model';

export type SimulationStep = 1 | 2 | 3 | 4 | 5;
export const simulationSteps = [
  ['总的设定', 'General setup'],
  ['范围与网格', 'Range and grid'],
  ['数据与时段', 'Data and period'],
  ['模型结构与方案', 'Model structure and schemes'],
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
  if (!validDate(value)) throw new Error(bilingual('请输入有效日期', 'Enter a valid date'));
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
export function caseRangeKeys(grid?: string | null): string[] {
  if (!grid) return ['SITE_lon_location', 'SITE_lat_location'];
  const bounds = ['DEF_domain%edgew', 'DEF_domain%edgee', 'DEF_domain%edges', 'DEF_domain%edgen'];
  return [
    ...bounds,
    ...(grid === 'latlon'
      ? ['DEF_GRIDBASED_lon_res', 'DEF_GRIDBASED_lat_res']
      : grid === 'catchment'
        ? ['DEF_CatchmentMesh_data']
        : ['DEF_file_mesh']),
  ];
}
export const primaryDataFields = [
  'SITE_fsitedata',
  'DEF_forcing_namelist',
  'DEF_dir_rawdata',
  'DEF_dir_runtime',
  'DEF_dir_existing_srfdata',
  // 预热与模拟时段同属“数据与时段”这一步，不在输出步骤重复出现。
  'DEF_simulation_time%spinup_repeat',
];
export const primaryOutputFields = [
  'DEF_simulation_time%timestep',
  'DEF_simulation_time%greenwich',
  'DEF_HIST_FREQ',
  'DEF_WRST_FREQ',
];
export const legacyMpiFields = [
  'DEF_nx_blocks',
  'DEF_ny_blocks',
  'DEF_PIO_groupsize',
  'DEF_nIO_eq_nBlock',
  'DEF_BlockInfoFile',
];
export function isSiteConfiguration(name: string): boolean {
  const key = name.toUpperCase();
  return (
    /^(SITE_|USE_SITE_)/.test(key) &&
    ![
      'SITE_FSITEDATA',
      'SITE_LON_LOCATION',
      'SITE_LAT_LOCATION',
      'USE_SITE_HISTWRITEBACK',
      'USE_SITE_FORCINGREADAHEAD',
    ].includes(key)
  );
}
export function isForcingScheme(name: string): boolean {
  return (
    /^(DEF_DS_|DEF_FORCING%CBL_)/i.test(name) ||
    [
      'DEF_FORCING_INTERP_METHOD',
      'DEF_USE_CBL_HEIGHT',
      'DEF_USE_FORCING_DOWNSCALING',
      'DEF_USE_FORCING_DOWNSCALING_SIMPLE',
    ].includes(name.toUpperCase())
  );
}
export function simulationFieldStep(field: Pick<SchemaField, 'name' | 'section'>): SimulationStep {
  const name = field.name.toUpperCase();
  if (name === 'DEF_CASE_NAME') return 1;
  if (name === 'USE_SITE_HISTWRITEBACK') return 5;
  if (['DEF_LC_YEAR', 'USE_SITE_FORCINGREADAHEAD'].includes(name)) return 3;
  if (['SITE_lon_location', 'SITE_lat_location'].includes(field.name)) return 2;
  if (field.section === '网格与并行' || /^DEF_domain%/.test(field.name)) return 2;
  if (field.name.startsWith('DEF_simulation_time%'))
    return /%(start|end|spinup)_/.test(field.name) ? 3 : 5;
  if (['输出与重启', '输出变量', '调试与诊断'].includes(field.section)) return 5;
  if (isSiteConfiguration(field.name)) return 3;
  if (isForcingScheme(field.name)) return 3;
  if (
    ['水热过程', '生态与生地化', '河道与水库', '数据同化', '示踪剂', '城市'].includes(field.section)
  )
    return 4;
  if (
    /^DEF_LAI_/i.test(field.name) ||
    [
      'DEF_LANDONLY',
      'DEF_USE_DOMINANT_PATCHTYPE',
      'DEF_HIGHRESSOIL',
      'DEF_HIGHRESVEG',
      'DEF_HIGHRESURBAN_ALBEDO',
    ].includes(field.name.toUpperCase())
  )
    return 4;
  return 3;
}
export function simulationFields(
  schema: SchemaField[],
  step: SimulationStep,
  advanced: boolean,
  engine: string = 'rust',
): SchemaField[] {
  if (step === 1 || step === 4) return [];
  if (step === 2) {
    if (engine === 'rust' || !advanced) return [];
    return schema.filter((f) => simulationFieldStep(f) === 2 && !legacyMpiFields.includes(f.name));
  }
  return schema.filter(
    (f) =>
      ![
        'DEF_USE_LCT',
        'DEF_USE_PFT',
        'DEF_USE_PC',
        'DEF_SUBGRID_SCHEME',
        'DEF_dir_output',
      ].includes(f.name) &&
      simulationFieldStep(f) === step &&
      !legacyMpiFields.includes(f.name) &&
      !/^DEF_simulation_time%(start|end|spinup)_(year|month|day)$/.test(f.name) &&
      // 预热只以日期界定；“预热截止秒”含义易混，不再单独暴露。
      f.name !== 'DEF_simulation_time%spinup_sec' &&
      // 历史开关由输出变量卡片管理；“默认输出历史变量”这一同步规则与卡片选择重复。
      // DEF_HIST_mode 不在此处屏蔽：单点由后端隐藏，只有空间 Fortran 引擎才真有 'block' 可选。
      !['DEF_HIST_vars_out_default', 'DEF_HIST_vars_namelist'].includes(f.name) &&
      !f.name.startsWith('DEF_hist_vars%') &&
      (advanced || (step === 5 ? primaryOutputFields : primaryDataFields).includes(f.name)),
  );
}
export function newCaseGeneralProblem(form: Record<string, string>, en = false): string | null {
  const t = (zh: string, english: string) => (en ? english : zh);
  if (!form.engine?.trim() || !['rust', 'fortran'].includes(form.engine))
    return t('请选择计算核心。', 'Choose a computation core.');
  if (!form.root?.trim())
    return t('请选择保存模拟的项目目录。', 'Choose a project folder for this simulation.');
  if (!form.name?.trim()) return t('请填写模拟名称。', 'Enter a simulation name.');
  try {
    caseDirectory(form.root, form.name);
  } catch {
    return t('请填写有效的模拟名称。', 'Enter a valid simulation name.');
  }
  return null;
}
export function newCaseRangeProblem(form: Record<string, string>, en = false): string | null {
  const t = (zh: string, english: string) => (en ? english : zh);
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
  replayStart = false,
): string | null {
  if (!cutoff) return null;
  if (!validDate(cutoff) || cutoff <= start || (replayStart ? cutoff > end : cutoff >= end))
    return replayStart
      ? en
        ? 'The first spinup year must fit within the simulation period.'
        : '模拟时段须覆盖完整的第一年预热。'
      : en
        ? 'Spinup cutoff must be after the start and before the end.'
        : '预热截止日期须晚于开始日期、早于结束日期。';
  if (!/^\d+$/.test(repeat) || Number(repeat) < 1)
    return en ? 'Spinup repetitions must be a positive integer.' : '预热重复次数须为正整数。';
  return null;
}
export function newCaseProblem(
  form: Record<string, string>,
  step: SimulationStep,
  site: Site | undefined,
  physics: Record<string, string>,
  en = false,
  replayStart = false,
): string | null {
  const t = (zh: string, english: string) => (en ? english : zh);
  const generalProblem = newCaseGeneralProblem(form, en);
  if (generalProblem) return generalProblem;
  if (step === 1) return null;
  const rangeProblem = newCaseRangeProblem(form, en);
  if (rangeProblem) return rangeProblem;
  if (step === 2) return null;
  if (step >= 3) {
    if (!validDate(form.start) || !validDate(form.end) || form.end < form.start)
      return t(
        '请填写有效时段，结束日期不能早于开始日期。',
        'Choose valid dates with the end on or after the start.',
      );
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
      if (!form.rawdata || !form.runtime)
        return t(
          '请提供地表数据与 Runtime 数据目录。',
          'Provide surface data and runtime directories.',
        );
      const configurationFile =
        form.forcing_mode === 'config' || (!form.forcing_mode && !!form.forcing);
      if (
        configurationFile
          ? !form.forcing?.trim()
          : !form.spatial_forcingdir?.trim() || !form.forcing_dataset
      )
        return configurationFile
          ? t('请选择强迫场配置文件。', 'Choose a forcing configuration file.')
          : t(
              '请选择强迫场数据路径，并识别或确认数据集。',
              'Choose the forcing data path and identify or confirm its dataset.',
            );
      if (!configurationFile) {
        const path = form.spatial_forcingdir.trim();
        const absolute = form.host
          ? path.startsWith('/')
          : /^(\/|[A-Za-z]:[\\/]|\\\\[^\\/]+[\\/][^\\/]+)/.test(path);
        if (!absolute)
          return t(
            '强迫场数据路径须为绝对路径，请使用目录选择按钮。',
            'Use an absolute forcing data path or the folder picker.',
          );
      }
      if (
        !form.timestep?.trim() ||
        !Number.isFinite(Number(form.timestep)) ||
        Number(form.timestep) <= 0
      )
        return t('时间步长须为正数。', 'The time step must be positive.');
    }
  }
  if (step === 3) return null;
  if (step >= 4) {
    if (!['IGBP', 'USGS'].includes(form.landcover))
      return t('请选择 IGBP 或 USGS 地表分类。', 'Choose IGBP or USGS land-cover classification.');
  }
  if (step === 4) return null;
  return spinupProblem(
    form.start,
    form.end,
    form.spinup_end,
    physics['DEF_simulation_time%spinup_repeat'] ?? '1',
    en,
    replayStart,
  );
}
