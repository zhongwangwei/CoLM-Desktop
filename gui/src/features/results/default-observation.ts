import { invoke } from '../../lib/ipc';
import type { OpenedCase, Site } from '../../lib/types';

const normalize = (path: string) => path.replace(/\\/g, '/').replace(/\/+$/, '');
const parent = (path: string) => normalize(path).replace(/\/[^/]*$/, '');
const unquote = (value: string) => {
  const quote = value[0];
  return (quote === "'" || quote === '"') && value.endsWith(quote)
    ? value.slice(1, -1).replaceAll(quote + quote, quote)
    : value;
};
const absolute = (path: string, base: string) =>
  /^(?:\/|[A-Za-z]:[\\/])/.test(path) ? normalize(path) : `${normalize(base)}/${path}`;

/** Reuse the scanner's verified dataset pairing; case names may be user-defined. */
export async function defaultObservation(caseDir: string, call: typeof invoke = invoke) {
  const opened = await call<OpenedCase>('open_case', { dir: caseDir });
  if (opened.profile.spatial) return '';
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
  const forcingDir = absolute(directory, parent(path));
  const forcingFile = absolute(file, forcingDir);
  const found = await call<{ sites: Site[] }>('scan_sites', {
    dir: `${parent(forcingDir)}/Sitedata`,
    forcingDir,
    quick: true,
  });
  const matches = found.sites.filter(
    (site) => site.met_file && normalize(site.met_file) === forcingFile && site.obs_file,
  );
  const observations = [...new Set(matches.map((site) => site.obs_file!))];
  return observations.length === 1 ? observations[0] : '';
}
