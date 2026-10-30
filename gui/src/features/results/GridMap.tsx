import { useEffect, useRef, useState } from 'react';
import { plainError } from '../../lib/bilingual';
import { invokeJson } from '../../lib/ipc';
import { timeBasisLabel, timeBasisOf } from './time-basis';
import { formatModelClock } from './time-ticks';
import { readErrorText } from './variable-labels';

/** What `history_map` returns (`colm-cli history-map`). */
type MapData = {
  var: string;
  units?: string | null;
  time: number;
  index: number;
  steps: number;
  lat: number[];
  lon: number[];
  stride: number;
  values: (number | null)[][];
  min?: number | null;
  max?: number | null;
  /** 后端给出的时间基准；旧版 sidecar 没有时按模型钟处理。 */
  time_basis?: unknown;
};

// viridis 的 9 个色标：明暗单调，色盲与灰度打印下也分得清高低。
const STOPS = [
  [68, 1, 84],
  [72, 40, 120],
  [62, 74, 137],
  [49, 104, 142],
  [38, 130, 142],
  [31, 158, 137],
  [53, 183, 121],
  [110, 206, 88],
  [253, 231, 37],
];
function color(fraction: number): string {
  const x = Math.min(1, Math.max(0, fraction)) * (STOPS.length - 1);
  const i = Math.min(STOPS.length - 2, Math.floor(x));
  const f = x - i;
  const [r, g, b] = STOPS[i].map((c, k) => Math.round(c + (STOPS[i + 1][k] - c) * f));
  return `rgb(${r},${g},${b})`;
}
const number = (value: number) =>
  Math.abs(value) >= 1000 || (value !== 0 && Math.abs(value) < 0.01)
    ? value.toExponential(2)
    : value.toFixed(2);

/** 一帧可用的色标范围（含端点）。 */
export type MapColorScale = { low: number; high: number };

/**
 * 决定这一帧该按哪个范围着色。
 *
 * 审计 §五.5：原来每帧都按本帧 data.min/max 着色，同一个颜色在相邻两帧代表的数值不同，
 * 跨时间对比会看错。固定范围非空时优先用它，哪怕这一帧全缺测（min/max 都是 null）也不
 * 退回自动范围 —— 那样一换时刻整张图就重新铺色，固定色标等于没固定。
 * 没有固定范围时按本帧 min/max 自动取；两者任一不是有限数就视为没有范围（全缺测帧），
 * 画布保持空白而不是涂成一色。
 */
export function colorRange(
  dataMin: number | null | undefined,
  dataMax: number | null | undefined,
  fixed?: MapColorScale | null,
): MapColorScale | undefined {
  if (fixed) return fixed;
  if (
    typeof dataMin === 'number' &&
    Number.isFinite(dataMin) &&
    typeof dataMax === 'number' &&
    Number.isFinite(dataMax)
  )
    return { low: dataMin, high: dataMax };
  return undefined;
}

/**
 * 数值 → 色标比例 0..1，越界值夹到端点（与原来 color() 的夹取一致）。
 * min == max 的退化范围按跨度 1 处理，和旧画布的 `(max - min) || 1` 一致。
 */
export function colorFraction(value: number, range: MapColorScale): number {
  const span = range.high - range.low || 1;
  return Math.min(1, Math.max(0, (value - range.low) / span));
}

/** 色标端点的显示文本；没有范围时用长横线（与旧图例一致），数值沿用现有 number() 格式。 */
export function scaleValueText(value: number | null | undefined): string {
  return typeof value === 'number' ? number(value) : '—';
}

/** 格点变量某一时刻的空间分布：色块图、色标、时间滑块，鼠标悬停读格点值。 */
export default function GridMap({
  source,
  variable,
  title,
  steps,
  en,
}: {
  source: string;
  variable: string;
  title: string;
  steps: number;
  en: boolean;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [index, setIndex] = useState(Math.max(0, steps - 1));
  const [data, setData] = useState<MapData | null>(null);
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [hover, setHover] = useState('');
  /** 是否处于固定色标模式（false = 每帧按自身 min/max 自动着色）。 */
  const [fixedMode, setFixedMode] = useState(false);
  /** 最近一次锁定的色标范围；解除固定后仍保留，下次固定时当前帧缺测还能沿用。 */
  const [fixedScale, setFixedScale] = useState<MapColorScale | null>(null);
  const canvas = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    setIndex(Math.max(0, steps - 1));
    setData(null);
    // 换变量/换数据源后旧范围没有可比性：颜色单位都变了，不能把上一张图的色标带过来。
    setFixedMode(false);
    setFixedScale(null);
  }, [source, variable, steps]);
  useEffect(() => {
    let current = true;
    // 拖动滑块时只读停下来的那一步：服务器上读一张图要一两秒。
    const timer = window.setTimeout(() => {
      setBusy(true);
      setError('');
      invokeJson<MapData>('history_map', { case: source, var: variable, index })
        .then((value) => {
          if (!current) return;
          setData(value);
          // 这个变量自己的步数比主输出少时，后端取了最后一步：滑块跟着它走。
          if (value.index !== index) setIndex(value.index);
        })
        .catch((e: unknown) => {
          if (current) setError(readErrorText(e, en));
        })
        .finally(() => {
          if (current) setBusy(false);
        });
    }, 250);
    return () => {
      current = false;
      window.clearTimeout(timer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [source, variable, index]);
  // 固定模式下切换时当前帧全缺测、还没有可锁定的范围：等第一个有有效值的帧再锁定。
  // 这之前画布按自动范围回退（全缺测帧保持空白），不会把整图涂成一色。
  useEffect(() => {
    if (!fixedMode || fixedScale || !data) return;
    const captured = colorRange(data.min, data.max);
    if (captured) setFixedScale(captured);
  }, [fixedMode, fixedScale, data]);
  useEffect(() => {
    const element = canvas.current;
    if (!element || !data?.values.length) return;
    const rows = data.values.length,
      cols = data.values[0].length;
    // 每格至少 2 像素、整图最宽 720 像素；纬度递增时把北放到上面。
    const cell = Math.max(2, Math.floor(720 / cols));
    element.width = cols * cell;
    element.height = rows * cell;
    const context = element.getContext('2d');
    if (!context) return;
    const northUp = data.lat.length > 1 && data.lat[0] < data.lat[data.lat.length - 1];
    // 固定色标优先；固定范围还没建立时退回本帧自动范围。没有可用范围（全缺测帧）就不着色，
    // 不能退回 0..1 把整张图涂成一色。
    const range = colorRange(data.min, data.max, fixedMode ? fixedScale : null);
    context.clearRect(0, 0, element.width, element.height);
    if (!range) return;
    data.values.forEach((row, r) => {
      const y = (northUp ? rows - 1 - r : r) * cell;
      row.forEach((value, c) => {
        if (value == null) return;
        context.fillStyle = color(colorFraction(value, range));
        context.fillRect(c * cell, y, cell, cell);
      });
    });
  }, [data, fixedMode, fixedScale]);
  function readCell(event: React.MouseEvent<HTMLCanvasElement>) {
    const element = canvas.current;
    if (!element || !data?.values.length) return;
    const box = element.getBoundingClientRect();
    const rows = data.values.length,
      cols = data.values[0].length;
    const c = Math.floor(((event.clientX - box.left) / box.width) * cols);
    const shownRow = Math.floor(((event.clientY - box.top) / box.height) * rows);
    const northUp = data.lat.length > 1 && data.lat[0] < data.lat[data.lat.length - 1];
    const r = northUp ? rows - 1 - shownRow : shownRow;
    const value = data.values[r]?.[c];
    if (r < 0 || c < 0 || r >= rows || c >= cols) return setHover('');
    setHover(
      `${data.lat[r].toFixed(2)}°, ${data.lon[c].toFixed(2)}° · ${
        value == null ? t('缺测', 'missing') : `${number(value)} ${data.units ?? ''}`
      }`,
    );
  }
  // 地图只读某一时刻：钟面照模型钟显示，基准（UTC / UTC+08:00 / 模型钟）跟着数据走，
  // 不再硬编码 “UTC”。
  const basis = timeBasisOf(data);
  const basisLabel = timeBasisLabel(basis, en);
  const when = data ? formatModelClock(data.time) : '';
  /** 实际着色范围：固定模式已锁定时优先锁定范围，尚未锁定时退回本帧自动范围；全缺测帧为 undefined。 */
  const scale = colorRange(data?.min, data?.max, fixedMode ? fixedScale : null);
  const units = data?.units ?? '';
  const unitText = units ? ` ${units}` : '';
  const lowText = scaleValueText(scale?.low);
  const highText = scaleValueText(scale?.high);
  /** 固定模式但当前帧全缺测：范围沿用上次锁定的值，界面上要说明。 */
  const keepingFixed = fixedMode && !!fixedScale && !!data && data.min == null;
  function toggleFixedScale() {
    if (fixedMode) {
      // 解除固定：回到每帧 min/max；上次锁定的范围留着，下次再固定且当前帧缺测时还能沿用。
      setFixedMode(false);
      return;
    }
    setFixedMode(true);
    // 切换瞬间锁定当前帧的范围；当前帧全缺测就保留上一次锁定的范围（可能还没建立）。
    const current = colorRange(data?.min, data?.max);
    if (current) setFixedScale(current);
  }
  // 无障碍：把固定色标范围写进区域标题，读屏时也能知道颜色代表什么数值。
  const sectionLabel =
    fixedMode && scale
      ? t(
          `空间分布（固定色标 ${lowText}–${highText}${unitText}）`,
          `Spatial distribution (fixed color scale ${lowText}–${highText}${unitText})`,
        )
      : t('空间分布', 'Spatial distribution');
  return (
    <section className="grid-map" aria-label={sectionLabel}>
      <div className="grid-map-head">
        <h2>{t('空间分布', 'Spatial distribution')}</h2>
        <span className="muted">
          {title}
          {when ? ` · ${when} ${basisLabel}` : ''}
        </span>
      </div>
      <div className="grid-map-time">
        <input
          type="range"
          min={0}
          max={Math.max(0, (data?.steps ?? steps) - 1)}
          value={index}
          aria-label={t('时间步', 'Time step')}
          onChange={(event) => setIndex(Number(event.target.value))}
        />
        <span>
          {t(
            `第 ${index + 1} / ${data?.steps ?? steps} 步`,
            `step ${index + 1} of ${data?.steps ?? steps}`,
          )}
        </span>
      </div>
      <div className="grid-map-scale-mode">
        <button
          type="button"
          className="button button-secondary"
          aria-pressed={fixedMode}
          title={
            fixedMode
              ? t('解除固定，回到每帧自动色标', 'Unfix and use each frame’s own range')
              : t(
                  '锁定当前帧的色标范围，跨时间比较同一颜色时数值一致',
                  'Lock this frame’s color range so a color means the same value across time',
                )
          }
          onClick={toggleFixedScale}
        >
          {fixedMode
            ? t('固定色标', 'Fixed color scale')
            : t('自动色标（每帧 min/max）', 'Auto color scale (per frame)')}
        </button>
        <span>
          {fixedMode
            ? scale
              ? t(
                  `固定范围 ${lowText}–${highText}${unitText}`,
                  `Fixed range ${lowText}–${highText}${unitText}`,
                )
              : t(
                  '当前帧还没有可锁定的范围（尚未载入或全缺测），着色暂时按自动色标处理。',
                  'No lockable range yet for this frame (still loading or all missing); colors temporarily follow the auto range.',
                )
            : t(
                '同一个颜色在不同时刻不代表同一个数值。',
                'The same color does not mean the same value across time.',
              )}
        </span>
      </div>
      {error ? (
        <p className="grid-map-error">{plainError(error)}</p>
      ) : (
        <div className={`grid-map-canvas${busy ? ' is-busy' : ''}`}>
          {data ? (
            <canvas
              ref={canvas}
              onMouseMove={readCell}
              onMouseLeave={() => setHover('')}
              aria-label={`${title} ${when} ${when ? basisLabel : ''}`}
            />
          ) : (
            <p className="muted">{t('正在读取这一时刻的格点图…', 'Reading this time step…')}</p>
          )}
        </div>
      )}
      {data && (
        <div className="grid-map-legend">
          <span>{lowText}</span>
          <span className="grid-map-scale" aria-hidden="true" />
          <span>{highText}</span>
          <span className="muted">{data.units ?? ''}</span>
          <span className="grid-map-hover">{hover}</span>
        </div>
      )}
      {keepingFixed && (
        <p className="muted grid-map-note">
          {t(
            '当前帧没有有效值，色标沿用上次锁定的范围。',
            'This frame has no valid values; keeping the last fixed color range.',
          )}
        </p>
      )}
      {data && data.stride > 1 && (
        <p className="muted grid-map-note">
          {t(
            `网格较大，图上每隔 ${data.stride} 格取一格显示。`,
            `The grid is large; every ${data.stride}th cell is drawn.`,
          )}
        </p>
      )}
    </section>
  );
}
