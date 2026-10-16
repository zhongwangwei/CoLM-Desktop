#!/usr/bin/env python3
"""Reuse declared GTK/WebKit inputs and unpack Xvfb into one runner job's results.

Run with run-7920 --network --input-remote UI_DEPS=<existing sysroot>.
No apt installation, maintainer scripts, sudo, or shared-directory writes occur.
--plan only reads package metadata; it creates no files and downloads nothing.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import subprocess
import urllib.parse
import urllib.request


def package_plan(output):
    packages = []
    for line in output.splitlines():
        if not line.startswith("'"):
            continue
        url, filename, size, digest = shlex.split(line)
        parsed = urllib.parse.urlsplit(url)
        if parsed.hostname not in {
            'archive.ubuntu.com', 'security.ubuntu.com',
            'deb.debian.org', 'security.debian.org',
        } or parsed.username or parsed.password or parsed.port:
            raise ValueError(f'Not an official public package URL: {url}')
        if parsed.scheme not in {'http', 'https'} or not parsed.path.endswith('.deb'):
            raise ValueError(f'Invalid package URL: {url}')
        algorithm, expected = digest.split(':', 1)
        if algorithm not in {'SHA256', 'SHA512'}:
            raise ValueError(f'Package lacks a strong archive digest: {filename}')
        if Path(filename).name != filename or not filename.endswith('.deb'):
            raise ValueError(f'Invalid package filename: {filename}')
        packages.append(dict(url=parsed._replace(scheme='https').geturl(), filename=filename,
                             size=int(size), algorithm=algorithm.lower(), digest=expected))
    if len(packages) != 2:
        raise ValueError('Expected exactly xvfb and xserver-common from apt metadata')
    return packages


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--plan', action='store_true')
    args = parser.parse_args()
    deps = Path(os.environ['OBREF_INPUT_UI_DEPS']).resolve(strict=True)
    env = deps / 'env.sh'
    if not env.is_file():
        raise ValueError(f'Missing existing GTK/WebKit environment: {env}')
    # The supplied sysroot has relocated .pc files; keep its original declared path.
    subprocess.run(['bash', '-c', 'source "$1"; pkg-config --exists gtk+-3.0 webkit2gtk-4.1',
                    'check-gui-deps', str(env)], check=True)
    plan = subprocess.check_output(
        ['apt-get', '--print-uris', 'download', 'xvfb', 'xserver-common'], text=True)
    packages = package_plan(plan)
    if args.plan:
        print(json.dumps({'existing_sysroot': str(deps), 'packages': packages}, indent=2))
        return

    results = Path(os.environ['OBREF_JOB_RESULTS']).resolve(strict=True)
    destination = results / 'gui-linux-deps'
    destination.mkdir()  # Never overwrite an earlier preparation or a symlink.
    archives = destination / 'packages'
    archives.mkdir()
    sysroot = destination / 'sysroot'
    sysroot.mkdir()
    (destination / 'apt-plan.txt').write_text(plan)
    (destination / 'manifest.json').write_text(json.dumps(packages, indent=2) + '\n')
    for package in packages:
        target = archives / package['filename']
        digest = hashlib.new(package['algorithm'])
        count = 0
        with urllib.request.urlopen(package['url'], timeout=120) as response, target.open('xb') as output:
            if urllib.parse.urlsplit(response.url).hostname != urllib.parse.urlsplit(package['url']).hostname:
                raise ValueError('Package download redirected away from the official archive')
            while chunk := response.read(1024 * 1024):
                count += len(chunk)
                if count > package['size']:
                    raise ValueError(f'Archive exceeds its declared size: {target.name}')
                digest.update(chunk)
                output.write(chunk)
        if count != package['size'] or digest.hexdigest() != package['digest']:
            raise ValueError(f'Archive size/digest mismatch: {target.name}')
        subprocess.run(['dpkg-deb', '--extract', str(target), str(sysroot)], check=True)

    environment = destination / 'env.sh'
    environment.write_text(
        '# Generated for this private job; source before Cargo and native GUI checks.\n'
        f'source {shlex.quote(str(env))}\n'
        f'export PATH={shlex.quote(str(sysroot / "usr/bin"))}:"$PATH"\n'
        'export GDK_BACKEND=x11\n'
        'export LIBGL_ALWAYS_SOFTWARE=1\n')
    xvfb = sysroot / 'usr/bin/Xvfb'
    linkage = subprocess.check_output(['ldd', str(xvfb)], text=True)
    (destination / 'xvfb-ldd.txt').write_text(linkage)
    if 'not found' in linkage:
        raise RuntimeError('Xvfb still has missing host libraries; see xvfb-ldd.txt')
    print(f'Prepared {environment}; source it before starting Xvfb. No display has been started.')


if __name__ == '__main__':
    main()
