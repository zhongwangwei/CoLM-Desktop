# Executed by the existing SSH transport; no server-side installation or imports beyond stdlib.
import json
import os
import re
import stat
import sys
import time

given, action, snapshot, token = sys.argv[1:5]
if not given.startswith('/') or given == '/' or any(p in ('.', '..', '') for p in given.split('/')[1:]):
    raise ValueError('unsafe engine resource root')
# Symlinks above the root are normal on servers ($HOME, /scratch, /data); resolve them once and only refuse
# symlinks inside the root, where they could widen what cleanup removes.
if action == 'acquire':
    os.makedirs(given, exist_ok=True)
root = os.path.realpath(given)


def safe(path):
    if path == given or path.startswith(given + '/'):
        path = root + path[len(given):]
    if not path.startswith('/') or any(p in ('.', '..', '') for p in path.split('/')[1:]):
        raise ValueError('unsafe engine resource path')
    if path != root and not path.startswith(root + '/'):
        raise ValueError('path is outside the engine resource root: ' + path)
    current = root
    for part in path[len(root):].split('/')[1:]:
        current = current + '/' + part
        if os.path.lexists(current) and stat.S_ISLNK(os.lstat(current).st_mode):
            raise ValueError('symbolic links are not allowed: ' + current)
    return path


lock = safe(root + '/.engine-lifecycle-lock')
leases_dir = safe(root + '/.engine-leases')


def leases():
    """Submissions in progress, or interrupted before they could remove their record."""
    if not os.path.isdir(leases_dir):
        return []
    now = time.time()
    out = []
    for name in sorted(os.listdir(leases_dir)):
        path = safe(leases_dir + '/' + name)
        out.append({'id': name, 'age_s': int(now - os.lstat(path).st_mtime)})
    return out


def lease_text(items):
    return ', '.join('%s (%d min)' % (item['id'][:12], item['age_s'] // 60) for item in items)


if action == 'acquire':
    os.mkdir(lock)
    with open(lock + '/owner', 'x') as out:
        out.write(token)
    active = leases()
    if active:
        os.unlink(lock + '/owner')
        os.rmdir(lock)
        raise ValueError('submissions are in progress or were interrupted: ' + lease_text(active))
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
if action == 'recover':
    # Human-confirmed: drop submission records and a cleanup lock left behind longer than `token` seconds.
    max_age = int(token)
    if max_age < 600:
        raise ValueError('refusing to recover records younger than 10 minutes')
    removed, kept = [], []
    for item in leases():
        if item['age_s'] >= max_age:
            os.unlink(safe(leases_dir + '/' + item['id']))
            removed.append(item['id'])
        else:
            kept.append(item['id'])
    lock_removed = False
    if os.path.isdir(lock) and time.time() - os.lstat(lock).st_mtime >= max_age:
        for name in os.listdir(lock):
            os.unlink(safe(lock + '/' + name))
        os.rmdir(lock)
        lock_removed = True
    print(json.dumps({'removed_leases': removed, 'kept_leases': kept, 'removed_lock': lock_removed}))
    sys.exit(0)
if action not in ('list', 'clean'):
    raise ValueError('unknown engine resource operation')
if action == 'clean' and not re.fullmatch('[0-9a-f]{16}', snapshot):
    raise ValueError('invalid engine snapshot')

blockers = []
active_leases = leases()
if active_leases:
    blockers.append('Submissions in progress or interrupted: ' + lease_text(active_leases))
if action == 'list' and os.path.isdir(lock):
    blockers.append('Engine cleanup in progress or interrupted (lock since %d min)' % (int(time.time() - os.lstat(lock).st_mtime) // 60))
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
    print(json.dumps({'engines': engines, 'cleanup_blockers': blockers, 'leases': active_leases}))
