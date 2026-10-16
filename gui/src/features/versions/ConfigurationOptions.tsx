import { bilingual } from '../../lib/bilingual';
import { useEffect, useRef, useState } from 'react';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { useApp } from '../../lib/app-state';
import { invoke, invokeJson } from '../../lib/ipc';
import { usePageContract, type UiField, type UiAction } from '../../lib/guide';
import { fieldInputOptions, fieldLabel, optionLabel } from '../../lib/param-presentation';
import type { Overlay } from '../../lib/types';
import {
  configurationReason,
  modelSettingGroups,
  modelSettingGroup,
  modelSettingValue,
  simulationMode,
  caseSchemeFields,
  processSettingOwner,
  simulationOptionField,
} from '../cases/model-settings';
import type { Descriptor, ModelField } from './model';
import { PFT } from './scope-labels';
import {
  caseFieldVisible,
  vicParameterDefault,
  caseOptionOverlay,
  caseOptionPatch,
  type CaseOptionPatch,
  tracerFamilies,
  tracerFamilyState,
  appendTracerFamily,
  setStomatalScheme,
  groupedField,
  unquote,
  setModelOption,
  resetModelOption,
  setScopedOption,
  resetScopedOption,
  setProcessOption,
  resetProcessOption,
  tracerTypes,
  tracerLabels,
  tracerColumns,
  tracerRows,
  setTracerRows,
  type ConfigurationSource,
  type ConfigurationResult,
  type FamilyContext,
} from './configuration-options';
type Props = {
  engine?: string;
  process?: string;
  hideProcesses?: boolean;
  modalActions?: UiAction[];
  onReady?: (ready: boolean) => void;
  removedCaseKeys?: readonly string[];
  onCaseChange?: (patch: CaseOptionPatch) => void | Promise<void>;
  subgrid?: string;
  excludeKeys?: readonly string[];
  kernelMacros?: FamilyContext['kernelMacros'];
  caseScope?: FamilyContext['caseScope'];
  groupPrefix?: string;
  caseValues?: Record<string, string> | null;
  source: ConfigurationSource;
  overlay: Overlay;
  fields: ModelField[];
  catalog: Descriptor[];
  caseDir?: string | null;
  kernelDir?: string | null;
  disabled?: boolean;
  onChange?: (overlay: Overlay) => void;
  onImportProcess?: (relativePath: string) => Promise<void>;
};
const empty: ConfigurationResult = {
  fields: [],
  process_files: [],
  pft_parameters: [],
  warnings: [],
  context_complete: false,
};
export default function ConfigurationOptions({
  engine,
  process,
  hideProcesses = false,
  modalActions,
  onReady,
  removedCaseKeys = [],
  subgrid,
  excludeKeys = [],
  kernelMacros,
  caseScope,
  groupPrefix = 'configuration',
  caseValues = null,
  source,
  overlay: sourceOverlay,
  fields,
  catalog,
  caseDir = null,
  kernelDir = null,
  disabled = false,
  onChange,
  onImportProcess,
  onCaseChange,
}: Props) {
  const { state } = useApp(),
    lang = state.language,
    en = lang === 'en',
    t = (zh: string, english: string) => (en ? english : zh);
  const [result, setResult] = useState(empty),
    [error, setError] = useState(''),
    [loaded, setLoaded] = useState(''),
    [opened, setOpened] = useState<Record<string, boolean>>({}),
    [inputs, setInputs] = useState<Record<string, string>>({}),
    [selectedPftKind, setPftKind] = useState('pft'),
    [selectedPftIndex, setPftIndex] = useState(''),
    [processName, setProcessName] = useState(''),
    [importing, setImporting] = useState(false),
    [tracerInput, setTracerInput] = useState<{
      key: string;
      rows: ReturnType<typeof tracerRows>;
    } | null>(null),
    [saving, setSaving] = useState(false),
    [pickingFile, setPickingFile] = useState(false);
  const scheme = simulationMode(undefined, undefined, subgrid);
  const fixedPftKind = scheme === 'PFT' ? 'pft' : scheme === 'PC' ? 'pc-pft' : null;
  const pftKind = fixedPftKind ?? selectedPftKind;
  const pftChoices = PFT.filter(
    ([index]) =>
      !kernelMacros || kernelMacros.some((macro) => macro.toUpperCase() === 'CROP') || index <= 15,
  );
  const pftIndex = pftChoices.some(([index]) => String(index) === selectedPftIndex)
    ? selectedPftIndex
    : '';
  const pftVisible = !process && !onCaseChange && (!scheme || !!fixedPftKind);
  const pftOpen = pftVisible && (opened.pft ?? false);
  const selection = pftOpen && pftIndex !== '' ? { kind: pftKind, index: Number(pftIndex) } : null;
  const queryValues = scheme ? { ...caseSchemeFields(scheme), ...caseValues } : caseValues;
  const request = JSON.stringify({
    source: source.kind === 'draft' ? { ...source, overlay: sourceOverlay } : source,
    caseDir,
    kernelDir,
    pftSelection: selection,
    caseValues: queryValues,
  });
  const overlay = onCaseChange
    ? removedCaseKeys.reduce(
        (next, key) => resetModelOption(next, key),
        caseOptionOverlay(
          sourceOverlay,
          fields,
          queryValues,
          loaded === request
            ? result.fields.filter((field) => !removedCaseKeys.includes(field.name))
            : [],
        ),
      )
    : sourceOverlay;
  const resistanceForcedOff =
    ['IGBP', 'USGS'].includes(scheme ?? '') &&
    (
      Object.entries(queryValues ?? {}).find(
        ([key]) => key.toUpperCase() === 'DEF_USE_CAMPBELL_SOIL_MODEL',
      )?.[1] ??
      modelSettingValue(overlay, 'DEF_USE_Campbell_SOIL_MODEL') ??
      fields.find((field) => field.name === 'DEF_USE_Campbell_SOIL_MODEL')?.default ??
      '.false.'
    )
      .trim()
      .toLowerCase() === '.false.';
  const sourceKey = JSON.stringify(
    source.kind === 'draft'
      ? { kind: source.kind, id: source.id, generation: source.generation }
      : source,
  );
  const parentValues = Object.fromEntries(
    Object.entries({
      ecology: 'DEF_USE_BGC',
      tracer: 'DEF_USE_TRACER',
      river: 'DEF_USE_GridRiverLakeFlow',
      urban: 'DEF_URBAN_RUN',
      subgrid: 'DEF_USE_LULCC',
    }).map(([id, name]) => [
      id,
      modelSettingValue(overlay, name)?.trim().toLowerCase() === '.true.',
    ]),
  );
  const parentSignature = JSON.stringify({ sourceKey, parentValues });
  const [seenParents, setSeenParents] = useState(parentSignature);
  const rising: string[] = [];
  if (seenParents !== parentSignature) {
    const previous = JSON.parse(seenParents) as {
      sourceKey: string;
      parentValues: Record<string, boolean>;
    };
    if (previous.sourceKey === sourceKey)
      for (const [id, enabled] of Object.entries(parentValues))
        if (enabled && !previous.parentValues[id]) rising.push(id);
    if (rising.length)
      setOpened((before) => ({
        ...before,
        ...Object.fromEntries(rising.map((id) => [`${sourceKey}:${id}`, true])),
      }));
    setSeenParents(parentSignature);
  }
  const tracers = tracerInput?.key === sourceKey ? tracerInput.rows : null;
  const setTracers = (rows: ReturnType<typeof tracerRows> | null) =>
    setTracerInput(rows ? { key: sourceKey, rows } : null);
  const current = useRef(request),
    savingNow = useRef(false),
    writable = useRef(false),
    mounted = useRef(true),
    pickingNow = useRef(false);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  current.current = request;
  useEffect(() => {
    let live = true;
    invokeJson<ConfigurationResult>('model_configuration', JSON.parse(request))
      .then((data) => {
        if (live) {
          setResult(data);
          setLoaded(request);
          setError('');
        }
      })
      .catch((e) => {
        if (live) {
          setLoaded('');
          setError(String(e));
        }
      });
    return () => {
      live = false;
    };
  }, [request]);
  const ready = loaded === request,
    locked =
      disabled || saving || (!onCaseChange && (source.kind !== 'draft' || !onChange)) || importing;
  writable.current = !locked;
  useEffect(() => {
    onReady?.(ready && !error && tracers === null && !pickingFile);
  }, [ready, error, tracers, pickingFile, onReady]);
  const values = ready
    ? {
        ...result,
        fields: result.fields.map((field) => ({
          ...field,
          ...(removedCaseKeys.includes(field.name) ? { configured_value: null } : {}),
          ...(removedCaseKeys.includes(field.parent_gate ?? '')
            ? { parent_gate_enabled: false }
            : {}),
        })),
      }
    : empty;
  const catchmentRoutingFields = [
    'DEF_ElementNeighbour_file',
    'DEF_USE_EstimatedRiverDepth',
    'DEF_Reservoir_Method',
  ];
  const catchmentRouting = kernelMacros?.includes('CATCHMENT') === true;
  const groupForField = (field: ModelField) =>
    catchmentRouting && catchmentRoutingFields.includes(field.name)
      ? 'water'
      : modelSettingGroup(field);
  const included = (field: ModelField) =>
    process
      ? processSettingOwner(field, values.fields) === process
      : !hideProcesses ||
        (catchmentRouting && catchmentRoutingFields.includes(field.name)) ||
        (simulationOptionField(field, values.fields) && !processSettingOwner(field, values.fields));
  const controls: UiField[] = [],
    actions: UiAction[] = [];
  function write(change: () => Overlay) {
    if (locked || !writable.current || savingNow.current || current.current !== request)
      throw new Error(bilingual('当前配置不可修改', 'Configuration is locked or stale'));
    try {
      const next = change();
      if (onCaseChange && modalActions) {
        onCaseChange(caseOptionPatch(overlay, next, fields));
      } else if (onCaseChange) {
        savingNow.current = true;
        setSaving(true);
        void Promise.resolve()
          .then(() => onCaseChange(caseOptionPatch(overlay, next, fields)))
          .then(() => setError(''))
          .catch((e) => setError(String(e)))
          .finally(() => {
            savingNow.current = false;
            setSaving(false);
          });
      } else onChange!(next);
      setError('');
    } catch (e) {
      setError(String(e));
      throw e;
    }
  }
  function uiWrite(change: () => Overlay) {
    try {
      write(change);
    } catch {
      /* Keep draft and invalid input for correction. */
    }
  }
  function open(id: string) {
    return (
      !!process || rising.includes(id) || Boolean(opened[`${sourceKey}:${id}`] ?? parentValues[id])
    );
  }
  function local(id: string, value: string) {
    setInputs((previous) => ({ ...previous, [`${sourceKey}:${id}`]: value }));
  }
  function editor(
    id: string,
    label: string,
    value: string,
    kind: string,
    options: string[],
    cannot: boolean,
    change: (v: string) => void,
    rawKey = id.replace(/^configuration-/, ''),
  ) {
    const choices =
      kind === 'list'
        ? []
        : kind.toLowerCase() === 'logical'
          ? ['.true.', '.false.']
          : fieldInputOptions(rawKey, kind, options);
    const cardChoice =
      [
        'DEF_METHANE%inundation_mode',
        'DEF_TRACER_USE_FRACTIONATION',
        'DEF_Runoff_SCHEME',
        'DEF_URBAN_type_scheme',
      ].includes(rawKey) &&
      choices.length > 0 &&
      choices.length <= 5;
    const displayValue = kind === 'list' ? value : unquote(value),
      shown =
        rawKey === 'DEF_RSS_SCHEME' && resistanceForcedOff
          ? displayValue
          : (inputs[`${sourceKey}:${id}`] ?? displayValue),
      select = choices.length > 0;
    const setter = (v: string) => {
      if (cannot || locked) throw new Error(bilingual('此字段不可编辑', 'Field is locked'));
      if (choices.length && !choices.some((option) => unquote(option) === v))
        throw new Error(bilingual('请选择可用值', 'Choose an available value'));
      local(id, v);
      change(v);
      setInputs((previous) => {
        const next = { ...previous };
        delete next[`${sourceKey}:${id}`];
        return next;
      });
    };
    controls.push({
      id,
      label,
      value: shown,
      type: select ? 'choice' : 'text',
      options: choices.map((v) => ({ value: unquote(v), label: optionLabel(rawKey, v, lang) })),
      disabled: cannot || locked,
      set: setter,
    });
    return (
      <Field key={id} label={label}>
        {cardChoice ? (
          <div className="simulation-choice-grid">
            {choices.map((v) => (
              <Button
                key={v}
                className="simulation-choice-card"
                variant={shown === unquote(v) ? 'primary' : 'secondary'}
                aria-pressed={shown === unquote(v)}
                disabled={cannot || locked}
                onClick={() => {
                  try {
                    setter(unquote(v));
                  } catch {
                    /* Preserve invalid input and draft. */
                  }
                }}
              >
                {optionLabel(rawKey, v, lang)}
              </Button>
            ))}
            {!shown && <p className="muted">{t('请选择', 'Choose an option')}</p>}
          </div>
        ) : select ? (
          <Select
            disabled={cannot || locked}
            value={shown}
            onChange={(e) => {
              try {
                setter(e.target.value);
              } catch {
                /* Error remains visible. */
              }
            }}
          >
            {!shown && <option value="">{t('请选择', 'Choose an option')}</option>}
            {!choices.some((v) => unquote(v) === shown) && shown && (
              <option value={shown}>{shown}</option>
            )}
            {choices.map((v) => (
              <option key={v} value={unquote(v)}>
                {optionLabel(rawKey, v, lang)}
              </option>
            ))}
          </Select>
        ) : (
          <Input
            disabled={cannot || locked}
            value={shown}
            onChange={(e) => {
              if (!modalActions) local(id, e.target.value);
              else
                try {
                  setter(e.target.value);
                } catch (e) {
                  setError(String(e));
                }
            }}
            onBlur={(e) => {
              if (!cannot && !locked && e.target.value !== displayValue)
                try {
                  setter(e.target.value);
                } catch {
                  /* Error remains visible. */
                }
            }}
          />
        )}
      </Field>
    );
  }
  function row(field: ModelField) {
    const context = values.fields.find((f) => f.name === field.name),
      configured = removedCaseKeys.includes(field.name)
        ? null
        : (modelSettingValue(overlay, field.name) ?? context?.configured_value ?? null),
      forcedResistance = field.name === 'DEF_RSS_SCHEME' && resistanceForcedOff,
      defaultValue = forcedResistance ? '0' : (context?.built_in_default ?? field.default ?? ''),
      cnInitialFile = field.name === 'DEF_file_cn_init',
      urbanAlbedoFile = field.name === 'DEF_HighResUrban_albedo',
      vicFile = ['DEF_file_VIC_para', 'DEF_file_VIC_OPT'].includes(field.name),
      vicDefault = vicFile
        ? vicParameterDefault(
            field.name,
            modelSettingValue(overlay, 'DEF_dir_runtime') ??
              values.fields.find((entry) => entry.name === 'DEF_dir_runtime')?.configured_value ??
              values.fields.find((entry) => entry.name === 'DEF_dir_runtime')?.built_in_default ??
              '',
          )
        : '',
      fileInput = cnInitialFile || urbanAlbedoFile || vicFile,
      rawValue = forcedResistance ? '0' : (configured ?? defaultValue),
      value = fileInput && /^(null|none)?$/i.test(unquote(rawValue).trim()) ? vicDefault : rawValue,
      overridden = !forcedResistance && configured !== null && configured !== '';
    const inactive = ['disabled', 'readonly', 'read-only'].includes(context?.mode ?? ''),
      cannot = !ready || !!inactive || forcedResistance,
      id = `configuration-${field.name}`;
    actions.push({
      id: `${id}-reset`,
      label: `${fieldLabel(field.name, lang)} · ${t('恢复默认', 'Restore default')}`,
      kind: 'click',
      enabled: !locked && overridden,
      visible: overridden,
      run: () => write(() => resetModelOption(overlay, field.name)),
    });
    async function pickFile() {
      if (
        cannot ||
        locked ||
        !writable.current ||
        pickingNow.current ||
        !mounted.current ||
        current.current !== request
      )
        throw new Error(bilingual('当前配置不可修改', 'Configuration is locked or stale'));
      pickingNow.current = true;
      setPickingFile(true);
      try {
        const path = await invoke<string | null>('pick_file', { key: field.name, filter: null });
        if (path && mounted.current && writable.current && current.current === request) {
          write(() => setModelOption(overlay, fields, field.name, path));
          setInputs((previous) => {
            const next = { ...previous };
            delete next[`${sourceKey}:${id}`];
            return next;
          });
        }
      } catch (e) {
        if (mounted.current && current.current === request) setError(String(e));
      } finally {
        pickingNow.current = false;
        if (mounted.current) setPickingFile(false);
      }
    }
    function restoreDefault() {
      write(() => resetModelOption(overlay, field.name));
      setInputs((previous) => {
        const next = { ...previous };
        delete next[`${sourceKey}:${id}`];
        return next;
      });
    }
    if (vicFile)
      actions.push({
        id: `${id}-preset`,
        label: `${t('使用预设路径', 'Use preset path')} · ${fieldLabel(field.name, lang)}`,
        kind: 'click',
        enabled: !cannot && !locked && !!vicDefault,
        run: restoreDefault,
      });
    if (fileInput)
      actions.push({
        id: `${id}-browse`,
        label: `${t('选择文件', 'Choose file')} · ${fieldLabel(field.name, lang)}`,
        kind: 'click',
        enabled: !cannot && !locked && !pickingFile,
        run: pickFile,
      });
    const reason = configurationReason(context?.reason);
    const resistanceReason = forcedResistance
      ? t(
          'LCT + van Genuchten 时模型固定关闭；切换后恢复自定义值或默认 SL14（1）',
          'LCT + van Genuchten forces this off; changing schemes restores the custom value or default SL14 (1)',
        )
      : null;
    return (
      <div className="model-setting-row" key={field.name}>
        {editor(
          id,
          fieldLabel(field.name, lang),
          value,
          field.kind,
          context?.allowed_values.length ? context.allowed_values : field.values,
          cannot,
          (v) => write(() => setModelOption(overlay, fields, field.name, v)),
        )}
        <small>
          {field.name} · {(resistanceReason || reason) && <>{resistanceReason || reason} · </>}{' '}
          {t('默认值', 'Default')}:{' '}
          {vicFile
            ? vicDefault || t('请先设置运行数据目录', 'Set the runtime data directory first')
            : cnInitialFile
              ? t(
                  '运行数据目录下的 cnsteadystate.nc',
                  'cnsteadystate.nc in the runtime data directory',
                )
              : urbanAlbedoFile && /^(null|none)?$/i.test(unquote(defaultValue).trim())
                ? t(
                    '未提供默认文件，请选择城市高光谱反照率 NetCDF 文件',
                    'No default file; choose an urban hyperspectral albedo NetCDF file',
                  )
                : defaultValue || '—'}
        </small>
        {vicFile && (
          <Button
            disabled={cannot || locked || !vicDefault}
            onClick={() => {
              try {
                restoreDefault();
              } catch {
                /* Keep the error visible. */
              }
            }}
          >
            {t('使用预设路径', 'Use preset path')}
          </Button>
        )}
        {fileInput && (
          <Button
            disabled={cannot || locked || pickingFile}
            onClick={() => {
              void pickFile().catch(() => {});
            }}
          >
            {t('选择文件', 'Choose file')}
          </Button>
        )}
        {overridden && !vicFile && (
          <Button
            disabled={locked}
            onClick={() => uiWrite(() => resetModelOption(overlay, field.name))}
          >
            {t('恢复默认', 'Restore default')}
          </Button>
        )}
      </div>
    );
  }
  const stomatalFields = ['DEF_USE_MEDLYNST', 'DEF_USE_WUEST'];
  const showStomatal = stomatalFields.every((name) =>
    fields.some((f) => f.name === name && !f.derived && included(f)),
  );
  function stomatal() {
    if (!showStomatal) return null;
    const overridden = stomatalFields.some(
      (name) =>
        !removedCaseKeys.includes(name) &&
        (modelSettingValue(overlay, name) ??
          values.fields.find((field) => field.name === name)?.configured_value) != null,
    );
    const flags = stomatalFields.map((name) => {
      const context = values.fields.find((field) => field.name === name);
      const configured = removedCaseKeys.includes(name)
        ? null
        : (modelSettingValue(overlay, name) ?? context?.configured_value);
      return (
        configured ??
        context?.built_in_default ??
        fields.find((field) => field.name === name)?.default
      )
        ?.trim()
        .toLowerCase();
    });
    const selected = flags.every((v) => v === '.true.' || v === '.false.')
      ? flags[0] === '.true.' && flags[1] === '.true.'
        ? ''
        : flags[0] === '.true.'
          ? 'MEDLYN'
          : flags[1] === '.true.'
            ? 'WUE'
            : 'BALL_BERRY'
      : '';
    const unavailable =
      !ready ||
      stomatalFields.some((name) =>
        ['disabled', 'readonly', 'read-only'].includes(
          values.fields.find((f) => f.name === name)?.mode ?? '',
        ),
      );
    const options = [
      ['BALL_BERRY', 'Ball–Berry'],
      ['MEDLYN', 'Medlyn'],
      ['WUE', t('水分利用效率 WUE', 'Water-use efficiency WUE')],
    ];
    const set = (value: string) => {
      if (unavailable) throw new Error('Stomatal scheme unavailable');
      write(() => setStomatalScheme(overlay, fields, value));
    };
    controls.push({
      id: 'configuration-stomatal-scheme',
      label: t('气孔导度方案', 'Stomatal scheme'),
      type: 'choice',
      value: selected,
      options: options.map(([value, label]) => ({ value, label })),
      disabled: locked || unavailable,
      set,
    });
    const reset = () =>
      write(() => stomatalFields.reduce((next, name) => resetModelOption(next, name), overlay));
    actions.push({
      id: 'configuration-stomatal-reset',
      label: t('气孔导度方案 · 恢复默认', 'Stomatal scheme · Restore default'),
      kind: 'click',
      enabled: !locked && overridden,
      visible: overridden,
      run: reset,
    });
    return (
      <div className="model-setting-row">
        <h3>{t('气孔导度方案', 'Stomatal scheme')}</h3>
        <div className="simulation-choice-grid">
          {options.map(([value, label]) => (
            <Button
              key={value}
              className="simulation-choice-card"
              aria-pressed={selected === value}
              variant={selected === value ? 'primary' : 'secondary'}
              disabled={locked || unavailable}
              onClick={() => {
                try {
                  set(value);
                } catch {
                  /* Preserve draft. */
                }
              }}
            >
              {label}
            </Button>
          ))}
        </div>
        {!selected && (
          <p className="muted">
            {t(
              '开关值冲突或缺少可用默认值，请核对。',
              'Switches conflict or no usable defaults are available; check the configuration.',
            )}
          </p>
        )}
        {overridden && (
          <Button
            disabled={locked}
            onClick={() => {
              try {
                reset();
              } catch {
                /* Preserve draft. */
              }
            }}
          >
            {t('恢复默认', 'Restore default')}
          </Button>
        )}
      </div>
    );
  }
  const orderedGroups = [
    'site',
    'forcing',
    'subgrid',
    'soil',
    'water',
    'river',
    'ecology',
    'urban',
    'tracer',
    'assimilation',
    'other',
  ].map((id) => modelSettingGroups.find((g) => g[0] === id)!);
  const caseProcessVisible = (id: string) =>
    (!hideProcesses || !['ecology', 'tracer', 'river', 'urban'].includes(id)) &&
    (!process || id !== 'tracer' || process === 'DEF_USE_TRACER') &&
    (!onCaseChange || parentValues[id] === true);
  const groups = orderedGroups.map(([id, zh, english]) => {
    if (['river', 'urban', 'tracer'].includes(id) && !caseProcessVisible(id)) return null;
    const rows = fields
      .filter(
        (field) =>
          included(field) &&
          !(
            engine === 'fortran' && ['DEF_file_VIC_para', 'DEF_file_VIC_OPT'].includes(field.name)
          ) &&
          !excludeKeys.includes(field.name) &&
          groupedField(field, catalog) &&
          !(field.name === 'DEF_USE_SOILPAR_UPS_FIT' && caseScope === 'site'),
      )
      .filter(
        (field) =>
          (!onCaseChange &&
            !['DEF_VIC_OPT', 'DEF_file_VIC_para', 'DEF_file_VIC_OPT'].includes(field.name)) ||
          (field.name === 'DEF_RSS_SCHEME' && resistanceForcedOff) ||
          caseFieldVisible(field.name, values.fields, overlay.physics),
      )
      .filter(
        (field) =>
          !onCaseChange ||
          processSettingOwner(field, values.fields) !== 'DEF_USE_BGC' ||
          parentValues.ecology,
      )
      .filter(
        (field) =>
          !onCaseChange ||
          (field.name === 'DEF_SOLO_PFT'
            ? subgrid === 'PFT'
            : ['DEF_FAST_PC', 'DEF_PC_CROP_SPLIT'].includes(field.name)
              ? subgrid === 'PC'
              : field.name === 'DEF_LULCC_SCHEME'
                ? parentValues.subgrid === true
                : true),
      )
      .filter((f) => groupForField(f) === id && !(showStomatal && stomatalFields.includes(f.name)));
    if (id === 'soil') {
      const order = ['DEF_RSS_SCHEME', 'DEF_SOIL_REFL_SCHEME'];
      const rank = (name: string) => (order.includes(name) ? order.indexOf(name) : order.length);
      rows.sort((a, b) => rank(a.name) - rank(b.name));
    }
    if (id === 'water') {
      const snow = rows.findIndex((field) => field.name === 'DEF_VEG_SNOW');
      if (snow >= 0 && rows.some((field) => field.name === 'DEF_Interception_scheme')) {
        const [field] = rows.splice(snow, 1);
        rows.splice(
          rows.findIndex((item) => item.name === 'DEF_Interception_scheme') + 1,
          0,
          field,
        );
      }
    }
    if (
      onCaseChange &&
      !rows.length &&
      !(id === 'ecology' && showStomatal) &&
      !(id === 'tracer' && caseProcessVisible(id))
    )
      return null;
    if (process)
      return (
        <Card key={id} title={t(zh, english)}>
          {id === 'ecology' && stomatal()}
          {rows.map(row)}
        </Card>
      );
    actions.push({
      id: `${groupPrefix}-group-${id}`,
      label: t(zh, english),
      kind: 'click',
      enabled: !disabled,
      run: () => {
        if (disabled) throw new Error('Configuration is locked');
        setOpened((previous) => ({ ...previous, [`${sourceKey}:${id}`]: !open(id) }));
      },
    });
    return (
      <Advanced
        key={id}
        title={`${t(zh, english)} · ${rows.length}`}
        open={open(id)}
        onOpenChange={(value) => {
          if (!disabled) setOpened((previous) => ({ ...previous, [`${sourceKey}:${id}`]: value }));
        }}
      >
        {id === 'ecology' && open(id) && stomatal()}
        {open(id) && rows.map(row)}
      </Advanced>
    );
  });
  let storedTracers: ReturnType<typeof tracerRows>,
    tracerError = '';
  try {
    storedTracers = tracerRows(overlay);
  } catch (e) {
    storedTracers = tracerRows(resetModelOption(overlay, 'DEF_TRACER_PARAM_FILES'));
    tracerError = String(e);
  }
  const tracerOpen = caseProcessVisible('tracer') && open('tracer'),
    table = tracers ?? storedTracers,
    tracerFields = fields.filter((f) => f.name.startsWith('DEF_TRACER_'));
  const tracerTooLarge = Number(modelSettingValue(overlay, 'DEF_TRACER_NUM') ?? '0') > 100;
  const tracerReady =
    !tracerTooLarge &&
    !locked &&
    ready &&
    tracerFields.length > 0 &&
    !['disabled', 'readonly', 'read-only'].includes(
      values.fields.find((f) => f.name === 'DEF_TRACER_NAMES')?.mode ?? '',
    );
  const familyContext: FamilyContext = {
    subgrid,
    kernelMacros:
      kernelMacros ?? state.kernels?.find((item) => item.dir === kernelDir)?.macros ?? null,
    caseScope,
  };
  const familyVisible =
    tracerOpen && modelSettingValue(overlay, 'DEF_USE_TRACER')?.trim().toLowerCase() === '.true.';
  const familyCards = familyVisible && (
    <Card
      title={t('先选择要追踪的物质', 'Choose tracer families')}
      description={t(
        '只预填名称和类型，保留已有与自定义条目。请核对并完善数值列、导入及关联过程文件，再应用定义；此操作不启动计算。',
        'Only names and types are prefilled; existing and custom rows are retained. Complete numeric metadata and import/link process files before applying. No simulation starts here.',
      )}
    >
      <div className="simulation-choice-grid">
        {tracerFamilies.map((family) => {
          const availability = tracerFamilyState(overlay, family.id, familyContext),
            next = appendTracerFamily(table, family.id),
            already = next.length === table.length,
            blocked = !tracerReady || availability.status === 'blocked' || already;
          const add = () => {
            if (!familyVisible || blocked || current.current !== request)
              throw new Error('Family is unavailable or already present');
            setTracers(next);
          };
          actions.push({
            id: `configuration-family-${family.id}`,
            label: t(family.zh, family.en),
            kind: 'click',
            enabled: !blocked,
            run: add,
          });
          return (
            <Button
              key={family.id}
              className="simulation-choice-card"
              variant={already ? 'primary' : 'secondary'}
              aria-pressed={already}
              disabled={blocked}
              onClick={add}
            >
              <strong>{t(family.zh, family.en)}</strong>
              <span>{family.rows.map(([name]) => name).join(' / ')}</span>
              <small>
                {already
                  ? t('已在定义表中', 'Already in the table')
                  : availability.status === 'blocked'
                    ? t('先解决依赖冲突', 'Resolve prerequisites first')
                    : availability.status === 'pending'
                      ? t('可以预填；适用性待核对', 'Can prefill; applicability pending')
                      : t('预填到定义表', 'Prefill definition table')}
              </small>
              {availability.reasons.map(([zh, english]) => (
                <small key={zh}>{t(zh, english)}</small>
              ))}
            </Button>
          );
        })}
      </div>
      <p className="muted">
        {t(
          '预填后尚未保存；可在应用前用“移除末行”撤回新增行。关闭示踪开关不会删除已有定义。',
          'Prefilled rows are not saved. Use Remove last row before applying to undo added rows. Turning tracers off preserves saved definitions.',
        )}
      </p>
    </Card>
  );
  const tracerTable = tracerOpen && (
    <Card
      title={t('示踪物定义（每行对应同一个示踪物）', 'Tracer definitions (aligned rows)')}
      description={t(
        '名称、类型和可选参数列按行对应；留空整列保留内核解释，不自动填目录默认值。过程文件须先导入到当前版本。',
        'Names, types and optional columns are aligned by row. Empty columns are not filled with catalog defaults. Import process files into this version first.',
      )}
    >
      {tracerError && (
        <Notice tone="error">
          {tracerError} · {modelSettingValue(overlay, 'DEF_TRACER_PARAM_FILES')}
        </Notice>
      )}
      {tracerTooLarge && (
        <Notice tone="warning">
          {t(
            '超过100个示踪物的定义仅展示前100项，请通过原始字段编辑；此表不会覆盖完整定义。',
            'Over 100 tracers: only the first 100 are shown. Edit the raw fields; this table cannot replace the full definition.',
          )}
        </Notice>
      )}
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              {tracerColumns.map((c) => (
                <th key={c}>{t(...tracerLabels[c])}</th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.map((row, index) => (
              <tr key={index}>
                {tracerColumns.map((column) => {
                  const id = `configuration-tracer-${index}-${column}`,
                    set = (value: string) => {
                      if (!tracerReady) throw new Error('Tracer editor is locked');
                      setTracers(
                        table.map((r, i) => (i === index ? { ...r, [column]: value } : r)),
                      );
                    };
                  controls.push({
                    id,
                    label: `${index + 1} ${t(...tracerLabels[column])}`,
                    value: row[column],
                    type: column === 'TYPES' ? 'choice' : 'text',
                    options:
                      column === 'TYPES'
                        ? tracerTypes.map(([value, zh, english]) => ({
                            value,
                            label: t(zh, english),
                          }))
                        : undefined,
                    disabled: !tracerReady,
                    set,
                  });
                  return (
                    <td key={column}>
                      {column === 'TYPES' ? (
                        <Select
                          disabled={!tracerReady}
                          aria-label={`${index + 1} ${t(...tracerLabels[column])}`}
                          value={row[column]}
                          onChange={(e) => set(e.target.value)}
                        >
                          <option value="">{t('选择类型', 'Choose type')}</option>
                          {row[column] && !tracerTypes.some(([kind]) => kind === row[column]) && (
                            <option value={row[column]}>
                              {row[column]} · {t('当前源码类型待核对', 'Source type pending')}
                            </option>
                          )}
                          {tracerTypes.map(([value, zh, english]) => (
                            <option value={value} key={value}>
                              {t(zh, english)}
                            </option>
                          ))}
                        </Select>
                      ) : (
                        <Input
                          disabled={!tracerReady}
                          aria-label={`${index + 1} ${column}`}
                          value={row[column]}
                          onChange={(e) => set(e.target.value)}
                        />
                      )}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="button-row">
        <Button
          disabled={!tracerReady}
          onClick={() =>
            setTracers([
              ...table,
              Object.fromEntries(tracerColumns.map((c) => [c, ''])) as (typeof table)[number],
            ])
          }
        >
          {t('增加示踪物', 'Add tracer')}
        </Button>
        <Button
          disabled={!tracerReady || !table.length}
          onClick={() => setTracers(table.slice(0, -1))}
        >
          {t('移除末行', 'Remove last row')}
        </Button>
        <Button
          disabled={!tracerReady || tracers === null}
          onClick={() => {
            try {
              write(() => setTracerRows(overlay, fields, table));
              setTracers(null);
            } catch {
              /* Preserve incomplete table. */
            }
          }}
        >
          {t('应用示踪物定义', 'Apply tracer definitions')}
        </Button>
      </div>
      <p>
        {t('示踪物数量', 'Tracer count')} (DEF_TRACER_NUM): {table.length}
      </p>
      <p>
        {t(
          '尚未应用的表格修改不会写入草稿。',
          'Table changes are not written to the draft until applied.',
        )}
      </p>
    </Card>
  );
  actions.push(
    {
      id: 'configuration-tracers-add',
      label: t('增加示踪物', 'Add tracer'),
      kind: 'click',
      visible: tracerOpen,
      enabled: tracerReady,
      run: () => {
        if (!tracerReady || !tracerOpen) throw new Error('Tracer editor is locked');
        setTracers([
          ...table,
          Object.fromEntries(tracerColumns.map((c) => [c, ''])) as (typeof table)[number],
        ]);
      },
    },
    {
      id: 'configuration-tracers-remove',
      label: t('移除末行', 'Remove last row'),
      kind: 'click',
      visible: tracerOpen,
      enabled: tracerReady && table.length > 0,
      run: () => {
        if (!tracerReady || !tracerOpen || !table.length)
          throw new Error('Tracer editor is locked');
        setTracers(table.slice(0, -1));
      },
    },
    {
      id: 'configuration-tracers-apply',
      label: t('应用示踪物定义', 'Apply tracer definitions'),
      kind: 'click',
      visible: tracerOpen,
      enabled: tracerOpen && tracerReady && tracers !== null,
      run: () => {
        if (!tracerOpen || !tracerReady || tracers === null)
          throw new Error('Tracer table is unavailable');
        write(() => setTracerRows(overlay, fields, table));
        setTracers(null);
      },
    },
  );
  const assets = (caseProcessVisible('tracer') ? values.process_files : []).map((asset) => {
    const isOpen = opened[`${sourceKey}:${asset.file}`] ?? tracerOpen;
    actions.push({
      id: `configuration-asset-${asset.file}`,
      label: asset.file,
      kind: 'click',
      enabled: !disabled,
      run: () => {
        if (disabled) throw new Error('Configuration is locked');
        setOpened((previous) => ({ ...previous, [`${sourceKey}:${asset.file}`]: !isOpen }));
      },
    });
    return (
      <Advanced
        key={asset.file}
        title={`${asset.title} · ${asset.file} · ${asset.active_methane_asset === true ? t('当前 CH4', 'Current CH4') : asset.active_methane_asset === false ? t('CH4 未采用', 'Not used by CH4') : t('是否被 CH4 采用待核对', 'CH4 usage pending')}`}
        open={isOpen}
        onOpenChange={(value) => {
          if (!disabled)
            setOpened((previous) => ({ ...previous, [`${sourceKey}:${asset.file}`]: value }));
        }}
      >
        {isOpen &&
          asset.entries.map((entry) => {
            const descriptor = catalog.find(
                (d) => d.id === entry.descriptor_id && d.storage === 'process-parameter-file',
              ),
              id = `configuration-process-${asset.file}-${entry.path}`,
              cannot =
                !!onCaseChange || !ready || !asset.verified || !entry.editable || !descriptor;
            const reset = () => write(() => resetProcessOption(overlay, descriptor!, asset.file));
            actions.push({
              id: `${id}-reset`,
              label: `${fieldLabel(entry.path, lang)} · ${t('恢复资产原值', 'Restore asset value')}`,
              kind: 'click',
              enabled: !locked && !cannot,
              run: () => {
                if (cannot) throw new Error('Process parameter is unavailable');
                reset();
              },
            });
            return (
              <div className="model-setting-row" key={entry.path}>
                {editor(
                  id,
                  fieldLabel(entry.path, lang),
                  entry.unset ? entry.default : entry.value,
                  entry.kind,
                  entry.options,
                  cannot,
                  (v) =>
                    write(() => setProcessOption(overlay, descriptor!, v, asset.file, entry.kind)),
                  entry.path,
                )}
                <small>
                  {entry.path} ·{' '}
                  {entry.pending
                    ? t('适用性待核对', 'Applicability pending')
                    : t('已核对目录条件', 'Catalog conditions checked')}{' '}
                  · {entry.doc} · {t('目录参考', 'Catalog reference')}: {entry.default || '—'}
                </small>
                <Button
                  disabled={locked || cannot}
                  onClick={() => {
                    try {
                      reset();
                    } catch {
                      /* Preserve draft. */
                    }
                  }}
                >
                  {t('恢复资产原值', 'Restore asset value')}
                </Button>
              </div>
            );
          })}
      </Advanced>
    );
  });
  const indexed = (pftOpen ? values.pft_parameters : []).map((entry) => {
    const descriptor = catalog.find(
        (d) =>
          d.id === entry.parameter_id &&
          d.storage === (pftKind === 'pft' ? 'pft-override' : 'pc-pft-override'),
      ),
      id = `configuration-${pftKind}-${pftIndex}-${entry.name}`,
      cannot = !!onCaseChange || !ready || entry.mode !== 'editable' || !descriptor || !selection,
      overridden = entry.configured_value !== null;
    const reset = () => write(() => resetScopedOption(overlay, descriptor!, Number(pftIndex), ''));
    actions.push({
      id: `${id}-reset`,
      label: `${entry.name} · ${t('恢复默认', 'Restore default')}`,
      kind: 'click',
      enabled: !onCaseChange && !locked && !!descriptor && !!selection && overridden,
      visible: overridden,
      run: () => {
        if (!descriptor || !selection) throw new Error('PFT parameter is unavailable');
        reset();
      },
    });
    return (
      <div className="model-setting-row" key={id}>
        {editor(
          id,
          en ? entry.label_en : entry.label_zh,
          entry.configured_value ?? entry.default ?? '',
          entry.kind,
          entry.allowed_values,
          cannot,
          (v) => write(() => setScopedOption(overlay, descriptor!, v, Number(pftIndex), '')),
          entry.name,
        )}
        <small>
          {entry.scope_label} · {entry.unit ?? '—'} ·{' '}
          {entry.reason || t('待按冻结源码核对', 'Pending frozen-source verification')} ·{' '}
          {t('目录参考', 'Catalog reference')}: {entry.default ?? '—'}
        </small>
        {overridden && (
          <Button
            disabled={!!onCaseChange || locked || !descriptor || !selection}
            onClick={() => {
              try {
                reset();
              } catch {
                /* Preserve draft. */
              }
            }}
          >
            {t('恢复默认', 'Restore default')}
          </Button>
        )}
      </div>
    );
  });
  async function importProcess() {
    if (locked || !onImportProcess || !processName.trim())
      throw new Error('Process import is unavailable');
    setImporting(true);
    try {
      await onImportProcess(processName.trim());
      setError('');
    } catch (e) {
      setError(String(e));
      throw e;
    } finally {
      setImporting(false);
    }
  }
  if (onImportProcess) {
    controls.push({
      id: 'configuration-process-file',
      label: t('版本中的相对文件名', 'Relative filename in version'),
      type: 'text',
      value: processName,
      disabled: locked,
      set: (value) => {
        if (locked) throw new Error('Configuration is locked');
        setProcessName(value);
      },
    });
    actions.push({
      id: 'configuration-process-import',
      label: t('选择并导入过程文件', 'Choose and import process file'),
      kind: 'commit',
      enabled: !locked && !!processName.trim(),
      run: importProcess,
    });
  }
  if (pftVisible)
    actions.push({
      id: 'configuration-pft-toggle',
      label: t('按植物类型调整参数', 'Adjust parameters by plant type'),
      kind: 'click',
      enabled: !disabled,
      run: () => {
        if (disabled) throw new Error('Configuration is locked');
        setOpened((previous) => ({ ...previous, pft: !pftOpen }));
      },
    });
  if (pftOpen && !fixedPftKind)
    controls.push({
      id: 'configuration-pft-kind',
      label: 'PFT / PC',
      type: 'choice',
      value: pftKind,
      disabled,
      options: [
        { value: 'pft', label: 'PFT' },
        { value: 'pc-pft', label: 'PC PFT' },
      ],
      set: (value) => {
        if (disabled || fixedPftKind || !['pft', 'pc-pft'].includes(value))
          throw new Error('Invalid PFT scope');
        setPftKind(value);
        setInputs({});
      },
    });
  if (pftOpen)
    controls.push({
      id: 'configuration-pft-index',
      label: t('植物类型', 'Plant type'),
      type: 'choice',
      value: pftIndex,
      disabled,
      options: pftChoices.map(([index, zh, english]) => ({
        value: String(index),
        label: `${index} · ${t(zh, english)}`,
      })),
      set: (value) => {
        if (disabled || (value !== '' && !pftChoices.some(([index]) => String(index) === value)))
          throw new Error('Invalid PFT index');
        setPftIndex(value);
        setInputs({});
      },
    });
  usePageContract({
    title: t('完整模型配置', 'Complete model configuration'),
    step: 'configuration-options',
    modal: !!modalActions,
    fields: controls,
    actions: [...actions, ...(modalActions ?? [])],
    status: error || (!ready ? 'loading' : 'pending_source_verification'),
    details: { context_complete: values.context_complete, warnings: values.warnings },
  });
  return (
    <div className="configuration-options">
      <Notice>
        {t(
          '未单独设置时显示默认值；修改并保存后覆盖默认值。启用过程后显示相关配置，关闭过程保留已有设置。',
          'Defaults are shown when no override is set; saved edits override them. Enable a process to see its settings. Disabling it preserves saved values.',
        )}
      </Notice>
      {error && <Notice tone="error">{error}</Notice>}
      {!ready && <p>{t('正在查询配置条件…', 'Checking configuration conditions…')}</p>}
      {groups}
      {familyCards}
      {tracerTable}
      {modalActions && tracers !== null && (
        <Notice>
          {t(
            '请先应用示踪物定义，再保存设置。',
            'Apply tracer definitions before saving settings.',
          )}
        </Notice>
      )}
      {caseProcessVisible('tracer') && (
        <Card title={t('示踪过程文件', 'Tracer process files')}>
          {ready && !overlay.process_files.length && (
            <p className="muted">
              {t(
                '当前版本尚未导入过程参数文件。',
                'This version has no imported process parameter files.',
              )}
            </p>
          )}
          {overlay.process_files.map((asset) => (
            <p key={asset.relative_path}>
              {asset.relative_path} · <code>{asset.sha256}</code>
            </p>
          ))}
          {assets}
          {onImportProcess && (
            <div className="form-grid">
              <Field label={t('版本中的相对文件名', 'Relative filename in version')}>
                <Input
                  disabled={locked}
                  value={processName}
                  placeholder="CH4.nml"
                  onChange={(e) => setProcessName(e.target.value)}
                />
              </Field>
              <Button
                disabled={locked || !processName.trim()}
                onClick={() => {
                  void importProcess().catch(() => {});
                }}
              >
                {t('选择并导入过程文件', 'Choose and import process file')}
              </Button>
            </div>
          )}
        </Card>
      )}
      {pftVisible && (
        <Advanced
          title={t('按植物类型调整参数', 'Adjust parameters by plant type')}
          open={pftOpen}
          onOpenChange={(value) => {
            if (!disabled) setOpened((previous) => ({ ...previous, pft: value }));
          }}
        >
          {pftOpen && (
            <>
              <p>
                {t(
                  'PC 参数按群落内的 PFT 组分设置；类型索引与 PFT 共用。',
                  'PC parameters apply to PFT components within a community and share the PFT type indices.',
                )}
              </p>
              <div className="form-grid">
                {!fixedPftKind && (
                  <Field label="PFT / PC">
                    <Select
                      disabled={disabled}
                      value={pftKind}
                      onChange={(e) => {
                        setPftKind(e.target.value);
                        setInputs({});
                      }}
                    >
                      <option value="pft">PFT</option>
                      <option value="pc-pft">PC PFT</option>
                    </Select>
                  </Field>
                )}
                <Field label={t('植物类型', 'Plant type')}>
                  <Select
                    disabled={disabled}
                    value={pftIndex}
                    onChange={(e) => {
                      setPftIndex(e.target.value);
                      setInputs({});
                    }}
                  >
                    <option value="">{t('明确选择类型', 'Choose a type')}</option>
                    {pftChoices.map(([index, zh, english]) => (
                      <option value={index} key={index}>
                        {index} · {t(zh, english)}
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
              {!selection && (
                <Notice tone="warning">
                  {t(
                    '请选择当前内核支持的植物类型。',
                    'Choose a plant type supported by the current kernel.',
                  )}
                </Notice>
              )}
              {indexed}
            </>
          )}
        </Advanced>
      )}
    </div>
  );
}
