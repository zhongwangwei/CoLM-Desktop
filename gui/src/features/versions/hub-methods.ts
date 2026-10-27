import { researchMethods } from '../studies/research-home';
import type { AppState, StudyPick, VersionRef } from '../../lib/types';

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

/**
 * 进入研究页时要带过去的 `research*` 状态（U-02 两个入口共用一份定义）。
 *
 * 从"新建修改版"来：用户在版本页选了基线，研究用它读参数、建研究；这**不是**给模拟换版本，
 * 所以只写 `researchBaseline`，绝不碰 version/versionRef/chosenVersion（U-01）。
 * 从研究首页（侧栏或首页方法卡片）来：没有显式基线，清成 null 让研究页退回当前运行修订。
 *
 * 两个入口都顺手清掉上一个草稿带来的 `researchTarget`/`researchSeed`：它们只属于从该草稿的
 * "研究"页签进入的那次研究，留着会让下一次研究悄悄写回旧草稿或勾上旧参数。
 */
export type ResearchEntryState = Pick<
  AppState,
  'researchBaseline' | 'researchMethod' | 'researchSeed' | 'researchTarget'
>;

export function hubResearchEntry(method: string, baseline: VersionRef): ResearchEntryState {
  return {
    // 复制一份：调用方之后改版本对象也不该影响已带过去的研究基线。
    researchBaseline: { ...baseline },
    researchMethod: method,
    researchSeed: null,
    researchTarget: null,
  };
}

export function homeResearchEntry(): ResearchEntryState {
  return {
    researchBaseline: null,
    researchMethod: null,
    researchSeed: null,
    researchTarget: null,
  };
}

/** 从某个草稿的"研究"页签进入时带的参数种子（写回目标由调用方在冻结后补上）。 */
export function studyResearchEntry(
  method: string,
  picks: StudyPick[],
): Pick<ResearchEntryState, 'researchMethod' | 'researchSeed'> {
  return {
    researchMethod: method,
    researchSeed: picks.length ? { parameters: picks } : null,
  };
}
