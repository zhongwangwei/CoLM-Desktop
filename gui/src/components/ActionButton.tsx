import { useRef, type ComponentProps } from 'react';
import { Button } from './ui';
import { useRenderedAction } from '../lib/guide';

/**
 * 界面按钮兼 AI 助手可按的动作：渲染出来才登记，没显示就不存在，名字、能否按都取自这个按钮本身。
 * `run` 的返回值交给助手作为结果；界面点击时它的错误由 `run` 自己提示，这里只吞掉 Promise 拒绝。
 */
export function ActionButton({
  action,
  label,
  kind = 'click',
  run,
  children,
  ...props
}: Omit<ComponentProps<typeof Button>, 'onClick'> & {
  /** 助手用的动作名（`ui_click` / `ui_commit` 的 target）。 */
  action: string;
  /** 助手看到的说明；按钮上没有别的内容时也作为按钮文字。 */
  label: string;
  /** `commit`：会保存或运行东西，助手要先征得用户同意。 */
  kind?: 'click' | 'commit';
  run: () => unknown | Promise<unknown>;
}) {
  const ref = useRef<HTMLButtonElement>(null);
  useRenderedAction({ id: action, label, kind, run }, ref);
  return (
    <Button
      ref={ref}
      data-action={action}
      onClick={() => {
        // 同步抛出的错误也要接住：run 自己已经提示过用户。
        try {
          void Promise.resolve(run()).catch(() => {});
        } catch {
          /* 已由 run 提示 */
        }
      }}
      {...props}
    >
      {children ?? label}
    </Button>
  );
}
