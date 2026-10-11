# Executed by the existing SSH transport; no server-side installation or imports beyond stdlib.
import json
import os
import re
import stat
import sys

root, action, snapshot, token = sys.argv[1:5]


def safe(path):
    if not path.startswith('/') or path == '/' or any(p in ('.', '..', '') for p in path.split('/')[1:]):
        raise ValueError('unsafe engine resource path')
    current = '/'
    for part in path.split('/')[1:]:
        current = os.path.join(current, part)
        if os.path.lexists(current) and stat.S_ISLNK(os.lstat(current).st_mode):
            raise ValueError('symbolic links are not allowed: ' + current)
    return path


safe(root)
lock = root + '/.engine-lifecycle-lock'
safe(lock)
if action == 'acquire':
    os.makedirs(root, exist_ok=True)
    os.mkdir(lock)
    with open(lock + '/owner', 'x') as out:
        out.write(token)
    print('{}')
    sys.exit(0)
if action in ('release', 'clean'):
    safe(lock + '/owner')
    with open(lock + '/owner') as source:
        if source.read() != token:
            raise ValueError('engine lifecycle lock ownership changed')
if action == 'release':
    os.unlink(lock + '/owner')
    os.rmdir(lock)
    print('{}')
    sys.exit(0)
if action not in ('list', 'clean'):
    raise ValueError('unknown engine resource operation')
if action == 'clean' and not re.fullmatch('[0-9a-f]{16}', snapshot):
    raise ValueError('invalid engine snapshot')

blockers = []
references = {}
study_roots = set()
jobs = safe(root + '/jobs')
if os.path.exists(jobs):
    for name in os.listdir(jobs):
        try:
            script = safe(jobs + '/' + name + '/script.sh')
            with open(script) as source:
                text = source.read(2 * 1024 * 1024 + 1)
            matches = re.findall(r'^# colm-engine-snapshot: ((?:pre-)?[0-9a-f]{16})$', text, re.M)
            if len(text) > 2 * 1024 * 1024 or len(matches) != 1:
                raise ValueError('missing or ambiguous engine reference')
            references.setdefault(matches[0], []).append('Referenced by job ' + name)
            study_markers = re.findall(r'^# colm-study-root: (.+)$', text, re.M)
            if len(study_markers) > 1:
                raise ValueError('ambiguous study reference')
            if study_markers:
                study = safe(json.loads(study_markers[0]))
                if os.path.dirname(study) not in (root + '/s', root + '/studies'):
                    raise ValueError('study reference is outside the resource root')
                study_roots.add(study)
        except (OSError, ValueError) as error:
            blockers.append('Cannot verify job ' + name + ': ' + str(error))
# Old studies have no authoritative server-side engine reference. Preserve them all.
for name in ('s', 'studies'):
    directory = safe(root + '/' + name)
    if os.path.exists(directory):
        for child in os.listdir(directory):
            study = safe(directory + '/' + child)
            if study not in study_roots:
                blockers.append('Cannot verify engine reference for existing study: ' + study)
engines = []
directory = safe(root + '/engine')
if os.path.exists(directory):
    for name in sorted(os.listdir(directory)):
        if not re.fullmatch('(?:pre-)?[0-9a-f]{16}', name):
            continue
        reasons = list(blockers) + references.get(name, [])
        if name.startswith('pre-'):
            reasons.append('Prebuilt engine cache is preserved')
        binaries = []
        try:
            target = safe(directory + '/' + name)
            if not os.path.isdir(target):
                raise ValueError('engine snapshot is not a directory')
            for binary in ('colm-cli', 'mksrfdata-rs', 'mkinidata-rs', 'colm-rs'):
                path = safe(target + '/bin/' + binary)
                binaries.append({'name': binary, 'ready': os.path.isfile(path) and os.access(path, os.X_OK)})
        except (OSError, ValueError) as error:
            reasons.append(str(error))
        engines.append({'id': name, 'ready': len(binaries) == 4 and all(b['ready'] for b in binaries),
                        'binaries': binaries, 'protected_reasons': reasons, 'can_clean': not reasons})
if action == 'clean':
    item = next((item for item in engines if item['id'] == snapshot), None)
    if item is None or not item['can_clean']:
        print(json.dumps({'refused': item['protected_reasons'] if item else ['snapshot not found']}))
        sys.exit(0)
    target = safe(directory + '/' + snapshot)
    # Refuse any descendant symlink rather than relying on recursive-delete behavior.
    for current, dirs, files in os.walk(target, followlinks=False):
        for name in dirs + files:
            safe(current + '/' + name)
    import shutil
    shutil.rmtree(target)
    print(json.dumps({'removed': snapshot}))
else:
    print(json.dumps({'engines': engines, 'cleanup_blockers': blockers}))
