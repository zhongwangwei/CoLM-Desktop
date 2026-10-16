import fcntl, json, os, pathlib, re, shutil, subprocess, sys, uuid

payload = json.loads(sys.argv[1])
r = payload['request']
action = payload['action']
cli = payload['cli']

def call(args):
    p = subprocess.run([cli] + [str(x) for x in args], text=True, capture_output=True)
    if p.returncode:
        raise RuntimeError(p.stderr.strip() or p.stdout.strip() or 'server case command failed')
    return p.stdout

def flags(args, names):
    for name in names:
        value = r.get(name)
        if value is not None and str(value).strip():
            args.extend(['--' + name.replace('_', '-'), value])
    return args

def path(value):
    if not isinstance(value, str) or not value.startswith('/') or '\x00' in value or '..' in pathlib.PurePosixPath(value).parts:
        raise ValueError('an absolute server path without parent traversal is required')
    return pathlib.Path(value)

if action == 'scan':
    args = flags(['scan'], ['dir', 'forcing_dir'])
    path(r['dir'])
    if r.get('quick'):
        args.extend(['--quick', '1'])
    print(json.dumps(json.loads(call(args))))
    sys.exit(0)

out = path(r['out'])
if out == pathlib.Path('/'):
    raise ValueError('cannot use the server root as a case directory')
marker = out / '.colm-desktop-creation.json'
if action in ('publish', 'abort', 'sync'):
    if action == 'abort' and r.get('incomplete_only'):
        if not out.exists() and not out.is_symlink():
            print(json.dumps({'ok': True}))
            sys.exit(0)
        if out.is_symlink() or marker.is_symlink():
            raise ValueError('case creation ownership changed')
        lock = marker.open('r+')
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        saved = json.load(lock)
        if saved.get('token') != r['token'] or saved.get('files') is not None:
            raise ValueError('server creation may have completed; retry the original settings to recover it')
    if action == 'sync':
        owner = out / '.colm-desktop-server.json'
        identity = json.loads(owner.read_text())
        if owner.is_symlink() or identity != {'schema': 1, 'remote_directory': str(out)}:
            raise ValueError('server case identity changed; refusing to modify it')
    elif marker.is_symlink() or json.loads(marker.read_text())['token'] != r['token']:
        raise ValueError('case creation ownership changed; refusing to modify it')
    if action == 'abort':
        shutil.rmtree(out)
    else:
        files = r['files']
        if 'case.nml' not in files:
            raise ValueError('case.nml is required')
        for check in r.get('checks', []):
            value = path(check['path'])
            if not (value.is_dir() if check['directory'] else value.is_file()):
                raise ValueError('required server input does not exist: ' + str(value))
        for name, text in files.items():
            if pathlib.PurePosixPath(name).name != name or not name.endswith('.nml') or not isinstance(text, str):
                raise ValueError('only flat namelist text can be published')
            if (out / name).is_symlink():
                raise ValueError('refusing to overwrite a symlink')
        staged = []
        applied = []
        preserve_backups = False
        try:
            for name, text in files.items():
                target = out / name
                temporary = out / ('.publish-' + uuid.uuid4().hex)
                temporary.write_text(text)
                backup = None
                if target.exists():
                    backup = out / ('.backup-' + uuid.uuid4().hex)
                    shutil.copyfile(target, backup)
                staged.append((temporary, target, backup))
            for temporary, target, backup in staged:
                os.replace(temporary, target)
                applied.append((target, backup))
        except BaseException as error:
            failures = []
            for target, backup in reversed(applied):
                try:
                    if backup is None:
                        target.unlink(missing_ok=True)
                    else:
                        os.replace(backup, target)
                except BaseException as rollback:
                    failures.append(str(rollback))
            if failures:
                preserve_backups = True
                raise RuntimeError('publish failed: ' + str(error) + '; rollback failed, .backup files retained: ' + '; '.join(failures)) from error
            raise
        finally:
            for temporary, target, backup in staged:
                temporary.unlink(missing_ok=True)
                if backup is not None and not preserve_backups:
                    backup.unlink(missing_ok=True)
        if action == 'publish':
            (out / '.colm-desktop-server.json').write_text(json.dumps({'schema': 1, 'remote_directory': str(out)}))
            marker.unlink()
    print(json.dumps({'ok': True}))
    sys.exit(0)

if action != 'create':
    raise ValueError('unsupported case action')
# A persisted client token makes a lost SSH response recoverable without adopting other cases.
token = r['token']
if not isinstance(token, str) or not re.fullmatch(r'[a-f0-9]{32,64}', token):
    raise ValueError('invalid creation token')
out.parent.mkdir(parents=True, exist_ok=True)
try:
    out.mkdir(parents=False, exist_ok=False)
    lock = marker.open('x+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    lock.write(json.dumps({'token': token}))
    lock.flush()
    os.fsync(lock.fileno())
except FileExistsError:
    if out.is_symlink() or marker.is_symlink():
        raise ValueError('case destination already exists')
    lock = marker.open('r+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    saved = json.load(lock)
    if saved['token'] != token:
        raise ValueError('case destination belongs to another creation')
    if saved.get('files') is not None:
        print(json.dumps({'remote_dir': str(out), 'token': token, 'files': saved['files']}))
        sys.exit(0)
    # The same interrupted creator owns this directory and no process holds its lock.
    for child in out.iterdir():
        if child == marker:
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()

try:
    kind = r['kind']
    if kind == 'site':
        args = flags(['new', '--out', str(out)], ['site', 'name', 'start', 'end', 'rawdata', 'runtime', 'met', 'mode'])
        if any(f['path'].upper() == 'DEF_TUNING_CROP_PLANTING_DAY' for f in r.get('fields', [])):
            args.extend(['--crop', '1'])
        call(args)
    else:
        if kind not in ('region', 'watershed', 'global'):
            raise ValueError('unsupported spatial domain')
        grid = r['grid_kind']
        if grid == 'unstructured':
            mesh = str(path(r['mesh_file']))
        elif grid == 'catchment':
            mesh = str(path(r['catchment_file']))
        elif grid == 'latlon':
            import math
            dlon, dlat = float(r['dlon']), float(r['dlat'])
            if not math.isfinite(dlon) or not math.isfinite(dlat) or dlon <= 0 or dlat <= 0:
                raise ValueError('grid resolution must be positive and finite')
            nlon, nlat = round(360 / dlon), round(180 / dlat)
            if nlon < 1 or nlat < 1 or abs(nlon - 360 / dlon) > 1e-9 or abs(nlat - 180 / dlat) > 1e-9 or nlon * nlat > 2**63-1:
                raise ValueError('grid resolution must divide the global dimensions')
            mesh = str(out / 'mesh.nc')
            args = ['mesh-new', '--out', mesh, '--grid-kind', grid, '--nlon', nlon, '--nlat', nlat]
            if kind == 'watershed':
                args.extend(['--shp', r['shapefile']])
            elif kind == 'region':
                flags(args, ['west', 'east', 'south', 'north'])
            call(args)
        else:
            raise ValueError('unsupported grid kind')
        call(['spatial-preflight', '--grid-kind', grid, '--input', mesh, '--out', str(out / 'spatial-input.manifest.json')])
        if r.get('forcing_text'):
            (out / 'forcing.nml').write_text(r['forcing_text'])
            forcing = str(out / 'forcing.nml')
        else:
            forcing = str(path(r['forcing']))
        args = flags(['spatial-new', '--out', str(out), '--grid-kind', grid, '--mesh', mesh, '--forcing', forcing], ['name', 'rawdata', 'runtime', 'start', 'end', 'timestep', 'mode'])
        if grid == 'latlon':
            flags(args, ['dlon', 'dlat'])
        call(args)
    # Copy externally referenced configuration text into the case. Never copy NetCDF inputs.
    files = {p.name: p.read_text().replace(str(out.resolve()) + '/', str(out) + '/') for p in out.glob('*.nml') if p.is_file() and not p.is_symlink()}
    sources = {str((out / name).resolve()): name for name in files}
    pending = list(files)
    while pending:
        name = pending.pop()
        text = files[name]
        # Fortran string literals escape a quote by doubling it.
        quoted = r"![^\n]*|'((?:[^']|'')*)'|\"((?:[^\"]|\"\")*)\""
        def reference(match):
            if match.group(0).startswith('!'):
                return match.group(0)
            quote = match.group(0)[0]
            value = match.group(0)[1:-1].replace(quote + quote, quote)
            for raw in re.split(r'([,;])', value):
                candidate = raw.strip()
                if not candidate.startswith('/') or not candidate.lower().endswith('.nml'):
                    continue
                source = pathlib.Path(candidate)
                if source.parent == out and source.name in files:
                    continue
                if not source.is_file():
                    raise ValueError('referenced namelist does not exist: ' + str(source))
                identity = str(source.resolve())
                target = sources.get(identity)
                if target is None:
                    if len(files) >= 256 or source.stat().st_size > 4 * 1024 * 1024:
                        raise ValueError('referenced namelist configuration exceeds size limit')
                    target = source.name
                    content = source.read_text()
                    if target in files:
                        target = uuid.uuid4().hex[:8] + '-' + target
                    sources[identity] = target
                    files[target] = content
                    pending.append(target)
                value = value.replace(candidate, str(out / target))
            return quote + value.replace(quote, quote + quote) + quote
        text = re.sub(quoted, reference, text)
        files[name] = text
    for name, text in files.items():
        (out / name).write_text(text)
    if sum(len(value.encode()) for value in files.values()) > 16 * 1024 * 1024:
        raise ValueError('namelist configuration exceeds total size limit')
    completed = out / ('.creation-complete-' + uuid.uuid4().hex)
    with completed.open('x') as saved:
        saved.write(json.dumps({'token': token, 'files': files}))
        saved.flush()
        os.fsync(saved.fileno())
    os.replace(completed, marker)
    print(json.dumps({'remote_dir': str(out), 'token': token, 'files': files}))
except BaseException:
    shutil.rmtree(out)
    raise
