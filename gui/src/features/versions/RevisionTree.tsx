import { useEffect, useState } from 'react';
import { invokeJson } from '../../lib/ipc';
import type { FrozenRevision, ModelDraft, VersionRef } from '../../lib/types';
import { Notice } from '../../components/ui';
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

/**
 * 版本库：修改版（按派生关系缩进）、草稿、官方版本三组。官方版每次升级软件都会多一个，
 * 只突出最新的，更早的收起来（2026-10-20 本机 12 个官方修订占满了整个列表）。每行标出谁在用，
 * 没用的可以单个删，也可以一次清理。
 */
export default function RevisionTree({
  versions,
  drafts,
  selected,
  openDraft,
  dirty,
  disabled,
  en,
  onSelect,
  onOpenDraft,
  onDeleted,
}: {
  versions: FrozenRevision[];
  drafts: ModelDraft[];
  selected: FrozenRevision | null;
  /** 正在右边打开的草稿。 */
  openDraft: string | null;
  dirty: boolean;
  disabled: boolean;
  en: boolean;
  onSelect: (v: FrozenRevision) => void;
  onOpenDraft: (id: string) => void;
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
  const nameOf = (ref: VersionRef) => {
    const found = versions.find((v) => same(v.version, ref));
    return found ? `${found.name} · r${found.version.revision}` : `${ref.id} r${ref.revision}`;
  };
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
  const row = (v: FrozenRevision, indent = 0) => {
    const use = of(v);
    const active = !openDraft && same(v.version, chosen);
    return (
      <li key={`${v.version.id}@${v.version.revision}`} style={{ paddingInlineStart: indent * 14 }}>
        <div className={`version-row${active ? ' is-active' : ''}`}>
          <button
            type="button"
            className="version-row-main"
            disabled={disabled || busy}
            aria-current={active ? 'true' : undefined}
            onClick={() => onSelect(v)}
          >
            <span className="version-row-title">
              <span className="version-row-name">{v.name}</span>
              <span className="version-row-rev">r{v.version.revision}</span>
            </span>
            <span className="version-row-meta">
              {same(v.version, chosen) && <span className="version-dot" aria-hidden="true" />}
              {same(v.version, chosen)
                ? t('模拟正在使用', 'Used for simulations')
                : usageLabel(use, chosen, en)}
            </span>
          </button>
          {removable(use, chosen) && (
            <button
              type="button"
              className="version-row-delete"
              disabled={disabled || busy}
              title={t('删除这个版本', 'Delete this version')}
              aria-label={t(
                `删除 ${v.name} r${v.version.revision}`,
                `Delete ${v.name} r${v.version.revision}`,
              )}
              onClick={() => void remove([v])}
            >
              ×
            </button>
          )}
        </div>
      </li>
    );
  };
  const olderOpen = older.some((v) => same(v.version, chosen));
  return (
    <nav className="version-library" aria-label={t('版本库', 'Version library')}>
      <section>
        <h3>{t('修改版', 'Modified versions')}</h3>
        {modified.length === 0 ? (
          <p className="version-library-empty">
            {t(
              '还没有修改版。点右上角「新建修改版」，从选中的版本派生一个。',
              'No modified versions yet. Use “New modified version” to derive one from the selected version.',
            )}
          </p>
        ) : (
          <ul>{modified.map((v) => row(v, Math.max(0, versionDepth(v, versions) - 1)))}</ul>
        )}
      </section>
      {drafts.length > 0 && (
        <section>
          <h3>{t('草稿', 'Drafts')}</h3>
          <ul>
            {drafts.map((d) => {
              const active = openDraft === d.id;
              return (
                <li key={d.id}>
                  <div className={`version-row is-draft${active ? ' is-active' : ''}`}>
                    <button
                      type="button"
                      className="version-row-main"
                      disabled={disabled || busy}
                      aria-current={active ? 'true' : undefined}
                      onClick={() => onOpenDraft(d.id)}
                    >
                      <span className="version-row-title">
                        <span className="version-row-name">{d.name}</span>
                        {active && dirty && (
                          <span className="version-row-unsaved">{t('未保存', 'Unsaved')}</span>
                        )}
                      </span>
                      <span className="version-row-meta">
                        {t(`基于 ${nameOf(d.parent)}`, `Based on ${nameOf(d.parent)}`)}
                      </span>
                    </button>
                  </div>
                </li>
              );
            })}
          </ul>
        </section>
      )}
      <section>
        <h3>{t('官方版本', 'Official versions')}</h3>
        <ul>{latest && row(latest)}</ul>
        {older.length > 0 && (
          <details className="version-older" open={olderOpen || undefined}>
            <summary>
              {t(
                `更早的官方版本（${older.length}）`,
                `Earlier official versions (${older.length})`,
              )}
            </summary>
            <ul>{older.map((v) => row(v))}</ul>
          </details>
        )}
      </section>
      {spare.length > 0 && (
        <button
          type="button"
          className="version-library-clean"
          disabled={disabled || busy}
          onClick={() => void remove(spare)}
        >
          {t(`清理没用的版本（${spare.length} 个）`, `Remove unused versions (${spare.length})`)}
        </button>
      )}
      {notice && <Notice tone="success">{notice}</Notice>}
      {error && <Notice tone="error">{error}</Notice>}
    </nav>
  );
}
