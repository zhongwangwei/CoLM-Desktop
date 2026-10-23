import { useEffect, useRef, useState } from 'react';
import { useApp } from '../lib/app-state';
import { setConfirmHost, type ConfirmRequest } from '../lib/confirm';
import { usePageContract } from '../lib/guide';
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
  // 确认框开着时登记成助手眼里的"对话框"，而且不给它任何字段和按钮：助手不能替用户回答确认，也不能趁
  // 对话框开着去切页面、改后面的字段（GUI 审查 A-04；原来这个对话框不在 guide 的 modal 判断里）。
  usePageContract(
    {
      title: en ? 'Please confirm' : '请确认',
      step: 'confirm',
      modal: true,
      status: request?.message ?? '',
      fields: [],
      actions: [],
    },
    !!request,
  );
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
