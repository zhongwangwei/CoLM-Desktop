import { useEffect, useRef, useState } from 'react';
import { Advanced, Button, Card, Dialog, Field, Input, Notice, Select } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { invoke } from '../../lib/ipc';
import { usePageContract, type UiAction, type UiField } from '../../lib/guide';
import type { FrozenRevision, KernelEntry } from '../../lib/types';
import ConfigurationOptions from '../versions/ConfigurationOptions';
import ScientificSetupCards from '../versions/ScientificSetupCards';
import { setupHandledKeys, type SetupEnvironment } from '../versions/scientific-setup';
import { isCaseField, type SchemaField } from './case-model';
import {
  caseFieldVisible,
  cnInitialStatePatch,
  type CaseOptionPatch,
} from '../versions/configuration-options';
import {
  filterModelParameters,
  modelSettingValue,
  processSettingOwner,
  modelParameterEditable,
  schemeFlagKeys,
  schemeHasSettings,
  settingAvailable,
  simulationMode,
  spatialOnlySchemeKeys,
  type SimulationMode,
  type ModelParameter,
} from './model-settings';
import { parameterKey } from '../versions/model';
type Props = {
  setupEnvironment?: SetupEnvironment;
  onCaseChange?: (patch: CaseOptionPatch) => void | Promise<void>;
  version: FrozenRevision | null;
  onEdit: () => void;
  disabled?: boolean;
  classification?: 'IGBP' | 'USGS';
  mode?: SimulationMode;
  onModeChange?: (value: SimulationMode) => void;
  onClassificationChange?: (value: 'IGBP' | 'USGS') => void;
  profile?: {
    subgrid?: string;
    soil?: string;
    bgc?: boolean;
    urban?: boolean;
    crop?: boolean;
    [key: string]: unknown;
  };
  kernel?: KernelEntry;
  caseDir?: string | null;
};
export default function ModelSettings({
  version,
  onEdit,
  disabled = false,
  classification,
  onClassificationChange,
  mode,
  onModeChange,
  profile,
  kernel,
  caseDir = null,
  setupEnvironment,
  onCaseChange,
}: Props) {
  const { state } = useApp(),
    en = state.language === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const [fields, setFields] = useState<SchemaField[]>([]),
    [catalog, setCatalog] = useState<ModelParameter[]>([]),
    [error, setError] = useState(''),
    [loading, setLoading] = useState(true),
    [catalogOpen, setCatalogOpen] = useState(false),
    [query, setQuery] = useState(''),
    [scope, setScope] = useState(''),
    [modelFields, setModelFields] = useState<SchemaField[]>([]),
    [settings, setSettings] = useState<{ id: string; identity: string } | null>(null);
  const identity = JSON.stringify({
    caseDir,
    version: version?.version,
    mode,
    kernel: kernel?.dir,
    domain: setupEnvironment?.domain,
  });
  const currentIdentity = useRef(identity);
  currentIdentity.current = identity;
  useEffect(() => {
    let live = true;
    Promise.all([
      invoke<SchemaField[]>('model_fields', {}),
      invoke<SchemaField[]>('describe_fields', {}),
      invoke<ModelParameter[]>('parameter_catalog', {}),
    ])
      .then(([models, all, parameters]) => {
        if (live) {
          setModelFields(models);
          setFields(
            all
              .filter(isCaseField)
              .filter(
                (field) =>
                  !models.some((model) => model.name.toLowerCase() === field.name.toLowerCase()),
              ),
          );
          setCatalog(parameters);
        }
      })
      .catch((e) => {
        if (live) setError(String(e));
      })
      .finally(() => {
        if (live) setLoading(false);
      });
    return () => {
      live = false;
    };
  }, []);
  const ready = !!version && !loading && !error,
    scopes = [...new Set(catalog.map((item) => item.scope))].sort(),
    filtered = filterModelParameters(catalog, query, scope);
  const facts =
    classification ??
    (profile?.subgrid === 'USGS' ? 'USGS' : profile?.subgrid === 'IGBP' ? 'IGBP' : undefined);
  const selectedMode =
      onModeChange && !caseDir
        ? mode
        : (mode ?? simulationMode(version?.overlay, facts, profile?.subgrid)),
    vegetationMode = selectedMode === 'PFT' || selectedMode === 'PC';
  // 方案切换后组件身份随之变化；等新方案渲染出来再打开它的专属弹窗。
  const [pendingScheme, setPendingScheme] = useState<SimulationMode | null>(null);
  const schemeSettings = (value: SimulationMode | undefined) =>
    schemeHasSettings(
      value,
      fields,
      setupEnvironment?.domain ?? (caseDir ? (profile?.grid ? 'region' : 'site') : null),
    );
  const pendingReady = !!pendingScheme && selectedMode === pendingScheme && !loading,
    pendingOpens = pendingReady && schemeSettings(pendingScheme);
  useEffect(() => {
    if (!pendingReady || !pendingScheme) return;
    setPendingScheme(null);
    if (pendingOpens) setSettings({ id: pendingScheme, identity: currentIdentity.current });
  }, [pendingReady, pendingOpens, pendingScheme]);
  // 卡片上说明当前方案的专属状态；未写入的开关按 CoLM 默认显示（DEF_FAST_PC 默认开启）。
  const caseValue = (key: string) =>
    setupEnvironment?.caseValues?.[key]?.trim().toLowerCase() ??
    modelSettingValue(version?.overlay ?? { physics: {}, parameters: [], process_files: [] }, key)
      ?.trim()
      .toLowerCase();
  const schemeDetail = (value: SimulationMode) => {
    const site = ['site', 'single'].includes(setupEnvironment?.domain ?? '');
    if (selectedMode === value && value === 'PC' && !site)
      return caseValue('DEF_FAST_PC') === '.false.'
        ? t('标准植物群落 · 可在专属设置中修改', 'Standard PC · adjustable in settings')
        : t('快速植物群落（Fast PC）· 可在专属设置中修改', 'Fast PC · adjustable in settings');
    if (selectedMode === value && value === 'PFT' && !site)
      return caseValue('DEF_SOLO_PFT') === '.true.'
        ? t('单一 PFT 模式 · 可在专属设置中修改', 'Solo PFT · adjustable in settings')
        : t('多 PFT 镶嵌 · 可在专属设置中修改', 'Mosaic PFT · adjustable in settings');
    if (value === 'PFT' || value === 'PC')
      return t('需匹配站点植被数据', 'Requires matching vegetation data');
    return state.kernels.some((item) => item.macros.includes(`LULC_${value}`))
      ? t('本机已有此分类内核 · 点选即生效', 'Kernel installed · applies on click')
      : t('本机尚无此分类内核', 'No local kernel for this classification');
  };
  const modeActionId = (value: SimulationMode) =>
    value === 'PFT' || value === 'PC'
      ? `simulation-mode-${value.toLowerCase()}`
      : vegetationMode
        ? `simulation-mode-${value.toLowerCase()}`
        : `simulation-classification-${value.toLowerCase()}`;
  const modeAvailable = (value: SimulationMode) =>
    !!onModeChange || ((value === 'IGBP' || value === 'USGS') && !!onClassificationChange);
  const changeMode = (value: SimulationMode) => {
    if (disabled || !modeAvailable(value)) return;
    if (onCaseChange && value !== selectedMode && schemeSettings(value)) setPendingScheme(value);
    if (onModeChange) onModeChange(value);
    else if (value === 'IGBP' || value === 'USGS') onClassificationChange?.(value);
  };
  const actions: UiAction[] = [
    {
      id: 'simulation-model-edit',
      label: t('研究与参数调整', 'Research and parameter adjustment'),
      kind: 'click',
      enabled: !disabled && !!version,
      run: onEdit,
    },
  ];
  for (const value of ['IGBP', 'USGS', 'PFT', 'PC'] as const) {
    if (modeAvailable(value))
      actions.push({
        id: modeActionId(value),
        label: value,
        kind: 'click',
        enabled: !disabled,
        run: () => changeMode(value),
      });
  }
  if (vegetationMode && onClassificationChange)
    for (const value of ['IGBP', 'USGS'] as const)
      actions.push({
        id: `simulation-classification-${value.toLowerCase()}`,
        label: value,
        kind: 'click',
        enabled: !disabled,
        run: () => onClassificationChange(value),
      });
  if (onCaseChange && vegetationMode && schemeSettings(selectedMode))
    actions.push({
      id: 'simulation-scheme-settings',
      label: t(`${selectedMode} 专属设置`, `${selectedMode} settings`),
      kind: 'click',
      enabled: !disabled && ready,
      run: () => setSettings({ id: selectedMode!, identity: currentIdentity.current }),
    });
  if (ready)
    actions.push({
      id: 'simulation-parameter-catalog-toggle',
      label: t('完整参数目录', 'Full parameter catalog'),
      kind: 'click',
      enabled: !disabled,
      run: () => setCatalogOpen(!catalogOpen),
    });
  const guideFields: UiField[] =
    ready && catalogOpen
      ? [
          {
            id: 'simulation-parameter-query',
            label: t('搜索参数', 'Search parameters'),
            value: query,
            type: 'text',
            disabled,
            set: setQuery,
          },
          {
            id: 'simulation-parameter-scope',
            label: t('参数作用域', 'Parameter scope'),
            value: scope,
            type: 'choice',
            disabled,
            options: [
              { value: '', label: t('全部作用域', 'All scopes') },
              ...scopes.map((value) => ({ value, label: value })),
            ],
            set: setScope,
          },
        ]
      : [];
  usePageContract({
    title: t('算例方案', 'Case settings'),
    step: 'simulation-model',
    fields: guideFields,
    actions,
    status: error || (loading ? 'loading' : 'ready'),
    details: {
      version: version?.version,
      classification: facts ?? null,
      simulation_mode: selectedMode ?? null,
      applicability: 'pending_final_check',
      model_fields: fields.length,
      parameter_catalog: catalog.length,
    },
  });

  return (
    <Card
      title={t('算例方案与模型参数', 'Case schemes and model parameters')}
      description={t(
        '先确认基础结构，再检查各过程方案与参数。',
        'Confirm the basic structure, then review process schemes and parameters.',
      )}
    >
      <h3>{t('地表覆盖与植被方案', 'Land cover and vegetation scheme')}</h3>
      <p className="muted">
        {t(
          'IGBP / USGS 使用地类结构；PFT / PC 使用植被结构。选择 PFT / PC 后仍需确认地类数据来源。',
          'IGBP / USGS use land-cover classes; PFT / PC use vegetation structures. PFT / PC also require a land-cover data classification.',
        )}
      </p>
      <div className="form-grid">
        {(['IGBP', 'USGS', 'PFT', 'PC'] as const).map((value) => (
          <Button
            key={value}
            className="simulation-choice-card"
            aria-pressed={selectedMode === value}
            variant={selectedMode === value ? 'primary' : 'secondary'}
            disabled={disabled || !modeAvailable(value)}
            onClick={() => changeMode(value)}
          >
            <strong>{value}</strong>
            <span>
              {value === 'IGBP'
                ? t('17 类地表覆盖', '17 land-cover classes')
                : value === 'USGS'
                  ? t('24 类地表覆盖', '24 land-cover classes')
                  : value === 'PFT'
                    ? t('植被功能型结构', 'Plant functional types')
                    : t('植物群落结构', 'Plant communities')}
            </span>
            <small>{schemeDetail(value)}</small>
          </Button>
        ))}
      </div>
      {!selectedMode && (
        <Notice tone="warning">
          {t(
            '请先选择一种下垫面 / 植被方案；未选择前不能进入下一步。',
            'Choose a land-cover / vegetation scheme before continuing.',
          )}
        </Notice>
      )}
      {onCaseChange && vegetationMode && schemeSettings(selectedMode) && (
        <div className="button-row">
          <Button
            disabled={disabled || loading}
            onClick={() => setSettings({ id: selectedMode!, identity: currentIdentity.current })}
          >
            {t(`${selectedMode} 专属设置`, `${selectedMode} settings`)}
          </Button>
        </div>
      )}
      {vegetationMode && (
        <Field label={t('地类数据来源', 'Land-cover data classification')}>
          <div className="button-row">
            {(['IGBP', 'USGS'] as const).map((value) => (
              <Button
                key={value}
                aria-pressed={facts === value}
                variant={facts === value ? 'primary' : 'secondary'}
                disabled={disabled || !onClassificationChange}
                onClick={() => onClassificationChange?.(value)}
              >
                {value}
              </Button>
            ))}
          </div>
          {!onClassificationChange && (
            <p className="muted">
              {caseDir
                ? t(
                    '保留算例现有的地类数据来源。',
                    'The existing case classification is preserved.',
                  )
                : t(
                    '新建 PFT / PC 算例使用 IGBP 地类数据。',
                    'New PFT / PC cases use IGBP land-cover data.',
                  )}
            </p>
          )}
        </Field>
      )}
      <p className="muted">
        {t(
          '四种方案均保存在算例中；选择后无需派生模型版本。地类数据来源决定候选内核，输入数据仍需匹配所选方案。',
          'All four schemes are case settings; selecting one requires no model revision. Land-cover classification selects candidate kernels, and input data must match the scheme.',
        )}
        {!selectedMode && t(' 当前方案尚未确认。', ' Current scheme is unconfirmed.')}
      </p>
      {version ? (
        <>
          <div className="button-row">
            <strong>
              {version.name} · r{version.version.revision}
            </strong>
            <Button disabled={disabled} onClick={onEdit}>
              {t('研究与参数调整', 'Research and parameter adjustment')}
            </Button>
          </div>
          <Notice>
            {t(
              '目录默认值仅供参考。实际生效值、输入与引擎适用性在运行检查中确认。',
              'Catalog defaults are reference values. Effective values, inputs and engine applicability are confirmed by run checks.',
            )}
          </Notice>
          <ScientificSetupCards
            source={{ kind: 'frozen', version: version.version }}
            overlay={version.overlay}
            fields={fields}
            environment={{
              ...setupEnvironment,
              classification: facts,
              kernel,
              subgrid: selectedMode,
            }}
            caseDir={caseDir}
            includeSubgrid={false}
            disabled={disabled || loading}
            onCaseChange={onCaseChange}
            hasSettings={(id, states) =>
              // 土壤设置同时解释强制条件和参数来源，即使当前没有可编辑项也保留入口。
              id === 'soil' ||
              id === 'DEF_USE_TRACER' ||
              fields.some(
                (field) =>
                  processSettingOwner(field, states) === id &&
                  (!setupHandledKeys.includes(field.name) ||
                    field.name === 'DEF_TUNING_CROP_PLANTING_DAY') &&
                  settingAvailable(field.name, states) &&
                  (!states?.length ||
                    caseFieldVisible(
                      field.name,
                      states.map((state) =>
                        state.parent_gate?.toLowerCase() ===
                        (id === 'crop' ? 'def_use_crop' : id.toLowerCase())
                          ? { ...state, parent_gate_enabled: true }
                          : state,
                      ),
                    )),
              )
            }
            onOpenSettings={
              onCaseChange
                ? (id) => setSettings({ id, identity: currentIdentity.current })
                : undefined
            }
          />
          {error && <Notice tone="error">{error}</Notice>}
          {loading && (
            <p>{t('正在读取模型字段与参数目录…', 'Loading model fields and parameter catalog…')}</p>
          )}
          {ready && (
            <>
              <ConfigurationOptions
                subgrid={selectedMode}
                caseValues={setupEnvironment?.caseValues}
                onCaseChange={onCaseChange}
                excludeKeys={[...setupHandledKeys, ...schemeFlagKeys]}
                hideProcesses={!!onCaseChange}
                groupPrefix="simulation-model"
                source={{ kind: 'frozen', version: version.version }}
                overlay={version.overlay}
                fields={fields}
                catalog={catalog}
                caseDir={caseDir}
                kernelDir={kernel?.dir ?? null}
                kernelMacros={kernel?.macros}
                caseScope={
                  setupEnvironment?.domain === 'site'
                    ? 'site'
                    : setupEnvironment?.domain
                      ? 'spatial'
                      : 'unknown'
                }
                disabled={disabled}
              />
              <p className="muted">
                {t(
                  '开启过程后可在对应卡片的设置中调整初始化、输入数据和过程选项。',
                  'Use Settings on each enabled process to adjust initialization, inputs and process options.',
                )}
              </p>
              {settings?.identity === identity && onCaseChange && (
                <ProcessSettingsDialog
                  key={`${identity}:${settings.id}`}
                  process={settings.id}
                  fields={fields}
                  catalog={catalog}
                  version={version}
                  caseDir={caseDir}
                  kernel={kernel}
                  subgrid={selectedMode}
                  environment={setupEnvironment}
                  disabled={disabled}
                  onSave={onCaseChange}
                  onClose={() => setSettings(null)}
                />
              )}
              <Advanced
                title={`${t('完整参数目录', 'Full parameter catalog')} · ${catalog.length}`}
                open={catalogOpen}
                onOpenChange={(open) => {
                  if (!disabled) setCatalogOpen(open);
                }}
              >
                <div className="form-grid">
                  <Field label={t('搜索参数', 'Search parameters')}>
                    <Input
                      disabled={disabled}
                      value={query}
                      onChange={(event) => setQuery(event.target.value)}
                    />
                  </Field>
                  <Field label={t('参数作用域', 'Parameter scope')}>
                    <Select
                      disabled={disabled}
                      value={scope}
                      onChange={(event) => setScope(event.target.value)}
                    >
                      <option value="">{t('全部作用域', 'All scopes')}</option>
                      {scopes.map((value) => (
                        <option key={value}>{value}</option>
                      ))}
                    </Select>
                  </Field>
                </div>
                <p>
                  {filtered.length} / {catalog.length}
                </p>
                {filtered.map((parameter) => (
                  <div key={parameter.id} className="model-setting-row">
                    <strong>
                      {en ? parameter.label_en : parameter.label_zh} · {parameter.raw_key}
                    </strong>
                    <p>
                      {parameter.section} · {parameter.scope} · {parameter.unit ?? '—'}
                    </p>
                    <p>
                      {modelParameterEditable(parameter, modelFields)
                        ? t('可在版本草稿中调整', 'Editable in a version draft')
                        : `${t('本页仅查看；算例设置在相应步骤调整', 'Read here; case settings belong to their setup steps')} · ${parameter.visibility}`}
                    </p>
                    <p className="muted">
                      {t('启用条件', 'Activation')}:{' '}
                      {parameter.activation.join(', ') ||
                        t('目录未提供；待检查', 'Not specified; pending check')}{' '}
                      · {t('目录参考默认值', 'Catalog reference default')}:{' '}
                      {parameter.default ?? '—'}
                    </p>
                    <small>
                      {parameter.storage} · {parameter.id}
                      {parameter.source_location ? ` · ${parameter.source_location}` : ''}
                    </small>
                  </div>
                ))}
              </Advanced>
            </>
          )}
          <h3>{t('参数与资产来源', 'Parameter and asset sources')}</h3>
          <p>
            {t('源码内容哈希', 'Source content hash')}: <code>{version.code.source_sha256}</code>
          </p>
          <p>
            {t('参数覆盖', 'Parameter overrides')}: {version.overlay.parameters.length} ·{' '}
            {t('过程文件', 'Process files')}: {version.overlay.process_files.length}
          </p>
          {version.overlay.parameters.map((record) => (
            <p key={parameterKey(record)}>
              <code>
                {record.raw_key} = {record.value}
              </code>{' '}
              {record.unit ?? ''} · {record.scope_instance.kind}
              {record.scope_instance.scheme ? ` · ${record.scope_instance.scheme}` : ''}
              {record.scope_instance.index != null ? ` · ${record.scope_instance.index}` : ''}
              {record.scope_instance.process_file ? ` · ${record.scope_instance.process_file}` : ''}
            </p>
          ))}
          {version.overlay.process_files.map((asset) => (
            <p key={asset.relative_path}>
              {asset.relative_path} · <code>{asset.sha256}</code>
            </p>
          ))}
          {version.overlay.hybrid && (
            <p>
              {t('AI 网络配置', 'AI network configuration')}: {version.overlay.hybrid.config}
            </p>
          )}
        </>
      ) : (
        <Notice tone="warning">{t('请先选择模型版本。', 'Select a model version first.')}</Notice>
      )}
    </Card>
  );
}

function ProcessSettingsDialog({
  process,
  fields,
  catalog,
  version,
  caseDir,
  kernel,
  subgrid,
  environment,
  disabled,
  onSave,
  onClose,
}: {
  process: string;
  fields: SchemaField[];
  catalog: ModelParameter[];
  version: FrozenRevision;
  caseDir: string | null;
  kernel?: KernelEntry;
  subgrid?: SimulationMode;
  environment?: SetupEnvironment;
  disabled: boolean;
  onSave: (patch: CaseOptionPatch) => void | Promise<void>;
  onClose: () => void;
}) {
  const { state } = useApp(),
    t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  const [patch, setPatch] = useState<CaseOptionPatch>({}),
    [saving, setSaving] = useState(false),
    [ready, setReady] = useState(false),
    [error, setError] = useState('');
  const live = useRef(true),
    savingNow = useRef(false),
    draft = useRef<CaseOptionPatch>({}),
    lockedNow = useRef(disabled);
  lockedNow.current = disabled;
  const titles: Record<string, [string, string]> = {
    DEF_USE_BGC: ['BGC 设置', 'BGC settings'],
    DEF_USE_TRACER: ['示踪设置', 'Tracer settings'],
    DEF_USE_GridRiverLakeFlow: ['河湖流与水库设置', 'River and reservoir settings'],
    DEF_UnitCatchment_regional: ['区域汇流设置', 'Regional routing settings'],
    DEF_URBAN_RUN: ['城市设置', 'Urban settings'],
    crop: ['作物设置', 'Crop settings'],
    DEF_USE_LULCC: ['土地覆盖变化设置', 'Land-cover change settings'],
    soil: ['土壤水力方案设置', 'Soil hydraulic settings'],
    DEF_USE_VariablySaturatedFlow: ['变饱和流设置', 'Variably saturated flow settings'],
    PFT: ['PFT 植被功能型设置', 'PFT settings'],
    PC: ['PC 植物群落设置', 'PC settings'],
  };
  const title = titles[process] ?? ['过程设置', 'Process settings'];
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  const values = { ...environment?.caseValues };
  for (const [key, value] of Object.entries(patch)) {
    if (value === null) delete values[key];
    else values[key] = value;
  }
  const cnDefaults =
    process === 'DEF_USE_BGC' ? cnInitialStatePatch(version.overlay, values, patch) : {};
  Object.assign(values, cnDefaults);
  const close = () => {
    if (!savingNow.current) onClose();
  };
  const save = async () => {
    if (disabled || lockedNow.current || !ready || savingNow.current || !live.current)
      throw new Error('Configuration is locked');
    savingNow.current = true;
    setSaving(true);
    try {
      const changes = { ...draft.current };
      if (process === 'DEF_USE_BGC')
        Object.assign(
          changes,
          cnInitialStatePatch(version.overlay, environment?.caseValues ?? {}, changes),
        );
      if (Object.keys(changes).length) await onSave(changes);
      if (live.current) onClose();
    } catch (e) {
      if (live.current) setError(String(e));
    } finally {
      if (live.current) {
        savingNow.current = false;
        setSaving(false);
      }
    }
  };
  const actions: UiAction[] = [
    {
      id: 'process-settings-save',
      label: t('保存设置', 'Save settings'),
      kind: 'click',
      enabled: ready && !disabled && !saving,
      run: save,
    },
    {
      id: 'process-settings-cancel',
      label: t('取消', 'Cancel'),
      kind: 'click',
      enabled: !saving,
      run: close,
    },
  ];
  return (
    <Dialog
      title={t(...title)}
      showClose={false}
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
      description={t(
        '修改暂存在此窗口，保存后生效。取消不会更改已开启的过程。',
        'Changes stay in this window until saved. Cancel keeps the enabled process unchanged.',
      )}
    >
      {error && <Notice tone="error">{error}</Notice>}
      {process === 'DEF_USE_BGC' && (
        <Notice>
          {t(
            '碳氮初始场默认使用运行数据目录中的 cnsteadystate.nc；启用后会保存完整路径。也可指定其他已有文件。请先设置运行数据目录或填写文件路径。',
            'The carbon/nitrogen initial state defaults to cnsteadystate.nc in the runtime data directory. Enabling it saves the resolved path; an existing custom file can also be used. Set the runtime directory or enter a file path.',
          )}
        </Notice>
      )}
      {process === 'crop' && (
        <Notice>
          <p>
            {t(
              '播种日为 0 时读取运行数据；单点新建算例预设为第 120 天，可自行修改。空间模拟使用运行数据中的播种日期。施肥、固氮和灌溉以当前算例设置为准。',
              'Planting day 0 reads runtime data. New single-point cases start at day 120 and can be edited. Spatial simulations use planting dates from runtime data. Fertilization, nitrogen fixation and irrigation follow the current case settings.',
            )}
          </p>
          <p>
            {t(
              '作物文件位于运行数据目录的 crop/ 中：播种日期 plantdt-colm-64cfts-rice2_fillcoast.nc；施肥源 1 为 fertnitro_fillcoast.nc，源 2 为 fertilizer_2015soc.nc；灌溉方式为 surfdata_irrigation_method_96x144.nc，分配方案 3 另需 surfdata_irrigation_allocation.nc。仅启用对应过程时才需要相关文件。',
              'Crop files are under runtime/crop/: planting dates in plantdt-colm-64cfts-rice2_fillcoast.nc; fertilizer source 1 in fertnitro_fillcoast.nc and source 2 in fertilizer_2015soc.nc; irrigation methods in surfdata_irrigation_method_96x144.nc, with surfdata_irrigation_allocation.nc also needed for allocation scheme 3. Related files are required only when their processes are enabled.',
            )}
          </p>
        </Notice>
      )}
      {process === 'DEF_USE_LULCC' && (
        <Notice>
          {t(
            '土地覆盖变化仅支持空间模拟，要求月尺度、逐年变化的 LAI，并关闭区域单元汇流。年度土地覆盖文件从地表数据目录读取。LAI 起止年控制 LAI 数据读取范围，不代表土地覆盖数据也覆盖这些年份；土地覆盖数据与模拟时段仍需匹配。',
            'Land-cover change requires a spatial simulation, monthly LAI changing by year, and regional unit routing off. Annual land-cover files come from the land-data directory. LAI start/end years control LAI reads and do not guarantee land-cover data coverage; land-cover inputs must also match the simulation period.',
          )}
        </Notice>
      )}
      {process === 'soil' && (
        <Notice>
          <p>
            {t(
              'Campbell 的 b、ψs 和 van Genuchten 的 α、n、L、θr 等曲线参数来自逐层土壤数据；此处只调整关联选项。',
              'Campbell b and ψs, and van Genuchten α, n, L and θr come from soil data for each layer; this window edits related options only.',
            )}
          </p>
          <p>
            {t(
              'Campbell 的变饱和流可在对应过程卡片中调整；van Genuchten 始终启用变饱和流。土壤表面阻抗在下方“土壤方案”中设置。土壤参数升尺度拟合仅用于区域地表数据构建，单点不使用。',
              'For Campbell, adjust variably saturated flow on its process card; van Genuchten always enables it. Soil-surface resistance is configured below under Soil schemes. Soil-parameter upscaling fit is used only when building spatial surface data, not at single points.',
            )}
          </p>
        </Notice>
      )}
      <ConfigurationOptions
        process={process}
        modalActions={actions}
        onReady={setReady}
        source={{ kind: 'frozen', version: version.version }}
        overlay={version.overlay}
        fields={fields}
        catalog={catalog}
        caseDir={caseDir}
        kernelDir={kernel?.dir}
        kernelMacros={kernel?.macros}
        caseScope={
          ['site', 'single'].includes(environment?.domain ?? '')
            ? 'site'
            : environment?.domain
              ? 'spatial'
              : 'unknown'
        }
        subgrid={subgrid}
        caseValues={values}
        removedCaseKeys={Object.keys(patch).filter(
          (key) => patch[key] === null && !(key in cnDefaults),
        )}
        disabled={disabled || saving}
        excludeKeys={[
          ...setupHandledKeys.filter((key) => key !== 'DEF_TUNING_CROP_PLANTING_DAY'),
          // SinglePoint 不读这些方案项；单点弹窗里出现只会制造“已生效”的假象。
          ...(['site', 'single'].includes(environment?.domain ?? '')
            ? [...spatialOnlySchemeKeys, 'DEF_USE_SOILPAR_UPS_FIT']
            : []),
        ]}
        onCaseChange={(change) => {
          if (!live.current || lockedNow.current || savingNow.current)
            throw new Error('Configuration is locked');
          draft.current = { ...draft.current, ...change };
          setPatch(draft.current);
        }}
      />
      <div className="button-row">
        <Button
          disabled={!ready || disabled || saving}
          onClick={() => {
            void save();
          }}
        >
          {t('保存设置', 'Save settings')}
        </Button>
        <Button disabled={saving} onClick={close}>
          {t('取消', 'Cancel')}
        </Button>
      </div>
    </Dialog>
  );
}
