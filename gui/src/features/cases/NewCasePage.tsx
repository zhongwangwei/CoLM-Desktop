import { useEffect, useRef, useState } from 'react';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { invoke } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import { Button, Card, Field, Input, Select, Advanced, Notice } from '../../components/ui';
import {
  caseDirectory,
  requiredNumber,
  physicsFields,
  versionMode,
  siteVersionMismatch,
  effectiveModelFields,
  isCaseField,
  type SchemaField,
  type Site,
} from './case-model';
import {
  caseDatePatch,
  newCaseProblem,
  newCaseRangeProblem,
  scannedForcing,
  simulationFields,
  simulationSteps,
  processDataSections,
  processDataGroupId,
  spinupProblem,
  type SimulationStep,
} from './simulation-model';
import { fieldLabel, optionLabel } from '../../lib/param-presentation';
import { caseFieldVisible, type ConfigurationField } from '../versions/configuration-options';
import { compatibleKernel } from '../runs/run-model';
import ProcessDataGroups from './ProcessDataGroups';
import SimulationSteps from './SimulationSteps';
import ModelSettings from './ModelSettings';
import { simulationMode, caseSchemeFields, type SimulationMode } from './model-settings';
import type { CaseOptionPatch } from '../versions/configuration-options';
export default function NewCasePage() {
  const { state, update, openCase, navigate } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const seed = state.newCaseSeed;
  const [form, setForm] = useState<Record<string, string>>({
    root: state.root ?? '',
    name: '',
    kind: 'site',
    landcover: 'IGBP',
    sitedir: '',
    forcingdir: '',
    site: '',
    met: '',
    rawdata: '',
    runtime: '',
    start: '',
    end: '',
    spinup_end: '',
    grid: 'latlon',
    west: '',
    east: '',
    south: '',
    north: '',
    dlon: '',
    dlat: '',
    mesh: '',
    catchment: '',
    shapefile: '',
    forcing: '',
    timestep: '1800',
    ...seed?.form,
    ...(['PFT', 'PC'].includes(seed?.form.subgrid ?? '') ? { landcover: 'IGBP' } : {}),
  });
  const [caseValues, setCaseValues] = useState<Record<string, string>>(seed?.caseValues ?? {}),
    [schema, setSchema] = useState<SchemaField[]>([]),
    [schemaReady, setSchemaReady] = useState(false),
    [modelFields, setModelFields] = useState<SchemaField[]>([]);
  const [sites, setSites] = useState<Site[]>(seed?.sites ?? []),
    [step, setStep] = useState<SimulationStep>(seed?.step ?? 1),
    [hint, setHint] = useState(''),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [ownData, setOwnData] = useState(seed?.ownData ?? false);
  const [dataOpened, setDataOpened] = useState<Record<string, boolean>>({}),
    [optionStates, setOptionStates] = useState<ConfigurationField[]>([]),
    [optionError, setOptionError] = useState('');
  const lock = useRef(false),
    sequence = useRef(0),
    alive = useRef(true),
    physics = {
      ...effectiveModelFields(state.version?.overlay, modelFields),
      ...caseValues,
      ...caseSchemeFields(
        simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid)!,
      ),
    },
    selected = sites.find((s) => s.site_file === form.site);
  useEffect(() => {
    alive.current = true;
    update({ newCaseSeed: null, simulationReturn: null });
    return () => {
      alive.current = false;
    };
  }, [update]);
  useEffect(() => {
    let current = true;
    setSchemaReady(false);
    void Promise.all([
      invoke<SchemaField[]>('describe_fields'),
      invoke<SchemaField[]>('model_fields'),
    ])
      .then(([all, model]) => {
        if (!current) return;
        const supplied = [
          'DEF_USE_LCT',
          'DEF_USE_PFT',
          'DEF_USE_PC',
          'DEF_SUBGRID_SCHEME',
          'SITE_fsitedata',
          'DEF_forcing_namelist',
          'DEF_dir_rawdata',
          'DEF_dir_runtime',
          'DEF_dir_output',
        ];
        setModelFields(model);
        setSchema(
          all
            .filter(isCaseField)
            .filter(
              (f) =>
                !model.some((m) => m.name === f.name) &&
                !supplied.includes(f.name) &&
                !f.name.startsWith('DEF_hist_vars%') &&
                !/^DEF_(domain%|GRIDBASED_|file_mesh|CatchmentMesh)/.test(f.name),
            ),
        );
        setSchemaReady(true);
      })
      .catch((e) => {
        if (current) setError(String(e));
      });
    return () => {
      current = false;
    };
  }, []);
  const configurationKernel = compatibleKernel(
    state.kernels,
    {
      grid: form.kind === 'site' ? null : form.grid,
      classification: form.landcover as 'IGBP' | 'USGS',
    },
    physics,
  );
  const dataPatch = {
    ...caseSchemeFields(
      simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid)!,
    ),
    ...Object.fromEntries(
      Object.entries(caseValues).filter(
        ([key, value]) =>
          schema.some((f) => f.name === key) &&
          (value !== '' || schema.find((f) => f.name === key)?.kind.startsWith('Character')),
      ),
    ),
    ...(form.site ? { SITE_fsitedata: form.site } : {}),
    ...(form.rawdata ? { DEF_dir_rawdata: form.rawdata } : {}),
    ...(form.runtime ? { DEF_dir_runtime: form.runtime } : {}),
    ...(form.kind !== 'site' && form.forcing ? { DEF_forcing_namelist: form.forcing } : {}),
  };
  const dataRequest = JSON.stringify({
    source: { kind: 'frozen', version: state.versionRef },
    caseDir: null,
    kernelDir: configurationKernel?.dir ?? null,
    pftSelection: null,
    caseValues: dataPatch,
  });
  useEffect(() => {
    let current = true;
    if (!schemaReady || !state.version) return;
    setOptionError('');
    void invoke<{ fields: ConfigurationField[] }>('model_configuration', JSON.parse(dataRequest))
      .then((result) => {
        if (current) setOptionStates(result.fields);
      })
      .catch((e) => {
        if (current) setOptionError(String(e));
      });
    return () => {
      current = false;
    };
  }, [dataRequest, schemaReady, state.version]);
  useNavigationGuard(() =>
    lock.current
      ? t(
          '正在准备模拟，请等待当前操作完成。',
          'Wait for the current simulation operation to finish.',
        )
      : null,
  );
  const set = (k: string, v: string) => {
    setError('');
    const s = k === 'site' ? sites.find((site) => site.site_file === v) : undefined;
    setForm((f) => ({
      ...f,
      [k]: v,
      ...(s ? { name: s.name, met: s.met_file ?? '', start: s.start ?? '', end: s.end ?? '' } : {}),
    }));
  };
  function setOption(name: string, value: string) {
    setError('');
    setCaseValues((old) => {
      const next = { ...old };
      if (value) next[name] = value;
      else delete next[name];
      return next;
    });
    if (name === 'DEF_simulation_time%timestep')
      setForm((old) => ({ ...old, timestep: value || '1800' }));
  }
  async function work<T>(fn: () => Promise<T>) {
    if (lock.current) throw new Error(t('请等待当前操作。', 'Wait for the current operation.'));
    lock.current = true;
    setBusy(true);
    setError('');
    try {
      return await fn();
    } catch (e) {
      if (alive.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      if (alive.current) setBusy(false);
    }
  }
  async function scan(dir = form.sitedir, forcingDir = form.forcingdir) {
    const id = ++sequence.current;
    const result = await invoke<{ sites: Site[]; hint?: string }>('scan_sites', {
      dir,
      forcingDir: forcingDir || null,
      quick: false,
    });
    if (alive.current && id === sequence.current) {
      setSites(result.sites);
      setHint(result.hint ?? '');
      setForm((f) => {
        const s = result.sites.find((site) => site.site_file === f.site);
        if (
          !s ||
          !scannedForcing(s, s.met_file ?? '') ||
          siteVersionMismatch(s, physics) ||
          (f.met && f.met !== s.met_file && f.met !== selected?.met_file)
        )
          return f;
        return { ...f, met: s.met_file ?? '', start: s.start ?? '', end: s.end ?? '' };
      });
    }
    return result;
  }
  async function example() {
    return work(async () => {
      const x = await invoke<{
        sitedir: string;
        forcingdir: string;
        root: string;
        runtimedir: string;
      }>('install_example', {});
      const found = await scan(x.sitedir, x.forcingdir);
      if (!alive.current) return;
      const s = found.sites.find((site) => site.name === 'CN-Cng');
      setForm((f) => ({
        ...f,
        sitedir: x.sitedir,
        forcingdir: x.forcingdir,
        root: f.root || x.root,
        runtime: x.runtimedir,
        ...(s
          ? {
              name: s.name,
              site: s.site_file,
              met: s.met_file ?? '',
              start: s.start ?? '',
              end: s.end ?? '',
            }
          : {}),
      }));
      return { sites: found.sites.map((s) => s.name) };
    });
  }
  function selectSite(path: string) {
    set('site', path);
  }
  const folderKeys = ['root', 'sitedir', 'forcingdir', 'rawdata', 'runtime'],
    fileKeys = ['site', 'met', 'mesh', 'catchment', 'shapefile', 'forcing'];
  async function pick(k: string) {
    return work(async () => {
      const path = folderKeys.includes(k)
        ? await invoke<string | null>('pick_folder', { key: `new-${k}` })
        : await invoke<string | null>('pick_file', { key: `new-${k}`, filter: null });
      if (path) set(k, path);
      return { path, cancelled: !path };
    });
  }
  function problem(at: SimulationStep) {
    if (at >= 2 && !['IGBP', 'USGS'].includes(form.landcover))
      return t('请选择 IGBP 或 USGS 地表分类。', 'Choose IGBP or USGS land-cover classification.');
    return at <= 2
      ? newCaseRangeProblem(form, en)
      : newCaseProblem(form, 2, selected, physics, en) ||
          (at >= 4
            ? spinupProblem(
                form.start,
                form.end,
                form.spinup_end,
                caseValues['DEF_simulation_time%spinup_repeat'] ?? '1',
                en,
              )
            : null);
  }
  function next() {
    const issue = problem(step);
    if (issue) {
      setError(issue);
      throw new Error(issue);
    }
    setError('');
    setStep((step + 1) as SimulationStep);
  }
  function openResearch() {
    update({
      newCaseSeed: {
        form,
        sites,
        caseValues: {
          ...caseValues,
          ...caseSchemeFields(
            simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid)!,
          ),
        },
        step: 2,
        ownData,
      },
      simulationReturn: { route: 'new-case', step: 2 },
    });
    return navigate('research');
  }
  function changeCaseOptions(patch: CaseOptionPatch) {
    setError('');
    setCaseValues((old) => {
      const next = { ...old };
      for (const [key, value] of Object.entries(patch)) {
        if (value === null) delete next[key];
        else next[key] = value;
      }
      return next;
    });
  }
  function selectMode(mode: SimulationMode) {
    setForm({ ...form, subgrid: mode, landcover: mode === 'USGS' ? 'USGS' : 'IGBP' });
    setError('');
  }
  const editedOptions = Object.entries(caseValues).filter(
    ([name, value]) =>
      name !== 'DEF_dir_output' && value !== '' && schema.some((f) => f.name === name),
  );
  async function create() {
    return work(async () => {
      const issue = problem(5);
      if (issue) throw new Error(issue);
      if (
        !schemaReady ||
        !state.version ||
        state.version.version.id !== state.versionRef.id ||
        state.version.version.revision !== state.versionRef.revision
      )
        throw new Error(t('模型与设置尚未读取完成。', 'Wait for model and settings to load.'));
      const out = caseDirectory(form.root, form.name),
        revision = state.versionRef;
      const changed = editedOptions.map(([path, value]) => ({ path, value }));
      const spinup = form.spinup_end
        ? {
            ...caseDatePatch(form.spinup_end, 'spinup'),
            'DEF_simulation_time%spinup_sec': caseValues['DEF_simulation_time%spinup_sec'] ?? '0',
          }
        : { 'DEF_simulation_time%spinup_year': '0' };
      const fields = [
          ...physicsFields(effectiveModelFields(state.version?.overlay, modelFields)),
          ...physicsFields(
            caseSchemeFields(
              simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid)!,
            ),
          ),
          ...changed,
          ...Object.entries(spinup).map(([path, value]) => ({ path, value })),
        ],
        mode = versionMode(physics, form.landcover as 'IGBP' | 'USGS');
      if (form.kind === 'site')
        await invoke('new_case', {
          site: form.site,
          out,
          name: form.name,
          start: form.start,
          end: form.end,
          rawdata: form.rawdata || null,
          runtime: form.runtime || null,
          met: form.met,
          mode,
          fields,
          modelRevision: revision,
        });
      else {
        const number = (k: string) => requiredNumber(form[k], k),
          bounds = form.kind === 'region' && form.grid === 'latlon';
        await invoke('new_spatial_case', {
          domain: form.kind,
          gridKind: form.grid,
          shapefile: form.shapefile || null,
          west: bounds ? number('west') : null,
          east: bounds ? number('east') : null,
          south: bounds ? number('south') : null,
          north: bounds ? number('north') : null,
          dlon: form.grid === 'latlon' ? number('dlon') : null,
          dlat: form.grid === 'latlon' ? number('dlat') : null,
          meshFile: form.mesh || null,
          catchmentFile: form.catchment || null,
          out,
          name: form.name,
          forcing: form.forcing,
          forcingDataset: null,
          forcingDir: null,
          rawdata: form.rawdata,
          runtime: form.runtime,
          start: form.start,
          end: form.end,
          timestep: number('timestep'),
          mode,
          fields,
          modelRevision: revision,
        });
      }
      update({ root: form.root, versionRef: revision });
      lock.current = false;
      return await openCase(out);
    });
  }
  const labels: Record<string, string> = {
    root: t('保存到项目目录', 'Project folder'),
    name: t('模拟名称', 'Simulation name'),
    sitedir: t('站点数据目录', 'Site data folder'),
    forcingdir: t('强迫场目录', 'Forcing folder'),
    site: t('站点文件', 'Site file'),
    met: t('强迫场文件', 'Forcing file'),
    rawdata: t('地表数据目录', 'Surface data folder'),
    runtime: t('Runtime 数据目录', 'Runtime data folder'),
    start: t('开始日期', 'Start date'),
    end: t('结束日期', 'End date'),
    spinup_end: t('预热截止日期（留空不预热）', 'Spinup cutoff (blank disables spinup)'),
    west: t('西界（°）', 'West (°)'),
    east: t('东界（°）', 'East (°)'),
    south: t('南界（°）', 'South (°)'),
    north: t('北界（°）', 'North (°)'),
    dlon: t('经度分辨率（°）', 'Longitude resolution (°)'),
    dlat: t('纬度分辨率（°）', 'Latitude resolution (°)'),
    mesh: t('网格文件', 'Mesh file'),
    catchment: t('汇水区网格文件', 'Catchment file'),
    shapefile: t('流域边界文件', 'Watershed boundary'),
    forcing: t('强迫场配置文件', 'Forcing configuration'),
  };
  const rangeKeys = [
    ...(form.kind === 'region' && form.grid === 'latlon' ? ['west', 'east', 'south', 'north'] : []),
    ...(form.kind !== 'site'
      ? form.grid === 'latlon'
        ? ['dlon', 'dlat']
        : form.grid === 'catchment'
          ? ['catchment']
          : ['mesh']
      : []),
    ...(form.kind === 'watershed' && form.grid === 'latlon' ? ['shapefile'] : []),
  ];
  const dataKeys = [
    'root',
    ...(form.kind === 'site'
      ? ['site', 'met', 'rawdata', 'runtime']
      : ['rawdata', 'runtime', 'forcing']),
  ];
  const keys =
    step === 1
      ? ['name', ...rangeKeys]
      : step === 3
        ? [
            ...dataKeys,
            'start',
            'end',
            ...(form.kind === 'site' && ownData ? ['sitedir', 'forcingdir'] : []),
          ]
        : step === 4
          ? ['spinup_end']
          : [];
  const visibleSchema = schema.filter((field) => caseFieldVisible(field.name, optionStates));
  const enabled = !busy && !state.busy,
    dataGroups =
      step === 3
        ? processDataSections
            .filter(([, , , parent]) => /^(true|\.true\.)$/i.test(physics[parent] ?? ''))
            .map(([id, zh, english, parent]) => ({
              id,
              title: t(zh, english),
              fields: visibleSchema.filter((f) => processDataGroupId(f) === id),
              open: dataOpened[id] ?? /^(true|\.true\.)$/i.test(physics[parent] ?? ''),
            }))
            .filter((group) => group.fields.length)
        : [];
  const groupFields = schema.filter((field) => processDataGroupId(field) !== null),
    options = [
      ...simulationFields(visibleSchema, step, advanced).filter((f) => !groupFields.includes(f)),
      ...dataGroups.filter((group) => group.open).flatMap((group) => group.fields),
    ],
    basicOptions = simulationFields(visibleSchema, step, false).filter(
      (f) => !groupFields.includes(f),
    );
  const optionState = (name: string) => optionStates.find((field) => field.name === name),
    optionDisabled = (name: string) =>
      optionState(name)?.mode === 'disabled' || optionState(name)?.mode === 'readonly';
  const domains = [
    ['site', t('单点', 'Single site'), t('一个观测站或位置', 'One station or location')],
    ['watershed', t('流域', 'Watershed'), t('按流域边界开展模拟', 'Simulate a watershed')],
    ['region', t('区域', 'Region'), t('指定区域或网格范围', 'Choose a spatial region')],
    ['global', t('全球', 'Global'), t('覆盖全球陆地', 'Simulate global land')],
  ];
  const grids = [
    [
      'latlon',
      t('经纬度网格', 'Latitude/longitude'),
      t('规则网格与经纬度分辨率', 'Regular grid and resolution'),
    ],
    [
      'unstructured',
      t('非结构网格', 'Unstructured'),
      t('读取已有网格文件', 'Use an existing mesh'),
    ],
    ['catchment', t('汇水区网格', 'Catchment'), t('读取汇水区网格文件', 'Use a catchment mesh')],
  ];
  let outputDirectory = '';
  try {
    outputDirectory = `${caseDirectory(form.root, form.name)}/out`;
  } catch {
    /* A valid save location is required before this stage. */
  }
  const autoLabel = t('沿用算例默认', 'Use case defaults');
  const optionLabels: Record<string, string> = {
    DEF_dir_existing_srfdata: t('已有地表资料目录（可选）', 'Existing surface data (optional)'),
    'DEF_simulation_time%timestep': t('时间步长（秒）', 'Time step (seconds)'),
    'DEF_simulation_time%greenwich': t('使用 UTC 时间', 'Use UTC time'),
    'DEF_simulation_time%spinup_repeat': t('预热重复次数', 'Spinup repetitions'),
    DEF_HIST_FREQ: t('输出间隔', 'Output interval'),
    DEF_HIST_groupby: t('输出文件分组', 'Output file grouping'),
    DEF_WRST_FREQ: t('重启文件间隔', 'Restart interval'),
    DEF_dir_output: t('输出目录', 'Output folder'),
  };
  const fields: UiField[] = keys.map((k) => ({
    id: `new-${k}`,
    label: labels[k],
    value: form[k],
    type: 'text',
    disabled: !enabled,
    set: (v) => set(k, v),
  }));
  if (step === 1) {
    fields.push({
      id: 'new-kind',
      label: t('模拟范围', 'Domain'),
      value: form.kind,
      type: 'choice',
      options: domains.map(([value, label]) => ({ value, label })),
      disabled: !enabled,
      set: (v) => set('kind', v),
    });
    if (form.kind !== 'site')
      fields.push({
        id: 'new-grid',
        label: t('网格类型', 'Grid type'),
        value: form.grid,
        type: 'choice',
        options: grids.map(([value, label]) => ({ value, label })),
        disabled: !enabled,
        set: (v) => set('grid', v),
      });
  }
  if ([1, 3, 4].includes(step)) {
    fields.push({
      id: 'new-advanced',
      label: t('更多配置', 'More settings'),
      value: String(advanced),
      type: 'switch',
      disabled: !enabled,
      set: (v) => setAdvanced(v === 'true'),
    });
    fields.push(
      ...options.map((f) => ({
        id: `new-option-${f.name}`,
        label: optionLabels[f.name] || fieldLabel(f.name, state.language),
        value: caseValues[f.name] ?? '',
        type: f.kind === 'Logical' || f.values.length ? ('choice' as const) : ('text' as const),
        ...(f.kind === 'Logical' || f.values.length
          ? {
              options: [
                { value: '', label: autoLabel },
                ...(f.kind === 'Logical'
                  ? ['.true.', '.false.']
                  : optionState(f.name)?.allowed_values.length
                    ? optionState(f.name)!.allowed_values
                    : f.values
                ).map((value) => ({
                  value,
                  label:
                    f.kind === 'Logical'
                      ? t(value === '.true.' ? '开启' : '关闭', value === '.true.' ? 'On' : 'Off')
                      : optionLabel(f.name, value, state.language),
                })),
              ],
            }
          : {}),
        disabled: !enabled || optionDisabled(f.name),
        set: (v: string) => setOption(f.name, v),
      })),
    );
  }
  if (step === 4)
    fields.push({
      id: 'new-output-directory',
      label: t('输出位置', 'Output location'),
      value: outputDirectory,
      type: 'text',
      disabled: true,
      set: () => {
        throw new Error(t('输出目录由算例管理。', 'The case manages its output directory.'));
      },
    });
  const actions: UiAction[] = [
    ...dataGroups.map((group) => ({
      id: `new-data-group-${group.id}`,
      label: group.title,
      kind: 'click' as const,
      enabled,
      run: () => setDataOpened((previous) => ({ ...previous, [group.id]: !group.open })),
    })),
    ...simulationSteps.slice(0, step - 1).map((label, i) => ({
      id: `new-step-${i + 1}`,
      label: label[en ? 1 : 0],
      kind: 'click' as const,
      enabled,
      run: () => {
        setError('');
        setStep((i + 1) as SimulationStep);
      },
    })),
    ...keys
      .filter((k) => folderKeys.includes(k) || fileKeys.includes(k))
      .map((k) => ({
        id: `new-pick-${k}`,
        label: labels[k],
        kind: 'click' as const,
        enabled,
        run: () => pick(k),
      })),
    ...(step > 1
      ? [
          {
            id: 'new-previous',
            label: t('上一步', 'Back'),
            kind: 'click' as const,
            enabled,
            run: () => {
              setError('');
              setStep((step - 1) as SimulationStep);
            },
          },
        ]
      : []),
    ...(step < 5
      ? [
          {
            id: 'new-next',
            label: t('下一步', 'Next'),
            kind: 'click' as const,
            enabled: enabled && (step === 1 || schemaReady),
            run: next,
          },
        ]
      : [
          {
            id: 'new-create',
            label: t('创建算例', 'Create case'),
            kind: 'commit' as const,
            enabled: enabled && schemaReady && !!state.version,
            run: create,
          },
        ]),
    ...(step === 3 && form.kind === 'site'
      ? [
          {
            id: 'new-example',
            label: t('使用内置示例', 'Use bundled examples'),
            kind: 'commit' as const,
            enabled,
            run: example,
          },
          {
            id: 'new-own-data',
            label: t('使用自己的数据', 'Use your data'),
            kind: 'click' as const,
            enabled,
            run: () => setOwnData(true),
          },
          ...(ownData
            ? [
                {
                  id: 'new-scan',
                  label: t('读取站点与覆盖时段', 'Read sites and coverage'),
                  kind: 'click' as const,
                  enabled: enabled && !!form.sitedir,
                  run: () => work(() => scan()),
                },
              ]
            : []),
          ...sites.map((s) => ({
            id: `new-select-${s.name}`,
            label: s.name,
            kind: 'click' as const,
            enabled: enabled && !s.problem && !siteVersionMismatch(s, physics),
            run: () => selectSite(s.site_file),
          })),
        ]
      : []),
  ];
  usePageContract({
    title: t('新建模拟', 'New simulation'),
    step: 'new-case',
    fields,
    actions,
    status: busy ? 'busy' : error || 'ready',
    details: {
      step,
      model: state.versionRef,
      ...(step >= 2 ? { classification: form.landcover } : {}),
      ...(step >= 3 ? { period: { start: form.start, end: form.end } } : {}),
      ...(step === 5 ? { name: form.name, input_check: 'performed_when_creating' } : {}),
    },
  });
  const render = (k: string) => (
    <Field key={k} label={labels[k]}>
      <div className="browse-input">
        <Input
          value={form[k]}
          disabled={!enabled}
          placeholder={['start', 'end', 'spinup_end'].includes(k) ? 'YYYY-MM-DD' : undefined}
          maxLength={['start', 'end', 'spinup_end'].includes(k) ? 10 : undefined}
          onChange={(e) => set(k, e.target.value)}
        />
        {(folderKeys.includes(k) || fileKeys.includes(k)) && (
          <Button
            disabled={!enabled}
            aria-label={`${labels[k]} · ${t('选择', 'Browse')}`}
            onClick={() => void pick(k).catch(() => {})}
          >
            …
          </Button>
        )}
      </div>
    </Field>
  );
  const renderOption = (f: SchemaField) => (
    <Field
      key={f.name}
      label={optionLabels[f.name] || fieldLabel(f.name, state.language)}
      hint={optionState(f.name)?.reason ?? (advanced ? f.name : undefined)}
    >
      {f.kind === 'Logical' ? (
        <Select
          disabled={!enabled || optionDisabled(f.name)}
          value={caseValues[f.name] ?? ''}
          onChange={(e) => setOption(f.name, e.target.value)}
        >
          <option value="">{autoLabel}</option>
          <option value=".true.">{t('开启', 'On')}</option>
          <option value=".false.">{t('关闭', 'Off')}</option>
        </Select>
      ) : f.values.length ? (
        <Select
          value={caseValues[f.name] ?? ''}
          disabled={!enabled || optionDisabled(f.name)}
          onChange={(e) => setOption(f.name, e.target.value)}
        >
          <option value="">{autoLabel}</option>
          {(optionState(f.name)?.allowed_values.length
            ? optionState(f.name)!.allowed_values
            : f.values
          ).map((value) => (
            <option key={value} value={value}>
              {optionLabel(f.name, value, state.language)}
            </option>
          ))}
        </Select>
      ) : (
        <Input
          value={caseValues[f.name] ?? ''}
          placeholder={
            f.name === 'DEF_simulation_time%timestep'
              ? t('自动（1800 秒）', 'Automatic (1800 seconds)')
              : t('创建时自动配置', 'Set automatically at creation')
          }
          disabled={!enabled || optionDisabled(f.name)}
          onChange={(e) => setOption(f.name, e.target.value)}
        />
      )}
    </Field>
  );
  const timeOptions = basicOptions.filter((f) => f.name.startsWith('DEF_simulation_time%'));
  const outputOptions = ['DEF_HIST_FREQ', 'DEF_HIST_groupby', 'DEF_WRST_FREQ'].flatMap((name) =>
    basicOptions.filter((f) => f.name === name),
  );
  const extraOptions = options.filter((f) => !basicOptions.includes(f) && !groupFields.includes(f));
  const moreOptions = () => (
    <Advanced open={advanced} onOpenChange={setAdvanced} title={t('更多配置', 'More settings')}>
      {advanced && <div className="form-grid">{extraOptions.map(renderOption)}</div>}
    </Advanced>
  );
  return (
    <div className="page-stack simulation-page">
      <header className="simulation-heading">
        <div>
          <h1>{t('新建模拟', 'New simulation')}</h1>
          <p className="muted">
            {t(
              '先确定范围与模型方案，再准备数据、设置输出并运行。',
              'Choose the range and model structure first, then prepare data, output and run.',
            )}
          </p>
        </div>
        <span className="badge">
          {state.version?.name ?? t('读取模型…', 'Loading model…')} · r{state.versionRef.revision}
        </span>
      </header>
      <SimulationSteps
        step={step}
        en={en}
        disabled={!enabled}
        onChange={(value) => {
          setError('');
          setStep(value);
        }}
      />
      {error && <Notice tone="error">{error}</Notice>}
      {optionError && <Notice tone="warning">{optionError}</Notice>}
      {busy && <Notice>{t('正在处理，请稍候…', 'Working, please wait…')}</Notice>}
      {step === 1 && (
        <Card title={t('范围与网格', 'Range and grid')}>
          <section className="simulation-choice-group">
            <h3>{t('选择模拟范围', 'Choose a simulation range')}</h3>
            <div className="simulation-choices domain-choices">
              {domains.map(([value, label, description]) => (
                <Button
                  key={value}
                  variant="ghost"
                  className="simulation-choice-card"
                  aria-pressed={form.kind === value}
                  disabled={!enabled}
                  onClick={() => set('kind', value)}
                >
                  <strong>{label}</strong>
                  <span>{description}</span>
                </Button>
              ))}
            </div>
          </section>
          {form.kind !== 'site' && (
            <section className="simulation-choice-group">
              <h3>{t('选择网格类型', 'Choose the grid type')}</h3>
              <div className="simulation-choices">
                {grids.map(([value, label, description]) => (
                  <Button
                    key={value}
                    variant="ghost"
                    className="simulation-choice-card"
                    aria-pressed={form.grid === value}
                    disabled={!enabled}
                    onClick={() => set('grid', value)}
                  >
                    <strong>{label}</strong>
                    <span>{description}</span>
                  </Button>
                ))}
              </div>
            </section>
          )}
          <div className="simulation-name">{render('name')}</div>
          {rangeKeys.length > 0 && <div className="form-grid">{rangeKeys.map(render)}</div>}
          {moreOptions()}
        </Card>
      )}
      {step === 3 && (
        <Card
          title={t('数据与时段', 'Data and period')}
          description={t(
            '选择数据来源和模拟时间；初始条件与强迫处理在更多配置中。',
            'Choose data sources and dates; more settings include initial conditions and forcing options.',
          )}
        >
          {form.kind === 'site' && (
            <>
              <div className="actions simulation-source">
                <Button disabled={!enabled} onClick={() => void example().catch(() => {})}>
                  {t('使用内置示例', 'Use bundled examples')}
                </Button>
                <Button disabled={!enabled} onClick={() => setOwnData(true)}>
                  {t('使用自己的数据', 'Use your data')}
                </Button>
              </div>
              {ownData && (
                <div className="simulation-own-data">
                  <div className="form-grid">{['sitedir', 'forcingdir'].map(render)}</div>
                  <Button
                    disabled={!enabled || !form.sitedir}
                    onClick={() => void work(() => scan()).catch(() => {})}
                  >
                    {t('读取站点与覆盖时段', 'Read sites and coverage')}
                  </Button>
                </div>
              )}
              {hint && <Notice>{hint}</Notice>}
              {sites.length > 0 && (
                <div className="simulation-site-list">
                  {sites.map((s) => {
                    const issue = s.problem ?? siteVersionMismatch(s, physics);
                    return (
                      <Button
                        key={s.site_file}
                        className={s.site_file === form.site ? 'selected' : ''}
                        disabled={!enabled || !!issue}
                        title={issue ?? undefined}
                        onClick={() => selectSite(s.site_file)}
                      >
                        <strong>{s.name}</strong>
                        <span>{issue ?? `${s.lat.toFixed(2)}°, ${s.lon.toFixed(2)}°`}</span>
                      </Button>
                    );
                  })}
                </div>
              )}
              {scannedForcing(selected, form.met) && selected && (
                <p className="muted">
                  {t('已读取强迫场覆盖', 'Scanned forcing coverage')}：{selected.start} —{' '}
                  {selected.end}
                </p>
              )}
            </>
          )}
          <div className="simulation-dates">{['start', 'end'].map(render)}</div>
          <p className="field-hint">
            {t(
              '日期格式 YYYY-MM-DD，例如 2001-01-01。单点通常包含结束当天，到强迫场末尾时以实际覆盖截止。',
              'Use YYYY-MM-DD, e.g. 2001-01-01. Site runs normally include the end date, limited by forcing coverage.',
            )}
          </p>
          <div className="form-grid">
            {dataKeys.map(render)}
            {basicOptions.map(renderOption)}
          </div>
          <ProcessDataGroups
            groups={dataGroups}
            onToggle={(id, open) => setDataOpened((previous) => ({ ...previous, [id]: open }))}
            render={renderOption}
            disabled={!enabled}
          />
          {moreOptions()}
        </Card>
      )}
      {step === 2 && (
        <ModelSettings
          version={state.version}
          disabled={!enabled}
          classification={form.landcover as 'IGBP' | 'USGS'}
          mode={simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid)}
          onModeChange={selectMode}
          kernel={configurationKernel}
          setupEnvironment={{
            subgrid: simulationMode(undefined, form.landcover as 'IGBP' | 'USGS', form.subgrid),
            caseValues: dataPatch,
            domain: form.kind,
            grid: form.kind === 'site' ? null : form.grid,
            classification: form.landcover as 'IGBP' | 'USGS',
            kernel: configurationKernel,
            cropKernel: compatibleKernel(
              state.kernels,
              {
                grid: form.kind === 'site' ? null : form.grid,
                classification: form.landcover as 'IGBP' | 'USGS',
                crop: true,
              },
              physics,
            ),
          }}
          onCaseChange={changeCaseOptions}
          onClassificationChange={
            form.subgrid === 'PFT' || form.subgrid === 'PC'
              ? undefined
              : (value) => set('landcover', value)
          }
          onEdit={() => openResearch()}
        />
      )}
      {step === 4 && (
        <Card
          title={t('预热与输出', 'Spinup and output')}
          description={t(
            '设置时间步长、预热截止和输出频率；只把修改过的选项写入新算例。',
            'Set the time step, spinup cutoff and output frequency. Only edited options override the new case.',
          )}
        >
          <h3 className="simulation-subheading">{t('时间与预热', 'Time and spinup')}</h3>
          <div className="form-grid">
            {timeOptions.filter((f) => !f.name.endsWith('spinup_repeat')).map(renderOption)}
            {render('spinup_end')}
            {timeOptions.filter((f) => f.name.endsWith('spinup_repeat')).map(renderOption)}
          </div>
          <h3 className="simulation-subheading">{t('输出设置', 'Output settings')}</h3>
          <Field label={t('输出位置', 'Output location')}>
            <Input readOnly value={outputDirectory} />
          </Field>
          <div className="form-grid simulation-output-options">
            {outputOptions.map(renderOption)}
          </div>
          <p className="field-hint">
            {t(
              '预热在开始至截止日期之间重复，截止后进入正式模拟；留空不预热。输出变量在创建后按匹配内核选择。',
              'Spinup repeats the period before the cutoff; a blank cutoff disables it. Choose output variables for the matched kernel after creation.',
            )}
          </p>
          {moreOptions()}
        </Card>
      )}
      {step === 5 && (
        <Card title={t('检查并创建算例', 'Review and create the case')}>
          <div className="simulation-review">
            <dl>
              <dt>{t('模拟', 'Simulation')}</dt>
              <dd>{form.name}</dd>
              <dt>{t('范围', 'Range')}</dt>
              <dd>
                {domains.find(([value]) => value === form.kind)?.[1]}
                {form.kind !== 'site'
                  ? ` · ${grids.find(([value]) => value === form.grid)?.[1]}`
                  : ''}
              </dd>
              <dt>{t('时段', 'Period')}</dt>
              <dd>
                {form.start} — {form.end}
              </dd>
              <dt>{t('模型', 'Model')}</dt>
              <dd>
                {state.version?.name} · r{state.versionRef.revision}
              </dd>
              <dt>{t('预热截止', 'Spinup cutoff')}</dt>
              <dd>{form.spinup_end || t('不预热', 'Disabled')}</dd>
              <dt>{t('保存位置', 'Save location')}</dt>
              <dd className="path">
                {form.root}/{form.name}
              </dd>
            </dl>
            <div className="simulation-checks">
              <p className="check-fact">
                <span>✓</span>
                {t('必填设置已填写', 'Required settings filled')}
              </p>
              {scannedForcing(selected, form.met) && selected && (
                <p className="check-fact">
                  <span>✓</span>
                  {t('时段在已扫描覆盖内', 'Period within scanned coverage')}
                </p>
              )}
              <p className="check-pending">
                <span>○</span>
                {t('创建时检查实际输入与方案', 'Inputs and schemes checked when creating')}
              </p>
            </div>
          </div>
          <Notice>
            {t(
              '创建后进入运行页，由你选择何时开始计算。所有预热、输出与变量仍可在运行前返回调整。',
              'After creation, choose when to start in the Run page. Spinup, output and variables remain adjustable before running.',
            )}
          </Notice>
          <Advanced title={t('查看输入路径与修改选项', 'Inspect inputs and edited options')}>
            <dl className="simulation-paths">
              {dataKeys
                .filter((k) => k !== 'root')
                .map((k) => (
                  <div key={k}>
                    <dt>{labels[k]}</dt>
                    <dd className="path">{form[k] || t('未指定', 'Not specified')}</dd>
                  </div>
                ))}
              {editedOptions.map(([key, value]) => (
                <div key={key}>
                  <dt>{optionLabels[key] || key}</dt>
                  <dd>{value}</dd>
                </div>
              ))}
            </dl>
          </Advanced>
        </Card>
      )}
      <footer className="simulation-footer">
        {step > 1 ? (
          <Button
            disabled={!enabled}
            onClick={() => {
              setError('');
              setStep((step - 1) as SimulationStep);
            }}
          >
            {t('上一步', 'Back')}
          </Button>
        ) : (
          <span />
        )}
        {step < 5 ? (
          <Button
            variant="primary"
            disabled={!enabled || (step > 1 && !schemaReady)}
            onClick={() => {
              try {
                next();
              } catch {
                /* The inline error explains the failed check. */
              }
            }}
          >
            {t('下一步', 'Next')} →
          </Button>
        ) : (
          <Button
            variant="primary"
            disabled={!enabled || !schemaReady || !state.version}
            onClick={() => void create().catch(() => {})}
          >
            {t('创建算例', 'Create case')}
          </Button>
        )}
      </footer>
    </div>
  );
}
