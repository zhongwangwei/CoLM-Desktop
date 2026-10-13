import type { FrozenRevision, VersionRef } from '../../lib/types';

export type VersionRow = { version: FrozenRevision; depth: number };

const key = (ref: VersionRef) => `${ref.id}@${ref.revision}`;

/**
 * Versions a user can start new work from, as a source tree.
 *
 * Only the newest revision of each version is offered: older revisions stay in the registry for
 * the runs that used them, but nobody should start new work on them. The official version's older
 * revisions are app rebuilds, so children of any official revision hang under the current one.
 */
export function choosableVersions(versions: FrozenRevision[]): VersionRow[] {
  const latest = new Map<string, FrozenRevision>();
  for (const version of versions) {
    const current = latest.get(version.version.id);
    if (!current || version.version.revision > current.version.revision)
      latest.set(version.version.id, version);
  }
  const shown = [...latest.values()];
  const byId = new Map(shown.map((version) => [version.version.id, version]));
  const parentOf = (version: FrozenRevision): string | null => {
    const parent = version.parent;
    if (!parent) return null;
    return byId.has(parent.id) ? parent.id : null;
  };
  const children = new Map<string | null, FrozenRevision[]>();
  for (const version of shown) {
    const parent = parentOf(version);
    children.set(parent, [...(children.get(parent) ?? []), version]);
  }
  const order = (a: FrozenRevision, b: FrozenRevision) =>
    a.version.id === 'official'
      ? -1
      : b.version.id === 'official'
        ? 1
        : a.created_unix - b.created_unix || a.name.localeCompare(b.name);
  const rows: VersionRow[] = [];
  const seen = new Set<string>();
  const walk = (parent: string | null, depth: number) => {
    for (const version of [...(children.get(parent) ?? [])].sort(order)) {
      if (seen.has(key(version.version))) continue;
      seen.add(key(version.version));
      rows.push({ version, depth });
      walk(version.version.id, depth + 1);
    }
  };
  walk(null, 0);
  return rows;
}

/** Newest official revision: the default for new work. */
export function latestOfficial(versions: FrozenRevision[]): FrozenRevision | undefined {
  return versions
    .filter((version) => version.version.id === 'official')
    .sort((a, b) => b.version.revision - a.version.revision)[0];
}

/**
 * Name for a version derived to apply a scheme. Earlier scheme suffixes are replaced rather than
 * appended, so switching PFT → PC → LCT does not grow "· PFT · PC · LCT".
 */
export function schemeVersionName(baseName: string, scheme: string): string {
  const base = baseName.replace(/(\s·\s(PFT|PC|LCT|IGBP|USGS))+$/u, '');
  return `${base} · ${scheme}`;
}
