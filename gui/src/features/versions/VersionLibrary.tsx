import { useEffect, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import { askConfirm } from '../../lib/confirm';
import type { FrozenRevision, ModelDraft, Overlay, VersionRef } from '../../lib/types';
import { Card, Notice } from '../../components/ui';
import { versionDepth } from './model';
import { appLocale } from '../history/history';

export type RevisionUsage = {
  version: VersionRef;
  cases: number;
  history: number;
  blocker: string | null;
};
type DeleteReport = {
  deleted: VersionRef[];
  failed: { version: VersionRef; reason: string }[];
};

const same = (a: VersionRef, b: VersionRef | null | undefined) =>
  !!b && a.id === b.id && a.revision === b.revision;

/**
 * 能不能清理：注册表允许（不是最新官方版、没有派生版本或草稿），没有现存的算例或研究绑定，也不是
 * 当前选中的版本。只剩历史记录提到它也可以删——记录留着，只是不能再按原版本重放
 * （旧修订原来因此永远删不掉：2026-10-20 实测 6 个修订只剩历史引用）。
 */
export function removable(usage: RevisionUsage | undefined, selected: VersionRef | null) {
  return !!usage && usage.blocker == null && usage.cases === 0 && !same(usage.version, selected);
}

export function usageLabel(
  usage: RevisionUsage | undefined,
  selected: VersionRef | null,
  en: boolean,
) {
  const t = (zh: string, english: string) => (en ? english : zh);
  if (!usage) return '';
  if (same(usage.version, selected)) return t('当前选中', 'Selected');
  if (usage.blocker != null)
    return usage.version.id === 'official' && /latest official/.test(usage.blocker)
      ? t('当前官方版本', 'Current official')
      : t('有派生版本或草稿', 'Has derived versions or drafts');
  if (usage.cases > 0) return t(`${usage.cases} 个算例在用`, `Used by ${usage.cases} cases`);
  if (usage.history > 0)
    return t(`只有 ${usage.history} 条历史记录`, `Only ${usage.history} history records`);
  return t('没有用到', 'Unused');
}

/** 一个版本相对父版本改了什么，一行写完。 */
export function changeSummary(overlay: Overlay, ownCode: boolean, en: boolean): string {
  const t = (zh: string, english: string) => (en ? english : zh);
  const parts = [
    overlay.parameters.length > 0 &&
      t(`参数 ${overlay.parameters.length}`, `${overlay.parameters.length} parameters`),
    Object.keys(overlay.physics).length > 0 &&
      t(
        `开关 ${Object.keys(overlay.physics).length}`,
        `${Object.keys(overlay.physics).length} switches`,
      ),
    overlay.process_files.length > 0 &&
      t(
        `过程文件 ${overlay.process_files.length}`,
        `${overlay.process_files.length} process files`,
      ),
    overlay.hybrid && t('AI 网络', 'AI network'),
    ownCode && t('自己的代码', 'own code'),
  ].filter(Boolean);
  return parts.length ? parts.join(' · ') : t('没有改动', 'No changes');
}

/**
 * 版本库：整页宽的三张表——修改版（按派生关系缩进）、草稿、官方版本。原来是挤在左边的一栏
 * （2026-10-20 用户反映左右分栏浪费地方），信息只放得下名字。点名字打开版本；"设为模拟使用"
 * 单独一个按钮，打开查看不再顺手换掉模拟用的版本。
 */
export default function VersionLibrary({
  versions,
  drafts,
  inUse,
  disabled,
  en,
  onOpen,
  onUse,
  onOpenDraft,
  onDeleted,
  onDraftDeleted,
}: {
  versions: FrozenRevision[];
  drafts: ModelDraft[];
  /** 模拟正在使用的版本。 */
  inUse: FrozenRevision | null;
  disabled: boolean;
  en: boolean;
  onOpen: (v: FrozenRevision) => void;
  onUse: (v: FrozenRevision) => void;
  onOpenDraft: (id: string) => void;
  onDeleted: () => Promise<void>;
  /** 删掉一个草稿之后：刷新草稿列表。 */
  onDraftDeleted: (id: string) => Promise<void>;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [usage, setUsage] = useState<RevisionUsage[]>([]);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [tick, setTick] = useState(0);
  const chosen = inUse?.version ?? null;
  useEffect(() => {
    let live = true;
    void invokeJson<RevisionUsage[]>('model_usage')
      .then((value) => live && setUsage(value))
      .catch((e) => live && setError(String(e)));
    return () => {
      live = false;
    };
  }, [versions, tick]);
  const of = (v: FrozenRevision) => usage.find((u) => same(u.version, v.version));
  const spare = versions.filter((v) => removable(of(v), chosen));
  const officials = versions
    .filter((v) => v.version.id === 'official')
    .sort((a, b) => b.version.revision - a.version.revision);
  const [latest, ...older] = officials;
  // 修改版按派生关系排：父版本在前，派生出的紧跟在它下面。
  const modified: FrozenRevision[] = [];
  const own = versions.filter((v) => v.version.id !== 'official');
  const place = (v: FrozenRevision) => {
    if (modified.includes(v)) return;
    modified.push(v);
    own.filter((c) => same(v.version, c.parent)).forEach(place);
  };
  own.filter((v) => !own.some((p) => same(p.version, v.parent))).forEach(place);
  own.forEach(place);
  const nameOf = (ref: VersionRef | null | undefined) => {
    if (!ref) return t('—（官方基线）', '— (official baseline)');
    const found = versions.find((v) => same(v.version, ref));
    return found ? `${found.name} · r${found.version.revision}` : `${ref.id} r${ref.revision}`;
  };
  const date = (unix: number) =>
    unix > 0 ? new Date(unix * 1000).toLocaleDateString(appLocale(en)) : '—';
  async function remove(targets: FrozenRevision[]) {
    if (busy || disabled || targets.length === 0) return;
    const records = targets.reduce((sum, v) => sum + (of(v)?.history ?? 0), 0);
    const names = targets.map((v) => `${v.name} · r${v.version.revision}`).join(t('、', ', '));
    if (
      !(await askConfirm(
        t(
          `删除 ${names}？${records > 0 ? `提到它们的 ${records} 条历史记录会保留，但不能再按原版本重放。` : ''}删除后不能恢复。`,
          `Delete ${names}? ${records > 0 ? `The ${records} history records that name them stay, but can no longer be replayed with that version. ` : ''}This cannot be undone.`,
        ),
        { confirmLabel: t('删除', 'Delete'), danger: true },
      ))
    )
      return;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      // 先删派生得最深的：父版本要等它的派生版本删掉后才能删。
      const ordered = [...targets].sort(
        (a, b) => versionDepth(b, versions) - versionDepth(a, versions),
      );
      const report = await invokeJson<DeleteReport>('model_delete', {
        versions: ordered.map((v) => v.version),
        keepHistory: true,
      });
      await onDeleted();
      setNotice(
        t(`已删除 ${report.deleted.length} 个版本`, `Deleted ${report.deleted.length} versions`),
      );
      if (report.failed.length > 0)
        setError(
          report.failed.map((f) => `${f.version.id}#${f.version.revision}: ${f.reason}`).join('\n'),
        );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setTick((n) => n + 1);
    }
  }
  async function removeDraft(d: ModelDraft) {
    if (busy || disabled) return;
    if (
      !(await askConfirm(
        t(
          `删除草稿「${d.name}」？已经从它冻结出的版本不受影响。删除后不能恢复。`,
          `Delete the draft “${d.name}”? Versions already frozen from it stay. This cannot be undone.`,
        ),
        { confirmLabel: t('删除草稿', 'Delete draft'), danger: true },
      ))
    )
      return;
    // 草稿自己的代码副本：里面可能有还没冻结的代码改动，单独问一次。别的草稿也在用的不删（后端会再查一遍）。
    const shared = drafts.some(
      (other) => other.id !== d.id && other.code.workspace === d.code.workspace,
    );
    const deleteWorkspace =
      !!d.code.workspace &&
      !shared &&
      (await askConfirm(
        t(
          `同时删除它的代码工作区 ${d.code.workspace}？里面还没冻结的代码改动会一起删掉。`,
          `Also delete its code workspace ${d.code.workspace}? Code changes not yet frozen go with it.`,
        ),
        {
          confirmLabel: t('一起删除代码', 'Delete the code too'),
          cancelLabel: t('保留代码', 'Keep the code'),
          danger: true,
        },
      ));
    setBusy(true);
    setError('');
    setNotice('');
    try {
      await invokeJson('model_delete_draft', { id: d.id, deleteWorkspace });
      await onDraftDeleted(d.id);
      setNotice(t(`已删除草稿「${d.name}」`, `Deleted the draft “${d.name}”`));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      setTick((n) => n + 1);
    }
  }
  const lock = disabled || busy;
  const action = (text: string, run: () => void, danger = false) => (
    <button
      type="button"
      className={`library-action${danger ? ' is-danger' : ''}`}
      disabled={lock}
      onClick={run}
    >
      {text}
    </button>
  );
  const versionRow = (v: FrozenRevision, indent = 0) => {
    const use = of(v);
    const used = same(v.version, chosen);
    return (
      <tr key={`${v.version.id}@${v.version.revision}`} className={used ? 'is-used' : undefined}>
        <td>
          <div className="library-name" style={{ paddingInlineStart: indent * 18 }}>
            {indent > 0 && (
              <span className="library-branch" aria-hidden="true">
                └
              </span>
            )}
            <button type="button" disabled={lock} onClick={() => onOpen(v)}>
              {v.name}
            </button>
            <span className="library-rev">r{v.version.revision}</span>
            {used && (
              <span className="badge versions-badge is-used">
                {t('模拟正在使用', 'Used for simulations')}
              </span>
            )}
          </div>
          {v.note && v.version.id !== 'official' && <p className="library-note">{v.note}</p>}
        </td>
        <td className="muted">{v.version.id === 'official' ? '—' : nameOf(v.parent)}</td>
        <td>
          {v.version.id === 'official'
            ? t('官方源码', 'Official source')
            : changeSummary(v.overlay, !!v.code.workspace, en)}
        </td>
        <td className="muted">{used ? '' : usageLabel(use, chosen, en)}</td>
        <td className="muted library-date">{date(v.created_unix)}</td>
        <td className="library-actions">
          {action(t('打开', 'Open'), () => onOpen(v))}
          {!used && action(t('设为模拟使用', 'Use for simulations'), () => onUse(v))}
          {removable(use, chosen) && action(t('删除', 'Delete'), () => void remove([v]), true)}
        </td>
      </tr>
    );
  };
  const head = (
    <thead>
      <tr>
        <th>{t('版本', 'Version')}</th>
        <th>{t('基于', 'Based on')}</th>
        <th>{t('改动', 'Changes')}</th>
        <th>{t('使用情况', 'Usage')}</th>
        <th>{t('冻结于', 'Frozen')}</th>
        <th />
      </tr>
    </thead>
  );
  return (
    <div className="version-library">
      {notice && <Notice tone="success">{notice}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
      <Card
        title={t('修改版', 'Modified versions')}
        description={t(
          '在官方代码上改了参数、开关、AI 网络或代码本身的版本；冻结后只读。',
          'Versions that change parameters, switches, an AI network or the code itself; read-only once frozen.',
        )}
      >
        {modified.length === 0 ? (
          <p className="versions-empty">
            {t(
              '还没有修改版。点右上角「新建修改版」从一个版本派生。',
              'No modified versions yet. Use “New modified version” to derive one.',
            )}
          </p>
        ) : (
          <div className="table-scroll">
            <table className="versions-table library-table">
              {head}
              <tbody>
                {modified.map((v) => versionRow(v, Math.max(0, versionDepth(v, versions) - 1)))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
      {drafts.length > 0 && (
        <Card
          title={t('草稿', 'Drafts')}
          description={t(
            '还在改的版本。冻结为新版本后草稿自动删除。',
            'Versions still being edited. A draft goes away once it is frozen as a version.',
          )}
        >
          <div className="table-scroll">
            <table className="versions-table library-table">
              <thead>
                <tr>
                  <th>{t('草稿', 'Draft')}</th>
                  <th>{t('基于', 'Based on')}</th>
                  <th>{t('改动', 'Changes')}</th>
                  <th>{t('创建于', 'Created')}</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {drafts.map((d) => (
                  <tr key={d.id}>
                    <td>
                      <div className="library-name">
                        <button type="button" disabled={lock} onClick={() => onOpenDraft(d.id)}>
                          {d.name}
                        </button>
                      </div>
                      {d.note && <p className="library-note">{d.note}</p>}
                    </td>
                    <td className="muted">{nameOf(d.parent)}</td>
                    <td>{changeSummary(d.overlay, !!d.code.workspace, en)}</td>
                    <td className="muted library-date">{date(d.created_unix)}</td>
                    <td className="library-actions">
                      {action(t('继续编辑', 'Edit'), () => onOpenDraft(d.id))}
                      {action(t('删除', 'Delete'), () => void removeDraft(d), true)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Card>
      )}
      <Card
        title={t('官方版本', 'Official versions')}
        description={t(
          '随软件提供的 CoLM 代码，每次升级软件多一个；只有最新的常用。',
          'The CoLM code shipped with the app; each upgrade adds one, and only the latest is normally used.',
        )}
      >
        <div className="table-scroll">
          <table className="versions-table library-table">
            {head}
            <tbody>{latest && versionRow(latest)}</tbody>
          </table>
        </div>
        {older.length > 0 && (
          <details
            className="version-older"
            open={older.some((v) => same(v.version, chosen)) || undefined}
          >
            <summary>
              {t(
                `更早的官方版本（${older.length}）`,
                `Earlier official versions (${older.length})`,
              )}
            </summary>
            <div className="table-scroll">
              <table className="versions-table library-table">
                <tbody>{older.map((v) => versionRow(v))}</tbody>
              </table>
            </div>
          </details>
        )}
        {spare.length > 0 && (
          <button
            type="button"
            className="library-action"
            disabled={lock}
            onClick={() => void remove(spare)}
          >
            {t(`清理没用的版本（${spare.length} 个）`, `Remove unused versions (${spare.length})`)}
          </button>
        )}
      </Card>
    </div>
  );
}
