import { useEffect, useRef, useState } from 'react';
import { invokeJson } from '../../lib/ipc';

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
  const canvas = useRef<HTMLCanvasElement | null>(null);
  useEffect(() => {
    setIndex(Math.max(0, steps - 1));
    setData(null);
  }, [source, variable, steps]);
  useEffect(() => {
    let current = true;
    // 拖动滑块时只读停下来的那一步：服务器上读一张图要一两秒。
    const timer = window.setTimeout(() => {
      setBusy(true);
      setError('');
      invokeJson<MapData>('history_map', { case: source, var: variable, index })
        .then((value) => {
          if (current) setData(value);
        })
        .catch((e: unknown) => {
          if (!current) return;
          const text = String(e);
          setError(
            /history-map|not a \(time, patch\) series/.test(text)
              ? t(
                  '服务器上的引擎版本较旧，还不能读格点图。用新版软件在这台服务器上提交一次运行（会编译新引擎）后再看。',
                  'The engine on the server is too old to read grid maps. Submit one run on this server with this app version (it builds the new engine), then look again.',
                )
              : text,
          );
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
    const low = data.min ?? 0,
      span = (data.max ?? 1) - low || 1;
    context.clearRect(0, 0, element.width, element.height);
    data.values.forEach((row, r) => {
      const y = (northUp ? rows - 1 - r : r) * cell;
      row.forEach((value, c) => {
        if (value == null) return;
        context.fillStyle = color((value - low) / span);
        context.fillRect(c * cell, y, cell, cell);
      });
    });
  }, [data]);
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
  const when = data ? new Date(data.time * 1000).toISOString().slice(0, 16).replace('T', ' ') : '';
  return (
    <section className="grid-map" aria-label={t('空间分布', 'Spatial distribution')}>
      <div className="grid-map-head">
        <h2>{t('空间分布', 'Spatial distribution')}</h2>
        <span className="muted">
          {title}
          {when ? ` · ${when} UTC` : ''}
        </span>
      </div>
      <div className="grid-map-time">
        <input
          type="range"
          min={0}
          max={Math.max(0, steps - 1)}
          value={index}
          aria-label={t('时间步', 'Time step')}
          onChange={(event) => setIndex(Number(event.target.value))}
        />
        <span>{t(`第 ${index + 1} / ${steps} 步`, `step ${index + 1} of ${steps}`)}</span>
      </div>
      {error ? (
        <p className="grid-map-error">{error}</p>
      ) : (
        <div className={`grid-map-canvas${busy ? ' is-busy' : ''}`}>
          {data ? (
            <canvas
              ref={canvas}
              onMouseMove={readCell}
              onMouseLeave={() => setHover('')}
              aria-label={`${title} ${when}`}
            />
          ) : (
            <p className="muted">{t('正在读取这一时刻的格点图…', 'Reading this time step…')}</p>
          )}
        </div>
      )}
      {data && (
        <div className="grid-map-legend">
          <span>{data.min == null ? '—' : number(data.min)}</span>
          <span className="grid-map-scale" aria-hidden="true" />
          <span>{data.max == null ? '—' : number(data.max)}</span>
          <span className="muted">{data.units ?? ''}</span>
          <span className="grid-map-hover">{hover}</span>
        </div>
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
