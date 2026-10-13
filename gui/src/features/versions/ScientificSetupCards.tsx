import { useEffect, useRef, useState } from 'react';
import { Button, Card, Field, Input, Notice } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import type { Overlay } from '../../lib/types';
import type { ModelField } from './model';
import { modelSettingValue } from '../cases/model-settings';
import type { ConfigurationSource, ConfigurationResult } from './configuration-options';
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
  onRequest?: (request: SetupRequest) => void;
  requested?: SetupRequest;
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
  onRequest,
  requested,
}: Props) {
  const { state } = useApp(),
    t = (zh: string, en: string) => (state.language === 'en' ? en : zh);
  const [loaded, setLoaded] = useState<{ key: string; result: ConfigurationResult } | null>(null),
    [error, setError] = useState(''),
    [cropInput, setCropInput] = useState<{ key: string; value: string } | null>(null),
    [cropRequested, setCropRequested] = useState<string | null>(null);
  const key = JSON.stringify({
      source: source.kind === 'draft' ? { ...source, overlay } : source,
      environment,
      caseDir,
    }),
    current = useRef(key),
    writeLocked = useRef(true);
  current.current = key;
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
      caseValues: null,
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
    locked = disabled || busy || (source.kind === 'draft' ? !onChange : !onRequest);
  writeLocked.current = locked;
  const choices = setupChoices(
    overlay,
    fields,
    environment,
    result?.fields,
    result ?? undefined,
  ).filter((c) => includeSubgrid || c.id !== 'subgrid');
  const cropOpen =
    cropRequested === key ||
    (requested?.id === 'crop' && requested.value === '.true.') ||
    choices.find((c) => c.id === 'crop')?.value === '.true.';
  const planting =
    cropInput?.key === key
      ? cropInput.value
      : (modelSettingValue(overlay, 'DEF_TUNING_CROP_PLANTING_DAY') ?? '');
  const guideFields: UiField[] = [],
    actions: UiAction[] = [];
  function write(request: SetupRequest) {
    if (locked || writeLocked.current || current.current !== key)
      throw new Error('配置不可修改或已切换 / Configuration is locked or stale');
    try {
      const requestedValue =
        request.id === 'crop' && !['.true.', '.false.'].includes(request.value)
          ? '.true.'
          : request.value;
      const reason = choices
        .find((c) => c.id === request.id)
        ?.choices.find((c) => c.value === requestedValue)?.reason;
      if (reason) throw new Error(reason);
      const next = applySetupRequest(overlay, fields, request, environment, result ?? undefined);
      if (source.kind === 'frozen') onRequest!(request);
      else if (request.id === 'crop' && request.value === '.true.') setCropRequested(key);
      else onChange!(next);
      setError('');
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }
  function invoke(request: SetupRequest) {
    try {
      write(request);
    } catch {
      /* Keep the last valid configuration and show the reason. */
    }
  }
  const cards = choices.map((control, index) => {
    const label = `${index + (includeSubgrid ? 1 : 2)}. ${t(control.zh, control.en)}`;
    const available = control.choices.filter((c) => !c.reason);
    guideFields.push({
      id: `setup-${control.id}`,
      label,
      value: control.value,
      type: 'choice',
      options: available.map((c) => ({ value: c.value, label: t(c.zh, c.en) })),
      disabled: locked || !available.length,
      set: (value) => write({ id: control.id, value }),
    });
    return (
      <Card
        key={control.id}
        title={label}
        description={
          control.value
            ? t('已选择', 'Selected') +
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
              disabled={locked || !!choice.reason}
              onClick={() => invoke({ id: control.id, value: choice.value })}
            >
              <strong>{t(choice.zh, choice.en)}</strong>
              <small>{choice.reason || t('选择此方案', 'Select this option')}</small>
            </Button>
          ))}
        </div>
        {(control.pending || !result?.context_complete) && (
          <p className="muted">
            {t(
              '待核对：尚未完整确认当前源码与运行环境。',
              'Pending: source and execution context are not fully verified.',
            )}
          </p>
        )}
        {control.id === 'crop' && (
          <p className="muted">
            {environment.cropKernel?.macros.includes('CROP')
              ? t(
                  '已有 CROP 候选内核；需明确填写种植日后启用。',
                  'A CROP kernel is available; enter a planting day to enable crop.',
                )
              : t('尚未确认 CROP 候选内核。', 'CROP kernel capability is not yet confirmed.')}
          </p>
        )}
        {control.id === 'crop' && cropOpen && (
          <Field
            label={t('种植日（单点 0–366，空间模拟 0）', 'Planting day (site 0–366, spatial 0)')}
          >
            <Input
              value={planting}
              disabled={locked}
              aria-label={t('种植日', 'Planting day')}
              onChange={(e) => setCropInput({ key, value: e.target.value })}
            />
            <Button
              disabled={locked || !planting.trim()}
              onClick={() => invoke({ id: 'crop', value: planting })}
            >
              {t('应用种植日', 'Apply planting day')}
            </Button>
          </Field>
        )}
      </Card>
    );
  });
  if (cropOpen) {
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
    actions.push({
      id: 'setup-crop-apply',
      label: t('应用种植日', 'Apply planting day'),
      kind: 'click',
      enabled: !locked && !!planting.trim(),
      run: () => write({ id: 'crop', value: planting }),
    });
  }
  usePageContract({
    title: t('按顺序配置模型结构', 'Configure model structure in order'),
    step: 'scientific-setup',
    fields: guideFields,
    actions,
    status:
      error ||
      (busy ? 'loading' : result?.context_complete ? 'ready' : 'pending_source_verification'),
  });
  return (
    <div className="scientific-setup">
      {error && <Notice tone="error">{error}</Notice>}
      {source.kind === 'frozen' && (
        <p>
          {onRequest
            ? t(
                '选择新方案将建立修改版；当前版本保持不变。',
                'Selecting a new option creates a modified version; the current version is preserved.',
              )
            : t('当前为冻结版本，仅查看。', 'Frozen version: read only.')}
        </p>
      )}
      {cards}
    </div>
  );
}
