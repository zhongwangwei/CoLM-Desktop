/**
 * 结果页的「时间基准」契约。
 *
 * 后端所有时间数值都是**模型钟面**经 `unix_seconds()` 编码成的 epoch 秒：
 * 单点算例按地方标准时推进（`greenwich=.false.`），区域算例按 UTC 推进。
 * 把一个地方时钟面标成 `time_utc` 再带 Z，下游软件会把它当真实 UTC 拼接，
 * 对非零偏移的站点整体错位。所以基准必须跟着数据一起传，而不是由前端猜：
 *
 * - `utc`：数值就是真实 UTC，可安全导出 `time_utc`；
 * - `local_standard_time`：数值是地方标准时**钟面**，偏移已知时可换算成 UTC；
 * - `model_clock`：数值是模型钟面，但偏移没有记录（历史运行、普通单点算例），
 *   只能当钟面用，**永远不生成 time_utc**。
 */
export type TimeBasisKind = 'utc' | 'local_standard_time' | 'model_clock';

export interface TimeBasis {
  kind: TimeBasisKind;
  /** 地方标准时相对 UTC 的偏移秒数；`utc` 为 0、`model_clock` 或未记录为 null。 */
  utc_offset_seconds: number | null;
  /** 日历名。目前历史文件只可能是公历；非公历后端应拒绝而不是静默换算。 */
  calendar: string;
  /** 这个基准从哪来（`case.nml` 的 greenwich、观测 `time:units`、缺省推断……），用于追溯。 */
  source: string;
}

/** 缺失/非法时的缺省基准：宁可按模型钟展示，也不冒称 UTC。 */
export function unknownTimeBasis(): TimeBasis {
  return {
    kind: 'model_clock',
    utc_offset_seconds: null,
    calendar: 'standard',
    source: 'unspecified',
  };
}

const KINDS: readonly TimeBasisKind[] = ['utc', 'local_standard_time', 'model_clock'];
const pad = (n: number) => String(n).padStart(2, '0');

function normalized(value: unknown): TimeBasis | null {
  if (!value || typeof value !== 'object') return null;
  const raw = value as Record<string, unknown>;
  const kind = KINDS.find((candidate) => candidate === raw.kind);
  if (!kind) return null;
  // 模型钟本身没有可用的偏移信息；即使载荷里带了也不采用，避免把两个时钟混在一起。
  const declared = raw.utc_offset_seconds;
  const offset =
    kind !== 'model_clock' && typeof declared === 'number' && Number.isFinite(declared)
      ? declared
      : null;
  return {
    kind,
    utc_offset_seconds: offset,
    calendar: typeof raw.calendar === 'string' && raw.calendar.trim() ? raw.calendar : 'standard',
    source: typeof raw.source === 'string' && raw.source.trim() ? raw.source : 'unspecified',
  };
}

/**
 * 从后端载荷里取 `time_basis`。旧版 sidecar 的响应没有这个字段（远程服务器上可能
 * 还是旧二进制），此时按 `model_clock` 处理 —— 少标一个 UTC 是保守的，
 * 把地方时标成 UTC 才是错的。
 */
export function timeBasisOf(payload: unknown): TimeBasis {
  if (payload && typeof payload === 'object') {
    const found = normalized((payload as Record<string, unknown>).time_basis);
    if (found) return found;
  }
  return unknownTimeBasis();
}

/** `UTC`、`UTC+08:00` / `UTC-05:30` 这类数值标签；中英一致，便于在轴上并排比较。 */
function offsetLabel(seconds: number): string {
  const sign = seconds < 0 ? '-' : '+';
  const absolute = Math.abs(seconds);
  const hours = Math.floor(absolute / 3600);
  const minutes = Math.floor((absolute % 3600) / 60);
  const rest = absolute % 60;
  return `UTC${sign}${pad(hours)}:${pad(minutes)}${rest ? `:${pad(rest)}` : ''}`;
}

/**
 * 轴/提示/导出里的基准说明。用户看到「2008-01-01 00:30」时必须同时知道它是什么钟，
 * 否则图上的时间会被当成 UTC 或浏览器本地时间。
 */
export function timeBasisLabel(basis: TimeBasis | undefined, en = false): string {
  if (!basis || basis.kind === 'model_clock') {
    return en
      ? 'model clock (local standard time, offset unknown)'
      : '模型钟（本地标准时，偏移未知）';
  }
  if (basis.kind === 'utc') return 'UTC';
  if (basis.utc_offset_seconds === null) {
    return en ? 'local standard time (offset not recorded)' : '地方标准时（偏移未记录）';
  }
  return offsetLabel(basis.utc_offset_seconds);
}
