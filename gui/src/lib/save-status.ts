/**
 * 保存状态的三态语义（U-05）：界面统一说「有未保存修改 / 正在保存 / 已保存」，字段一改就把过时的
 * 成功提示清掉。抽成纯函数是因为各页面（草稿、评估配置、设置对话框、数据准备）都要用同一套措辞，
 * 又不想让每个页面各写一遍中英文。
 */
export type SaveState = 'saved' | 'dirty' | 'saving';

export function saveStateText(state: SaveState, t: (zh: string, en: string) => string): string {
  switch (state) {
    case 'dirty':
      return t('有未保存修改', 'Unsaved changes');
    case 'saving':
      return t('正在保存', 'Saving…');
    case 'saved':
      return t('已保存', 'Saved');
  }
}
