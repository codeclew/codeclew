"""Typed frozen declaration bindings; these never satisfy SOURCE claims."""
from pathlib import Path
import runpy
import tempfile

S = runpy.run_path(str(Path(__file__).with_name('shared_catalog.py')))
SCHEMA = 'codeclew-author-declared-citations/1.0'
RELOADED = {}


def need(condition, status):
    if not condition:
        raise Stop(status, exit_code=2)


def load(path, manifest, alias, engine):
    try:
        return load_selected(path,manifest,alias,engine)
    except S['Stop'] as error:
        raise Stop(error.status,exit_code=2,**error.details) from error


def load_selected(path, manifest, alias, engine):
    need(alias is not None and engine is not None and manifest.get('input')==alias,
         'DECLARED_CITATIONS_REQUIRE_SELECTED_EVIDENCE_INPUT')
    need(set(manifest)=={'schema','input','records','id'} and manifest['schema']==SCHEMA
         and manifest['id']==digest({k:v for k,v in manifest.items() if k!='id'})
         and isinstance(manifest['records'],list) and 0<len(manifest['records'])<=4096,
         'INVALID_DECLARED_CITATION_MANIFEST')
    tag = 'd'+manifest['id'].split(':')[1][:12]; entries = {}
    for record in manifest['records']:
        need(set(record)=={'key','service','id','kind','localId','scope','title','work','snapshot',
                          'contextDigest','basis','authority','value'}, 'INVALID_DECLARED_CITATION_RECORD')
        need(isinstance(record['key'],str) and S['ID'].fullmatch(record['key']) is not None,
             'INVALID_DECLARED_CITATION_KEY')
        need(isinstance(record['basis'],dict) and isinstance(record['basis'].get('recordDigest'),str)
             and S['re'].fullmatch(r'sha256:[0-9a-f]{64}',record['basis']['recordDigest']) is not None,
             'INVALID_DECLARED_RECORD_DIGEST')
        name = record['value']
        need(name=='objects/'+record['basis']['recordDigest'].removeprefix('sha256:')+'.json'
             and S['re'].fullmatch(r'objects/[0-9a-f]{64}\.json',name) is not None
             and not (path.parent/'objects').is_symlink(), 'INVALID_DECLARED_VALUE_PATH')
        original = S['read_json'](path.parent/name)
        need(digest(original)==record['basis']['recordDigest'], 'DECLARED_VALUE_DIGEST_MISMATCH')
        clew, root = engine
        cache_key = (str(clew),str(root),record['service'],record['work'],record['snapshot'],record['contextDigest'])
        if cache_key not in RELOADED:
            try:
                with tempfile.TemporaryDirectory() as output:
                    reader = S['Reader'](clew,output)
                    current = reader.inventory(root,alias,{'id':record['service'],'snapshot':record['snapshot'],
                        'work':record['work'],'contextDigest':record['contextDigest']},None)
                    RELOADED[cache_key] = {item['id']:item['record'] for item in current['items']}
            except S['Stop'] as error:
                raise Stop(error.status,exit_code=2,**error.details) from error
        header = RELOADED[cache_key].get(record['id'])
        need(header is not None and all(header[k]==record[k]
             for k in ('kind','localId','scope','title','authority','basis')), 'DECLARED_NATIVE_BINDING_MISMATCH')
        handle = tag+':'+record['key']
        need(handle not in entries,'DUPLICATE_DECLARED_CITATION_KEY')
        entries[handle] = {'manifestId':manifest['id'],'basisKind':'DECLARED',
            'evidenceInput':alias,'service':record['service'],'work':record['work'],
            'snapshot':record['snapshot'],'contextDigest':record['contextDigest'],
            'id':record['id'],'kind':record['kind'],'localId':record['localId'],
            'title':record['title'],'scope':record['scope'],'authority':record['authority'],'basis':record['basis']}
    return manifest, {'kind':'DECLARED'}, {}, entries


def binding(entry):
    need(entry.get('basisKind')=='DECLARED', 'SOURCE_CANNOT_BIND_DECLARED_CLAIM')
    return {k:v for k,v in entry.items() if k!='manifestId'}
