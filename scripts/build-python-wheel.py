#!/usr/bin/env python3
"""Python distribution is paused; native desktop and CLI builds remain available."""
import argparse
import base64
import csv
import hashlib
import io
from pathlib import Path, PurePosixPath
import re
import struct
import tarfile
import zipfile

PIP_DISTRIBUTION_ENABLED = False
PAUSED_MESSAGE = "pip distribution is temporarily paused; use the native desktop app or colm-agent CLI."

PROGRAMS = {
    "colm-agent": "agent", "colm-cli": "cli", "colm-mcp": "mcp",
    "mksrfdata-rs": "surface", "mkinidata-rs": "initialize", "colm-rs": "runtime",
}
TARGETS = {
    "aarch64-apple-darwin": ("macho", 0x100000C, "arm64"),
    "x86_64-apple-darwin": ("macho", 0x1000007, "x86_64"),
    "x86_64-unknown-linux-gnu": ("elf", 62, "x86_64"),
    "aarch64-unknown-linux-gnu": ("elf", 183, "aarch64"),
    "x86_64-pc-windows-msvc": ("pe", 0x8664, "amd64"),
}


def pep_version(version):
    match = re.fullmatch(r"(\d+\.\d+\.\d+)(?:-(alpha|beta|rc)\.(\d+))?", version)
    if not match:
        raise ValueError(f"unsupported release version: {version}")
    return match[1] + ({"alpha": "a", "beta": "b", "rc": "rc"}[match[2]] + match[3] if match[2] else "")


def binary_platform(data, target):
    kind, machine, arch = TARGETS[target]
    if kind == "elf":
        if data[:6] != b"\x7fELF\x02\x01" or struct.unpack_from("<H", data, 18)[0] != machine:
            raise ValueError(f"ELF binary does not match {target}")
        return f"linux_{arch}"
    if kind == "pe":
        if data[:2] != b"MZ":
            raise ValueError("expected a Windows executable")
        offset = struct.unpack_from("<I", data, 60)[0]
        if data[offset:offset + 4] != b"PE\0\0" or struct.unpack_from("<H", data, offset + 4)[0] != machine:
            raise ValueError(f"PE binary does not match {target}")
        return "win_amd64"
    if data[:4] != b"\xcf\xfa\xed\xfe" or struct.unpack_from("<I", data, 4)[0] != machine:
        raise ValueError(f"Mach-O binary does not match {target}")
    ncmds = struct.unpack_from("<I", data, 16)[0]
    offset, minimum = 32, None
    for _ in range(ncmds):
        command, size = struct.unpack_from("<II", data, offset)
        if size < 8 or offset + size > len(data):
            raise ValueError("invalid Mach-O load command")
        if command == 0x32:  # LC_BUILD_VERSION
            platform, minimum = struct.unpack_from("<II", data, offset + 8)
            if platform != 1:
                raise ValueError("expected macOS binary")
        elif command == 0x24:  # LC_VERSION_MIN_MACOSX
            minimum = struct.unpack_from("<I", data, offset + 8)[0]
        offset += size
    if minimum is None:
        raise ValueError("Mach-O binary has no deployment target")
    major, minor = minimum >> 16, (minimum >> 8) & 255
    # Wheel tags use major_0 for macOS >= 11.
    return f"macosx_{major}_{minor if major < 11 else 0}_{arch}"


def build(binaries, source_pack, target, output, kernels=None):
    if not PIP_DISTRIBUTION_ENABLED:
        raise RuntimeError(PAUSED_MESSAGE)
    files = {}
    root = "colm_agent/_native/bin/"
    suffix = ".exe" if "windows" in target else ""
    platforms = []
    for program in PROGRAMS:
        data = (binaries / (program + suffix)).read_bytes()
        platforms.append(binary_platform(data, target))
        files[root + program + suffix] = (data, 0o755)
    platform = max(platforms, key=lambda tag: tuple(int(n) for n in re.findall(r"\d+", tag)))
    source = source_pack.read_bytes()
    files[root + "engine/colm-src.tar.gz"] = (source, 0o644)
    with tarfile.open(fileobj=io.BytesIO(source), mode="r:gz") as archive:
        manifest = archive.extractfile("Cargo.toml").read().decode()
        section = manifest.split("[workspace.package]", 1)[1].split("\n[", 1)[0]
        version = pep_version(re.search(r'^version\s*=\s*"([^"]+)"', section, re.M)[1])
        for member in archive:
            path = PurePosixPath(member.name)
            if path.parts and path.parts[0] == "docs":
                if ".." in path.parts or path.is_absolute():
                    raise ValueError("unsafe document path in source archive")
                if member.isdir():
                    continue
                if not member.isfile():
                    raise ValueError(f"documents must be regular files: {member.name}")
                name = root + path.as_posix()
                if name in files:
                    raise ValueError(f"duplicate document: {member.name}")
                files[name] = (archive.extractfile(member).read(), 0o644)
    if root + "docs/knowledge-sources.json" not in files:
        raise ValueError("source archive lacks version-bound knowledge manifest")
    if kernels is not None:
        for preset in ("default", "usgs", "bgc", "urban", "crop"):
            path = kernels / preset / "manifest.json"
            if path.is_file():
                files[root + f"kernels/{preset}/manifest.json"] = (path.read_bytes(), 0o644)
    package = Path(__file__).resolve().parents[1] / "packaging/python/colm_agent/__init__.py"
    files["colm_agent/__init__.py"] = (package.read_bytes(), 0o644)
    dist = f"colm_agent-{version}.dist-info/"
    metadata = ("Metadata-Version: 2.1\nName: colm-agent\n" + f"Version: {version}\n"
                "Summary: CoLM command-line tools and AI assistant\nRequires-Python: >=3.10\n\n")
    files[dist + "METADATA"] = (metadata.encode(), 0o644)
    files[dist + "WHEEL"] = (f"Wheel-Version: 1.0\nGenerator: colm-wheel\nRoot-Is-Purelib: false\nTag: py3-none-{platform}\n".encode(), 0o644)
    entries = "[console_scripts]\n" + "".join(f"{name} = colm_agent:{function}\n" for name, function in PROGRAMS.items())
    files[dist + "entry_points.txt"] = (entries.encode(), 0o644)
    record = io.StringIO(newline="")
    writer = csv.writer(record, lineterminator="\n")
    for name, (data, _) in sorted(files.items()):
        digest = base64.urlsafe_b64encode(hashlib.sha256(data).digest()).rstrip(b"=").decode()
        writer.writerow([name, "sha256=" + digest, len(data)])
    writer.writerow([dist + "RECORD", "", ""])
    files[dist + "RECORD"] = (record.getvalue().encode(), 0o644)
    output.mkdir(parents=True, exist_ok=True)
    destination = output / f"colm_agent-{version}-py3-none-{platform}.whl"
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as wheel:
        for name, (data, mode) in sorted(files.items()):
            info = zipfile.ZipInfo(name, date_time=(2020, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (0o100000 | mode) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            wheel.writestr(info, data)
    return destination


if __name__ == "__main__":
    if not PIP_DISTRIBUTION_ENABLED:
        raise SystemExit(PAUSED_MESSAGE)
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True, help="directory containing six freshly built binaries")
    parser.add_argument("--source-pack", type=Path, required=True, help="matching colm-cli engine-pack output")
    parser.add_argument("--target", choices=TARGETS, required=True)
    parser.add_argument("--kernels", type=Path, help="optional directory of kernel preset manifests (no Fortran binaries)")
    parser.add_argument("--output", type=Path, default=Path("dist"))
    args = parser.parse_args()
    print(build(args.binaries, args.source_pack, args.target, args.output, args.kernels))
