/**
 * 共享 Dialog 关闭后的键盘焦点（GUI-19）。
 *
 * Radix 的 FocusScope 本来会记下自己挂载时的 activeElement，并在卸载时还回去；但共享 Dialog 没有
 * `Dialog.Trigger`：modal content 的 `onCloseAutoFocus` 会先 `preventDefault()`，再对空的 triggerRef 调
 * `focus()`，于是 FocusScope 的默认恢复也被跳过，焦点落在 BODY（浏览器实测：版本选择弹窗按 Escape）。
 * 所以这里自己记录打开前的有效焦点，并在关闭时选一个真正能聚焦的入口。恢复/回退策略抽成纯函数，
 * 让没有真实 DOM 的 node:test 也能覆盖各个分支。
 */
export type FocusTarget = {
  focus: (options?: { preventScroll?: boolean }) => void;
  isConnected?: boolean;
  matches?: (selectors: string) => boolean;
  closest?: (selectors: string) => Element | null;
};

const DIALOG = '[role="dialog"], [role="alertdialog"]';
const CLOSED = '[data-state="closed"]';
const FOCUSABLE = [
  'button:not([disabled])',
  '[href]',
  'input:not([disabled])',
  'select:not([disabled])',
  'textarea:not([disabled])',
  '[tabindex]:not([tabindex="-1"])',
].join(', ');
const HIDDEN = '[aria-hidden="true"], [hidden]';

function browserDocument(): Document | null {
  // react-dom/server 渲染 UI 组件时没有 document；不要让这里的 DOM 访问把测试/SSR 弄崩。
  return typeof document === 'undefined' ? null : document;
}

/** 记录下来的元素能否当关闭后的焦点：BODY/文档根代表"没有焦点"，已卸载或已禁用的入口也不算。 */
export function usableFocusTarget(target: FocusTarget | null | undefined): target is FocusTarget {
  if (!target || typeof target.focus !== 'function' || target.isConnected === false) return false;
  if ((target as { disabled?: boolean }).disabled === true) return false;
  try {
    return target.matches?.('body, html') !== true;
  } catch {
    return false;
  }
}

/** 打开弹窗前聚焦的元素；无 DOM、焦点在 BODY 或入口不可用时返回 null，关闭时改走回退位置。 */
export function dialogOpenerTarget(doc: Document | null = browserDocument()): FocusTarget | null {
  const active = doc?.activeElement as unknown as FocusTarget | null;
  return usableFocusTarget(active) ? active : null;
}

function firstFocusable(root: ParentNode | null | undefined): FocusTarget | null {
  if (!root) return null;
  for (const candidate of Array.from(root.querySelectorAll(FOCUSABLE))) {
    if (candidate.closest(HIDDEN)) continue;
    const target = candidate as unknown as FocusTarget;
    if (usableFocusTarget(target)) return target;
  }
  return null;
}

/** 关闭中的弹窗（data-state=closed）不算还开着的；嵌套时最后挂载的那个是最上层。 */
function topOpenDialog(doc: Document): Element | null {
  const dialogs = Array.from(doc.querySelectorAll(DIALOG)).filter(
    (dialog) => !dialog.matches(CLOSED),
  );
  return dialogs.at(-1) ?? null;
}

/**
 * 入口不可用时的回退：先给还开着的上层弹窗（嵌套时内层先关，外层此刻可能被 aria-hidden），
 * 再给页面主区域的第一个可聚焦控件。都没有就返回 null。
 */
export function fallbackFocusTarget(doc: Document | null = browserDocument()): FocusTarget | null {
  if (!doc) return null;
  return firstFocusable(topOpenDialog(doc)) ?? firstFocusable(doc.querySelector('main'));
}

/**
 * 关闭弹窗时恢复焦点。返回 true 表示已经接管（调用方 preventDefault 并聚焦），false 表示保持
 * Radix 的默认行为。连续弹窗里另一个弹窗已经接住焦点时什么都不做，避免把焦点抢回旧页面。
 */
export function restoreDialogFocus(
  opener: FocusTarget | null,
  closeEvent?: { preventDefault?: () => void },
  doc: Document | null = browserDocument(),
): boolean {
  if (!doc) return false;
  const active = doc.activeElement as unknown as FocusTarget | null;
  const holder = active?.closest?.(DIALOG) ?? null;
  if (holder && !holder.matches(CLOSED)) return false;
  const target = usableFocusTarget(opener) ? opener : fallbackFocusTarget(doc);
  if (!target) return false;
  try {
    target.focus({ preventScroll: true });
  } catch {
    return false;
  }
  closeEvent?.preventDefault?.();
  return true;
}
