/**
 * 要用户确认的操作（放弃修改、删除版本、删除工作区……）。应用窗口是 macOS 的 WKWebView，它不实现
 * `window.confirm`：不弹框、直接返回 false，所以这些按钮按了都没反应（2026-10-21 用户：放弃修改按了没有反应；
 * 打包应用里实测点了不弹框、草稿仍是"未保存"）。改用应用自己的对话框，由 `ConfirmHost` 显示。
 *
 * 这里不引用任何组件：`model-tidy` 也要问（改绑模型版本），而组件库又引用 `app-state`，放在一起会成环。
 */
export type ConfirmRequest = {
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
};
export type ConfirmOptions = Omit<ConfirmRequest, 'message' | 'resolve'>;

let host: ((request: ConfirmRequest) => void) | null = null;

export function setConfirmHost(next: ((request: ConfirmRequest) => void) | null) {
  host = next;
}

/** 弹出确认框；没有挂对话框时（比如单元测试）当作"不确认"，不会误删东西。 */
export function askConfirm(message: string, options: ConfirmOptions = {}): Promise<boolean> {
  return new Promise((resolve) => {
    if (!host) resolve(false);
    else host({ message, ...options, resolve });
  });
}
