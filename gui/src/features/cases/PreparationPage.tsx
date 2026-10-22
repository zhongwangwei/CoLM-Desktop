import { useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import { requiredNumber, preparedDestination, casePreparationOptions } from './case-model';
interface Slot {
  index: number;
  meaning: string;
  optional: boolean;
  guessed?: string;
  column?: string;
  units?: string;
  wants: string;
  name?: string;
  also_add: string[];
}
interface Imported {
  site: string;
  safe_site: string;
  staged_path: string;
  final_path: string;
  latitude: number;
  longitude: number;
  landtype: number | null;
  timezone_offset_hours: number | null;
}
interface Probe {
  slots?: Slot[];
  suggest_dst?: string;
  time_column?: string;
  site_column?: string;
  latitude_column?: string;
  longitude_column?: string;
  landtype_column?: string;
  utc_offset_column?: string;
  columns?: { name: string }[];
  variables?: (
    | string
    | {
        name: string;
        label: string;
        column?: string;
        qc_column?: string;
        requires_qc: boolean;
      }
  )[];
}
let currentLang = 'zh';
const t = (zh: string, enText: string) => (currentLang === 'en' ? enText : zh);
export default function PreparationPage() {
  const { state, notify, navigate } = useApp();
  currentLang = state.language;
  const [imported, setImported] = useState<Imported[]>([]),
    [mode, setMode] = useState('netcdf'),
    [form, setForm] = useState<Record<string, string>>({
      src: '',
      dst: '',
      lon: '',
      lat: '',
      landtype: '',
      rawdata: '',
      time_column: '',
      site_column: '',
      site_name: '',
      latitude_column: '',
      longitude_column: '',
      landtype_column: '',
      utc_offset_column: '',
      utc_offset: '',
      step_seconds: '',
      era5: '',
      short_gap: '3',
      min_overlap: '48',
      start: '',
      end: '',
      height_v: '',
      height_t: '',
      height_q: '',
    }),
    [probe, setProbe] = useState<Probe | null>(null),
    [slots, setSlots] = useState<Slot[]>([]),
    [busy, setBusy] = useState(false),
    [advanced, setAdvanced] = useState(state.expert),
    [result, setResult] = useState<unknown>(null),
    [error, setError] = useState('');
  const lock = useRef(false);
  // 上次"检查"自动推出的输出路径。换了源文件就作废：原来选 A 检查后再换成 B，输出路径还是
  // A 的推荐名，B 的数据被写进 A 的文件（GUI 审查 2026-10-22，P1-1）。用户自己改过的路径不动。
  const autoDst = useRef('');
  const observationVariables = (probe?.variables ?? []).filter((v) => typeof v !== 'string');
  const sourceNames = [
    ...(probe?.variables ?? []).filter((v): v is string => typeof v === 'string'),
    ...(probe?.columns ?? []).map((v) => v.name),
  ];
  const preparation = casePreparationOptions(state.selectedCase?.profile, state.newCaseSeed);
  const set = (k: string, v: string) => {
    if (k === 'src' && v !== form.src) {
      setProbe(null);
      setSlots([]);
    }
    setForm((f) =>
      k !== 'src' || v === f.src
        ? { ...f, [k]: v }
        : { ...f, src: v, dst: f.dst === autoDst.current ? '' : f.dst },
    );
  };
  const num = (k: string) => requiredNumber(form[k], k);
  const optional = (k: string) => (form[k].trim() ? num(k) : null);
  async function work(fn: () => Promise<unknown>) {
    if (lock.current) return;
    lock.current = true;
    setBusy(true);
    setError('');
    try {
      const value = await fn();
      setResult(value);
      return value;
    } catch (e) {
      setError(String(e));
      // 用户选择"不覆盖"是一个决定，不是失败：页面内说明即可，不推全局错误横幅。否则取消一次覆盖
      // 会在其它页面上挂着一条红色 "Error: 没有覆盖已有的输出文件。"（2026-10-22 窗口实测）。
      // 用属性标记而不是自定义错误类：本文件被测试按源码抽取 function work 执行，catch 里引入新的
      // 自由标识符会让抽取出来的函数在测试上下文里 ReferenceError。
      const declined =
        !!e && typeof e === 'object' && (e as { userDeclined?: boolean }).userDeclined === true;
      if (!declined) notify(String(e), true);
      throw e;
    } finally {
      setBusy(false);
      lock.current = false;
    }
  }
  function changeMode(v: string) {
    setMode(v);
    setImported([]);
    setProbe(null);
    setSlots([]);
    setResult(null);
    setError('');
  }
  async function pick(k: string) {
    await work(async () => {
      const path =
        k === 'rawdata' || (k === 'dst' && ['table', 'observation'].includes(mode))
          ? await invoke<string | null>('pick_folder', { key: `prep-${k}` })
          : await invoke<string | null>('pick_file', { key: `prep-${k}`, filter: null });
      if (path) set(k, path);
      return null;
    });
  }
  async function inspect() {
    return work(async () => {
      const p = await invoke<Probe>(
        mode === 'table'
          ? 'probe_forcing_table'
          : mode === 'observation'
            ? 'probe_observation_table'
            : 'probe_forcing',
        { path: form.src },
      );
      setProbe(p);
      const hasSnow =
        (p.variables ?? []).includes('Snowf') ||
        (p.columns ?? []).some((column) => column.name === 'Snowf');
      setSlots(
        (p.slots ?? []).map((s) => ({
          ...s,
          name: s.guessed ?? s.column ?? '',
          units: s.units ?? '',
          also_add:
            s.index === 4 && (s.guessed ?? s.column) === 'Rainf' && hasSnow ? ['Snowf'] : [],
        })),
      );
      const suggested = p.suggest_dst
        ? ['netcdf', 'gaps'].includes(mode)
          ? preparedDestination(p.suggest_dst, form.src)
          : p.suggest_dst
        : '';
      setForm((f) => {
        const keep = !!f.dst && f.dst !== autoDst.current;
        if (!keep) autoDst.current = suggested;
        return {
          ...f,
          dst: keep ? f.dst : suggested,
          ...Object.fromEntries(
            [
              'time_column',
              'site_column',
              'latitude_column',
              'longitude_column',
              'landtype_column',
              'utc_offset_column',
            ].map((k) => [k, (p as unknown as Record<string, string>)[k] ?? f[k]]),
          ),
        };
      });
      return p;
    });
  }
  function choices() {
    const missing = slots.filter((s) => !s.optional && (!s.name || !s.units));
    if (missing.length)
      throw new Error(
        t(
          `请确认变量与单位：${missing.map((s) => s.meaning).join('、')}`,
          `Confirm variables and units: ${missing.map((s) => s.meaning).join(', ')}`,
        ),
      );
    return slots
      .filter((s) => s.name)
      .map((s) => ({
        index: s.index,
        name: s.name,
        units: s.units,
        also_add: [
          ...new Set(
            s.also_add.map((name) => name.trim()).filter((name) => name && name !== s.name),
          ),
        ],
      }));
  }
  function heights() {
    const v = ['height_v', 'height_t', 'height_q'].map((k) => optional(k));
    if (v.every((x) => x === null)) return null;
    if (v.some((x) => x === null || x <= 0))
      throw new Error(
        t(
          '三项观测高度须一同填写正数',
          'All three observation heights must be positive numbers together',
        ),
      );
    return v;
  }
  function gapOptions() {
    return {
      short_gap: num('short_gap'),
      min_overlap: num('min_overlap'),
      utc_offset: optional('utc_offset'),
      latitude: optional('lat'),
      longitude: optional('lon'),
      era5: form.era5 || null,
    };
  }
  // 产物已存在时后端不覆盖（它可能正被某个算例引用）：问过再带 overwrite 重来。
  async function confirmOverwrite<T>(write: (overwrite: boolean) => Promise<T>): Promise<T> {
    try {
      return await write(false);
    } catch (error) {
      if (!String(error).includes('产物已存在，未覆盖')) throw error;
      const ok = await askConfirm(
        t(
          `输出文件已经存在：${form.dst}。覆盖会改掉引用它的算例所用的强迫场。要覆盖吗？`,
          `The output file already exists: ${form.dst}. Overwriting changes the forcing of any case that uses it. Overwrite?`,
        ),
        { confirmLabel: t('覆盖', 'Overwrite'), danger: true },
      );
      if (!ok)
        throw Object.assign(
          new Error(
            t('已取消：没有覆盖已有的输出文件。', 'Cancelled: the existing output file was kept.'),
            { cause: error },
          ),
          { userDeclined: true },
        );
      return await write(true);
    }
  }
  async function execute() {
    return work(async () => {
      if (!form.dst) throw new Error(t('请选择独立的输出路径', 'Destination path required'));
      if (mode === 'site')
        return invoke('make_site', {
          out: form.dst,
          lon: num('lon'),
          lat: num('lat'),
          landtype: optional('landtype'),
          rawdata: form.rawdata || null,
          mode: preparation.mode,
          crop: preparation.crop,
        });
      if (mode === 'era5')
        return invoke('download_era5land', {
          dst: form.dst,
          latitude: num('lat'),
          longitude: num('lon'),
          start: form.start,
          end: form.end,
        });
      if (!probe)
        throw new Error(t('先检查输入并确认字段', 'Inspect input and confirm fields first'));
      if (mode === 'observation')
        return invoke('convert_observation_table', {
          src: form.src,
          dstDir: form.dst,
          options: {
            time_column: form.time_column,
            site_column: form.site_column || null,
            site_name: form.site_name || null,
            variables: observationVariables
              .filter((v) => v.column)
              .map((v) => ({ name: v.name, column: v.column, qc_column: v.qc_column || null })),
          },
        });
      const mapped = choices();
      if (mode === 'table') {
        const batch = await invoke<Imported[]>('convert_forcing_table', {
          src: form.src,
          dst: form.dst,
          slots: mapped,
          options: {
            time_column: form.time_column,
            site_column: form.site_column || null,
            latitude_column: form.latitude_column || null,
            longitude_column: form.longitude_column || null,
            landtype_column: form.landtype_column || null,
            utc_offset_column: form.utc_offset_column || null,
            utc_offset: optional('utc_offset'),
            latitude: optional('lat'),
            longitude: optional('lon'),
            step_seconds: optional('step_seconds'),
            land_cover_scheme: preparation.classification.toLowerCase(),
            heights: heights(),
          },
        });
        setImported(batch);
        return batch;
      }
      if (mode === 'gaps')
        return confirmOverwrite((overwrite) =>
          invoke('repair_forcing', {
            src: form.src,
            dst: form.dst,
            slots: mapped,
            options: gapOptions(),
            overwrite,
          }),
        );
      return confirmOverwrite((overwrite) =>
        invoke('convert_forcing', {
          src: form.src,
          dst: form.dst,
          slots: mapped,
          heights: heights(),
          overwrite,
        }),
      );
    });
  }
  async function finish(item: Imported) {
    return work(async () => {
      const canonical = slots
        .filter((s) => s.name)
        .map((s) => ({
          index: s.index,
          name: ['Tair', 'Qair', 'Psurf', 'Precip', 'Wind_E', 'Wind_N', 'SWdown', 'LWdown'][
            s.index - 1
          ],
          units: s.wants,
          also_add: [],
        }));
      const report = await invoke<{ unresolved: number }>('repair_forcing', {
        src: item.staged_path,
        dst: item.final_path,
        slots: canonical,
        options: {
          ...gapOptions(),
          latitude: item.latitude,
          longitude: item.longitude,
          utc_offset: item.timezone_offset_hours,
        },
      });
      if (report.unresolved)
        throw new Error(
          t(
            `仍有 ${report.unresolved} 个缺测；请提供 ERA5 后重试`,
            `${report.unresolved} gaps remain; provide ERA5 and retry`,
          ),
        );
      setImported((items) => items.filter((x) => x.staged_path !== item.staged_path));
      return { path: item.final_path, ...report };
    });
  }
  const labels: Record<string, string> = {
    src: t('原始输入文件', 'Source file'),
    dst: t('独立输出路径', 'Destination path'),
    lon: t('经度', 'Longitude'),
    lat: t('纬度', 'Latitude'),
    landtype: t('地类编号', 'Land class'),
    rawdata: t('Rawdata 目录', 'Rawdata directory'),
    time_column: t('时间列', 'Time column'),
    site_column: t('站点列', 'Site column'),
    site_name: t('单站名称', 'Site name'),
    latitude_column: t('纬度列', 'Latitude column'),
    longitude_column: t('经度列', 'Longitude column'),
    landtype_column: t('地类列', 'Land class column'),
    utc_offset_column: t('时区列', 'UTC offset column'),
    utc_offset: t('UTC 偏移（小时）', 'UTC offset (hours)'),
    step_seconds: t('时间步长（秒）', 'Time step (seconds)'),
    era5: t('ERA5 辅助文件', 'ERA5 auxiliary file'),
    short_gap: t('短缺测最大步数', 'Max short-gap steps'),
    min_overlap: t('最少重叠步数', 'Min overlap steps'),
    start: t('开始日期', 'Start date'),
    end: t('结束日期', 'End date'),
    height_v: t('风速观测高度（m）', 'Wind measurement height (m)'),
    height_t: t('温度观测高度（m）', 'Temperature measurement height (m)'),
    height_q: t('湿度观测高度（m）', 'Humidity measurement height (m)'),
  };
  const basic =
    mode === 'site'
      ? ['dst', 'lon', 'lat', 'landtype', 'rawdata']
      : mode === 'era5'
        ? ['dst', 'lon', 'lat', 'start', 'end']
        : [
            'src',
            'dst',
            ...(mode === 'table' || mode === 'observation' ? ['time_column', 'site_column'] : []),
            ...(mode === 'gaps' ? ['era5'] : []),
          ];
  const extra =
    mode === 'table'
      ? [
          'latitude_column',
          'longitude_column',
          'landtype_column',
          'utc_offset_column',
          'utc_offset',
          'lat',
          'lon',
          'step_seconds',
          'height_v',
          'height_t',
          'height_q',
        ]
      : mode === 'observation'
        ? ['site_name']
        : mode === 'netcdf'
          ? ['height_v', 'height_t', 'height_q']
          : mode === 'gaps'
            ? ['short_gap', 'min_overlap', 'utc_offset', 'lat', 'lon']
            : [];
  const fields: UiField[] = [
    {
      id: 'prep-mode',
      label: t('准备任务', 'Preparation task'),
      value: mode,
      type: 'choice',
      options: [
        ['netcdf', t('NetCDF 强迫场', 'NetCDF forcing')],
        ['table', t('表格强迫场（CSV）', 'Tabular forcing (CSV)')],
        ['site', t('制作站点', 'Create site')],
        ['gaps', t('修复缺测', 'Gap-filling')],
        ['era5', t('下载 ERA5-Land', 'Download ERA5-Land')],
        ['observation', t('转换观测', 'Convert observations')],
      ].map(([value, label]) => ({ value, label })),
      disabled: busy,
      set: changeMode,
    },
    ...[...basic, ...(advanced ? extra : [])].map((k) => ({
      id: `prep-${k}`,
      label: labels[k],
      value: form[k],
      type: 'text' as const,
      disabled: busy,
      set: (v: string) => {
        set(k, v);
        if (k === 'src') {
          setProbe(null);
          setSlots([]);
        }
      },
    })),
    ...slots.flatMap((s, i) =>
      (s.index === 4 ? ['name', 'units', 'also_add'] : ['name', 'units']).map((key) => ({
        id: `prep-slot-${s.index}-${key}`,
        label: `${s.meaning} ${key === 'name' ? t('变量名', 'Variable') : key === 'units' ? t('单位', 'Units') : t('附加降水分量（逗号分隔）', 'Additional precipitation components (comma-separated)')}`,
        value: key === 'also_add' ? s.also_add.join(',') : String(s[key as 'name' | 'units'] ?? ''),
        type: 'text' as const,
        disabled: busy,
        set: (v: string) =>
          setSlots((all) =>
            all.map((x, j) =>
              i !== j
                ? x
                : key === 'also_add'
                  ? {
                      ...x,
                      also_add: v.split(','),
                    }
                  : { ...x, [key]: v, ...(key === 'name' ? { also_add: [] } : {}) },
            ),
          ),
      })),
    ),
    ...(mode === 'observation'
      ? observationVariables.flatMap((v, i) =>
          ['column', 'qc_column'].map((key) => ({
            id: `prep-obs-${i}-${key}`,
            label: `${v.label} ${key === 'column' ? t('数据列', 'Data column') : t('质控列', 'QC column')}`,
            value: v[key as 'column' | 'qc_column'] ?? '',
            type: 'text' as const,
            disabled: busy,
            set: (value: string) =>
              setProbe((p) =>
                p
                  ? {
                      ...p,
                      variables: observationVariables.map((x, j) =>
                        i === j ? { ...x, [key]: value } : x,
                      ),
                    }
                  : p,
              ),
          })),
        )
      : []),
  ];
  fields.push({
    id: 'prep-advanced',
    label: t('高级设置', 'Advanced settings'),
    value: String(advanced),
    type: 'switch',
    set: (v) => setAdvanced(v === 'true'),
  });
  if (imported.length)
    fields.push({
      id: 'prep-batch-era5',
      label: t('ERA5 辅助文件', 'ERA5 auxiliary file'),
      value: form.era5,
      type: 'text',
      disabled: busy,
      set: (v) => set('era5', v),
    });
  const canInspect = !['site', 'era5'].includes(mode);
  usePageContract({
    title: t('准备输入数据', 'Prepare inputs'),
    step: 'preparation',
    status: busy ? 'busy' : error || 'ready',
    details: { probe, result, error, imported },
    fields,
    actions: [
      {
        id: 'prep-new-case',
        label: t('用于新算例', 'Use for new case'),
        kind: 'click',
        enabled: !busy,
        run: () => navigate('new-case'),
      },
      ...basic
        .filter(
          (k) =>
            ['src', 'rawdata'].includes(k) ||
            (k === 'dst' && ['table', 'observation'].includes(mode)),
        )
        .map((k) => ({
          id: `prep-pick-${k}`,
          label: t(`选择 ${labels[k]}`, `Choose ${labels[k]}`),
          kind: 'click' as const,
          enabled: !busy,
          run: () => pick(k),
        })),
      ...(canInspect
        ? [
            {
              id: 'prep-inspect',
              label: t('检查输入', 'Inspect input'),
              kind: 'click' as const,
              enabled: !busy && !!form.src,
              run: inspect,
            },
          ]
        : []),
      {
        id: 'prep-execute',
        label: t('生成数据', 'Generate data'),
        kind: 'commit',
        enabled: !busy && !!form.dst && (!canInspect || !!probe),
        run: execute,
      },
      ...imported.map((item, i) => ({
        id: `prep-finish-${i}`,
        label: t(`修复并完成 ${item.site}`, `Fix and finalize ${item.site}`),
        kind: 'commit' as const,
        enabled: !busy,
        run: () => finish(item),
      })),
      ...(mode === 'gaps'
        ? [
            {
              id: 'prep-gaps',
              label: t('检查缺测', 'Inspect gaps'),
              kind: 'click' as const,
              enabled: !busy && !!probe,
              run: () =>
                work(() =>
                  invoke('probe_forcing_gaps', {
                    src: form.src,
                    slots: choices(),
                    options: gapOptions(),
                  }),
                ),
            },
          ]
        : []),
    ],
  });
  const render = (f: UiField) => (
    <Field key={f.id} label={f.label}>
      {f.type === 'choice' ? (
        <Select disabled={busy} value={String(f.value)} onChange={(e) => f.set(e.target.value)}>
          {f.options?.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </Select>
      ) : (
        <Input value={String(f.value)} disabled={busy} onChange={(e) => f.set(e.target.value)} />
      )}
      {(['prep-src', 'prep-rawdata'].includes(f.id) ||
        (f.id === 'prep-dst' && ['table', 'observation'].includes(mode))) && (
        <Button disabled={busy} onClick={() => void pick(f.id.slice(5)).catch(() => {})}>
          {t('选择…', 'Browse…')}
        </Button>
      )}
    </Field>
  );
  return (
    <div className="page-stack">
      <h1>{t('准备输入数据', 'Prepare inputs')}</h1>
      <Notice>
        {t(
          '原始文件保留。先检查，再确认变量与单位，最后写入独立输出路径。ERA5 下载需要本机配置的 CDS 凭据。',
          'Source files are preserved. Inspect first, verify variables and units, then write to an isolated destination. ERA5 download requires CDS credentials configured locally.',
        )}
      </Notice>
      {/* 这一页原来只把失败放进 PageContract/全局通知；页面内没有落点，用户取消覆盖后看不到任何说明。 */}
      {error && <Notice tone="error">{error}</Notice>}
      <Card>
        <div className="form-grid">
          {fields
            .filter((f) => f.id === 'prep-mode' || basic.some((k) => f.id === `prep-${k}`))
            .map(render)}
        </div>
        {canInspect && (
          <Button disabled={busy || !form.src} onClick={() => void inspect().catch(() => {})}>
            {t('检查输入', 'Inspect input')}
          </Button>
        )}
        {sourceNames.includes('Rainf') && sourceNames.includes('Snowf') && (
          <Notice>
            {t(
              '检测到 Rainf 与 Snowf：主变量为 Rainf 时已预选 Snowf 合成总降水，请核对附加分量后生成。已有总降水变量时请留空，避免重复累加。附加分量须在源文件中声明单位，转换时统一单位后累加。',
              'Rainf and Snowf detected: Snowf is preselected when the primary variable is Rainf. Verify the additional components before generating total precipitation. Leave additions empty for an existing total. Additional components must declare their units in the source; conversion aligns units before summing.',
            )}
          </Notice>
        )}
        {slots.length > 0 && (
          <div className="form-grid">
            {fields.filter((f) => f.id.startsWith('prep-slot-')).map(render)}
          </div>
        )}
        {mode === 'observation' && (
          <div className="form-grid">
            {fields.filter((f) => f.id.startsWith('prep-obs-')).map(render)}
          </div>
        )}
        <Advanced
          open={advanced}
          onOpenChange={setAdvanced}
          title={t('高级设置', 'Advanced settings')}
        >
          <div className="form-grid">
            {fields.filter((f) => extra.some((k) => f.id === `prep-${k}`)).map(render)}
          </div>
        </Advanced>
        <div className="actions">
          {mode === 'gaps' && (
            <Button
              disabled={busy || !probe}
              onClick={() =>
                void work(() =>
                  invoke('probe_forcing_gaps', {
                    src: form.src,
                    slots: choices(),
                    options: gapOptions(),
                  }),
                ).catch(() => {})
              }
            >
              {t('检查缺测', 'Check gaps')}
            </Button>
          )}
          <Button
            variant="primary"
            disabled={busy || !form.dst || (canInspect && !probe)}
            onClick={() => void execute().catch(() => {})}
          >
            {t('生成数据', 'Generate data')}
          </Button>
          <Button disabled={busy} onClick={() => navigate('new-case')}>
            {t('用于新算例', 'Use for new case')}
          </Button>
        </div>
      </Card>
      {imported.length > 0 && (
        <Card
          title={t(
            '表格已拆分：检查并完成各站强迫场',
            'Table split: inspect and finalize forcing for each site',
          )}
        >
          <Notice>
            {t(
              '这些是暂存文件。修复后才写入最终强迫场路径；需要站点文件时选择“制作站点”。',
              'These are staged files. Fix gaps before writing to the final forcing path; choose "Create site" if site files are needed.',
            )}
          </Notice>
          <Field label={t('ERA5 辅助文件', 'ERA5 auxiliary file')}>
            <Input value={form.era5} onChange={(e) => set('era5', e.target.value)} />
          </Field>
          {imported.map((item) => (
            <div key={item.staged_path}>
              <span>
                {item.site} · {item.staged_path}
              </span>
              <Button disabled={busy} onClick={() => void finish(item).catch(() => {})}>
                {t('修复并完成', 'Finalize')}
              </Button>
            </div>
          ))}
        </Card>
      )}
      {result != null && (
        <Card title={t('实际处理结果', 'Processing result')}>
          <pre>{JSON.stringify(result, null, 2)}</pre>
        </Card>
      )}
    </div>
  );
}
