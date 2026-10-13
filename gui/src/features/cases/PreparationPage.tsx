import { useRef, useState } from 'react';
import { invoke } from '../../lib/ipc';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Input, Select, Field, Advanced, Notice } from '../../components/ui';
import {
  requiredNumber,
  versionMode,
  preparedDestination,
  effectiveModelFields,
  cropEnabled,
} from './case-model';
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
  variables?: {
    name: string;
    label: string;
    column?: string;
    qc_column?: string;
    requires_qc: boolean;
  }[];
}
export default function PreparationPage() {
  const { state, notify, navigate } = useApp();
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
  const set = (k: string, v: string) => setForm((f) => ({ ...f, [k]: v }));
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
      notify(String(e), true);
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
      if (path) {
        set(k, path);
        if (k === 'src') {
          setProbe(null);
          setSlots([]);
        }
      }
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
      setSlots(
        (p.slots ?? []).map((s) => ({
          ...s,
          name: s.guessed ?? s.column ?? '',
          units: s.units ?? '',
          also_add: [],
        })),
      );
      setForm((f) => ({
        ...f,
        dst:
          f.dst ||
          (p.suggest_dst
            ? ['netcdf', 'gaps'].includes(mode)
              ? preparedDestination(p.suggest_dst, form.src)
              : p.suggest_dst
            : ''),
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
      }));
      return p;
    });
  }
  function choices() {
    const missing = slots.filter((s) => !s.optional && (!s.name || !s.units));
    if (missing.length)
      throw new Error(`请确认变量与单位：${missing.map((s) => s.meaning).join('、')}`);
    return slots
      .filter((s) => s.name)
      .map((s) => ({ index: s.index, name: s.name, units: s.units, also_add: s.also_add }));
  }
  function heights() {
    const v = ['height_v', 'height_t', 'height_q'].map((k) => optional(k));
    if (v.every((x) => x === null)) return null;
    if (v.some((x) => x === null || x <= 0)) throw new Error('三项观测高度须一同填写正数');
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
  async function execute() {
    return work(async () => {
      if (!form.dst) throw new Error('请选择独立的输出路径 / Output path required');
      if (mode === 'site')
        return invoke('make_site', {
          out: form.dst,
          lon: num('lon'),
          lat: num('lat'),
          landtype: optional('landtype'),
          rawdata: form.rawdata || null,
          mode: versionMode(effectiveModelFields(state.version?.overlay)),
          crop: cropEnabled(effectiveModelFields(state.version?.overlay)),
        });
      if (mode === 'era5')
        return invoke('download_era5land', {
          dst: form.dst,
          latitude: num('lat'),
          longitude: num('lon'),
          start: form.start,
          end: form.end,
        });
      if (!probe) throw new Error('先检查输入并确认字段 / Inspect the input first');
      if (mode === 'observation')
        return invoke('convert_observation_table', {
          src: form.src,
          dstDir: form.dst,
          options: {
            time_column: form.time_column,
            site_column: form.site_column || null,
            site_name: form.site_name || null,
            variables: (probe.variables ?? [])
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
            land_cover_scheme: versionMode(effectiveModelFields(state.version?.overlay)).includes(
              'usgs',
            )
              ? 'usgs'
              : 'igbp',
            heights: heights(),
          },
        });
        setImported(batch);
        return batch;
      }
      if (mode === 'gaps')
        return invoke('repair_forcing', {
          src: form.src,
          dst: form.dst,
          slots: mapped,
          options: gapOptions(),
        });
      return invoke('convert_forcing', {
        src: form.src,
        dst: form.dst,
        slots: mapped,
        heights: heights(),
      });
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
        throw new Error(`仍有 ${report.unresolved} 个缺测；请提供 ERA5 后重试`);
      setImported((items) => items.filter((x) => x.staged_path !== item.staged_path));
      return { path: item.final_path, ...report };
    });
  }
  const labels: Record<string, string> = {
    src: '原始输入文件 / Source',
    dst: '独立输出路径 / Destination',
    lon: '经度 / Longitude',
    lat: '纬度 / Latitude',
    landtype: '地类编号 / Land class',
    rawdata: 'Rawdata 目录',
    time_column: '时间列 / Time column',
    site_column: '站点列 / Site column',
    site_name: '单站名称 / Site name',
    latitude_column: '纬度列',
    longitude_column: '经度列',
    landtype_column: '地类列',
    utc_offset_column: '时区列',
    utc_offset: 'UTC 偏移（小时）',
    step_seconds: '时间步长（秒）',
    era5: 'ERA5 辅助文件',
    short_gap: '短缺测最大步数',
    min_overlap: '最少重叠步数',
    start: '开始日期',
    end: '结束日期',
    height_v: '风速观测高度（m）',
    height_t: '温度观测高度（m）',
    height_q: '湿度观测高度（m）',
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
      label: '准备任务 / Task',
      value: mode,
      type: 'choice',
      options: [
        ['netcdf', 'NetCDF 强迫场'],
        ['table', '表格强迫场 / CSV'],
        ['site', '制作站点 / Site'],
        ['gaps', '修复缺测 / Gaps'],
        ['era5', '下载 ERA5-Land'],
        ['observation', '转换观测 / Observations'],
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
      ['name', 'units'].map((key) => ({
        id: `prep-slot-${s.index}-${key}`,
        label: `${s.meaning} ${key}`,
        value: String(s[key as 'name' | 'units'] ?? ''),
        type: 'text' as const,
        disabled: busy,
        set: (v: string) =>
          setSlots((all) => all.map((x, j) => (i === j ? { ...x, [key]: v } : x))),
      })),
    ),
    ...(mode === 'observation'
      ? (probe?.variables ?? []).flatMap((v, i) =>
          ['column', 'qc_column'].map((key) => ({
            id: `prep-obs-${i}-${key}`,
            label: `${v.label} ${key}`,
            value: v[key as 'column' | 'qc_column'] ?? '',
            type: 'text' as const,
            disabled: busy,
            set: (value: string) =>
              setProbe((p) =>
                p
                  ? {
                      ...p,
                      variables: p.variables?.map((x, j) => (i === j ? { ...x, [key]: value } : x)),
                    }
                  : p,
              ),
          })),
        )
      : []),
  ];
  fields.push({
    id: 'prep-advanced',
    label: '高级 / Advanced',
    value: String(advanced),
    type: 'switch',
    set: (v) => setAdvanced(v === 'true'),
  });
  if (imported.length)
    fields.push({
      id: 'prep-batch-era5',
      label: 'ERA5 辅助文件',
      value: form.era5,
      type: 'text',
      disabled: busy,
      set: (v) => set('era5', v),
    });
  const canInspect = !['site', 'era5'].includes(mode);
  usePageContract({
    title: '输入数据准备 / Prepare inputs',
    step: 'preparation',
    status: busy ? 'busy' : error || 'ready',
    details: { probe, result, error, imported },
    fields,
    actions: [
      {
        id: 'prep-new-case',
        label: '用于新算例 / New case',
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
          label: `选择 ${labels[k]}`,
          kind: 'click' as const,
          enabled: !busy,
          run: () => pick(k),
        })),
      ...(canInspect
        ? [
            {
              id: 'prep-inspect',
              label: '检查输入 / Inspect',
              kind: 'click' as const,
              enabled: !busy && !!form.src,
              run: inspect,
            },
          ]
        : []),
      {
        id: 'prep-execute',
        label: '生成数据 / Generate',
        kind: 'commit',
        enabled: !busy && !!form.dst && (!canInspect || !!probe),
        run: execute,
      },
      ...imported.map((item, i) => ({
        id: `prep-finish-${i}`,
        label: `修复并完成 ${item.site}`,
        kind: 'commit' as const,
        enabled: !busy,
        run: () => finish(item),
      })),
      ...(mode === 'gaps'
        ? [
            {
              id: 'prep-gaps',
              label: '检查缺测 / Inspect gaps',
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
          选择 / Browse
        </Button>
      )}
    </Field>
  );
  return (
    <div className="page-stack">
      <h1>准备输入数据 / Prepare inputs</h1>
      <Notice>
        原始文件保留。先检查，再确认变量与单位，最后写入独立输出路径。ERA5 下载需要本机配置的 CDS
        凭据。
      </Notice>
      <Card>
        <div className="form-grid">
          {fields
            .filter((f) => f.id === 'prep-mode' || basic.some((k) => f.id === `prep-${k}`))
            .map(render)}
        </div>
        {canInspect && (
          <Button disabled={busy || !form.src} onClick={() => void inspect().catch(() => {})}>
            检查输入 / Inspect
          </Button>
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
        <Advanced open={advanced} onOpenChange={setAdvanced} title="高级 / Advanced">
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
              检查缺测 / Check gaps
            </Button>
          )}
          <Button
            variant="primary"
            disabled={busy || !form.dst || (canInspect && !probe)}
            onClick={() => void execute().catch(() => {})}
          >
            生成数据 / Generate
          </Button>
          <Button disabled={busy} onClick={() => navigate('new-case')}>
            用于新算例 / New case
          </Button>
        </div>
      </Card>
      {imported.length > 0 && (
        <Card title="表格已拆分：检查并完成各站强迫场">
          <Notice>
            这些是暂存文件。修复后才写入最终强迫场路径；需要站点文件时选择“制作站点”。
          </Notice>
          <Field label="ERA5 辅助文件">
            <Input value={form.era5} onChange={(e) => set('era5', e.target.value)} />
          </Field>
          {imported.map((item) => (
            <div key={item.staged_path}>
              <span>
                {item.site} · {item.staged_path}
              </span>
              <Button disabled={busy} onClick={() => void finish(item).catch(() => {})}>
                修复并完成 / Finalize
              </Button>
            </div>
          ))}
        </Card>
      )}
      {result != null && (
        <Card title="实际处理结果 / Result">
          <pre>{JSON.stringify(result, null, 2)}</pre>
        </Card>
      )}
    </div>
  );
}
