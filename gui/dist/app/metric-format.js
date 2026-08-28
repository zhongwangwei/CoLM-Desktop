//! 指标可能不可定义（例如观测方差为零时 R²/KGE 是 null）。

export function metricText(value, digits = 6, signed = false) {
  if (!Number.isFinite(value)) return '—';
  const number = Number(value);
  let text = number.toFixed(digits);
  if (number !== 0 && Number(text) === 0) {
    const decimals = Math.min(12, Math.max(digits, Math.ceil(-Math.log10(Math.abs(number))) + 2));
    text = number.toFixed(decimals);
    if (Number(text) === 0) text = number.toExponential(4);
  }
  return signed && value >= 0 ? `+${text}` : text;
}
