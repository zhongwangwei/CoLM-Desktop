//! Pure helpers shared by the site and forcing halves of the preprocessing workbench.

export function normalizeSiteStem(value) {
  return String(value ?? '')
    .trim()
    .replace(/(?:_site(?:_v1)?\.nc|_site)$/i, '')
    .replace(/[^A-Za-z0-9._-]+/g, '-')
    .replace(/^-+|-+$/g, '');
}

export const siteOutputName = stem => `${normalizeSiteStem(stem)}_site.nc`;
export const forcingOutputName = stem => `${normalizeSiteStem(stem)}_Met.nc`;

export function missingForcingHeights(heights) {
  return [['V', heights?.v], ['T', heights?.t], ['Q', heights?.q]]
    .filter(([, value]) => value == null || !Number.isFinite(Number(value)))
    .map(([name]) => name);
}

export function parentDirectory(path) {
  const value = String(path ?? '').replace(/[\\/]+$/, '');
  const split = Math.max(value.lastIndexOf('/'), value.lastIndexOf('\\'));
  return split < 0 ? '' : value.slice(0, split);
}

export function prepMode(state) {
  if (state.wizard?.physics?.urban) return 'urban';
  return String(state.subgrid ?? state.wizard?.subgrid ?? 'IGBP').toLowerCase();
}
