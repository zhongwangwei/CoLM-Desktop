#!/usr/bin/env python3
"""Check the native kernels before a production desktop bundle; build them only when asked.

A local bundle does not compile Fortran: kernels built from older Fortran source or build settings
are bundled with a warning, and a missing or unusable kernel stops the bundle. COLM_BUILD_KERNELS=1
builds every missing or stale kernel first (release.yml sets it, so a release never ships a stale
kernel).
"""
import shutil
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
PROGRAMS = ('mksrfdata', 'mkinidata', 'colm')
# Makeoptions' configurable variables and GNU make's command/environment overrides.
BUILD_ENV = ('NETCDF_PREFIX', 'NETCDF_INC', 'NETCDF_LIB', 'NETCDF_FLIBS',
             'BLAS_LIBS', 'LDFLAGS', 'NF_CONFIG', 'NC_CONFIG', 'PKG_CONFIG',
             'PKG_CONFIG_PATH', 'PKG_CONFIG_LIBDIR', 'GFORTRAN', 'FC', 'FCMP',
             'CC', 'FF', 'FFLAGS', 'CFLAGS', 'CPPFLAGS', 'FOPTS', 'FOPTS_COMMON',
             'FOPTS_PRODUCTION', 'FOPTS_DEBUG', 'INCLUDE_DIR', 'LFLAGS',
             'MAKEFLAGS', 'GNUMAKEFLAGS', 'MAKEOVERRIDES', 'MFLAGS',
             'CPATH', 'C_INCLUDE_PATH', 'LIBRARY_PATH', 'SDKROOT',
             'MACOSX_DEPLOYMENT_TARGET', 'OMPI_FC', 'OMPI_CC', 'DMPI', 'DCDF', 'DATM', 'MOD_CMD',
             'RM', 'CP', 'GCC_VERSION', 'IS_GCC_ABOVE_10', 'GFORTRAN_MAJOR')


def source_fingerprint(root):
    # Not this script: it only decides whether to build, and counting it made a change of bundling
    # policy turn all fifteen kernels stale (2026-10-19).
    paths = [root / 'oracle/scripts/build_kernel.sh', root / 'oracle/scripts/kernel-presets.config']
    for directory in ['vendor/CoLM202X', 'oracle/scripts/makeoptions']:
        paths.extend(path for path in (root / directory).rglob('*')
                     if path.is_file() and not any(part in {'.git', '.bld', 'lib'} for part in path.parts)
                     and (path.suffix.lower() in {'.f90', '.f95', '.for', '.f', '.c', '.cpp', '.h', '.hpp', '.inc', '.bash', '.sh', '.mk', '.cmake'}
                          or path.name.startswith(('Makefile', 'Makeoptions'))))
    digest = hashlib.sha256()
    for path in sorted(set(paths)):
        digest.update(path.relative_to(root).as_posix().encode() + b'\0')
        digest.update(path.read_bytes())
        digest.update(b'\0')
    for key, default in [('COLM_KERNEL_PROFILE', 'production'), ('COLM_SPMD_MODE', 'flat'),
                         ('COLM_KERNEL_VECTORIZE', ''), ('COLM_KERNEL_EXTRA_FFLAGS', '')]:
        digest.update(f'{key}={os.environ.get(key, default)}\0'.encode())
    for key in BUILD_ENV:
        digest.update(f'{key}={os.environ.get(key, chr(1))}\0'.encode())
    return digest.hexdigest()


def current(directory, preset, fingerprint, platform):
    try:
        manifest = json.loads((directory / 'manifest.json').read_text())
        if (manifest['schema'] != 1 or manifest['preset'] != preset
                or manifest['platform'] != platform
                or manifest.get('source_sha256') != fingerprint
                or manifest.get('build_profile') != 'production'):
            return False
        grid = preset.split('-')[0]
        expected_grid = {'latlon': 'GRIDBASED', 'unstructured': 'UNSTRUCTURED',
                         'catchment': 'CATCHMENT'}.get(grid, 'SinglePoint')
        macros = set(manifest['macros'])
        if macros.intersection({'SinglePoint', 'GRIDBASED', 'UNSTRUCTURED', 'CATCHMENT'}) != {expected_grid}:
            return False
        if expected_grid != 'SinglePoint':
            routing = 'CatchLateralFlow' if expected_grid == 'CATCHMENT' else 'GridRiverLakeFlow'
            if not {'USEMPI', 'FLAT_SPMD', routing}.issubset(macros):
                return False
        expected_lulc = 'LULC_USGS' if 'usgs' in preset else 'LULC_IGBP'
        if macros.intersection({'LULC_USGS', 'LULC_IGBP'}) != {expected_lulc}:
            return False
        if ('CROP' in macros) != ('crop' in preset) or ('HYPERSPECTRAL' in macros) != preset.endswith('-hyper'):
            return False
        suffix = '.exe' if platform.startswith(('MINGW', 'MSYS')) else '.x'
        return all(os.access(directory / (name + suffix), os.X_OK)
                   and hashlib.sha256((directory / (name + suffix)).read_bytes()).hexdigest()
                   == manifest['sha256'][name] for name in PROGRAMS)
    except (OSError, ValueError, KeyError, TypeError):
        return False


def recorded_source(directory):
    try:
        return json.loads((directory / 'manifest.json').read_text()).get('source_sha256')
    except (OSError, ValueError, AttributeError):
        return None


def main():
    fingerprint = source_fingerprint(ROOT)
    if sys.argv[1:] == ['--fingerprint']:
        print(fingerprint)
        return
    if sys.argv[1:]:
        raise SystemExit('usage: ensure_native_kernels.py [--fingerprint]')
    if os.environ.get('COLM_KERNEL_PROFILE', 'production') != 'production' or os.environ.get('COLM_SPMD_MODE', 'flat') != 'flat':
        raise SystemExit('desktop production bundles require production kernels with flat SPMD')
    platform = '-'.join(subprocess.check_output(['uname', flag], text=True).strip() for flag in ['-s', '-m'])
    presets = (ROOT / 'oracle/scripts/kernel-presets.config').read_text().split()
    build = os.environ.get('COLM_BUILD_KERNELS') == '1'
    stale = []
    for preset in presets:
        directory = ROOT / 'kernels' / preset
        if current(directory, preset, fingerprint, platform):
            print(f'validated native kernel {preset}', flush=True)
            continue
        if not build:
            # Complete and consistent with the source it records, just not today's source.
            if current(directory, preset, recorded_source(directory), platform):
                stale.append(preset)
                continue
            raise SystemExit(f'native kernel {preset} is missing or unusable; build it with '
                             f'COLM_BUILD_KERNELS=1 (or oracle/scripts/build_kernel.sh {preset})')
        print(f'building missing or stale native kernel {preset}', flush=True)
        subprocess.run(['bash', str(ROOT / 'oracle/scripts/build_kernel.sh'), preset], cwd=ROOT, check=True)
        if source_fingerprint(ROOT) != fingerprint or not current(directory, preset, fingerprint, platform):
            raise SystemExit(f'kernel {preset} did not validate against the current source; rebuild before packaging')

    if source_fingerprint(ROOT) != fingerprint:
        raise SystemExit('kernel source changed during cache validation; rebuild before packaging')
    subprocess.run(['bash', str(ROOT / 'oracle/scripts/stage_mpi_runtime.sh'), str(ROOT / 'kernels')],
                   cwd=ROOT, check=True)
    runtime = ROOT / 'kernels/_runtime'
    launcher = runtime / 'bin' / ('mpiexec.exe' if platform.startswith(('MINGW', 'MSYS')) else 'mpiexec')
    if not os.access(launcher, os.X_OK):
        raise SystemExit('bundled MPI launcher is missing or not executable')
    environment = os.environ.copy()
    for key in ['OPAL_PREFIX', 'PRTE_PREFIX', 'PMIX_PREFIX']:
        environment[key] = str(runtime)
    environment['OMPI_MCA_component_path'] = str(runtime / 'lib/openmpi')
    for key, directory in [('PATH', runtime / 'bin'), ('LD_LIBRARY_PATH', runtime / 'lib'),
                           ('DYLD_LIBRARY_PATH', runtime / 'lib'), ('DYLD_FALLBACK_LIBRARY_PATH', runtime / 'lib')]:
        environment[key] = str(directory) + os.pathsep + environment.get(key, '')
    try:
        subprocess.run([str(launcher), '-n', '1', 'hostname'], env=environment, check=True, timeout=30)
    except subprocess.CalledProcessError:
        # Open MPI that cannot load its components does not even know `-n`; show where it looked.
        info = shutil.which('ompi_info')
        if info:
            for key in ['prefix', 'pkglibdir', 'datadir']:
                found = subprocess.run([info, '--path', key, '--parsable'], env=environment,
                                       capture_output=True, text=True).stdout.strip()
                print(f'bundled MPI {key}: {found}', flush=True)
                path = found.split(':', 2)[-1]
                if key == 'pkglibdir':
                    print(f'  exists={os.path.isdir(path)} entries={len(os.listdir(path)) if os.path.isdir(path) else 0}',
                          flush=True)
        raise
    if source_fingerprint(ROOT) != fingerprint or any(
            not current(ROOT / 'kernels' / preset, preset,
                        recorded_source(ROOT / 'kernels' / preset) if preset in stale else fingerprint,
                        platform) for preset in presets):
        raise SystemExit('native kernels changed during MPI staging; rebuild before packaging')
    if stale:
        print(f'WARNING: bundling {len(stale)} native kernel(s) built from older Fortran source or '
              f'build settings, not rebuilt: {" ".join(stale)}. Set COLM_BUILD_KERNELS=1 to rebuild '
              f'them.', flush=True)


if __name__ == '__main__':
    main()
