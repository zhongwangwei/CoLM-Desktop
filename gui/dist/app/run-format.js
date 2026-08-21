//! 批量运行视图的纯文本规则，DOM 与 IPC 都不进这里。

const LOG_LIMIT = 60000;

export function appendLogText(text, lines) {
  let out = text + lines.join('\n') + '\n';
  if (out.length > LOG_LIMIT) out = out.slice(-40000);
  return out;
}

export function progressText(p, stateLabel = '运行中') {
  if (stateLabel === '已完成') return '完成';
  if (stateLabel === '失败') return p.reason ? `失败：${p.reason}` : '失败';
  if (p.step > 0) {
    const spin = p.spinup ? `预热 ${p.spinup[0]}/${p.spinup[1]} 轮 · ` : '';
    return `${spin}第 ${p.step}/${p.total_steps || p.step} 步 · ${p.date}`;
  }
  if (p.stage) return `${p.stage} ${p.stage_state === 'begin' ? '运行中' : p.stage_state ?? ''}`.trim();
  return stateLabel === '待运行' ? '等待 CPU' : stateLabel;
}
