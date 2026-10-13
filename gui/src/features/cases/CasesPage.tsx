import { useEffect, useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { useApp, useNavigationGuard } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Input, Select, Field, Notice, Advanced } from '../../components/ui';
import { isCaseField, displayLiteral, effectiveModelFields, type SchemaField } from './case-model';
import { compatibleKernel } from '../runs/run-model';
import {
  caseDate,
  caseDatePatch,
  casePeriodValid,
  caseSpinupValid,
  casePeriodText,
  simulationFields,
  caseRangeKeys,
  simulationSteps,
  validDate,
  type SimulationStep,
} from './simulation-model';
import { fieldLabel, optionLabel } from '../../lib/param-presentation';
import { caseFieldVisible, type ConfigurationField } from '../versions/configuration-options';
import { simulationOptionField } from './model-settings';
import SimulationSteps from './SimulationSteps';
import ModelSettings from './ModelSettings';
import { caseSchemeFields, type SimulationMode } from './model-settings';
import type { CaseOptionPatch } from '../versions/configuration-options';
import RunPanel from '../runs/RunPanel';
import ResultsPage from '../results/ResultsPage';
import HistoryPage from '../history/HistoryPage';
export default function CasesPage() {
  const { state, update, navigate, notify } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh),
    dir = state.selectedCase?.entry.dir;
  const [step, setStep] = useState<SimulationStep>(
      state.simulationReturn?.route === 'case' ? 2 : 5,
    ),
    [error, setError] = useState('');
  const [dateDraft, setDateDraft] = useState<Partial<Record<'start' | 'end' | 'spinup', string>>>(
    {},
  );
  const [contextReason, setContextReason] = useState<Record<string, string>>({}),
    [disabledFields, setDisabledFields] = useState<string[]>([]),
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
  const dirty =
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
  async function changeCaseOptions(patch: CaseOptionPatch) {
    if (!dir || editingDisabled || dirty || lock.current)
      throw new Error(t('请先保存或放弃当前修改。', 'Save or discard current edits first.'));
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
        setError('');
      }
    });
  }
  function selectMode(mode: SimulationMode) {
    return changeCaseOptions(caseSchemeFields(mode));
  }
  function discard() {
    setValues({ ...saved });
    setDateDraft({});
    setError('');
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
    editingDisabled = busy || caseRunning;
  function periodProblem() {
    if (!validDate(start) || !validDate(end) || !casePeriodValid(values))
      return t(
        '请确认有效时段，结束时刻须晚于开始时刻。',
        'Choose a period whose end time is after its start.',
      );
    if (spinup && !validDate(spinup))
      return t('请输入有效的预热截止日期。', 'Enter a valid spinup cutoff.');
    if (!caseSpinupValid(values))
      return t(
        '预热截止时刻须晚于开始时刻、早于结束时刻。',
        'The spinup cutoff must be after the start and before the end.',
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
      if (!target || !fields.length) return;
      await invoke('case_data_save', {
        dirs: [target],
        fields,
        kernelDir:
          (effectiveProfile
            ? compatibleKernel(state.kernels, effectiveProfile, physics)
            : undefined
          )?.dir ?? null,
        modelRevision: state.versionRef,
      });
      const context = await invoke<{ profile: NonNullable<typeof modelContext>['profile'] }>(
        'model_case_context',
        { dir: target, modelRevision: state.versionRef },
      );
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
        setExplicitValues((old) => ({
          ...old,
          ...Object.fromEntries(fields.map((f) => [f.path, f.value])),
        }));
        setDateDraft({});
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
    DEF_HIST_groupby: t('输出文件分组', 'Output file grouping'),
    DEF_WRST_FREQ: t('重启文件间隔', 'Restart interval'),
    'DEF_simulation_time%timestep': t('时间步长（秒）', 'Time step (seconds)'),
    'DEF_simulation_time%greenwich': t('使用 UTC 时间', 'Use UTC time'),
    'DEF_simulation_time%spinup_repeat': t('预热重复次数', 'Spinup repetitions'),
  };
  const outputLabels: Record<string, string> = {
    TIMESTEP: t('每个时间步', 'Every time step'),
    HOURLY: t('每小时', 'Hourly'),
    DAILY: t('每日', 'Daily'),
    MONTHLY: t('每月', 'Monthly'),
    YEARLY: t('每年', 'Yearly'),
  };
  const visibleSchema = schema.filter((field) =>
    caseFieldVisible(field.name, optionStates, { ...explicitValues, ...dataPatch }),
  );
  const rangeKeys = caseRangeKeys(effectiveProfile?.grid ?? state.selectedCase?.profile.grid);
  const rangeFields =
    step === 1 ? visibleSchema.filter((field) => rangeKeys.includes(field.name)) : [];
  const groupFields = schema.filter((field) => simulationOptionField(field, optionStates)),
    shown = [
      ...rangeFields,
      ...simulationFields(visibleSchema, step, advanced).filter(
        (f) => !groupFields.includes(f) && !rangeFields.includes(f),
      ),
    ];
  const optionState = (name: string) => optionStates.find((field) => field.name === name),
    optionDisabled = (name: string) =>
      disabledFields.includes(name) ||
      optionState(name)?.mode === 'disabled' ||
      optionState(name)?.mode === 'readonly';
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
  function openResearch() {
    update({ simulationReturn: { route: 'case', step: 2 } });
    return navigate('research');
  }
  const datePrefixes: ('start' | 'end' | 'spinup')[] =
    step === 3 ? ['start', 'end'] : step === 4 ? ['spinup'] : [];
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
                  : t('预热截止日期（留空不预热）', 'Spinup cutoff (blank disables spinup)'),
            value: prefix === 'start' ? start : prefix === 'end' ? end : spinup,
            type: 'text' as const,
            disabled: dateDisabled(prefix),
            set: (value: string) => setDate(prefix, value),
          })),
          ...shown.map((f) => ({
            id: `case-${f.name}`,
            label: labels[f.name] || fieldLabel(f.name, state.language),
            value: values[f.name] ?? '',
            type: f.kind === 'Logical' || f.values.length ? ('choice' as const) : ('text' as const),
            ...(f.kind === 'Logical' || f.values.length
              ? {
                  options: (f.kind === 'Logical'
                    ? ['.true.', '.false.']
                    : optionState(f.name)?.allowed_values.length
                      ? optionState(f.name)!.allowed_values
                      : f.values
                  ).map((value) => ({
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
  if (dir && state.caseTab === 'settings' && [1, 3, 4].includes(step))
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
          }
        : undefined,
    actions: !dir
      ? [
          {
            id: 'case-project',
            label: t('打开项目', 'Open project'),
            kind: 'click',
            run: () => navigate('project'),
          },
          {
            id: 'case-new',
            label: t('新建模拟', 'New simulation'),
            kind: 'click',
            run: () => navigate('new-case'),
          },
        ]
      : [
          ...(state.caseTab === 'settings'
            ? (['runs', 'results'] as const)
            : (['settings'] as const)
          ).map((tab) => ({
            id: `case-tab-${tab}`,
            label:
              tab === 'runs'
                ? t('运行记录', 'Run history')
                : tab === 'results'
                  ? t('查看结果', 'View results')
                  : t('返回模拟', 'Back to simulation'),
            kind: 'click' as const,
            run: () => update({ caseTab: tab }),
          })),
          ...(dirty
            ? [
                {
                  id: 'case-discard',
                  label: t('放弃本地修改', 'Discard local edits'),
                  kind: 'click' as const,
                  enabled: !editingDisabled,
                  run: discard,
                },
              ]
            : []),
          ...(state.caseTab === 'settings'
            ? [
                ...simulationSteps.slice(0, step - 1).map((label, i) => ({
                  id: `case-step-${i + 1}`,
                  label: label[en ? 1 : 0],
                  kind: 'click' as const,
                  enabled: !editingDisabled,
                  run: () => setStep((i + 1) as SimulationStep),
                })),
                ...(step > 1
                  ? [
                      {
                        id: 'case-previous',
                        label: t('上一步', 'Back'),
                        kind: 'click' as const,
                        enabled: !editingDisabled,
                        run: () => setStep((step - 1) as SimulationStep),
                      },
                    ]
                  : []),
                ...(step < 5 && !dirty
                  ? [
                      {
                        id: 'case-next',
                        label: t('下一步', 'Next'),
                        kind: 'click' as const,
                        enabled: !editingDisabled && !!schema.length,
                        run: next,
                      },
                    ]
                  : []),
                ...(dirty
                  ? [
                      {
                        id: 'case-save',
                        label:
                          step < 5
                            ? t('保存并继续', 'Save and continue')
                            : t('保存设置', 'Save settings'),
                        kind: 'commit' as const,
                        enabled: !editingDisabled,
                        run: step < 5 ? saveAndContinue : save,
                      },
                    ]
                  : []),
              ]
            : []),
        ],
  });
  if (!dir)
    return (
      <div className="page-stack simulation-page">
        <h1>{t('模拟', 'Simulation')}</h1>
        <Card
          title={t('开始一次模拟', 'Start a simulation')}
          description={t(
            '新建模拟，或从项目中打开已有算例。',
            'Create a simulation or open an existing case from a project.',
          )}
        >
          <div className="actions">
            <Button variant="primary" onClick={() => navigate('new-case')}>
              ＋ {t('新建模拟', 'New simulation')}
            </Button>
            <Button onClick={() => navigate('project')}>{t('打开项目', 'Open project')}</Button>
          </div>
        </Card>
      </div>
    );
  const render = (f: SchemaField) => (
    <Field
      key={f.name}
      label={labels[f.name] || fieldLabel(f.name, state.language)}
      hint={optionState(f.name)?.reason ?? contextReason[f.name] ?? (advanced ? f.name : undefined)}
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
      ) : f.values.length ? (
        <Select
          value={values[f.name] ?? ''}
          disabled={editingDisabled || optionDisabled(f.name)}
          onChange={(e) => setValues((x) => ({ ...x, [f.name]: e.target.value }))}
        >
          {(optionState(f.name)?.allowed_values.length
            ? optionState(f.name)!.allowed_values
            : f.values
          ).map((v) => (
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
            : t('预热截止日期（留空不预热）', 'Spinup cutoff (blank disables spinup)')
      }
    >
      <Input
        type="text"
        placeholder="YYYY-MM-DD"
        maxLength={10}
        value={prefix === 'start' ? start : prefix === 'end' ? end : spinup}
        disabled={dateDisabled(prefix)}
        onChange={(e) => setDate(prefix, e.target.value)}
      />
    </Field>
  );
  const basic = simulationFields(visibleSchema, step, false).filter(
      (f) => !groupFields.includes(f),
    ),
    extras = shown.filter((f) => !basic.includes(f) && !groupFields.includes(f));
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
                <Button variant="ghost" onClick={() => update({ caseTab: 'runs' })}>
                  {t('运行记录', 'Run history')}
                </Button>
                <Button variant="ghost" onClick={() => update({ caseTab: 'results' })}>
                  {t('查看结果', 'View results')}
                </Button>
              </>
            ) : (
              <Button onClick={() => update({ caseTab: 'settings' })}>
                {t('返回模拟', 'Back to simulation')}
              </Button>
            )}
          </div>
        </header>
      )}
      {state.caseTab === 'settings' && (
        <>
          <SimulationSteps step={step} en={en} disabled={editingDisabled} onChange={setStep} />
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
              <Button disabled={editingDisabled} onClick={discard}>
                {t('放弃修改', 'Discard edits')}
              </Button>
            </Notice>
          )}
          {step === 1 && (
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
                '确认数据路径、模拟时段与初始条件。',
                'Check data paths, the simulation period and initial conditions.',
              )}
            >
              <div className="simulation-dates">
                {renderDate('start')}
                {renderDate('end')}
              </div>
              <p className="field-hint">
                {t(
                  '格式 YYYY-MM-DD，例如 2001-01-01。结束日期是截止时刻；日内起止秒数保持原配置。',
                  'Use YYYY-MM-DD, e.g. 2001-01-01. The end is a cutoff; existing time-of-day seconds are preserved.',
                )}
              </p>
              <div className="form-grid">{basic.map(render)}</div>

              {more()}
            </Card>
          )}
          {step === 2 && (
            <>
              <ModelSettings
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
                  subgrid: effectiveProfile?.subgrid,
                  caseValues: explicitValues,
                  domain: effectiveProfile?.grid ? 'region' : effectiveProfile ? 'site' : null,
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
          {step === 4 && (
            <Card
              title={t('预热与输出', 'Spinup and output')}
              description={t(
                '设置时间步长、预热截止和输出。更多配置中包含重启、诊断及当前内核可写的输出变量。',
                'Set time steps, spinup and output. More settings include restart, diagnostics and output variables available in the current kernel.',
              )}
            >
              <h3 className="simulation-subheading">{t('时间与预热', 'Time and spinup')}</h3>
              <div className="form-grid">
                {timeBasic.filter((f) => !f.name.endsWith('spinup_repeat')).map(render)}
                {renderDate('spinup')}
                {timeBasic.filter((f) => f.name.endsWith('spinup_repeat')).map(render)}
              </div>
              <h3 className="simulation-subheading">{t('输出设置', 'Output settings')}</h3>
              <div className="form-grid">{outputBasic.map(render)}</div>
              <p className="field-hint">
                {t(
                  '预热截止是实际日历日期，留空关闭预热；重复次数只作用于预热时段。',
                  'The spinup cutoff is a calendar date; blank disables spinup. Repetitions apply only to spinup.',
                )}
              </p>
              {more()}
            </Card>
          )}
          {step < 5 && (
            <footer className="simulation-footer">
              {step > 1 ? (
                <Button
                  disabled={editingDisabled}
                  onClick={() => setStep((step - 1) as SimulationStep)}
                >
                  {t('上一步', 'Back')}
                </Button>
              ) : (
                <span />
              )}
              <Button
                variant="primary"
                disabled={editingDisabled || !schema.length}
                onClick={() => {
                  if (dirty) void saveAndContinue().catch(() => {});
                  else
                    try {
                      next();
                    } catch {
                      /* The inline message describes the invalid period. */
                    }
                }}
              >
                {dirty ? t('保存并继续', 'Save and continue') : t('下一步', 'Next')} →
              </Button>
            </footer>
          )}
          <div hidden={step !== 5}>
            <RunPanel
              key={dir}
              active={step === 5}
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
            <Button
              variant="primary"
              disabled={editingDisabled}
              onClick={() => void save().catch(() => {})}
            >
              {t('保存设置', 'Save settings')}
            </Button>
          )}
        </>
      )}
      {state.caseTab === 'runs' && <HistoryPage casePath={dir} />}{' '}
      {state.caseTab === 'results' && (
        <ResultsPage casePath={dir} onBack={() => update({ caseTab: 'settings' })} />
      )}
    </div>
  );
}
