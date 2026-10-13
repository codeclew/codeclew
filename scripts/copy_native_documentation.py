#!/usr/bin/env python3
"""Stage exact native documentation bytes; publication and privacy review are separate."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import shutil


def digest(path):
    return 'sha256:' + hashlib.sha256(path.read_bytes()).hexdigest()


def inventory(source):
    files = []
    for path in source.rglob('*'):
        if any(part in ('.git', '.codeclew') for part in path.relative_to(source).parts):
            raise ValueError('private state is not documentation')
        if path.is_symlink() or not (path.is_file() or path.is_dir()):
            raise ValueError('non-regular documentation file')
        if path.is_file():
            files.append(path)
    return {path.relative_to(source).as_posix(): digest(path) for path in files}


def stage(source, destination):
    source, destination = Path(source).resolve(), Path(destination).absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError('destination already exists')
    target = destination.resolve()
    if target == source or source in target.parents or target in source.parents:
        raise ValueError('source and destination must be separate')
    before = inventory(source)
    bundles, current = [], []
    index = source / 'index.html'
    if not index.is_file() or not (source / 'generated').is_dir():
        raise ValueError('native documentation root is missing')
    for bundle in (source / 'generated').iterdir():
        if not bundle.is_dir() or bundle.is_symlink() or not re.fullmatch(r'[0-9a-f]{64}', bundle.name):
            raise ValueError('invalid native bundle directory')
        publication = json.loads((bundle / 'publication.json').read_text())
        bindings = json.loads((bundle / 'bindings.json').read_text())
        if publication.get('schema') != 'codeclew-documentation-publication/1.0' or publication.get('id') != bundle.name:
            raise ValueError('native publication identity mismatch')
        if bindings.get('schema') not in ('codeclew-documentation-bindings/1.4', 'codeclew-documentation-bindings/1.5'):
            raise ValueError('unsupported native bindings')
        if publication['files'].get('bindings.json') != digest(bundle / 'bindings.json'):
            raise ValueError('native bindings are not bound by publication')
        for records in (publication['files'], bindings['outputHashes']):
            for relative, expected in records.items():
                path = bundle / relative
                if Path(relative).is_absolute() or '..' in Path(relative).parts or not path.resolve().is_relative_to(bundle.resolve()):
                    raise ValueError('native artifact escapes bundle')
                if not path.is_file() or digest(path) != expected:
                    raise ValueError('native artifact digest mismatch: ' + relative)
        if publication['files'].get('root-overview.html') == digest(index):
            current.append(bundle.name)
        bundles.append(bundle.name)
    if len(current) != 1:
        raise ValueError('deployed index must match one native root output')
    shutil.copytree(source, destination, symlinks=True)
    if before != inventory(destination) or before != inventory(source):
        raise ValueError('staged documentation bytes changed')
    return {'status': 'STAGED', 'currentBundle': current[0], 'bundles': bundles,
            'files': len(before), 'bytes': sum((source / relative).stat().st_size for relative in before),
            'nativeBytesChanged': False, 'publicationPerformed': False, 'privacyReview': 'NOT_PERFORMED'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--source', required=True, type=Path)
    parser.add_argument('--destination', required=True, type=Path)
    args = parser.parse_args()
    print(json.dumps(stage(args.source, args.destination)))
