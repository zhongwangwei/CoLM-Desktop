import assert from 'node:assert/strict';
import test from 'node:test';
import { defaultObservation } from './default-observation';
import type { invoke } from '../../lib/ipc';

function fixture({
  dir = '/data/Forcing/',
  file = 'CN-Cng_2010_Met.nc',
  sites = [
    {
      met_file: '/data/Forcing/CN-Cng_2010_Met.nc',
      obs_file: '/data/Observation/CN-Cng_2010_Flux.nc',
    },
  ],
  spatial = false,
} = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const call = (async (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'open_case')
      return { profile: { spatial, inputs: { forcing_namelist: '/cases/custom/forcing.nml' } } };
    if (command === 'read_text') return 'forcing namelist';
    if (command === 'read_case')
      return [
        { path: 'DEF_dir_forcing', value: `'${dir.replaceAll("'", "''")}'` },
        { path: 'DEF_forcing%fprefix(1)', value: `'${file.replaceAll("'", "''")}'` },
      ];
    if (command === 'scan_sites') return { sites };
    throw new Error(`Unexpected command ${command}`);
  }) as typeof invoke;
  return { call, calls };
}

test('default observations follow the original forcing file even after renaming a case', async () => {
  const { call, calls } = fixture();
  assert.equal(
    await defaultObservation('/cases/my-renamed-case', call),
    '/data/Observation/CN-Cng_2010_Flux.nc',
  );
  assert.deepEqual(calls.at(-1)?.args, {
    dir: '/data/Sitedata',
    forcingDir: '/data/Forcing',
    quick: true,
  });
});

test('Urban-PLUMBER pairing supports Windows paths and quoted filenames', async () => {
  const { call } = fixture({
    dir: 'C:\\data\\Forcing\\',
    file: "O'Brien_metforcing_v1.nc",
    sites: [
      {
        met_file: "C:\\data\\Forcing\\O'Brien_metforcing_v1.nc",
        obs_file: 'C:\\data\\Observation\\urban.nc',
      },
    ],
  });
  assert.equal(
    await defaultObservation('C:\\cases\\urban', call),
    'C:\\data\\Observation\\urban.nc',
  );
});

test('missing or ambiguous observations never select an unrelated dataset', async () => {
  for (const sites of [
    [],
    [{ met_file: '/data/Forcing/other_Met.nc', obs_file: 'other.nc' }],
    [
      { met_file: '/data/Forcing/CN-Cng_2010_Met.nc', obs_file: 'one.nc' },
      { met_file: '/data/Forcing/CN-Cng_2010_Met.nc', obs_file: 'two.nc' },
    ],
  ]) {
    assert.equal(await defaultObservation('/cases/custom', fixture({ sites }).call), '');
  }
});

test('spatial cases do not load a point observation dataset', async () => {
  const { call, calls } = fixture({ spatial: true });
  assert.equal(await defaultObservation('/cases/grid', call), '');
  assert.equal(calls.length, 1);
});
