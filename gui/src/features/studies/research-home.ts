export const researchMethods = [
  {
    id: 'tuning',
    zh: '参数率定',
    en: 'Parameter calibration',
    descriptionZh: '寻找适合当前观测与时段的参数',
    descriptionEn: 'Find parameters for your observations and period',
    icon: 'M4 6h16M4 12h16M4 18h16M8 3v6M16 9v6M10 15v6',
  },
  {
    id: 'uncertainty',
    zh: '不确定性分析',
    en: 'Uncertainty analysis',
    descriptionZh: '检查敏感性与结果变化范围',
    descriptionEn: 'Explore sensitivity and the range of outcomes',
    icon: 'M5 20V12M12 20V4M19 20V8',
  },
  {
    id: 'parameters',
    zh: 'AI 学习参数',
    en: 'AI parameter learning',
    descriptionZh: '根据站点特征预测模型参数',
    descriptionEn: 'Predict parameters from site characteristics',
    icon: 'M9 5a3 3 0 00-5 3 4 4 0 000 8 3 3 0 005 3V5M15 5a3 3 0 015 3 4 4 0 010 8 3 3 0 01-5 3V5M9 9H6M15 15h3',
  },
  {
    id: 'process',
    zh: 'AI 替换过程',
    en: 'AI process replacement',
    descriptionZh: '用网络替代选定过程并验证适用性',
    descriptionEn: 'Replace a selected process and check applicability',
    icon: 'M8 6h8M6 8v8M8 18h8M18 8v8M8 8l8 8M8 16l8-8M8 6a2 2 0 11-4 0 2 2 0 014 0M20 6a2 2 0 11-4 0 2 2 0 014 0M8 18a2 2 0 11-4 0 2 2 0 014 0M20 18a2 2 0 11-4 0 2 2 0 014 0',
  },
  {
    id: 'source',
    zh: '修改源码',
    en: 'Modify source code',
    descriptionZh: '建立并验证新的模型版本',
    descriptionEn: 'Create and verify a new model version',
    icon: 'M8 5l-6 7 6 7M16 5l6 7-6 7M14 3l-4 18',
  },
] as const;
export function methodFromStudy(spec: unknown): string {
  if (!spec || typeof spec !== 'object') return 'tuning';
  const value = spec as Record<string, unknown>;
  if (value.kind === 'uncertainty') return 'uncertainty';
  if (value.hybrid && typeof value.hybrid === 'object')
    return (value.hybrid as { slot?: unknown }).slot === 'soil_stress' ? 'process' : 'parameters';
  return 'tuning';
}
export function completedCandidates(value: unknown): number | null {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : null;
}
