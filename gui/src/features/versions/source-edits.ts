/**
 * 源码编辑的未保存状态：CodeFiles 是真正的缓冲区持有者，CodeWorkspace / VersionsPage 只通过这个
 * 只读视图在"离开、切工作区、关窗、编译、冻结"前统一处理它（GUI-02）。
 *
 * 用 getter 而不是每次状态变化都往上抛一份快照：父组件在用户点按钮/导航的那一刻读到的必须是最新
 * 值；如果靠 React 状态同步，保存刚失败、文本框刚改一个字符这类瞬间会拿到上一帧的脏标记。
 */
export type SourceSaveBlockedReason = 'unreadable' | 'conflict' | 'message' | 'busy';

export type SourceEditState = {
  /** 有文本被改过但还没保存。 */
  readonly dirty: boolean;
  /** 有保存/新建/替换/删除在飞（父工作台此时不许冻结/编译/切对象）。 */
  readonly busy: boolean;
  /** 当前打开的文件路径；没打开为 null。 */
  readonly path: string | null;
  /** 现在保存能否成功（打开的是文本、且填写了"修改说明"）。 */
  readonly canSave: boolean;
  readonly saveBlockedReason?: SourceSaveBlockedReason | null;
  save: () => Promise<boolean>;
  discard: () => void;
};

export type SourceChoice = 'save' | 'discard' | 'cancel';

/**
 * 三选一之后的动作：返回 true 表示可以继续原来的操作。
 *
 * 保存失败返回 false 且**不丢文本**（文本始终在 CodeFiles 的缓冲区里，错误由它显示）；放弃是用户
 * 明确选的，直接清空缓冲区；取消什么都不做、页面原样保留。
 */
export async function applySourceChoice(
  state: SourceEditState | null,
  choice: SourceChoice,
): Promise<boolean> {
  if (choice === 'cancel') return false;
  if (choice === 'discard') {
    state?.discard();
    return true;
  }
  if (!state) return true;
  if (!state.canSave) return false;
  return await state.save();
}

/** 导航守卫返回的原因：没有未保存源码返回 null（同页页签切换不经过守卫，缓冲区因此保留）。 */
export function sourceGuardReason(
  state: SourceEditState | null,
  t: (zh: string, en: string) => string,
): string | null {
  if (!state?.dirty) return null;
  return t(
    '源码文件有未保存的修改，请先保存或放弃',
    'Source files have unsaved edits; save or discard them first',
  );
}

export function sourceSaveBlockedText(
  reason: SourceSaveBlockedReason | null | undefined,
  en: boolean,
): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  if (reason === 'busy')
    return t(
      '请先保存草稿修改，并等待当前操作完成，再保存源码。',
      'Save the draft changes and wait for the current operation before saving the source.',
    );
  if (reason === 'conflict')
    return t(
      '当前源码与磁盘内容冲突，请先在「代码」页处理冲突。',
      'The source conflicts with the file on disk. Resolve the conflict under the Code tab first.',
    );
  if (reason === 'unreadable')
    return t(
      '当前源码文件不可读取或不是文本文件，请先在「代码」页检查文件。',
      'The source file is unreadable or is not text. Check the file under the Code tab first.',
    );
  if (reason === 'message')
    return t(
      '先在「代码」页签填写“修改说明”，才能保存源码文件。',
      'Fill in the change message under the Code tab before saving the source file.',
    );
  return t(
    '当前源码暂时不能保存，请在「代码」页查看原因。',
    'The source cannot be saved right now. Check the Code tab for the reason.',
  );
}
