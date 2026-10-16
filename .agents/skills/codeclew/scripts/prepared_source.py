#!/usr/bin/env python3
"""Keep a docs context packet local and emit a bounded, explicitly partial view."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys

MAX_PACKET_BYTES = 32 * 1024 * 1024


class ViewError(Exception):
    pass


def encode(value):
    return json.dumps(value, ensure_ascii=False, separators=(',', ':')) + '\n'


def load_packet(path):
    with path.open('rb') as stream:
        raw = stream.read(MAX_PACKET_BYTES + 1)
    if len(raw) > MAX_PACKET_BYTES:
        raise ViewError('PACKET_EXCEEDS_32_MIB; full packet remains local')
    value = json.loads(raw)
    if not isinstance(value, dict) or value.get('schema') != 'codeclew-docs-page/1.0':
        raise ViewError('UNSUPPORTED_PACKET_SCHEMA; no evidence view delivered')
    if not isinstance(value.get('items'), list) or not all(isinstance(i, dict) for i in value['items']):
        raise ViewError('INVALID_ITEMS; no evidence view delivered')
    if any('record' in i and not isinstance(i['record'], dict) for i in value['items']):
        raise ViewError('INVALID_RECORD; no evidence view delivered')
    return value, hashlib.sha256(raw).hexdigest()


def limits(value):
    """Retain nested authority/limitations even when a record is handle-only."""
    if isinstance(value, dict):
        result = {}
        for key, child in value.items():
            if key.lower().endswith(('authority', 'coverage', 'boundaries', 'unresolved', 'limits')) or key in (
                    'omitted', 'nextCursor', 'truncated', 'ordering', 'targetStatus',
                    'targetResolution', 'sourceMapping'):
                result[key] = child
            else:
                nested = limits(child)
                if nested:
                    result[key] = nested
        return result
    if isinstance(value, list):
        return {str(index): nested for index, child in enumerate(value) if (nested := limits(child))}
    return {}


def record_limits(items):
    groups = {}
    for index, item in enumerate(items):
        if item.get('kind') in ('SOURCE', 'COVERAGE') or not (retained := limits(item)):
            continue
        key = json.dumps(retained, sort_keys=True)
        groups.setdefault(key, {'items': [], 'limits': retained})['items'].append(f'/items/{index}')
    return list(groups.values())


def project(packet, path, packet_digest, source_ids=(), max_bytes=12000,
            text_chars=6000, text_offset=0, max_sources=3, max_handles=4):
    """Do not summarize evidence limits or infer freshness from the local view."""
    items = packet['items']
    sources = [i for i in items if i.get('kind') == 'SOURCE']
    if source_ids:
        wanted = set(source_ids)
        sources = [i for i in sources if i.get('id') in wanted]
        if {i.get('id') for i in sources} != wanted:
            raise ViewError('SOURCE_NOT_RETURNED; use exact source drill or cursor, not an alias body')
    if text_offset and len(sources) != 1:
        raise ViewError('TEXT_OFFSET_REQUIRES_ONE_SOURCE')
    selected = sources[:max_sources]
    coverage = [i for i in items if i.get('kind') == 'COVERAGE']
    handles = []
    # Full normalized observations and repeated SOURCE_ALIAS bodies stay local.
    for index, item in enumerate(items):
        if item.get('kind') in ('SOURCE', 'COVERAGE', 'SOURCE_ALIAS'):
            continue
        record = item.get('record', {})
        handles.append({
            'kind': item.get('kind'), 'id': item.get('id'),
            'localPointer': f'/items/{index}',
            **{k: v for k, v in item.items() if k in ('fullRecord', 'drill', 'projection')},
            **{k: v for k, v in record.items() if k in ('fullRecordDigest',)},
        })
    rows = []
    for item in selected:
        record = item.get('record')
        if not isinstance(record, dict) or not isinstance(record.get('text'), str):
            raise ViewError('INVALID_SOURCE_RECORD; no evidence view delivered')
        text = record['text']
        if text_offset > len(text):
            raise ViewError('TEXT_OFFSET_OUTSIDE_SOURCE')
        rows.append({'id': item.get('id'),
                     'record': {k: v for k, v in record.items() if k != 'text'},
                     'text': text[text_offset:text_offset + text_chars],
                     'textDelivery': {'offset': text_offset, 'totalCharacters': len(text)}})
    view = {
        'schema': 'codeclew-prepared-source-view/1.0',
        'packet': {'path': str(path), 'sha256': packet_digest},
        # Preserve every packet-level field, including future limits/bindings.
        'evidence': {k: v for k, v in packet.items() if k != 'items'},
        'coverage': coverage, 'sources': rows, 'handles': handles[:max_handles],
        'recordLimits': record_limits(items),
        'projection': {
            'authority': 'LOCAL_PROJECTION_NOT_ADDITIONAL_VERIFICATION',
            'sourcesNotShown': len([i for i in items if i.get('kind') == 'SOURCE']) - len(rows),
            'aliasesNotShown': len([i for i in items if i.get('kind') == 'SOURCE_ALIAS']),
            'handlesNotShown': len(handles) - min(max_handles, len(handles)),
            'otherRecords': 'Handles only; complete records remain at packet path/localPointer.',
        },
    }
    while True:
        for row in rows:
            delivery = row['textDelivery']
            end = delivery['offset'] + len(row['text'])
            delivery.update(complete=delivery['offset'] == 0 and end == delivery['totalCharacters'],
                            nextOffset=end if end < delivery['totalCharacters'] else None)
        rendered = encode(view)
        if len(rendered.encode('utf-8')) <= max_bytes:
            return rendered
        if view['handles']:
            view['handles'].pop()
            view['projection']['handlesNotShown'] += 1
            continue
        longest = max(rows, key=lambda row: len(row['text']), default=None)
        if longest is None or not longest['text']:
            raise ViewError('EVIDENCE_METADATA_EXCEEDS_OUTPUT_BUDGET; no evidence view delivered')
        longest['text'] = longest['text'][:len(longest['text']) // 2]


def private_file(path):
    return os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb')


def fetch(args, path):
    if not Path(args.root).is_absolute() or not Path(args.clew).is_absolute():
        raise ViewError('FETCH_REQUIRES_ABSOLUTE_ROOT_AND_INSTALLED_LAUNCHER')
    command = [args.clew, 'docs', 'context', '--root', args.root, '--service', args.service,
               '--symbol', args.symbol, '--format', 'compact', '--limit', '40']
    command += ['--snapshot', args.snapshot] if args.snapshot else ['--refresh']
    stderr_path = path.with_name(path.name + '.stderr')
    with private_file(path) as output, private_file(stderr_path) as errors:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=output,
                                stderr=errors, check=False)
    if result.returncode:
        raise ViewError(f'CONTEXT_FAILED exit={result.returncode}; stdout and .stderr retained locally')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest='mode', required=True)
    for mode in ('read', 'fetch'):
        sub = modes.add_parser(mode)
        sub.add_argument('--input' if mode == 'read' else '--output', required=True, type=Path)
        sub.add_argument('--source', action='append', default=[], help='Exact returned SOURCE ID; repeat only as needed')
        sub.add_argument('--max-bytes', type=int, default=12000, help='Strict stdout UTF-8 byte budget, 1024..65536')
        sub.add_argument('--text-chars', type=int, default=6000, help='At most this many characters per selected source')
        sub.add_argument('--text-offset', type=int, default=0, help='Continue one exact source locally without fetching')
        sub.add_argument('--max-sources', type=int, default=3)
        sub.add_argument('--max-handles', type=int, default=4)
        if mode == 'fetch':
            sub.add_argument('--clew', required=True, help='Already resolved installed release launcher')
            sub.add_argument('--root', required=True)
            sub.add_argument('--service', required=True)
            sub.add_argument('--symbol', required=True, help='Exact supplied declaration selector')
            selection = sub.add_mutually_exclusive_group(required=True)
            selection.add_argument('--snapshot')
            selection.add_argument('--refresh', action='store_true')
    args = parser.parse_args()
    if not (1024 <= args.max_bytes <= 65536 and 1 <= args.max_sources <= 16
            and 0 <= args.max_handles <= 32 and 0 <= args.text_chars <= 65536
            and args.text_offset >= 0):
        parser.error('invalid projection bounds')
    path = (args.input if args.mode == 'read' else args.output).resolve()
    try:
        if args.mode == 'fetch':
            fetch(args, path)
        packet, digest = load_packet(path)
        sys.stdout.write(project(packet, path, digest, args.source, args.max_bytes,
                                 args.text_chars, args.text_offset, args.max_sources, args.max_handles))
        return 0
    except (ViewError, OSError, ValueError) as error:
        # Never echo raw packet/parser/subprocess messages into model context.
        reason = str(error) if isinstance(error, ViewError) else type(error).__name__
        sys.stdout.write(encode({'status': 'ERROR', 'reason': reason,
                                 'evidenceDelivered': False, 'packetRetainedLocally': path.is_file()}))
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
