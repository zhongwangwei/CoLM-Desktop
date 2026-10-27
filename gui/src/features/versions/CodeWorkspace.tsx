import { useEffect, useState, useRef } from 'react';
import { invokeJson, invoke } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import { useApp } from '../../lib/app-state';
import { usePageContract, type UiField } from '../../lib/guide';
import { Button, Card, Field, Input, Notice, Select } from '../../components/ui';
import { ActionButton } from '../../components/ActionButton';
import CodeFiles from './CodeFiles';
import { workspaceErrorText } from './workspace-errors';
import type { SourceEditState } from './source-edits';
import { useSourceChoicePrompt } from './source-guard';

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
type Commit = {
  id?: string;
  hash?: string;
  commit?: string;
  short?: string;
  subject?: string;
  at?: number;
};
type Light = 'unknown' | 'pass' | 'fail' | 'stale' | 'not_needed';
type Status = {
  workspace?: {
    name?: string;
    base_commit?: string;
    origin?: string;
    note?: string;
    based_on?: string | null;
    root_commit?: string | null;
  };
  changed_files?: { path: string; added?: number; removed?: number }[];
  commits?: Commit[];
  dirty?: boolean;
  head?: string;
  parity_needed?: boolean;
  lights?: Partial<Record<'compile' | 'tests' | 'regression' | 'parity', Light>>;
  [key: string]: unknown;
};
type CheckOption = {
  value: string;
  zh: string;
  en: string;
  zhHint: string;
  enHint: string;
  needs?: 'preset' | 'package' | 'case';
};
const CHECKS: CheckOption[] = [
  {
    value: 'build-engine',
    zh: '编译 Rust 引擎',
    en: 'Build Rust engine',
    zhHint: '确认 crates/ 里的 Rust 代码能编译。',
    enHint: 'Check that the Rust engine under crates/ compiles.',
  },
  {
    value: 'build-kernel',
    zh: '编译 Fortran 内核',
    en: 'Build Fortran kernel',
    zhHint: '用所选编译配置编译 Fortran 内核；改 vendor/CoLM202X 时需要。',
    enHint:
      'Build the Fortran kernel with the selected configuration; needed after vendor/CoLM202X changes.',
    needs: 'preset',
  },
  {
    value: 'test-drift',
    zh: '生成表一致性测试',
    en: 'Generated-table consistency test',
    zhHint: '检查生成表、namelist 与源码快照是否逐字节一致。',
    enHint: 'Check that generated tables and namelists match the source snapshot byte for byte.',
  },
  {
    value: 'test-cargo',
    zh: 'Cargo 测试',
    en: 'Cargo tests',
    zhHint: '运行指定 Rust package 的测试。',
    enHint: 'Run the tests for one Rust package.',
    needs: 'package',
  },
  {
    value: 'parity',
    zh: 'Rust 与 Fortran 结果对比',
    en: 'Compare Rust and Fortran results',
    zhHint: '用同一算例分别跑两个引擎，逐位比较历史输出；需要先编译。',
    enHint:
      'Run the same case with both engines and compare their history output bit for bit; build first.',
    needs: 'case',
  },
];
const shortHash = (commit: Commit) =>
  commit.short || (commit.id || commit.hash || commit.commit || '').slice(0, 8);
const commitId = (commit: Commit) => commit.id || commit.hash || commit.commit || '';
const lightText = (light: Light | undefined, t: (zh: string, en: string) => string) => {
  switch (light) {
    case 'pass':
      return t('通过', 'passed');
    case 'fail':
      return t('失败', 'failed');
    case 'stale':
      return t('已过期，需要重测', 'stale, rerun');
    case 'not_needed':
      return t('不需要', 'not needed');
    default:
      return t('未检查', 'not checked');
  }
};

export default function CodeWorkspace({
  name,
  onChanged,
  onPendingChange,
  onSourceState,
  disabled = false,
}: {
  name: string | null;
  onChanged: () => Promise<void>;
  onPendingChange?: (pending: boolean) => void;
  /** 源码缓冲区的未保存/保存中状态上抛给版本页：离开、冻结时统一处理（GUI-02）。 */
  onSourceState?: (state: SourceEditState | null) => void;
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
    [action, setAction] = useState('build-engine'),
    [preset, setPreset] = useState('default'),
    [pkg, setPkg] = useState('colm-core'),
    [caseDir, setCaseDir] = useState(state.selectedCase?.entry.dir ?? ''),
    [editor, setEditor] = useState(storedEditor),
    [snapshotNote, setSnapshotNote] = useState('');
  const lock = useRef(false),
    epoch = useRef({ active: false });
  // 源码缓冲区状态：CodeFiles 是唯一持有者，这里只存只读视图（getter 读到的永远是最新值）并转发给
  // 版本页。编译/检查必须用磁盘上的代码，所以点检查前要先处理未保存缓冲区（GUI-02）。
  const sourceRef = useRef<SourceEditState | null>(null);
  const [sourceBusy, setSourceBusy] = useState(false);
  // 快照把"外部软件改过的文件"记成提交后 +1：让 CodeFiles 核对当前打开的文件（GUI-03）。
  const [externalChangeToken, setExternalChangeToken] = useState(0);
  const onPendingRef = useRef(onPendingChange),
    onSourceStateRef = useRef(onSourceState);
  onPendingRef.current = onPendingChange;
  onSourceStateRef.current = onSourceState;
  const sourceChoice = useSourceChoicePrompt(state.language === 'en');
  const handleSourceState = (next: SourceEditState | null) => {
    sourceRef.current = next;
    onSourceStateRef.current?.(next);
  };
  const handleSourcePending = (pending: boolean) => {
    setSourceBusy(pending);
    onPendingRef.current?.(pending);
  };
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
              `已记下在外部软件里改的 ${files.length} 个文件：${files.slice(0, 5).join('、')}${files.length > 5 ? ' 等' : ''}`,
              `Recorded ${files.length} file${files.length === 1 ? '' : 's'} changed in an external app: ${files.slice(0, 5).join(', ')}${files.length > 5 ? ' …' : ''}`,
            ),
          );
          setStatus(await invokeJson<Status>('workspace_status', { name }));
          await onChanged();
          // 外部改动已记成提交：让打开中的文件核对内容哈希。干净缓冲区自动换成新内容，脏缓冲区
          // 显示冲突并保留用户文本（GUI-03）。
          setExternalChangeToken((token) => token + 1);
        })
        .catch((e) => {
          if (live) setError(t(...workspaceErrorText(String(e))));
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
  useEffect(
    () => () => {
      // 组件卸载（切草稿/离开页面）时把上抛状态清掉，免得版本页拿着过期缓冲区做冻结保护。
      onSourceStateRef.current?.(null);
    },
    [],
  );
  async function chooseEditor() {
    const app = await invoke<string | null>('pick_file', { key: 'external-editor' });
    if (!app) return;
    storeEditor(app);
    setEditor(app);
  }
  async function execute(fn: () => Promise<unknown>, refresh = true) {
    // 文件写操作在飞时不许再起检查/编译：两边都改同一个工作区，提交会打架。
    if (lock.current || disabled || sourceRef.current?.busy) return;
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
      if (token.active && token === epoch.current) setError(t(...workspaceErrorText(String(e))));
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
  const selectedCheck = CHECKS.find((check) => check.value === action) ?? CHECKS[0];
  const workspace = status?.workspace;
  const commits = status?.commits ?? [];
  const changedFiles = status?.changed_files ?? [];
  const baseCommit = workspace?.base_commit ?? '';
  const lights = status?.lights;
  const gates = [
    ['compile', t('编译', 'Build')],
    ['tests', t('测试', 'Tests')],
    ['regression', t('回归', 'Regression')],
    ['parity', t('两版一致', 'Parity')],
  ] as const;
  const checkCase = caseDir || state.selectedCase?.entry.dir || '';
  const checkDisabled =
    busy || sourceBusy || disabled || (selectedCheck.needs === 'case' && !checkCase);
  /**
   * 运行检查：编译内核/引擎、测试、两版一致都读磁盘上的源码，未保存的缓冲区不会算进去。先让用户
   * 保存/放弃/取消，再真正开始，免得编译的是旧代码却以为已经纳入（GUI-02）。
   */
  const runCheck = () => {
    const check = selectedCheck;
    const checkCaseDir = checkCase;
    void sourceChoice.ask(sourceRef.current).then((proceed) => {
      if (!proceed || !name) return;
      void execute(() =>
        invokeJson('workspace_check', {
          name,
          action: check.value,
          preset,
          package: pkg,
          case: checkCaseDir || null,
        }),
      ).catch(() => {});
    });
  };
  const revertFile = async (path: string) => {
    if (!name || !message.trim()) return;
    if (
      !(await askConfirm(t(`撤销 ${path} 的改动？`, `Discard the change to ${path}?`), {
        danger: true,
      }))
    )
      return;
    run(async () => {
      const value = await invokeJson('workspace_revert_file', { name, path, message });
      await onChanged();
      return value;
    });
  };
  const revertToCommit = async (commit: Commit) => {
    const id = commitId(commit);
    if (!name || !id) return;
    if (
      !(await askConfirm(
        t(
          `回到提交 ${shortHash(commit)}「${commit.subject ?? ''}」？\n这会丢弃这个提交之后的全部代码改动，不能撤销。`,
          `Return to commit ${shortHash(commit)} “${commit.subject ?? ''}”?\nThis discards every code change after it and cannot be undone.`,
        ),
        { danger: true, confirmLabel: t('回到这里', 'Return here') },
      ))
    )
      return;
    run(async () => {
      const value = await invokeJson('workspace_revert', { name, commit: id });
      await onChanged();
      return value;
    });
  };
  const deleteWorkspace = async () => {
    if (!name) return;
    if (
      !(await askConfirm(
        t(
          `删除代码工作区 ${name}？\n其中还没冻结的代码改动会一起删掉；官方代码和其他已冻结版本不受影响。`,
          `Delete code workspace ${name}?\nIts unfrozen code changes are deleted with it; the official code and other frozen versions are unaffected.`,
        ),
        { danger: true, confirmLabel: t('删除工作区', 'Delete workspace') },
      ))
    )
      return;
    run(async () => {
      const result = await invokeJson('workspace_delete', { name });
      setStatus(null);
      await onChanged();
      return result;
    }, false);
  };
  const pickCaseDir = () =>
    void invoke<string | null>('pick_folder', { key: 'workspace-case' }).then(
      (dir) => dir && setCaseDir(dir),
      (e) => setError(t(...workspaceErrorText(String(e)))),
    );
  const guideFields: UiField[] = [
    {
      id: 'workspace-message',
      label: t('本次修改说明（保存/替换时使用）', 'Change message for this edit (save/replace)'),
      value: message,
      type: 'text',
      set: setMessage,
    },
  ];
  if (name) {
    guideFields.push(
      {
        id: 'workspace-check-action',
        label: t('检查项', 'Check'),
        value: action,
        type: 'choice',
        options: CHECKS.map((check) => ({
          value: check.value,
          label: state.language === 'en' ? check.en : check.zh,
        })),
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
      {
        id: 'workspace-case',
        label: t('两版一致比较的算例目录', 'Case directory for parity comparison'),
        value: caseDir,
        type: 'text',
        set: setCaseDir,
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

  if (!name)
    return (
      <fieldset disabled={disabled || busy} style={{ border: 0, padding: 0 }}>
        <Card
          title={t('新建代码工作区', 'New code workspace')}
          description={t(
            '官方代码只读。工作区是它的独立可编辑副本；创建后可以修改、编译、检查，冻结版本时这份代码会一起冻结。',
            'The official code is read-only. A workspace is an independent editable copy; after creating it you can edit, build and check, and freezing the version freezes this code with it.',
          )}
        >
          {error && <Notice tone="error">{error}</Notice>}
          <Field
            label={t('工作区名称', 'Workspace name')}
            hint={t(
              '字母或数字开头，可含 - 和 _',
              'Start with a letter or digit; - and _ are allowed',
            )}
          >
            <Input value={newName} onChange={(e) => setNewName(e.target.value)} />
          </Field>
          <Field
            label={t('源码来源（留空使用内置来源）', 'Source (blank uses the bundled source)')}
          >
            <Input value={source} onChange={(e) => setSource(e.target.value)} />
          </Field>
          <Field label={t('目的（可选）', 'Purpose (optional)')}>
            <Input value={message} onChange={(e) => setMessage(e.target.value)} />
          </Field>
          <div className="button-row">
            <ActionButton
              action="workspace-create"
              label={t('创建工作区', 'Create workspace')}
              kind="commit"
              variant="primary"
              disabled={disabled || busy || !newName.trim()}
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
          </div>
          {output != null && (
            <details className="workspace-result">
              <summary>{t('创建结果（原始数据）', 'Creation result (raw)')}</summary>
              <pre>{JSON.stringify(output, null, 2)}</pre>
            </details>
          )}
        </Card>
      </fieldset>
    );

  return (
    <fieldset disabled={disabled || busy || sourceBusy} style={{ border: 0, padding: 0 }}>
      <div className="code-workspace-page">
        {error && <Notice tone="error">{error}</Notice>}
        <section className="workspace-hero">
          <div className="workspace-hero-main">
            <span className="workspace-eyebrow">{t('代码工作区', 'Code workspace')}</span>
            <h2>{workspace?.name ?? name}</h2>
            <p className="muted">
              {t(
                '这是官方代码的独立可编辑副本。改动、提交和检查都在这里进行；官方代码不会被修改。',
                'An independent editable copy of the official code. Changes, commits and checks happen here; the official code is not modified.',
              )}
            </p>
            <div className="workspace-meta">
              {baseCommit && (
                <span title={baseCommit}>
                  {t('基于', 'Based on')} <code>{baseCommit.slice(0, 8)}</code>
                </span>
              )}
              {workspace?.origin && <span title={workspace.origin}>{workspace.origin}</span>}
              {workspace?.note && <span>{workspace.note}</span>}
            </div>
          </div>
          <div className="workspace-chips" aria-label={t('工作区状态', 'Workspace status')}>
            {!status ? (
              <span className="workspace-chip">{t('正在读取状态…', 'Reading status…')}</span>
            ) : (
              <>
                {status.dirty ? (
                  <span className="workspace-chip is-warning">
                    {t('有未提交改动', 'Uncommitted changes')}
                  </span>
                ) : (
                  <span className="workspace-chip">{t('已提交', 'Committed')}</span>
                )}
                <span className="workspace-chip">
                  {t(
                    `${commits.length} 次提交`,
                    `${commits.length} commit${commits.length === 1 ? '' : 's'}`,
                  )}
                </span>
                {gates.map(([key, label]) => (
                  <span
                    className={`workspace-chip light-${lights?.[key] ?? 'unknown'}`}
                    key={key}
                    title={`${label}: ${lightText(lights?.[key], t)}`}
                  >
                    {label} · {lightText(lights?.[key], t)}
                  </span>
                ))}
                {status.parity_needed === false && (
                  <span className="workspace-chip light-not_needed">
                    {t('本次改动不需要两版一致', 'Parity not needed for this change')}
                  </span>
                )}
              </>
            )}
          </div>
        </section>
        {busy && <p className="workspace-busy">{t('正在处理…', 'Working…')}</p>}

        <section className="workspace-section">
          <div className="workspace-section-head">
            <div>
              <h3>{t('修改源码', 'Edit source')}</h3>
              <p className="muted">
                {t(
                  '选择文件直接编辑，或从本机替换文件/目录。每次保存、替换、删除都会形成一次可以回滚的提交。',
                  'Open a file to edit it, or replace a file/folder from this computer. Every save, replacement or deletion is a revertible commit.',
                )}
              </p>
            </div>
          </div>
          <Field
            label={t('本次修改说明（保存/替换时使用）', 'Change message (used by save/replace)')}
            hint={t(
              '一句话说明这次改了什么，最多 200 字。',
              'One line describing this change; at most 200 characters.',
            )}
          >
            <Input value={message} onChange={(e) => setMessage(e.target.value)} />
          </Field>
          <CodeFiles
            name={name}
            editor={editor}
            message={message}
            en={state.language === 'en'}
            disabled={busy || sourceBusy || disabled}
            onChanged={async () => {
              setStatus(await invokeJson<Status>('workspace_status', { name }));
              await onChanged();
            }}
            onPendingChange={handleSourcePending}
            onStateChange={handleSourceState}
            externalChangeToken={externalChangeToken}
          />
        </section>

        <section className="workspace-section">
          <div className="workspace-section-head">
            <div>
              <h3>{t('在外部软件中编辑', 'Edit in an external app')}</h3>
              <p className="muted">
                {t(
                  '也可以用 VS Code、记事本等软件打开源码文件夹。在外部保存后，回到窗口会自动记录改动，不需要手动保存。',
                  'You can also open the source folder in VS Code, Notepad or another app. Changes saved there are recorded when you return to the window.',
                )}
              </p>
            </div>
          </div>
          <div className="button-row workspace-actions">
            <ActionButton
              action="workspace-open-folder"
              label={t('在外部软件中打开源码文件夹', 'Open the source folder in an external app')}
              disabled={disabled || busy}
              run={() =>
                run(
                  () => invoke('workspace_open', { name, path: null, app: editor || null }),
                  false,
                )
              }
            >
              {t('打开源码文件夹', 'Open source folder')}
            </ActionButton>
            <ActionButton
              action="workspace-choose-editor"
              label={t('选择用哪个程序打开…', 'Choose the app to open with…')}
              disabled={disabled || busy}
              run={chooseEditor}
            >
              {t('选择程序…', 'Choose app…')}
            </ActionButton>
            {editor && (
              <Button
                variant="ghost"
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
              label={t('扫描外部修改', 'Scan for external changes')}
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
                  if (result.applied) {
                    await onChanged();
                    // 手动扫描也算"外部改动被记成提交"：让打开中的文件核对内容（GUI-03）。
                    setExternalChangeToken((token) => token + 1);
                  }
                  return result;
                })
              }
            >
              {t('扫描外部修改', 'Scan external changes')}
            </ActionButton>
          </div>
          <p className="muted workspace-footnote">
            {editor
              ? t(`当前使用：${editor}`, `Current app: ${editor}`)
              : t('当前使用系统默认程序。', 'Currently using the system default app.')}
            {t(
              ' .gitignore 忽略的编译产物和编辑器设置不算修改；oracle/golden 不能改。',
              ' Build outputs ignored by .gitignore and editor settings do not count; oracle/golden cannot be changed.',
            )}
          </p>
          {snapshotNote && <Notice tone="success">{snapshotNote}</Notice>}
        </section>

        <section className="workspace-section">
          <div className="workspace-section-head">
            <div>
              <h3>{t('改动', 'Changes')}</h3>
              <p className="muted">
                {t(
                  '相对官方代码的改动；可以查看差异或撤销单个文件。',
                  'Changes relative to the official code; inspect a diff or revert one file.',
                )}
              </p>
            </div>
            <span className="workspace-count">{changedFiles.length}</span>
          </div>
          {changedFiles.length ? (
            <div className="workspace-change-list">
              {changedFiles.map((file) => (
                <div className="workspace-change-row" key={file.path}>
                  <code className="workspace-change-path">{file.path}</code>
                  <span className="workspace-change-stat">
                    <span className="is-added">+{file.added ?? 0}</span>{' '}
                    <span className="is-removed">−{file.removed ?? 0}</span>
                  </span>
                  <div className="workspace-row-actions">
                    <ActionButton
                      action={`workspace-diff-${file.path}`}
                      label={`${t('差异', 'Diff')} ${file.path}`}
                      disabled={disabled || busy}
                      run={() => run(() => invokeJson('workspace_diff', { name, path: file.path }))}
                    >
                      {t('查看差异', 'Diff')}
                    </ActionButton>
                    <Button
                      variant="ghost"
                      disabled={disabled || busy || !message.trim()}
                      title={
                        message.trim()
                          ? undefined
                          : t('先填写修改说明', 'Enter a change message first')
                      }
                      onClick={() => void revertFile(file.path)}
                    >
                      {t('撤销文件改动', 'Revert file')}
                    </Button>
                  </div>
                </div>
              ))}
            </div>
          ) : (
            <p className="workspace-empty">
              {t('还没有相对官方代码的改动。', 'No changes relative to the official code yet.')}
            </p>
          )}
          <div className="button-row workspace-actions">
            <ActionButton
              action="workspace-patch"
              label={t('导入补丁…', 'Import patch…')}
              kind="commit"
              disabled={disabled || busy || !message.trim()}
              run={() => run(patch)}
            >
              {t('导入补丁…', 'Import patch…')}
            </ActionButton>
          </div>
          <details className="workspace-subsection">
            <summary>{t('导出补丁', 'Export a patch')}</summary>
            <Field label={t('导出补丁路径', 'Export patch path')}>
              <div className="study-path">
                <Input value={out} onChange={(e) => setOut(e.target.value)} />
                <Button
                  disabled={disabled || busy || !out}
                  onClick={() => run(() => invokeJson('workspace_export', { name, out }))}
                >
                  {t('导出改动', 'Export changes')}
                </Button>
              </div>
            </Field>
            <p className="muted workspace-footnote">
              {t(
                '补丁是相对官方代码的 unified diff；导入会作为一次新提交，导出不会改代码。',
                'A patch is a unified diff against the official code; importing creates a new commit, exporting does not change code.',
              )}
            </p>
          </details>
        </section>

        <section className="workspace-section">
          <div className="workspace-section-head">
            <div>
              <h3>{t('提交历史', 'Commit history')}</h3>
              <p className="muted">
                {t(
                  '每次保存或替换形成一次提交；回到某次提交会丢弃它之后的改动。',
                  'Every save or replacement creates a commit; returning to a commit discards everything after it.',
                )}
              </p>
            </div>
          </div>
          {commits.length ? (
            <ol className="workspace-commit-list">
              {commits.map((commit) => (
                <li className="workspace-commit-row" key={commitId(commit) || shortHash(commit)}>
                  <div className="workspace-commit-main">
                    <strong>{commit.subject || t('（没有说明）', '(no message)')}</strong>
                    <span className="workspace-commit-meta">
                      <code>{shortHash(commit)}</code>
                    </span>
                  </div>
                  <Button
                    variant="ghost"
                    disabled={disabled || busy || !commitId(commit)}
                    onClick={() => void revertToCommit(commit)}
                  >
                    {t('回到这次提交', 'Return to this commit')}
                  </Button>
                </li>
              ))}
            </ol>
          ) : (
            <p className="workspace-empty">{t('还没有提交。', 'No commits yet.')}</p>
          )}
          <p className="muted workspace-footnote">
            {t(
              '「回到这次提交」会把工作区代码整体回滚到该提交，并丢掉之后的全部改动，不能撤销。',
              'Return to this commit rolls the workspace back to it, discarding all later changes; this cannot be undone.',
            )}
          </p>
        </section>

        <section className="workspace-section">
          <div className="workspace-section-head">
            <div>
              <h3>{t('检查', 'Checks')}</h3>
              <p className="muted">
                {t(
                  '这些是开发者验收项，不会自动开始模拟；只运行你选择的一项。',
                  'These are developer gates, not simulations; only the selected check runs.',
                )}
              </p>
            </div>
          </div>
          <Field
            label={t('检查项', 'Check')}
            hint={state.language === 'en' ? selectedCheck.enHint : selectedCheck.zhHint}
          >
            <Select value={action} onChange={(e) => setAction(e.target.value)}>
              {CHECKS.map((check) => (
                <option key={check.value} value={check.value}>
                  {state.language === 'en' ? check.en : check.zh}
                </option>
              ))}
            </Select>
          </Field>
          <div className="form-grid workspace-check-fields">
            {(selectedCheck.needs === 'preset' || selectedCheck.needs === 'case') && (
              <Field label={t('编译配置', 'Build configuration')}>
                <Input value={preset} onChange={(e) => setPreset(e.target.value)} />
              </Field>
            )}
            {selectedCheck.needs === 'package' && (
              <Field label={t('测试包', 'Test package')}>
                <Input value={pkg} onChange={(e) => setPkg(e.target.value)} />
              </Field>
            )}
            {selectedCheck.needs === 'case' && (
              <Field
                label={t('比较用的算例目录', 'Case directory for comparison')}
                hint={t(
                  '两个引擎都会跑这个算例；空着用当前打开的算例。',
                  'Both engines run this case; empty uses the currently open case.',
                )}
              >
                <div className="study-path">
                  <Input value={caseDir} onChange={(e) => setCaseDir(e.target.value)} />
                  <Button disabled={disabled || busy} onClick={pickCaseDir}>
                    {t('选择…', 'Choose…')}
                  </Button>
                </div>
              </Field>
            )}
          </div>
          <div className="button-row workspace-actions">
            <ActionButton
              action="workspace-check"
              label={t('运行检查', 'Run check')}
              kind="commit"
              variant="primary"
              disabled={checkDisabled}
              run={runCheck}
            >
              {t('运行检查', 'Run check')}
            </ActionButton>
            {selectedCheck.needs === 'case' && !checkCase && (
              <span className="muted workspace-footnote">
                {t('先选择一个算例目录。', 'Choose a case directory first.')}
              </span>
            )}
          </div>
          {output != null && (
            <details className="workspace-result">
              <summary>
                {t('最近一次操作结果（原始数据）', 'Latest operation result (raw)')}
              </summary>
              <pre>{JSON.stringify(output, null, 2)}</pre>
            </details>
          )}
        </section>

        <details className="workspace-danger">
          <summary>{t('危险操作', 'Dangerous actions')}</summary>
          <p>
            {t(
              '删除这个代码工作区会删掉其中的未冻结代码和提交历史；官方代码、其他工作区和已经冻结的版本不受影响。',
              'Deleting this workspace removes its unfrozen code and commit history; the official code, other workspaces and already frozen versions are unaffected.',
            )}
          </p>
          <Button
            variant="danger"
            disabled={disabled || busy}
            onClick={() => void deleteWorkspace()}
          >
            {t('删除这个代码工作区', 'Delete this code workspace')}
          </Button>
        </details>
      </div>
      {/* 编译/测试前统一处理未保存源码：保存并继续 / 放弃并继续 / 取消。 */}
      {sourceChoice.dialog}
    </fieldset>
  );
}
