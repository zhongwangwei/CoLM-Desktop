//! 向导选配置，这里在后台匹配它需要的编译产物。

import { state } from './state.js';
/** USGS 地类仍需要不同的编译产物；其余结构共用 IGBP 产物。 */
export function kernelForSubgrid(subgrid = state.subgrid ?? state.wizard?.subgrid) {
  const want = subgrid === 'USGS' ? 'LULC_USGS' : 'LULC_IGBP';
  const matches = state.kernels.filter(k => k.generator_args.split(/\s+/).includes(want));
  return matches.find(k => k.preset === 'default') ?? matches[0] ?? null;
}

/** URBAN 已是运行时开关，站点匹配必须看向导配置而不是旧内核名。 */
export function urbanEnabled() {
  return !!state.wizard?.physics.urban;
}
