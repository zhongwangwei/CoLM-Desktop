"""Offline checks for the private GUI dependency package manifest."""
import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location(
    'gui_linux_private_deps', Path(__file__).with_name('gui-linux-private-deps.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class PackagePlanTests(unittest.TestCase):
    def plan(self, host='archive.ubuntu.com', digest='SHA512:abc', filename='xvfb.deb'):
        return (f"'http://{host}/ubuntu/xvfb.deb' {filename} 12 {digest}\n"
                "'http://archive.ubuntu.com/ubuntu/common.deb' common.deb 8 SHA512:def\n")

    def test_official_packages_use_https_and_keep_archive_digest(self):
        plan = module.package_plan(self.plan())
        self.assertEqual(plan[0]['url'], 'https://archive.ubuntu.com/ubuntu/xvfb.deb')
        self.assertEqual(plan[0]['algorithm'], 'sha512')
        self.assertEqual(plan[0]['size'], 12)

    def test_reject_untrusted_sources_weak_hashes_and_paths(self):
        for plan in [self.plan(host='example.com'), self.plan(digest='MD5:abc'),
                     self.plan(filename='../outside.deb'), '']:
            with self.subTest(plan=plan), self.assertRaises(ValueError):
                module.package_plan(plan)


if __name__ == '__main__':
    unittest.main()
