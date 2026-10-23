import { useEffect, useRef, useState } from 'react';
import { invoke, invokeJson } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { Button, Field, Input, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { workspaceErrorText } from './workspace-errors';

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
 * 给了 `source`（一个冻结版本）就只读浏览它冻结时的代码，不能改。
 */
export default function CodeFiles({
  name,
  message,
  en,
  disabled,
  onChanged,
  source,
  editor = '',
}: {
  name: string;
  message: string;
  en: boolean;
  disabled: boolean;
  onChanged: () => Promise<void>;
  source?: { id: string; revision: number };
  /** 用来打开文件的本地程序（`CodeWorkspace` 里选的）；空着用系统默认程序。 */
  editor?: string;
}) {
  const readOnly = !!source;
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
  const opening = useRef(0);
  const edited = open?.text != null && draft !== open.text;
  async function load(next = dir) {
    const mine = ++token.current;
    const value = source
      ? await invokeJson<{ entries: Entry[] }>('model_code_files', { ...source, dir: next })
      : await invokeJson<{ entries: Entry[] }>('workspace_files', { name, dir: next });
    if (mine === token.current) {
      setDir(next);
      setEntries(value.entries);
    }
  }
  useEffect(() => {
    opening.current++;
    setOpen(null);
    void load('').catch((e) => setError(String(e)));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, source?.id, source?.revision]);
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
    if (
      edited &&
      !(await askConfirm(t('当前文件的修改还没保存，放弃吗？', 'Discard unsaved edits?'), {
        danger: true,
      }))
    )
      return;
    setError('');
    // 连点两个文件时后发先至：只认最后一次点的那个，否则编辑框里是 B、保存却写进 A（GUI 审查 B-07）。
    const mine = ++opening.current;
    try {
      const file = source
        ? await invokeJson<FileText>('model_code_file', { ...source, path })
        : await invokeJson<FileText>('workspace_file', { name, path });
      if (mine !== opening.current) return;
      setOpen(file);
      setDraft(file.text ?? '');
    } catch (e) {
      if (mine === opening.current) setError(String(e));
    }
  }
  const needMessage = !readOnly && !message.trim();
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
          !(await askConfirm(
            t(
              `用所选文件夹整个替换 ${path}？原目录里的文件都会被换掉（改动可以撤回）。`,
              `Replace ${path} entirely with the chosen folder? Every file in it is replaced (the change can be reverted).`,
            ),
            { danger: true },
          ))
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
          !(await askConfirm(
            t(`删除 ${path}？（改动可以撤回）`, `Delete ${path}? (can be reverted)`),
            {
              danger: true,
            },
          ))
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
          {readOnly
            ? t(
                'Rust 引擎在 crates/，Fortran 上游在 vendor/CoLM202X/；冻结时的代码，只读。',
                'The Rust engine is under crates/, upstream Fortran under vendor/CoLM202X/; the frozen code, read-only.',
              )
            : t(
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
          <div className="code-files-actions" hidden={readOnly}>
            <Input
              value={newPath}
              placeholder={t('新文件名，如 MOD_Mine.F90', 'New file name, e.g. MOD_Mine.F90')}
              disabled={busy}
              onChange={(e) => setNewPath(e.target.value)}
            />
            <ActionButton
              action="code-file-create"
              label={t('新建文件', 'New file')}
              kind="commit"
              disabled={busy || disabled || needMessage || !newPath.trim()}
              run={createFile}
            >
              {t('新建文件', 'New file')}
            </ActionButton>
            <ActionButton
              action="code-file-add"
              label={t('从本机加入文件…', 'Add a file from this computer…')}
              kind="commit"
              disabled={busy || disabled || needMessage}
              run={() =>
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
            </ActionButton>
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
                  {readOnly
                    ? t(
                        '这是二进制文件或超过 2 MB，不在这里显示。',
                        'Binary or larger than 2 MB; not shown here.',
                      )
                    : t(
                        '这是二进制文件或超过 2 MB，不能在这里编辑；可以用本机文件替换。',
                        'Binary or larger than 2 MB: it cannot be edited here, but it can be replaced from this computer.',
                      )}
                </Notice>
              ) : (
                <textarea
                  className="code-files-text"
                  spellCheck={false}
                  value={draft}
                  readOnly={readOnly}
                  disabled={!readOnly && (busy || disabled)}
                  onChange={(e) => setDraft(e.target.value)}
                />
              )}
              <div className="code-files-actions" hidden={readOnly}>
                {open.text != null && (
                  <ActionButton
                    action="code-file-save"
                    label={t('保存', 'Save')}
                    kind="commit"
                    variant="primary"
                    disabled={busy || disabled || needMessage || !edited}
                    run={save}
                  >
                    {t('保存', 'Save')}
                  </ActionButton>
                )}
                {edited && (
                  <Button disabled={busy} onClick={() => setDraft(open.text ?? '')}>
                    {t('放弃修改', 'Discard edits')}
                  </Button>
                )}
                <ActionButton
                  action="code-file-open-external"
                  label={t('用本地软件打开', 'Open in a local app')}
                  disabled={busy || disabled}
                  run={() =>
                    invoke('workspace_open', { name, path: open.path, app: editor || null }).catch(
                      (e) => setError(t(...workspaceErrorText(String(e)))),
                    )
                  }
                >
                  {t('用本地软件打开', 'Open in a local app')}
                </ActionButton>
                <ActionButton
                  action="code-file-replace"
                  label={t('用本机文件替换…', 'Replace from this computer…')}
                  kind="commit"
                  disabled={busy || disabled || needMessage}
                  run={() => replaceWithFile(open.path)}
                >
                  {t('用本机文件替换…', 'Replace from this computer…')}
                </ActionButton>
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
