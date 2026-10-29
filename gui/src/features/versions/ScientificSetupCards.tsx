import { bilingual } from '../../lib/bilingual';
import { useEffect, useRef, useState } from 'react';
import { Button, Card, Field, Input, Notice } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField } from '../../lib/guide';
import { ActionButton } from '../../components/ActionButton';
import type { Overlay } from '../../lib/types';
import type { ModelField } from './model';
import { modelSettingValue } from '../cases/model-settings';
import {
  caseOptionOverlay,
  caseOptionPatch,
  type CaseOptionPatch,
  type ConfigurationField,
  type ConfigurationSource,
  type ConfigurationResult,
} from './configuration-options';
import {
  applySetupRequest,
  setupChoices,
  type SetupEnvironment,
  type SetupRequest,
} from './scientific-setup';
type Props = {
  caseDir?: string | null;
  source: ConfigurationSource;
  overlay: Overlay;
  fields: ModelField[];
  environment: SetupEnvironment;
  includeSubgrid?: boolean;
  disabled?: boolean;
  onChange?: (overlay: Overlay) => void;
  onCaseChange?: (patch: CaseOptionPatch) => void | Promise<void>;
  requested?: SetupRequest;
  onOpenSettings?: (id: string) => void;
  hasSettings?: (id: string, states?: ConfigurationField[]) => boolean;
};
export default function ScientificSetupCards({
  caseDir = null,
  source,
  overlay,
  fields,
  environment,
  includeSubgrid = true,
  disabled = false,
  onChange,
  onCaseChange,
  requested,
  onOpenSettings,
  hasSettings = () => true,
}: Props) {
  const { state } = useApp(),
    t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  // 只影响选项判定，不进入查询键：开关算例写入不应让后端查询结果过期。
  const choiceEnvironment = onCaseChange ? { ...environment, autoClose: true } : environment;
  const [loaded, setLoaded] = useState<{ key: string; result: ConfigurationResult } | null>(null),
    [error, setError] = useState(''),
    [cropInput, setCropInput] = useState<{ key: string; value: string } | null>(null),
    [saving, setSaving] = useState(false);
  const key = JSON.stringify({
      source: source.kind === 'draft' ? { ...source, overlay } : source,
      environment,
      caseDir,
    }),
    current = useRef(key),
    writeLocked = useRef(true);
  current.current = key;
  const identity = JSON.stringify({
    caseDir,
    source: source.kind === 'frozen' ? source : { id: source.id, generation: source.generation },
  });
  const currentIdentity = useRef(identity);
  currentIdentity.current = identity;
  useEffect(() => {
    let live = true;
    const data = JSON.parse(key) as {
      source: ConfigurationSource;
      environment: SetupEnvironment;
      caseDir: string | null;
    };
    invokeJson<ConfigurationResult>('model_configuration', {
      source: data.source,
      caseDir: data.caseDir,
      kernelDir: data.environment.kernel?.dir ?? null,
      pftSelection: null,
      caseValues: data.environment.caseValues ?? null,
    })
      .then((result) => {
        if (live) {
          setLoaded({ key, result });
          setError('');
        }
      })
      .catch((e) => {
        if (live) setError(String(e));
      });
    return () => {
      live = false;
    };
  }, [key]);
  const result = loaded?.key === key ? loaded.result : null,
    busy = !result,
    locked =
      disabled || busy || saving || (!onCaseChange && (source.kind !== 'draft' || !onChange));
  const effective = onCaseChange
    ? caseOptionOverlay(overlay, fields, environment.caseValues, result?.fields)
    : overlay;
  writeLocked.current = locked;
  const choices = setupChoices(
    effective,
    fields,
    choiceEnvironment,
    result?.fields,
    result ?? undefined,
  ).filter((c) => includeSubgrid || c.id !== 'subgrid');
  const cropOpen =
    (requested?.id === 'crop' && requested.value === '.true.') ||
    choices.find((c) => c.id === 'crop')?.value === '.true.';
  const planting =
    cropInput?.key === key
      ? cropInput.value
      : (modelSettingValue(effective, 'DEF_TUNING_CROP_PLANTING_DAY') ?? '');
  const guideFields: UiField[] = [];
  function write(request: SetupRequest) {
    if (locked || writeLocked.current || current.current !== key)
      throw new Error(bilingual('配置不可修改或已切换', 'Configuration is locked or stale'));
    try {
      if (request.id === 'tracer' && request.value === '.true.' && onOpenSettings) {
        onOpenSettings(request.id);
        setError('');
        return;
      }
      const requestedValue =
        request.id === 'crop' && !['.true.', '.false.'].includes(request.value)
          ? '.true.'
          : request.value;
      const choice = choices.find((c) => c.id === request.id);
      const reason = choice?.choices.find((c) => c.value === requestedValue)?.reason;
      if (reason) throw new Error(reason);
      const previous = choice?.value;
      if (
        onCaseChange &&
        onOpenSettings &&
        request.value !== '.false.' &&
        previous === request.value &&
        !choice?.inherited &&
        hasSettings(request.id, result?.fields)
      ) {
        onOpenSettings(request.id);
        return;
      }
      const next = applySetupRequest(
        effective,
        fields,
        request,
        choiceEnvironment,
        result ?? undefined,
      );
      if (onCaseChange) {
        writeLocked.current = true;
        setSaving(true);
        return Promise.resolve(onCaseChange(caseOptionPatch(effective, next, fields)))
          .then(() => {
            setError('');
            if (
              currentIdentity.current === identity &&
              request.value !== '.false.' &&
              (previous !== requestedValue || choice?.inherited) &&
              hasSettings(request.id, result?.fields)
            )
              onOpenSettings?.(request.id);
          })
          .catch((e) => {
            setError(String(e));
            throw e;
          })
          .finally(() => {
            setSaving(false);
            writeLocked.current = false;
          });
      } else onChange!(next);
      setError('');
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }
  function invoke(request: SetupRequest) {
    try {
      void Promise.resolve(write(request)).catch(() => {});
    } catch {
      /* Error remains visible. */
    }
  }
  const cards = choices.map((control, index) => {
    const tracerSetup = control.id === 'tracer' && !!onOpenSettings;
    const settingsAvailable =
      !!onOpenSettings &&
      hasSettings(control.id, result?.fields) &&
      (tracerSetup ||
        (!!control.value &&
          control.value !== '.false.' &&
          !control.choices.find((choice) => choice.value === control.value)?.reason));
    const label = `${index + (includeSubgrid ? 1 : 2)}. ${t(control.zh, control.en)}`;
    const available = control.choices.filter(
      (c) => !c.reason || (tracerSetup && c.value === '.true.'),
    );
    guideFields.push({
      id: `setup-${control.id}`,
      label,
      value: control.value,
      type: 'choice',
      options: available.map((c) => ({
        value: c.value,
        label:
          tracerSetup && c.value === '.true.' ? t('选择示踪剂', 'Choose tracers') : t(c.zh, c.en),
      })),
      disabled: locked || !available.length,
      set: (value) => write({ id: control.id, value }),
    });
    return (
      <Card
        key={control.id}
        title={label}
        description={
          control.value
            ? (control.inherited ? t('默认', 'Default') : t('已选择', 'Selected')) +
              ': ' +
              t(
                control.choices.find((c) => c.value === control.value)?.zh ?? control.value,
                control.choices.find((c) => c.value === control.value)?.en ?? control.value,
              )
            : t('未指定，请按前提选择', 'Not specified; choose after prerequisites')
        }
      >
        <div className="form-grid">
          {control.choices.map((choice) => (
            <Button
              key={choice.value}
              className="simulation-choice-card"
              variant={control.value === choice.value ? 'primary' : 'secondary'}
              aria-pressed={control.value === choice.value}
              disabled={locked || (!!choice.reason && !(tracerSetup && choice.value === '.true.'))}
              onClick={() => invoke({ id: control.id, value: choice.value })}
            >
              <strong>
                {tracerSetup && choice.value === '.true.'
                  ? t('选择示踪剂', 'Choose tracers')
                  : t(choice.zh, choice.en)}
              </strong>
              {choice.reason && <small>{choice.reason}</small>}
            </Button>
          ))}
        </div>
        {settingsAvailable && !tracerSetup && (
          <ActionButton
            action={`setup-${control.id}-settings`}
            label={t(
              `${t(control.zh, control.en)}：设置`,
              `${t(control.zh, control.en)}: settings`,
            )}
            disabled={locked}
            run={() => write({ id: control.id, value: control.value! })}
          >
            {t('设置', 'Settings')}
          </ActionButton>
        )}
        {control.pending && result?.context_complete && (
          <p className="muted">
            {t('这一项的前提还在核对。', 'The prerequisites of this item are still being checked.')}
          </p>
        )}
        {control.id === 'soil' && (
          <p className="muted">
            {control.value === 'Campbell'
              ? t(
                  'Campbell 的 b、ψs 来自逐层土壤数据。',
                  'Campbell b and ψs come from soil-layer data.',
                )
              : t(
                  'van Genuchten 的 α、n、L、θr 来自逐层土壤数据。',
                  'van Genuchten α, n, L and θr come from soil-layer data.',
                )}
            {t(
              '共用选项在下方“土壤方案”中设置。',
              'Shared options are configured under Soil schemes below.',
            )}
          </p>
        )}
        {control.id === 'crop' && (
          <p className="muted">
            {environment.cropKernel?.macros.includes('CROP')
              ? t(
                  '作物过程编译支持已就绪；开启时使用标准算例默认配置，种植日可随后调整。',
                  'Crop support is available. Enabling uses standard case defaults; the planting day remains adjustable.',
                )
              : t(
                  '尚未确认当前编译配置支持作物过程。',
                  'Crop support has not been confirmed for the current build configuration.',
                )}
          </p>
        )}
        {control.id === 'crop' && cropOpen && !onOpenSettings && (
          <Field
            label={t('种植日（单点 0–366，空间模拟 0）', 'Planting day (site 0–366, spatial 0)')}
          >
            <Input
              value={planting}
              disabled={locked}
              aria-label={t('种植日', 'Planting day')}
              onChange={(e) => setCropInput({ key, value: e.target.value })}
            />
            <ActionButton
              action="setup-crop-apply"
              label={t('应用种植日', 'Apply planting day')}
              disabled={locked || !planting.trim()}
              run={() => write({ id: 'crop', value: planting })}
            />
          </Field>
        )}
      </Card>
    );
  });
  if (cropOpen && !onOpenSettings) {
    guideFields.push({
      id: 'setup-crop-planting-day',
      label: t('种植日', 'Planting day'),
      value: planting,
      type: 'text',
      disabled: locked,
      set: (value) => {
        if (locked || writeLocked.current || current.current !== key)
          throw new Error('Configuration is locked');
        setCropInput({ key, value });
      },
    });
  }
  usePageContract({
    title: t('按顺序配置模型结构', 'Configure model structure in order'),
    step: 'scientific-setup',
    fields: guideFields,
    // 按钮自己登记（第 737 轮）。示踪剂的"设置"原来在清单里另有一条，界面上它是选项卡片本身。
    actions: [],
    status:
      error ||
      (busy ? 'loading' : result?.context_complete ? 'ready' : 'pending_source_verification'),
  });
  return (
    <div className="scientific-setup">
      {error && <Notice tone="error">{error}</Notice>}
      {onCaseChange && (
        <p>
          {t(
            '方案与过程设置直接保存在当前算例中。',
            'Schemes and process settings are saved directly in this case.',
          )}
        </p>
      )}
      {!busy && result && !result.context_complete && (
        <Notice tone="warning">
          {t(
            '还没核对完当前版本的源码和运行环境，下面的选项先按已知的前提显示。',
            'The source and run environment of this version are not fully checked yet; options below follow the known prerequisites.',
          )}
        </Notice>
      )}
      {cards}
    </div>
  );
}
