"""Explicit recorded decisions and navigation over selected frozen candidates.

Original fragments are copied verbatim to the existing compose input. This module
neither writes prose nor publishes HTML, infers identity, or promotes SOURCE proof.
"""
from collections import defaultdict
from pathlib import Path
import runpy

M = runpy.run_path(str(Path(__file__).with_name('page_citations.py')))
INPUT = 'codeclew-semantic-catalog-decisions/1.0'
SNAPSHOT = 'codeclew-semantic-catalog-snapshot/1.0'
KINDS = {'external','concept','local','representation'}


def need(condition,status,**details):
    if not condition: raise Stop(status,exit_code=2,**details)


def text(value,limit=1024):
    return isinstance(value,str) and 0<len(value.strip())<=limit


def normalize(record):
    need(isinstance(record,dict) and set(record)-{'scope','represents','replaces'}==
         {'id','kind','title','target','aliases','status','decision','original'}, 'INVALID_CANONICAL_RECORD')
    need(text(record['id'],64) and S['ID'].fullmatch(record['id']) is not None
         and record['kind'] in KINDS and text(record['title'],160)
         and record['status'] in {'CONFIRMED','PROPOSED','REJECTED'}, 'INVALID_CANONICAL_IDENTITY')
    decision=record['decision']
    need(isinstance(decision,dict) and set(decision)=={'origin','rationale'}
         and decision['origin'] in {'human','agent','model','import'} and text(decision['rationale']),
         'RECORDED_DECISION_ORIGIN_AND_RATIONALE_REQUIRED')
    need(isinstance(record['aliases'],list) and 0<len(record['aliases'])<=128
         and isinstance(record['original'],dict), 'INVALID_CANONICAL_ALIASES_OR_ORIGINAL')
    need((record['kind'] in {'local','representation'})==('scope' in record)
         and (record['kind']=='representation')==('represents' in record), 'INVALID_CANONICAL_SCOPE')
    if 'scope' in record:
        need(isinstance(record['scope'],dict) and set(record['scope'])=={'docset','service'}
             and all(text(record['scope'][k],64) for k in ('docset','service')), 'INVALID_CANONICAL_SCOPE')
    return {k:v for k,v in record.items() if k!='replaces'}


def decisions(path,previous):
    value=S['read_json'](path)
    need(isinstance(value,dict) and set(value)=={'schema','collection','revision','baseDigest','records','locations','remove'}
         and value['schema']==INPUT and text(value['collection'],64)
         and text(value['revision'],64) and S['ID'].fullmatch(value['revision']) is not None
         and isinstance(value['records'],list) and len(value['records'])<=128
         and isinstance(value['locations'],list) and len(value['locations'])<=4096
         and isinstance(value['remove'],list), 'INVALID_CATALOG_DECISIONS')
    old=S['read_json'](Path(previous)/'catalog.json') if previous else None
    need(value['baseDigest'] is None or old is not None, 'EXACT_PREVIOUS_CATALOG_REQUIRED')
    if old is not None:
        need(isinstance(old,dict) and old.get('schema')==SNAPSHOT and old.get('collection')==value['collection']
             and old.get('digest')==digest({k:v for k,v in old.items() if k!='digest'})
             and (value['baseDigest'] is None or old['digest']==value['baseDigest']), 'CATALOG_BASE_DIGEST_MISMATCH')
    active={r['definition']['id']:r for r in (old or {}).get('records',[])}
    need(len(active)==len((old or {}).get('records',[])), 'INVALID_PREVIOUS_CATALOG')
    for row in active.values():
        need(row['definitionDigest']==digest(row['definition']) and row['definition']['status']=='CONFIRMED',
             'INVALID_PREVIOUS_CONFIRMED_DECISION')
    seen,proposals,removed=set(),[],[]
    for raw in value['records']:
        record=normalize(raw);identity=record['id']
        need(identity not in seen,'DUPLICATE_CANONICAL_DECISION');seen.add(identity)
        if record['status']!='CONFIRMED':
            proposals.append(record);continue
        prior=active.get(identity)
        if prior and prior['definitionDigest']!=digest(record):
            need(raw.get('replaces')==prior['definitionDigest'], 'EXPLICIT_CANONICAL_REPLACEMENT_REQUIRED',record=identity)
        elif 'replaces' in raw and prior is None:
            need(False,'CANONICAL_REPLACEMENT_BASE_MISMATCH')
        active[identity]={'definition':record,'definitionDigest':digest(record)}
    for removal in value['remove']:
        need(isinstance(removal,dict) and set(removal)=={'id','previousDigest'}
             and removal['id'] in active and removal['id'] not in seen
             and active[removal['id']]['definitionDigest']==removal['previousDigest'], 'EXPLICIT_CANONICAL_REMOVAL_REQUIRED')
        removed.append(removal['id']);del active[removal['id']]
    return value,old,active,proposals,removed


def local_values(directory,inventory):
    result={}
    for record in inventory['records']:
        value=S['read_json'](Path(directory)/'objects'/record['object'])
        result[S['record_selector'](record)]={**record,'value':S['project'](record['kind'],value,record['service'])}
    return result


def alias_candidates(record,alias,index):
    need(isinstance(alias,dict) and set(alias)-{'service'}=={'input','docset','localId'}
         and all(text(alias[k],512) for k in ('input','docset','localId')), 'INVALID_LOCAL_CANONICAL_ALIAS')
    kind=record['kind'];scope=record.get('scope')
    if scope:
        need(isinstance(scope,dict) and set(scope)=={'docset','service'}
             and alias['docset']==scope['docset'] and alias.get('service')==scope['service'], 'SCOPED_ALIAS_OWNER_MISMATCH')
    if kind=='external':need('service' not in alias,'EXTERNAL_IDENTITY_IS_ROOT_LOCAL')
    key=(alias['input'],alias['docset'],'REPRESENTATION' if kind=='representation' else
         'EXTERNAL_SYSTEM' if kind=='external' else 'TERM_CANDIDATE',alias['localId'])
    matches=list(dict.fromkeys(selector for selector,row in index.get(key,[])
                              if 'service' not in alias or row['service']==alias['service']))
    need(matches,'LOCAL_ALIAS_NOT_COLLECTED',input=alias['input'],localId=alias['localId'])
    return matches


def links(collection_path,roots_path,clew,inventory_dir,decision_path,routes_path,output,previous=None):
    selected,old,active,proposals,removed=decisions(decision_path,previous)
    inventory=S['load_inventory'](inventory_dir)
    need(selected['collection']==inventory['collection'],'CANONICAL_COLLECTION_MISMATCH')
    registry=S['read_json'](routes_path)
    need(isinstance(registry,dict) and registry.get('schema') in {'codeclew-cross-docset-routes/1.1','codeclew-cross-docset-routes/1.2'}
         and isinstance(registry.get('docsets'),list), 'SUPPORTED_DOCSET_ROUTES_REQUIRED')
    docsets={d['id']:d for d in registry['docsets']}
    need(len(docsets)==len(registry['docsets']) and 0<len(docsets)<=64,'INVALID_CATALOG_DOCSET_NAMESPACES')
    service_names={(d['id'],s['id']) for d in docsets.values() for s in d['services']}
    scope=S['collection'](collection_path)
    need(digest(scope)==inventory['collectionDigest'] and all(
        (i['docset'],s['id']) in service_names for i in scope['inputs'] for s in i['services']),
        'COLLECTED_SERVICES_NOT_IN_DOCSET_ROUTES')
    output=Path(output)
    need(not output.exists() and '.codeclew' not in output.parts
         and all(not output.resolve().is_relative_to(Path(p).resolve()) for p in [inventory_dir,*([previous] if previous else [])]),
         'INVALID_CANONICAL_OUTPUT')
    output.mkdir()
    checked=S['check'](collection_path,roots_path,clew,inventory_dir,output/'input-check')
    need(checked['complete'],'CANONICAL_INPUTS_INCOMPLETE')
    locals=local_values(inventory_dir,inventory)
    index,external_uses=defaultdict(list),defaultdict(list)
    for selector,row in locals.items():
        index[(row['input'],row['docset'],row['kind'],row['localId'])].append((selector,row))
        if row['kind']=='EXTERNAL_USE':external_uses[(row['input'],row['value'].get('externalService'))].append((selector,row))
        if row['kind']=='TERM_CANDIDATE':
            for relation in row['value'].get('relations',[]):
                for name in relation.get('representations',[]):
                    index[(row['input'],row['docset'],'REPRESENTATION',name)].append((selector,row))
    locations={}
    for location in selected['locations']:
        need(isinstance(location,dict) and set(location)-{'process'}=={'record','target'}
             and location['record'] in locals and location['record'] not in locations,'INVALID_LOCAL_DETAIL_LOCATION')
        row=locals[location['record']]
        need(isinstance(location['target'],dict) and location['target'].get('docset')==row['docset'], 'LOCAL_DETAIL_DOCSET_MISMATCH')
        locations[location['record']]=location
    entries,issues,aliases,targets,originals=[],[],set(),set(),defaultdict(list)
    previous_records={r['definition']['id']:r for r in (old or {}).get('records',[])}
    original_reused=0;bound_records=[];covered=set();local_uses={}
    for identity,row in sorted(active.items()):
        record=row['definition'];bound={};uses=[];seen_uses=set();record_uses={}
        try:
            target=record['target'];need(isinstance(target,dict) and text(target.get('page'),255)
                and target['page'].endswith('.html') and target.get('docset') in docsets,'INVALID_CANONICAL_TARGET')
            if record.get('scope'):
                need((record['scope'].get('docset'),record['scope'].get('service')) in service_names
                     and record['scope']['docset']==target['docset'],'SCOPED_CANONICAL_OWNER_MISMATCH')
            key=(target['docset'],target['page']);need(key not in targets,'DUPLICATE_CANONICAL_PAGE');targets.add(key)
            if record['kind']=='representation':
                need(record['represents'] in active and active[record['represents']]['definition']['kind']=='concept',
                     'REPRESENTATION_CONCEPT_NOT_CONFIRMED')
            selected_aliases=[]
            for alias in record['aliases']:
                matches=alias_candidates(record,alias,index)
                alias_key=(record['kind'] if record['kind'] in {'external','representation'} else 'term',
                           alias['input'],alias['localId'],alias.get('service'))
                need(alias_key not in aliases and alias_key not in selected_aliases,'CANONICAL_ALIAS_ALREADY_CONFIRMED',record=identity)
                selected_aliases.append(alias_key)
                for selector in matches:
                    value=locals[selector]
                    bound[selector]={k:value[k] for k in ('recordDigest','projectionDigest','snapshot','work','basis')}
                    related=[(selector,value)]
                    if record['kind']=='external':
                        related=external_uses.get((alias['input'],alias['localId']),[])
                    for local_key,local in related:
                        need(local_key in locations,'LOCAL_DETAIL_TARGET_MISSING',record=local_key)
                        location=locations[local_key];use_key=(local['docset'],local['service'],digest(location['target']))
                        if use_key in seen_uses:continue
                        seen_uses.add(use_key)
                        uses.append({'docset':local['docset'],'service':local['service'],'title':local['title'],
                                     'target':location['target'],**({'process':location['process']} if 'process' in location else {})})
                        record_uses[local_key]={k:local[k] for k in ('input','docset','service','id','kind','work','snapshot','basis','value')}
            page=record['original']
            need(page.get('page')==target['page'].removesuffix('.html')+'.md', 'CANONICAL_ORIGINAL_TARGET_MISMATCH')
            try:M['compose_pages']({'pages':[page]})
            except M['Stop'] as error:raise Stop(error.status,exit_code=2,**error.details) from error
            entry={k:record[k] for k in ('id','kind','title','target','aliases','scope','represents') if k in record}
            entry['uses']=uses;entries.append(entry);aliases.update(selected_aliases);covered.update(bound)
            local_uses.update(record_uses);covered.update(record_uses)
            originals[target['docset']].append(page)
            prior=previous_records.get(identity)
            if prior and digest(prior['definition']['original'])==digest(page):original_reused+=1
            if prior and {k:v['projectionDigest'] for k,v in prior.get('bindings',{}).items()}!={k:v['projectionDigest'] for k,v in bound.items()}:
                issues.append({'record':identity,'status':'LOCAL_DECLARED_VALUES_CHANGED','severity':'REVIEW',
                    'meaning':'Confirmed definition preserved with its original exact citations; inspect changed local values.'})
            bound_records.append({**row,'bindings':bound})
        except Stop as error:
            issues.append({'record':identity,'status':error.status,'details':error.details,'severity':'BLOCKER'})
            bound_records.append(row)
    registry['schema']='codeclew-cross-docset-routes/1.2'
    registry['semanticCatalog']={'id':selected['collection'],'revision':selected['revision'],'records':entries}
    for docset,pages in originals.items():
        need(isinstance(docset,str) and S['ID'].fullmatch(docset) is not None,'INVALID_CANONICAL_DOCSET')
        S['write_json'](output/('originals.'+docset+'.json'),{'pages':pages})
    S['write_json'](output/'routes.json',registry)
    S['write_json'](output/'local-uses.json',{'records':list(local_uses.values())})
    snapshot={'schema':SNAPSHOT,'collection':selected['collection'],'revision':selected['revision'],
        'inventoryDigest':inventory['digest'],'baseDigest':old['digest'] if old else None,'records':bound_records,
        'proposals':proposals,'removed':removed,'issues':issues,'complete':not any(i['severity']=='BLOCKER' for i in issues)}
    snapshot['digest']=digest(snapshot);S['write_json'](output/'catalog.json',snapshot)
    S['write_json'](output/'decisions.input.json',selected)
    result={'status':'SEMANTIC_CATALOG_LINKED','complete':snapshot['complete'],'catalogDigest':snapshot['digest'],
        'records':len(entries),'unappliedProposals':len(proposals),'issues':issues,'removed':removed,
        'unmappedCandidates':sorted(set(locals)-covered),'counters':{**checked['counters'],
            'originalPagesReused':original_reused,'originalPagesChanged':len(entries)-original_reused,
            'proseAuthorCalls':0,'htmlPublications':0},
        'authority':'EXPLICIT_RECORDED_DECISIONS_AND_DECLARED_LOCAL_USES_NOT_SOURCE_OR_RUNTIME_PROOF'}
    S['write_json'](output/'result.json',result)
    return result
