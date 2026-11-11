#!/usr/bin/env python3
"""Collect, inspect and check explicitly selected frozen declared information."""
from __future__ import annotations

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import runpy
import subprocess
import sys
import time

C = runpy.run_path(str(Path(__file__).with_name('author_citations.py')))
Stop, digest, encode = (C[k] for k in ('Stop', 'digest', 'encode'))
COLLECTION = 'codeclew-semantic-collection/1.0'
INVENTORY = 'codeclew-semantic-inventory/1.0'
ID = re.compile(r'[A-Za-z0-9][A-Za-z0-9_-]{0,63}')
MAX_JSON = 8 * 1024 * 1024


def need(condition, status, **details):
    if not condition:
        raise Stop(status, exit_code=2, **details)


def read_json(path):
    path = Path(path)
    need(path.is_file() and not path.is_symlink() and path.stat().st_size <= MAX_JSON,
         'CATALOG_INPUT_UNAVAILABLE')
    def unique(pairs):
        need(len(pairs) == len({k for k, _ in pairs}), 'DUPLICATE_CATALOG_KEY')
        return dict(pairs)
    return json.loads(path.read_bytes(), object_pairs_hook=unique)


def write_json(path, value):
    raw = encode(value)
    need(len(raw.encode('utf8')) <= MAX_JSON, 'CATALOG_ARTIFACT_TOO_LARGE')
    with Path(path).open('x', encoding='utf8') as stream:
        stream.write(raw)


def collection(path):
    value = read_json(path)
    need(isinstance(value, dict) and value.get('schema') == COLLECTION
         and not set(value) - {'schema','id','revision','inputs'},
         'INVALID_SEMANTIC_COLLECTION')
    need(all(isinstance(value.get(k), str) and ID.fullmatch(value[k]) for k in ('id','revision')),
         'INVALID_COLLECTION_IDENTITY')
    inputs = value.get('inputs')
    need(isinstance(inputs, list) and 1 <= len(inputs) <= 64, 'INVALID_COLLECTION_INPUTS')
    seen = set(); selected = 0
    for item in inputs:
        need(isinstance(item, dict) and set(item) == {'id','docset','services'}
             and all(isinstance(item.get(k), str) and ID.fullmatch(item[k]) for k in ('id','docset'))
             and item['id'] not in seen, 'INVALID_COLLECTION_INPUT')
        seen.add(item['id']); services = item['services']
        need(isinstance(services, list) and 1 <= len(services) <= 64, 'INVALID_COLLECTION_SERVICES')
        names = set()
        for service in services:
            need(isinstance(service, dict) and not set(service) - {'id','snapshot','work','contextDigest'}
                 and isinstance(service.get('id'), str) and ID.fullmatch(service['id'])
                 and service['id'] not in names and isinstance(service.get('snapshot'), str)
                 and 0 < len(service['snapshot']) <= 256, 'INVALID_COLLECTION_SERVICE')
            need(('work' in service)==('contextDigest' in service)
                 and all(isinstance(service[k],str) and service[k] for k in ('work','contextDigest') if k in service),
                 'EXACT_WORK_CONTEXT_REQUIRED')
            names.add(service['id']); selected += 1
    need(selected <= 256, 'COLLECTION_SERVICE_LIMIT')
    return value


def roots(path, selected):
    value = read_json(path)
    need(isinstance(value, dict) and set(value) == {'schema','inputs','manifests'}
         and value['schema'] == 'codeclew-evidence-inputs/1.0'
         and isinstance(value['inputs'], dict) and isinstance(value['manifests'], dict),
         'INVALID_TRUSTED_CATALOG_ROOTS')
    result = {}
    for item in selected['inputs']:
        alias = item['id']; entry = value['inputs'].get(alias)
        need(isinstance(entry, dict) and set(entry) == {'root'}
             and isinstance(entry['root'], str) and Path(entry['root']).is_absolute(),
             'SELECTED_CATALOG_ROOT_NOT_CONFIGURED', input=alias)
        result[alias] = Path(entry['root'])
    return result


def pointer(value, path):
    need(isinstance(path, str) and path.startswith('/normalized/'), 'INVALID_DECLARED_RECORD_POINTER')
    for part in path.split('/')[1:]:
        part = part.replace('~1','/').replace('~0','~')
        value = value[int(part)] if isinstance(value, list) and part.isdecimal() else value[part]
    return value


class Reader:
    def __init__(self, clew, output):
        self.clew = Path(clew)
        need(self.clew.is_absolute() and self.clew.is_file(), 'CALLER_TRUSTED_CLEW_REQUIRED')
        self.output = Path(output); self.receipts = self.output / 'receipts'; self.receipts.mkdir()
        self.calls = []
        self.counters = {'nativeCalls':0,'prepareCalls':0,'inventoryPages':0,'dependencyPartCalls':0,
                         'sourceReadCalls':0,'captureCalls':0,'sourceMaterializationCalls':0,
                         'recordsImported':0,'recordsReused':0,'nativeStdoutBytes':0}

    def invoke(self, root, operation, arguments, request=None):
        need(tuple(operation) in {('work','prepare'),('work','read'),('work','read-dependency-part'),
                                  ('work','answer-context')},'CATALOG_ACQUISITION_OPERATION_NOT_ALLOWED')
        number = len(self.calls) + 1
        args = [str(self.clew), 'docs', *operation, '--root', str(root), *map(str, arguments)]
        if request is not None:
            path = self.receipts / f'{number:04d}.input.json'; write_json(path, request)
            args += ['--input', str(path)]
        started = time.monotonic()
        run = subprocess.run(args, capture_output=True, stdin=subprocess.DEVNULL)
        self.counters['nativeCalls'] += 1
        self.counters['nativeStdoutBytes'] += len(run.stdout)
        if operation == ['work','prepare']: self.counters['prepareCalls'] += 1
        if operation == ['work','read']: self.counters['inventoryPages'] += 1
        if operation == ['work','read-dependency-part']: self.counters['dependencyPartCalls'] += 1
        receipt = {'argv':args,'exitCode':run.returncode,'elapsedMs':round((time.monotonic()-started)*1000)}
        self.calls.append(receipt)
        for suffix, data in [('stdout',run.stdout),('stderr',run.stderr)]:
            (self.receipts / f'{number:04d}.{suffix}').write_bytes(data)
        write_json(self.receipts / f'{number:04d}.command.json',receipt)
        need(run.returncode == 0, 'RETAINED_CATALOG_READ_FAILED', operation=' '.join(operation),
             exitCode=run.returncode, receipt=f'receipts/{number:04d}.command.json')
        value = json.loads(run.stdout)
        need(isinstance(value,dict), 'INVALID_RETAINED_CATALOG_RESPONSE')
        return value

    def inventory(self, root, alias, service, prior):
        selected = None
        if service.get('work') or prior:
            work = service.get('work') or prior['work']
            context = service.get('contextDigest') or (prior or {}).get('contextDigest')
            need(isinstance(context,str) and context, 'EXACT_WORK_CONTEXT_REQUIRED', input=alias, service=service['id'])
            selected = {'query':{'kind':'SEMANTIC_INVENTORY','expectedSnapshot':service['snapshot'],
                                'expectedContextDigest':context}}
            page = self.invoke(root,['work','read'],['--work',work],selected)
        else:
            request = {'schema':'codeclew-documentation-work-request/1.0','audience':'Shared catalog maintainers',
                       'contextProfile':'semantic-inventory-v1','maxItems':100,'maxBytes':49152}
            page = self.invoke(root,['work','prepare'],['--subject','service:'+service['id'],'--snapshot',service['snapshot']],request)
            work = page.get('work'); context = page.get('contextDigest')
        need(isinstance(work,str) and isinstance(context,str) and page.get('subject')=='service:'+service['id'],
             'CATALOG_WORK_SCOPE_MISMATCH')
        items = []; membership = None; cursors = set(); scope = None; member_ids = []; total = None
        while True:
            need(page.get('schema')=='codeclew-documentation-work-page/1.0'
                 and page.get('snapshot')==service['snapshot']
                 and page.get('work')==work and page.get('contextDigest')==context
                 and not page.get('omitted') and isinstance(page.get('items'),list)
                 and page.get('receiptDigest')==digest({k:v for k,v in page.items() if k!='receiptDigest'})
                 and isinstance(page.get('membershipDigest'),str) and type(page.get('total')) is int,
                 'INCOMPLETE_SEMANTIC_INVENTORY', input=alias,service=service['id'])
            if membership is not None:
                need(page['membershipDigest']==membership and page['total']==total,'SEMANTIC_MEMBERSHIP_CHANGED')
            membership, total = page['membershipDigest'], page['total']
            for item in page['items']:
                member_ids.append([item['kind'],item['id']])
                if item.get('kind')=='SEMANTIC_SCOPE':
                    need(scope is None,'DUPLICATE_SEMANTIC_SCOPE'); scope = item['record']
                    need(scope.get('schema')=='codeclew-semantic-inventory-scope/1.0'
                         and scope.get('snapshot')==service['snapshot'] and scope.get('contextDigest')==context
                         and scope.get('service')==service['id'], 'CATALOG_SNAPSHOT_SCOPE_MISMATCH')
                else:
                    need(item.get('kind')=='SEMANTIC_INVENTORY','INVALID_SEMANTIC_INVENTORY_ITEM'); items.append(item)
            cursor = page.get('nextCursor')
            if cursor is None: break
            need(isinstance(cursor,str) and cursor not in cursors and len(cursors)<10000,'SEMANTIC_CURSOR_DID_NOT_ADVANCE')
            cursors.add(cursor); selection = dict(selected or {}); selection['cursor']=cursor
            page = self.invoke(root,['work','read'],['--work',work],selection)
        need(scope is not None,'SEMANTIC_INVENTORY_NOT_SUPPORTED', nextAction='Use a runtime with semantic-inventory-v1; do not recapture source.')
        need(scope.get('items')==len(items) and len({item['id'] for item in items})==len(items),
             'INCOMPLETE_SEMANTIC_INVENTORY')
        need(total==len(member_ids) and digest(member_ids)==membership,'INCOMPLETE_SEMANTIC_MEMBERSHIP')
        return {'input':alias,'service':service['id'],'snapshot':service['snapshot'],'work':work,
                'contextDigest':context,'scope':scope,'items':items}

    def dependency(self, root, work, basis):
        data = bytearray(); cursor = None; digest_value = None; total = None; seen = set()
        while True:
            request = {'schema':'codeclew-documentation-dependency-part-request/1.0',
                       'kind':'DEPENDENCY','id':basis['dependencyId'],'cursor':cursor}
            part = self.invoke(root,['work','read-dependency-part'],['--work',work],request)
            fragment = part.get('text','').encode('utf8')
            need(part.get('work')==work and part.get('snapshot')==basis['snapshot']
                 and part.get('receiptDigest')==digest({k:v for k,v in part.items() if k!='receiptDigest'})
                 and part.get('id')==basis['dependencyId'] and part.get('startByte')==len(data)
                 and part.get('endByte')==len(data)+len(fragment) and fragment
                 and part.get('fragmentDigest')=='sha256:'+hashlib.sha256(fragment).hexdigest(),
                 'DECLARED_PART_BINDING_MISMATCH')
            if digest_value is not None: need(part['recordDigest']==digest_value and part['totalRecordBytes']==total,'DECLARED_PART_BINDING_MISMATCH')
            digest_value, total = part['recordDigest'], part['totalRecordBytes']; data.extend(fragment)
            need(len(data)<=MAX_JSON,'DECLARED_RECORD_TOO_LARGE')
            cursor = part.get('nextCursor')
            if cursor is None: break
            need(cursor not in seen,'DECLARED_PART_DID_NOT_ADVANCE'); seen.add(cursor)
        need(len(data)==total and 'sha256:'+hashlib.sha256(data).hexdigest()==digest_value
             and digest_value==basis['dependencyRecordDigest'],'DECLARED_RECORD_DIGEST_MISMATCH')
        value = json.loads(data)
        need(value.get('id')==basis['dependencyId'] and value.get('digest')==basis['dependencyDigest'],
             'DECLARED_RECORD_IDENTITY_MISMATCH')
        return value


def project(kind, value, service):
    if kind=='EXTERNAL_SYSTEM':
        return {k:v for k,v in value.items() if k!='configurationReferences'}
    if kind=='EXTERNAL_USE': return value
    need(kind=='TERM_CANDIDATE','UNSUPPORTED_SEMANTIC_RECORD_KIND')
    return {**value,'relations':[r for r in value.get('relations',[]) if r.get('service')==service]}


def record_key(record):
    return (record['input'], record['service'], record['id'])


def record_selector(record):
    return '/'.join(record_key(record))


def load_inventory(directory):
    """Check saved bytes, not freshness or semantic truth; check() reloads native scope."""
    directory = Path(directory)
    need(directory.is_dir() and not directory.is_symlink(), 'CATALOG_INVENTORY_UNAVAILABLE')
    value = read_json(directory/'inventory.json')
    need(isinstance(value,dict) and value.get('schema') == INVENTORY
         and value.get('digest') == digest({k:v for k,v in value.items() if k!='digest'})
         and isinstance(value.get('services'),list) and len(value['services'])<=256
         and isinstance(value.get('records'),list) and len(value['records'])<=65536,
         'INVALID_CATALOG_INVENTORY')
    services = {(s['input'],s['service']):s for s in value['services']}
    need(len(services)==len(value['services']), 'DUPLICATE_CATALOG_SERVICE')
    if value['records']:
        need((directory/'objects').is_dir() and not (directory/'objects').is_symlink(), 'CATALOG_OBJECTS_UNAVAILABLE')
    seen = set()
    for record in value['records']:
        key = record_key(record); service = services.get(key[:2]); basis = record['basis']
        need(key not in seen and service is not None, 'DUPLICATE_OR_FOREIGN_CATALOG_RECORD')
        seen.add(key)
        need(record['work']==service['work'] and record['snapshot']==service['snapshot']
             and record['contextDigest']==service['contextDigest']
             and basis['snapshot']==record['snapshot'] and basis['contextDigest']==record['contextDigest']
             and basis['recordDigest']==record['recordDigest']
             and basis['dependencyRecordDigest']==record['dependencyRecordDigest']
             and isinstance(record['recordDigest'],str) and re.fullmatch(r'sha256:[0-9a-f]{64}',record['recordDigest']) is not None
             and re.fullmatch(r'[0-9a-f]{64}\.json',record['object']) is not None
             and record['object']==record['recordDigest'].removeprefix('sha256:')+'.json',
             'CATALOG_RECORD_BINDING_MISMATCH')
        original = read_json(directory/'objects'/record['object'])
        need(isinstance(original,dict) and digest(original)==record['recordDigest']
             and digest(project(record['kind'],original,record['service']))==record['projectionDigest'],
             'CACHED_DECLARATION_CHANGED')
    counts = Counter(record_key(r)[:2] for r in value['records'])
    need(all(counts[key]==s['scope']['items']
             for key,s in services.items()), 'INCOMPLETE_SAVED_SEMANTIC_INVENTORY')
    return value


def inspect(directory, selectors=()):
    """Bounded author view; immutable native identities stay in inventory.json."""
    value = load_inventory(directory)
    records = {record_selector(r):r for r in value['records']}
    need(len(selectors)<=64 and len(set(selectors))==len(selectors)
         and all(s in records for s in selectors), 'UNKNOWN_OR_AMBIGUOUS_CATALOG_SELECTION')
    if not selectors:
        return {'status':'SAVED_CATALOG_INVENTORY','complete':value['complete'],
            'records':[{'id':key,'title':r['title']} for key,r in records.items()],
            'issues':value['issues']}
    bodies, selected = {}, []
    for selector in selectors:
        record = records[selector]
        original = read_json(Path(directory)/'objects'/record['object'])
        projected = project(record['kind'],original,record['service'])
        identity = digest(projected)
        if identity not in bodies:
            bodies[identity] = {'id':'b'+str(len(bodies)+1),'value':projected}
        selected.append({'id':selector,'body':bodies[identity]['id']})
    return {'status':'SAVED_DECLARED_INFORMATION_NOT_SOURCE_PROOF','selected':selected,
            'bodies':list(bodies.values())}


def citations(directory, alias, output):
    value = load_inventory(directory)
    records = [r for r in value['records'] if r['input']==alias]
    need(value['complete'] and 0<len(records)<=4096,'COMPLETE_SELECTED_DECLARED_INPUT_REQUIRED')
    output = Path(output)
    need(not output.exists() and '.codeclew' not in output.parts
         and not output.resolve().is_relative_to(Path(directory).resolve()), 'INVALID_DECLARED_CITATION_OUTPUT')
    output.mkdir(); (output/'objects').mkdir()
    entries = []
    for index,record in enumerate(records):
        entry = {k:record[k] for k in ('service','id','kind','localId','scope','title','work','snapshot',
                                     'contextDigest','basis','authority')}
        entry.update(key='r'+str(index+1),value='objects/'+record['object']); entries.append(entry)
        destination = output/entry['value']
        if not destination.exists(): write_json(destination,read_json(Path(directory)/'objects'/record['object']))
    manifest = {'schema':'codeclew-author-declared-citations/1.0','input':alias,'records':entries}
    manifest['id'] = digest(manifest); write_json(output/'citations.json',manifest)
    tag = 'd'+manifest['id'].split(':')[1][:12]
    return {'status':'DECLARED_CITATIONS_SAVED','manifestId':manifest['id'],'input':alias,
        'records':[{'id':alias+'/'+r['service']+'/'+r['id'],'cite':tag+':'+r['key'],'title':r['title']} for r in entries],
        'nextAction':'Assign this manifestId to its input in caller-trusted evidence-inputs before binding; these handles cannot satisfy SOURCE claims.'}


def check(collection_path, roots_path, clew, directory, output):
    selected = collection(collection_path); locations = roots(roots_path,selected)
    value = load_inventory(directory)
    need(value['collectionDigest']==digest(selected), 'CATALOG_COLLECTION_CHANGED')
    output = Path(output)
    need(not output.exists() and '.codeclew' not in output.parts
         and not output.resolve().is_relative_to(Path(directory).resolve()), 'INVALID_CATALOG_CHECK_OUTPUT')
    output.mkdir(); reader = Reader(clew,output)
    services = {(s['input'],s['service']):s for s in value['services']}
    issues = list(value['issues']); checked = 0
    expected = {(i['id'],s['id']) for i in selected['inputs'] for s in i['services']}
    need(set(services)<=expected, 'FOREIGN_CATALOG_SERVICE')
    for item in selected['inputs']:
        alias, docset = item['id'], item['docset']
        for service in item['services']:
            try:
                need(locations[alias].is_dir() and not locations[alias].is_symlink(), 'SELECTED_ROOT_UNAVAILABLE')
                need((alias,service['id']) in services, 'CATALOG_SERVICE_INCOMPLETE')
                saved = services[(alias,service['id'])]
                need(saved['snapshot']==service['snapshot'], 'CATALOG_SNAPSHOT_SCOPE_MISMATCH')
                current = reader.inventory(locations[alias],alias,{**service,
                    'work':saved['work'],'contextDigest':saved['contextDigest']},None)
                need(current['scope']==saved['scope'], 'CATALOG_SERVICE_SCOPE_CHANGED')
                records = {r['id']:r for r in value['records'] if record_key(r)[:2]==(alias,service['id'])}
                need(set(records)=={r['id'] for r in current['items']}, 'CATALOG_RECORD_MEMBERSHIP_CHANGED')
                for row in current['items']:
                    old = records[row['id']]; header = row['record']
                    need(old['docset']==docset
                         and all(old[k]==header[k] for k in ('kind','localId','scope','title','authority','basis')),
                         'CATALOG_DECLARED_BINDING_CHANGED', record=row['id'])
                checked += len(records)
            except (Stop,OSError,ValueError,KeyError,TypeError) as error:
                issues.append({'input':alias,'service':service['id'],
                    'status':error.status if isinstance(error,Stop) else 'INVALID_RETAINED_CATALOG_INPUT'})
    result = {'status':'SEMANTIC_INVENTORY_CHECKED','complete':not issues,'recordsChecked':checked,
        'inventoryDigest':value['digest'],'issues':issues,'counters':reader.counters,
        'authority':'FROZEN_DECLARED_BINDINGS_NOT_SOURCE_OR_MEANING_APPROVAL'}
    write_json(output/'result.json',result)
    return result


def collect(collection_path, roots_path, clew, output, previous=None):
    selected = collection(collection_path); locations = roots(roots_path,selected)
    output = Path(output); need(not output.exists() and not output.is_symlink(),'CATALOG_OUTPUT_EXISTS')
    need('.codeclew' not in output.parts and
         (previous is None or not output.resolve().is_relative_to(Path(previous).resolve())),
         'CATALOG_OUTPUT_OVERLAPS_PRIVATE_OR_HISTORICAL_INPUT')
    output.mkdir(); reader = Reader(clew,output)
    prior = load_inventory(previous) if previous else None
    if prior:
        need(prior.get('schema')==INVENTORY and prior.get('collection')==selected['id']
             and prior.get('digest')==digest({k:v for k,v in prior.items() if k!='digest'}),'INVALID_PREVIOUS_CATALOG_INVENTORY')
    prior_services = {(s['input'],s['service']):s for s in (prior or {}).get('services',[])}
    prior_records = {(r['input'],r['service'],r['id']):r for r in (prior or {}).get('records',[])}
    services, records, issues, objects = [], [], [], {}
    for input_value in selected['inputs']:
        alias = input_value['id']; root = locations[alias]
        for service in input_value['services']:
            try:
                need(root.is_dir() and not root.is_symlink(),'SELECTED_ROOT_UNAVAILABLE',input=alias)
                old_service = prior_services.get((alias,service['id']))
                if old_service and old_service['snapshot']!=service['snapshot']: old_service = None
                current = reader.inventory(root,alias,service,old_service)
                service_records = []
                for item in current['items']:
                    header = item['record']; basis = header['basis']; kind = header['kind']
                    need(basis['snapshot']==service['snapshot'] and basis['contextDigest']==current['contextDigest'],
                         'DECLARED_ITEM_SCOPE_MISMATCH')
                    key = (alias,service['id'],item['id']); old = prior_records.get(key)
                    if old and old['kind']==kind and old['recordDigest']==basis['recordDigest']:
                        original = read_json(Path(previous)/'objects'/old['object'])
                        need(digest(original)==basis['recordDigest'],'CACHED_DECLARATION_CHANGED')
                        reader.counters['recordsReused'] += 1
                    else:
                        object_key = (alias,service['snapshot'],basis['dependencyId'],basis['dependencyRecordDigest'])
                        if object_key not in objects: objects[object_key] = reader.dependency(root,current['work'],basis)
                        original = pointer(objects[object_key],basis['pointer'])
                        need(digest(original)==basis['recordDigest'],'DECLARED_ITEM_DIGEST_MISMATCH')
                        reader.counters['recordsImported'] += 1
                    object_name = basis['recordDigest'].split(':',1)[1]+'.json'
                    directory = output/'objects'; directory.mkdir(exist_ok=True)
                    if not (directory/object_name).exists(): write_json(directory/object_name,original)
                    service_records.append({'input':alias,'docset':input_value['docset'],'service':service['id'],'id':item['id'],
                        'kind':kind,'localId':header['localId'],'scope':header['scope'],'title':header['title'],
                        'work':current['work'],'snapshot':service['snapshot'],'contextDigest':current['contextDigest'],
                        'recordDigest':basis['recordDigest'],'dependencyRecordDigest':basis['dependencyRecordDigest'],
                        'basis':basis,'object':object_name,'projectionDigest':digest(project(kind,original,service['id'])),
                        'authority':header['authority'],'definitionStatus':'CANDIDATE_NOT_SHARED_DEFINITION'})
                services.append({k:v for k,v in current.items() if k!='items'})
                records.extend(service_records)
            except (Stop,OSError,ValueError,KeyError,TypeError) as error:
                issues.append({'input':alias,'service':service['id'],'snapshot':service['snapshot'],
                    'status':error.status if isinstance(error,Stop) else 'INVALID_RETAINED_CATALOG_INPUT',
                    **({'details':error.details} if isinstance(error,Stop) else {})})
    previous_keys = set(prior_records); keys = {(r['input'],r['service'],r['id']) for r in records}
    successful = {(s['input'],s['service']) for s in services}
    removed = [dict(input=k[0],service=k[1],id=k[2]) for k in sorted(previous_keys-keys) if k[:2] in successful]
    value = {'schema':INVENTORY,'collection':selected['id'],'collectionRevision':selected['revision'],
        'collectionDigest':digest(selected),'complete':not issues,'services':services,'records':records,
        'issues':issues,'removed':removed,'counters':reader.counters,
        'authority':'FROZEN_DECLARED_CANDIDATES_NOT_SOURCE_OR_SEMANTIC_APPROVAL'}
    value['digest']=digest(value); write_json(output/'inventory.json',value)
    write_json(output/'result.json',{'status':'SEMANTIC_INVENTORY_COLLECTED','complete':not issues,
        'services':len(services),'records':len(records),'issues':issues,'removed':removed,'counters':reader.counters})
    return read_json(output/'result.json')


def main(argv=None):
    parser=argparse.ArgumentParser(description=__doc__); sub=parser.add_subparsers(dest='action',required=True)
    command=sub.add_parser('collect',help='Read selected frozen metadata; never captures or scans other roots')
    for name in ('collection','roots','clew','output-dir'):command.add_argument('--'+name,required=True,type=Path)
    command.add_argument('--previous-inventory',type=Path)
    command=sub.add_parser('check',help='Check saved records against their owning immutable native inputs')
    for name in ('collection','roots','clew','inventory','output-dir'):command.add_argument('--'+name,required=True,type=Path)
    command=sub.add_parser('inspect',help='List saved records or read at most 64 exact selected declarations')
    command.add_argument('--inventory',required=True,type=Path)
    command.add_argument('--record',action='append',default=[])
    command=sub.add_parser('citations',help='Save separately typed declared citations; binding reloads native headers')
    for name in ('inventory','output-dir'):command.add_argument('--'+name,required=True,type=Path)
    command.add_argument('--input',required=True)
    command=sub.add_parser('links',help='Apply recorded catalog decisions and copy authored originals into navigation inputs')
    for name in ('collection','roots','clew','inventory','catalog','routes','output-dir'):
        command.add_argument('--'+name,required=True,type=Path)
    command.add_argument('--previous-catalog',type=Path)
    args=parser.parse_args(argv)
    try:
        if args.action=='collect':result=collect(args.collection,args.roots,args.clew,args.output_dir,args.previous_inventory)
        elif args.action=='check':result=check(args.collection,args.roots,args.clew,args.inventory,args.output_dir)
        elif args.action=='inspect':result=inspect(args.inventory,args.record)
        elif args.action=='citations':result=citations(args.inventory,args.input,args.output_dir)
        else:
            module=runpy.run_path(str(Path(__file__).with_name('semantic_catalog.py')),
                init_globals={'S':globals(),'Stop':Stop,'digest':digest})
            result=module['links'](args.collection,args.roots,args.clew,args.inventory,args.catalog,args.routes,
                                   args.output_dir,args.previous_catalog)
        print(encode(result),end='');return 2 if args.action in ('check','links') and not result['complete'] else 0
    except (Stop,OSError,ValueError,KeyError,TypeError) as error:
        print(encode({'status':error.status if isinstance(error,Stop) else 'INVALID_CATALOG_INPUT',
            **({'details':error.details} if isinstance(error,Stop) else {})}),end='');return 2


if __name__=='__main__':raise SystemExit(main())
