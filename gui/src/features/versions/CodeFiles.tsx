import { useEffect, useMemo, useRef, useState } from 'react';
import { invoke, invokeJson } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { Button, Field, Input, Notice } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import { isContentConflict, workspaceErrorText } from './workspace-errors';
import type { SourceEditState } from './source-edits';

type Entry = { name: string; path: string; kind: 'file' | 'dir' | 'link'; size: number };
/** `workspace_file` 带内容哈希；冻结版本只读，`model_code_file` 没有哈希（也不需要保存基线）。 */
type FileText = { path: string; size: number; text: string | null; hash?: string | null };
/** 保存时后端报告内容基线过期：磁盘上的版本已经和打开时不一样了。 */
type Conflict = {
  path: string;
  /** 磁盘上现在还有这个文件吗（外部删除/换成目录时为 false）。 */
  exists: boolean;
  text: string | null;
  hash: string | null;
  size: number | null;
};

const parentOf = (path: string) => path.split('/').slice(0, -1).join('/');
const joinPath = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);
const sizeLabel = (bytes: number) =>
  bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : bytes >= 1024
      ? `${(bytes / 1024).toFixed(1)} KB`
      : `${bytes} B`;
const byteLength = (text: string) => new TextEncoder().encode(text).length;

/**
 * 修改版的代码文件：浏览源码目录，文本文件直接在这里编辑保存；任何文件或整个目录都能用本机的
 * 文件、文件夹替换（可以是二进制，比如查找表），也能新增和删除。每次改动都是工作区里的一次提交。
 * 给了 `source`（一个冻结版本）就只读浏览它冻结时的代码，不能改。
 *
 * 这个组件是源码缓冲区的唯一持有者。未保存状态通过 `onStateChange` 交给 CodeWorkspace / VersionsPage，
 * 让离开页面、切工作区、关窗、编译、冻结都能统一处理（GUI-02）。保存/新建/替换时把工作区名、
 * 文件路径和上下文代次钉住：迟到响应属于旧对象就不许回写（GUI 审查的异步身份约定）。
 */
export default function CodeFiles({
  name,
  message,
  en,
  disabled,
  onChanged,
  onPendingChange,
  onStateChange,
  externalChangeToken = 0,
  source,
  editor = '',
}: {
  name: string;
  message: string;
  en: boolean;
  disabled: boolean;
  onChanged: () => Promise<void>;
  /** 写操作在飞：父工作台据此禁止冻结、编译、切换对象。 */
  onPendingChange?: (pending: boolean) => void;
  /** 把未保存/保存中状态交给父组件（离开与冻结的保护用它）。 */
  onStateChange?: (state: SourceEditState | null) => void;
  /** 外部改动被快照记成提交后 +1：干净缓冲区自动重载，脏缓冲区显示冲突（GUI-03）。 */
  externalChangeToken?: number;
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
  const [conflict, setConflict] = useState<Conflict | null>(null);
  const [showTheirs, setShowTheirs] = useState(false);
  const [newPath, setNewPath] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [busy, setBusy] = useState(false);
  const token = useRef(0);
  const opening = useRef(0);
  const recheckEpoch = useRef(0);
  const busyRef = useRef(false);
  const alive = useRef(true);
  const opEpoch = useRef(0);
  // 上下文身份（工作区 + 冻结版本）：换了对象之后，任何在飞的读取/保存响应都不许回写。
  const contextRef = useRef('');
  contextRef.current = `${name}\u0000${source?.id ?? ''}\u0000${source?.revision ?? ''}`;
  // 异步返回后要用"现在"的说明、回调，而不是发起时那一帧闭包里的。
  const latest = useRef({ message, onChanged });
  latest.current = { message, onChanged };
  const openRef = useRef(open);
  openRef.current = open;
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const conflictRef = useRef(conflict);
  conflictRef.current = conflict;
  const edited = open?.text != null && draft !== open.text;
  const editedRef = useRef(edited);
  editedRef.current = edited;
  const readFile = (path: string) =>
    source
      ? invokeJson<FileText>('model_code_file', { ...source, path })
      : invokeJson<FileText>('workspace_file', { name, path });

  async function load(next = dir) {
    const mine = ++token.current;
    const context = contextRef.current;
    const value = source
      ? await invokeJson<{ entries: Entry[] }>('model_code_files', { ...source, dir: next })
      : await invokeJson<{ entries: Entry[] }>('workspace_files', { name, dir: next });
    // 连点目录后发先至：只认最后一次；换了工作区则整次作废。
    if (mine === token.current && context === contextRef.current && alive.current) {
      setDir(next);
      setEntries(value.entries);
    }
  }
  /** 让在飞的读取/保存/核对全部作废（切换上下文、卸载时用）。 */
  function invalidatePending() {
    opening.current++;
    opEpoch.current++;
    recheckEpoch.current++;
    busyRef.current = false;
  }
  // 进入/切换工作区或冻结版本：清空缓冲区、让在飞操作全部失效，再读这个上下文的第一层目录。
  useEffect(() => {
    invalidatePending();
    setBusy(false);
    setOpen(null);
    setDraft('');
    setConflict(null);
    setShowTheirs(false);
    setNewPath('');
    setError('');
    setNotice('');
    const context = contextRef.current;
    void load('').catch((e) => {
      if (context === contextRef.current && alive.current)
        setError(t(...workspaceErrorText(String(e))));
    });
    return invalidatePending;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, source?.id, source?.revision]);
  useEffect(
    () => () => {
      alive.current = false;
    },
    [],
  );
  /** 成功返回 true；被取消/失败/上下文已变返回 false（三选一对话框用这个结果决定是否继续）。 */
  async function act(
    work: (stillCurrent: () => boolean) => Promise<unknown>,
    done: string | (() => string),
  ): Promise<boolean> {
    if (busyRef.current || disabled) return false;
    // save/create/replace 在飞时锁住父工作台：这段时间不许冻结、编译或切到另一个对象。
    busyRef.current = true;
    setBusy(true);
    onPendingChange?.(true);
    setError('');
    setNotice('');
    const epoch = ++opEpoch.current;
    const context = contextRef.current;
    const stillCurrent = () =>
      alive.current && context === contextRef.current && epoch === opEpoch.current;
    try {
      // 选文件时点了取消：什么都没做，不提示成功。
      if ((await work(stillCurrent)) === false) return false;
      if (!stillCurrent()) return false;
      setNotice(typeof done === 'function' ? done() : done);
      await load();
      if (!stillCurrent()) return false;
      await latest.current.onChanged();
      return true;
    } catch (e) {
      if (stillCurrent()) setError(t(...workspaceErrorText(String(e))));
      return false;
    } finally {
      busyRef.current = false;
      setBusy(false);
      onPendingChange?.(false);
    }
  }
  /** 保存失败进冲突界面时，读磁盘上的当前版本给用户比较；读不到多半是外部删了。 */
  async function showConflict(path: string, stillCurrent: () => boolean) {
    try {
      const theirs = await readFile(path);
      if (!stillCurrent()) return;
      setConflict({
        path,
        exists: true,
        text: theirs.text,
        hash: theirs.hash ?? null,
        size: theirs.size,
      });
    } catch {
      if (!stillCurrent()) return;
      setConflict({ path, exists: false, text: null, hash: null, size: null });
    }
  }
  async function openFile(path: string, force = false) {
    if (
      !force &&
      editedRef.current &&
      !(await askConfirm(t('当前文件的修改还没保存，放弃吗？', 'Discard unsaved edits?'), {
        danger: true,
      }))
    )
      return;
    setError('');
    setConflict(null);
    setShowTheirs(false);
    // 连点两个文件时后发先至：只认最后一次点的那个，否则编辑框里是 B、保存却写进 A（GUI 审查 B-07）。
    // 换了工作区/冻结版本同理：旧响应不许把旧文件塞进新上下文。
    const mine = ++opening.current;
    const context = contextRef.current;
    try {
      const file = await readFile(path);
      if (mine !== opening.current || context !== contextRef.current || !alive.current) return;
      setOpen(file);
      setDraft(file.text ?? '');
    } catch (e) {
      if (mine === opening.current && context === contextRef.current && alive.current)
        setError(t(...workspaceErrorText(String(e))));
    }
  }
  const needMessage = !readOnly && !message.trim();
  const save = () =>
    act(
      async (stillCurrent) => {
        const file = openRef.current;
        if (!file || file.text == null) return false;
        const path = file.path;
        const textNow = draftRef.current;
        try {
          // 带打开时的内容哈希：写入端先比对磁盘，不一致整次拒绝（GUI-03）。返回值里的新哈希就是
          // 下一次保存的基线——不靠保存后重读，避免中间被外部写入时拿错基线。
          const written = await invokeJson<{ hash?: string | null }>('workspace_write_file', {
            name,
            path,
            text: textNow,
            message: latest.current.message,
            expectedHash: file.hash ?? null,
          });
          if (!stillCurrent() || openRef.current?.path !== path) return false;
          const hash = typeof written?.hash === 'string' ? written.hash : null;
          setOpen({ ...file, text: textNow, size: byteLength(textNow), hash });
          setDraft(textNow);
          setConflict(null);
          return true;
        } catch (e) {
          if (!stillCurrent()) return false;
          if (isContentConflict(String(e))) {
            await showConflict(path, stillCurrent);
            return false;
          }
          throw e;
        }
      },
      t(`已保存 ${open?.path}`, `Saved ${open?.path}`),
    );
  /** 明确用缓冲区里的文本覆盖磁盘（或重建被外部删掉的文件）。用户看过冲突内容后才会点。 */
  const overwriteConflict = () => {
    const current = conflict;
    if (!current) return;
    void act(
      async (stillCurrent) => {
        const textNow = draftRef.current;
        const written = await invokeJson<{ hash?: string | null }>('workspace_write_file', {
          name,
          path: current.path,
          text: textNow,
          message: latest.current.message,
          // 不带基线才是"明确覆盖"：后端把它当新建/覆盖，但这是用户看到冲突后自己选的。
        });
        if (!stillCurrent()) return false;
        const hash = typeof written?.hash === 'string' ? written.hash : null;
        setOpen({ path: current.path, size: byteLength(textNow), text: textNow, hash });
        setDraft(textNow);
        setConflict(null);
        setShowTheirs(false);
        return true;
      },
      t(`已用你的文本覆盖 ${current.path}`, `Overwrote ${current.path} with your text`),
    );
  };
  const reloadConflict = () => {
    const current = conflict;
    if (!current) return;
    if (current.exists) {
      setOpen({
        path: current.path,
        size: current.size ?? 0,
        text: current.text,
        hash: current.hash,
      });
      setDraft(current.text ?? '');
    } else {
      setOpen(null);
      setDraft('');
    }
    setConflict(null);
    setShowTheirs(false);
    setNotice(t('已重新读取磁盘上的版本', 'Reloaded the version on disk'));
  };
  const discardEdits = () => {
    // 冲突时"放弃我的修改"要让缓冲区回到磁盘上的版本，而不是回到打开时的旧基线。
    if (conflictRef.current) {
      reloadConflict();
      return;
    }
    setDraft(openRef.current?.text ?? '');
  };
  const copyDraft = () => {
    const clipboard = globalThis.navigator?.clipboard;
    if (!clipboard) {
      setError(
        t(
          '无法访问剪贴板，请手动选择文本复制',
          'Clipboard is not available; copy the text manually',
        ),
      );
      return;
    }
    void clipboard.writeText(draftRef.current).then(
      () => setNotice(t('已复制当前文本', 'Copied the current text')),
      () =>
        setError(
          t(
            '无法访问剪贴板，请手动选择文本复制',
            'Clipboard is not available; copy the text manually',
          ),
        ),
    );
  };
  async function recheckOpen() {
    const current = openRef.current;
    // 只对真正打开了的文本文件比；已在冲突界面时等用户处理，不覆盖任何一边。
    if (!current || current.text == null || conflictRef.current) return;
    const mine = ++recheckEpoch.current;
    const context = contextRef.current;
    const path = current.path;
    let theirs: FileText;
    try {
      theirs = await readFile(path);
    } catch {
      // 外部删除会在保存时由写入端报冲突；这里不主动打断。
      return;
    }
    if (
      !alive.current ||
      context !== contextRef.current ||
      mine !== recheckEpoch.current ||
      openRef.current?.path !== path
    )
      return;
    if ((theirs.hash ?? null) === (current.hash ?? null)) return;
    if (draftRef.current === current.text) {
      // 干净缓冲区：外部提交变化后自动换成新内容，不用重挂载（重挂载会连未保存文本一起销毁）。
      setOpen(theirs);
      setDraft(theirs.text ?? '');
      setConflict(null);
    } else {
      // 两边都改过：保留用户文本，只显示冲突，等用户比较/复制/重载/明确覆盖。
      setConflict({
        path,
        exists: true,
        text: theirs.text,
        hash: theirs.hash ?? null,
        size: theirs.size,
      });
    }
  }
  // 外部软件里的改动被 CodeWorkspace 的快照记成提交后（externalChangeToken 变化），核对一次当前文件。
  // 新 effect 放最后：测试按出现顺序取 effect，前面的编号保持稳定。
  useEffect(() => {
    if (externalChangeToken <= 0) return;
    void recheckOpen();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [externalChangeToken]);
  const saveRef = useRef(save);
  saveRef.current = save;
  const discardRef = useRef(discardEdits);
  discardRef.current = discardEdits;
  // 只读视图（冻结版本）不注册：浏览版本不该产生"未保存"提示，也没有保存入口。
  const sourceState = useMemo<SourceEditState>(
    () => ({
      get dirty() {
        return editedRef.current;
      },
      get busy() {
        return busyRef.current;
      },
      get path() {
        return openRef.current?.path ?? null;
      },
      get canSave() {
        const file = openRef.current;
        return !!file && file.text != null && latest.current.message.trim() !== '';
      },
      save: () => saveRef.current(),
      discard: () => discardRef.current(),
    }),
    [],
  );
  const onStateChangeRef = useRef(onStateChange);
  onStateChangeRef.current = onStateChange;
  useEffect(() => {
    if (readOnly) return;
    const handler = onStateChangeRef.current;
    handler?.(sourceState);
    return () => handler?.(null);
  }, [readOnly, sourceState]);

  async function addFileFromComputer() {
    const from = await invoke<string | null>('pick_file', { key: 'workspace-put' });
    if (!from) return;
    const fileName = from.split(/[\\/]/).at(-1)!;
    await act(
      async (stillCurrent) => {
        // createOnly：同名文件由后端拒绝（OpenOptions::create_new），不能以"加入"的名义覆盖它。
        await invokeJson('workspace_put', {
          name,
          path: joinPath(dir, fileName),
          from,
          folder: false,
          createOnly: true,
          message: latest.current.message,
        });
        return stillCurrent();
      },
      t(`已加入 ${fileName}`, `Added ${fileName}`),
    );
  }
  const createFile = () => {
    const path = joinPath(dir, newPath.trim());
    return act(
      async (stillCurrent) => {
        // 新建走独立的 workspace_create_file：后端用 OpenOptions::create_new 原子拒绝重名（GUI-01）。
        const written = await invokeJson<{ hash?: string | null }>('workspace_create_file', {
          name,
          path,
          text: '',
          message: latest.current.message,
        });
        if (!stillCurrent()) return false;
        setNewPath('');
        const hash = typeof written?.hash === 'string' ? written.hash : null;
        setOpen({ path, size: 0, text: '', hash });
        setDraft('');
        setConflict(null);
        return true;
      },
      t(`已新建 ${path}`, `Created ${path}`),
    );
  };
  const replaceWithFile = (path: string) =>
    act(
      async (stillCurrent) => {
        if (openRef.current?.path === path && editedRef.current) {
          if (
            !(await askConfirm(
              t(
                '替换会用本机文件整个换掉这个文件，当前未保存的修改会丢失。继续吗？',
                'Replacing overwrites this file with the local one; unsaved edits are lost. Continue?',
              ),
              { danger: true },
            ))
          )
            return false;
        }
        const from = await invoke<string | null>('pick_file', { key: 'workspace-put' });
        if (!from) return false;
        await invokeJson('workspace_put', {
          name,
          path,
          from,
          folder: false,
          createOnly: false,
          message: latest.current.message,
        });
        if (!stillCurrent()) return false;
        // 替换是明确的整文件换掉：缓冲区跟着磁盘重载，别让 user 的旧文本在下一次保存时又成冲突。
        if (openRef.current?.path === path) await openFile(path, true);
        return true;
      },
      t(`已替换 ${path}`, `Replaced ${path}`),
    );
  const replaceWithFolder = (path: string) =>
    act(
      async (stillCurrent) => {
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
        await invokeJson('workspace_put', {
          name,
          path,
          from,
          folder: true,
          createOnly: false,
          message: latest.current.message,
        });
        if (!stillCurrent()) return false;
        setOpen(null);
        setDraft('');
        setConflict(null);
        return true;
      },
      t(`已替换目录 ${path}`, `Replaced folder ${path}`),
    );
  const remove = (path: string) =>
    act(
      async (stillCurrent) => {
        if (
          !(await askConfirm(
            t(`删除 ${path}？（改动可以撤回）`, `Delete ${path}? (can be reverted)`),
            {
              danger: true,
            },
          ))
        )
          return false;
        await invokeJson('workspace_remove', {
          name,
          path,
          message: latest.current.message,
        });
        if (!stillCurrent()) return false;
        if (openRef.current?.path.startsWith(path)) {
          setOpen(null);
          setDraft('');
          setConflict(null);
        }
        return true;
      },
      t(`已删除 ${path}`, `Deleted ${path}`),
    );
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
              onClick={() =>
                void load(parentOf(dir)).catch((e) => {
                  if (alive.current) setError(t(...workspaceErrorText(String(e))));
                })
              }
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
                      (e) => {
                        if (alive.current) setError(t(...workspaceErrorText(String(e))));
                      },
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
              run={() => void addFileFromComputer()}
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
                {conflict && <span className="badge">{t('内容冲突', 'Content conflict')}</span>}
              </div>
              {conflict ? (
                <Notice tone="error">
                  <p>
                    {conflict.exists
                      ? t(
                          '这个文件在软件外被更新了，你的保存没有覆盖它。磁盘上的版本与你的文本都还在；比较后可以重载、复制或明确覆盖。',
                          'This file changed outside the app; your save did not overwrite it. Both versions are still here: compare them, then reload, copy your text, or overwrite explicitly.',
                        )
                      : t(
                          '这个文件在软件外被删除或替换了，你的保存没有重新创建它。可以用你的文本明确重建，或重新加载。',
                          'This file was deleted or replaced outside the app; your save did not recreate it. Explicitly recreate it with your text, or reload.',
                        )}
                  </p>
                  <div className="button-row">
                    <Button onClick={() => setShowTheirs((shown) => !shown)}>
                      {showTheirs
                        ? t('收起磁盘版本', 'Hide the version on disk')
                        : t('比较磁盘上的版本', 'Compare with the version on disk')}
                    </Button>
                    <Button onClick={reloadConflict}>
                      {t(
                        '重新加载磁盘版本（放弃我的文本）',
                        'Reload the disk version (discard my text)',
                      )}
                    </Button>
                    <Button variant="danger" onClick={overwriteConflict}>
                      {conflict.exists
                        ? t('用我的文本覆盖磁盘', 'Overwrite the file with my text')
                        : t('用我的文本重建文件', 'Recreate the file with my text')}
                    </Button>
                    <Button onClick={copyDraft}>{t('复制我的文本', 'Copy my text')}</Button>
                  </div>
                  {showTheirs && (
                    <textarea
                      className="code-files-text"
                      readOnly
                      spellCheck={false}
                      value={
                        conflict.exists
                          ? (conflict.text ??
                            t(
                              '（磁盘上的版本是二进制文件，无法按文本比较）',
                              '(the version on disk is binary; it cannot be compared as text)',
                            ))
                          : t('（磁盘上已经没有这个文件）', '(the file no longer exists on disk)')
                      }
                    />
                  )}
                </Notice>
              ) : null}
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
                    disabled={busy || disabled || needMessage || !edited || !!conflict}
                    run={save}
                  >
                    {t('保存', 'Save')}
                  </ActionButton>
                )}
                {edited && !conflict && (
                  <Button disabled={busy} onClick={discardEdits}>
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
              .catch((err) => {
                if (alive.current) setError(t(...workspaceErrorText(String(err))));
              });
          }}
        />
      </Field>
    </section>
  );
}
