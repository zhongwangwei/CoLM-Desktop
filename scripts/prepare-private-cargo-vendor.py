#!/usr/bin/env python3
"""Create a job-private Cargo directory source from pinned locks and a read-only vendor.

Inputs: OBREF_INPUT_CARGO_VENDOR (existing vendor directory), OBREF_JOB_RESULTS.
The runner must enable public network access. No Cargo home or registry is modified.
"""
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import tarfile
import tomllib
import urllib.request

REGISTRY = 'registry+https://github.com/rust-lang/crates.io-index'


def locked_packages(paths):
    packages = {}
    for path in paths:
        for package in tomllib.loads(path.read_text())['package']:
            if 'source' not in package:
                continue
            if package['source'] != REGISTRY:
                raise ValueError(f"Unsupported package source: {package['source']}")
            name, version, checksum = (package[key] for key in ('name', 'version', 'checksum'))
            if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_-]*', name) or not re.fullmatch(r'[0-9][A-Za-z0-9.+_-]*', version):
                raise ValueError('Invalid crate name/version')
            if not re.fullmatch(r'[0-9a-f]{64}', checksum):
                raise ValueError(f'Invalid checksum for {name}-{version}')
            key = (name, version)
            if key in packages and packages[key] != checksum:
                raise ValueError(f'Conflicting lock checksums for {name}-{version}')
            packages[key] = checksum
    return packages


def existing_packages(root):
    packages = {}
    for directory in sorted(root.iterdir()):
        manifest, checksum = directory / 'Cargo.toml', directory / '.cargo-checksum.json'
        if not manifest.is_file() or not checksum.is_file():
            continue
        package = tomllib.loads(manifest.read_text())['package']
        digest = json.loads(checksum.read_text()).get('package')
        packages[(package['name'], package['version'], digest)] = directory.resolve(strict=True)
    return packages


def extract_crate(archive, vendor, name, version, expected):
    with archive.open('rb') as source:
        if hashlib.file_digest(source, 'sha256').hexdigest() != expected:
            raise ValueError(f'Checksum mismatch for {name}-{version}')
    top = f'{name}-{version}'
    with tarfile.open(archive, 'r:gz') as tar:
        members = tar.getmembers()
        seen = set()
        for member in members:
            path = PurePosixPath(member.name)
            if (path.is_absolute() or '..' in path.parts or not path.parts or
                    path.parts[0] != top or not (member.isfile() or member.isdir())):
                raise ValueError(f'Unsafe archive member: {member.name}')
            if path.as_posix() in seen:
                raise ValueError(f'Duplicate archive member: {member.name}')
            seen.add(path.as_posix())
        tar.extractall(vendor, members=members, filter='data')
    crate = vendor / top
    manifest = tomllib.loads((crate / 'Cargo.toml').read_text())['package']
    if (manifest['name'], manifest['version']) != (name, version):
        raise ValueError(f'Unexpected package manifest in {top}')
    files = {}
    for path in sorted(crate.rglob('*')):
        if path.is_file() and path != crate / '.cargo-checksum.json':
            with path.open('rb') as source:
                files[path.relative_to(crate).as_posix()] = hashlib.file_digest(source, 'sha256').hexdigest()
    (crate / '.cargo-checksum.json').write_text(json.dumps({'package': expected, 'files': files}) + '\n')


def main():
    repo = Path(__file__).resolve().parent.parent
    packages = locked_packages([repo / 'Cargo.lock', repo / 'gui/src-tauri/Cargo.lock'])
    existing = existing_packages(Path(os.environ['OBREF_INPUT_CARGO_VENDOR']).resolve(strict=True))
    output = Path(os.environ['OBREF_JOB_RESULTS']).resolve(strict=True) / 'private-cargo-vendor'
    output.mkdir()  # Refuse to mix a new preparation with earlier or incomplete output.
    vendor, archives = output / 'vendor', output / 'archives'
    vendor.mkdir()
    archives.mkdir()
    report = []
    for (name, version), checksum in sorted(packages.items()):
        reused = existing.get((name, version, checksum))
        destination = vendor / f'{name}-{version}'
        if reused:
            destination.symlink_to(reused, target_is_directory=True)
        else:
            archive = archives / f'{name}-{version}.crate'
            url = f'https://static.crates.io/crates/{name}/{name}-{version}.crate'
            with urllib.request.urlopen(url, timeout=120) as response, archive.open('xb') as target:
                shutil.copyfileobj(response, target)
            extract_crate(archive, vendor, name, version, checksum)
        report.append({'name': name, 'version': version, 'checksum': checksum,
                       'reused': str(reused) if reused else None})
        print(f'{"reuse" if reused else "download"}: {name}-{version}', flush=True)
    config = output / 'config.toml'
    config.write_text('[source.crates-io]\nreplace-with = "private-job-vendor"\n\n'
                      '[source.private-job-vendor]\ndirectory = ' + json.dumps(str(vendor)) + '\n')
    (output / 'manifest.json').write_text(json.dumps(report, indent=2) + '\n')
    print(f'Cargo configuration: {config}\nUse cargo --config {config} ... --offline --locked')


if __name__ == '__main__':
    main()
