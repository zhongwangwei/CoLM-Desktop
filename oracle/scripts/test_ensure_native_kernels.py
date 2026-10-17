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
