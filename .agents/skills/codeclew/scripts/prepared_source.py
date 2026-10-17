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


def record_limits(items, include_sources=False):
    groups = {}
    for index, item in enumerate(items):
        if item.get('kind') == 'COVERAGE' or (not include_sources and item.get('kind') in ('SOURCE', 'SOURCE_NATIVE')) or not (retained := limits(item)):
            continue
        key = json.dumps(retained, sort_keys=True)
        groups.setdefault(key, {'items': [], 'limits': retained})['items'].append(f'/items/{index}')
    return list(groups.values())


def project(packet, path, packet_digest, source_ids=(), max_bytes=12000,
            text_chars=6000, text_offset=0, max_sources=3, max_handles=4):
    """Do not summarize evidence limits or infer freshness from the local view."""
    items = packet['items']
    sources = [i for i in items if i.get('kind') in ('SOURCE', 'SOURCE_NATIVE')]
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
        if item.get('kind') in ('SOURCE', 'SOURCE_NATIVE', 'COVERAGE', 'SOURCE_ALIAS'):
            continue
        record = item.get('record', {})
        handles.append({
            'kind': item.get('kind'), 'id': item.get('id'),
            'localPointer': f'/items/{index}',
            **{k: v for k, v in item.items() if k in ('fullRecord', 'drill', 'projection')},
            **{k: v for k, v in record.items() if k in ('fullRecordDigest',)},
            **({'change': {k: v for k, v in record.items() if k in ('reason', 'file', 'source', 'input', 'kind', 'sourceReferences', 'changedWindow')}}
               if item.get('kind') == 'EVIDENCE_CHANGE' else {}),
            **({'binding': record} if item.get('kind') == 'SOURCE_BINDING_UPDATE' else {}),
        })
    rows = []
    for item in selected:
        record = item.get('record')
        if not isinstance(record, dict) or not isinstance(record.get('text'), str):
            raise ViewError('INVALID_SOURCE_RECORD; no evidence view delivered')
        text = record['text']
        if text_offset > len(text):
            raise ViewError('TEXT_OFFSET_OUTSIDE_SOURCE')
        rows.append({'id': item.get('id'), 'kind': item.get('kind'),
                     'record': {k: v for k, v in record.items() if k != 'text'},
                     'text': text[text_offset:text_offset + text_chars],
                     'textDelivery': {'offset': text_offset, 'totalCharacters': len(text)}})
    view = {
        'schema': 'codeclew-prepared-source-view/1.0',
        'packet': {'path': str(path), 'sha256': packet_digest},
        # Preserve every packet-level field, including future limits/bindings.
        'evidence': {k: v for k, v in packet.items() if k != 'items'},
        'coverage': coverage, 'sources': rows, 'handles': handles[:max_handles],
        'recordLimits': record_limits(items, include_sources=max_sources == 0),
        'projection': {
            'authority': 'LOCAL_PROJECTION_NOT_ADDITIONAL_VERIFICATION',
            'sourcesNotShown': len([i for i in items if i.get('kind') in ('SOURCE', 'SOURCE_NATIVE')]) - len(rows),
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


def citation(value):
    """Navigation metadata, not a replacement for the retained source record."""
    return {k: v for k, v in value.items() if k in (
        'id', 'service', 'bindingService', 'revision', 'file', 'startLine', 'endLine',
        'url', 'textDigest', 'authority', 'membershipCoverage')}


def maintenance(packet, path, packet_digest, max_bytes=12000, text_chars=6000,
                max_sources=3, record_offset=0):
    comparison = packet.get('comparison')
    if not isinstance(comparison, dict) or comparison.get('status') not in ('UNCHANGED', 'CHANGED', 'UNKNOWN'):
        raise ViewError('ENGINE_COMPARE_RESULT_MISSING; no comparison view delivered')
    items = packet['items']
    records = [(i, item) for i, item in enumerate(items)
               if item.get('kind') in ('EVIDENCE_CHANGE', 'SOURCE_BINDING_UPDATE')]
    if record_offset > len(records):
        raise ViewError('MAINTENANCE_RECORD_OFFSET_OUTSIDE_PACKET')
    source_items = [i for i in items if i.get('kind') in ('SOURCE', 'SOURCE_NATIVE')]
    if any(not isinstance(i.get('record', {}).get('text'), str) for i in source_items):
        raise ViewError('INVALID_SOURCE_RECORD; no evidence view delivered')
    changed_ids, changed_files = set(), set()
    for _, item in records:
        change = item.get('record', {})
        if item.get('kind') != 'EVIDENCE_CHANGE':
            continue
        if change.get('reason') == 'SELECTED_SOURCE_BYTES_CHANGED':
            changed_ids.add(change.get('source'))
        if change.get('reason') == 'REGISTERED_NATIVE_FILE_CHANGED':
            after = change.get('after') or {}
            if not isinstance(after, dict):
                raise ViewError('INVALID_NATIVE_CHANGE_RECORD')
            changed_files.add((after.get('service'), after.get('path')))
    rows = []
    for item in source_items:
        record = item.get('record', {})
        if item.get('id') not in changed_ids and (record.get('service'), record.get('file')) not in changed_files:
            continue
        if len(rows) == max_sources:
            break
        if not isinstance(record.get('text'), str):
            raise ViewError('INVALID_SOURCE_RECORD; no evidence view delivered')
        rows.append({'id': item.get('id'), 'kind': item.get('kind'),
                     'record': {k: v for k, v in record.items() if k != 'text'},
                     'text': record['text'][:text_chars],
                     'textDelivery': {'offset': 0, 'totalCharacters': len(record['text'])}})
    evidence = {k: v for k, v in packet.items() if k != 'items'}
    if records:
        # Concrete records replace the small redundant reason-example header.
        evidence['comparison'] = {k: v for k, v in comparison.items() if k != 'reasonExamples'}
    view = {'schema': 'codeclew-prepared-maintenance-view/1.0', 'status': comparison['status'],
            'packet': {'path': str(path), 'sha256': packet_digest}, 'evidence': evidence,
            'coverage': [i for i in items if i.get('kind') == 'COVERAGE'],
            'changes': [], 'citationUpdates': [], 'sources': rows,
            'recordLimits': record_limits(items, include_sources=True),
            'projection': {'authority': 'LOCAL_PROJECTION_NOT_ADDITIONAL_VERIFICATION',
                           'unchangedSourceBodies': 'NOT_REPLAYED',
                           'sourceBodies': 'METADATA_ONLY' if max_sources == 0 else 'CHANGED_SOURCE_ONLY',
                           'totalRecords': len(records), 'recordOffset': record_offset,
                           'bindingCompleteness': 'ENGINE_BOUND' if all(isinstance(comparison.get(k), dict) for k in ('baselineBinding', 'targetBinding')) else 'LEGACY_PACKET_BINDINGS_NOT_RETURNED',
                           'sourcesNotShown': len(source_items) - len(rows)},
            'continuation': None}
    shown = records[record_offset:]
    while True:
        changes, bindings = [], []
        for index, item in shown:
            record = item.get('record', {})
            row = {'id': item.get('id'), 'localPointer': f'/items/{index}'}
            if item.get('kind') == 'SOURCE_BINDING_UPDATE':
                row['binding'] = {k: v for k, v in record.items() if k not in ('before', 'after')}
                row['binding']['after'] = citation(record.get('after', {}))
                row['binding']['previousRevision'] = record.get('before', {}).get('revision')
                row['binding']['completeBeforeAfter'] = {'kind': 'LOCAL_PACKET_RECORD',
                                                        'input': str(path), 'pointer': f'/items/{index}'}
                bindings.append(row)
            else:
                row['record'] = dict(record)
                window = record.get('changedWindow')
                if isinstance(window, dict) and isinstance(window.get('afterCitation'), dict):
                    row['record']['changedWindow'] = {k: citation(v) if k in ('beforeCitation', 'afterCitation') and isinstance(v, dict) else v for k, v in window.items()}
                    if record.get('reason') == 'SELECTED_SOURCE_BYTES_CHANGED':
                        row['record'].pop('before', None)
                        row['record'].pop('after', None)
                    source = window['afterCitation']
                    if source.get('id') not in {i.get('id') for i in source_items}:
                        row['sourceContinuation'] = {'kind': 'PINNED_SOURCE_DRILL',
                            'snapshot': comparison.get('targetSnapshot'), 'service': source.get('service'),
                            'source': source.get('id')}
                changes.append(row)
        view['changes'], view['citationUpdates'] = changes, bindings
        consumed = record_offset + len(shown)
        view['projection'].update(recordsNotShown=len(records) - len(shown),
                                  remainingRecords=len(records) - consumed)
        view['continuation'] = ({'kind': 'LOCAL_MAINTENANCE_RECORDS', 'input': str(path),
                                 'recordOffset': consumed} if consumed < len(records) else None)
        for row in rows:
            end = len(row['text'])
            row['textDelivery'].update(complete=end == row['textDelivery']['totalCharacters'],
                nextOffset=end if end < row['textDelivery']['totalCharacters'] else None)
            row['continuation'] = {'kind': 'LOCAL_SOURCE_TEXT', 'input': str(path),
                                   'source': row['id'], 'textOffset': end}
        rendered = encode(view)
        if len(rendered.encode('utf-8')) <= max_bytes:
            return rendered
        longest = max(rows, key=lambda row: len(row['text']), default=None)
        if longest and longest['text']:
            longest['text'] = longest['text'][:len(longest['text']) // 2]
        elif len(shown) > 1:
            shown = shown[:-1]
        else:
            raise ViewError('MAINTENANCE_METADATA_OR_CHANGE_EXCEEDS_OUTPUT_BUDGET; no evidence view delivered')


def local_record(packet, path, packet_digest, pointer, max_bytes):
    parts = pointer.split('/')
    if len(parts) != 3 or parts[:2] != ['', 'items'] or not parts[2].isdigit() or str(int(parts[2])) != parts[2]:
        raise ViewError('INVALID_LOCAL_PACKET_POINTER')
    index = int(parts[2])
    if index >= len(packet['items']):
        raise ViewError('LOCAL_PACKET_POINTER_OUTSIDE_PACKET')
    view = {'schema': 'codeclew-prepared-local-record/1.0',
            'packet': {'path': str(path), 'sha256': packet_digest},
            'localPointer': pointer, 'item': packet['items'][index],
            'evidence': {k: v for k, v in packet.items() if k != 'items'},
            'recordLimits': record_limits(packet['items'], include_sources=True)}
    rendered = encode(view)
    if len(rendered.encode('utf-8')) > max_bytes:
        raise ViewError('LOCAL_RECORD_EXCEEDS_OUTPUT_BUDGET; no evidence view delivered')
    return rendered


def private_file(path):
    return os.fdopen(os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600), 'wb')


def invoke(args, path, command):
    if not Path(args.root).is_absolute() or not Path(args.clew).is_absolute():
        raise ViewError('FETCH_REQUIRES_ABSOLUTE_ROOT_AND_INSTALLED_LAUNCHER')
    stderr_path = path.with_name(path.name + '.stderr')
    with private_file(path) as output, private_file(stderr_path) as errors:
        result = subprocess.run(command, stdin=subprocess.DEVNULL, stdout=output,
                                stderr=errors, check=False)
    if result.returncode:
        raise ViewError(f'CONTEXT_FAILED exit={result.returncode}; stdout and .stderr retained locally')


def fetch(args, path):
    command = [args.clew, 'docs', 'evidence', 'retain-source', '--root', args.root,
               '--service', args.service, '--symbol', args.symbol]
    command += ['--snapshot', args.snapshot] if args.snapshot else ['--refresh']
    if args.native_inputs:
        if not args.native_inputs.is_absolute():
            raise ViewError('NATIVE_INPUTS_REQUIRES_ABSOLUTE_PATH')
        command += ['--native-inputs', str(args.native_inputs)]
    invoke(args, path, command)


def compare(args, path):
    command = [args.clew, 'docs', 'evidence', 'compare-source', '--root', args.root,
               '--input', str(args.input.resolve()), '--snapshot', args.snapshot]
    if args.include_context:
        command += ['--include-context']
    invoke(args, path, command)


def drill(args, path):
    command = [args.clew, 'docs', 'context', '--root', args.root,
               '--service', args.service, '--snapshot', args.snapshot, '--limit', '20']
    for source in args.source:
        command += ['--source', source]
    invoke(args, path, command)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    modes = parser.add_subparsers(dest='mode', required=True)
    for mode in ('read', 'fetch', 'compare', 'drill'):
        sub = modes.add_parser(mode)
        if mode in ('read', 'compare'):
            sub.add_argument('--input', required=True, type=Path)
        if mode == 'read':
            sub.add_argument('--record', help='Exact LOCAL_PACKET_RECORD pointer, for example /items/0')
        if mode != 'read':
            sub.add_argument('--output', required=True, type=Path)
        sub.add_argument('--source', action='append', default=[], required=mode == 'drill', help='Exact returned SOURCE ID; repeat only as needed')
        sub.add_argument('--max-bytes', type=int, default=12000, help='Strict stdout UTF-8 byte budget, 1024..65536')
        sub.add_argument('--text-chars', type=int, default=6000, help='At most this many characters per selected source')
        sub.add_argument('--text-offset', type=int, default=0, help='Continue one exact source locally without fetching')
        sub.add_argument('--max-sources', type=int, default=3, help='0 is metadata-only; no SOURCE bodies')
        sub.add_argument('--metadata-only', action='store_true', help='Do not deliver SOURCE bodies; keep decisions, changed values and limits')
        sub.add_argument('--record-offset', type=int, default=0, help='Local maintenance continuation, no acquisition')
        sub.add_argument('--max-handles', type=int, default=4)
        if mode != 'read':
            sub.add_argument('--clew', required=True, help='Already resolved installed release launcher')
            sub.add_argument('--root', required=True)
        if mode == 'drill':
            sub.add_argument('--snapshot', required=True)
            sub.add_argument('--service', required=True)
        if mode == 'compare':
            sub.add_argument('--snapshot', required=True)
            sub.add_argument('--include-context', action='store_true')
        if mode == 'fetch':
            sub.add_argument('--native-inputs', type=Path, help='Explicit engine-registered native file/membership scopes')
            sub.add_argument('--service', required=True)
            sub.add_argument('--symbol', required=True, help='Exact supplied declaration selector')
            selection = sub.add_mutually_exclusive_group(required=True)
            selection.add_argument('--snapshot')
            selection.add_argument('--refresh', action='store_true')
    args = parser.parse_args()
    if not (1024 <= args.max_bytes <= 65536 and 0 <= args.max_sources <= 16
            and 0 <= args.max_handles <= 32 and 0 <= args.text_chars <= 65536
            and args.text_offset >= 0 and args.record_offset >= 0):
        parser.error('invalid projection bounds')
    if args.metadata_only:
        args.max_sources = 0
    path = (args.input if args.mode == 'read' else args.output).resolve()
    try:
        if args.mode == 'fetch':
            fetch(args, path)
        elif args.mode == 'compare':
            compare(args, path)
        elif args.mode == 'drill':
            drill(args, path)
        packet, digest = load_packet(path)
        if args.mode == 'fetch' and (not isinstance(packet.get('evidenceSet'), dict) or packet['evidenceSet'].get('schema') != 'codeclew-prepared-source-evidence/1.0'):
            raise ViewError('ENGINE_RETAIN_RESULT_MISSING; no admitted evidence view delivered')
        if args.mode == 'compare' and (not isinstance(packet.get('comparison'), dict) or packet['comparison'].get('status') not in ('UNCHANGED', 'CHANGED', 'UNKNOWN')):
            raise ViewError('ENGINE_COMPARE_RESULT_MISSING; no comparison view delivered')
        if args.mode == 'drill' and (not isinstance(packet.get('evidenceValidation'), dict) or packet['evidenceValidation'].get('mode') != 'PINNED_SOURCE_SNAPSHOT_CLOSURE'):
            raise ViewError('PINNED_SOURCE_ADMISSION_MISSING; no historical source view delivered')
        if args.mode == 'read' and args.record:
            if args.source or args.text_offset or args.record_offset:
                raise ViewError('LOCAL_RECORD_SELECTOR_CONFLICT')
            sys.stdout.write(local_record(packet, path, digest, args.record, args.max_bytes))
        elif (args.mode == 'compare' or args.mode == 'read' and 'comparison' in packet) and not args.source:
            if args.text_offset:
                raise ViewError('TEXT_OFFSET_REQUIRES_ONE_SOURCE')
            sys.stdout.write(maintenance(packet, path, digest, args.max_bytes, args.text_chars,
                                         args.max_sources, args.record_offset))
        else:
            if args.record_offset:
                raise ViewError('RECORD_OFFSET_REQUIRES_MAINTENANCE_PACKET')
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
