//! 参数预设：存下这份算例的物理与输出设置，套到别的算例上。
//!
//! 站点、路径、算例名**不进预设**（后端 `IDENTITY_PREFIXES` 挡着）——
//! 它们是算例身份不是参数，混进去会让套用变成「悄悄换了个站点」。
//! 被挡下的字段由 `save_preset` 返回，这里说出来：静默丢弃会让人以为
//! 存进去了，然后在套用时发现站点没跟着过来。

import { invoke } from './ipc.js';
import { state } from './state.js';
import { editTarget } from './batch.js';
import { $, status } from './ui.js';
import { renderFields } from './params.js';

export async function refreshPresets() {
  const s = $('preset');
  let names = [];
  try { names = await invoke('list_presets'); } catch (e) { status(e); }
  s.textContent = '';
  if (!names.length) {
    const o = document.createElement('option');
    o.textContent = '（还没有预设）'; o.value = '';
    s.appendChild(o);
  }
  for (const n of names) {
    const o = document.createElement('option');
    o.value = n; o.textContent = n;
    s.appendChild(o);
  }
  const has = !!names.length;
  $('preset-apply').disabled = !(has && state.text);
  $('preset-del').disabled = !has;
  $('preset-save').disabled = !state.text;
}

$('preset-save').onclick = async () => {
  const name = prompt('预设名');
  if (!name) return;
  try {
    const skipped = await invoke('save_preset', { name, text: state.text });
    await refreshPresets();
    $('preset').value = name;
    status(skipped.length
      ? `已存 ${name}；${skipped.length} 个身份字段未收进去（${skipped.join('、')}）`
      : `已存 ${name}`);
  } catch (e) { status(e); }
};

$('preset-apply').onclick = async () => {
  const name = $('preset').value;
  if (!name || !state.selected) return;
  try {
    const dirs = editTarget();
    const r = await invoke('apply_preset_batch', { name, dirs });
    state.text = r.text;
    await renderFields();
    status(r.written > 1 ? `已把 ${name} 套到 ${r.written} 个算例上` : `已套用 ${name}`);
  } catch (e) { status(e); }
};

$('preset-del').onclick = async () => {
  const name = $('preset').value;
  if (!name) return;
  try { await invoke('delete_preset', { name }); await refreshPresets(); status(`已删除 ${name}`); }
  catch (e) { status(e); }
};
