import fcntl, hashlib, json, os, pathlib, re, shutil, subprocess, sys, uuid

def report_error(kind, error, traceback):
    # Expected ownership/input refusals are actionable messages, not Python failures.
    if issubclass(kind, ValueError):
        print(str(error), file=sys.stderr)
    else:
        sys.__excepthook__(kind, error, traceback)

sys.excepthook = report_error

payload = json.loads(sys.argv[1])
r = payload['request']
action = payload['action']
cli = payload['cli']

def forget(path):
    # Path.unlink's flag for a missing file needs Python 3.8; CentOS 8's python3 is 3.6.
    try:
        path.unlink()
    except FileNotFoundError:
        pass

def call(args):
    # CentOS 8's python3 is 3.6: subprocess.run takes universal_newlines and explicit pipes.
    p = subprocess.run([cli] + [str(x) for x in args], universal_newlines=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
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
receipt = out / '.colm-desktop-publication.json'

def digest(value):
    return hashlib.sha256(json.dumps(value, sort_keys=True, separators=(',', ':')).encode()).hexdigest()

def request_identity():
    return digest({key: value for key, value in r.items() if key != 'token'})

def published_receipt():
    # A receipt authorizes acknowledgement only, never rewriting a published case.
    if out.is_symlink() or receipt.is_symlink():
        raise ValueError('case already published and its ownership differs; choose a new case directory')
    saved = json.loads(receipt.read_text())
    stat = out.stat()
    if saved.get('token') != r.get('token') or saved.get('directory') != [stat.st_dev, stat.st_ino]:
        raise ValueError('case already published and its ownership differs; choose a new case directory')
    owner = out / '.colm-desktop-server.json'
    if owner.is_symlink() or json.loads(owner.read_text()) != {'schema': 1, 'remote_directory': str(out)}:
        raise ValueError('server case identity changed')
    for name, checksum in saved['published'].items():
        target = out / name
        if pathlib.PurePosixPath(name).name != name or target.is_symlink() or not target.is_file() or digest(target.read_text()) != checksum:
            raise ValueError('published configuration changed; refusing to replay creation')
    return saved


def clear_children():
    for child in out.iterdir():
        if child == marker:
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()

def remove_owned(lock):
    # NFS keeps an unlinked open file as .nfsXXXX, which makes rmdir fail with ENOTEMPTY:
    # remove everything else under the lock, then release the marker before deleting it.
    clear_children()
    lock.close()
    forget(marker)
    out.rmdir()

def empty_leftover():
    # An earlier cleanup may leave only the empty directory; nothing in it can be lost.
    return out.is_dir() and not out.is_symlink() and not marker.exists() and not any(out.iterdir())

if action in ('publish', 'abort', 'sync'):
    if action != 'sync' and (receipt.exists() or receipt.is_symlink()):
        if action == 'abort' and r.get('incomplete_only'):
            if out.is_symlink() or receipt.is_symlink():
                raise ValueError('case publication ownership changed')
            # Release only the client's obsolete pending request. Never mutate a
            # published directory, even when its token or current config differs.
            print(json.dumps({'ok': True, 'directory_retained': True}))
            sys.exit(0)
        saved = published_receipt()
        if action == 'publish' and {name: digest(text) for name, text in r['files'].items()} == saved['published']:
            print(json.dumps({'ok': True}))
            sys.exit(0)
        raise ValueError('case already published; retry the original settings to recover it')
    if action == 'abort' and r.get('incomplete_only'):
        if not out.exists() and not out.is_symlink():
            print(json.dumps({'ok': True}))
            sys.exit(0)
        if empty_leftover():
            out.rmdir()
            print(json.dumps({'ok': True}))
            sys.exit(0)
        if out.is_symlink() or marker.is_symlink():
            raise ValueError('case creation ownership changed')
        if not marker.exists():
            # No claim means there is nothing owned by this attempt to abort. Leave
            # every foreign file intact; create will independently check collisions.
            print(json.dumps({'ok': True}))
            sys.exit(0)
        lock = marker.open('r+')
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        if not os.path.samestat(os.fstat(lock.fileno()), marker.stat()):
            raise ValueError('case creation ownership changed')
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
        if json.loads(marker.read_text()).get('adopted'):
            # The user chose an existing folder: keep its files, drop only our claim on it.
            if r.get('incomplete_only'):
                lock.close()
            marker.unlink()
        elif r.get('incomplete_only'):
            remove_owned(lock)
        else:
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
                        forget(target)
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
                forget(temporary)
                if backup is not None and not preserve_backups:
                    forget(backup)
        if action == 'publish':
            (out / '.colm-desktop-server.json').write_text(json.dumps({'schema': 1, 'remote_directory': str(out)}))
            saved = json.loads(marker.read_text())
            stat = out.stat()
            published = dict(saved, directory=[stat.st_dev, stat.st_ino],
                             published={name: digest(text) for name, text in files.items()})
            temporary = out / ('.publication-' + uuid.uuid4().hex)
            with temporary.open('x') as stream:
                stream.write(json.dumps(published))
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, receipt)
            marker.unlink()
    print(json.dumps({'ok': True}))
    sys.exit(0)

if action != 'create':
    raise ValueError('unsupported case action')
# A persisted client token makes a lost SSH response recoverable.
token = r['token']
if not isinstance(token, str) or not re.fullmatch(r'[a-f0-9]{32,64}', token):
    raise ValueError('invalid creation token')
if receipt.exists() or receipt.is_symlink():
    saved = published_receipt()
    if saved.get('request_identity') != request_identity():
        raise ValueError('creation settings changed; refusing to replay a published case')
    print(json.dumps({'remote_dir': str(out), 'token': token, 'files': saved['files']}))
    sys.exit(0)
out.parent.mkdir(parents=True, exist_ok=True)

def write_claim(lock, adopted):
    lock.seek(0)
    lock.truncate()
    lock.write(json.dumps({'token': token, 'adopted': adopted, 'outputs_owned': True, 'request_identity': request_identity()}))
    lock.flush()
    os.fsync(lock.fileno())

def check_unowned_outputs(owned=False):
    # 后续会重写所有平铺 namelist；仅有无关数据的目录才可接管。
    for child in out.iterdir():
        # `new` creates out/ before finishing. Only a claim that already owns
        # generated outputs may reuse that empty ordinary directory after interruption.
        if child.name == 'out' and owned and not child.is_symlink() and child.is_dir() and not any(child.iterdir()):
            continue
        if child.name.endswith('.nml') or child.name in (
                'site.nc', 'mesh.nc', 'out', 'runs', 'spatial-input.manifest.json', '.colm-desktop-server.json', '.colm-desktop-publication.json'):
            if not owned or child.is_symlink() or child.name in ('out', 'runs', '.colm-desktop-server.json', '.colm-desktop-publication.json'):
                raise ValueError('拒绝覆盖服务器上已有的算例配置、数据或结果 / refusing to overwrite existing case configuration, data or results: ' + str(child))
    # CLI 会自动识别城市站点；默认参数表的目录也必须由本次创建拥有。
    if r.get('kind') == 'site':
        for name, filename in (('rawdata', 'NCAR_urban_properties.nc'), ('runtime', 'LUCY_rawdata.nc')):
            if r.get(name) is not None and str(r[name]).strip():
                continue
            root = out / name
            if not owned and (root.exists() or root.is_symlink()):
                raise ValueError('拒绝覆盖服务器上已有的算例配置、数据或结果 / refusing to overwrite existing case configuration, data or results: ' + str(root))
            for target in (root, root / 'urban', root / 'urban' / filename):
                if target.is_symlink():
                    raise ValueError('refusing to create case data through a symlink: ' + str(target))

def claim(adopted):
    lock = marker.open('x+')
    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    try:
        check_unowned_outputs()
        write_claim(lock, adopted)
    except BaseException:
        lock.close()
        forget(marker)
        raise
    return lock

if out.is_symlink() or marker.is_symlink():
    raise ValueError('refusing to create a case through a symlink: ' + str(out))
if out.exists() and not out.is_dir():
    raise ValueError('case destination is not a directory: ' + str(out))
try:
    out.mkdir(parents=False, exist_ok=False)
    lock, adopted = claim(False), False
except FileExistsError:
    if not marker.exists():
        # An empty leftover holds nothing to keep, so a failed retry may remove it again.
        adopted = any(out.iterdir())
        lock = claim(adopted)
    else:
        lock = marker.open('r+')
        try:
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise ValueError('another case creation is still writing ' + str(out))
        # 完成记录用原子替换发布；旧 inode 的锁不能授权修改新的记录。
        if out.is_symlink() or marker.is_symlink() or not os.path.samestat(os.fstat(lock.fileno()), marker.stat()):
            raise ValueError('case creation ownership changed')
        saved = json.load(lock)
        if saved.get('token') != token:
            raise ValueError('case creation ownership changed; refusing to modify it')
        # Pre-receipt incomplete claims used token ownership only; the next successful
        # creation upgrades them. New claims also bind the complete request.
        if 'request_identity' in saved and saved['request_identity'] != request_identity():
            raise ValueError('creation settings changed; abort the incomplete attempt before retrying')
        if saved.get('files') is not None:
            print(json.dumps({'remote_dir': str(out), 'token': token, 'files': saved['files']}))
            sys.exit(0)
        adopted = bool(saved.get('adopted'))
        if (out / '.colm-desktop-server.json').exists() or (out / '.colm-desktop-server.json').is_symlink():
            raise ValueError('refusing to overwrite an existing server case')
        if adopted:
            check_unowned_outputs(owned=saved.get('outputs_owned', False))
        else:
            # This interrupted creator made the directory; its partial output can go.
            clear_children()
        write_claim(lock, adopted)

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
        saved.write(json.dumps({'token': token, 'adopted': adopted, 'files': files, 'request_identity': request_identity()}))
        saved.flush()
        os.fsync(saved.fileno())
    os.replace(completed, marker)
    print(json.dumps({'remote_dir': str(out), 'token': token, 'files': files}))
except BaseException:
    try:
        if adopted:
            lock.close()
            forget(marker)
        else:
            remove_owned(lock)
    except OSError as cleanup:
        # Report the creation failure itself; a cleanup problem must not replace it.
        print('cleanup of ' + str(out) + ' failed: ' + str(cleanup), file=sys.stderr)
    raise
