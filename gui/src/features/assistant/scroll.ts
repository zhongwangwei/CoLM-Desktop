/**
 * 流式回答的"跟随底部"状态（U-07）。用户往上看历史时不能每来一段增量就把视口拽回底部，
 * 但也不能让新内容悄悄堆在下面 —— 暂停时记一个未读标记，滚回底部后恢复跟随。
 *
 * 抽成纯函数是因为滚动位置只能在有 DOM 的浏览器里真实测量，而 node:test 没有 DOM；
 * 这里只处理"距底部多少、要不要跟随"的判定，真实 scrollTop/scrollHeight 由页面读取后传进来。
 */
export interface FollowState {
  /** 视口是否还在底部、是否要继续跟随新内容。 */
  stick: boolean;
  /** 暂停跟随时，底部是否有用户还没看到的新内容。 */
  unread: boolean;
}

export interface ScrollMetrics {
  scrollTop: number;
  scrollHeight: number;
  clientHeight: number;
}

/** 距底部这个像素以内都算"还在底部"：滚动条有像素取整误差，要求严格等于 0 会永远追不上。 */
export const BOTTOM_SLACK_PX = 24;

export function atBottom(metrics: ScrollMetrics): boolean {
  return metrics.scrollHeight - metrics.scrollTop - metrics.clientHeight <= BOTTOM_SLACK_PX;
}

/** 用户滚动后：在底部就恢复跟随并清掉未读；不在底部就暂停，但保留已有未读标记。 */
export function followAfterScroll(state: FollowState, metrics: ScrollMetrics): FollowState {
  const bottom = atBottom(metrics);
  return { stick: bottom, unread: bottom ? false : state.unread };
}

/** 新内容到达后：跟随中不需要提示；暂停中标记"有新内容"。 */
export function followAfterContent(state: FollowState): FollowState {
  return state.stick ? { stick: true, unread: false } : { stick: false, unread: true };
}

/** 点"有新内容"回到最新：恢复跟随、清未读。 */
export function followAfterJump(): FollowState {
  return { stick: true, unread: false };
}
