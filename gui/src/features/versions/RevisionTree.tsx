import { useEffect, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import type { FrozenRevision, VersionRef } from '../../lib/types';
import { Button, Notice } from '../../components/ui';
import { versionDepth } from './model';

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

/** 版本树：每个修订标出谁在用，没用的可以单个删，也可以一次清理。 */
export default function RevisionTree({
  versions,
  selected,
  disabled,
  en,
  onSelect,
  onDeleted,
}: {
  versions: FrozenRevision[];
  selected: FrozenRevision | null;
  disabled: boolean;
  en: boolean;
  onSelect: (v: FrozenRevision) => void;
  onDeleted: () => Promise<void>;
}) {
  const t = (zh: string, english: string) => (en ? english : zh);
  const [usage, setUsage] = useState<RevisionUsage[]>([]);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [tick, setTick] = useState(0);
  const chosen = selected?.version ?? null;
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
  async function remove(targets: FrozenRevision[]) {
    if (busy || disabled || targets.length === 0) return;
    const records = targets.reduce((sum, v) => sum + (of(v)?.history ?? 0), 0);
    const names = targets.map((v) => `${v.name} · r${v.version.revision}`).join(t('、', ', '));
    if (
      !window.confirm(
        t(
          `删除 ${names}？${records > 0 ? `提到它们的 ${records} 条历史记录会保留，但不能再按原版本重放。` : ''}删除后不能恢复。`,
          `Delete ${names}? ${records > 0 ? `The ${records} history records that name them stay, but can no longer be replayed with that version. ` : ''}This cannot be undone.`,
        ),
      )
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
  return (
    <>
      <nav aria-label={t('版本树', 'Version tree')} className="version-tree">
        {versions.map((v) => {
          const use = of(v);
          return (
            <div
              key={`${v.version.id}@${v.version.revision}`}
              className="version-tree-row"
              style={{ paddingInlineStart: versionDepth(v, versions) * 14 }}
            >
              <Button
                disabled={disabled || busy}
                variant={same(v.version, chosen) ? 'primary' : 'ghost'}
                onClick={() => onSelect(v)}
              >
                {v.name} · r{v.version.revision}{' '}
                {v.version.id === 'official' ? t('官方', 'Official') : ''}
              </Button>
              <small className="muted">{usageLabel(use, chosen, en)}</small>
              {removable(use, chosen) && (
                <button
                  type="button"
                  className="version-tree-delete"
                  disabled={disabled || busy}
                  title={t('删除这个版本', 'Delete this version')}
                  onClick={() => void remove([v])}
                >
                  {t('删除', 'Delete')}
                </button>
              )}
            </div>
          );
        })}
      </nav>
      {spare.length > 0 && (
        <Button disabled={disabled || busy} onClick={() => void remove(spare)}>
          {t(`清理没用的版本（${spare.length} 个）`, `Remove unused versions (${spare.length})`)}
        </Button>
      )}
      {notice && <Notice tone="success">{notice}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
    </>
  );
}
