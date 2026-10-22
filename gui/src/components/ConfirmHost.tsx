import { useEffect, useRef, useState } from 'react';
import { useApp } from '../lib/app-state';
import { setConfirmHost, type ConfirmRequest } from '../lib/confirm';
import { Button, Dialog } from './ui';

/** `askConfirm` 的对话框；整个应用挂一个。新的确认来了，还没答的那个当作"不确认"。 */
export default function ConfirmHost() {
  const { state } = useApp();
  const en = state.language === 'en';
  const [request, setRequest] = useState<ConfirmRequest | null>(null);
  const current = useRef<ConfirmRequest | null>(null);
  useEffect(() => {
    setConfirmHost((next) => {
      current.current?.resolve(false);
      current.current = next;
      setRequest(next);
    });
    return () => setConfirmHost(null);
  }, []);
  const close = (ok: boolean) => {
    const answered = current.current;
    current.current = null;
    setRequest(null);
    answered?.resolve(ok);
  };
  if (!request) return null;
  return (
    <Dialog
      open
      showClose={false}
      className="confirm-dialog"
      title={en ? 'Please confirm' : '请确认'}
      onOpenChange={(open) => !open && close(false)}
      // 问题本身作对话框的描述（aria-describedby）：原来是普通段落，读屏只念"请确认"和两个按钮（GUI 审查 S08）。
      description={<span className="confirm-message">{request.message}</span>}
    >
      <div className="button-row confirm-actions">
        <Button onClick={() => close(false)}>
          {request.cancelLabel ?? (en ? 'Cancel' : '取消')}
        </Button>
        <Button variant={request.danger ? 'danger' : 'primary'} onClick={() => close(true)}>
          {request.confirmLabel ?? (en ? 'OK' : '确定')}
        </Button>
      </div>
    </Dialog>
  );
}
