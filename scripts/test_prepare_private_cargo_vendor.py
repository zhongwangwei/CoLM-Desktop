"""Offline regression checks; never downloads or accesses shared Cargo state."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    'prepare_private_cargo_vendor', Path(__file__).with_name('prepare-private-cargo-vendor.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class VendorTests(unittest.TestCase):
    def archive(self, root, extra=None):
        archive = root / 'sample.crate'
        with tarfile.open(archive, 'w:gz') as tar:
            payload = b'[package]\nname="sample"\nversion="1.0.0"\n'
            member = tarfile.TarInfo('sample-1.0.0/Cargo.toml')
            member.size = len(payload)
            tar.addfile(member, io.BytesIO(payload))
            if extra:
                tar.addfile(extra)
        return archive, hashlib.sha256(archive.read_bytes()).hexdigest()

    def test_extract_generates_vendor_checksum_and_reusable_identity(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, digest = self.archive(root)
            module.extract_crate(archive, root / 'vendor', 'sample', '1.0.0', digest)
            checksum = json.loads((root / 'vendor/sample-1.0.0/.cargo-checksum.json').read_text())
            self.assertEqual(checksum['package'], digest)
            self.assertIn('Cargo.toml', checksum['files'])
            self.assertIn(('sample', '1.0.0', digest), module.existing_packages(root / 'vendor'))

    def test_rejects_traversal_wrong_root_and_archive_links_before_extraction(self):
        for name, kind in [('sample-1.0.0/../escape', tarfile.REGTYPE),
                           ('other/Cargo.toml', tarfile.REGTYPE),
                           ('/absolute', tarfile.REGTYPE),
                           ('sample-1.0.0/link', tarfile.SYMTYPE),
                           ('sample-1.0.0/hardlink', tarfile.LNKTYPE)]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                member = tarfile.TarInfo(name)
                member.type, member.linkname = kind, '/outside'
                archive, digest = self.archive(root, member)
                with self.assertRaises(ValueError):
                    module.extract_crate(archive, root / 'vendor', 'sample', '1.0.0', digest)
                self.assertFalse((root / 'vendor').exists())

    def test_rejects_unverified_archive(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            archive, _ = self.archive(root)
            with self.assertRaises(ValueError):
                module.extract_crate(archive, root / 'vendor', 'sample', '1.0.0', '0' * 64)

    def test_lock_union_rejects_conflicting_checksum_and_unknown_registry(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            first, second = root / 'a.lock', root / 'b.lock'
            content = ('[[package]]\nname="sample"\nversion="1.0.0"\n'
                       f'source="{module.REGISTRY}"\nchecksum="{"a" * 64}"\n')
            first.write_text(content)
            second.write_text(content)
            self.assertEqual(len(module.locked_packages([first, second])), 1)
            second.write_text(content.replace('a' * 64, 'b' * 64))
            with self.assertRaises(ValueError):
                module.locked_packages([first, second])
            second.write_text(content.replace(module.REGISTRY, 'registry+https://example.com'))
            with self.assertRaises(ValueError):
                module.locked_packages([second])


if __name__ == '__main__':
    unittest.main()
