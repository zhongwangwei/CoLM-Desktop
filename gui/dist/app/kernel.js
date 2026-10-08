//! 向导选配置，这里在后台匹配它需要的编译产物。

import { state } from './state.js';

const GRID_MACROS = { latlon: 'GRIDBASED', unstructured: 'UNSTRUCTURED', catchment: 'CATCHMENT' };

/** USGS 与 CROP 仍需要不同编译产物；PFT/PC 是 IGBP 分类内的运行时结构。 */
export function kernelForSubgrid(subgrid = state.subgrid ?? state.wizard?.subgrid, opts = state.wizard) {
  const want = subgrid === 'USGS' ? 'LULC_USGS' : 'LULC_IGBP';
  const grid = opts && Object.hasOwn(opts, 'grid') ? opts.grid : (state.wizard?.grid ?? state.grid);
  const wantGrid = GRID_MACROS[grid] ?? 'SinglePoint';
  const wantCrop = !!(opts?.crop ?? opts?.physics?.crop);
  // 采纳过的实验内核排在前面（“设为默认”）；没采纳过就只有正式内核，行为不变。
  const pool = [...(state.adoptedKernels ?? []), ...state.kernels];
  const matches = pool.filter(k => k.macros?.includes(wantGrid)
    && k.macros?.includes(want) && !!k.macros?.includes('CROP') === wantCrop);
  const preferred = wantCrop ? 'crop' : (subgrid === 'USGS' ? 'usgs' : 'default');
  return matches.find(k => k.preset === preferred) ?? matches[0] ?? null;
}

/** URBAN 已是运行时开关，站点匹配必须看向导配置而不是旧内核名。 */
export function urbanEnabled() {
  return !!state.wizard?.physics.urban;
}

// 示踪剂可多选，`picked.tracer` / `wizard.tracer` 是逗号分隔的名单（如 'methane,isotope'）。
export const tracerList = value => String(value ?? '').split(',').map(id => id.trim()).filter(Boolean);
/** 向导配置里有没有选这种示踪剂。 */
export function hasTracer(wizard, id) {
  return !!wizard?.physics?.tracer && tracerList(wizard?.tracer).includes(id);
}
