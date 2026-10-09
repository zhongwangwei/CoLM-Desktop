#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("bind-knowledge.py")
spec = importlib.util.spec_from_file_location("binding", SCRIPT)
binding = importlib.util.module_from_spec(spec)
spec.loader.exec_module(binding)


class BindingTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        (self.root / "docs").mkdir()
        (self.root / "crates/core/src").mkdir(parents=True)
        self.source = self.root / "crates/core/src/model.rs"
        self.source.write_bytes(b"old source")
        self.document = self.root / "docs" / binding.DOCUMENT
        self.document.write_text("Source: `crates/core/src/model.rs::model`\n", encoding="utf-8")
        self.manifest = self.root / binding.MANIFEST

    def test_explicit_review_required(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root)], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(self.manifest.exists())

    def test_same_size_source_change_detected_without_rebinding(self):
        binding.bind(self.root)
        baseline = self.manifest.read_bytes()
        self.source.write_bytes(b"new source")
        self.assertEqual(binding.check(self.root), {"status": "needs_review", "document_changed": False,
                                                   "changed_sources": ["crates/core/src/model.rs"]})
        self.assertEqual(self.manifest.read_bytes(), baseline)
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root), "--reviewed"], capture_output=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(binding.check(self.root)["status"], "matched")

    def test_document_change_detected_without_rebinding(self):
        binding.bind(self.root)
        baseline = self.manifest.read_bytes()
        self.document.write_text(self.document.read_text().replace("Source", "source"))
        self.assertTrue(binding.check(self.root)["document_changed"])
        self.assertEqual(self.manifest.read_bytes(), baseline)

    def test_missing_source_preserves_existing_baseline(self):
        binding.bind(self.root)
        baseline = self.manifest.read_bytes()
        self.source.unlink()
        with self.assertRaises(ValueError):
            binding.bind(self.root)
        self.assertEqual(self.manifest.read_bytes(), baseline)

    def test_paths_and_symlinks_rejected(self):
        for path in ("crates/../../secret.rs", "crates/.env.rs", "vendor/other/secret.rs", "crates/core/private.key"):
            with self.subTest(path=path):
                self.document.write_text(f"`{path}`")
                with self.assertRaises(ValueError):
                    binding.bind(self.root)
        for path in ("/tmp/source.rs", "../source.rs", "crates/../source.rs", "C:/source.rs"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                binding.regular_file(self.root, path)
        self.source.unlink()
        self.source.symlink_to(self.document)
        self.document.write_text("`crates/core/src/model.rs`")
        with self.assertRaises(ValueError):
            binding.bind(self.root)
        self.assertFalse(self.manifest.exists())

    def test_invalid_manifest_check_fails_without_mutation(self):
        self.manifest.write_text('{"schema": 1, "documents": {"colm-process-knowledge.md": {"sources": "bad"}}}')
        baseline = self.manifest.read_bytes()
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root), "--check"], capture_output=True)
        self.assertEqual(result.returncode, 2)
        self.assertEqual(self.manifest.read_bytes(), baseline)

    def test_manifest_contract_and_no_empty_binding(self):
        result = binding.bind(self.root)
        self.assertEqual(result["schema"], 1)
        self.assertGreater(result["bound_at_unix_ms"], 0)
        self.assertEqual(json.loads(self.manifest.read_text()), result)
        entry = result["documents"][binding.DOCUMENT]
        self.assertEqual(len(entry["document_sha256"]), 64)
        self.assertEqual(len(entry["sources"]["crates/core/src/model.rs"]), 64)
        self.document.write_text("No sources yet")
        with self.assertRaises(ValueError):
            binding.bind(self.root)


if __name__ == "__main__":
    unittest.main()
