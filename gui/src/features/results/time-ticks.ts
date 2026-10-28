/**
 * 时间轴刻度与钟面格式化。
 *
 * 后端时间数值都是**模型钟面**经 epoch 秒编码：格式化必须用 UTC getter，
 * 否则浏览器时区会把钟面整体平移。这个文件只负责钟面本身；
 * 「这是什么钟」（UTC / UTC+08:00 / 模型钟）由 `time-basis.ts` 给出，
 * 轴标签和提示必须一并显示，不能再硬编码 UTC。
 */
import { timeBasisLabel, type TimeBasis } from './time-basis';

const pad = (n: number) => String(n).padStart(2, '0');

/** 把「当作 UTC 编码的钟面秒」拆成日期与钟点；非法值返回 null。 */
function clockParts(seconds: number): {
  day: string;
  hourMinute: string;
  second: number;
  milliseconds: string;
} | null {
  const date = new Date(seconds * 1000);
  if (!Number.isFinite(date.getTime())) return null;
  return {
    day: `${date.getUTCFullYear()}-${pad(date.getUTCMonth() + 1)}-${pad(date.getUTCDate())}`,
    hourMinute: `${pad(date.getUTCHours())}:${pad(date.getUTCMinutes())}`,
    second: date.getUTCSeconds(),
    milliseconds: String(date.getUTCMilliseconds()).padStart(3, '0'),
  };
}

/**
 * 钟面的 ISO 形式，**不带 Z**：它表示地方/模型钟面上的读数，不是 UTC 瞬间。
 * 全量时序 CSV 用这一列时必须保持无 Z，避免下游把它当真实 UTC。
 */
export function modelClockIso(seconds: number): string {
  const parts = clockParts(seconds);
  if (!parts) throw new Error('Invalid model clock time');
  return `${parts.day}T${parts.hourMinute}:${pad(parts.second)}.${parts.milliseconds}`;
}

/** 悬停/地图标题里的可读钟面：`YYYY-MM-DD HH:MM`；非法值返回空串。 */
export function formatModelClock(seconds: number): string {
  const parts = clockParts(seconds);
  return parts ? `${parts.day} ${parts.hourMinute}` : '';
}

/** 轴标签的基准说明；保留在这里，避免各图各写一份「UTC」。 */
export function timeAxisLabel(basis: TimeBasis | undefined, en = false): string {
  return timeBasisLabel(basis, en);
}

export function timeTicks(
  _u: unknown,
  splits: number[],
  _axis?: number,
  _space?: number,
  increment = 0,
) {
  let lastDay = '';
  return splits.map((seconds) => {
    const parts = clockParts(seconds);
    if (!parts) return '';
    if (increment >= 86400) return increment >= 86400 * 28 ? parts.day.slice(0, 7) : parts.day;
    const label = parts.day === lastDay ? parts.hourMinute : `${parts.hourMinute}\n${parts.day}`;
    lastDay = parts.day;
    return label;
  });
}
