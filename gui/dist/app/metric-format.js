//! 指标可能不可定义（例如观测方差为零时 R²/KGE 是 null）。

export function metricText(value, digits = 3, signed = false) {
  if (!Number.isFinite(value)) return '—';
  const text = Number(value).toFixed(digits);
  return signed && value >= 0 ? `+${text}` : text;
}
