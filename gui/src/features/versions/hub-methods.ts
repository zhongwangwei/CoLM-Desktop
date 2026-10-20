import { researchMethods } from '../studies/research-home';

/**
 * "新建修改版"的 6 种方式：手动改参数，加上原研究页的 5 项（参数率定、不确定性分析、AI 学习参数、
 * AI 替换过程、修改源码）。研究不再单独占侧栏一项，从这里进去（2026-10-21 用户要求：一页选方式，
 * 点了就开始，不再先填表单确认）。
 */
export const hubMethods = [
  {
    id: 'manual',
    zh: '修改参数（手动）',
    en: 'Edit parameters by hand',
    descriptionZh: '按全局、地类、PFT、PC 直接改参数值',
    descriptionEn: 'Change global, land-class, PFT and PC values directly',
    icon: 'M4 6h10M18 6h2M4 12h4M12 12h8M4 18h12M20 18h0M14 4v4M8 10v4M16 16v4',
  },
  ...['tuning', 'uncertainty', 'parameters', 'process'].map((id) =>
    researchMethods.find((m) => m.id === id)!,
  ),
  {
    ...researchMethods.find((m) => m.id === 'source')!,
    descriptionZh: '复制一份代码，直接改 Rust 或 Fortran 文件',
    descriptionEn: 'Copy the code and change Rust or Fortran files directly',
  },
] as const;

export type HubMethod = (typeof hubMethods)[number]['id'];
