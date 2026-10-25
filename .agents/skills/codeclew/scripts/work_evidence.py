#!/usr/bin/env python3
"""Bounded offline view/search of saved native Work SOURCE and SYMBOL evidence."""
from __future__ import annotations
import argparse
import hashlib
import json
from pathlib import Path
import runpy

S = runpy.run_path(str(Path(__file__).with_name('work_session.py')))
T = S['T']
Incomplete = T['Incomplete']


def digest(value):
    return 'sha256:' + hashlib.sha256(json.dumps(value, sort_keys=True, ensure_ascii=False, separators=(',', ':')).encode()).hexdigest()


def view(input_root, work, pages=(), collections=(), *, references=(), query='', cursor=0, limit=8, offset=None, max_bytes=2048):
    if not 1 <= limit <= 32 or not 4 <= max_bytes <= 8192 or cursor < 0 or (offset is not None and offset < 0):
        raise Incomplete('VIEW_LIMIT_OUT_OF_RANGE')
    root = S['assigned_root'](input_root)
    paths = list(pages)
    collection_gaps = []
    for collection in collections:
        directory = S['anchored'](root, collection, exists=False)
        _, manifest = S['read_json'](S['anchored'](root, directory / 'manifest.json'))
        if not manifest.get('schema', '').startswith('codeclew-work-'):
            raise Incomplete('EXPECTED_WORK_COLLECTION_NOT_PROPOSAL_OR_DOCS_PAGE')
        identity = manifest.get('identity') or {}
        if identity.get('work') != work:
            raise Incomplete('COLLECTION_WORK_IDENTITY_CHANGED')
        if manifest.get('status') != 'COMPLETE':
            collection_gaps.append({'collection': str(collection), 'status': manifest.get('status'), 'reason': manifest.get('reason'), 'nextCursor': manifest.get('nextCursor')})
        for page in manifest.get('artifacts', {}).get('pages', []):
            paths.append(directory / page['stdout'])
    if not 1 <= len(paths) <= 512:
        raise Incomplete('SUPPLY_1_TO_512_SAVED_WORK_PAGES')
    entries, parts, continuations, omissions = {}, {}, [], []
    identity = None
    for name in paths:
        path = S['anchored'](root, name)
        raw, page = S['read_json'](path)
        if not isinstance(page, dict):
            raise Incomplete('INVALID_WORK_PAGE')
        origin = {'path': str(path.relative_to(root)), 'rawDigest': 'sha256:' + hashlib.sha256(raw).hexdigest()}
        if page.get('schema') == 'codeclew-documentation-source-part/1.0':
            if page.get('work') != work or not isinstance(page.get('source'), dict) or not isinstance(page.get('reference'), str):
                raise Incomplete('SOURCE_PART_WORK_IDENTITY_CHANGED')
            parts.setdefault(page.get('reference'), []).append((page, origin))
            continue
        T['page_identity'](page, work, 'work-evidence')
        observed = {key: page.get(key) for key in ('work', 'snapshot', 'subject', 'contextDigest', 'inputDigest', 'authority')}
        if identity is not None and identity != observed:
            raise Incomplete('WORK_EVIDENCE_IDENTITY_CHANGED')
        identity = observed
        if page['nextCursor'] is not None:
            continuations.append({**origin, 'nextCursor': page['nextCursor'], 'membershipDigest': page['membershipDigest']})
        omissions.extend([{**row, 'page': origin} for row in page['omitted']])
        for index, row in enumerate(page['items']):
            record = row.get('record', {})
            if not isinstance(record, dict):
                raise Incomplete('INVALID_EVIDENCE_RECORD')
            if row['kind'] == 'SOURCE' and record.get('id') != row['id']:
                raise Incomplete('SOURCE_RECORD_ID_CHANGED')
            kind = ('SOURCE' if row['kind'] == 'SOURCE' else 'SYMBOL' if row['kind'] == 'DEPENDENCY' and record.get('kind') == 'SYMBOL'
                    else 'SYMBOL_NAVIGATION' if row['kind'] == 'CALLABLE_SUMMARY' else None)
            if kind is None:
                continue  # Retained prose is never indexed as code evidence.
            reference = row.get('reference') or record.get('fullRecordReference')
            if not isinstance(reference, str) or not isinstance(record, dict):
                raise Incomplete('MISSING_NATIVE_EVIDENCE_REFERENCE')
            if kind == 'SOURCE' and isinstance(record.get('text'), str) and record.get('textDigest') != 'sha256:' + hashlib.sha256(record['text'].encode()).hexdigest():
                raise Incomplete('SOURCE_TEXT_DIGEST_CHANGED')
            source_references = row.get('sourceReferences', record.get('sourceReferences', []))
            if not isinstance(source_references, list) or any(not isinstance(ref, str) for ref in source_references):
                raise Incomplete('INVALID_NATIVE_SOURCE_REFERENCES')
            key = (kind, reference)
            original = {**origin, 'itemIndex': index, 'recordDigest': digest(record)}
            if key in entries:
                previous = entries[key]['record']
                if previous != record:
                    same_metadata = kind == 'SOURCE' and {k:v for k,v in previous.items() if k!='text'} == {k:v for k,v in record.items() if k!='text'}
                    if not same_metadata or ('text' in previous and 'text' in record):
                        raise Incomplete('EVIDENCE_REFERENCE_RECORD_CHANGED: ' + reference)
                    if 'text' in record:
                        entries[key]['record'] = record
                entries[key]['origins'].append(original)
            else:
                entries[key] = {'kind': kind, 'reference': reference, 'id': row['id'], 'record': record,
                                'sourceReferences': source_references, 'origins': [original]}
    if identity is None:
        raise Incomplete('WORK_PAGE_REQUIRED_WITH_SOURCE_PARTS')
    for reference, chunks in parts.items():
        first = chunks[0][0]
        required = ('snapshot', 'sourceId', 'recordDigest', 'source', 'totalTextBytes')
        for part, _ in chunks:
            if part.get('snapshot') != identity['snapshot'] or any(part.get(k) != first.get(k) for k in required):
                raise Incomplete('SOURCE_PART_IDENTITY_CHANGED')
            text = part.get('text')
            if not isinstance(text, str) or part.get('endByte') != part.get('startByte', -1) + len(text.encode()) or part.get('fragmentDigest') != 'sha256:' + hashlib.sha256(text.encode()).hexdigest():
                raise Incomplete('SOURCE_PART_RANGE_OR_DIGEST_CHANGED')
        unique = {}
        for part, origin in chunks:
            start = part['startByte']
            if start in unique and unique[start][0] != part:
                raise Incomplete('SOURCE_PART_OVERLAP_CHANGED')
            unique[start] = (part, origin)
        ordered = [unique[key] for key in sorted(unique)]
        source_start = ordered[0][0]['startByte']
        end = source_start
        for part, _ in ordered:
            if part['startByte'] != end:
                raise Incomplete('SOURCE_PART_GAP_OR_OVERLAP')
            end = part['endByte']
        complete = source_start == 0 and ordered[-1][0]['nextCursor'] is None and end == first['totalTextBytes']
        record = {**first['source'], 'text': ''.join(part['text'] for part, _ in ordered)}
        if record.get('id') != first['sourceId']:
            raise Incomplete('SOURCE_PART_RECORD_ID_CHANGED')
        if end > first['totalTextBytes']:
            raise Incomplete('SOURCE_PART_EXCEEDS_TOTAL')
        if complete and (record.get('textDigest') != 'sha256:' + hashlib.sha256(record['text'].encode()).hexdigest() or digest(record) != first['recordDigest']):
            raise Incomplete('SOURCE_PART_FULL_TEXT_DIGEST_CHANGED')
        key = ('SOURCE', reference)
        if key in entries:
            full = entries[key]['record']
            if {k: v for k, v in full.items() if k != 'text'} != first['source'] or ('text' in full and full['text'].encode()[source_start:end] != record['text'].encode()):
                raise Incomplete('SOURCE_PART_CONFLICTS_WITH_ARCHIVED_SOURCE')
            if isinstance(full.get('text'), str):
                continue
        entries[key] = {'kind': 'SOURCE', 'reference': reference, 'id': first['sourceId'], 'record': record,
                        'partial': not complete, 'sourceStartByte': source_start, 'sourceEndByte': end, 'totalTextBytes': first['totalTextBytes'],
                        'nativePartNextCursor': ordered[-1][0]['nextCursor'], 'sourceReferences': [],
                        'origins': [origin for _, origin in ordered]}
    for omitted in omissions:
        reference = omitted.get('reference')
        if omitted.get('kind') == 'SOURCE' and isinstance(reference, str) and ('SOURCE', reference) not in entries:
            entries[('SOURCE', reference)] = {'kind': 'SOURCE', 'reference': reference, 'id': omitted.get('id'),
                'record': {'id': omitted.get('id')}, 'sourceReferences': [], 'origins': [], 'omitted': True}
    known_sources = {ref for entry in entries.values() for ref in entry['sourceReferences']}
    for ref in known_sources:
        if ('SOURCE', ref) not in entries:
            entries[('SOURCE', ref)] = {'kind': 'SOURCE', 'reference': ref, 'id': None, 'record': {}, 'sourceReferences': [], 'origins': []}
    selected = sorted((entry for entry in entries.values() if not references or entry['reference'] in references), key=lambda e: (e['reference'], e['kind']))
    unknown = sorted(set(references) - {entry['reference'] for entry in selected})
    found = []
    for entry in selected:
        record = entry['record']
        if entry['kind'] == 'SOURCE':
            content = record.get('text')
            availability = ('SOURCE_METADATA_ONLY' if record else 'NOT_YET_ARCHIVED_TEXT') if not isinstance(content, str) else 'PARTIAL_SOURCE_TEXT' if entry.get('partial') else 'COMPLETE_SOURCE_TEXT'
        else:
            content = json.dumps({'symbol': record.get('symbol'), 'normalized': record.get('normalized', {})}, ensure_ascii=False, sort_keys=True)
            availability = 'NAVIGATION_ONLY' if entry['kind'] == 'SYMBOL_NAVIGATION' else 'SYMBOL_RECORD_ONLY'
        matched = isinstance(content, str) and (not query or query in content)
        if query and not matched:
            continue
        raw_content = (content or '').encode()
        start = offset if offset is not None and len(references) == 1 else max(0, raw_content.find(query.encode()) - max_bytes // 4) if query else 0
        start = min(start, len(raw_content))
        while start and start < len(raw_content) and raw_content[start] & 0xc0 == 0x80:
            start += 1
        text = raw_content[start:start + max_bytes].decode('utf-8', errors='ignore')
        end = start + len(text.encode())
        metadata = {key: record.get(key) for key in ('id', 'file', 'startLine', 'endLine', 'revision', 'authority', 'textDigest', 'evidenceDigest', 'occurrence', 'symbol') if key in record}
        if entry['kind'] != 'SOURCE':
            metadata.update(authority=record.get('authority', record.get('normalized', {}).get('authority')), sourceIds=record.get('sourceIds', []))
        bindings = []
        for reference in entry['sourceReferences'][:16]:
            linked = entries.get(('SOURCE', reference), {}).get('record', {})
            if entry['kind'] == 'SYMBOL' and linked.get('id') and linked['id'] not in record.get('sourceIds', []):
                raise Incomplete('SYMBOL_SOURCE_BINDING_CHANGED')
            bindings.append({'reference': reference, **{key: linked.get(key) for key in ('id', 'file', 'startLine', 'endLine', 'revision', 'authority') if key in linked},
                             'sourceTextInArchive': isinstance(linked.get('text'), str)})
        found.append({'kind': entry['kind'], 'reference': entry['reference'], 'id': entry['id'], 'native': metadata,
                      'sourceReferences': entry['sourceReferences'][:16], 'sourceBindings': bindings, 'sourceReferencesNotShown': max(0, len(entry['sourceReferences'])-16), 'textAvailability': availability,
                      'view': {'text': text, 'startByte': start, 'endByte': end, 'availableBytes': len(raw_content),
                               'remainingAvailableBytes': len(raw_content) - end, 'nextOffset': end if end < len(raw_content) else None},
                      'nativePartNextCursor': entry.get('nativePartNextCursor'),
                      'nativeSourceOmitted': entry.get('omitted',False),
                      'nextNativeRead': {'command':'docs work read-part', 'input':{'schema':'codeclew-documentation-source-part-request/1.0','reference':entry['reference'], **({'cursor':entry['nativePartNextCursor']} if entry.get('nativePartNextCursor') else {})}} if entry['kind']=='SOURCE' and (not isinstance(content,str) or entry.get('partial')) else None,
                      'sourceWindow': {'startByte': entry.get('sourceStartByte',0), 'endByte': entry.get('sourceEndByte',len(raw_content)),
                                       'missingLeadingBytes': entry.get('sourceStartByte',0), 'missingTrailingBytes': entry.get('totalTextBytes',len(raw_content))-entry.get('sourceEndByte',len(raw_content))}, 'totalSourceTextBytes': entry.get('totalTextBytes', len(raw_content)),
                      'origins': entry['origins'][:8], 'originsNotShown': max(0, len(entry['origins'])-8), 'modelDelivery': 'UNATTESTED', 'modelRead': 'UNATTESTED'})
    if cursor > len(found):
        raise Incomplete('VIEW_CURSOR_OUT_OF_RANGE')
    complete_selected = bool(references) and not unknown and bool(selected) and all(e['kind'] == 'SOURCE' and isinstance(e['record'].get('text'), str) and not e.get('partial') for e in selected)
    state = 'FOUND_IN_ARCHIVED_EVIDENCE' if found else 'ABSENT_FROM_COMPLETE_SELECTED_SOURCE_TEXT' if complete_selected else 'NOT_FOUND_IN_AVAILABLE_TEXT'
    return {'schema': 'codeclew-work-evidence-view/1.0', 'status': state, 'identity': identity,
            'scope': 'EXPLICIT_SAVED_WORK_PAGES_ONLY', 'modelDelivery': 'UNATTESTED', 'modelRead': 'UNATTESTED',
            'totalMatches': len(found), 'items': found[cursor:cursor + limit],
            'nextCursor': cursor + limit if cursor + limit < len(found) else None,
            'unknownReferences': unknown[:16], 'unknownReferencesNotShown': max(0,len(unknown)-16),
            'textNotYetArchived': [e['reference'] for e in selected if e['kind'] == 'SOURCE' and not isinstance(e['record'].get('text'), str)][:16],
            'textNotYetArchivedCount': sum(e['kind']=='SOURCE' and not isinstance(e['record'].get('text'),str) for e in selected),
            'omitted': omissions[:16], 'omittedCount': len(omissions), 'omittedNotShown': max(0,len(omissions)-16),
            'savedPageContinuations': continuations[:16], 'savedPageContinuationsNotShown': max(0,len(continuations)-16),
            'collectionGaps': collection_gaps[:16], 'collectionGapsNotShown': max(0,len(collection_gaps)-16),
            'nextAction': 'Open exact original rows via origins; use nextOffset for remaining view text or native source-part cursor for missing source bytes. Native saved-page presence is not model delivery or reading. A negative search is scoped to these archived records, not the service.'}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--input-root', type=Path, required=True)
    parser.add_argument('--work', required=True)
    parser.add_argument('--page', type=Path, action='append', default=[])
    parser.add_argument('--collection', type=Path, action='append', default=[])
    parser.add_argument('--reference', action='append', default=[])
    parser.add_argument('--query', default='')
    for name, default in [('cursor', 0), ('limit', 8), ('offset', None), ('max-bytes', 2048)]:
        parser.add_argument('--' + name, type=int, default=default)
    args = parser.parse_args()
    try:
        if not 1 <= args.limit <= 32 or not 4 <= args.max_bytes <= 8192 or args.cursor < 0 or (args.offset is not None and args.offset < 0) or len(args.reference) > 8 or len(args.query.encode()) > 512:
            raise Incomplete('VIEW_LIMIT_OUT_OF_RANGE')
        result = view(args.input_root, args.work, args.page, args.collection, references=args.reference,
                      query=args.query, cursor=args.cursor, limit=args.limit, offset=args.offset, max_bytes=args.max_bytes)
    except (OSError, ValueError, TypeError, KeyError, RecursionError) as error:
        result = {'status': 'BLOCKED', 'reason': str(error), 'nativeCalls': 0}
    print(json.dumps(result, ensure_ascii=False, separators=(',', ':')))
    return 2 if result['status'] == 'BLOCKED' else 0


if __name__ == '__main__':
    raise SystemExit(main())
