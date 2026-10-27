import { useState } from 'react';
import { Button, Dialog } from '../../components/ui';
import { applySourceChoice, type SourceChoice, type SourceEditState } from './source-edits';

/**
 * 源码未保存时的三选一。用于编译、冻结这类"必须用磁盘上代码"的动作之前：保存并继续 / 放弃修改并
 * 继续 / 取消。取消保留文本和原页面；保存失败时关闭对话框、由编辑区显示错误，文本仍在缓冲区里。
 */
export function UnsavedSourceDialog({
  open,
  en,
  canSave,
  saving,
  onChoose,
}: {
  open: boolean;
  en: boolean;
  canSave: boolean;
  saving: boolean;
  onChoose: (choice: SourceChoice) => void;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) onChoose('cancel');
      }}
      showClose={false}
      title={t('源码还有修改没保存', 'Source edits are not saved yet')}
      description={t(
        '接下来要用磁盘上的源码。先保存当前文件的修改，还是放弃它们？取消会保留文本和这个页面。',
        'The next step uses the source on disk. Save the open file first, or discard its edits? Cancel keeps your text and this page.',
      )}
    >
      <div className="button-row">
        <Button variant="primary" disabled={!canSave || saving} onClick={() => onChoose('save')}>
          {t('保存并继续', 'Save and continue')}
        </Button>
        <Button variant="danger" disabled={saving} onClick={() => onChoose('discard')}>
          {t('放弃修改并继续', 'Discard edits and continue')}
        </Button>
        <Button disabled={saving} onClick={() => onChoose('cancel')}>
          {t('取消', 'Cancel')}
        </Button>
      </div>
      {!canSave && (
        <p className="muted">
          {t(
            '先在「代码」页签填写“修改说明”，才能保存源码文件。',
            'Fill in the change message under the Code tab before saving the source file.',
          )}
        </p>
      )}
    </Dialog>
  );
}

/**
 * 把"是否需要三选一"和对话框绑在一起。`ask` 在动作前调用：没有未保存源码立即放行（返回 true），
 * 否则等用户选择。同一个页面只挂一个对话框实例，避免连点两个按钮叠出两个框。
 */
export function useSourceChoicePrompt(en: boolean) {
  const [pending, setPending] = useState<{
    state: SourceEditState;
    resolve: (ok: boolean) => void;
  } | null>(null);
  const [saving, setSaving] = useState(false);
  const ask = (state: SourceEditState | null): Promise<boolean> => {
    if (!state?.dirty) return Promise.resolve(true);
    return new Promise<boolean>((resolve) => setPending({ state, resolve }));
  };
  const choose = (choice: SourceChoice) => {
    if (!pending || saving) return;
    const request = pending;
    if (choice !== 'save') {
      setPending(null);
      void applySourceChoice(request.state, choice).then(request.resolve);
      return;
    }
    setSaving(true);
    void applySourceChoice(request.state, choice)
      // 保存失败也关闭对话框：错误由 CodeFiles 的 Notice 显示，用户可在编辑区重试或再点一次。
      .then((ok) => {
        setPending(null);
        request.resolve(ok);
      })
      .finally(() => setSaving(false));
  };
  const dialog = (
    <UnsavedSourceDialog
      open={!!pending}
      en={en}
      canSave={pending?.state.canSave ?? false}
      saving={saving}
      onChoose={choose}
    />
  );
  return { ask, dialog };
}
