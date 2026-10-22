import { invoke } from '../../lib/ipc';
import type { OpenedCase, Site } from '../../lib/types';

const normalize = (path: string) => {
  const slashes = path.replace(/\\/g, '/');
  if (/^(?:\/|[A-Za-z]:\/)\/*$/.test(slashes)) return slashes.replace(/\/+$/, '/');
  return slashes.replace(/\/+$/, '');
};
const parent = (path: string) => {
  const normalized = normalize(path);
  if (/^(?:\/|[A-Za-z]:\/|\/\/[^/]+\/[^/]+)$/.test(normalized)) return normalized;
  return normalize(normalized.slice(0, normalized.lastIndexOf('/') + 1));
};
const unquote = (value: string) => {
  const quote = value[0];
  return (quote === "'" || quote === '"') && value.endsWith(quote)
    ? value.slice(1, -1).replaceAll(quote + quote, quote)
    : value;
};
const absolute = (path: string, base: string) => {
  const normalized = normalize(path);
  if (/^(?:\/|[A-Za-z]:\/)/.test(normalized)) return normalized;
  const directory = normalize(base);
  return normalize(`${directory}${directory.endsWith('/') ? '' : '/'}${normalized}`);
};

/** Reuse the scanner's verified dataset pairing; case names may be user-defined. */
export async function defaultObservation(caseDir: string, call: typeof invoke = invoke) {
  const opened = await call<OpenedCase>('open_case', { dir: caseDir });
  if (opened.profile.spatial) return '';
  // 后端按 Sitedata/../Observation 的约定已经找好（服务器项目给的是服务器上的路径，评估也在服务器上算）。
  if (opened.profile.obs_file) return opened.profile.obs_file;
  // Install release dataset corrections while preserving user-edited examples.
  await call('install_example', {}).catch(() => {});
  const forcingNamelist = opened.profile.inputs.forcing_namelist;
  if (typeof forcingNamelist !== 'string' || !forcingNamelist) return '';
  const path = absolute(forcingNamelist, caseDir);
  const text = await call<string>('read_text', { path });
  const entries = await call<{ path: string; value: string }[]>('read_case', { text });
  const value = (key: string) =>
    unquote(entries.find((entry) => entry.path.toLowerCase() === key.toLowerCase())?.value || '');
  const directory = value('DEF_dir_forcing');
  const file = value('DEF_forcing%fprefix(1)');
  if (!directory || !file) return '';
  const forcingDir = absolute(directory, caseDir);
  const forcingFile = absolute(file, forcingDir);
  const found = await call<{ sites: Site[] }>('scan_sites', {
    dir: absolute('Sitedata', parent(forcingDir)),
    forcingDir,
    quick: true,
  });
  const matches = found.sites.filter(
    (site) => site.met_file && normalize(site.met_file) === forcingFile && site.obs_file,
  );
  const observations = [...new Set(matches.map((site) => site.obs_file!))];
  return observations.length === 1 ? observations[0] : '';
}
