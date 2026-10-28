/**
 * 桌面壳的布局决策（任务书第 6 节 U-06）。
 *
 * 审查实测：960px 窗口里助手展开（320px）时，左栏 208px + 助手 320px 只剩约 432px 主区；
 * 1240px 窗口主区约 712px。这里把"侧栏 full/rail、助手分栏/覆盖抽屉、主区宽度"算成纯数据，
 * App 只负责套类名和 CSS 变量，测试可以在没有真实浏览器的情况下覆盖各尺寸组合。
 *
 * 两条规则：
 * 1. 窄窗口（≤ 1100px）助手改成覆盖式抽屉，不占网格列——主区立刻多出 280–600px；
 * 2. 分栏模式下助手宽度最多让主区保持 MIN_MAIN_WIDTH，拖动条只在分栏时存在（宽度会被夹住，
 *    不会出现"越拖主区越窄"）。
 *
 * 只有"收起导航"一个视图开关：它保留导航、仍可点；想再腾地方时把助手一起关掉即可（工具栏已有
 * AI 助手开关），不再额外提供"专注视图"这个和前者重叠的开关。
 */
export type ShellNav = 'full' | 'rail';
export type ShellAssistant = 'column' | 'drawer' | 'hidden';

export type ShellLayout = {
  nav: ShellNav;
  assistant: ShellAssistant;
  /** 侧栏实际占的网格宽度（rail 64、full 232/208）。 */
  sidebarWidth: number;
  /** 助手实际占的宽度：分栏是网格列宽，抽屉是覆盖层宽度，隐藏为 0。 */
  assistantWidth: number;
  /** 主工作区可用宽度；分栏、抽屉都由它证明"主区没有被挤没"。 */
  mainWidth: number;
  /** 拆分条只在分栏模式显示；抽屉没有可拖的列。 */
  splitter: boolean;
};

export type ShellLayoutInput = {
  viewportWidth: number;
  assistantOpen: boolean;
  navCollapsed?: boolean;
  assistantWidth?: number;
};

/** 宽窗口完整侧栏；≤1240px 用 208px，和原先媒体查询的数值保持连续。 */
export const NAV_WIDTH_WIDE = 232;
export const NAV_WIDTH_NARROW = 208;
/** 收起导航后的图标 rail。 */
export const NAV_WIDTH_RAIL = 64;
/** 低于它用窄侧栏；和原 @media (max-width: 1240px) 对齐。 */
export const NARROW_VIEWPORT = 1240;
/** 助手从分栏改成覆盖式抽屉的窗口宽度上限。 */
export const ASSISTANT_DRAWER_BELOW = 1100;
export const ASSISTANT_MIN_WIDTH = 280;
export const ASSISTANT_MAX_WIDTH = 600;
export const ASSISTANT_DEFAULT_WIDTH = 320;
/** 分栏时给主工作区留的最小宽度；不够就让位给抽屉，而不是继续缩字体/缩内容。 */
export const MIN_MAIN_WIDTH = 560;

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

export function shellLayout(input: ShellLayoutInput): ShellLayout {
  const viewport = Number.isFinite(input.viewportWidth)
    ? Math.max(0, Math.round(input.viewportWidth))
    : 0;
  const nav: ShellNav = input.navCollapsed === true ? 'rail' : 'full';
  const sidebarWidth =
    nav === 'rail'
      ? NAV_WIDTH_RAIL
      : viewport <= NARROW_VIEWPORT
        ? NAV_WIDTH_NARROW
        : NAV_WIDTH_WIDE;
  const mainBehindSidebar = Math.max(0, viewport - sidebarWidth);
  if (!input.assistantOpen)
    return {
      nav,
      assistant: 'hidden',
      sidebarWidth,
      assistantWidth: 0,
      mainWidth: mainBehindSidebar,
      splitter: false,
    };
  const requested =
    typeof input.assistantWidth === 'number' && Number.isFinite(input.assistantWidth)
      ? input.assistantWidth
      : ASSISTANT_DEFAULT_WIDTH;
  const clamped = clamp(Math.round(requested), ASSISTANT_MIN_WIDTH, ASSISTANT_MAX_WIDTH);
  if (viewport <= ASSISTANT_DRAWER_BELOW) {
    // 抽屉覆盖在主区上，不占列；宽度也不能大过窗口，否则右侧内容被推出屏幕。
    const drawerWidth = Math.min(clamped, Math.max(ASSISTANT_MIN_WIDTH, viewport - NAV_WIDTH_RAIL));
    return {
      nav,
      assistant: 'drawer',
      sidebarWidth,
      assistantWidth: drawerWidth,
      mainWidth: mainBehindSidebar,
      splitter: false,
    };
  }
  // 分栏模式：助手列最宽只能到"主区还剩 MIN_MAIN_WIDTH"，拖动条仍存在，可以再往回拖。
  const columnWidth = Math.max(
    ASSISTANT_MIN_WIDTH,
    Math.min(clamped, mainBehindSidebar - MIN_MAIN_WIDTH),
  );
  return {
    nav,
    assistant: 'column',
    sidebarWidth,
    assistantWidth: columnWidth,
    mainWidth: mainBehindSidebar - columnWidth,
    splitter: true,
  };
}

/**
 * 抽屉打开时按 Escape 收起。已有对话框处理过 Escape（Radix 在 document 捕获阶段 preventDefault），
 * 这里不抢：否则关掉确认框会连带收起后面的助手抽屉。
 */
export function drawerEscapeCloses(
  event: { key: string; defaultPrevented?: boolean },
  drawerOpen: boolean,
): boolean {
  return drawerOpen && event.key === 'Escape' && event.defaultPrevented !== true;
}
