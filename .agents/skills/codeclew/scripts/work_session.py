#!/usr/bin/env python3
"""Guard caller-owned inputs for recorded Work reads."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import runpy

T = runpy.run_path(str(Path(__file__).with_name('work_transport.py')))
Incomplete = T['Incomplete']
MAX_INPUT_BYTES = 16 * 1024 * 1024


def assigned_root(path):
    if not path.is_absolute():
        raise Incomplete('INPUT_ROOT_MUST_BE_ABSOLUTE: supply the caller-assigned output root once')
    root = path.resolve(strict=True)
    if not root.is_dir():
        raise Incomplete('INPUT_ROOT_NOT_DIRECTORY')
    return root


def anchored(root, path, *, exists=True):
    selected = (root / path).resolve()
    if not selected.is_relative_to(root):
        raise Incomplete(f'INPUT_PATH_OUTSIDE_ASSIGNED_ROOT: {path}; assigned root: {root}')
    if exists and not selected.is_file():
        raise Incomplete(f'INPUT_FILE_NOT_FOUND: {path}; resolved: {selected}. Correct this path relative to --input-root; keep the same Work, not a new prepare.')
    return selected


def read_json(path):
    with path.open('rb') as source:
        raw = source.read(MAX_INPUT_BYTES + 1)
    if len(raw) > MAX_INPUT_BYTES:
        raise Incomplete('LOCAL_INPUT_BYTE_LIMIT')
    return raw, json.loads(raw.decode('utf-8'), object_pairs_hook=T['unique_object'],
                           parse_constant=T['reject_constant'])


def validate_selection(value):
    if not isinstance(value, dict) or set(value) - {'references', 'symbols', 'query', 'cursor', 'untrackedReads'}:
        raise Incomplete('INVALID_WORK_SELECTION_FIELDS: use the native Selection JSON')
    for name in ('references', 'symbols'):
        values = value.get(name, [])
        if not isinstance(values, list) or len(values) > 8 or any(not isinstance(v, str) or not v for v in values):
            raise Incomplete(f'INVALID_SELECTION_{name.upper()}: at most eight nonempty strings')
    if sum(bool(value.get(key)) for key in ('references', 'symbols', 'query')) > 1:
        raise Incomplete('MIXED_WORK_SELECTION: select references, symbols, or one query')
    query = value.get('query')
    if query is not None and (not isinstance(query, dict) or set(query) - {'kind', 'symbolContains', 'projection'}
            or not isinstance(query.get('kind'), str) or not query['kind'].strip()):
        raise Incomplete('INVALID_WORK_QUERY')
    if value.get('cursor') is not None and (not isinstance(value['cursor'], str) or not value['cursor']):
        raise Incomplete('INVALID_WORK_CURSOR')
    if value.get('untrackedReads', False) is not False:
        raise Incomplete('UNTRACKED_READS_NOT_SUPPORTED')



def run(action, launcher, documentation_root, input_root, work, input_name, output_name,
        *, max_pages=None, timeout=120):
    result = {'schema': 'codeclew-work-session-result/1.0', 'status': 'BLOCKED', 'work': work,
              'nativeCallsAttempted': 0, 'publication': 'NOT_PERFORMED', 'meaningValidation': 'NOT_PERFORMED'}
    output = None
    try:
        root = assigned_root(input_root)
        target = anchored(root, output_name, exists=False)
        target.mkdir(mode=0o700, parents=True, exist_ok=False)
        output = target
        source = anchored(root, input_name)
        raw, value = read_json(source)
        archived = output / 'input.original.json'
        archived.write_bytes(raw)
        result.update(inputRoot=str(root), resolvedInput=str(source), inputDigest='sha256:' + hashlib.sha256(raw).hexdigest())
        if action == 'read':
            validate_selection(value)
            # Discovery is a first-page probe unless the caller sets a budget.
            # Exact references and initial/cursor-only reads keep full collection.
            page_budget = max_pages if max_pages is not None else (1 if value.get('query') is not None else 1000)
            delivery = T['collect'](launcher, documentation_root, work, output / 'read-collection',
                                    kind='work-selection', selection=value, max_pages=page_budget, timeout=timeout)
            result.update(status=delivery['status'], collection='read-collection/manifest.json',
                          selectionComplete=delivery['selectionComplete'], nextCursor=delivery['nextCursor'])
            if delivery['reason'] is not None:
                result['reason'] = delivery['reason']
        else:
            raise Incomplete('UNSUPPORTED_ACTION')
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        result['reason'] = str(error)
    if output:
        result['nativeCallsAttempted'] = len(list(output.rglob('*.command.json')))
    if result['nativeCallsAttempted']:
        result['nextAction'] = 'Inspect saved Work pages, omissions and the exact continuation selection; continue this same Work. An archived response is not proof of model reading.'
    else:
        result['nextAction'] = 'Correct the named local path or selection relative to --input-root. No native command was invoked; no new Work is needed.'
    if output:
        T['save'](output / 'result.json', result)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action', choices=('read',))
    for name in ('launcher', 'root', 'input-root', 'input', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--work', required=True)
    parser.add_argument('--max-pages', type=int, help='Explicit page budget; discovery queries default to 1, other collection to 1000')
    parser.add_argument('--timeout', type=int, default=120)
    args = parser.parse_args()
    if (args.max_pages is not None and not 1 <= args.max_pages <= 10000) or not 1 <= args.timeout <= 900:
        parser.error('max-pages must be 1..10000 and timeout1..900')
    result = run(args.action, args.launcher.absolute(), args.root.absolute(), args.input_root,
                 args.work, args.input, args.output, max_pages=args.max_pages, timeout=args.timeout)
    print(json.dumps(result, ensure_ascii=False, separators=(',', ':')))
    return 0 if result['status'] in ('COMPLETE', 'TAIL_RECORDED') else 2


if __name__ == '__main__':
    raise SystemExit(main())
