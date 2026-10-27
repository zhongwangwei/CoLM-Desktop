import { saveStateText, type SaveState } from '../lib/save-status';

/**
 * 统一的三态保存徽标（U-05）。沿用现有 `badge` 样式，不新增 CSS 类，免得各页面各自造一套色调；
 * `saved` 也显示出来，用户才分得清"这次页面里的内容已经落盘"还是"只是看起来没改"。
 */
export function SaveBadge({
  state,
  t,
}: {
  state: SaveState;
  t: (zh: string, en: string) => string;
}) {
  return (
    <span className={`badge save-badge is-${state}`} data-save-state={state}>
      {saveStateText(state, t)}
    </span>
  );
}
