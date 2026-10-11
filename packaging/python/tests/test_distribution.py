import base64
import csv
import hashlib
import importlib.util
import io
from pathlib import Path, PurePosixPath
import struct
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import zipfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packaging/python"))
import colm_agent

spec = importlib.util.spec_from_file_location("builder", ROOT / "scripts/build-python-wheel.py")
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


def elf(machine=62):
    data = bytearray(64)
    data[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", data, 18, machine)
    return data


class DistributionTests(unittest.TestCase):
    def test_distribution_is_paused_before_reading_or_writing_files(self):
        self.assertFalse(builder.PIP_DISTRIBUTION_ENABLED)
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            with self.assertRaisesRegex(RuntimeError, "temporarily paused"):
                builder.build(root, root / "missing.tar.gz", "x86_64-unknown-linux-gnu", root / "out")
            self.assertFalse((root / "out").exists())
        result = subprocess.run([sys.executable, str(ROOT / "scripts/build-python-wheel.py")], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("temporarily paused", result.stderr)

    def test_launch_preserves_arguments_and_exec(self):
        with patch.object(sys, "argv", ["colm-agent", "chat", "a b", "--model", "x"]), patch.object(colm_agent.os, "execv") as execute:
            colm_agent.agent()
        path = str(colm_agent.resources() / "colm-agent")
        execute.assert_called_once_with(path, [path, "chat", "a b", "--model", "x"])

    def test_windows_child_status_and_interrupt(self):
        with patch.object(colm_agent.os, "name", "nt"), patch.object(colm_agent, "resources", return_value=PurePosixPath("/native")), patch.object(colm_agent.subprocess, "Popen") as popen:
            popen.return_value.wait.side_effect = [KeyboardInterrupt, 23]
            with self.assertRaises(SystemExit) as result:
                colm_agent.cli()
            self.assertEqual(result.exception.code, 23)
            self.assertTrue(popen.call_args.args[0][0].endswith("colm-cli.exe"))

    def test_target_and_version_validation(self):
        self.assertEqual(builder.pep_version("0.3.0-beta.3"), "0.3.0b3")
        self.assertEqual(builder.binary_platform(elf(), "x86_64-unknown-linux-gnu"), "linux_x86_64")
        with self.assertRaises(ValueError):
            builder.binary_platform(elf(183), "x86_64-unknown-linux-gnu")
        data = bytearray(56)
        struct.pack_into("<IIIIIIII", data, 0, 0xfeedfacf, 0x100000C, 0, 2, 1, 24, 0, 0)
        struct.pack_into("<IIIIII", data, 32, 0x32, 24, 1, 13 << 16, 0, 0)
        self.assertEqual(builder.binary_platform(data, "aarch64-apple-darwin"), "macosx_13_0_arm64")

    def test_wheel_integrity_resources_and_modes(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for name in builder.PROGRAMS:
                (root / name).write_bytes(elf())
            source = root / "colm-src.tar.gz"
            docs = b'{"version":"fixture"}'
            with tarfile.open(source, "w:gz") as archive:
                for name, data in {"Cargo.toml": b'[workspace.package]\nversion = "0.3.0-beta.3"\n', "docs/knowledge-sources.json": docs}.items():
                    info = tarfile.TarInfo(name)
                    info.size = len(data)
                    archive.addfile(info, io.BytesIO(data))
            with patch.object(builder, "PIP_DISTRIBUTION_ENABLED", True):
                wheel = builder.build(root, source, "x86_64-unknown-linux-gnu", root / "out")
            self.assertEqual(wheel.name, "colm_agent-0.3.0b3-py3-none-linux_x86_64.whl")
            with zipfile.ZipFile(wheel) as package:
                self.assertEqual(package.read("colm_agent/_native/bin/docs/knowledge-sources.json"), docs)
                self.assertEqual(package.read("colm_agent/_native/bin/engine/colm-src.tar.gz"), source.read_bytes())
                self.assertEqual(package.getinfo("colm_agent/_native/bin/colm-agent").external_attr >> 16 & 0o777, 0o755)
                records = list(csv.reader(io.StringIO(package.read("colm_agent-0.3.0b3.dist-info/RECORD").decode())))
                self.assertEqual(len(records), len(package.namelist()))
                for name, digest, size in records[:-1]:
                    data = package.read(name)
                    self.assertEqual(int(size), len(data))
                    self.assertEqual(digest, "sha256=" + base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode())
