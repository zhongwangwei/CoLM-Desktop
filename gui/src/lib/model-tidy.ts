import type { FrozenRevision, VersionRef } from './types';

/** `model_prune`: old official revisions removed (or, for a dry run, removable). */
export type PruneReport = {
  dry_run: boolean;
  removed: VersionRef[];
  kept: { version: VersionRef; reason: string }[];
  paths: string[];
  bytes: number;
};

type InvokeJson = <T>(command: 'model_prune', args: { dryRun: boolean }) => Promise<T>;

/**
 * Once the version list is loaded at start, tidy away old official revisions nothing uses.
 * The backend refuses while a run or case creation is active; any failure is only logged, it
 * never interrupts the user. Returns what was removed.
 */
export async function tidyModelsAfterStartup(invokeJson: InvokeJson): Promise<VersionRef[]> {
  try {
    const report = await invokeJson<PruneReport>('model_prune', { dryRun: false });
    return report.removed;
  } catch (error) {
    console.warn(`model cache tidy skipped: ${String(error)}`);
    return [];
  }
}

const same = (a: VersionRef, b: VersionRef) => a.id === b.id && a.revision === b.revision;

export function withoutRevisions(versions: FrozenRevision[], removed: VersionRef[]) {
  return versions.filter((v) => !removed.some((r) => same(r, v.version)));
}

export const versionLabel = (v: VersionRef) => `${v.id}#${v.revision}`;

export function formatBytes(bytes: number) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
  if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(0)} MB`;
  return `${Math.ceil(bytes / 1024)} KB`;
}

/** The revision a case is bound to, when opening failed because that revision is gone. */
export function missingRevision(error: unknown): VersionRef | null {
  const text = error instanceof Error ? error.message : String(error);
  const match = /model revision ([A-Za-z0-9_.-]+)#(\d+) is no longer in the model registry/.exec(
    text,
  );
  return match ? { id: match[1], revision: Number(match[2]) } : null;
}

export function rebindQuestion(
  missing: VersionRef,
  target: VersionRef,
  bilingual: (zh: string, en: string) => string,
) {
  return bilingual(
    `该算例绑定的模型版本 ${versionLabel(missing)} 已不在模型库中（不再使用的旧官方修订会被自动整理）。\n\n改绑到 ${versionLabel(target)} 并打开？选择"取消"则不打开、不改动算例。`,
    `This case is bound to model revision ${versionLabel(missing)}, which is no longer in the model registry (unused old official revisions are tidied away).\n\nRebind it to ${versionLabel(target)} and open it? Cancel leaves the case unopened and unchanged.`,
  );
}

/** Ask before rebinding; a separate function so tests can answer for the user. */
export const askToRebind = (question: string) => window.confirm(question);
