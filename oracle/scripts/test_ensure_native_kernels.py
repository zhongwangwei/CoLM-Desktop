import hashlib
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import os
import ensure_native_kernels as kernels

from ensure_native_kernels import current, source_fingerprint


class NativeKernelTests(unittest.TestCase):
    def test_cache_requires_source_platform_capabilities_and_every_binary(self):
        with tempfile.TemporaryDirectory() as temp:
            directory = Path(temp)
            manifest = dict(schema=1, preset='latlon', platform='Darwin-arm64',
                            source_sha256='source', build_profile='production',
                            macros=['GRIDBASED', 'LULC_IGBP', 'USEMPI', 'FLAT_SPMD', 'GridRiverLakeFlow'], sha256={})
            for name in ['mksrfdata', 'mkinidata', 'colm']:
                path = directory / (name + '.x')
                path.write_bytes(name.encode())
                path.chmod(0o755)
                manifest['sha256'][name] = hashlib.sha256(path.read_bytes()).hexdigest()
            def save():
                (directory / 'manifest.json').write_text(json.dumps(manifest))
            save()
            self.assertTrue(current(directory, 'latlon', 'source', 'Darwin-arm64'))
            self.assertFalse(current(directory, 'latlon', 'new-source', 'Darwin-arm64'))
            self.assertFalse(current(directory, 'latlon', 'source', 'Linux-x86_64'))
            for bad in ['SinglePoint', 'LULC_USGS', 'CROP', 'HYPERSPECTRAL']:
                manifest['macros'].append(bad)
                save()
                self.assertFalse(current(directory, 'latlon', 'source', 'Darwin-arm64'))
                manifest['macros'].pop()
            save()
            (directory / 'colm.x').write_text('wrong binary')
            self.assertFalse(current(directory, 'latlon', 'source', 'Darwin-arm64'))
            (directory / 'colm.x').unlink()
            self.assertFalse(current(directory, 'latlon', 'source', 'Darwin-arm64'))

    def test_all_cache_hits_still_check_for_source_changes(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'oracle/scripts').mkdir(parents=True)
            (root / 'oracle/scripts/kernel-presets.config').write_text('default latlon')
            with patch.object(kernels, 'ROOT', root), patch.object(kernels.sys, 'argv', ['ensure']), \
                    patch.object(kernels, 'source_fingerprint', side_effect=['before', 'after']), \
                    patch.object(kernels, 'current', return_value=True), \
                    patch.object(kernels.subprocess, 'check_output', side_effect=['Darwin', 'arm64']), \
                    patch.object(kernels.subprocess, 'run') as run:
                with self.assertRaisesRegex(SystemExit, 'source changed during cache validation'):
                    kernels.main()
                run.assert_not_called()

    def test_all_cache_hits_still_stage_and_run_bundled_mpi(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root / 'oracle/scripts').mkdir(parents=True)
            (root / 'oracle/scripts/kernel-presets.config').write_text('latlon')
            launcher = root / 'kernels/_runtime/bin/mpiexec'
            launcher.parent.mkdir(parents=True)
            launcher.write_text('launcher')
            launcher.chmod(0o755)
            with patch.object(kernels, 'ROOT', root), patch.object(kernels.sys, 'argv', ['ensure']), \
                    patch.object(kernels, 'source_fingerprint', return_value='source'), \
                    patch.object(kernels, 'current', return_value=True), \
                    patch.object(kernels.subprocess, 'check_output', side_effect=['Darwin', 'arm64']), \
                    patch.object(kernels.subprocess, 'run') as run:
                kernels.main()
                self.assertEqual(run.call_args_list[0].args[0][1], str(root / 'oracle/scripts/stage_mpi_runtime.sh'))
                self.assertEqual(run.call_args_list[1].args[0], [str(launcher), '-n', '1', 'hostname'])
                self.assertEqual(run.call_args_list[1].kwargs['env']['OPAL_PREFIX'], str(root / 'kernels/_runtime'))

    def run_main(self, root, presets, current, environment, run=None):
        """main() for `presets`, with `current(directory, preset, fingerprint, platform)` deciding;
        returns every command it ran."""
        (root / 'oracle/scripts').mkdir(parents=True, exist_ok=True)
        (root / 'oracle/scripts/kernel-presets.config').write_text(' '.join(presets))
        launcher = root / 'kernels/_runtime/bin/mpiexec'
        launcher.parent.mkdir(parents=True, exist_ok=True)
        launcher.write_text('launcher')
        launcher.chmod(0o755)
        for preset in presets:
            (root / 'kernels' / preset).mkdir(parents=True, exist_ok=True)
            (root / 'kernels' / preset / 'manifest.json').write_text(json.dumps({'source_sha256': 'old'}))
        with patch.object(kernels, 'ROOT', root), patch.object(kernels.sys, 'argv', ['ensure']), \
                patch.dict(os.environ, environment), \
                patch.object(kernels, 'source_fingerprint', return_value='source'), \
                patch.object(kernels, 'current', side_effect=current), \
                patch.object(kernels.subprocess, 'check_output', side_effect=['Darwin', 'arm64']), \
                patch.object(kernels.subprocess, 'run', side_effect=run) as mock:
            kernels.main()
        return [call.args[0] for call in mock.call_args_list]

    def test_stale_kernels_are_bundled_without_compiling_unless_asked(self):
        # `latlon` was built from older Fortran source: valid against what its manifest records only.
        stale = lambda directory, preset, fingerprint, platform: preset == 'default' or fingerprint == 'old'
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            commands = self.run_main(root, ['default', 'latlon'], stale, {'COLM_BUILD_KERNELS': ''})
            self.assertFalse(any('build_kernel.sh' in ' '.join(command) for command in commands))
            self.assertEqual(commands[0][1], str(root / 'oracle/scripts/stage_mpi_runtime.sh'))

    def test_a_missing_kernel_stops_the_bundle_instead_of_compiling(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(SystemExit, 'COLM_BUILD_KERNELS=1'):
                self.run_main(Path(temp), ['latlon'], lambda *_: False, {'COLM_BUILD_KERNELS': ''})

    def test_asking_for_kernels_rebuilds_the_stale_ones(self):
        built = set()
        def record(command, **_):
            if command[1].endswith('build_kernel.sh'):
                built.add(command[2])
        current = lambda directory, preset, fingerprint, platform: preset in built or preset == 'default'
        with tempfile.TemporaryDirectory() as temp:
            commands = self.run_main(Path(temp), ['default', 'latlon'], current,
                                     {'COLM_BUILD_KERNELS': '1'}, run=record)
        self.assertEqual([c[2] for c in commands if c[1].endswith('build_kernel.sh')], ['latlon'])

    def test_editing_this_script_does_not_make_every_kernel_stale(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ['oracle/scripts/build_kernel.sh', 'oracle/scripts/ensure_native_kernels.py',
                         'oracle/scripts/kernel-presets.config', 'vendor/CoLM202X/main/model.F90']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            before = source_fingerprint(root)
            (root / 'oracle/scripts/ensure_native_kernels.py').write_text('a new bundling policy')
            self.assertEqual(before, source_fingerprint(root))
            (root / 'oracle/scripts/build_kernel.sh').write_text('a new compile step')
            self.assertNotEqual(before, source_fingerprint(root))

    def test_fingerprint_works_without_git_and_tracks_uncommitted_source(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            for name in ['oracle/scripts/build_kernel.sh', 'oracle/scripts/ensure_native_kernels.py',
                         'oracle/scripts/kernel-presets.config', 'vendor/CoLM202X/main/model.F90']:
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(name)
            before = source_fingerprint(root)
            for variable in kernels.BUILD_ENV:
                with patch.dict(os.environ, {variable: 'changed-build-input'}):
                    self.assertNotEqual(before, source_fingerprint(root), variable)
            (root / 'vendor/CoLM202X/main/model.o').write_text('ignored build product')
            self.assertEqual(before, source_fingerprint(root))
            (root / 'vendor/CoLM202X/main/model.F90').write_text('changed locally')
            self.assertNotEqual(before, source_fingerprint(root))


if __name__ == '__main__':
    unittest.main()
