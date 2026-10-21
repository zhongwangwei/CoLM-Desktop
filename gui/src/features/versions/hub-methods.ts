import { researchMethods } from '../studies/research-home';

/**
 * "新建修改版"的 6 种方式：手动改参数，加上原研究页的 5 项（参数率定、不确定性分析、AI 学习参数、
 * AI 替换过程、修改源码）。一页选方式，点了就开始，不再先填表单确认（2026-10-21 用户要求）。
 * 侧栏的研究首页也列这 6 种（同日用户要求把手动改参数也放到研究页）。
 */
export const hubMethods = [
  {
    id: 'manual',
    zh: '修改参数（手动）',
    en: 'Edit parameters by hand',
    descriptionZh: '按全局、地类、PFT、PC 直接改参数值',
    descriptionEn: 'Change global, land-class, PFT and PC values directly',
    // 铅笔：原来的滑杆图标和"参数率定"几乎一样，两张卡片并排时分不出来。
    icon: 'M12 20h9M16.5 3.5a2.12 2.12 0 013 3L7 19l-4 1 1-4z',
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
