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
  namelist = '/cases/custom/forcing.nml',
  obs_file = undefined as string | undefined,
} = {}) {
  const calls: { command: string; args: Record<string, unknown> }[] = [];
  const call = (async (command: string, args: Record<string, unknown>) => {
    calls.push({ command, args });
    if (command === 'install_example') return {};
    if (command === 'open_case')
      return { profile: { spatial, obs_file, inputs: { forcing_namelist: namelist } } };
    if (command === 'read_forcing_namelist') return 'forcing namelist';
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
  assert.ok(calls.some((call) => call.command === 'install_example'));
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

test('nested forcing namelists resolve data against the case working directory', async () => {
  const correct = '/cases/site/Observation/right.nc';
  const { call, calls } = fixture({
    namelist: 'config/forcing.nml',
    dir: 'met/',
    sites: [
      {
        met_file: '/cases/site/config/met/CN-Cng_2010_Met.nc',
        obs_file: '/cases/site/config/Observation/wrong.nc',
      },
      { met_file: '/cases/site/met/CN-Cng_2010_Met.nc', obs_file: correct },
    ],
  });
  assert.equal(await defaultObservation('/cases/site', call), correct);
  assert.deepEqual(calls.find((call) => call.command === 'read_forcing_namelist')?.args, {
    path: '/cases/site/config/forcing.nml',
  });
  assert.deepEqual(calls.at(-1)?.args, {
    dir: '/cases/site/Sitedata',
    forcingDir: '/cases/site/met',
    quick: true,
  });
});

test('UNC forcing directories and namelists stay absolute', async () => {
  const { call, calls } = fixture({
    namelist: '\\\\server\\share\\config\\forcing.nml',
    dir: '\\\\server\\share\\Forcing\\',
    sites: [
      {
        met_file: '\\\\server\\share\\Forcing\\CN-Cng_2010_Met.nc',
        obs_file: '\\\\server\\share\\Observation\\flux.nc',
      },
    ],
  });
  assert.equal(
    await defaultObservation('C:\\cases\\site', call),
    '\\\\server\\share\\Observation\\flux.nc',
  );
  assert.deepEqual(calls.find((call) => call.command === 'read_forcing_namelist')?.args, {
    path: '//server/share/config/forcing.nml',
  });
  assert.deepEqual(calls.at(-1)?.args, {
    dir: '//server/share/Sitedata',
    forcingDir: '//server/share/Forcing',
    quick: true,
  });
});

test('filesystem, drive, and UNC share roots remain rooted during dataset scanning', async () => {
  for (const [dir, root] of [
    ['/', '/'],
    ['C:\\', 'C:/'],
    ['\\\\server\\share\\', '//server/share'],
  ]) {
    const base = root.endsWith('/') ? root : `${root}/`;
    const { call, calls } = fixture({
      dir,
      sites: [{ met_file: `${base}CN-Cng_2010_Met.nc`, obs_file: `${base}flux.nc` }],
    });
    assert.equal(await defaultObservation('/cases/site', call), `${base}flux.nc`);
    assert.deepEqual(calls.at(-1)?.args, { dir: `${base}Sitedata`, forcingDir: root, quick: true });
  }
});

test('the observation the backend found is used as is, including server paths', async () => {
  const server =
    '/media/zhwei/data02/zhwei/training2026/PLUMBER2s/Observation/US-MMS_1999-2014_FLUXNET2015_Flux.nc';
  const { call, calls } = fixture({ obs_file: server });
  assert.equal(await defaultObservation('/cases/US-MMS', call), server);
  // 服务器上的路径不能拿到本机去扫描。
  assert.ok(!calls.some((c) => c.command === 'scan_sites'));
});
