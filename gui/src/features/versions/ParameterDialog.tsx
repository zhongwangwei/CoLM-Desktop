import { useEffect, useMemo, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import type { Overlay } from '../../lib/types';
import { Button, Dialog, Input, Select } from '../../components/ui';
import {
  makeRecord,
  parameterType,
  recordType,
  upsertParameter,
  type Descriptor,
  type ParameterType,
} from './model';
import { IGBP, PFT, USGS } from './scope-labels';

type TypeDefault = { index: number; value: number | null };

/** namelist 字面值读成数：`1.5d-3`、`0.01_r8` 这类写法都认；不是数时为 null。 */
export function literalNumber(text: string): number | null {
  const cleaned = text
    .trim()
    .replace(/_r[48]$/i, '')
    .replace(/[dD]/, 'e');
  if (!cleaned || !/^[-+]?(\d+\.?\d*|\.\d+)([eE][-+]?\d+)?$/.test(cleaned)) return null;
  const value = Number(cleaned);
  return Number.isFinite(value) ? value : null;
}

/** 取值是否在描述给出的范围里；返回问题说明，没问题时为空。 */
export function valueProblem(d: Descriptor, text: string, en: boolean): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  if (d.value_kind === 'logical')
    return /^\.(true|false)\.$/i.test(text.trim())
      ? ''
      : t('只能是 .true. 或 .false.', 'Must be .true. or .false.');
  if (!['real', 'integer'].includes(d.value_kind)) return '';
  const value = literalNumber(text);
  if (value == null) return t('不是数', 'Not a number');
  if (d.value_kind === 'integer' && !Number.isInteger(value))
    return t('须为整数', 'Must be an integer');
  const min = d.validation.min == null ? null : literalNumber(d.validation.min);
  const max = d.validation.max == null ? null : literalNumber(d.validation.max);
  if (min != null && value < min)
    return t(`不能小于 ${d.validation.min}`, `Below ${d.validation.min}`);
  if (max != null && value > max)
    return t(`不能大于 ${d.validation.max}`, `Above ${d.validation.max}`);
  return '';
}

const show = (value: number | null | undefined) =>
  value == null
    ? '—'
    : Number.isInteger(value)
      ? String(value)
      : String(Number(value.toPrecision(6)));

const typesOf = (type: ParameterType, crops: boolean): [number, string, string][] =>
  type === 'igbp' ? IGBP : type === 'usgs' ? USGS : PFT.filter(([id]) => crops || id <= 16);

/**
 * 修改参数：按参数类型分开编辑。全局参数是一张列表；地类、PFT、PC 参数是"参数 × 类型"矩阵，可以
 * 按参数看全部类型，也可以按类型看全部参数——原来一次只能选一个类型填一个值（2026-10-20 用户反映）。
 * 默认值灰显，改过的高亮；每个格子就是一条改值记录，数据格式不变。冻结的版本只读打开。
 */
export default function ParameterDialog({
  open,
  onOpenChange,
  catalog,
  overlay,
  editable,
  en,
  onChange,
  initialType = 'global',
  initialParameter = '',
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  catalog: Descriptor[];
  overlay: Overlay;
  editable: boolean;
  en: boolean;
  onChange: (next: Overlay) => void;
  initialType?: ParameterType;
  /** 打开时直接选中这个参数（改动清单里"在矩阵里看"）。 */
  initialParameter?: string;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [type, setType] = useState<ParameterType>(initialType);
  const [query, setQuery] = useState('');
  const [onlyChanged, setOnlyChanged] = useState(false);
  const [mode, setMode] = useState<'parameter' | 'class'>('parameter');
  const [group, setGroup] = useState('');
  const [crops, setCrops] = useState(false);
  const [chosen, setChosen] = useState('');
  const [typeIndex, setTypeIndex] = useState(1);
  const [processFile, setProcessFile] = useState('');
  const [factor, setFactor] = useState('1.2');
  const [problem, setProblem] = useState('');
  const [defaults, setDefaults] = useState<Record<string, TypeDefault[]>>({});
  useEffect(() => {
    if (open) setType(initialType);
  }, [open, initialType]);
  useEffect(() => {
    setGroup('');
    setChosen(initialParameter);
    setProblem('');
    setTypeIndex(1);
    setMode('parameter');
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [type, open]);
  const label = (d: Descriptor) => (en ? d.label_en : d.label_zh);
  const subgroup = (d: Descriptor) =>
    (en
      ? (d as Descriptor & { subgroup_en?: string }).subgroup_en
      : (d as Descriptor & { subgroup_zh?: string }).subgroup_zh) || d.section;
  const ofType = useMemo(() => catalog.filter((d) => parameterType(d) === type), [catalog, type]);
  const records = overlay.parameters;
  const changedIn = (kind: ParameterType) =>
    records.filter((r) => recordType(r) === kind).length +
    (kind === 'global'
      ? Object.keys(overlay.physics).filter((k) =>
          catalog.some(
            (d) => parameterType(d) === 'global' && d.raw_key.toLowerCase() === k.toLowerCase(),
          ),
        ).length
      : 0);
  const recordOf = (d: Descriptor, index?: number, file?: string) =>
    records.find(
      (r) =>
        r.parameter_id === d.id &&
        (index == null || r.scope_instance.index === index) &&
        (file == null || r.scope_instance.process_file === file),
    );
  const physicsOf = (d: Descriptor) =>
    Object.entries(overlay.physics).find(([k]) => k.toLowerCase() === d.raw_key.toLowerCase())?.[1];
  // 写一个格子：空值就是恢复默认。全局参数也可能以开关改值（按过程配置写的）的形式存在，一并换掉。
  function write(d: Descriptor, index: number, file: string, text: string) {
    if (!editable) return;
    const value = text.trim();
    const kind = parameterType(d);
    const perType = ['igbp', 'usgs', 'pft', 'pc'].includes(kind);
    let parameters = records.filter(
      (r) =>
        !(
          r.parameter_id === d.id &&
          (r.scope_instance.index ?? null) === (perType ? index : null) &&
          (r.scope_instance.process_file ?? null) === (kind === 'process' ? file : null)
        ),
    );
    if (value) {
      const issue = valueProblem(d, value, en);
      if (issue) {
        setProblem(`${d.raw_key}: ${issue}`);
        return;
      }
      parameters = upsertParameter(parameters, makeRecord(d, value, index, file));
    }
    const physics =
      parameterType(d) === 'global'
        ? Object.fromEntries(
            Object.entries(overlay.physics).filter(
              ([k]) => k.toLowerCase() !== d.raw_key.toLowerCase(),
            ),
          )
        : overlay.physics;
    setProblem('');
    onChange({ ...overlay, parameters, physics });
  }
  async function defaultsFor(ids: string[]) {
    const missing = ids.filter((id) => !defaults[id]);
    if (!missing.length) return;
    const loaded = await Promise.all(
      missing.map(
        async (id) =>
          [
            id,
            await invokeJson<TypeDefault[]>('parameter_type_defaults', { parameterId: id }),
          ] as const,
      ),
    );
    setDefaults((previous) => ({ ...previous, ...Object.fromEntries(loaded) }));
  }
  const groups = [...new Set(ofType.map(subgroup))];
  const matches = (d: Descriptor) =>
    (!group || subgroup(d) === group) &&
    (!query ||
      `${d.raw_key} ${d.label_zh} ${d.label_en}`.toLowerCase().includes(query.toLowerCase()));
  const listed = ofType.filter(matches);
  const matrix = ['igbp', 'usgs', 'pft', 'pc'].includes(type);
  const parameter = listed.find((d) => d.id === chosen) ?? listed[0];
  const types = typesOf(type, crops);
  useEffect(() => {
    if (!open || !matrix) return;
    const ids = mode === 'parameter' ? (parameter ? [parameter.id] : []) : listed.map((d) => d.id);
    void defaultsFor(ids).catch((e) => setProblem(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, matrix, mode, parameter?.id, listed.length, group, query, type]);
  const defaultAt = (d: Descriptor, index: number) =>
    defaults[d.id]?.find((x) => x.index === index)?.value ?? null;
  const cell = (d: Descriptor, index: number, file: string, fallback: string) => {
    const current = ['igbp', 'usgs', 'pft', 'pc'].includes(type)
      ? recordOf(d, index)?.value
      : type === 'process'
        ? recordOf(d, undefined, file)?.value
        : (recordOf(d)?.value ?? physicsOf(d));
    return (
      <input
        key={`${d.id}|${index}|${file}|${current ?? ''}`}
        className={`param-input${current != null ? ' is-changed' : ''}`}
        defaultValue={current ?? ''}
        placeholder={fallback}
        disabled={!editable}
        aria-label={`${d.raw_key} ${index}`}
        onBlur={(e) => {
          if ((current ?? '') !== e.target.value.trim()) write(d, index, file, e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === 'Enter') (e.target as HTMLInputElement).blur();
        }}
      />
    );
  };
  const changedCount = (d: Descriptor) => records.filter((r) => r.parameter_id === d.id).length;
  function scaleAll(d: Descriptor) {
    const by = literalNumber(factor);
    if (by == null) return setProblem(t('系数不是数', 'The factor is not a number'));
    let parameters = records;
    for (const [index] of types) {
      // PFT/PC 的 0 号是非植被：整体缩放植被参数时不该带上它。
      if ((type === 'pft' || type === 'pc') && index === 0) continue;
      const base = defaultAt(d, index);
      if (base == null) continue;
      const value = base * by;
      const text =
        d.value_kind === 'integer'
          ? String(Math.round(value))
          : String(Number(value.toPrecision(8)));
      const issue = valueProblem(d, text, en);
      if (issue) return setProblem(`${d.raw_key} · ${index}: ${issue}`);
      parameters = upsertParameter(parameters, makeRecord(d, text, index, ''));
    }
    setProblem('');
    onChange({ ...overlay, parameters });
  }
  function resetAll(d: Descriptor) {
    onChange({ ...overlay, parameters: records.filter((r) => r.parameter_id !== d.id) });
  }
  const tabs: [ParameterType, string][] = [
    ['global', t('全局参数', 'Global')],
    ['igbp', t('地类 · IGBP', 'Land class · IGBP')],
    ['usgs', t('地类 · USGS', 'Land class · USGS')],
    ['pft', 'PFT'],
    ['pc', 'PC'],
    ['process', t('过程文件', 'Process files')],
  ];
  const meta = (d: Descriptor) =>
    [
      d.unit,
      d.validation.min != null || d.validation.max != null
        ? `${d.validation.min ?? '−∞'} … ${d.validation.max ?? '∞'}`
        : null,
    ]
      .filter(Boolean)
      .join(' · ');
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      className="param-dialog"
      title={editable ? t('修改参数', 'Edit parameters') : t('全部参数', 'All parameters')}
      description={
        editable
          ? t(
              '留空就是用默认值。改过的格子高亮；改完点右上角「保存草稿」。',
              'Leave a cell empty for the default. Changed cells are highlighted; save the draft when done.',
            )
          : t('冻结的版本只读。', 'A frozen version is read-only.')
      }
    >
      <div className="param-tabs" role="tablist">
        {tabs.map(([id, text]) => (
          <button
            key={id}
            type="button"
            role="tab"
            aria-selected={type === id}
            className={`versions-tab${type === id ? ' is-active' : ''}`}
            onClick={() => setType(id)}
          >
            {text}
            {changedIn(id) > 0 && <span className="param-count">{changedIn(id)}</span>}
          </button>
        ))}
      </div>
      <div className="param-toolbar">
        <Input
          value={query}
          placeholder={t('搜索名称或变量名', 'Search by name or variable')}
          onChange={(e) => setQuery(e.target.value)}
        />
        {groups.length > 1 && (
          <Select value={group} onChange={(e) => setGroup(e.target.value)}>
            <option value="">{t('全部分组', 'All groups')}</option>
            {groups.map((g) => (
              <option key={g}>{g}</option>
            ))}
          </Select>
        )}
        {matrix && (
          <div className="param-mode" role="radiogroup">
            {(
              [
                ['parameter', t('按参数', 'By parameter')],
                [
                  'class',
                  type === 'pft' || type === 'pc'
                    ? t('按植物类型', 'By plant type')
                    : t('按地类', 'By class'),
                ],
              ] as const
            ).map(([id, text]) => (
              <button
                key={id}
                type="button"
                role="radio"
                aria-checked={mode === id}
                className={`versions-tab${mode === id ? ' is-active' : ''}`}
                onClick={() => setMode(id)}
              >
                {text}
              </button>
            ))}
          </div>
        )}
        {(type === 'pft' || type === 'pc') && (
          <label className="param-check">
            <input type="checkbox" checked={crops} onChange={(e) => setCrops(e.target.checked)} />
            {t('显示作物类型（17–78）', 'Show crop types (17–78)')}
          </label>
        )}
        {type === 'global' && (
          <label className="param-check">
            <input
              type="checkbox"
              checked={onlyChanged}
              onChange={(e) => setOnlyChanged(e.target.checked)}
            />
            {t('只看改过的', 'Changed only')}
          </label>
        )}
      </div>
      {problem && <p className="param-problem">{problem}</p>}
      <div className="param-body">
        {type === 'global' && (
          <table className="versions-table param-table">
            <thead>
              <tr>
                <th>{t('参数', 'Parameter')}</th>
                <th>{t('默认', 'Default')}</th>
                <th>{t('改值', 'Value')}</th>
                <th>{t('单位 · 范围', 'Unit · range')}</th>
              </tr>
            </thead>
            <tbody>
              {listed
                .filter((d) => !onlyChanged || recordOf(d) || physicsOf(d) != null)
                .map((d) => (
                  <tr key={d.id} title={en ? d.doc_en : d.doc_zh}>
                    <td>
                      <strong>{label(d)}</strong>
                      <code className="param-key">{d.raw_key}</code>
                    </td>
                    <td className="muted">{d.default ?? '—'}</td>
                    <td>{cell(d, 0, '', d.default ?? '')}</td>
                    <td className="muted">{meta(d)}</td>
                  </tr>
                ))}
            </tbody>
          </table>
        )}
        {matrix && mode === 'parameter' && (
          <div className="param-split">
            <ul className="param-list">
              {listed.map((d) => (
                <li key={d.id}>
                  <button
                    type="button"
                    className={d.id === parameter?.id ? 'is-active' : undefined}
                    onClick={() => setChosen(d.id)}
                  >
                    <span>{label(d)}</span>
                    <code>{d.raw_key}</code>
                    {changedCount(d) > 0 && <span className="param-count">{changedCount(d)}</span>}
                  </button>
                </li>
              ))}
            </ul>
            {parameter && (
              <div className="param-detail">
                <h3>{label(parameter)}</h3>
                <p className="muted">
                  <code>{parameter.raw_key}</code>
                  {meta(parameter) && ` · ${meta(parameter)}`}
                </p>
                {(en ? parameter.doc_en : parameter.doc_zh) && (
                  <p className="param-doc">{en ? parameter.doc_en : parameter.doc_zh}</p>
                )}
                {editable && (
                  <div className="param-bulk">
                    <span>
                      {type === 'pft' || type === 'pc'
                        ? t('所有植被类型乘以', 'Scale every vegetated type by')
                        : t('全部类型乘以', 'Scale every type by')}
                    </span>
                    <Input value={factor} onChange={(e) => setFactor(e.target.value)} />
                    <Button onClick={() => scaleAll(parameter)}>{t('应用', 'Apply')}</Button>
                    <Button variant="ghost" onClick={() => resetAll(parameter)}>
                      {t('全部恢复默认', 'Reset all')}
                    </Button>
                  </div>
                )}
                <table className="versions-table param-table">
                  <thead>
                    <tr>
                      <th>
                        {type === 'pft' || type === 'pc'
                          ? t('植物类型', 'Plant type')
                          : t('地类', 'Class')}
                      </th>
                      <th>{t('默认', 'Default')}</th>
                      <th>{t('改值', 'Value')}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {types.map(([index, zh, english]) => (
                      <tr key={index}>
                        <td>
                          <span className="param-index">{index}</span> {en ? english : zh}
                        </td>
                        <td className="muted">{show(defaultAt(parameter, index))}</td>
                        <td>{cell(parameter, index, '', show(defaultAt(parameter, index)))}</td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
            )}
          </div>
        )}
        {matrix && mode === 'class' && (
          <div className="param-split">
            <ul className="param-list">
              {types.map(([index, zh, english]) => {
                const count = records.filter(
                  (r) => recordType(r) === type && r.scope_instance.index === index,
                ).length;
                return (
                  <li key={index}>
                    <button
                      type="button"
                      className={index === typeIndex ? 'is-active' : undefined}
                      onClick={() => setTypeIndex(index)}
                    >
                      <span>
                        <span className="param-index">{index}</span> {en ? english : zh}
                      </span>
                      {count > 0 && <span className="param-count">{count}</span>}
                    </button>
                  </li>
                );
              })}
            </ul>
            <div className="param-detail">
              <table className="versions-table param-table">
                <thead>
                  <tr>
                    <th>{t('参数', 'Parameter')}</th>
                    <th>{t('默认', 'Default')}</th>
                    <th>{t('改值', 'Value')}</th>
                    <th>{t('单位 · 范围', 'Unit · range')}</th>
                  </tr>
                </thead>
                <tbody>
                  {listed.map((d) => (
                    <tr key={d.id} title={en ? d.doc_en : d.doc_zh}>
                      <td>
                        <strong>{label(d)}</strong>
                        <code className="param-key">{d.raw_key}</code>
                      </td>
                      <td className="muted">{show(defaultAt(d, typeIndex))}</td>
                      <td>{cell(d, typeIndex, '', show(defaultAt(d, typeIndex)))}</td>
                      <td className="muted">{meta(d)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        )}
        {type === 'process' &&
          (overlay.process_files.length === 0 ? (
            <p className="versions-empty">
              {t(
                '这个版本还没有过程文件。先在「其他模型设置」里导入过程文件，再改它里面的参数。',
                'This version has no process files yet. Import one under “Other model settings” first, then edit its parameters here.',
              )}
            </p>
          ) : (
            <>
              <Select
                value={processFile || overlay.process_files[0]!.relative_path}
                onChange={(e) => setProcessFile(e.target.value)}
              >
                {overlay.process_files.map((a) => (
                  <option key={a.relative_path}>{a.relative_path}</option>
                ))}
              </Select>
              <table className="versions-table param-table">
                <tbody>
                  {listed.map((d) => (
                    <tr key={d.id}>
                      <td>
                        <strong>{label(d)}</strong>
                        <code className="param-key">{d.raw_key}</code>
                      </td>
                      <td>
                        {cell(
                          d,
                          0,
                          processFile || overlay.process_files[0]!.relative_path,
                          d.default ?? '',
                        )}
                      </td>
                      <td className="muted">{meta(d)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </>
          ))}
        {listed.length === 0 && type !== 'process' && (
          <p className="versions-empty">{t('没有符合条件的参数。', 'No parameters match.')}</p>
        )}
      </div>
    </Dialog>
  );
}
