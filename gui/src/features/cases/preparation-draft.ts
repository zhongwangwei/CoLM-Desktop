/**
 * 准备页输入的本地草稿：刷新/重开应用后恢复表单，避免一次刷新就从空表单重来。
 * 只用 localStorage；不可用（无窗口/隐私模式/测试）时静默跳过，不影响页面。
 * 「放弃未生成的输入」会清掉这份草稿，之后离开就是空表单（D-05 残留）。
 */
export const PREP_DRAFT_KEY = 'colm-prep-draft-v1';
export const PREP_MODES = ['netcdf', 'table', 'site', 'gaps', 'era5', 'observation'];
export const EMPTY_PREP_FORM: Record<string, string> = {
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
  site_mode: '',
};
export type PrepDraft = { mode: string; form: Record<string, string> };
export function readPrepDraft(): PrepDraft | null {
  try {
    const store = globalThis.localStorage;
    const raw = store?.getItem(PREP_DRAFT_KEY);
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!parsed || typeof parsed !== 'object') return null;
    const record = parsed as Record<string, unknown>;
    const form = record.form;
    if (!form || typeof form !== 'object') return null;
    const values = form as Record<string, unknown>;
    const clean = Object.fromEntries(
      Object.entries(EMPTY_PREP_FORM).map(([key, fallback]) => [
        key,
        typeof values[key] === 'string' ? values[key] : fallback,
      ]),
    ) as Record<string, string>;
    const mode =
      typeof record.mode === 'string' && PREP_MODES.includes(record.mode) ? record.mode : 'netcdf';
    return { mode, form: clean };
  } catch {
    return null;
  }
}
export function prepDraftHasInput(draft: PrepDraft): boolean {
  return (
    draft.mode !== 'netcdf' ||
    Object.entries(EMPTY_PREP_FORM).some(([key, fallback]) => draft.form[key] !== fallback)
  );
}
export function writePrepDraft(draft: PrepDraft | null) {
  try {
    if (!draft || !prepDraftHasInput(draft)) globalThis.localStorage?.removeItem(PREP_DRAFT_KEY);
    else globalThis.localStorage?.setItem(PREP_DRAFT_KEY, JSON.stringify(draft));
  } catch {
    /* 存储不可用：本次会话内的表单照常工作，只是不跨刷新。 */
  }
}
export function clearPrepDraft() {
  try {
    globalThis.localStorage?.removeItem(PREP_DRAFT_KEY);
  } catch {
    /* ignore */
  }
}
