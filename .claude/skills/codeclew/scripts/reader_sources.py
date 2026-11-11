"""Materialize cited excerpts from checked page maps; never bind or acquire source."""
import argparse
import json
from pathlib import Path
import runpy

M = runpy.run_path(str(Path(__file__).with_name('page_citations.py')))
C = M['C']
need, read, sha, digest = (M[k] for k in ('require', 'read', 'sha', 'digest'))


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
    pages, used = [], {}
    for p in value['pages']:
        body, raw = read(root, p['markdown']), read(root, p['map'], M['MAX_MAP'])
        mapped = json.loads(raw)
        need(sha(body) == p['markdownDigest'] == mapped['markdownDigest'], 'PAGE_MARKDOWN_DIGEST_MISMATCH')
        need(mapped['mapDigest'] == p['mapDigest'] == digest({k:v for k,v in mapped.items() if k != 'mapDigest'}), 'PAGE_MAP_DIGEST_MISMATCH')
        handles = set()
        for claim in mapped['claims']:
            loc = claim['location']
            need(body[loc['startByte']:loc['endByte']].decode('utf8') == claim['text'], 'BOUND_SELECTION_LOCATION_CHANGED')
            need(len(claim['evidence']) == len(claim['sources']), 'CITATION_BINDING_UNAVAILABLE')
            for handle, source in zip(claim['evidence'], claim['sources']):
                need(handle in entries and C['mapped_source'](entries[handle], mapping) == source, 'CITATION_BINDING_UNAVAILABLE')
                used[handle] = source; handles.add(handle)
        pages.append({'page':p['page'], 'markdownDigest':p['markdownDigest'], 'map':str(root / p['map']),
                      'mapFileDigest':sha(raw), 'handles':sorted(handles)})
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
                    texts[item['cite']] = groups[item['group']]['fragments'][item['fragment']]['text']
    for page in pages:
        groups = {}
        for h in page.pop('handles'):
            source = used[h]; text = texts[h]
            key = tuple(source.get(k) for k in ('service','file','revision','snapshot')) + (
                source.get('evidenceInput'), source.get('bindingRepository') or source.get('repository'))
            group = groups.setdefault(key, {'service':key[0],'file':key[1],'revision':key[2],'snapshot':key[3],'ranges':{}})
            if key[4] is not None:
                group['evidenceInput'] = key[4]
                if key[5] is not None:
                    group['repository'] = key[5]
            span = (source['startLine'],source['endLine'])
            need(span not in group['ranges'] or group['ranges'][span]['text'] == text, 'SOURCE_EXCERPT_CONFLICT')
            group['ranges'][span] = {'startLine':span[0],'endLine':span[1],'text':text}
        page['groups'] = []
        for key in sorted(groups, key=lambda k:tuple(str(x) for x in k)):
            group = groups[key]; group['ranges'] = [group['ranges'][s] for s in sorted(group['ranges'])]; page['groups'].append(group)
    index_reference = str(index)
    if any(s.get('evidenceInput') for s in used.values()):
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
    return {'status':'SOURCE_FOOTERS_MATERIALIZED','pages':len(pages),'citations':len(used),'output':str(output),'semanticTruthAssessed':False}


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
