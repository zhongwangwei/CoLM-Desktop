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
  newCaseGeneralProblem,
  scannedForcing,
  simulationFields,
  simulationSteps,
  spinupProblem,
  type SimulationStep,
} from './simulation-model';
import { fieldInputOptions, fieldLabel, optionLabel } from '../../lib/param-presentation';
import { caseFieldVisible, type ConfigurationField } from '../versions/configuration-options';
import { compatibleKernel } from '../runs/run-model';
import { simulationOptionField } from './model-settings';
import SimulationSteps from './SimulationSteps';
import ModelSettings from './ModelSettings';
import {
  autoClosedText,
  exclusionPatch,
  schemeFlagKeys,
  simulationMode,
  caseSchemeFields,
  type SimulationMode,
} from './model-settings';
import type { CaseOptionPatch } from '../versions/configuration-options';
import HistoryVariablesCard from './HistoryVariablesCard';

function siteInfo(s: Site, t: (zh: string, en: string) => string) {
  if (s.urban)
    return {
      badge: t('城市过程', 'Urban'),
      desc: t('城市冠层与微气象 (Urban-PLUMBER)', 'Urban canopy & microclimate (Urban-PLUMBER)'),
      adaptHint: t(
        '点击选择：自动开启城市过程并关闭 BGC',
        'Select: enables urban processes and disables BGC',
      ),
      activeHint: t('✓ 当前已选定 · 城市方案已就绪', '✓ Selected · Urban scheme active'),
    };
  if (s.crop)
    return {
      badge: t('农田作物', 'Crop'),
      desc: t('农田生态与生长发育 (FLUXNET-CROP)', 'Crop phenology & agriculture (FLUXNET-CROP)'),
      adaptHint: t(
        '点击选择：自动采用 PFT、BGC 与作物默认配置',
        'Select: uses PFT, BGC and the crop defaults',
      ),
      activeHint: t('✓ 当前已选定 · 作物方案已就绪', '✓ Selected · Crop scheme active'),
    };
  if (s.name === 'AT-Neu' || s.obs_file?.includes('CH4') || s.site_file?.includes('CH4'))
    return {
      badge: t('甲烷 / 示踪', 'Methane / Tracer'),
      desc: t('草地甲烷过程 (FLUXNET-CH4)', 'Grassland methane (FLUXNET-CH4)'),
      adaptHint: t('点击选择该站点', 'Click to select this station'),
      activeHint: t('✓ 当前已选定', '✓ Selected'),
    };
  if (s.name === 'CN-Cng')
    return {
      badge: t('自然草地', 'Grassland'),
      desc: t('常规水热能量基准 (FLUXNET2015)', 'Baseline water & heat (FLUXNET2015)'),
      adaptHint: t('点击选择该站点', 'Click to select this station'),
      activeHint: t('✓ 当前已选定', '✓ Selected'),
    };
  return {
    badge: t('自然植被', 'Natural'),
    desc: t('自然植被基准', 'Natural baseline'),
    adaptHint: t('点击选择该站点', 'Click to select this station'),
    activeHint: t('✓ 当前已选定', '✓ Selected'),
  };
}

export default function NewCasePage() {
  const { state, update, openCase, navigate } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const seed = state.newCaseSeed;
  const initialRoot = seed?.form?.root ?? state.root ?? '',
    initialName = seed?.form?.name ?? '';
  const initialLandcover = seed?.form?.landcover ?? '';
  const initialSubgrid = seed?.form?.subgrid ?? (seed?.form?.landcover ? seed.form.landcover : '');
  const [form, setForm] = useState<Record<string, string>>({
    // 计算核心、范围、网格与下垫面方案都必须由用户明确选择，不预选任何一项。
    engine: '',
    kernel: '',
    root: initialRoot,
    name: initialName,
    kind: '',
    landcover: initialLandcover,
    subgrid: initialSubgrid,
    sitedir: '',
    forcingdir: '',
    site: '',
    met: '',
    rawdata: '',
    runtime: '',
    start: '',
    end: '',
    spinup_end: '',
    grid: '',
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
    ...(['PFT', 'PC'].includes(seed?.form?.subgrid ?? '') ? { landcover: 'IGBP' } : {}),
  });
  const [caseValues, setCaseValues] = useState<Record<string, string>>(seed?.caseValues ?? {}),
    [schema, setSchema] = useState<SchemaField[]>([]),
    [schemaReady, setSchemaReady] = useState(false),
    [modelFields, setModelFields] = useState<SchemaField[]>([]);
  const [historyVariables, setHistoryVariables] = useState<
    Array<{ name: string; long_name?: string }>
  >([]);
  const [sites, setSites] = useState<Site[]>(seed?.sites ?? []),
    [step, setStep] = useState<SimulationStep>(seed?.step ?? 1),
    [hint, setHint] = useState(''),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [creationDir, setCreationDir] = useState<string | null>(null),
    [cancellingCreation, setCancellingCreation] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [ownData, setOwnData] = useState(seed?.ownData ?? false),
    // 自动关闭提示只属于触发它的那一步，离开后不再残留。
    [autoClosed, setAutoClosed] = useState<{ text: string; step: number }>({ text: '', step: 0 });
  const stepProblem = (at: SimulationStep) => {
    try {
      return problem(at);
    } catch (e) {
      return String(e);
    }
  };
  const [optionStates, setOptionStates] = useState<ConfigurationField[]>([]),
    [optionError, setOptionError] = useState(''),
    [cropDefaults, setCropDefaults] = useState<Record<string, string>>({});
  const creation = useRef<{ dir: string; cancelRequested: boolean } | null>(null),
    lock = useRef(false),
    sequence = useRef(0),
    alive = useRef(true),
    classification = (form.landcover || undefined) as 'IGBP' | 'USGS' | undefined,
    mode: SimulationMode | undefined = form.subgrid
      ? simulationMode(undefined, classification, form.subgrid)
      : undefined,
    schemeFields = mode ? caseSchemeFields(mode) : {},
    physics = {
      ...effectiveModelFields(state.version?.overlay, modelFields),
      ...caseValues,
      ...schemeFields,
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
        const hist = all
          .filter((f) => f.name.startsWith('DEF_hist_vars%'))
          .map((f) => ({
            name: f.name.replace('DEF_hist_vars%', ''),
            long_name: f.doc ?? undefined,
          }));
        setHistoryVariables(hist);
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
      classification: classification,
    },
    physics,
  );
  const dataPatch = {
    ...schemeFields,
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
    void invoke<{ fields: ConfigurationField[]; crop_defaults?: Record<string, string> }>(
      'model_configuration',
      JSON.parse(dataRequest),
    )
      .then((result) => {
        if (current) {
          setOptionStates(result.fields);
          setCropDefaults(result.crop_defaults ?? {});
        }
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
  // 上游选择改变后，把失去物理前提或与之互斥的已开启过程一并关闭，并告知用户。
  function reconcile(
    patch: CaseOptionPatch,
    context: { subgrid?: string; classification?: string; domain?: string; grid?: string } = {},
  ) {
    const result = exclusionPatch(physics, patch, {
      subgrid: context.subgrid ?? mode,
      classification: context.classification ?? classification,
      domain: context.domain ?? form.kind,
      grid: (context.domain ?? form.kind) === 'site' ? null : (context.grid ?? form.grid),
      compiledCrop: configurationKernel?.macros.includes('CROP') === true,
    });
    setAutoClosed({ text: autoClosedText(result.closed, en), step });
    setCaseValues((old) => {
      const next = { ...old };
      for (const [key, value] of Object.entries(result.patch)) {
        if (schemeFlagKeys.includes(key)) continue;
        if (value === null) delete next[key];
        else next[key] = value;
      }
      return next;
    });
  }
  const set = (k: string, v: string) => {
    setError('');
    if (k === 'kind' || k === 'grid') reconcile({}, k === 'kind' ? { domain: v } : { grid: v });
    const s = k === 'site' ? sites.find((site) => site.site_file === v) : undefined;
    setForm((f) => {
      const next: Record<string, string> = {
        ...f,
        [k]: v,
        ...(s
          ? { name: s.name, met: s.met_file ?? '', start: s.start ?? '', end: s.end ?? '' }
          : {}),
      };
      return next;
    });
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
      if (s) selectSite(s.site_file, found.sites);
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
  function selectSite(path: string, availableSites = sites) {
    const site = availableSites.find((item) => item.site_file === path);
    if (!site || site.problem) return;
    if (site.crop && !cropDefaults.DEF_TUNING_CROP_PLANTING_DAY) {
      setError(t('正在读取作物默认配置，请稍后选择。', 'Wait for crop defaults to load.'));
      return;
    }
    setError('');
    setForm((previous) => ({
      ...previous,
      site: path,
      name: site.name,
      met: site.met_file ?? '',
      start: site.start ?? '',
      end: site.end ?? '',
    }));
    setCaseValues((previous) => {
      const next: Record<string, string> = {
        ...previous,
        DEF_URBAN_RUN: site.urban ? '.true.' : '.false.',
      };
      if (site.crop) {
        for (const [key, value] of Object.entries(cropDefaults)) next[key] = previous[key] ?? value;
        Object.assign(next, {
          DEF_USE_BGC: '.true.',
          DEF_USE_LULCC: '.false.',
          DEF_URBAN_RUN: '.false.',
        });
      } else {
        delete next.DEF_TUNING_CROP_PLANTING_DAY;
        if (site.urban) Object.assign(next, { DEF_USE_BGC: '.false.', DEF_USE_TRACER: '.false.' });
      }
      return next;
    });
    if (site.crop || site.urban)
      setForm((previous) => ({
        ...previous,
        subgrid: site.crop ? 'PFT' : 'IGBP',
        landcover: 'IGBP',
      }));
  }
  // 作物站、城市站由站点数据决定过程结构（作物：PFT + BGC + 后端作物默认；城市：开启城市、关闭 BGC），
  // 普通站点只选站点，下垫面方案仍须在第四步明确选择。
  function adaptAndSelectSite(s: Site) {
    selectSite(s.site_file);
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
    if (at >= 1) {
      const issue = newCaseGeneralProblem(form, en);
      if (issue) return issue;
    }
    if (at >= 2) {
      if (!form.kind) return t('请选择模拟范围。', 'Choose a simulation range.');
      if (form.kind !== 'site' && !form.grid) return t('请选择网格类型。', 'Choose a grid type.');
      const issue = newCaseRangeProblem(form, en);
      if (issue) return issue;
    }
    if (at >= 3) {
      const issue = newCaseProblem(form, 3, selected, physics, en);
      if (issue) return issue;
    }
    if (at >= 4 && !mode)
      return t(
        '请选择一种下垫面 / 植被方案（IGBP、USGS、PFT 或 PC）。',
        'Choose a land-cover / vegetation scheme (IGBP, USGS, PFT or PC).',
      );
    if (at >= 4 && !['IGBP', 'USGS'].includes(form.landcover))
      return t('请选择 IGBP 或 USGS 地表分类。', 'Choose IGBP or USGS land-cover classification.');
    if (at >= 5) {
      return (
        newCaseProblem(form, 5, selected, physics, en) ||
        spinupProblem(
          form.start,
          form.end,
          form.spinup_end,
          caseValues['DEF_simulation_time%spinup_repeat'] ?? '1',
          en,
        )
      );
    }
    return null;
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
          ...schemeFields,
        },
        step: 4,
        ownData,
      },
      simulationReturn: { route: 'new-case', step: 4 },
    });
    return navigate('research');
  }
  function changeCaseOptions(patch: CaseOptionPatch) {
    setError('');
    reconcile(patch);
  }
  function selectMode(next: SimulationMode) {
    const landcover = next === 'USGS' ? 'USGS' : 'IGBP';
    setForm((f) => ({ ...f, subgrid: next, landcover }));
    reconcile(caseSchemeFields(next), { subgrid: next, classification: landcover });
    setError('');
  }

  // 输出变量开关不在通用 schema 里，但同样要随新算例写出，否则在第五步选的变量会被丢掉。
  const editedOptions = Object.entries(caseValues).filter(
    ([name, value]) =>
      name !== 'DEF_dir_output' &&
      value !== '' &&
      (schema.some((f) => f.name === name) || name.startsWith('DEF_hist_vars%')),
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
            // 预热以日期为界（当日 00:00），不再暴露含义易混的“预热截止秒”。
            'DEF_simulation_time%spinup_sec': '0',
          }
        : { 'DEF_simulation_time%spinup_year': '0' };
      const fields = [
          ...physicsFields(effectiveModelFields(state.version?.overlay, modelFields)),
          ...physicsFields(schemeFields),
          ...changed,
          ...Object.entries(spinup).map(([path, value]) => ({ path, value })),
        ],
        mode = versionMode(physics, classification);
      const job = { dir: out, cancelRequested: false };
      creation.current = job;
      setCreationDir(out);
      setCancellingCreation(false);
      try {
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
            engine: form.engine,
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
            engine: form.engine,
          });
        }
      } finally {
        if (creation.current === job) {
          creation.current = null;
          if (alive.current) {
            setCreationDir(null);
            setCancellingCreation(false);
          }
        }
      }
      update({ root: form.root, versionRef: revision });
      lock.current = false;
      return await openCase(out);
    });
  }
  async function cancelCreation() {
    const job = creation.current;
    if (!alive.current || !job || job.cancelRequested)
      throw new Error(t('当前没有可取消的创建任务。', 'No creation can be cancelled now.'));
    job.cancelRequested = true;
    setCancellingCreation(true);
    try {
      const cancelled = await invoke<number>('case_creation_cancel', { dir: job.dir });
      if (!cancelled && creation.current === job)
        throw new Error(
          t(
            '尚未取消创建，请等待当前任务结束或重试取消。',
            'Creation was not cancelled. Wait for it to finish or retry cancellation.',
          ),
        );
    } catch (e) {
      if (alive.current && creation.current === job) {
        job.cancelRequested = false;
        setCancellingCreation(false);
        setError(String(e));
      }
      throw e;
    }
  }
  const labels: Record<string, string> = {
    engine: t('计算核心', 'Computation core'),
    kernel: t('内核预设', 'Kernel preset'),
    root: t('保存到项目目录', 'Project folder'),
    output: t('模拟输出路径', 'Simulation output path'),
    name: t('模拟名称', 'Simulation name'),
    sitedir: t('站点数据目录', 'Site data folder'),
    forcingdir: t('强迫场目录', 'Forcing folder'),
    site: t('站点文件', 'Site file'),
    met: t('强迫场文件', 'Forcing file'),
    rawdata: t('地表数据目录', 'Surface data folder'),
    runtime: t('Runtime 数据目录', 'Runtime data folder'),
    start: t('开始日期', 'Start date'),
    end: t('结束日期', 'End date'),
    spinup_end: t('预热结束日期（留空不预热）', 'Spin-up end (blank disables spin-up)'),
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
    ...(form.kind && form.kind !== 'site' && form.grid
      ? form.grid === 'latlon'
        ? ['dlon', 'dlat']
        : form.grid === 'catchment'
          ? ['catchment']
          : ['mesh']
      : []),
    ...(form.kind === 'watershed' && form.grid === 'latlon' ? ['shapefile'] : []),
  ];
  const dataKeys =
    form.kind === 'site'
      ? ['site', 'met', 'rawdata', 'runtime']
      : ['rawdata', 'runtime', 'forcing'];
  const keys =
    step === 1
      ? ['root', 'name']
      : step === 2
        ? [...rangeKeys]
        : step === 3
          ? [
              ...dataKeys,
              'start',
              'end',
              ...(form.kind === 'site' && ownData ? ['sitedir', 'forcingdir'] : []),
            ]
          : step === 5
            ? ['spinup_end']
            : [];
  const visibleSchema = schema.filter(
    (field) =>
      caseFieldVisible(field.name, optionStates, caseValues) &&
      // 单点只写单文件历史（'one'）；建例前后端还拿不到 SinglePoint 上下文去隐藏它，这里先挡住。
      !(form.kind === 'site' && field.name === 'DEF_HIST_mode'),
  );
  const enabled = !busy && !state.busy;
  // 当前步骤的必填项没齐全时，“下一步/创建”置灰，并直接说明还差什么。
  const blocker = stepProblem(step);
  const groupFields = schema.filter((field) => simulationOptionField(field, optionStates)),
    options = [
      ...simulationFields(visibleSchema, step, advanced, form.engine).filter(
        (f) => !groupFields.includes(f),
      ),
    ],
    basicOptions = simulationFields(visibleSchema, step, false, form.engine).filter(
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
  const autoLabel = t('沿用算例默认', 'Use case defaults');
  const optionLabels: Record<string, string> = {
    DEF_dir_existing_srfdata: t('已有地表资料目录（可选）', 'Existing surface data (optional)'),
    'DEF_simulation_time%timestep': t('时间步长（秒）', 'Time step (seconds)'),
    'DEF_simulation_time%greenwich': t('使用 UTC 时间', 'Use UTC time'),
    'DEF_simulation_time%spinup_repeat': t('预热循环重复次数', 'Spin-up repetitions'),
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
    fields.unshift({
      id: 'new-engine',
      label: labels.engine,
      value: form.engine,
      type: 'choice',
      options: [
        { value: 'rust', label: t('Rust 引擎', 'Rust engine') },
        { value: 'fortran', label: t('Fortran 官方内核', 'Fortran official kernel') },
      ],
      disabled: !enabled,
      set: (v) => set('engine', v),
    });
    if (form.engine === 'fortran') {
      fields.push({
        id: 'new-kernel',
        label: labels.kernel,
        value: form.kernel,
        type: 'choice',
        options: [
          { value: '', label: t('自动匹配', 'Auto-matched') },
          ...state.kernels.map((k) => ({ value: k.dir, label: k.preset })),
        ],
        disabled: !enabled,
        set: (v) => set('kernel', v),
      });
    }
  }
  if (step === 2) {
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
  if ([2, 3, 5].includes(step)) {
    if (step !== 2 || form.engine === 'fortran') {
      fields.push({
        id: 'new-advanced',
        label: t('更多配置', 'More settings'),
        value: String(advanced),
        type: 'switch',
        disabled: !enabled,
        set: (v) => setAdvanced(v === 'true'),
      });
      fields.push(
        // 重复次数只在开启预热后出现在预热框里，契约字段与之保持一致。
        ...options
          .filter((f) => !f.name.endsWith('spinup_repeat') || !!form.spinup_end)
          .map((f) => ({
            id: `new-option-${f.name}`,
            label: optionLabels[f.name] || fieldLabel(f.name, state.language),
            value: caseValues[f.name] ?? '',
            type:
              f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
                ? ('choice' as const)
                : ('text' as const),
            ...(f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
              ? {
                  options: [
                    { value: '', label: autoLabel },
                    ...(f.kind === 'Logical'
                      ? ['.true.', '.false.']
                      : optionState(f.name)?.allowed_values.length
                        ? optionState(f.name)!.allowed_values
                        : f.values
                    ).map((value: string) => ({
                      value,
                      label:
                        f.kind === 'Logical'
                          ? t(
                              value === '.true.' ? '开启' : '关闭',
                              value === '.true.' ? 'On' : 'Off',
                            )
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
  }

  const actions: UiAction[] = [
    ...(creationDir
      ? [
          {
            id: 'new-cancel-creation',
            label: t('取消创建', 'Cancel creation'),
            kind: 'click' as const,
            enabled: !cancellingCreation,
            run: cancelCreation,
          },
        ]
      : []),
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
            enabled: enabled && (step === 1 || schemaReady) && !blocker,
            run: next,
          },
        ]
      : [
          {
            id: 'new-create',
            label: t('创建算例', 'Create case'),
            kind: 'commit' as const,
            enabled: enabled && schemaReady && !!state.version && !blocker,
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
            enabled: enabled && !s.problem,
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
      ...(blocker ? { blocker } : {}),
      engine: form.engine,
      ...(step >= 2
        ? { domain: form.kind, ...(form.kind !== 'site' ? { grid: form.grid } : {}) }
        : {}),
      ...(step >= 3 ? { period: { start: form.start, end: form.end } } : {}),
      ...(step >= 4 ? { classification: form.landcover } : {}),
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
      ) : fieldInputOptions(f.name, f.kind, f.values).length ? (
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
  const timeOptions = basicOptions.filter(
    (f) => f.name.startsWith('DEF_simulation_time%') && f.name !== 'DEF_simulation_time%spinup_sec',
  );
  const outputOptions = ['DEF_HIST_FREQ', 'DEF_WRST_FREQ'].flatMap((name) =>
    basicOptions.filter((f) => f.name === name),
  );
  const extraOptions = options.filter(
    (f) =>
      !basicOptions.includes(f) &&
      !groupFields.includes(f) &&
      !f.name.startsWith('DEF_hist_vars%') &&
      f.name !== 'DEF_HIST_vars_out_default' &&
      f.name !== 'DEF_HIST_vars_namelist',
  );
  const moreOptions = () => (
    <Advanced open={advanced} onOpenChange={setAdvanced} title={t('更多配置', 'More settings')}>
      {advanced && (
        <>
          <div className="form-grid">{extraOptions.map(renderOption)}</div>
        </>
      )}
    </Advanced>
  );

  return (
    <div className="page-stack simulation-page">
      <header className="simulation-heading">
        <div>
          <h1>{t('新建模拟', 'New simulation')}</h1>
          <p className="muted">
            {t(
              '先确定总设定与范围，再配置方案、准备数据并创建。',
              'Choose general settings and range first, then configure model, data, and create.',
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
      {autoClosed.text && autoClosed.step === step && (
        <Notice tone="warning">{autoClosed.text}</Notice>
      )}
      {optionError && <Notice tone="warning">{optionError}</Notice>}
      {busy && (
        <Notice>
          {cancellingCreation
            ? t('正在取消创建，请等待清理完成…', 'Cancelling creation; waiting for cleanup…')
            : t('正在处理，请稍候…', 'Working, please wait…')}
        </Notice>
      )}
      {creationDir && (
        <Button disabled={cancellingCreation} onClick={() => void cancelCreation().catch(() => {})}>
          {t('取消创建', 'Cancel creation')}
        </Button>
      )}
      {step === 1 && (
        <Card
          title={t('总的设定', 'General setup')}
          description={t(
            '选择计算核心，并设置保存项目目录与模拟名称。',
            'Choose computation core, project folder, and simulation name.',
          )}
        >
          <section className="simulation-choice-group">
            <h3>{t('计算核心', 'Computation core')}</h3>
            <div className="simulation-choices engine-choices">
              <Button
                variant="ghost"
                className="simulation-choice-card"
                aria-pressed={form.engine === 'rust'}
                disabled={!enabled}
                onClick={() => set('engine', 'rust')}
              >
                <strong>{t('Rust 引擎（推荐）', 'Rust engine (recommended)')}</strong>
                <span>
                  {t(
                    '现代单进程多线程并发，免 Fortran 编译与依赖',
                    'Modern multithreaded engine, zero Fortran compilation required',
                  )}
                </span>
              </Button>
              <Button
                variant="ghost"
                className="simulation-choice-card"
                aria-pressed={form.engine === 'fortran'}
                disabled={!enabled}
                onClick={() => set('engine', 'fortran')}
              >
                <strong>{t('Fortran 官方内核', 'Fortran official kernel')}</strong>
                <span>
                  {t(
                    '传统科学基线，需匹配已编译的内核预设',
                    'Official scientific baseline, requires compiled kernel preset',
                  )}
                </span>
              </Button>
            </div>
          </section>
          {form.engine === 'fortran' && (
            <Field
              label={t('匹配内核预设', 'Matched kernel preset')}
              hint={
                configurationKernel?.preset
                  ? `preset: ${configurationKernel.preset}`
                  : t(
                      '未找到匹配预设，可在版本管理中编译',
                      'No matching preset, compile in Model Versions',
                    )
              }
            >
              <Select
                disabled={!enabled || !state.kernels.length}
                value={form.kernel || configurationKernel?.dir || ''}
                onChange={(e) => set('kernel', e.target.value)}
              >
                <option value="">
                  {configurationKernel
                    ? `${configurationKernel.preset} (${t('自动匹配', 'Auto-matched')})`
                    : t('请选择内核预设', 'Select kernel preset')}
                </option>
                {state.kernels.map((k) => (
                  <option key={k.dir} value={k.dir}>
                    {k.preset}
                  </option>
                ))}
              </Select>
            </Field>
          )}
          <div className="form-grid">{['root', 'name'].map(render)}</div>
          <p className="muted">
            {t(
              '算例配置与模拟结果将统一保存在该算例目录下（输出结果存放在 out/ 子目录）。',
              'Case configurations and simulation outputs will be saved together in this case directory (outputs in the out/ subdirectory).',
            )}
          </p>
        </Card>
      )}
      {step === 2 && (
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
          {rangeKeys.length > 0 && <div className="form-grid">{rangeKeys.map(render)}</div>}
          {form.engine === 'fortran' && moreOptions()}
        </Card>
      )}
      {step === 3 && (
        <Card
          title={t('数据与时段', 'Data and period')}
          description={t(
            '选择数据来源和模拟起止时段。',
            'Choose data sources and simulation period.',
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
                    const info = siteInfo(s, t);
                    const issue = s.problem;
                    const isSelected = s.site_file === form.site;
                    return (
                      <div
                        key={s.site_file}
                        style={{ display: 'flex', flexDirection: 'column', gap: '2px' }}
                      >
                        <Button
                          className={isSelected ? 'selected' : ''}
                          disabled={!enabled || Boolean(s.problem)}
                          title={issue ?? undefined}
                          onClick={() => adaptAndSelectSite(s)}
                        >
                          <div
                            style={{
                              display: 'flex',
                              justifyContent: 'space-between',
                              alignItems: 'center',
                              width: '100%',
                            }}
                          >
                            <strong>{s.name}</strong>
                            <span
                              className="badge"
                              style={{
                                fontSize: '10px',
                                background: 'var(--accent-soft)',
                                color: 'var(--accent)',
                              }}
                            >
                              {info.badge}
                            </span>
                          </div>
                          <span style={{ fontSize: '11px' }}>
                            {issue ?? `${info.desc} · ${s.lat.toFixed(2)}°, ${s.lon.toFixed(2)}°`}
                          </span>
                        </Button>
                        {!s.problem &&
                          (isSelected ? (
                            <span
                              style={{
                                color: 'var(--accent)',
                                fontSize: '11px',
                                padding: '0 4px',
                                fontWeight: 500,
                              }}
                            >
                              {info.activeHint}
                            </span>
                          ) : (
                            <button
                              type="button"
                              style={{
                                color: 'var(--accent)',
                                background: 'none',
                                border: 'none',
                                textDecoration: 'underline',
                                cursor: 'pointer',
                                fontSize: '11px',
                                textAlign: 'left',
                                padding: '0 4px',
                              }}
                              onClick={() => adaptAndSelectSite(s)}
                            >
                              {info.adaptHint}
                            </button>
                          ))}
                      </div>
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
          <div
            className="simulation-spinup-box"
            style={{
              marginTop: '1.25rem',
              padding: '1rem',
              borderRadius: 'var(--r-md)',
              background: 'var(--surface)',
              border: '1px solid var(--border)',
            }}
          >
            <div
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                alignItems: 'center',
                flexWrap: 'wrap',
                gap: '8px',
              }}
            >
              <div>
                <strong style={{ fontSize: '14px' }}>
                  {t('模型预热（Spin-up）设定', 'Model Spin-up Settings')}
                </strong>
                <p className="muted" style={{ margin: '2px 0 0', fontSize: '12px' }}>
                  {t(
                    '让土壤温湿与碳库等慢变量先趋于平衡；预热期不写输出，正式输出从预热结束后开始。',
                    'Let soil moisture, temperature and carbon pools reach equilibrium; outputs begin after spin-up.',
                  )}
                </p>
              </div>
              <Button
                variant={form.spinup_end ? 'primary' : 'secondary'}
                style={{ fontSize: '12px', padding: '4px 10px' }}
                disabled={!enabled}
                onClick={() => {
                  if (form.spinup_end) {
                    set('spinup_end', '');
                    setOption('DEF_simulation_time%spinup_repeat', '');
                  } else {
                    const startYear = form.start ? Number(form.start.slice(0, 4)) : 0;
                    const endYear = form.end ? Number(form.end.slice(0, 4)) : 0;
                    // 预热在结束日当天 00:00 截止，默认取开始后一整年（次年 1 月 1 日）。
                    const defaultCutoff =
                      startYear && endYear && endYear > startYear
                        ? `${startYear + 1}-01-01`
                        : form.start
                          ? `${form.start.slice(0, 7)}-15`
                          : '';
                    set('spinup_end', defaultCutoff);
                    setOption(
                      'DEF_simulation_time%spinup_repeat',
                      caseValues['DEF_simulation_time%spinup_repeat'] || '1',
                    );
                  }
                }}
              >
                {form.spinup_end
                  ? t('✓ 已开启预热（点击关闭）', '✓ Spin-up Enabled (Click to disable)')
                  : t('＋ 开启预热（Spin-up）', '＋ Enable Spin-up')}
              </Button>
            </div>
            {Boolean(form.spinup_end) && (
              <div className="form-grid" style={{ marginTop: '12px' }}>
                <Field
                  label={t('预热起始日期', 'Spin-up start')}
                  hint={t(
                    'CoLM 预热总是从模拟开始日期起算',
                    'CoLM spin-up always starts at the simulation start',
                  )}
                >
                  <Input readOnly value={form.start} />
                </Field>
                {render('spinup_end')}
                <Field
                  label={t('预热循环重复次数', 'Spin-up repetitions')}
                  hint={t(
                    '从起始日期跑到结束日期（当日 00:00 截止）后重置时间，重复这一段的次数（通常 1~5）',
                    'Times the start-to-end period (ending 00:00 that day) is repeated (normally 1–5)',
                  )}
                >
                  <Input
                    type="number"
                    min={1}
                    max={99}
                    step={1}
                    disabled={!enabled}
                    value={caseValues['DEF_simulation_time%spinup_repeat'] || '1'}
                    onChange={(e) => setOption('DEF_simulation_time%spinup_repeat', e.target.value)}
                  />
                </Field>
              </div>
            )}
          </div>
          <div className="form-grid" style={{ marginTop: '1rem' }}>
            {dataKeys.map(render)}
            {basicOptions.filter((f) => !f.name.endsWith('spinup_repeat')).map(renderOption)}
          </div>

          {moreOptions()}
        </Card>
      )}
      {step === 4 && (
        <ModelSettings
          version={state.version}
          disabled={!enabled}
          classification={classification}
          mode={mode}
          onModeChange={selectMode}
          kernel={configurationKernel}
          setupEnvironment={{
            subgrid: mode,
            caseValues: dataPatch,
            domain: form.kind,
            grid: form.kind === 'site' ? null : form.grid,
            classification: classification,
            kernel: configurationKernel,
            cropKernel: compatibleKernel(
              state.kernels,
              {
                grid: form.kind === 'site' ? null : form.grid,
                classification: classification,
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
      {step === 5 && (
        <>
          <Card
            title={t('检查并创建算例', 'Review and create the case')}
            description={t(
              '设置时间步长、预热与输出频率，核对设置后创建算例。',
              'Configure time step, spinup and output frequency, review settings, and create case.',
            )}
          >
            <h3 className="simulation-subheading">{t('时间步长与输出', 'Time step and output')}</h3>
            <div className="form-grid">
              {timeOptions
                .filter((f) => f.name === 'DEF_simulation_time%timestep')
                .map(renderOption)}
            </div>
            <h3 className="simulation-subheading">{t('输出设置', 'Output settings')}</h3>
            <div className="form-grid simulation-output-options">
              {outputOptions.map(renderOption)}
            </div>
            <div className="simulation-review" style={{ marginTop: '1.5rem' }}>
              <dl>
                <dt>{t('模拟', 'Simulation')}</dt>
                <dd>{form.name}</dd>
                <dt>{t('计算核心', 'Computation core')}</dt>
                <dd>
                  {form.engine === 'rust'
                    ? t('Rust 引擎', 'Rust engine')
                    : t('Fortran 官方内核', 'Fortran official kernel')}
                </dd>
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
                <dt>{t('下垫面方案', 'Land-cover scheme')}</dt>
                <dd>
                  {mode ?? '—'}
                  {mode === 'PFT' || mode === 'PC' ? ` · ${form.landcover}` : ''}
                </dd>
                <dt>{t('预热', 'Spin-up')}</dt>
                <dd>
                  {form.spinup_end
                    ? `${form.start} — ${form.spinup_end} × ${caseValues['DEF_simulation_time%spinup_repeat'] || '1'}`
                    : t('不预热', 'Disabled')}
                </dd>
                <dt>{t('保存位置', 'Save location')}</dt>
                <dd className="path">
                  {form.root}/{form.name} ({t('输出在 out/', 'outputs in out/')})
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
                {dataKeys.map((k) => (
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
            {moreOptions()}
          </Card>

          <HistoryVariablesCard
            variables={historyVariables.map((v) => ({
              name: v.name,
              settable: true,
              longName: v.long_name,
            }))}
            values={caseValues}
            defaultOn
            disabled={!enabled}
            en={en}
            onChange={(patch) => setCaseValues((old) => ({ ...old, ...patch }))}
          />
        </>
      )}
      {blocker && !error && (
        <p className="field-hint simulation-blocker" role="status">
          {blocker}
        </p>
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
            disabled={!enabled || (step > 1 && !schemaReady) || !!blocker}
            title={blocker ?? undefined}
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
            disabled={!enabled || !schemaReady || !state.version || !!blocker}
            title={blocker ?? undefined}
            onClick={() => void create().catch(() => {})}
          >
            {t('创建算例', 'Create case')}
          </Button>
        )}
      </footer>
    </div>
  );
}
