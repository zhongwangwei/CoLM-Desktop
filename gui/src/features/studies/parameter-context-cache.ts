import type { VersionRef } from '../../lib/types';

/**
 * 第 2 步“这些算例能研究哪些参数”的读取身份（GUI-14）。
 *
 * 后端 `study_parameter_contexts` 只认 dirs/kernelDir/types，但 dirs 是 `model_case_context`
 * 按模型修订换算出来的生效目录：同一算例、同一编译配置换一份模型修订后，生效目录和读出的
 * 默认值都可能不同，所以模型 ID 与修订号必须进缓存身份，旧修订的结果不能当缓存命中。
 * 站点顺序也进身份：参数勾选按行号记，而后端返回的参数行顺序跟着 dirs 走。
 */
export type ParameterContextIdentity = {
  version: VersionRef;
  kernel: string;
  sites: string[];
  spatial: boolean;
  types: number[];
};

/**
 * 缓存键用 JSON 数组而不是 `a|b|c` 拼串：路径里可能带任何分隔符，拼串会把两个不同身份
 * 拼成同一个键（那样换了目录却命中旧参数）。类型只在格点研究里影响参数行（单点按算例
 * 自己的地类定），所以单点不因残留的类型选择换键。
 */
export function parameterContextKey(identity: ParameterContextIdentity): string {
  return JSON.stringify([
    identity.version.id,
    identity.version.revision,
    identity.kernel,
    identity.sites,
    identity.spatial,
    identity.spatial ? identity.types : [],
  ]);
}

/**
 * 请求返回时核对身份。版本、算例（含顺序）、编译配置、格点类型任一变了，这次请求就不再
 * 属于当前页面：旧模型修订读出的参数不能覆盖新修订的（GUI-14），哪怕是同一个算例目录。
 */
export function sameParameterContextIdentity(
  requested: ParameterContextIdentity,
  current: ParameterContextIdentity,
): boolean {
  return parameterContextKey(requested) === parameterContextKey(current);
}

/**
 * 第 2 步自动读取的触发条件。
 *
 * 成功读过的身份不重复请求（普通渲染、busy 翻转都不再发）。刚失败过的身份也不自动重试：
 * 读取失败时加载标记会被清掉（数据没有、标记也要没有），而 busy 翻回 false 本身就是
 * effect 依赖变化，若只清标记就会立刻再发一次——失败一次变成无限重试死循环。失败身份在
 * 换身份（版本/算例/配置/类型）时不再命中，或用户点“重新读取”时清除。
 */
export function shouldStartParameterContextLoad({
  step,
  identity,
  loadedFor,
  failedFor,
  typeChoicesReady,
  busy,
}: {
  step: number;
  identity: ParameterContextIdentity;
  loadedFor: string;
  failedFor: string;
  typeChoicesReady: boolean;
  busy: boolean;
}): boolean {
  if (step !== 2 || busy) return false;
  if (!identity.kernel || !identity.sites.length) return false;
  // 格点算例要先知道有哪些地类/植物类型（不选也能读，只是只有整算例的系数）；类型还没读出来
  // 就读参数会读成一套缺类型行的结果，等类型回来还得再读一次。
  if (identity.spatial && !typeChoicesReady) return false;
  const key = parameterContextKey(identity);
  return loadedFor !== key && failedFor !== key;
}

/**
 * 只清掉“这次请求自己写下的”加载标记。
 *
 * 旧身份的请求收尾时，新身份的请求可能已经把标记换成自己的键：这时不能清（否则新请求
 * 还没回来，effect 会因为 busy 翻转再发一次同样的请求）。
 */
export function clearParameterContextMarker(marker: { current: string }, key: string): boolean {
  if (marker.current !== key) return false;
  marker.current = '';
  return true;
}
