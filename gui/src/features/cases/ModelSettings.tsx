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
import type { CaseOptionPatch } from '../versions/configuration-options';
import {
  filterModelParameters,
  processSettingOwner,
  modelParameterEditable,
  simulationMode,
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
  const selectedMode = mode ?? simulationMode(version?.overlay, facts, profile?.subgrid),
    vegetationMode = selectedMode === 'PFT' || selectedMode === 'PC';
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
            <small>
              {value === 'PFT' || value === 'PC'
                ? t('需匹配站点植被数据', 'Requires matching vegetation data')
                : state.kernels.some((item) => item.macros.includes(`LULC_${value}`))
                  ? t('本机已有此分类内核', 'A kernel for this classification is installed')
                  : t('本机尚无此分类内核', 'No local kernel for this classification')}
            </small>
          </Button>
        ))}
      </div>
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
            hasSettings={(id) =>
              fields.some(
                (field) =>
                  processSettingOwner(field) === id &&
                  (!setupHandledKeys.includes(field.name) ||
                    field.name === 'DEF_TUNING_CROP_PLANTING_DAY'),
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
                excludeKeys={setupHandledKeys}
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
    soil: ['土壤设置', 'Soil settings'],
    DEF_USE_VariablySaturatedFlow: ['变饱和流设置', 'Variably saturated flow settings'],
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
  const close = () => {
    if (!savingNow.current) onClose();
  };
  const save = async () => {
    if (disabled || lockedNow.current || !ready || savingNow.current || !live.current)
      throw new Error('Configuration is locked');
    savingNow.current = true;
    setSaving(true);
    try {
      if (Object.keys(draft.current).length) await onSave(draft.current);
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
          environment?.domain === 'site' ? 'site' : environment?.domain ? 'spatial' : 'unknown'
        }
        subgrid={subgrid}
        caseValues={values}
        removedCaseKeys={Object.keys(patch).filter((key) => patch[key] === null)}
        disabled={disabled || saving}
        excludeKeys={setupHandledKeys.filter((key) => key !== 'DEF_TUNING_CROP_PLANTING_DAY')}
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
