//! 向导选配置，这里在后台匹配它需要的编译产物。

import { state } from './state.js';
/** USGS 与 CROP 仍需要不同编译产物；PFT/PC 是 IGBP 分类内的运行时结构。 */
export function kernelForSubgrid(subgrid = state.subgrid ?? state.wizard?.subgrid, opts = state.wizard) {
  const want = subgrid === 'USGS' ? 'LULC_USGS' : 'LULC_IGBP';
  const wantCrop = !!(opts?.physics?.crop ?? opts?.crop);
  const matches = state.kernels.filter(k => k.macros?.includes(want) && !!k.macros?.includes('CROP') === wantCrop);
  const preferred = wantCrop ? 'crop' : (subgrid === 'USGS' ? 'usgs' : 'default');
  return matches.find(k => k.preset === preferred) ?? matches[0] ?? null;
}

/** URBAN 已是运行时开关，站点匹配必须看向导配置而不是旧内核名。 */
export function urbanEnabled() {
  return !!state.wizard?.physics.urban;
}
