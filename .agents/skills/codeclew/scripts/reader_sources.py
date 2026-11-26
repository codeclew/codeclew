"""Materialize cited excerpts from checked page maps; never bind or acquire source."""
import argparse
from collections import defaultdict
import json
from pathlib import Path
import runpy

M = runpy.run_path(str(Path(__file__).with_name('page_citations.py')))
C = M['C']
need, read, sha, digest = (M[k] for k in ('require', 'read', 'sha', 'digest'))


def checked_ranges(excerpts):
    """Keep exact byte occurrences and reject contradictory excerpts."""
    by_line, by_capture = defaultdict(dict), defaultdict(dict)
    for source, text in excerpts:
        lines = (source['startLine'], source['endLine'])
        row = {'startLine': lines[0], 'endLine': lines[1], 'text': text}
        coordinates = ()
        if (occurrence := source.get('occurrence')) is not None:
            need(isinstance(occurrence, dict), 'INVALID_SOURCE_OCCURRENCE')
            snapshot, blob, start, end = (occurrence.get(k) for k in ('snapshot', 'blob', 'startByte', 'endByte'))
            raw = text.encode('utf8')
            need(isinstance(snapshot, str) and snapshot and isinstance(blob, str) and blob
                 and type(start) is int and type(end) is int and 0 <= start <= end
                 and end - start == len(raw), 'INVALID_SOURCE_OCCURRENCE')
            coordinates = (snapshot, blob, start, end)
            row['occurrence'] = dict(zip(('snapshot', 'blob', 'startByte', 'endByte'), coordinates))
            spans = by_capture[snapshot, blob]
            need((start, end) not in spans or spans[start, end] == raw, 'SOURCE_EXCERPT_CONFLICT')
            spans[start, end] = raw
        ranges = by_line[lines]
        need(coordinates not in ranges or ranges[coordinates]['text'] == text, 'SOURCE_EXCERPT_CONFLICT')
        ranges[coordinates] = row

    for ranges in by_line.values():
        # Line labels alone cannot distinguish disagreeing captures or legacy excerpts.
        if len({row['text'] for row in ranges.values()}) > 1:
            need(() not in ranges and len({key[:2] for key in ranges}) == 1, 'SOURCE_EXCERPT_CONFLICT')
    for spans in by_capture.values():
        # By start order, the prior span reaching furthest covers any current overlap.
        prior_start, prior_end, prior = 0, 0, b''
        for (start, end), raw in sorted(spans.items()):
            if start < prior_end:
                overlap = min(prior_end, end) - start
                offset = start - prior_start
                need(prior[offset:offset + overlap] == raw[:overlap], 'SOURCE_EXCERPT_CONFLICT')
            if end > prior_end:
                prior_start, prior_end, prior = start, end, raw
    return [row for _, ranges in sorted(by_line.items()) for _, row in sorted(ranges.items())]


def materialize(index, manifests, output, views=()):
    index, output = Path(index).resolve(), Path(output)
    value = M['json_file'](index)
    if 'checkedIndex' in value:
        read(index.parent, value['checkedIndex'])
        index = index.parent / value['checkedIndex']
        value = M['json_file'](index)
    need(value.get('schema') == M['INDEX'] and value['indexDigest'] == digest({k:v for k,v in value.items() if k != 'indexDigest'}), 'INVALID_PAGE_INDEX_DIGEST')
    root = index.parent
    mapping_raw = read(root, value['repositories'])
    need(sha(mapping_raw) == value['repositoriesDigest'], 'PAGE_REPOSITORIES_DIGEST_MISMATCH')
    mapping, entries, owners = json.loads(mapping_raw), {}, {}
    for manifest in manifests:
        _, _, _, found = C['load'](Path(manifest))
        need(not entries.keys() & found.keys(), 'AMBIGUOUS_CITATION_MANIFEST')
        entries.update(found); owners.update({h:manifest for h in found})
    pages, used, used_declared = [], {}, {}
    for p in value['pages']:
        body, raw = read(root, p['markdown']), read(root, p['map'], M['MAX_MAP'])
        mapped = json.loads(raw)
        need(sha(body) == p['markdownDigest'] == mapped['markdownDigest'], 'PAGE_MARKDOWN_DIGEST_MISMATCH')
        need(mapped['mapDigest'] == p['mapDigest'] == digest({k:v for k,v in mapped.items() if k != 'mapDigest'}), 'PAGE_MAP_DIGEST_MISMATCH')
        handles, declared_handles = set(), set()
        for claim in mapped['claims']:
            loc = claim['location']
            need(body[loc['startByte']:loc['endByte']].decode('utf8') == claim['text'], 'BOUND_SELECTION_LOCATION_CHANGED')
            if 'declaredEvidence' in claim:
                need(mapped.get('schema')==M['TYPED_MAP'] and 'evidence' not in claim
                     and len(claim['declaredEvidence'])==len(claim['declarations']), 'DECLARED_BINDING_UNAVAILABLE')
                for handle, declaration in zip(claim['declaredEvidence'],claim['declarations']):
                    need(handle in entries and C['declared_module']()['binding'](entries[handle])==declaration,
                         'DECLARED_BINDING_UNAVAILABLE')
                    used_declared[handle] = declaration; declared_handles.add(handle)
                continue
            need(len(claim['evidence']) == len(claim['sources']), 'CITATION_BINDING_UNAVAILABLE')
            for handle, source in zip(claim['evidence'], claim['sources']):
                need(handle in entries and C['mapped_source'](entries[handle], mapping) == source, 'CITATION_BINDING_UNAVAILABLE')
                used[handle] = source; handles.add(handle)
        pages.append({'page':p['page'], 'markdownDigest':p['markdownDigest'], 'map':str(root / p['map']),
                      'mapFileDigest':sha(raw), 'handles':sorted(handles)})
        if declared_handles:
            pages[-1]['declarations'] = [{'cite':h,'record':used_declared[h]} for h in sorted(declared_handles)]
    texts = {}
    for path in views:
        view = json.loads(Path(path).read_bytes())
        for row in view['records']:
            h = row['cite']
            if h not in used: continue
            source = used[h]
            need(row['location'] == f"{source['service']}:{source['file']}:{source['startLine']}-{source['endLine']}", 'SOURCE_VIEW_IDENTITY_MISMATCH')
            raw = view['texts'][row.get('text', row.get('textKey'))].encode('utf8')
            if 'byteSlice' in row:
                a,b = (row['byteSlice'][k] for k in ('startByte','endByte'))
                need(type(a) is int and type(b) is int and 0 <= a <= b <= len(raw), 'INVALID_SOURCE_BYTE_SLICE')
                raw = raw[a:b]
            text = raw.decode('utf8')
            need(sha(raw) == entries[h]['source']['textDigest'], 'SOURCE_VIEW_TEXT_DIGEST_MISMATCH')
            need(h not in texts or texts[h] == text, 'SOURCE_EXCERPT_CONFLICT')
            texts[h] = text
    if views:
        need(used.keys() <= texts.keys(), 'CITED_SOURCE_VIEW_MISSING')
    else:
        for manifest in manifests:
            handles = sorted(h for h in used if owners[h] == manifest)
            for offset in range(0, len(handles), 256):
                batch = handles[offset:offset+256]
                result = C['grouped_sources'](Path(manifest), [{'file':used[h]['file'],'cite':h,'startLine':used[h]['startLine'],'endLine':used[h]['endLine']} for h in batch])
                groups = {g['id']:g for g in result['groups']}
                for item in result['views']:
                    h = item['cite']
                    text = groups[item['group']]['fragments'][item['fragment']]['text']
                    need(sha(text.encode('utf8')) == entries[h]['source']['textDigest'], 'SOURCE_VIEW_TEXT_DIGEST_MISMATCH')
                    texts[h] = text
    for page in pages:
        groups = {}
        for h in page.pop('handles'):
            source = used[h]
            key = tuple(source.get(k) for k in ('service','file','revision','snapshot')) + (
                source.get('evidenceInput'), source.get('bindingRepository') or source.get('repository'))
            groups.setdefault(key, []).append((source, texts[h]))
        page['groups'] = []
        for key, excerpts in sorted(groups.items(), key=lambda pair:tuple(str(x) for x in pair[0])):
            group = dict(zip(('service', 'file', 'revision', 'snapshot'), key[:4]))
            if key[4] is not None:
                group['evidenceInput'] = key[4]
                if key[5] is not None:
                    group['repository'] = key[5]
            group['ranges'] = checked_ranges(excerpts)
            page['groups'].append(group)
    index_reference = str(index)
    if any(s.get('evidenceInput') for s in [*used.values(),*used_declared.values()]):
        bundle_root = output.absolute().parent.resolve()
        need(index.is_relative_to(bundle_root)
             and all(Path(page['map']).is_relative_to(bundle_root) for page in pages),
             'SCOPED_SOURCE_BUNDLE_REQUIRES_LOCAL_BOUND_INDEX')
        index_reference = index.relative_to(bundle_root).as_posix()
        for page in pages:
            page['map'] = Path(page['map']).relative_to(bundle_root).as_posix()
    bundle = {'schema':'codeclew-reader-sources/1', 'checkedIndex':index_reference,
              'indexFileDigest':sha(index.read_bytes()),'pages':pages}
    with output.open('x', encoding='utf8') as stream: json.dump(bundle, stream, ensure_ascii=False, sort_keys=True, separators=(',',':'))
    return {'status':'SOURCE_FOOTERS_MATERIALIZED','pages':len(pages),'citations':len(used),
            **({'declaredCitations':len(used_declared)} if used_declared else {}),
            'output':str(output),'semanticTruthAssessed':False}


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--index', required=True, type=Path)
    p.add_argument('--manifest', action='append', required=True, type=Path)
    p.add_argument('--view', action='append', default=[], type=Path)
    p.add_argument('--output', required=True, type=Path)
    p.add_argument('--clew'); p.add_argument('--root'); p.add_argument('--evidence-inputs', type=Path)
    a = p.parse_args()
    try:
        M['configure_engine'](a.clew, a.root, evidence_inputs=a.evidence_inputs)
        print(json.dumps(materialize(a.index, a.manifest, a.output, a.view)))
    except M['Stop'] as error:
        print(json.dumps({'status':error.status})); raise SystemExit(2)
