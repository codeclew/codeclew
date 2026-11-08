"""Refresh unchanged official-content skills once per installed release transition."""
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import stat
import sys
import tempfile

INSTALL = runpy.run_path(str(Path(__file__).with_name('install_agent_skill.py')))
canonical = lambda value: json.dumps(value, sort_keys=True, separators=(',', ':')).encode()


def safe_path(path):
    path = Path(path)
    if not path.is_absolute() or str(path) != str(path.resolve()):
        raise ValueError('noncanonical skill path')
    if path.name != 'codeclew' or path.parent == Path('/'):
        raise ValueError('invalid skill destination')
    return path


def read_state(path, default):
    if not path.exists() and not path.is_symlink(): return default
    metadata = path.lstat()
    if not stat.S_ISREG(metadata.st_mode) or metadata.st_uid != os.geteuid() or metadata.st_mode & 0o077 or metadata.st_size > 65536:
        raise ValueError('unsafe skill state')
    return json.loads(path.read_bytes())


def write_state(path, value):
    if path.exists() or path.is_symlink(): read_state(path, {})
    descriptor, name = tempfile.mkstemp(prefix='.skills-', dir=path.parent)
    try:
        with os.fdopen(descriptor, 'wb') as stream:
            stream.write(canonical(value)); stream.flush(); os.fsync(stream.fileno())
        os.replace(name, path)
    finally:
        if Path(name).exists(): Path(name).unlink()


@contextlib.contextmanager
def locked(root):
    fd = os.open(root / '.skill-update.lock', os.O_CREAT | os.O_RDWR | getattr(os, 'O_NOFOLLOW', 0), 0o600)
    try:
        meta = os.fstat(fd)
        if not stat.S_ISREG(meta.st_mode) or meta.st_uid != os.geteuid() or meta.st_mode & 0o077:
            raise ValueError('unsafe skill lock')
        fcntl.flock(fd, fcntl.LOCK_EX)
        yield
    finally: os.close(fd)


def release_context(source):
    source = Path(source).resolve(); release = source.parent
    if source.name != 'source' or release.parent.name != 'releases' or not (release / 'VERSION').is_file(): return None
    return release, release.parent.parent


def verified_skill(release):
    release = Path(release).resolve(strict=True)
    version = (release / 'VERSION').read_text().strip()
    if not re.fullmatch(r'v\d+\.\d+\.\d+', version) or not release.name.startswith(version + '-'):
        raise ValueError('invalid installed release')
    paths = list((release / 'seed').glob('release-N-*/seed.json'))
    if len(paths) != 1 or paths[0].is_symlink(): raise ValueError('installed release seed unavailable')
    meta = paths[0].stat()
    if meta.st_uid != os.geteuid() or stat.S_IMODE(meta.st_mode) != 0o400: raise ValueError('unsafe installed release seed')
    seed = json.loads(paths[0].read_bytes()); unsigned = {k:v for k,v in seed.items() if k != 'seedDigest'}
    if seed.get('mode') != 'RELEASE' or seed.get('seedDigest') != 'sha256:' + hashlib.sha256(canonical(unsigned)).hexdigest():
        raise ValueError('invalid installed release seed digest')
    bytecode = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        bootstrap = runpy.run_path(str(release / 'source/bootstrap/clew_bootstrap.py'))
    finally: sys.dont_write_bytecode = bytecode
    try: bootstrap['_verify_release_source'](release / 'source', seed)
    except Exception as error: raise ValueError('installed source verification failed') from error
    rows = INSTALL['package_files'](release / 'source/skills/codeclew')
    return rows, INSTALL['digest'](rows), version


def receipts(root):
    value = read_state(root / 'skill-installs.json', {'schema':'codeclew-skill-installs/1','copies':{}})
    if not isinstance(value, dict) or value.get('schema') != 'codeclew-skill-installs/1' or not isinstance(value.get('copies'), dict):
        raise ValueError('invalid skill receipts')
    return value


def record(source, installations, expected, pinned=False):
    context = release_context(source)
    if context is None:
        if pinned: raise ValueError('--pin requires an installed release')
        return
    release, root = context
    with locked(root):
        _, actual, _ = verified_skill(release)
        if actual != expected: raise ValueError('bundled skill digest changed')
        value = receipts(root)
        for item in installations:
            path = safe_path(item['path'])
            value['copies'][str(path)] = {'digest':actual,'release':str(release),'pinned':pinned}
        write_state(root / 'skill-installs.json', value)


def refresh(release):
    release = Path(release).resolve(strict=True); root = release.parent.parent
    if release.parent.name != 'releases': raise ValueError('invalid installed release layout')
    with locked(root):
        state = read_state(root / 'skill-release.json', {})
        if state.get('release') == str(release): return []
        rows, expected, _ = verified_skill(release)
        value = receipts(root); copies = value['copies']
        official = {str(release):(rows,expected)}; messages = []
        prior = [p for p in release.parent.glob('v*-*-core') if p != release and not p.is_symlink()]
        transition = False
        for old in sorted(prior, reverse=True):
            try:
                official[str(old)] = verified_skill(old)[:2]; transition = True; break
            except Exception: continue
        if prior and not transition:
            messages.append('Preserved existing skills: no verified previous release. Use clew skill install explicitly.')
        targets = set(copies)
        if transition: targets |= {str(Path.home()/part/'skills/codeclew') for part in ('.agents','.claude')}
        for name in sorted(targets):
            try:
                path = safe_path(name)
                if not path.exists() and not path.is_symlink(): continue
                receipt = copies.get(name)
                if receipt and receipt.get('pinned'): raise ValueError('pinned copy')
                current = INSTALL['digest'](INSTALL['package_files'](path))
                if receipt:
                    old = Path(receipt['release'])
                    if old.parent != release.parent or old.is_symlink(): raise ValueError('invalid receipt release')
                    if str(old) not in official: official[str(old)] = verified_skill(old)[:2]
                    if current != receipt['digest'] or current != official[str(old)][1]: raise ValueError('modified copy')
                    reason = 'managed receipt'
                elif current == expected:
                    reason = 'legacy official-content match'
                else:
                    match = re.search(r'^  version: "(\d+\.\d+\.\d+)"$', (path/'SKILL.md').read_text(), re.M)
                    candidates = list(release.parent.glob('v'+match[1]+'-*-core')) if match else []
                    matched = False
                    for old in candidates:
                        if old.is_symlink(): continue
                        if str(old) not in official: official[str(old)] = verified_skill(old)[:2]
                        matched |= current == official[str(old)][1]
                    if not matched: raise ValueError('unmanaged or modified copy')
                    reason = 'legacy official-content match'
                if current != expected:
                    INSTALL['install_one'](path.parent, rows, True)
                    messages.append(f'Updated {path} ({reason}); reload the skill or start a new task.')
                copies[name] = {'digest':expected,'release':str(release),'pinned':False}
            except (OSError, ValueError, TypeError, KeyError, AttributeError, INSTALL['InstallError']) as error:
                messages.append(f'Preserved {name}: {error}. Review it and use clew skill install explicitly.')
        write_state(root/'skill-installs.json', value)
        write_state(root/'skill-release.json', {'schema':'codeclew-skill-release/1','release':str(release),'digest':expected})
        return messages


if __name__ == '__main__':
    try:
        for message in refresh(Path(sys.argv[1])): print('[codeclew] '+message, file=sys.stderr)
    except Exception as error:
        print('[codeclew] Skill refresh preserved existing copies: '+str(error)+'. Use clew skill install explicitly.', file=sys.stderr)
