/**
 * 评估设计的稳定指纹工具。配置文件（OpenBench YAML）与表单两条路径都要算"这是不是同一份评估"，
 * 所以把规范化与哈希单独放一份：比较页（表单设计）与有效配置（手改后的 YAML）共用同一算法，
 * 两处判断（例如完成提示是否过期、配置保存徽标）才不会各算一套。
 */

/** 指纹里的对象键按名字排序：同一份设计不因勾选/写入顺序不同得到两个指纹。 */
export function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(',')}]`;
  if (value && typeof value === 'object')
    return `{${Object.keys(value as Record<string, unknown>)
      .sort()
      .map(
        (key) => `${JSON.stringify(key)}:${canonicalJson((value as Record<string, unknown>)[key])}`,
      )
      .join(',')}}`;
  if (typeof value === 'number' && !Number.isFinite(value)) return 'null';
  return JSON.stringify(value) ?? 'null';
}

/**
 * 32 位 FNV-1a：只用来判断"是不是同一份输入"，不是密码学校验。写成短串是为了在提示里显示、
 * 也方便用户对照两次运行。
 */
export function fingerprintOf(value: unknown): string {
  const text = canonicalJson(value);
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index++) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, '0');
}
