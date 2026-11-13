import { bilingual } from '../../lib/bilingual';
import RemotePathBrowser from '../../components/RemotePathBrowser';
import { useEffect, useRef, useState } from 'react';
import { Advanced, Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
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
  materialFieldVisible,
  vicParameterDefault,
  caseOptionOverlay,
  caseOptionPatch,
  type CaseOptionPatch,
  materialFamilies,
  materialFamilyState,
  materialSetupIssues,
  materialProcessEnabled,
  appendMaterialFamily,
  applyMaterialSelection,
  setStomatalScheme,
  groupedField,
  unquote,
  setModelOption,
  resetModelOption,
  setScopedOption,
  resetScopedOption,
  setProcessOption,
  resetProcessOption,
  materialTypes,
  materialLabels,
  materialColumns,
  materialKeys,
  materialRows,
  setMaterialRows,
  legacyCaseOverrideEntries,
  AQUIFER_MIXING_WATER,
  AQUIFER_MIXING_WATER_TEST_MM,
  type ConfigurationSource,
  type ConfigurationResult,
  type FamilyContext,
} from './configuration-options';
type Props = {
  runHost?: string;
  engine?: string;
  process?: string;
  hideProcesses?: boolean;
  /** 只表示"这份配置在对话框里"（页面契约按对话框合并、草稿改动暂存）；不再承载动作，
   *  对话框的保存/取消在对话框里由 `ActionButton` 自己登记（第 737 轮）。 */
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
  runHost,
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
    [caseEdits, setCaseEdits] = useState<{ scope: string; fields: string[] } | null>(null),
    [selectedPftKind, setPftKind] = useState('pft'),
    [selectedPftIndex, setPftIndex] = useState(''),
    [processName, setProcessName] = useState(''),
    [importing, setImporting] = useState(false),
    [materialInput, setMaterialInput] = useState<{
      key: string;
      rows: ReturnType<typeof materialRows>;
    } | null>(null),
    [saving, setSaving] = useState(false),
    [pickingFile, setPickingFile] = useState(false);
  const [remotePicker, setRemotePicker] = useState<{
    host: string;
    initialPath?: string;
  } | null>(null);
  const remoteResolve = useRef<((path: string | null) => void) | null>(null);
  const currentHost = useRef(runHost);
  currentHost.current = runHost;
  useEffect(
    () => () => {
      remoteResolve.current?.(null);
    },
    [],
  );
  function closeRemotePicker(path: string | null) {
    const resolve = remoteResolve.current;
    remoteResolve.current = null;
    setRemotePicker(null);
    resolve?.(path);
  }
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
  const sourceKey = JSON.stringify(
    source.kind === 'draft'
      ? { kind: source.kind, id: source.id, generation: source.generation }
      : source,
  );
  const editScope = JSON.stringify({ sourceKey, caseDir });
  const queryValues = scheme ? { ...caseSchemeFields(scheme), ...caseValues } : caseValues;
  const request = JSON.stringify({
    source: source.kind === 'draft' ? { ...source, overlay: sourceOverlay } : source,
    caseDir,
    kernelDir,
    engine: engine ?? null,
    pftSelection: selection,
    caseValues: queryValues,
    caseEditedFields:
      caseEdits?.scope === editScope
        ? caseEdits.fields.filter((key) => !removedCaseKeys.includes(key))
        : null,
    caseRemovedFields: removedCaseKeys,
  });
  const overlay = onCaseChange
    ? removedCaseKeys.reduce(
        (next, key) => resetModelOption(next, key),
        caseOptionOverlay(
          sourceOverlay,
          fields,
          loaded === request ? { ...queryValues, ...result.legacy_case_overrides } : queryValues,
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
  const parentValues = Object.fromEntries(
    Object.entries({
      ecology: 'DEF_USE_BGC',
      material: 'material-process',
      river: 'DEF_USE_GridRiverLakeFlow',
      urban: 'DEF_URBAN_RUN',
      subgrid: 'DEF_USE_LULCC',
    }).map(([id, name]) => [
      id,
      name === 'material-process'
        ? materialProcessEnabled(overlay)
        : modelSettingValue(overlay, name)?.trim().toLowerCase() === '.true.',
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
  const materials = materialInput?.key === sourceKey ? materialInput.rows : null;
  const setMaterials = (rows: ReturnType<typeof materialRows> | null) =>
    setMaterialInput(rows ? { key: sourceKey, rows } : null);
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
  const controls: UiField[] = [];
  function write(change: () => Overlay) {
    if (locked || !writable.current || savingNow.current || current.current !== request)
      throw new Error(bilingual('当前配置不可修改', 'Configuration is locked or stale'));
    try {
      const next = change();
      if (onCaseChange) {
        const patch = caseOptionPatch(overlay, next, fields);
        const edited = Object.keys(patch).filter((key) => patch[key] !== null);
        const rememberEdits = () => {
          if (edited.length)
            setCaseEdits((previous) => ({
              scope: editScope,
              fields: [
                ...new Set([...(previous?.scope === editScope ? previous.fields : []), ...edited]),
              ],
            }));
        };
        if (modalActions) {
          onCaseChange(patch);
          rememberEdits();
        } else {
          savingNow.current = true;
          setSaving(true);
          void Promise.resolve()
            .then(() => onCaseChange(patch))
            .then(() => {
              rememberEdits();
              setError('');
            })
            .catch((e) => setError(String(e)))
            .finally(() => {
              savingNow.current = false;
              setSaving(false);
            });
        }
      } else onChange!(next);
      setError('');
    } catch (e) {
      setError(String(e));
      throw e;
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
        'DEF_ISO_USE_FRACTIONATION',
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
      id = `configuration-${field.name}`,
      // 同一页里有很多行都有"恢复默认/选择文件"，助手的动作说明带上字段名才分得清。
      fieldName = fieldLabel(field.name, lang);
    async function pickFile() {
      if (
        cannot ||
        locked ||
        !writable.current ||
        pickingNow.current ||
        !mounted.current ||
        current.current !== request ||
        currentHost.current !== runHost
      )
        throw new Error(bilingual('当前配置不可修改', 'Configuration is locked or stale'));
      pickingNow.current = true;
      setPickingFile(true);
      try {
        const path = runHost
          ? await new Promise<string | null>((resolve) => {
              remoteResolve.current = resolve;
              const raw = unquote(value);
              setRemotePicker({
                host: runHost,
                initialPath: raw.startsWith('/')
                  ? raw.slice(0, raw.lastIndexOf('/')) || '/'
                  : undefined,
              });
            })
          : await invoke<string | null>('pick_file', { key: field.name, filter: null });
        if (
          path &&
          mounted.current &&
          writable.current &&
          current.current === request &&
          currentHost.current === runHost
        ) {
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
    const reason = configurationReason(context?.reason, en);
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
          <ActionButton
            action={`${id}-preset`}
            label={`${t('使用预设路径', 'Use preset path')} · ${fieldName}`}
            disabled={cannot || locked || !vicDefault}
            run={restoreDefault}
          >
            {t('使用预设路径', 'Use preset path')}
          </ActionButton>
        )}
        {fileInput && (
          <ActionButton
            action={`${id}-browse`}
            label={`${t('选择文件', 'Choose file')} · ${fieldName}`}
            disabled={cannot || locked || pickingFile}
            run={pickFile}
          >
            {t('选择文件', 'Choose file')}
          </ActionButton>
        )}
        {overridden && !vicFile && (
          <ActionButton
            action={`${id}-reset`}
            label={`${fieldName} · ${t('恢复默认', 'Restore default')}`}
            disabled={locked}
            run={() => write(() => resetModelOption(overlay, field.name))}
          >
            {t('恢复默认', 'Restore default')}
          </ActionButton>
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
    const closed = (name: string) =>
      ['disabled', 'readonly', 'read-only'].includes(
        values.fields.find((f) => f.name === name)?.mode ?? '',
      );
    // 城市算例里 WUE 在运行期被两个引擎关掉：只灰掉 WUE 这一项，按实际运行的方案显示，
    // 其余两种照常可选（原来 WUE 照常可选可存，界面一直显示 WUE，GUI 审查 2026-10-22）。
    const wueClosed = closed('DEF_USE_WUEST');
    const wueReason =
      configurationReason(values.fields.find((f) => f.name === 'DEF_USE_WUEST')?.reason, en) ??
      t('关闭原因尚未核实', 'The reason for disabling WUE has not been verified');
    const flags = stomatalFields.map((name) => {
      if (name === 'DEF_USE_WUEST' && wueClosed) return '.false.';
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
    const unavailable = !ready || closed('DEF_USE_MEDLYNST');
    const options = [
      ['BALL_BERRY', 'Ball–Berry'],
      ['MEDLYN', 'Medlyn'],
      ['WUE', t('水分利用效率 WUE', 'Water-use efficiency WUE')],
    ].filter(([value]) => value !== 'WUE' || !wueClosed);
    const set = (value: string) => {
      if (unavailable || (value === 'WUE' && wueClosed))
        throw new Error('Stomatal scheme unavailable');
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
        {wueClosed && wueReason && (
          <p className="muted">{t(`WUE：${wueReason}`, `WUE: ${wueReason}`)}</p>
        )}
        {!selected && (
          <p className="muted">
            {t(
              '开关值冲突或缺少可用默认值，请核对。',
              'Switches conflict or no usable defaults are available; check the configuration.',
            )}
          </p>
        )}
        {overridden && (
          <ActionButton
            action="configuration-stomatal-reset"
            label={t('气孔导度方案 · 恢复默认', 'Stomatal scheme · Restore default')}
            disabled={locked}
            run={reset}
          >
            {t('恢复默认', 'Restore default')}
          </ActionButton>
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
    'material',
    'assimilation',
    'other',
  ].map((id) => modelSettingGroups.find((g) => g[0] === id)!);
  const caseProcessVisible = (id: string) =>
    (!hideProcesses || !['ecology', 'material', 'river', 'urban'].includes(id)) &&
    (!process || id !== 'material' || process === 'material') &&
    (!onCaseChange || parentValues[id] === true || (id === 'material' && process === 'material'));
  // 同位素专属字段是否显示：编辑中的表格优先，其次看当前配置里的 family 开关。
  const currentIsotope = materials
    ? materials.some((row) => row.TYPES.toLowerCase() === 'isotope')
    : (() => {
        const raw =
          overlay.physics.DEF_USE_ISOTOPE ??
          values.fields.find((field) => field.name === 'DEF_USE_ISOTOPE')?.configured_value ??
          values.fields.find((field) => field.name === 'DEF_USE_ISOTOPE')?.built_in_default;
        return raw === undefined || raw === null
          ? undefined
          : raw.trim().toLowerCase() === '.true.';
      })();
  const groups = orderedGroups.map(([id, zh, english]) => {
    if (['river', 'urban', 'material'].includes(id) && !caseProcessVisible(id)) return null;
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
      .filter((field) => materialFieldVisible(field.name, currentIsotope))
      .filter(
        (field) =>
          (!onCaseChange &&
            !['DEF_VIC_OPT', 'DEF_file_VIC_para', 'DEF_file_VIC_OPT'].includes(field.name)) ||
          (field.name === 'DEF_RSS_SCHEME' && resistanceForcedOff) ||
          caseFieldVisible(field.name, values.fields, {
            ...overlay.physics,
            ...(currentIsotope === undefined
              ? {}
              : { DEF_USE_ISOTOPE: currentIsotope ? '.true.' : '.false.' }),
          }),
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
    // 没有可设项的分组不显示：版本页原来列出一串"· 0"（2026-10-20 用户反映），算例页早就这样。
    if (
      !rows.length &&
      !(id === 'ecology' && showStomatal) &&
      !(id === 'material' && (caseProcessVisible(id) || !onCaseChange))
    )
      return null;
    if (process)
      return (
        <Card key={id} title={t(zh, english)}>
          {id === 'ecology' && stomatal()}
          {rows.map(row)}
        </Card>
      );
    return (
      <Advanced
        key={id}
        // 折叠标题就是展开/收起动作；配置锁定时界面上按不动，也就不登记（第 737 轮）。
        action={disabled ? undefined : `${groupPrefix}-group-${id}`}
        actionLabel={t(zh, english)}
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
  let storedMaterials: ReturnType<typeof materialRows>,
    materialError = '';
  try {
    storedMaterials = materialRows(overlay);
  } catch (e) {
    storedMaterials = [];
    materialError = String(e);
  }
  const materialOpen = caseProcessVisible('material') && open('material'),
    table = materials ?? storedMaterials,
    materialFields = fields.filter((f) => materialKeys.includes(f.name));
  const materialTooLarge = table.length > 100;
  const materialReady = !materialTooLarge && !locked && ready && materialFields.length > 0;
  const familyContext: FamilyContext = {
    subgrid,
    processFiles: values.process_files,
    kernelMacros:
      kernelMacros ?? state.kernels?.find((item) => item.dir === kernelDir)?.macros ?? null,
    caseScope,
    engine,
    defaults: Object.fromEntries([
      ...fields.map((field) => [field.name.toUpperCase(), field.default]),
      ...values.fields.flatMap((field) => {
        const value = field.configured_value ?? field.built_in_default;
        return value == null ? [] : [[field.name.toUpperCase(), value]];
      }),
    ]),
  };
  let checkedMaterialOverlay = overlay;
  let definitionError = materials === null ? materialError : '';
  if (!definitionError && !materialTooLarge && materialFields.length) {
    try {
      checkedMaterialOverlay = setMaterialRows(overlay, fields, table);
    } catch (e) {
      definitionError = String(e);
    }
  }
  const materialIssues = definitionError
    ? []
    : materialSetupIssues(checkedMaterialOverlay, familyContext);
  const materialValid = !definitionError && !materialTooLarge && materialIssues.length === 0;
  const canSave =
    ready &&
    !error &&
    materials === null &&
    !pickingFile &&
    (process !== 'material' || (materialFields.length > 0 && materialValid));
  useEffect(() => {
    onReady?.(canSave);
  }, [canSave, onReady]);
  const stagedMaterialModal = process === 'material' && !!modalActions && !!onCaseChange;
  const familyVisible = materialOpen;
  const familyCards = familyVisible && (
    <Card
      title={t('先选择要追踪的物质', 'Choose material families')}
      description={t(
        '选择要追踪的物质，保留已有与自定义条目。预填名称、类型和内置过程文件名；请补齐必需的数值及文件路径，再应用定义。',
        'Choose what to trace. Existing and custom rows are retained; names, types and built-in process filenames are prefilled. Complete required numeric values and file paths, then apply the definitions.',
      )}
    >
      <div className="simulation-choice-grid">
        {materialFamilies.map((family) => {
          const availability = materialFamilyState(overlay, family.id, familyContext),
            next = appendMaterialFamily(table, family.id),
            already = next.length === table.length,
            singlepointSediment =
              family.id === 'sediment' &&
              (caseScope === 'site' ||
                familyContext.kernelMacros?.some((macro) => macro.toLowerCase() === 'singlepoint')),
            blocked = !materialReady || already || singlepointSediment;
          const add = () => {
            if (!familyVisible || blocked || current.current !== request)
              throw new Error('Family is unavailable or already present');
            if (stagedMaterialModal) {
              try {
                write(() => applyMaterialSelection(overlay, fields, next));
                setMaterials(null);
              } catch {
                setMaterials(next);
              }
            } else setMaterials(next);
          };
          return (
            <ActionButton
              key={family.id}
              action={`configuration-family-${family.id}`}
              label={t(family.zh, family.en)}
              className="simulation-choice-card"
              variant={already ? 'primary' : 'secondary'}
              aria-pressed={already}
              disabled={blocked}
              run={add}
            >
              <strong>{t(family.zh, family.en)}</strong>
              <span>{family.rows.map(([name]) => name).join(' / ')}</span>
              {family.id === 'methane' && (
                <small>{t('选择甲烷会同时启用 BGC', 'Selecting methane also enables BGC')}</small>
              )}
              {singlepointSediment && (
                <small>
                  {t(
                    '单点算例不支持河道泥沙，请选择空间算例',
                    'River sediment requires a spatial case; single-site cases are unsupported',
                  )}
                </small>
              )}
              {!singlepointSediment && (
                <small>
                  {already
                    ? t('已在定义表中', 'Already in the table')
                    : availability.status === 'blocked'
                      ? t(
                          '可先选择；保存前须解决依赖冲突',
                          'Can prefill; resolve prerequisites before saving',
                        )
                      : availability.status === 'pending'
                        ? t('可以预填；适用性待核对', 'Can prefill; applicability pending')
                        : t('预填到定义表', 'Prefill definition table')}
                </small>
              )}
              {availability.reasons.map(([zh, english]) => (
                <small key={zh}>{t(zh, english)}</small>
              ))}
            </ActionButton>
          );
        })}
      </div>
      <p className="muted">
        {stagedMaterialModal
          ? t(
              '选择预设后暂存到此弹窗，点击“保存并启用”才保存。手动编辑或移除行后请应用定义；取消弹窗会放弃修改。',
              'Presets are staged in this dialog until Save and enable. Apply definitions after manual edits or removing rows; cancelling discards changes.',
            )
          : t(
              '预填后尚未保存；编辑或移除行后请应用定义。关闭材料开关不会删除已有定义。',
              'Prefilled rows are not saved. Apply definitions after editing or removing rows. Turning materials off preserves saved definitions.',
            )}
      </p>
    </Card>
  );
  function removeMaterial(index: number) {
    if (!materialReady || !materialOpen || current.current !== request)
      throw new Error('Material editor is locked or stale');
    setMaterials(table.filter((_, rowIndex) => rowIndex !== index));
  }
  const materialTable = materialOpen && (
    <Card
      title={t('材料定义（每行对应同一个材料）', 'Material definitions (aligned rows)')}
      description={t(
        '每行对应一个材料。参数文件可填写内置文件名或运行位置上的有效路径；数值列留空时使用内核及参数文件中的定义；水同位素需设置正的含水层混合水量。',
        'Each row defines one material. Parameter files may use built-in filenames or valid paths on the run host. Empty numeric columns use kernel and parameter-file definitions; water isotopes require positive aquifer mixing water.',
      )}
    >
      {definitionError && <Notice tone="error">{definitionError}</Notice>}
      {materialIssues.map(([zh, english]) => (
        <Notice key={zh} tone="warning">
          {t(zh, english)}
        </Notice>
      ))}
      {materialTooLarge && (
        <Notice tone="warning">
          {t(
            '超过100个材料的定义仅展示前100项，请通过原始字段编辑；此表不会覆盖完整定义。',
            'Over 100 materials: only the first 100 are shown. Edit the raw fields; this table cannot replace the full definition.',
          )}
        </Notice>
      )}
      <div className="table-scroll">
        <table>
          <thead>
            <tr>
              {materialColumns.map((c) => (
                <th key={c}>{t(...materialLabels[c])}</th>
              ))}
              <th>{t('操作', 'Actions')}</th>
            </tr>
          </thead>
          <tbody>
            {table.map((row, index) => (
              <tr key={index}>
                {materialColumns.map((column) => {
                  const id = `configuration-material-${index}-${column}`,
                    set = (value: string) => {
                      if (!materialReady) throw new Error('Material editor is locked');
                      setMaterials(
                        table.map((r, i) => (i === index ? { ...r, [column]: value } : r)),
                      );
                    };
                  controls.push({
                    id,
                    label: `${index + 1} ${t(...materialLabels[column])}`,
                    value: row[column],
                    type: column === 'TYPES' ? 'choice' : 'text',
                    options:
                      column === 'TYPES'
                        ? materialTypes.map(([value, zh, english]) => ({
                            value,
                            label: t(zh, english),
                          }))
                        : undefined,
                    disabled: !materialReady,
                    set,
                  });
                  return (
                    <td key={column}>
                      {column === 'TYPES' ? (
                        <Select
                          disabled={!materialReady}
                          aria-label={`${index + 1} ${t(...materialLabels[column])}`}
                          value={row[column]}
                          onChange={(e) => set(e.target.value)}
                        >
                          <option value="">{t('选择类型', 'Choose type')}</option>
                          {row[column] && !materialTypes.some(([kind]) => kind === row[column]) && (
                            <option value={row[column]}>
                              {row[column]} · {t('当前源码类型待核对', 'Source type pending')}
                            </option>
                          )}
                          {materialTypes.map(([value, zh, english]) => (
                            <option value={value} key={value}>
                              {t(zh, english)}
                            </option>
                          ))}
                        </Select>
                      ) : (
                        <Input
                          disabled={!materialReady}
                          aria-label={`${index + 1} ${column}`}
                          value={row[column]}
                          onChange={(e) => set(e.target.value)}
                        />
                      )}
                    </td>
                  );
                })}
                <td>
                  <ActionButton
                    action={`configuration-material-${index}-remove`}
                    label={t(`移除 ${row.NAMES || index + 1}`, `Remove ${row.NAMES || index + 1}`)}
                    disabled={!materialReady}
                    run={() => removeMaterial(index)}
                    aria-label={t(
                      `移除 ${row.NAMES || index + 1}`,
                      `Remove ${row.NAMES || index + 1}`,
                    )}
                  >
                    {t('移除', 'Remove')}
                  </ActionButton>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <div className="button-row">
        <ActionButton
          action="configuration-materials-add"
          label={t('增加材料', 'Add material')}
          disabled={!materialReady}
          run={() =>
            setMaterials([
              ...table,
              Object.fromEntries(materialColumns.map((c) => [c, ''])) as (typeof table)[number],
            ])
          }
        >
          {t('增加材料', 'Add material')}
        </ActionButton>
        <ActionButton
          action="configuration-materials-remove"
          label={t('移除末行', 'Remove last row')}
          disabled={!materialReady || !table.length}
          run={() => setMaterials(table.slice(0, -1))}
        >
          {t('移除末行', 'Remove last row')}
        </ActionButton>
        <ActionButton
          action="configuration-materials-apply"
          label={t('应用材料定义', 'Apply material definitions')}
          disabled={!materialReady || materials === null}
          run={() => {
            write(() => applyMaterialSelection(overlay, fields, table));
            setMaterials(null);
          }}
        >
          {t('应用材料定义', 'Apply material definitions')}
        </ActionButton>
      </div>
      <p>
        {t('材料数量', 'Material count')}: {table.length}
      </p>
      <p>
        {t(
          '尚未应用的表格修改不会写入草稿。',
          'Table changes are not written to the draft until applied.',
        )}
      </p>
    </Card>
  );
  // 水同位素的地下含水层混合水量：必须 >0 才能保存，但后端在 `DEF_USE_ISOTOPE`
  // 尚未保存为 true 时把它标成 hidden（“只有水同位素使用”），对话框里本来没有
  // 输入框。选中同位素族时在这里显式渲染，默认 1000 mm 测试值（历史设计），
  // 用户可改；不渲染就会首次启用时被自身校验卡死（实测 2026-11-12）。
  const isotopeMixingVisible =
    process === 'material' &&
    (currentIsotope === true || table.some((row) => row.TYPES.trim().toLowerCase() === 'isotope'));
  const aquiferMixing =
    modelSettingValue(overlay, AQUIFER_MIXING_WATER) ??
    values.fields.find((field) => field.name === AQUIFER_MIXING_WATER)?.configured_value ??
    values.fields.find((field) => field.name === AQUIFER_MIXING_WATER)?.built_in_default ??
    AQUIFER_MIXING_WATER_TEST_MM;
  const setAquiferMixing = (value: string) =>
    write(() => setModelOption(overlay, fields, AQUIFER_MIXING_WATER, value));
  if (isotopeMixingVisible)
    controls.push({
      id: 'configuration-aquifer-mixing',
      label: t('含水层混合水量（mm）', 'Aquifer mixing water (mm)'),
      value: aquiferMixing,
      type: 'text',
      disabled: locked || !ready,
      set: (value) => {
        if (locked) throw new Error(bilingual('当前配置不可修改', 'Configuration is locked'));
        setAquiferMixing(value);
      },
    });
  const isotopeMixingCard = isotopeMixingVisible && (
    <Card
      title={t('水同位素混合参数', 'Water isotope mixing')}
      description={t(
        '通用水输运的同位素需要正的含水层混合水量；默认 1000 mm 是测试值，正式模拟请用实测或率定值。',
        'Water isotopes need positive aquifer mixing water. The 1000 mm default is a test value; use measured or calibrated values for real simulations.',
      )}
    >
      <Field label={t('含水层混合水量（mm）', 'Aquifer mixing water (mm)')}>
        <Input
          id="configuration-aquifer-mixing"
          aria-label={t('含水层混合水量（mm）', 'Aquifer mixing water (mm)')}
          value={aquiferMixing}
          disabled={locked || !ready}
          onChange={(event) => setAquiferMixing(event.target.value)}
        />
      </Field>
      <small>
        {AQUIFER_MIXING_WATER} ·{' '}
        {t('默认 1000 mm 仅供测试', 'The 1000 mm default is for testing only')}
      </small>
    </Card>
  );
  const assets = (caseProcessVisible('material') ? values.process_files : []).map((asset) => {
    const isOpen = opened[`${sourceKey}:${asset.file}`] ?? materialOpen;
    return (
      <Advanced
        key={asset.file}
        action={disabled ? undefined : `configuration-asset-${asset.file}`}
        actionLabel={asset.file}
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
                <ActionButton
                  action={`${id}-reset`}
                  label={`${fieldLabel(entry.path, lang)} · ${t('恢复资产原值', 'Restore asset value')}`}
                  disabled={locked || cannot}
                  run={reset}
                >
                  {t('恢复资产原值', 'Restore asset value')}
                </ActionButton>
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
          {configurationReason(entry.reason, en) ||
            t('待按冻结源码核对', 'Pending frozen-source verification')}{' '}
          · {t('目录参考', 'Catalog reference')}: {entry.default ?? '—'}
        </small>
        {overridden && (
          <ActionButton
            action={`${id}-reset`}
            label={`${entry.name} · ${t('恢复默认', 'Restore default')}`}
            disabled={!!onCaseChange || locked || !descriptor || !selection}
            run={reset}
          >
            {t('恢复默认', 'Restore default')}
          </ActionButton>
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
  }
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
    // 动作由界面上的按钮自己登记（第 737 轮）；modalActions 只表示"这是对话框里的一份配置"。
    actions: [],
    status: error || (!ready ? 'loading' : 'pending_source_verification'),
    details: { context_complete: values.context_complete, warnings: values.warnings },
  });
  return (
    <div className="configuration-options">
      {remotePicker && (
        <RemotePathBrowser
          host={remotePicker.host}
          initialPath={remotePicker.initialPath}
          directory={false}
          en={en}
          onSelect={(path) => closeRemotePicker(path)}
          onClose={() => closeRemotePicker(null)}
        />
      )}
      <Notice>
        {t(
          '未单独设置时显示默认值；修改并保存后覆盖默认值。启用过程后显示相关配置，关闭过程保留已有设置。',
          'Defaults are shown when no override is set; saved edits override them. Enable a process to see its settings. Disabling it preserves saved values.',
        )}
      </Notice>
      {ready && legacyCaseOverrideEntries(values).length > 0 && (
        <Notice tone="warning">
          {t(
            `该模型的旧定义里有 ${legacyCaseOverrideEntries(values).length} 个参数已改为算例拥有，已按历史值迁移并应用到本次运行：${legacyCaseOverrideEntries(
              values,
            )
              .map(([key, value]) => `${key}=${value}`)
              .join(', ')}`,
            `${legacyCaseOverrideEntries(values).length} legacy model parameter(s) now belong to the case; their historical values were migrated and applied to this run: ${legacyCaseOverrideEntries(
              values,
            )
              .map(([key, value]) => `${key}=${value}`)
              .join(', ')}`,
          )}
        </Notice>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      {!ready && <p>{t('正在查询配置条件…', 'Checking configuration conditions…')}</p>}
      {!stagedMaterialModal && groups}
      {familyCards}
      {materialTable}
      {isotopeMixingCard}
      {stagedMaterialModal && groups}
      {modalActions && materials !== null && (
        <Notice>
          {t(
            '请先应用材料定义，再保存设置。',
            'Apply material definitions before saving settings.',
          )}
        </Notice>
      )}
      {caseProcessVisible('material') && (
        <Card title={t('材料过程文件', 'Material process files')}>
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
              <ActionButton
                action="configuration-process-import"
                label={t('选择并导入过程文件', 'Choose and import process file')}
                kind="commit"
                disabled={locked || !processName.trim()}
                run={importProcess}
              >
                {t('选择并导入过程文件', 'Choose and import process file')}
              </ActionButton>
            </div>
          )}
        </Card>
      )}
      {pftVisible && (
        <Advanced
          action={disabled ? undefined : 'configuration-pft-toggle'}
          actionLabel={t('按植物类型调整参数', 'Adjust parameters by plant type')}
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
                    '请选择当前编译配置支持的植物类型。',
                    'Choose a plant type supported by the current build configuration.',
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
