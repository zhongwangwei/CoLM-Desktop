import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { ActionButton } from '../../components/ActionButton';
import { usePageContract, type UiField } from '../../lib/guide';
import { Card, Input, DateInput, Select, Field, Notice, Advanced } from '../../components/ui';
import {
  isCaseField,
  displayLiteral,
  effectiveModelFields,
  selectChoice,
  type SchemaField,
} from './case-model';
import { compatibleKernel } from '../runs/run-model';
import {
  caseDate,
  caseDatePatch,
  casePeriodValid,
  caseSpinupValid,
  casePeriodText,
  simulationFields,
  isSiteConfiguration,
  caseRangeKeys,
  validDate,
  type SimulationStep,
} from './simulation-model';
import { fieldInputOptions, fieldLabel, optionLabel } from '../../lib/param-presentation';
import { caseFieldVisible, type ConfigurationField } from '../versions/configuration-options';
import {
  autoClosedText,
  configurationReason,
  exclusionPatch,
  simulationOptionField,
  caseSimulationDomain,
} from './model-settings';
import HistoryVariablesCard from './HistoryVariablesCard';
import { historyKey, type HistoryVariable } from './history-variables';
import SimulationSteps from './SimulationSteps';
import ModelSettings from './ModelSettings';
import { caseSchemeFields, type SimulationMode } from './model-settings';
import type { CaseOptionPatch } from '../versions/configuration-options';
import RunPanel from '../runs/RunPanel';
import RunLocationSettings from '../runs/RunLocationSettings';
import type { Server } from '../resources/resource';
import ResultsPage from '../results/ResultsPage';
import HistoryPage from '../history/HistoryPage';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import SpatialForcingFields, { type ForcingLayoutProbe } from './SpatialForcingFields';
type ForcingInput = { directory: string; dataset: string };
export default function CasesPage() {
  const { state, update, navigate, notify } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh),
    dir = state.selectedCase?.entry.dir;
  const [step, setStep] = useState<SimulationStep>(
      state.simulationReturn?.route === 'case'
        ? ((state.simulationReturn.step as SimulationStep) ?? 4)
        : 5,
    ),
    [error, setError] = useState('');
  const [runPending, setRunPending] = useState(false);
  const [servers, setServers] = useState<Server[] | null>(null);
  const [resourceError, setResourceError] = useState('');
  const [forcingInput, setForcingInput] = useState<ForcingInput | null>(null);
  const [savedForcing, setSavedForcing] = useState<ForcingInput | null>(null);
  const [forcingTemplate, setForcingTemplate] = useState('');
  const [forcingDatasets, setForcingDatasets] = useState<string[]>([]);
  const [forcingInputError, setForcingInputError] = useState('');
  const [forcingProbe, setForcingProbe] = useState<ForcingLayoutProbe | null>(null);
  const [forcingPicker, setForcingPicker] = useState(false);
  // 单点算例的站点文件带了哪几套分类（第 4 步的 IGBP/USGS 卡片用）；读不到就不限制。
  const [siteClasses, setSiteClasses] = useState<{
    dir: string;
    value: { igbp: boolean; usgs: boolean };
  } | null>(null);
  const forcingSequence = useRef(0);
  useEffect(() => {
    let current = true;
    invoke<{ servers: Server[] }>('remote_config', {})
      .then((config) => {
        if (current) setServers(config.servers);
      })
      .catch((e) => {
        if (current) {
          setServers([]);
          setResourceError(String(e));
        }
      });
    return () => {
      current = false;
    };
  }, []);
  const runHost = state.selectedCase?.profile.run_host ?? '';
  const remoteDirectory = state.selectedCase?.profile.remote_directory;
  const forcingPathHost = remoteDirectory ? runHost : '';
  async function changeRunHost(host: string) {
    if (remoteDirectory && host !== runHost) return;
    if (!dir || editingDisabled || dirty || lock.current) return;
    const target = dir;
    await work(async () => {
      await invoke('set_case_run_host', { dir: target, host: host || null });
      ++forcingSequence.current;
      setForcingProbe(null);
      update((old) =>
        old.selectedCase?.entry.dir === target
          ? {
              selectedCase: {
                ...old.selectedCase,
                profile: { ...old.selectedCase.profile, run_host: host || null },
              },
            }
          : {},
      );
    });
  }
  function openResources() {
    if (editingDisabled || dirty) return;
    update({ simulationReturn: { route: 'case', step: 1, resourceHost: runHost } });
    navigate('resources');
  }
  function backToSimulation() {
    if (state.projectView?.root === state.root && state.projectView?.page === 'batch') {
      if (navigate('project')) update({ projectView: { ...state.projectView, home: false } });
    } else update({ caseTab: 'settings' });
  }
  const [dateDraft, setDateDraft] = useState<Partial<Record<'start' | 'end' | 'spinup', string>>>(
    {},
  );
  const [contextReason, setContextReason] = useState<Record<string, string>>({}),
    [disabledFields, setDisabledFields] = useState<string[]>([]),
    [historyVars, setHistoryVars] = useState<HistoryVariable[]>([]),
    [schema, setSchema] = useState<SchemaField[]>([]),
    [values, setValues] = useState<Record<string, string>>({}),
    [saved, setSaved] = useState<Record<string, string>>({}),
    [busy, setBusy] = useState(false),
    [advanced, setAdvanced] = useState(state.expert);
  const [optionStates, setOptionStates] = useState<ConfigurationField[]>([]),
    [optionError, setOptionError] = useState(''),
    [explicitValues, setExplicitValues] = useState<Record<string, string>>({}),
    [modelKeys, setModelKeys] = useState<SchemaField[]>([]);
  const epoch = useRef({ active: false }),
    lock = useRef(false),
    versionId = state.versionRef.id,
    revision = state.versionRef.revision,
    overlay = state.version?.overlay;
  const physics = { ...effectiveModelFields(overlay, modelKeys), ...explicitValues };
  const [modelContext, setModelContext] = useState<{
    dir: string;
    id: string;
    revision: number;
    profile: {
      grid?: string;
      domain_kind?: string | null;
      domain?: number[] | null;
      classification?: 'IGBP' | 'USGS';
      subgrid?: string;
      soil?: string;
      bgc?: boolean;
      urban?: boolean;
      crop?: boolean;
    };
  } | null>(null);
  const effectiveProfile =
    modelContext &&
    modelContext.dir === dir &&
    modelContext.id === versionId &&
    modelContext.revision === revision
      ? modelContext.profile
      : null;
  const forcingDirty =
    !!forcingInput && (forcingInput.directory !== savedForcing?.directory || !!forcingTemplate);
  const dirty =
    forcingDirty ||
    Object.keys(values).some((k) => values[k] !== saved[k]) ||
    (['start', 'end', 'spinup'] as const).some(
      (prefix) => dateDraft[prefix] !== undefined && dateDraft[prefix] !== caseDate(saved, prefix),
    );
  useNavigationGuard(() =>
    dirty
      ? t(
          '请先保存算例设置，或放弃本地修改后再离开。',
          'Save case settings or discard local edits before leaving.',
        )
      : null,
  );
  useEffect(() => {
    update({ simulationReturn: null });
  }, [update]);
  async function changeCaseOptions(requested: CaseOptionPatch) {
    if (!dir || editingDisabled || dirty || lock.current)
      throw new Error(t('请先保存或放弃当前修改。', 'Save or discard current edits first.'));
    // 上游选择否定了已开启过程的物理前提时，同一次保存里一并关闭它。
    const { patch, closed } = exclusionPatch({ ...physics, ...values }, requested, {
      subgrid: effectiveProfile?.subgrid,
      classification: effectiveProfile?.classification,
      domain: caseSimulationDomain(effectiveProfile),
      grid: effectiveProfile?.grid ?? null,
      compiledCrop: configurationKernel?.macros.includes('CROP') === true,
    });
    await work(async () => {
      const target = dir,
        request = epoch.current;
      await invoke('case_data_save', {
        dirs: [target],
        fields: Object.entries(patch)
          .filter(([, value]) => value !== null)
          .map(([path, value]) => ({ path, value })),
        removeFields: Object.keys(patch).filter((key) => patch[key] === null),
        modelRevision: state.versionRef,
        kernelDir: configurationKernel?.dir ?? null,
      });
      const context = await invoke<{
        entry: { dir: string };
        profile: NonNullable<typeof modelContext>['profile'];
      }>('model_case_context', { dir: target, modelRevision: state.versionRef });
      if (request.active && epoch.current === request) {
        setModelContext({ dir: target, id: versionId, revision, profile: context.profile });
        if (state.selectedCase?.entry.dir === target)
          update({
            selectedCase: {
              ...state.selectedCase,
              profile: { ...state.selectedCase.profile, ...context.profile },
            },
          });
        const apply = (old: Record<string, string>) => {
          const next = { ...old };
          for (const [key, value] of Object.entries(patch)) {
            if (value === null) delete next[key];
            else next[key] = value;
          }
          return next;
        };
        setValues(apply);
        setSaved(apply);
        setExplicitValues(apply);
        await refreshHistory(context, request);
        if (!request.active || epoch.current !== request) return;
        setError('');
        if (closed.length) notify(autoClosedText(closed, en));
      }
    });
  }
  async function refreshHistory(
    context: { entry: { dir: string }; profile: NonNullable<typeof modelContext>['profile'] },
    request: { active: boolean },
  ) {
    const kernel = compatibleKernel(state.kernels, context.profile);
    const text = kernel
      ? await invoke<string>('read_text', { path: `${context.entry.dir}/case.nml` })
      : '';
    const histories = kernel
      ? await invoke<
          {
            name: string;
            writable: boolean | null;
            blocked_by?: string;
            settable: boolean;
            long_name?: string | null;
          }[]
        >('hist_vars', { text, kernelDir: kernel.dir, dir: context.entry.dir })
      : [];
    if (!request.active || epoch.current !== request) return;
    setHistoryVars(
      histories.map((h) => ({
        name: h.name,
        settable: h.settable && h.writable !== false,
        longName: h.long_name,
        reason: h.blocked_by,
      })),
    );
    setContextReason(
      Object.fromEntries(
        histories
          .filter((h) => h.blocked_by)
          .map((h) => [`DEF_hist_vars%${h.name}`, h.blocked_by!]),
      ),
    );
    setDisabledFields(
      histories
        .filter((h) => !h.settable || h.writable === false)
        .map((h) => `DEF_hist_vars%${h.name}`),
    );
  }
  function selectMode(mode: SimulationMode) {
    return changeCaseOptions(caseSchemeFields(mode));
  }
  function discard() {
    setValues({ ...saved });
    setDateDraft({});
    setError('');
    ++forcingSequence.current;
    setForcingInput(savedForcing);
    setForcingTemplate('');
    setForcingProbe(null);
  }
  useEffect(() => {
    const request = { active: true };
    epoch.current = request;
    const versionRef = { id: versionId, revision };
    setSchema([]);
    setValues({});
    setExplicitValues({});
    setSaved({});
    setDateDraft({});
    setContextReason({});
    setDisabledFields([]);
    setModelContext(null);
    setOptionStates([]);
    setError('');
    ++forcingSequence.current;
    setForcingInput(null);
    setSavedForcing(null);
    setForcingTemplate('');
    setForcingProbe(null);
    setForcingPicker(false);
    setForcingInputError('');
    if (!dir) {
      setBusy(false);
      return () => {
        request.active = false;
      };
    }
    setBusy(true);
    async function load() {
      const context = await invoke<{
        entry: { dir: string };
        profile: {
          grid?: string;
          classification?: 'IGBP' | 'USGS';
          subgrid?: string;
          soil?: string;
          bgc?: boolean;
          urban?: boolean;
          crop?: boolean;
        };
      }>('model_case_context', { dir, modelRevision: versionRef });
      if (!request.active) return;
      setModelContext({ dir: dir!, id: versionId, revision, profile: context.profile });
      if (context.profile.grid) {
        const [input, datasets] = await Promise.all([
          invoke<ForcingInput>('case_forcing_input', { dir }).catch((error) => {
            if (request.active) setForcingInputError(String(error));
            return null;
          }),
          invoke<string[]>('forcing_datasets').catch((error) => {
            if (request.active) setForcingInputError(String(error));
            return [];
          }),
        ]);
        if (!request.active) return;
        setForcingInput(input);
        setSavedForcing(input);
        setForcingDatasets(datasets);
      }
      const preview = context.entry.dir,
        kernel = compatibleKernel(state.kernels, context.profile);
      const [all, text, sourceText, modelFields] = await Promise.all([
        invoke<SchemaField[]>('describe_fields', {}),
        invoke<string>('read_text', { path: `${preview}/case.nml` }),
        invoke<string>('read_text', { path: `${dir}/case.nml` }),
        invoke<SchemaField[]>('model_fields', {}),
      ]);
      // Display source paths; the preview is the authority for model-dependent gates.
      const entries = await invoke<{ path: string; value: string }[]>('read_case', {
          text: sourceText,
        }),
        actual = Object.fromEntries(entries.map((e) => [e.path, displayLiteral(e.value)]));
      const modes: { name: string; mode: string; reason?: string }[] = [];
      const histories = kernel
        ? await invoke<
            {
              name: string;
              on: boolean;
              writable: boolean | null;
              blocked_by?: string;
              settable: boolean;
              long_name?: string | null;
            }[]
          >('hist_vars', { text, kernelDir: kernel.dir, dir: preview })
        : [];
      for (const h of histories) actual[`DEF_hist_vars%${h.name}`] = h.on ? '.true.' : '.false.';
      const fields = all
        .filter(isCaseField)
        .filter((f) => !modelFields.some((m) => m.name.toLowerCase() === f.name.toLowerCase()))
        .filter((f) => !f.name.startsWith('DEF_hist_vars%') || histories.length > 0);
      const v = Object.fromEntries(
        fields.map((f) => [f.name, actual[f.name] ?? displayLiteral(f.default)]),
      );
      if (!request.active) return;
      setModelKeys(modelFields);
      setSchema(fields);
      setValues(v);
      setSaved(v);
      setExplicitValues(
        Object.fromEntries(
          Object.entries(actual).filter(([key]) => fields.some((f) => f.name === key)),
        ),
      );
      setContextReason(
        Object.fromEntries([
          ...modes.filter((m) => m.reason).map((m) => [m.name, m.reason!]),
          ...histories
            .filter((h) => h.blocked_by)
            .map((h) => [`DEF_hist_vars%${h.name}`, h.blocked_by!]),
        ]),
      );
      setHistoryVars(
        histories.map((h) => ({
          name: h.name,
          settable: h.settable && h.writable !== false,
          longName: h.long_name,
          reason: h.blocked_by,
        })),
      );
      setDisabledFields([
        ...modes
          .filter((m) => ['disabled', 'readonly'].includes(m.mode.toLowerCase()))
          .map((m) => m.name),
        ...histories
          .filter((h) => !h.settable || h.writable === false)
          .map((h) => `DEF_hist_vars%${h.name}`),
      ]);
    }
    void load()
      .catch((e) => {
        if (request.active) setError(String(e));
      })
      .finally(() => {
        if (request.active) setBusy(false);
      });
    return () => {
      request.active = false;
    };
  }, [dir, versionId, revision, state.kernels, overlay, notify]);
  const configurationKernel = effectiveProfile
    ? compatibleKernel(state.kernels, effectiveProfile, physics)
    : undefined;
  const dataPatch = Object.fromEntries(
    Object.entries(values).filter(
      ([key, value]) =>
        (value !== saved[key] ||
          ['DEF_USE_LCT', 'DEF_USE_PFT', 'DEF_USE_PC', 'DEF_SUBGRID_SCHEME'].includes(key)) &&
        (value !== '' || schema.find((f) => f.name === key)?.kind.startsWith('Character')),
    ),
  );
  const dataRequest = JSON.stringify({
    source: { kind: 'frozen', version: { id: versionId, revision } },
    caseDir: dir ?? null,
    kernelDir: configurationKernel?.dir ?? null,
    pftSelection: null,
    caseValues: dataPatch,
  });
  function changeForcingDirectory(directory: string) {
    ++forcingSequence.current;
    setForcingProbe(null);
    setForcingInput((old) => (old ? { ...old, directory } : old));
  }
  async function detectCaseForcing(path = forcingInput?.directory ?? '') {
    const sequence = ++forcingSequence.current;
    const request = epoch.current;
    if (!path.trim()) {
      setForcingProbe(null);
      return;
    }
    setForcingProbe({ status: 'loading', candidates: [] });
    try {
      const result = await invoke<{ path: string; candidates: string[]; truncated: boolean }>(
        'probe_forcing_directory',
        { path, host: forcingPathHost || null },
      );
      if (request.active && epoch.current === request && forcingSequence.current === sequence)
        setForcingProbe({ status: 'done', ...result });
    } catch (error) {
      if (request.active && epoch.current === request && forcingSequence.current === sequence)
        setForcingProbe({ status: 'error', candidates: [], error: String(error) });
    }
  }
  async function pickForcingDirectory() {
    if (forcingPathHost) {
      setForcingPicker(true);
      return;
    }
    const request = epoch.current;
    const path = await invoke<string | null>('pick_folder', { key: 'case-forcing-directory' });
    if (path && request.active && epoch.current === request) {
      changeForcingDirectory(path);
      await detectCaseForcing(path);
    }
  }
  useEffect(() => {
    let current = true;
    if (!dir || !schema.length) return;
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
  }, [dataRequest, dir, schema.length, explicitValues]);
  const start = dateDraft.start ?? caseDate(values, 'start'),
    end = dateDraft.end ?? caseDate(values, 'end'),
    spinup = dateDraft.spinup ?? caseDate(values, 'spinup');
  const caseRunning = !!(dir && state.running[dir]?.running),
    editingDisabled = busy || caseRunning || runPending;
  function periodProblem() {
    if (!validDate(start) || !validDate(end) || !casePeriodValid(values))
      return t(
        '请确认有效时段，结束时刻须晚于开始时刻。',
        'Choose a period whose end time is after its start.',
      );
    if (spinup && !validDate(spinup))
      return t('请输入有效的预热结束日期。', 'Enter a valid spin-up end date.');
    if (!caseSpinupValid(values))
      return t(
        '预热结束日期须晚于开始日期、早于结束日期。',
        'The spin-up end must be after the start and before the end.',
      );
    return null;
  }
  async function work(fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
      notify(String(e), true);
      throw e;
    } finally {
      lock.current = false;
      setBusy(false);
    }
  }
  async function save() {
    const target = dir,
      id = epoch.current;
    await work(async () => {
      const issue = periodProblem();
      if (issue) throw new Error(issue);
      const fields = Object.entries(values)
        .filter(([k, v]) => v !== saved[k])
        .map(([path, value]) => ({ path, value }));
      if (forcingDirty && !forcingInput?.directory.trim())
        throw new Error(t('请选择强迫场数据路径。', 'Choose the forcing data path.'));
      if (!target || (!fields.length && !forcingDirty)) return;
      await invoke('case_data_save', {
        dirs: [target],
        fields,
        kernelDir:
          (effectiveProfile
            ? compatibleKernel(state.kernels, effectiveProfile, physics)
            : undefined
          )?.dir ?? null,
        modelRevision: state.versionRef,
        ...(forcingDirty && forcingInput
          ? {
              forcing: {
                directory: forcingInput.directory,
                dataset: forcingTemplate || null,
              },
            }
          : {}),
      });
      const context = await invoke<{
        entry: { dir: string };
        profile: NonNullable<typeof modelContext>['profile'];
      }>('model_case_context', { dir: target, modelRevision: state.versionRef });
      if (id.active && epoch.current === id) {
        setModelContext({ dir: target, id: versionId, revision, profile: context.profile });
        if (state.selectedCase?.entry.dir === target)
          update({
            selectedCase: {
              ...state.selectedCase,
              profile: { ...state.selectedCase.profile, ...context.profile },
            },
          });
        setSaved({ ...values });
        if (forcingDirty && forcingInput) {
          const input = { ...forcingInput, dataset: forcingTemplate || forcingInput.dataset };
          setForcingInput(input);
          setSavedForcing(input);
          setForcingTemplate('');
          setValues((old) => ({ ...old, DEF_forcing_namelist: 'forcing.nml' }));
          setSaved((old) => ({ ...old, DEF_forcing_namelist: 'forcing.nml' }));
        }
        setExplicitValues((old) => ({
          ...old,
          ...Object.fromEntries(fields.map((f) => [f.path, f.value])),
        }));
        setDateDraft({});
        await refreshHistory(context, id);
        if (!id.active || epoch.current !== id) return;
        setError('');
        notify(t('算例设置已保存', 'Case settings saved'));
      }
    });
  }
  const labels: Record<string, string> = {
    DEF_dir_existing_srfdata: t('已有地表资料目录', 'Existing surface data'),
    SITE_fsitedata: t('站点文件', 'Site file'),
    DEF_forcing_namelist: t('强迫场配置', 'Forcing configuration'),
    DEF_dir_rawdata: t('地表数据目录', 'Surface data folder'),
    DEF_dir_runtime: t('Runtime 数据目录', 'Runtime data folder'),
    DEF_dir_output: t('算例输出目录', 'Case output folder'),
    DEF_HIST_FREQ: t('输出间隔', 'Output interval'),
    DEF_WRST_FREQ: t('重启文件间隔', 'Restart interval'),
    'DEF_simulation_time%timestep': t('时间步长（秒）', 'Time step (seconds)'),
    'DEF_simulation_time%greenwich': t('使用 UTC 时间', 'Use UTC time'),
    'DEF_simulation_time%spinup_repeat': t('预热循环重复次数', 'Spin-up repetitions'),
  };
  const outputLabels: Record<string, string> = {
    TIMESTEP: t('每个时间步', 'Every time step'),
    HOURLY: t('每小时', 'Hourly'),
    DAILY: t('每日', 'Daily'),
    MONTHLY: t('每月', 'Monthly'),
    YEARLY: t('每年', 'Yearly'),
  };
  const engine: string =
    typeof state.selectedCase?.profile?.engine === 'string'
      ? state.selectedCase.profile.engine
      : 'rust';
  const visibleSchema = schema.filter(
    (field) =>
      !(forcingInput && field.name === 'DEF_forcing_namelist') &&
      (caseSimulationDomain(effectiveProfile) === 'site' || !isSiteConfiguration(field.name)) &&
      caseFieldVisible(field.name, optionStates, { ...explicitValues, ...dataPatch }),
  );
  const rangeKeys = caseRangeKeys(effectiveProfile?.grid ?? state.selectedCase?.profile.grid);
  const rangeFields =
    step === 2 ? visibleSchema.filter((field) => rangeKeys.includes(field.name)) : [];
  const groupFields = schema.filter((field) => simulationOptionField(field, optionStates)),
    shown = [
      ...rangeFields,
      ...simulationFields(visibleSchema, step, advanced, engine).filter(
        (f) => !groupFields.includes(f) && !rangeFields.includes(f),
      ),
    ];
  const optionState = (name: string) => optionStates.find((field) => field.name === name),
    optionDisabled = (name: string) =>
      disabledFields.includes(name) ||
      optionState(name)?.mode === 'disabled' ||
      optionState(name)?.mode === 'readonly';
  const forcingEditingDisabled = editingDisabled || optionDisabled('DEF_forcing_namelist');
  const dateDisabled = (prefix: string) =>
    editingDisabled ||
    ['year', 'month', 'day'].some(
      (part) =>
        !schema.some((f) => f.name === `DEF_simulation_time%${prefix}_${part}`) ||
        disabledFields.includes(`DEF_simulation_time%${prefix}_${part}`),
    );
  function setDate(prefix: 'start' | 'end' | 'spinup', value: string) {
    setDateDraft((old) => ({ ...old, [prefix]: value }));
    setError('');
    if (prefix === 'spinup' && !value) {
      setValues((old) => ({
        ...old,
        'DEF_simulation_time%spinup_year': '0',
        'DEF_simulation_time%spinup_month':
          saved['DEF_simulation_time%spinup_month'] ?? old['DEF_simulation_time%spinup_month'],
        'DEF_simulation_time%spinup_day':
          saved['DEF_simulation_time%spinup_day'] ?? old['DEF_simulation_time%spinup_day'],
      }));
      return;
    }
    if (validDate(value)) {
      const patch = caseDatePatch(value, prefix);
      setValues((old) => ({ ...old, ...patch }));
    }
  }
  function next() {
    const issue = step >= 3 ? periodProblem() : null;
    if (issue) {
      setError(issue);
      throw new Error(issue);
    }
    setError('');
    setStep((step + 1) as SimulationStep);
  }
  async function saveAndContinue() {
    await save();
    setStep((step + 1) as SimulationStep);
  }
  // 研究从"新建修改版"进去（2026-10-21 研究不再单独占侧栏一项）。
  function openResearch() {
    update({ simulationReturn: { route: 'case', step: 4 }, versionsView: 'create' });
    return navigate('versions');
  }
  const datePrefixes: ('start' | 'end' | 'spinup')[] = step === 3 ? ['start', 'end', 'spinup'] : [];
  const fields: UiField[] =
    dir && state.caseTab === 'settings'
      ? [
          ...datePrefixes.map((prefix) => ({
            id: `case-${prefix}-date`,
            label:
              prefix === 'start'
                ? t('开始日期', 'Start date')
                : prefix === 'end'
                  ? t('结束日期', 'End date')
                  : t('预热结束日期（留空不预热）', 'Spin-up end (blank disables spin-up)'),
            value: prefix === 'start' ? start : prefix === 'end' ? end : spinup,
            type: 'text' as const,
            disabled: dateDisabled(prefix),
            set: (value: string) => setDate(prefix, value),
          })),
          ...shown.map((f) => ({
            id: `case-${f.name}`,
            label: labels[f.name] || fieldLabel(f.name, state.language),
            value: values[f.name] ?? '',
            type:
              f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
                ? ('choice' as const)
                : ('text' as const),
            ...(f.kind === 'Logical' || fieldInputOptions(f.name, f.kind, f.values).length
              ? {
                  options: (f.kind === 'Logical'
                    ? ['.true.', '.false.']
                    : optionState(f.name)?.allowed_values.length
                      ? optionState(f.name)!.allowed_values
                      : f.values
                  ).map((value: string) => ({
                    value,
                    label:
                      f.kind === 'Logical'
                        ? t(value === '.true.' ? '开启' : '关闭', value === '.true.' ? 'On' : 'Off')
                        : f.name === 'DEF_HIST_FREQ'
                          ? (outputLabels[value] ?? value)
                          : optionLabel(f.name, value, state.language),
                  })),
                }
              : {}),
            disabled: editingDisabled || optionDisabled(f.name) || f.name === 'DEF_dir_output',
            set: (v: string) => setValues((x) => ({ ...x, [f.name]: v })),
          })),
        ]
      : [];
  if (dir && state.caseTab === 'settings' && step === 3 && forcingInput)
    fields.push(
      {
        id: 'case-forcing-directory',
        label: t('强迫场数据路径', 'Forcing data path'),
        type: 'text',
        value: forcingInput.directory,
        disabled: forcingEditingDisabled,
        set: (value) => {
          changeForcingDirectory(value);
          void detectCaseForcing(value);
        },
      },
      {
        id: 'case-forcing-dataset',
        label: t('强迫场数据集', 'Forcing dataset'),
        type: 'choice',
        value: forcingTemplate,
        disabled: forcingEditingDisabled,
        options: [
          {
            value: '',
            label: `${t('保留当前配置', 'Keep current configuration')}（${forcingInput.dataset}）`,
          },
          ...forcingDatasets.map((value) => ({ value, label: value })),
        ],
        set: setForcingTemplate,
      },
    );
  if (dir && state.caseTab === 'settings' && step === 5)
    fields.push(
      ...historyVars.map((v) => ({
        id: `case-${historyKey(v.name)}`,
        label: v.longName ? `${v.name} · ${v.longName}` : v.name,
        value: values[historyKey(v.name)] ?? '',
        type: 'choice' as const,
        options: ['.true.', '.false.'].map((value) => ({
          value,
          label: t(value === '.true.' ? '输出' : '不输出', value === '.true.' ? 'On' : 'Off'),
        })),
        disabled: editingDisabled || !v.settable,
        set: (value: string) => setValues((x) => ({ ...x, [historyKey(v.name)]: value })),
      })),
    );
  if (dir && state.caseTab === 'settings' && step === 1) {
    fields.push({
      id: 'case-run-host',
      label: t('运行位置', 'Run location'),
      type: 'choice',
      value: runHost,
      options: [
        { value: '', label: t('本机', 'Local') },
        ...(servers ?? []).map((server) => ({ value: server.host, label: server.host })),
      ],
      disabled: editingDisabled || dirty || servers === null || !!remoteDirectory,
      set: changeRunHost,
    });
    fields.push({
      id: 'case-DEF_dir_output',
      label: labels.DEF_dir_output,
      value: values['DEF_dir_output'] ?? (dir ? `${dir}/out` : ''),
      type: 'text' as const,
      disabled: true,
      set: () => {},
    });
  }
  if (
    dir &&
    state.caseTab === 'settings' &&
    [2, 3, 5].includes(step) &&
    (step !== 2 || engine === 'fortran')
  )
    fields.push({
      id: 'case-advanced',
      label: t('更多配置', 'More settings'),
      value: String(advanced),
      type: 'switch',
      disabled: editingDisabled,
      set: (v) => setAdvanced(v === 'true'),
    });
  usePageContract({
    title:
      state.caseTab === 'settings'
        ? t('模拟', 'Simulation')
        : (state.selectedCase?.entry.name ?? t('模拟', 'Simulation')),
    step: 'case',
    status: busy ? 'busy' : error ? 'error' : dirty ? 'unsaved' : 'ready',
    fields,
    details:
      state.caseTab === 'settings'
        ? {
            simulation_step: step,
            ...(step >= 3 ? { period: { start, end } } : {}),
            case: state.selectedCase?.entry,
            settings_saved: !busy && !error && !dirty,
            ...(step === 3 && forcingInput ? { forcing: forcingProbe } : {}),
          }
        : undefined,
    // 按钮自己登记（第 737 轮）。
    actions: [],
  });
  // 项目里有算例、只是没选（批量运行之后）：结果页自带选站点。一个算例都没有时结果页什么都选不了，
  // 原来照样铺开一整页空的评估界面、按钮点了也没用，和模拟页一样给出打开项目或新建模拟。
  const singlePointCase = state.selectedCase?.entry.spatial === false;
  useEffect(() => {
    if (!dir || step !== 4 || !singlePointCase) return;
    let live = true;
    invoke<{ igbp: boolean; usgs: boolean }>('site_classifications', { dir })
      .then((value) => {
        if (live) setSiteClasses({ dir, value });
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [dir, step, singlePointCase]);
  if (!dir && state.caseTab === 'results' && state.cases.length > 0)
    return <ResultsPage onBack={backToSimulation} />;
  if (!dir)
    return (
      <div className="page-stack simulation-page">
        <h1>{state.caseTab === 'results' ? t('结果', 'Results') : t('模拟', 'Simulation')}</h1>
        <Card
          title={
            state.caseTab === 'results'
              ? t('还没有可看的结果', 'No results to show yet')
              : t('开始一次模拟', 'Start a simulation')
          }
          description={
            state.caseTab === 'results'
              ? t(
                  '打开一个项目选算例，或新建模拟跑完后在这里看结果。',
                  'Open a project and choose a case, or create a simulation and view its results here once it has run.',
                )
              : t(
                  '新建模拟，或从项目中打开已有算例。',
                  'Create a simulation or open an existing case from a project.',
                )
          }
        >
          <div className="actions">
            <ActionButton
              action="case-new"
              label={t('新建模拟', 'New simulation')}
              variant="primary"
              run={() => navigate('new-case')}
            >
              ＋ {t('新建模拟', 'New simulation')}
            </ActionButton>
            {/* 和首页的"打开项目"去同一个地方（历史记录），不先回首页再点一次。 */}
            <ActionButton
              action="case-project"
              label={t('打开项目', 'Open project')}
              run={() => navigate('history')}
            />
          </div>
        </Card>
      </div>
    );
  const choice = (f: SchemaField) =>
    selectChoice(
      values[f.name] ?? '',
      optionState(f.name)?.allowed_values.length ? optionState(f.name)!.allowed_values : f.values,
    );
  const render = (f: SchemaField) => (
    <Field
      key={f.name}
      label={labels[f.name] || fieldLabel(f.name, state.language)}
      hint={
        configurationReason(optionState(f.name)?.reason) ??
        configurationReason(contextReason[f.name]) ??
        (advanced ? f.name : undefined)
      }
    >
      {f.name === 'DEF_dir_output' ? (
        <Input readOnly value={values[f.name] ?? ''} />
      ) : f.kind === 'Logical' ? (
        <Select
          value={values[f.name] ?? ''}
          disabled={editingDisabled || optionDisabled(f.name)}
          onChange={(e) => setValues((x) => ({ ...x, [f.name]: e.target.value }))}
        >
          <option value=".true.">{t('开启', 'On')}</option>
          <option value=".false.">{t('关闭', 'Off')}</option>
        </Select>
      ) : fieldInputOptions(f.name, f.kind, f.values).length ? (
        <Select
          value={choice(f).value}
          disabled={editingDisabled || optionDisabled(f.name)}
          onChange={(e) => setValues((x) => ({ ...x, [f.name]: e.target.value }))}
        >
          {choice(f).options.map((v) => (
            <option key={v} value={v}>
              {f.name === 'DEF_HIST_FREQ'
                ? (outputLabels[v] ?? v)
                : optionLabel(f.name, v, state.language)}
            </option>
          ))}
        </Select>
      ) : (
        <Input
          value={values[f.name] ?? ''}
          disabled={editingDisabled || optionDisabled(f.name)}
          onChange={(e) => setValues((x) => ({ ...x, [f.name]: e.target.value }))}
        />
      )}
    </Field>
  );
  const renderDate = (prefix: 'start' | 'end' | 'spinup') => (
    <Field
      key={prefix}
      label={
        prefix === 'start'
          ? t('开始日期', 'Start date')
          : prefix === 'end'
            ? t('结束日期', 'End date')
            : t('预热结束日期（留空不预热）', 'Spin-up end (blank disables spin-up)')
      }
    >
      <DateInput
        calendarLabel={`${t('选择日期', 'Choose date')} · ${prefix === 'start' ? t('开始日期', 'Start date') : prefix === 'end' ? t('结束日期', 'End date') : t('预热结束日期', 'Spin-up end')}`}
        value={prefix === 'start' ? start : prefix === 'end' ? end : spinup}
        disabled={dateDisabled(prefix)}
        onChange={(e) => setDate(prefix, e.target.value)}
      />
    </Field>
  );
  const basic = simulationFields(visibleSchema, step, false, engine).filter(
      (f) => !groupFields.includes(f),
    ),
    extras = shown.filter(
      (f) => !basic.includes(f) && !groupFields.includes(f) && !rangeFields.includes(f),
    );
  const timeBasic = basic.filter((f) => f.name.startsWith('DEF_simulation_time%'));
  const outputBasic = basic.filter((f) => !f.name.startsWith('DEF_simulation_time%'));
  const more = () => (
    <Advanced open={advanced} onOpenChange={setAdvanced} title={t('更多配置', 'More settings')}>
      {advanced && (
        <>
          <div className="form-grid">{extras.map(render)}</div>
        </>
      )}
    </Advanced>
  );
  return (
    <div className="page-stack simulation-page">
      {state.caseTab !== 'results' && (
        <header className="simulation-heading">
          <div>
            <h1>
              {state.caseTab === 'settings'
                ? t('模拟', 'Simulation')
                : state.selectedCase?.entry.name}
            </h1>
            <p className="muted">
              {state.selectedCase?.entry.name}
              {casePeriodText(values) ? ` · ${casePeriodText(values)}` : ''}
            </p>
          </div>
          <div className="actions">
            {state.caseTab === 'settings' ? (
              <>
                <ActionButton
                  action="case-tab-runs"
                  label={t('运行记录', 'Run history')}
                  variant="ghost"
                  run={() => update({ caseTab: 'runs' })}
                />
                <ActionButton
                  action="case-tab-results"
                  label={t('查看结果', 'View results')}
                  variant="ghost"
                  run={() => update({ caseTab: 'results' })}
                />
              </>
            ) : (
              <ActionButton
                action="case-tab-settings"
                label={t('返回模拟', 'Back to simulation')}
                run={backToSimulation}
              />
            )}
          </div>
        </header>
      )}
      {state.caseTab === 'settings' && (
        <>
          <SimulationSteps
            step={step}
            en={en}
            disabled={editingDisabled}
            actionPrefix="case"
            onChange={setStep}
          />
          {error && <Notice tone="error">{error}</Notice>}
          {optionError && <Notice tone="warning">{optionError}</Notice>}
          {busy && (
            <Notice>{t('正在读取模型与算例设置…', 'Reading model and case settings…')}</Notice>
          )}
          {dirty && (
            <Notice tone="warning">
              {t(
                '设置尚未保存，保存后才能开始运行或调整模型。',
                'Save your changes before running or changing the model.',
              )}{' '}
              <ActionButton
                action="case-discard"
                label={t('放弃本地修改', 'Discard local edits')}
                disabled={editingDisabled}
                run={discard}
              >
                {t('放弃修改', 'Discard edits')}
              </ActionButton>
            </Notice>
          )}
          {step === 1 && (
            <Card
              title={t('总的设定', 'General setup')}
              description={t(
                '选择运行位置，核对计算引擎、算例保存路径与输出目录。',
                'Choose the run location and review the engine, case directory and output directory.',
              )}
            >
              <RunLocationSettings
                host={runHost}
                servers={servers}
                disabled={editingDisabled || dirty}
                serverNative={!!remoteDirectory}
                locationLocked={!!remoteDirectory}
                error={resourceError}
                en={en}
                onChange={(host) => void changeRunHost(host).catch(() => {})}
                onConfigure={openResources}
                configureAction="case-run-resources"
              />
              {remoteDirectory && (
                <Notice>
                  {t(
                    '此算例的数据和项目位于服务器，运行位置固定；迁移请新建模拟。',
                    'This case uses server data and a server project directory. Create a new simulation to migrate it.',
                  )}{' '}
                  {runHost}: {remoteDirectory}
                </Notice>
              )}
              <div className="simulation-site-summary">
                <strong>{state.selectedCase?.entry.name}</strong>
                <span>
                  {t('计算引擎：', 'Engine: ')}
                  {engine === 'fortran' ? 'Fortran' : 'Rust'}
                </span>
              </div>
              <div className="form-grid">
                <Field
                  label={
                    remoteDirectory
                      ? t('服务器算例路径', 'Server case directory')
                      : runHost
                        ? t('本地算例路径', 'Local case directory')
                        : t('算例路径', 'Case directory')
                  }
                >
                  <Input readOnly value={remoteDirectory ?? dir ?? ''} />
                </Field>
                <Field
                  label={t('输出目录', 'Output directory')}
                  hint={t(
                    '直接运行使用此目录；模型版本运行的配置和结果保存在算例目录内 runs/ 下各次运行的子目录中。',
                    'Direct runs use this directory; model-version runs keep their configurations and outputs in separate run directories under runs/ inside this case.',
                  )}
                >
                  <Input readOnly value={values['DEF_dir_output'] ?? (dir ? `${dir}/out` : '')} />
                </Field>
              </div>
              <p className="muted">
                {t(
                  '已有算例的基础配置由建例时确定；如需更换计算引擎或算例目录，请新建模拟。',
                  'General settings are set when creating the case. Create a new simulation to change the engine or directory.',
                )}
              </p>
            </Card>
          )}
          {step === 2 && (
            <Card title={t('范围与网格', 'Range and grid')}>
              <div className="simulation-site-summary">
                <strong>{state.selectedCase?.entry.name}</strong>
                <span>
                  {state.selectedCase?.profile.spatial
                    ? t('空间模拟', 'Spatial simulation')
                    : t('单点', 'Single site')}
                </span>
                <span>{state.selectedCase?.profile.grid ?? ''}</span>
              </div>
              <p className="muted">
                {t(
                  '可修改当前范围与网格配置。更换站点或网格后，请在下一步匹配对应数据；旧运行结果仍保留。',
                  'Edit the current range and grid settings, then match the corresponding inputs in the next step. Existing run results are preserved.',
                )}
              </p>
              <div className="form-grid">{rangeFields.map(render)}</div>
              {more()}
            </Card>
          )}
          {step === 3 && (
            <Card
              title={t('数据与时段', 'Data and period')}
              description={t(
                '确认数据路径与模拟起止时段。',
                'Check data paths and simulation period.',
              )}
            >
              <div className="simulation-dates">
                {renderDate('start')}
                {renderDate('end')}
              </div>
              <p className="field-hint">
                {t(
                  '可填 YYYY-MM-DD 或 YYYYMMDD；完整有效日期自动补齐分隔符，也可点日历选择。结束日期是截止时刻；日内起止秒数保持原配置。',
                  'Enter YYYY-MM-DD or YYYYMMDD; complete valid dates gain separators automatically. You can also choose from the calendar. The end is a cutoff; existing time-of-day seconds are preserved.',
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
                <div>
                  <strong style={{ fontSize: '14px' }}>
                    {t('模型预热（Spin-up）设定', 'Model Spin-up Settings')}
                  </strong>
                  <p className="muted" style={{ margin: '2px 0 0', fontSize: '12px' }}>
                    {t(
                      '让土壤温湿与碳库等慢变量先趋于平衡。模型从开始日期跑到预热结束日期（当日 00:00 截止），重置时间后重复指定次数，再进入正式模拟；结束日期留空不预热。',
                      'Let slow pools reach equilibrium. The model runs from the start to the spin-up end (00:00 that day), resets and repeats the given number of times, then starts the formal run; leave the end blank to disable spin-up.',
                    )}
                  </p>
                </div>
                <div className="form-grid" style={{ marginTop: '12px' }}>
                  <Field
                    label={t('预热起始日期', 'Spin-up start')}
                    hint={t(
                      'CoLM 预热总是从模拟开始日期起算',
                      'CoLM spin-up always starts at the simulation start',
                    )}
                  >
                    <Input readOnly value={start} />
                  </Field>
                  {renderDate('spinup')}
                  {timeBasic.filter((f) => f.name.endsWith('spinup_repeat')).map(render)}
                </div>
              </div>
              <div className="form-grid" style={{ marginTop: '1rem' }}>
                {basic.filter((f) => !f.name.startsWith('DEF_simulation_time%')).map(render)}
              </div>
              {forcingInputError && <Notice tone="warning">{forcingInputError}</Notice>}
              {forcingInput && (
                <SpatialForcingFields
                  directory={forcingInput.directory}
                  dataset={forcingTemplate}
                  currentDataset={forcingInput.dataset}
                  datasets={forcingDatasets}
                  probe={forcingProbe}
                  catalogError=""
                  disabled={forcingEditingDisabled}
                  en={en}
                  onDirectoryChange={changeForcingDirectory}
                  onDirectoryBlur={() => void detectCaseForcing()}
                  onPick={() =>
                    void pickForcingDirectory().catch((error) => setError(String(error)))
                  }
                  onDatasetChange={setForcingTemplate}
                  pickAction="case-pick-forcing-directory"
                  detectAction="case-detect-forcing"
                  onDetect={() => void detectCaseForcing()}
                />
              )}
              {forcingPicker && (
                <RemotePathBrowser
                  host={forcingPathHost}
                  initialPath={forcingInput?.directory}
                  directory
                  en={en}
                  onSelect={(path) => {
                    setForcingPicker(false);
                    changeForcingDirectory(path);
                    void detectCaseForcing(path);
                  }}
                  onClose={() => setForcingPicker(false)}
                />
              )}
              {more()}
            </Card>
          )}
          {step === 4 && (
            <>
              <ModelSettings
                runHost={remoteDirectory ? runHost : undefined}
                version={state.version}
                profile={effectiveProfile ?? undefined}
                caseDir={dir}
                classification={effectiveProfile?.classification}
                kernel={
                  effectiveProfile
                    ? compatibleKernel(state.kernels, effectiveProfile, physics)
                    : undefined
                }
                onModeChange={(mode) => {
                  void selectMode(mode).catch(() => {});
                }}
                setupEnvironment={{
                  engine,
                  subgrid: effectiveProfile?.subgrid,
                  caseValues: explicitValues,
                  domain: caseSimulationDomain(effectiveProfile),
                  grid: effectiveProfile?.grid,
                  classification: effectiveProfile?.classification,
                  kernel: effectiveProfile
                    ? compatibleKernel(state.kernels, effectiveProfile, physics)
                    : undefined,
                  cropKernel: effectiveProfile
                    ? compatibleKernel(state.kernels, { ...effectiveProfile, crop: true }, physics)
                    : undefined,
                }}
                onCaseChange={changeCaseOptions}
                siteClassifications={siteClasses?.dir === dir ? siteClasses.value : null}
                disabled={editingDisabled || dirty}
                onEdit={() => openResearch()}
              />
              {dirty && (
                <p className="field-hint">
                  {t(
                    '先保存或放弃算例修改，再进入模型方案调整。',
                    'Save or discard case edits before changing model settings.',
                  )}
                </p>
              )}
            </>
          )}
          {step === 5 && (
            <>
              <Card
                title={t('时间步长与输出', 'Time step and output')}
                description={t(
                  '设置时间步长、输出频率与输出变量；预热在“数据与时段”中设置。更多配置中包含重启与诊断。',
                  'Set the time step, output frequency and variables; spin-up is set under Data and period. More settings include restart and diagnostics.',
                )}
              >
                <div className="form-grid">
                  {timeBasic.filter((f) => !f.name.endsWith('spinup_repeat')).map(render)}
                </div>
                <h3 className="simulation-subheading">{t('输出设置', 'Output settings')}</h3>
                <div className="form-grid">{outputBasic.map(render)}</div>
                {more()}
              </Card>
              {historyVars.length > 0 && (
                <HistoryVariablesCard
                  actionPrefix="case"
                  variables={historyVars}
                  values={values}
                  disabled={editingDisabled}
                  en={en}
                  onChange={(patch) => setValues((x) => ({ ...x, ...patch }))}
                />
              )}
            </>
          )}
          {step < 5 && (
            <footer className="simulation-footer">
              {step > 1 ? (
                <ActionButton
                  action="case-previous"
                  label={t('上一步', 'Back')}
                  disabled={editingDisabled}
                  run={() => setStep((step - 1) as SimulationStep)}
                />
              ) : (
                <span />
              )}
              <ActionButton
                action={dirty ? 'case-save' : 'case-next'}
                kind={dirty ? 'commit' : 'click'}
                label={dirty ? t('保存并继续', 'Save and continue') : t('下一步', 'Next')}
                variant="primary"
                disabled={
                  editingDisabled || !schema.length || (step >= 3 && periodProblem() !== null)
                }
                run={dirty ? saveAndContinue : next}
              >
                {dirty ? t('保存并继续', 'Save and continue') : t('下一步', 'Next')} →
              </ActionButton>
            </footer>
          )}
          {/* 时段或预热截止不对时保存/下一步不可用；原来原因只出现在点击后的报错里，按钮置灰后就看不到了。 */}
          {step >= 3 && !editingDisabled && periodProblem() && (
            <Notice tone="warning">{periodProblem()}</Notice>
          )}
          <div hidden={step !== 5}>
            <RunPanel
              key={dir}
              active={step === 5}
              onBusyChange={setRunPending}
              casePath={dir}
              effectiveProfile={effectiveProfile}
              disabled={busy || dirty || !!error || !schema.length}
              summary={{
                name: state.selectedCase?.entry.name ?? '',
                period: casePeriodText(values) || t('时段未读取', 'Period unavailable'),
              }}
              onBack={() => setStep(4)}
            />
          </div>
          {step === 5 && dirty && (
            <ActionButton
              action="case-save"
              kind="commit"
              label={t('保存设置', 'Save settings')}
              variant="primary"
              disabled={editingDisabled || periodProblem() !== null}
              run={save}
            />
          )}
        </>
      )}
      {state.caseTab === 'runs' && <HistoryPage casePath={dir} />}{' '}
      {state.caseTab === 'results' && <ResultsPage casePath={dir} onBack={backToSimulation} />}
    </div>
  );
}
