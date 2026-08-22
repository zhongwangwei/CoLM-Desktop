import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const frontend = await readFile(new URL('../dist/app/forcing.js', import.meta.url), 'utf8');
for (const command of [
  'probe_forcing_gaps',
  'repair_forcing',
  'download_era5land',
  'convert_forcing',
]) {
  assert.match(frontend, new RegExp(`invoke\\('${command}'`), `${command} must be wired`);
}
assert.match(frontend, /短缺口上限（时间步）/);
assert.match(frontend, /ERA5-Land 缓存目录/);
assert.match(frontend, /\*_gapfill_qc/);
assert.match(frontend, /if \(!gapReport\) reasons\.push\('先完成缺测与时区诊断'\)/);
assert.match(frontend, /gapReport\.missing > 0 && !repairedSource/);
assert.match(frontend, /src: repairedSource \?\? src/);

const backend = await readFile(new URL('../src-tauri/src/forcing.rs', import.meta.url), 'utf8');
for (const command of ['forcing-gap-probe', 'forcing-repair', 'era5land-download']) {
  assert.match(backend, new RegExp(command));
}

const design = await readFile(new URL('../../docs/plan-forcing-gap-repair.md', import.meta.url), 'utf8');
for (const contract of ['原始文件永不覆盖', 'longitude_inferred_offset', 'ERA5-Land', 'QC']) {
  assert.match(design, new RegExp(contract));
}

console.log('forcing gap fill: diagnosis, timezone, ERA5-Land, provenance, and conversion gates are wired');
