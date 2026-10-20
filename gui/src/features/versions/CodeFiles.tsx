import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson } from '../../lib/ipc';
import { Button, Field, Input, Notice } from '../../components/ui';

type Entry = { name: string; path: string; kind: 'file' | 'dir' | 'link'; size: number };
type FileText = { path: string; size: number; text: string | null };

const parentOf = (path: string) => path.split('/').slice(0, -1).join('/');
const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
const sizeLabel = (bytes: number) =>
  bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : bytes >= 1024
      ? `${(bytes / 1024).toFixed(1)} KB`
      : `${bytes} B`;

/**
 * 修改版的代码文件：浏览源码目录，文本文件直接在这里编辑保存；任何文件或整个目录都能用本机的
 * 文件、文件夹替换（可以是二进制，比如查找表），也能新增和删除。每次改动都是工作区里的一次提交。
 */
export default function CodeFiles({
  name,
  message,
  en,
  disabled,
  onChanged,
}: {
  name: string;
  message: string;
  en: boolean;
  disabled: boolean;
  onChanged: () => Promise<void>;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [dir, setDir] = useState('');
  const [entries, setEntries] = useState<Entry[]>([]);
  const [open, setOpen] = useState<FileText | null>(null);
  const [draft, setDraft] = useState('');
  const [newPath, setNewPath] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const token = useRef(0);
  const edited = open?.text != null && draft !== open.text;
  async function load(next = dir) {
    const mine = ++token.current;
    const value = await invokeJson<{ entries: Entry[] }>('workspace_files', { name, dir: next });
    if (mine === token.current) {
      setDir(next);
      setEntries(value.entries);
    }
  }
  useEffect(() => {
    setOpen(null);
    void load('').catch((e) => setError(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name]);
  async function act(work: () => Promise<unknown>, done: string) {
    if (busy || disabled) return;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      // 选文件时点了取消：什么都没做，不提示成功。
      if ((await work()) === false) return;
      setNotice(done);
      await load();
      await onChanged();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function openFile(path: string) {
    if (edited && !window.confirm(t('当前文件的修改还没保存，放弃吗？', 'Discard unsaved edits?')))
      return;
    setError('');
    const file = await invokeJson<FileText>('workspace_file', { name, path });
    setOpen(file);
    setDraft(file.text ?? '');
  }
  const needMessage = !message.trim();
  const save = () =>
    act(
      async () => {
        await invokeJson('workspace_write_file', { name, path: open!.path, text: draft, message });
        setOpen({ ...open!, text: draft, size: new TextEncoder().encode(draft).length });
      },
      t(`已保存 ${open?.path}`, `Saved ${open?.path}`),
    );
  const replaceWithFile = (path: string) =>
    act(
      async () => {
        const from = await invoke<string | null>('pick_file', { key: 'workspace-put' });
        if (!from) return false;
        await invokeJson('workspace_put', { name, path, from, folder: false, message });
        if (open?.path === path) await openFile(path);
      },
      t(`已替换 ${path}`, `Replaced ${path}`),
    );
  const replaceWithFolder = (path: string) =>
    act(
      async () => {
        const from = await invoke<string | null>('pick_folder', { key: 'workspace-put-dir' });
        if (!from) return false;
        if (
          !window.confirm(
            t(
              `用所选文件夹整个替换 ${path}？原目录里的文件都会被换掉（改动可以撤回）。`,
              `Replace ${path} entirely with the chosen folder? Every file in it is replaced (the change can be reverted).`,
            ),
          )
        )
          return false;
        await invokeJson('workspace_put', { name, path, from, folder: true, message });
        setOpen(null);
      },
      t(`已替换目录 ${path}`, `Replaced folder ${path}`),
    );
  const remove = (path: string) =>
    act(
      async () => {
        if (
          !window.confirm(t(`删除 ${path}？（改动可以撤回）`, `Delete ${path}? (can be reverted)`))
        )
          return false;
        await invokeJson('workspace_remove', { name, path, message });
        if (open?.path.startsWith(path)) setOpen(null);
      },
      t(`已删除 ${path}`, `Deleted ${path}`),
    );
  const createFile = () => {
    const path = joinPath(dir, newPath.trim());
    return act(
      async () => {
        await invokeJson('workspace_write_file', { name, path, text: '', message });
        setNewPath('');
        await openFile(path);
      },
      t(`已新建 ${path}`, `Created ${path}`),
    );
  };
  return (
    <section className="code-files" aria-label={t('代码文件', 'Code files')}>
      <div className="code-files-head">
        <h3>{t('代码文件', 'Code files')}</h3>
        <span className="muted">
          {t(
            'Rust 引擎在 crates/，Fortran 上游在 vendor/CoLM202X/；每次保存、替换、删除都是一次可以撤回的改动。',
            'The Rust engine is under crates/, upstream Fortran under vendor/CoLM202X/; every save, replacement or deletion is one revertible change.',
          )}
        </span>
      </div>
      {needMessage && (
        <Notice tone="warning">
          {t('先在上面填写"修改说明"，再保存或替换。', 'Fill in the change message above first.')}
        </Notice>
      )}
      {error && <Notice tone="error">{error}</Notice>}
      {notice && <Notice tone="success">{notice}</Notice>}
      <div className="code-files-body">
        <div className="code-files-tree">
          <div className="code-files-path">
            <button
              type="button"
              className="button button-ghost"
              disabled={!dir || busy}
              onClick={() => void load(parentOf(dir)).catch((e) => setError(String(e)))}
            >
              ↑
            </button>
            <code>{dir || '/'}</code>
          </div>
          <ul>
            {entries.map((entry) => (
              <li key={entry.path} className={open?.path === entry.path ? 'is-open' : undefined}>
                <button
                  type="button"
                  className="code-files-entry"
                  disabled={busy || entry.kind === 'link'}
                  onClick={() =>
                    void (entry.kind === 'dir' ? load(entry.path) : openFile(entry.path)).catch(
                      (e) => setError(String(e)),
                    )
                  }
                >
                  <span aria-hidden="true">
                    {entry.kind === 'dir' ? '▸' : entry.kind === 'link' ? '↪' : '·'}
                  </span>
                  {entry.name}
                  {entry.kind === 'file' && <small>{sizeLabel(entry.size)}</small>}
                </button>
              </li>
            ))}
          </ul>
          <div className="code-files-actions">
            <Input
              value={newPath}
              placeholder={t('新文件名，如 MOD_Mine.F90', 'New file name, e.g. MOD_Mine.F90')}
              disabled={busy}
              onChange={(e) => setNewPath(e.target.value)}
            />
            <Button
              disabled={busy || disabled || needMessage || !newPath.trim()}
              onClick={createFile}
            >
              {t('新建文件', 'New file')}
            </Button>
            <Button
              disabled={busy || disabled || needMessage}
              onClick={() =>
                void (async () => {
                  const from = await invoke<string | null>('pick_file', { key: 'workspace-put' });
                  if (!from) return;
                  const fileName = from.split(/[\\/]/).at(-1)!;
                  await act(
                    () =>
                      invokeJson('workspace_put', {
                        name,
                        path: joinPath(dir, fileName),
                        from,
                        folder: false,
                        message,
                      }),
                    t(`已加入 ${fileName}`, `Added ${fileName}`),
                  );
                })()
              }
            >
              {t('从本机加入文件…', 'Add a file from this computer…')}
            </Button>
            {dir && (
              <>
                <Button
                  disabled={busy || disabled || needMessage}
                  onClick={() => replaceWithFolder(dir)}
                >
                  {t('用本机文件夹替换这个目录…', 'Replace this folder from this computer…')}
                </Button>
                <Button
                  variant="danger"
                  disabled={busy || disabled || needMessage}
                  onClick={() => remove(dir)}
                >
                  {t('删除这个目录', 'Delete this folder')}
                </Button>
              </>
            )}
          </div>
        </div>
        <div className="code-files-editor">
          {!open ? (
            <p className="muted">
              {t('在左边选一个文件打开。', 'Choose a file on the left to open it.')}
            </p>
          ) : (
            <>
              <div className="code-files-path">
                <code>{open.path}</code>
                <small className="muted">{sizeLabel(open.size)}</small>
                {edited && <span className="badge">{t('未保存', 'Unsaved')}</span>}
              </div>
              {open.text == null ? (
                <Notice>
                  {t(
                    '这是二进制文件或超过 2 MB，不能在这里编辑；可以用本机文件替换。',
                    'Binary or larger than 2 MB: it cannot be edited here, but it can be replaced from this computer.',
                  )}
                </Notice>
              ) : (
                <textarea
                  className="code-files-text"
                  spellCheck={false}
                  value={draft}
                  disabled={busy || disabled}
                  onChange={(e) => setDraft(e.target.value)}
                />
              )}
              <div className="code-files-actions">
                {open.text != null && (
                  <Button
                    variant="primary"
                    disabled={busy || disabled || needMessage || !edited}
                    onClick={save}
                  >
                    {t('保存', 'Save')}
                  </Button>
                )}
                {edited && (
                  <Button disabled={busy} onClick={() => setDraft(open.text ?? '')}>
                    {t('放弃修改', 'Discard edits')}
                  </Button>
                )}
                <Button
                  disabled={busy || disabled || needMessage}
                  onClick={() => replaceWithFile(open.path)}
                >
                  {t('用本机文件替换…', 'Replace from this computer…')}
                </Button>
                <Button
                  variant="danger"
                  disabled={busy || disabled || needMessage}
                  onClick={() => remove(open.path)}
                >
                  {t('删除', 'Delete')}
                </Button>
              </div>
            </>
          )}
        </div>
      </div>
      <Field label={t('按路径打开', 'Open by path')}>
        <Input
          placeholder="vendor/CoLM202X/main/MOD_Vars_TimeInvariants.F90"
          disabled={busy}
          onKeyDown={(e) => {
            if (e.key !== 'Enter') return;
            const path = (e.target as HTMLInputElement).value.trim();
            if (!path) return;
            void load(parentOf(path))
              .then(() => openFile(path))
              .catch((err) => setError(String(err)));
          }}
        />
      </Field>
    </section>
  );
}
