import { useEffect, useState, useRef } from 'react';
import { invokeJson, invoke } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import CodeFiles from './CodeFiles';
/** 选来打开源码的本地程序（VS Code、记事本……）；空着就用系统默认程序。每台机器各记各的。 */
const EDITOR_KEY = 'colm.externalEditor';
function storedEditor(): string {
  try {
    return globalThis.localStorage?.getItem(EDITOR_KEY) ?? '';
  } catch {
    return '';
  }
}
function storeEditor(app: string) {
  try {
    if (app) globalThis.localStorage?.setItem(EDITOR_KEY, app);
    else globalThis.localStorage?.removeItem(EDITOR_KEY);
  } catch {
    /* 存不下就只在这次会话里用 */
  }
}
type Snapshot = { applied: { short?: string; files: string[] } | null };
type Status = {
  workspace?: { base_commit?: string };
  changed_files?: { path: string; added?: number; removed?: number }[];
  commits?: { hash?: string; commit?: string; subject?: string }[];
  [key: string]: unknown;
};
export default function CodeWorkspace({
  name,
  onChanged,
  onPendingChange,
  disabled = false,
}: {
  name: string | null;
  onChanged: () => Promise<void>;
  onPendingChange?: (pending: boolean) => void;
  disabled?: boolean;
}) {
  const { state } = useApp(),
    t = (a: string, b: string) => (state.language === 'en' ? b : a);
  const [status, setStatus] = useState<Status | null>(null),
    [output, setOutput] = useState<unknown>(null),
    [error, setError] = useState(''),
    [busy, setBusy] = useState(false),
    [newName, setNewName] = useState(''),
    [source, setSource] = useState(''),
    [message, setMessage] = useState(''),
    [out, setOut] = useState(''),
    [commit, setCommit] = useState(''),
    [action, setAction] = useState('test-drift'),
    [preset, setPreset] = useState('default'),
    [pkg, setPkg] = useState('colm-core'),
    [editor, setEditor] = useState(storedEditor),
    [snapshotNote, setSnapshotNote] = useState('');
  const lock = useRef(false),
    epoch = useRef({ active: false });
  useEffect(() => {
    const token = { active: true };
    epoch.current = token;
    setStatus(null);
    setBusy(false);
    if (name)
      void invokeJson<Status>('workspace_status', { name })
        .then((s) => {
          if (token.active && token === epoch.current) setStatus(s);
        })
        .catch((e) => {
          if (token.active && token === epoch.current) setError(String(e));
        });
    return () => {
      token.active = false;
    };
  }, [name]);
  // 用本地软件改过的文件：打开这个版本时、窗口回到前台时检查一次，有改动就记成一次提交，差异列表、
  // 编译与冻结随之看得到它们。不用用户记得点"保存"。
  useEffect(() => {
    if (!name || disabled) return;
    let live = true;
    const check = () => {
      if (!live || lock.current) return;
      void invokeJson<Snapshot>('workspace_snapshot', { name })
        .then(async (result) => {
          if (!live || !result.applied) return;
          const files = result.applied.files;
          setSnapshotNote(
            t(
              `已记下在本地软件里改的 ${files.length} 个文件：${files.slice(0, 5).join('、')}${files.length > 5 ? ' 等' : ''}`,
              `Recorded ${files.length} file${files.length === 1 ? '' : 's'} changed in a local app: ${files.slice(0, 5).join(', ')}${files.length > 5 ? ' …' : ''}`,
            ),
          );
          setStatus(await invokeJson<Status>('workspace_status', { name }));
          await onChanged();
        })
        .catch((e) => {
          if (live) setError(String(e));
        });
    };
    check();
    globalThis.addEventListener?.('focus', check);
    return () => {
      live = false;
      globalThis.removeEventListener?.('focus', check);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [name, disabled]);
  async function chooseEditor() {
    const app = await invoke<string | null>('pick_file', { key: 'external-editor' });
    if (!app) return;
    storeEditor(app);
    setEditor(app);
  }
  async function execute(fn: () => Promise<unknown>, refresh = true) {
    if (lock.current || disabled) return;
    lock.current = true;
    onPendingChange?.(true);
    setBusy(true);
    setError('');
    const token = epoch.current;
    try {
      const result = await fn();
      if (token.active && token === epoch.current) {
        setOutput(result);
        if (name && refresh) setStatus(await invokeJson<Status>('workspace_status', { name }));
      }
    } catch (e) {
      if (token.active && token === epoch.current) setError(String(e));
      throw e;
    } finally {
      lock.current = false;
      onPendingChange?.(false);
      if (token.active && token === epoch.current) setBusy(false);
    }
  }
  const run = (fn: () => Promise<unknown>, refresh = true) =>
    void execute(fn, refresh).catch(() => {});
  async function patch() {
    if (!name) return;
    const file = await invoke<string | null>('pick_file', {
      key: 'workspace-patch',
      filter: 'patch,diff',
    });
    if (!file) return;
    const value = await invokeJson('workspace_import_patch', { name, file, message });
    await onChanged();
    return value;
  }
  const guideFields: UiField[] = [
    {
      id: 'workspace-message',
      label: t('修改说明', 'Change message'),
      value: message,
      type: 'text',
      set: setMessage,
    },
  ];
  if (name) {
    guideFields.push(
      {
        id: 'workspace-check-action',
        label: t('检查', 'Check'),
        value: action,
        type: 'choice',
        options: ['test-drift', 'test-cargo', 'build-engine', 'build-kernel', 'parity'].map(
          (v) => ({ value: v, label: v }),
        ),
        set: setAction,
      },
      {
        id: 'workspace-preset',
        label: t('编译配置', 'Build configuration'),
        value: preset,
        type: 'text',
        set: setPreset,
      },
      {
        id: 'workspace-package',
        label: t('测试包', 'Test package'),
        value: pkg,
        type: 'text',
        set: setPkg,
      },
      {
        id: 'workspace-export-path',
        label: t('导出补丁路径', 'Export patch path'),
        value: out,
        type: 'text',
        set: setOut,
      },
    );
  } else {
    guideFields.push(
      {
        id: 'workspace-new-name',
        label: t('新工作区名称', 'New workspace name'),
        value: newName,
        type: 'text',
        set: setNewName,
      },
      {
        id: 'workspace-source',
        label: t('源码仓库', 'Source repository'),
        value: source,
        type: 'text',
        set: setSource,
      },
    );
  }
  usePageContract({
    title: t('代码工作区', 'Code workspace'),
    step: 'versions',
    status: error,
    fields: guideFields.map((f) => ({ ...f, disabled: busy || disabled })),
    // 按钮自己登记（第 737 轮）。导入补丁、执行检查是 C 级写操作，按 commit 登记；导出补丁、
    // 恢复文件和删除工作区属于 D 级，只在界面上做。
    actions: [],
  });

  return (
    <fieldset disabled={disabled || busy} style={{ border: 0, padding: 0 }}>
      <Card title={t('代码', 'Code')}>
        {error && <Notice tone="error">{error}</Notice>}
        <p>
          {name ??
            t(
              '官方代码只读；创建独立工作区后可作为新版本来源',
              'Official code is read-only. Create a workspace to derive a version.',
            )}
        </p>
        <Field label={t('修改说明', 'Change message')}>
          <Input value={message} disabled={busy} onChange={(e) => setMessage(e.target.value)} />
        </Field>
        {!name ? (
          <>
            <Field label={t('新工作区名称', 'New workspace name')}>
              <Input value={newName} onChange={(e) => setNewName(e.target.value)} />
            </Field>
            <Field
              label={t(
                '源码仓库（留空使用内置来源）',
                'Source repository (blank uses bundled source)',
              )}
            >
              <Input value={source} onChange={(e) => setSource(e.target.value)} />
            </Field>
            <ActionButton
              action="workspace-create"
              label={t('创建工作区', 'Create workspace')}
              kind="commit"
              disabled={disabled || busy || !newName}
              run={() =>
                run(() =>
                  invokeJson('workspace_create', {
                    name: newName,
                    source: source || null,
                    note: message || null,
                  }),
                )
              }
            >
              {t('创建工作区', 'Create workspace')}
            </ActionButton>
          </>
        ) : (
          <>
            <CodeFiles
              name={name}
              editor={editor}
              message={message}
              en={state.language === 'en'}
              disabled={busy || disabled}
              onChanged={async () => {
                setStatus(await invokeJson<Status>('workspace_status', { name }));
                await onChanged();
              }}
            />
            <div className="button-row">
              <ActionButton
                action="workspace-open-folder"
                label={t('用本地软件打开源码文件夹', 'Open the source folder in a local app')}
                disabled={disabled || busy}
                run={() =>
                  run(
                    () => invoke('workspace_open', { name, path: null, app: editor || null }),
                    false,
                  )
                }
              />
              <ActionButton
                action="workspace-choose-editor"
                label={t('选择用哪个程序打开…', 'Choose the app to open with…')}
                disabled={disabled || busy}
                run={chooseEditor}
              />
              {editor && (
                <Button
                  disabled={disabled || busy}
                  onClick={() => {
                    storeEditor('');
                    setEditor('');
                  }}
                >
                  {t('改用系统默认程序', 'Use the system default')}
                </Button>
              )}
              <ActionButton
                action="workspace-snapshot"
                label={t('检查本地软件的修改', 'Check for changes made in local apps')}
                disabled={disabled || busy}
                run={() =>
                  run(async () => {
                    const result = await invokeJson<Snapshot>('workspace_snapshot', { name });
                    setSnapshotNote(
                      result.applied
                        ? t(
                            `已记下 ${result.applied.files.length} 个文件的修改`,
                            `Recorded changes to ${result.applied.files.length} file${result.applied.files.length === 1 ? '' : 's'}`,
                          )
                        : t('没有新的修改', 'No new changes'),
                    );
                    if (result.applied) await onChanged();
                    return result;
                  })
                }
              />
            </div>
            <p className="muted">
              {editor
                ? t(`用 ${editor} 打开。`, `Opens with ${editor}.`)
                : t('用系统默认程序打开。', 'Opens with the system default app.')}
              {t(
                '在 VS Code、记事本等任何软件里改源码，回到这里时会自动记下和原版的差异；被 .gitignore 忽略的编译产物和编辑器自己的设置文件不算修改，黄金参考（oracle/golden）不能改。',
                ' Edit the source in VS Code, Notepad or any app; when you come back the differences from the official source are recorded automatically. Build outputs ignored by .gitignore and editor settings do not count, and the golden references (oracle/golden) cannot be changed.',
              )}
            </p>
            {snapshotNote && <Notice tone="success">{snapshotNote}</Notice>}
            <ActionButton
              action="workspace-patch"
              label={t('导入补丁', 'Import patch')}
              kind="commit"
              disabled={disabled || busy || !message}
              run={() => run(patch)}
            >
              {t('导入补丁', 'Import patch')}
            </ActionButton>
            {status?.changed_files?.map((file) => (
              <div key={file.path}>
                <code>
                  {file.path} +{file.added ?? 0} −{file.removed ?? 0}
                </code>
                <ActionButton
                  action={`workspace-diff-${file.path}`}
                  label={`${t('差异', 'Diff')} ${file.path}`}
                  disabled={disabled || busy}
                  run={() => run(() => invokeJson('workspace_diff', { name, path: file.path }))}
                >
                  {t('差异', 'Diff')}
                </ActionButton>
                <Button
                  disabled={busy || !message}
                  variant="danger"
                  onClick={async () => {
                    if (
                      await askConfirm(t(`恢复 ${file.path}？`, `Revert ${file.path}?`), {
                        danger: true,
                      })
                    )
                      run(async () => {
                        const value = await invokeJson('workspace_revert_file', {
                          name,
                          path: file.path,
                          message,
                        });
                        await onChanged();
                        return value;
                      });
                  }}
                >
                  {t('恢复文件', 'Revert file')}
                </Button>
              </div>
            ))}
            <Field label={t('检查', 'Check')}>
              <Select value={action} onChange={(e) => setAction(e.target.value)}>
                {['test-drift', 'test-cargo', 'build-engine', 'build-kernel', 'parity'].map((a) => (
                  <option key={a}>{a}</option>
                ))}
              </Select>
            </Field>
            <Field label={t('编译配置', 'Build configuration')}>
              <Input value={preset} onChange={(e) => setPreset(e.target.value)} />
            </Field>
            <Field label={t('测试包', 'Test package')}>
              <Input value={pkg} onChange={(e) => setPkg(e.target.value)} />
            </Field>
            <ActionButton
              action="workspace-check"
              label={t('执行检查', 'Run check')}
              kind="commit"
              disabled={disabled || busy}
              run={() =>
                run(() =>
                  invokeJson('workspace_check', {
                    name,
                    action,
                    preset,
                    package: pkg,
                    case: state.selectedCase?.dir ?? null,
                  }),
                )
              }
            >
              {t('执行检查', 'Run check')}
            </ActionButton>
            <Field label={t('导出补丁路径', 'Export patch path')}>
              <Input value={out} onChange={(e) => setOut(e.target.value)} />
            </Field>
            <Button
              disabled={busy || !out}
              onClick={() => run(() => invokeJson('workspace_export', { name, out }))}
            >
              {t('导出改动', 'Export changes')}
            </Button>
            <Field label={t('恢复到提交', 'Revert to commit')}>
              <Input value={commit} onChange={(e) => setCommit(e.target.value)} />
            </Field>
            <Button
              variant="danger"
              disabled={busy || !commit}
              onClick={async () => {
                if (
                  await askConfirm(
                    t('丢弃该提交后的代码改动？', 'Discard code changes after this commit?'),
                    { danger: true },
                  )
                )
                  run(async () => {
                    const v = await invokeJson('workspace_revert', { name, commit });
                    await onChanged();
                    return v;
                  });
              }}
            >
              {t('恢复工作区', 'Revert workspace')}
            </Button>
            <Button
              variant="danger"
              disabled={busy}
              onClick={async () => {
                if (
                  await askConfirm(t(`删除工作区 ${name}？`, `Delete workspace ${name}?`), {
                    confirmLabel: t('删除', 'Delete'),
                    danger: true,
                  })
                )
                  run(async () => {
                    const result = await invokeJson('workspace_delete', { name });
                    setStatus(null);
                    return result;
                  }, false);
              }}
            >
              {t('删除工作区', 'Delete workspace')}
            </Button>
            <pre>{JSON.stringify(status, null, 2)}</pre>
          </>
        )}
        {output != null && <pre>{JSON.stringify(output, null, 2)}</pre>}
      </Card>
    </fieldset>
  );
}
