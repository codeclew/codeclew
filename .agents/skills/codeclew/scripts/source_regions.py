#!/usr/bin/env python3
"""Compact C# source with source-bound joins and exit scopes from native Work.

Select returned CSHARP_SOURCE_REGIONS IDs. This is a display projection of
validated native records, never a parser, exception dispatcher or semantic CFG.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import runpy

P = runpy.run_path(str(Path(__file__).with_name('evidence_packet.py')))
Packet, BoundSource, Stop, encode = (P[k] for k in ('Packet', 'BoundSource', 'Stop', 'encode'))


def require(ok, status):
    if not ok:
        raise Stop(status, exit_code=2)


def project(items):
    require(1 <= len(items) <= 16, 'BOUNDED_REGION_SELECTION_REQUIRED')
    labels = {item['owner']['id']: chr(65+i) for i, item in enumerate(items)}
    require(len(labels) == len(items), 'DUPLICATE_REGION_OWNER')
    targets = {}
    for item in items:
        targets.setdefault(item['owner']['symbol'], []).append(labels[item['owner']['id']])
    out = ['Selected C# source. /*n*/ names a source region; /*→B*/ links an exact call target to body B.',
           'Joins apply only if a region completes normally. Exception selection, await success and callee completion are unproved.']
    metadata = {'schema': 'codeclew-csharp-source-region-view/1.0', 'callables': [], 'wholeProgramComplete': False}
    for item in items:
        record, owner, source = (item[k] for k in ('record', 'owner', 'source'))
        n = record['normalized']; g = n['graph']; rel = n['relationships']; site = n['sourceSite']
        label = labels[owner['id']]; raw = source['text'].encode()
        require(record['kind'] == 'CSHARP_SOURCE_REGIONS' and n['schema'] == 'codeclew-csharp-source-regions-evidence/1.0'
                and g['schema'] == 'codeclew-csharp-source-regions/1.0' and g['authority'] == 'ROSLYN_SOURCE_REGIONS'
                and g['execution'] == 'NOT_ASSERTED' and g['interpretation'] == 'LEXICAL_AND_CONDITIONAL_NORMAL_ONLY', 'REGION_AUTHORITY_MISMATCH')
        require(n['ownerId'] == owner['id'] and n['ownerSymbolIdentity'] == owner['symbol'] == g['ownerSymbolIdentity']
                and n['scope'] == owner['normalized']['scope'] and site == owner['normalized']['outlineOwnerSource']
                and record['service'] == owner['service'] == source['service']
                and source['id'] in owner['sourceIds'] and record['sourceIds'] == [source['id']]
                and site['sourceId'] == source['id'] and site['file'] == source['file'] == g['file'] == n['file']
                and n['compilerFactBinding'] == source['evidenceDigest'] == site['evidenceDigest']
                and source['textDigest'] == site['sourceDigest'] == 'sha256:'+hashlib.sha256(raw).hexdigest()
                and site['byteEnd']-site['byteStart'] == len(raw), 'REGION_SOURCE_BINDING_CHANGED')
        nodes = g['nodes']; base = site['byteStart']; end = site['byteEnd']
        require(nodes and len(nodes)+len(g['sites'])+len(g['expressions']) <= 96, 'REGION_BUDGET_CHANGED')
        def span(s):
            require(s['file'] == source['file'] and base <= s['byteStart'] < s['byteEnd'] <= end, 'REGION_OUTSIDE_SOURCE')
            raw[:s['byteStart']-base].decode(); raw[:s['byteEnd']-base].decode()
        for i, node in enumerate(nodes):
            span(node['source']); parent = node['parent']
            require(node['id'] == i and (parent is None and i == 0 or type(parent) is int and 0 <= parent < i), 'REGION_PARENT_CHANGED')
            if parent is not None:
                p = nodes[parent]['source']; s = node['source']
                require(p['byteStart'] <= s['byteStart'] < s['byteEnd'] <= p['byteEnd'], 'REGION_CONTAINMENT_CHANGED')
        for s in g['sites'] + g['expressions']:
            span(s['source'])
        # Mark only references used by the compact structural notes, not every
        # expression or compiler identity. The original bytes remain reversible.
        marked, notes = set(), []
        def ref(i):
            require(type(i) is int and 0 <= i < len(nodes), 'REGION_REFERENCE_CHANGED')
            marked.add(i); return str(i)
        for node in nodes:
            if node['kind'] != 'TRY':
                continue
            ids = {node['id']} | {x['id'] for x in nodes if x['parent'] == node['id'] and x['role'] in ('TRY_BODY', 'CATCH')}
            joins = [x for x in rel['normalContinuation'] if x['from'] in ids and type(x['to']) is int]
            for target in sorted({x['to'] for x in joins}):
                starts = sorted(x['from'] for x in joins if x['to'] == target and nodes[x['from']]['role'] != 'TRY_BODY')
                if starts:
                    notes.append('If normal: '+','.join(ref(i) for i in starts)+' → '+ref(target)+'.')
        for exit in rel['abrupt']:
            kind = exit['kind'].lower(); target = exit['targetScope']
            destination = label+' exit' if kind == 'return' else 'unknown exception handler' if kind == 'throw' else 'loop '+ref(target)
            if kind == 'continue':
                destination += ' ('+exit['continuePhase'].lower().replace('_', ' ')+')'
            cleanup = exit['unknownCleanupBoundaries']
            notes.append(ref(exit['from'])+' '+kind+' → '+destination+('; cleanup unresolved: '+','.join(ref(i) for i in cleanup) if cleanup else '')+'.')
        awaiting = {}
        for context in rel['siteContexts']:
            if g['sites'][context['site']]['kind'] == 'AWAIT':
                awaiting.setdefault(context['region'], set()).add(context['protectedTry'])
        for region, protected in awaiting.items():
            chain = next(c['enclosingRegions'] for c in rel['siteContexts'] if c['region'] == region and g['sites'][c['site']]['kind'] == 'AWAIT')
            caught = next((i for i in chain if nodes[i]['kind'] == 'CATCH'), None)
            notes.append('Await '+ref(region)+': '+('in catch '+ref(caught)+'; ' if caught is not None else '')+', '.join('no enclosing try body' if p is None else 'in try '+ref(p) for p in sorted(protected, key=lambda x: -1 if x is None else x))+'.')
        for node in nodes:
            if node.get('opaque') or node.get('scopeCleanup'):
                reason = node.get('opaque') or node['scopeCleanup']
                notes.append(ref(node['id'])+': '+reason.lower().replace('_', ' ')+'.')
        for region, boundary in sorted({(s['region'], s['boundary']) for s in g['sites'] if s['kind'] == 'OPAQUE'}):
            notes.append(ref(region)+': '+boundary.lower().replace('_', ' ')+'.')
        edits = []
        for i in sorted(marked):
            edits.append({'offset': nodes[i]['source']['byteStart']-base, 'text': '/*'+str(i)+'*/', 'node': i})
        for index, call in enumerate(g['sites']):
            choices = targets.get(call.get('target'), [])
            if call['kind'] in ('CALL', 'CONSTRUCT') and call.get('resolution') == 'COMPILER_EXACT_TARGET_NOT_RUNTIME_DISPATCH' and len(choices) == 1:
                edits.append({'offset': call['source']['byteStart']-base, 'text': '/*→'+choices[0]+'*/', 'site': index})
        edits.sort(key=lambda e: (e['offset'], e['text']))
        display, cursor, displayed = bytearray(), 0, []
        for edit in edits:
            offset = edit['offset']; raw[:offset].decode()
            display.extend(raw[cursor:offset]); start = len(display); display.extend(edit['text'].encode())
            displayed.append({**edit, 'displayStart': start, 'displayEnd': len(display)})
            cursor = offset
        display.extend(raw[cursor:]); restored = bytes(display)
        for edit in reversed(displayed):
            restored = restored[:edit['displayStart']]+restored[edit['displayEnd']:]
        require(restored == raw, 'REGION_DISPLAY_NOT_REVERSIBLE')
        out.extend(['', label+' · '+owner['normalized'].get('name', 'callable')+' · '+source['file'], '```csharp', display.decode(), '```', *notes])
        metadata['callables'].append({**item, 'label': label, 'displayEdits': displayed})
    return '\n'.join(out)+'\n', metadata


def acquire(args):
    packet = Packet(args)
    coverage, _ = packet.call('coverage', ['docs', 'context', '--root', args.root, '--service', args.service, '--snapshot', args.snapshot, '--format', 'compact', '--limit', '1'])
    packet.validate_page(coverage, 'codeclew-docs-page/1.0')
    rows = [r for r in coverage['items'] if r.get('kind') == 'COVERAGE' and r.get('id') == args.service]
    require(len(rows) == 1, 'MISSING_SERVICE_REVISION')
    selected = json.loads(args.selection.read_bytes())
    require(isinstance(selected, list) and 1 <= len(selected) <= 16 and all(isinstance(s, dict) and set(s) == {'regionId'} for s in selected), 'EXPLICIT_REGION_SELECTION_REQUIRED')
    require(len({s['regionId'] for s in selected}) == len(selected), 'DUPLICATE_REGION_SELECTION')
    items = []
    for selection in selected:
        record, artifact = packet.dependency(selection['regionId'])
        require(record.get('id') == selection['regionId'] and record.get('kind') == 'CSHARP_SOURCE_REGIONS' and record.get('service') == args.service, 'REGION_SCOPE_MISMATCH')
        owner, owner_artifact = packet.dependency(record['normalized']['ownerId'])
        require(owner.get('kind') == 'SYMBOL', 'REGION_OWNER_MISMATCH')
        references = packet.navigation(owner); sid = record['normalized']['sourceSite']['sourceId']
        # Every native reference confirms its returned ID; never infer a handle.
        source = None
        for reference in references:
            reader = BoundSource.__new__(BoundSource)
            reader.args = args; reader.work = args.work; reader.parts = []
            reader.binding = {'revision': rows[0]['revision']}; reader.call = packet.call
            try:
                candidate = reader.source(sid, reference)
            except Stop as error:
                if error.status == 'SOURCE_PART_IDENTITY_MISMATCH':
                    continue
                raise
            source = candidate; break
        require(source is not None, 'REGION_SOURCE_NOT_RETURNED')
        items.append({'record': record, 'owner': owner, 'source': source, 'dependencyArtifact': artifact, 'ownerArtifact': owner_artifact})
    text, metadata = project(items)
    metadata.update(work=args.work, snapshot=args.snapshot, service=args.service, nativeReceipts=packet.receipts, coverage=coverage)
    (args.output_dir/'author.txt').write_bytes(text.encode())
    (args.output_dir/'metadata.json').write_text(encode(metadata), encoding='utf-8')
    (args.output_dir/'direct-source.txt').write_bytes(b'\n'.join(item['source']['text'].encode() for item in items))
    measured = {'bodies': len(items), 'regions': sum(len(i['record']['normalized']['graph']['nodes']) for i in items),
                'sites': sum(len(i['record']['normalized']['graph']['sites']) for i in items), 'authorBytes': len(text.encode()),
                'sourceBytes': sum(len(i['source']['text'].encode()) for i in items), 'nativeCommands': len(packet.receipts),
                'nativeSeconds': sum(r['elapsedSeconds'] for r in packet.receipts), 'tokens': 'UNMEASURED', 'executionOrCoverageProved': False}
    (args.output_dir/'measurement.json').write_text(encode(measured), encoding='utf-8')
    return measured


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('clew', 'root', 'service', 'snapshot', 'work'):
        parser.add_argument('--'+key, required=True)
    parser.add_argument('--selection', type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args(); args.output_dir.mkdir(parents=True, exist_ok=False)
    try:
        print(encode(acquire(args)), end=''); return 0
    except (Stop, ValueError, KeyError, TypeError, UnicodeError, OSError) as error:
        result = {'status': error.status if isinstance(error, Stop) else 'INVALID_REGION_INPUT', 'details': str(error)}
        (args.output_dir/'failure.json').write_text(encode(result), encoding='utf-8'); print(encode(result), end=''); return 2


if __name__ == '__main__':
    raise SystemExit(main())
