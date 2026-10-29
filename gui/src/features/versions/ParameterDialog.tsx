import { useEffect, useMemo, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import type { Overlay } from '../../lib/types';
import { Dialog, Input, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
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
/** 示踪物标准参数文件里版本能改的参数（后端 `standard_process_parameters`）。 */
type StandardFile = {
  file: string;
  parameters: { id: string; path: string; value: string; group: string; kind: string }[];
};
const tracerFiles: Record<string, [string, string]> = {
  'standard_sediment_parameter.nml': ['河道泥沙', 'River sediment'],
  'standard_methane_parameter.nml': ['甲烷 CH4', 'Methane CH4'],
  'standard_isotope_parameter.nml': ['水同位素', 'Water isotopes'],
  'standard_chloride_parameter.nml': ['氯离子 Cl', 'Chloride Cl'],
  'standard_nh4_parameter.nml': ['铵盐 NH4', 'Ammonium NH4'],
  'standard_no3_parameter.nml': ['硝酸盐 NO3', 'Nitrate NO3'],
  // 旧算例可能仍引用拆开前的单物种文件；保留标签让迁移前的算例也能读到名字。
  'standard_ch4_parameter.nml': ['甲烷 CH4（旧）', 'Methane CH4 (legacy)'],
  'standard_O18_parameter.nml': ['水同位素 H₂¹⁸O（旧）', 'Water isotope H₂¹⁸O (legacy)'],
  'standard_HDO_parameter.nml': ['水同位素 HDO（旧）', 'Water isotope HDO (legacy)'],
};
/**
 * 材质输运共用的设置（DEF_ISO_/DEF_MT_/DEF_SOLUTE_*，如 Craig–Gordon 相对湿度上限、非稳态叶片水）：
 * 存法是普通 namelist 参数，但只跟示踪物有关，放在"示踪物与泥沙"页，不放全局参数。
 */
const sharedTracer = (key: string) =>
  /^DEF_(ISO_|ISOTOPE_|MT_|SOLUTE_|METHANE_|SEDIMENT_|USE_ISOTOPE$|USE_METHANE$|USE_SEDIMENT$)/i.test(
    key,
  );
/** namelist 里的一组数显示成逗号隔开，和输入时的写法一样。 */
const listText = (value: string) => value.trim().split(/\s+/).join(', ');
const SHARED = 'shared-tracer-settings';
const tracerGroups: Record<string, [string, string]> = {
  nl_colm_isotope_parameter: ['同位素参数', 'Isotope parameters'],
  nl_colm_methane_parameter: ['甲烷参数', 'Methane parameters'],
  nl_colm_sediment_parameter: ['泥沙参数', 'Sediment parameters'],
  nl_colm_solute_parameter: ['溶质参数', 'Solute parameters'],
  nl_colm_tracer_parameter: ['通用示踪参数（旧）', 'Generic tracer parameters (legacy)'],
  nl_colm_mt_forcing: ['同位素强迫', 'Isotope forcing'],
};

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
  // 有的参数是一组数（泥沙各粒级的粒径），逐个检查；逗号、空格隔开都认。
  const items = text.trim().split(/[\s,]+/);
  if (items.length > 1) return items.map((item) => valueProblem(d, item, en)).find(Boolean) ?? '';
  const value = literalNumber(text);
  if (value == null) return t('不是数', 'Not a number');
  if (d.value_kind === 'integer' && !Number.isInteger(value))
    return t('须为整数', 'Must be an integer');
  const min = d.validation.min == null ? null : literalNumber(d.validation.min);
  const max = d.validation.max == null ? null : literalNumber(d.validation.max);
  if (min != null && value < min)
    return tinyAbove(min)
      ? t('必须大于 0', 'Must be above 0')
      : t(`不能小于 ${show(min)}`, `Below ${show(min)}`);
  // 开区间端点本身也不行（`GRADM` 要 > 1.6）：原来 1.6 能填，保存或运行时才被拒。
  if (min != null && d.validation.min_exclusive && value === min)
    return t(`必须大于 ${show(min)}`, `Must be above ${show(min)}`);
  if (max != null && value > max)
    return tinyBelow(max)
      ? t('必须小于 0', 'Must be below 0')
      : t(`不能大于 ${show(max)}`, `Above ${show(max)}`);
  if (max != null && d.validation.max_exclusive && value === max)
    return t(`必须小于 ${show(max)}`, `Must be below ${show(max)}`);
  return '';
}

// 目录把"必须 > 0 / < 0"写成 ±最小正双精度（2.2e-308），字符串是展开的三百多位小数，原样显示就是
// "中间好多个 0"（2026-10-21 用户反映叶片特征尺寸倒平方根、根 50% 失导水势）。
const tinyAbove = (v: number) => v > 0 && v < 1e-300;
const tinyBelow = (v: number) => v < 0 && v > -1e-300;

/** 参数说明；目录里没写说明的，后端填的是"CoLM 配置字段 X。"，只重复键名，不显示。 */
export function docOf(d: Descriptor, en: boolean): string {
  const doc = (en ? d.doc_en : d.doc_zh) ?? '';
  return /^(CoLM 配置字段 \S+。|CoLM configuration field \S+\.?)$/.test(doc.trim()) ? '' : doc;
}

/** 参数的取值范围，给人看的写法：`> 0`、`< 0`、`≥ 1`、`0 … 1`。没有范围时为空。 */
export function rangeText(d: Descriptor, en: boolean): string {
  const min = d.validation.min == null ? null : literalNumber(d.validation.min);
  const max = d.validation.max == null ? null : literalNumber(d.validation.max);
  const openMin = !!d.validation.min_exclusive,
    openMax = !!d.validation.max_exclusive;
  if (min != null && max != null && !tinyAbove(min) && !tinyBelow(max) && !openMin && !openMax)
    return `${show(min)} … ${show(max)}`;
  const lower = min == null ? null : tinyAbove(min) ? '> 0' : `${openMin ? '>' : '≥'} ${show(min)}`;
  const upper = max == null ? null : tinyBelow(max) ? '< 0' : `${openMax ? '<' : '≤'} ${show(max)}`;
  return [lower, upper].filter(Boolean).join(en ? ', ' : '，');
}

const show = (value: number | null | undefined) =>
  value == null
    ? '—'
    : Number.isInteger(value)
      ? String(value)
      : String(Number(value.toPrecision(6)));

/** 目录里的默认值是 Fortran 写法（`5._r8`），显示成普通的数。 */
const plain = (text: string | null) => {
  if (text == null) return '—';
  const value = literalNumber(text);
  return value == null ? text : show(value);
};

// 0 起编号里 15 是 C3 作物、16 是灌溉 C3：16 起才要 CROP 编译配置（后端 `index > 15` 才放行，ConfigurationOptions
// 也是 `<= 15`）。原来这里写 `<= 16`、文案写 17–78，不勾"作物"仍能改 16 号，冻结或运行时才被拒（GUI 审查 VE#3）。
export const typesOf = (type: ParameterType, crops: boolean): [number, string, string][] =>
  type === 'igbp' ? IGBP : type === 'usgs' ? USGS : PFT.filter(([id]) => crops || id <= 15);

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
  onImportProcessFile,
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
  /** 把自己的参数文件整份带进版本（专家用；标准文件不用导入）。 */
  onImportProcessFile?: () => void;
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
  const [problem, setProblem] = useState('');
  const [defaults, setDefaults] = useState<Record<string, TypeDefault[]>>({});
  useEffect(() => {
    if (open) setType(initialType);
  }, [open, initialType]);
  const [standard, setStandard] = useState<StandardFile[] | null>(null);
  useEffect(() => {
    if (!open || type !== 'process' || standard) return;
    let live = true;
    invokeJson<StandardFile[]>('standard_process_parameters')
      .then((files) => {
        if (live) setStandard(files);
      })
      .catch((e) => {
        if (live) setProblem(String(e));
      });
    return () => {
      live = false;
    };
  }, [open, type, standard]);
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
  const ofType = useMemo(
    () =>
      catalog.filter(
        (d) => parameterType(d) === type && !(type === 'global' && sharedTracer(d.raw_key)),
      ),
    [catalog, type],
  );
  const sharedTracerParams = catalog.filter(
    (d) => parameterType(d) === 'global' && sharedTracer(d.raw_key),
  );
  const records = overlay.parameters;
  const globalPhysics = Object.keys(overlay.physics).filter((k) =>
    catalog.some(
      (d) => parameterType(d) === 'global' && d.raw_key.toLowerCase() === k.toLowerCase(),
    ),
  );
  // 示踪物共用的设置算在"示踪物与泥沙"页上。
  const sharedChanged =
    records.filter((r) => recordType(r) === 'global' && sharedTracer(r.raw_key)).length +
    globalPhysics.filter(sharedTracer).length;
  const changedIn = (kind: ParameterType) =>
    kind === 'global'
      ? records.filter((r) => recordType(r) === 'global' && !sharedTracer(r.raw_key)).length +
        globalPhysics.filter((k) => !sharedTracer(k)).length
      : records.filter((r) => recordType(r) === kind).length +
        (kind === 'process' ? sharedChanged : 0);
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
      // 一组数统一写成逗号隔开的 namelist 写法。
      const literal = ['real', 'integer'].includes(d.value_kind)
        ? value.split(/[\s,]+/).join(', ')
        : value;
      parameters = upsertParameter(parameters, makeRecord(d, literal, index, file));
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
    const kind = parameterType(d);
    const current = ['igbp', 'usgs', 'pft', 'pc'].includes(kind)
      ? recordOf(d, index)?.value
      : kind === 'process'
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
  // 示踪物（水同位素、甲烷、氯离子、河道泥沙）各自参数文件里的参数，放在修改参数里一起改，不再要先导入文件
  // （2026-10-21 用户：示踪物与过程文件为什么不一起做到修改参数里，河道泥沙也要支撑，所有尺度都能用）。
  const tabs: [ParameterType, string][] = [
    ['global', t('全局参数', 'Global')],
    ['igbp', 'IGBP'],
    ['usgs', 'USGS'],
    ['pft', 'PFT'],
    ['pc', 'PC'],
    ['process', t('示踪物与泥沙', 'Tracers and sediment')],
  ];
  // 可选的文件：标准参数文件，加上版本自己带着的其他参数文件。
  const ownFiles = overlay.process_files
    .map((a) => a.relative_path)
    .filter((path) => !tracerFiles[path]);
  // 河道泥沙排第一；共用设置排在各示踪物后面。
  const standardFiles = [...(standard ?? [])].sort(
    (a, b) => Object.keys(tracerFiles).indexOf(a.file) - Object.keys(tracerFiles).indexOf(b.file),
  );
  const fileChoices = [
    ...standardFiles.map((f) => f.file),
    ...(sharedTracerParams.length ? [SHARED] : []),
    ...ownFiles,
  ];
  const chosenFile = processFile || fileChoices[0] || '';
  const chosenStandard = standard?.find((f) => f.file === chosenFile);
  const meta = (d: Descriptor) => [d.unit, rangeText(d, en)].filter(Boolean).join(' · ');
  return (
    <Dialog
      open={open}
      onOpenChange={onOpenChange}
      // 对话框自带的"关闭"也是助手能按的动作（第 737 轮）。
      closeAction="version-parameters-close"
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
          <ActionButton
            key={id}
            bare
            action={`version-parameters-tab-${id}`}
            label={text}
            role="tab"
            aria-selected={type === id}
            className={`versions-tab${type === id ? ' is-active' : ''}`}
            run={() => setType(id)}
          >
            {text}
            {changedIn(id) > 0 && <span className="param-count">{changedIn(id)}</span>}
          </ActionButton>
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
              <ActionButton
                key={id}
                bare
                action={`version-parameters-mode-${id}`}
                label={text}
                role="radio"
                aria-checked={mode === id}
                className={`versions-tab${mode === id ? ' is-active' : ''}`}
                run={() => setMode(id)}
              >
                {text}
              </ActionButton>
            ))}
          </div>
        )}
        {(type === 'pft' || type === 'pc') && (
          <label className="param-check">
            <input type="checkbox" checked={crops} onChange={(e) => setCrops(e.target.checked)} />
            {t('显示作物类型（16–78）', 'Show crop types (16–78)')}
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
      {(type === 'igbp' || type === 'usgs') && (
        // 2026-10-21 起 vendor 把地类表改值改成逐地类、各种尺度都生效（原来只在单点、只改本站地类）。
        <p className="muted param-note">
          {t(
            '改哪个地类，就作用在那个地类的所有 patch 上：单点、多站点、流域、区域、全球都一样，只在内核是这套分类时起作用。气孔参数（Ball–Berry、Medlyn、WUE）也按地类改；全局参数里还有一组"全部类型统一"的，写了就盖过这里各地类的值。',
            'A change to a class applies to every patch of that class, at every scale (single point, multi-site, watershed, regional, global), whenever the kernel uses this classification. The stomatal parameters (Ball–Berry, Medlyn, WUE) are per class too; Global also has an "all types" set that, when written, overrides every class here.',
          )}
        </p>
      )}
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
              {[...listed]
                .filter((d) => !onlyChanged || recordOf(d) || physicsOf(d) != null)
                .sort((a, b) =>
                  `${a.section}|${subgroup(a)}`.localeCompare(`${b.section}|${subgroup(b)}`),
                )
                .flatMap((d, i, rows) => {
                  const heading = `${d.section} · ${subgroup(d)}`;
                  const first =
                    i === 0 || heading !== `${rows[i - 1]!.section} · ${subgroup(rows[i - 1]!)}`;
                  return [
                    ...(first
                      ? [
                          <tr key={`group:${heading}`} className="param-group">
                            <td colSpan={4}>{d.section === subgroup(d) ? d.section : heading}</td>
                          </tr>,
                        ]
                      : []),
                    <tr key={d.id} title={docOf(d, en) || undefined}>
                      <td>
                        <strong>{label(d)}</strong>
                        <code className="param-key">{d.raw_key}</code>
                      </td>
                      <td className="muted">{plain(d.default)}</td>
                      <td>{cell(d, 0, '', plain(d.default))}</td>
                      <td className="muted">{meta(d)}</td>
                    </tr>,
                  ];
                })}
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
                {docOf(parameter, en) && <p className="param-doc">{docOf(parameter, en)}</p>}
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
                    <tr key={d.id} title={docOf(d, en) || undefined}>
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
        {type === 'process' && (
          <>
            <p className="muted param-note">
              {t(
                '各示踪物参数文件里的参数，标准值是 CoLM 随附的标准参数文件里的值。版本只记你改了的值，运行时改到算例自己的参数文件上：只在开了这个示踪物的模拟里起作用，河道泥沙还要开河道汇流；单点、区域、全球都一样。哪些示踪物参与模拟、甲烷淹没方案、溶质浓度在新建模拟里选。',
                "Parameters in each tracer's parameter file; the standard values come from the files shipped with CoLM. The version records only the values you change, and they are applied to the case's own parameter file at run time: only simulations with that tracer on use them, and river sediment also needs river routing; the same at every scale. Which tracers run, the methane inundation scheme and solute concentrations are chosen when creating a simulation.",
              )}
            </p>
            <div className="param-mode" role="radiogroup">
              {fileChoices.map((file) => {
                const count =
                  file === SHARED
                    ? sharedChanged
                    : records.filter((r) => r.scope_instance.process_file === file).length;
                return (
                  <ActionButton
                    key={file}
                    bare
                    action={`version-parameters-file-${file}`}
                    label={
                      file === SHARED
                        ? t('所有示踪物共用', 'Shared by all tracers')
                        : (tracerFiles[file]?.[en ? 1 : 0] ?? file)
                    }
                    role="radio"
                    aria-checked={chosenFile === file}
                    className={`versions-tab${chosenFile === file ? ' is-active' : ''}`}
                    run={() => setProcessFile(file)}
                  >
                    {file === SHARED
                      ? t('所有示踪物共用', 'Shared by all tracers')
                      : (tracerFiles[file]?.[en ? 1 : 0] ?? file)}
                    {count > 0 && <span className="param-count">{count}</span>}
                  </ActionButton>
                );
              })}
              {editable && onImportProcessFile && (
                // 把专家的参数文件整份带进版本：写盘动作，按 commit 登记。
                <ActionButton
                  bare
                  action="version-import-process"
                  label={t('导入自己的参数文件…', 'Import your own parameter file…')}
                  kind="commit"
                  className="library-action"
                  run={onImportProcessFile}
                >
                  {t('导入自己的参数文件…', 'Import your own parameter file…')}
                </ActionButton>
              )}
            </div>
            {chosenFile === SHARED ? (
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
                  {sharedTracerParams.filter(matches).map((d) => (
                    <tr key={d.id} title={docOf(d, en) || undefined}>
                      <td>
                        <strong>{label(d)}</strong>
                        <code className="param-key">{d.raw_key}</code>
                      </td>
                      <td className="muted">{plain(d.default)}</td>
                      <td>{cell(d, 0, '', plain(d.default))}</td>
                      <td className="muted">{meta(d)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            ) : !standard && !ownFiles.length ? (
              <p className="muted">{t('正在读取标准参数…', 'Reading the standard parameters…')}</p>
            ) : chosenStandard ? (
              <table className="versions-table param-table">
                <thead>
                  <tr>
                    <th>{t('参数', 'Parameter')}</th>
                    <th>{t('标准值', 'Standard')}</th>
                    <th>{t('改值', 'Value')}</th>
                    <th>{t('单位 · 范围', 'Unit · range')}</th>
                  </tr>
                </thead>
                <tbody>
                  {chosenStandard.parameters
                    .flatMap((p) => {
                      const d = catalog.find((x) => x.id === p.id);
                      return d && matches(d) ? [{ p, d }] : [];
                    })
                    .flatMap(({ p, d }, i, rows) => [
                      ...(i === 0 || rows[i - 1]!.p.group !== p.group
                        ? [
                            <tr key={`group:${p.group}`} className="param-group">
                              <td colSpan={4}>{tracerGroups[p.group]?.[en ? 1 : 0] ?? p.group}</td>
                            </tr>,
                          ]
                        : []),
                      <tr key={p.id} title={docOf(d, en) || undefined}>
                        <td>
                          <strong>{label(d)}</strong>
                          <code className="param-key">{d.raw_key}</code>
                        </td>
                        <td className="muted">{listText(p.value)}</td>
                        <td>{cell(d, 0, chosenFile, listText(p.value))}</td>
                        <td className="muted">{meta(d)}</td>
                      </tr>,
                    ])}
                </tbody>
              </table>
            ) : (
              // 版本自己带着的其他参数文件：整份跟着版本走，里面的参数照旧按文件改。
              <table className="versions-table param-table">
                <tbody>
                  {listed.map((d) => (
                    <tr key={d.id}>
                      <td>
                        <strong>{label(d)}</strong>
                        <code className="param-key">{d.raw_key}</code>
                      </td>
                      <td>{cell(d, 0, chosenFile, plain(d.default))}</td>
                      <td className="muted">{meta(d)}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </>
        )}
        {listed.length === 0 && type !== 'process' && (
          <p className="versions-empty">{t('没有符合条件的参数。', 'No parameters match.')}</p>
        )}
      </div>
    </Dialog>
  );
}
