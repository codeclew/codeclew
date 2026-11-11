"""Explicit saved glossary reads; no run discovery, authoring or SOURCE promotion."""
from pathlib import Path
from collections import defaultdict
import tempfile

INVENTORY='codeclew-semantic-glossary-inventory/1.0'
MANIFEST='codeclew-author-glossary-citations/1.0'


def need(condition,status,**details):
    if not condition:raise Stop(status,exit_code=2,**details)


def read_saved(reader,root,service,work,review,snapshot,ids=()):
    args=['--work',work,'--review-run',review,'--snapshot',snapshot,'--include-glossary']
    for identity in ids:args+=['--glossary-id',identity]
    try:value=reader.invoke(root,['work','answer-context'],args)
    except S['Stop'] as error:raise Stop(error.status,exit_code=2,**error.details) from error
    glossary=value.get('glossary',{})
    need(isinstance(glossary,dict),'SAVED_GLOSSARY_NOT_SUPPORTED')
    need(value.get('schema')=='codeclew-saved-answer-context/1.1'
         and value.get('work')==work and value.get('reviewRun')==review
         and value.get('subject')=='service:'+service and value.get('comparedSnapshot')==snapshot
         and value.get('meaningReview')=='MODEL_APPROVED_AGAINST_SAVED_PACKET'
         and glossary.get('schema')=='codeclew-saved-glossary/1.0'
         and glossary.get('work')==work and glossary.get('reviewRun')==review
         and glossary.get('subject')==value['subject'] and glossary.get('comparedSnapshot')==snapshot
         and glossary.get('savedSnapshot')==value.get('savedSnapshot')
         and glossary.get('packetDigest')==value.get('packetDigest')
         and value.get('captures')==0 and value.get('agentInvocations')==0 and value.get('writes')==0,
         'SAVED_GLOSSARY_SCOPE_OR_AUTHORITY_MISMATCH')
    need(isinstance(glossary.get('entries'),list) and isinstance(glossary.get('entryBindings'),list)
         and len(glossary['entries'])==len(glossary['entryBindings'])==glossary.get('selectedEntries')
         and isinstance(glossary.get('inventory'),list)
         and len(glossary['inventory'])==glossary.get('availableEntries')
         and len(glossary['entries'])+glossary.get('omittedEntries',-1)==len(glossary['inventory']),
         'INCOMPLETE_SAVED_GLOSSARY_SELECTION')
    inventory={entry['id']:entry for entry in glossary['inventory']}
    need(len(inventory)==len(glossary['inventory']),'AMBIGUOUS_SAVED_GLOSSARY_ID')
    seen=set()
    for entry,binding in zip(glossary['entries'],glossary['entryBindings']):
        identity=entry['id'];need(identity in inventory and identity not in seen,'UNKNOWN_OR_AMBIGUOUS_GLOSSARY_ENTRY')
        seen.add(identity);header=inventory[identity]
        need(binding=={k:header[k] for k in ('id','pointer','entryDigest')}
             and binding['entryDigest']==digest(entry),'SAVED_GLOSSARY_ENTRY_BINDING_MISMATCH')
    need(not ids or seen==set(ids),'INCOMPLETE_SAVED_GLOSSARY_SELECTION')
    return value


def load_inventory(directory):
    directory=Path(directory);value=S['read_json'](directory/'inventory.json')
    need(isinstance(value,dict) and value.get('schema')==INVENTORY
         and value.get('digest')==digest({k:v for k,v in value.items() if k!='digest'})
         and isinstance(value.get('records'),list),'INVALID_SAVED_GLOSSARY_INVENTORY')
    for record in value['records']:
        need(S['re'].fullmatch(r'[0-9a-f]{64}\.json',record['object']) is not None
             and not (directory/'objects').is_symlink(),'INVALID_SAVED_GLOSSARY_OBJECT_PATH')
        response=S['read_json'](directory/'objects'/record['object'])
        need(record['object']==digest(response)[7:]+'.json','SAVED_GLOSSARY_OBJECT_CHANGED')
        entries=response['glossary']['entries'];entry=next((e for e in entries if e['id']==record['localId']),None)
        need(entry is not None and digest(entry)==record['entryDigest'],'SAVED_GLOSSARY_OBJECT_CHANGED')
    return value


def collect(collection_path,roots_path,clew,selection_path,output):
    collection=S['collection'](collection_path);roots=S['roots'](roots_path,collection)
    selections=S['read_json'](selection_path)
    need(isinstance(selections,list) and 0<len(selections)<=64,'INVALID_EXPLICIT_GLOSSARY_SELECTION')
    services={(i['id'],s['id']):(i['docset'],s['snapshot']) for i in collection['inputs'] for s in i['services']}
    output=Path(output);need(not output.exists() and '.codeclew' not in output.parts,'INVALID_GLOSSARY_OUTPUT')
    output.mkdir();(output/'objects').mkdir();reader=S['Reader'](clew,output)
    records,issues,seen=[],[],set()
    for selection in selections:
        need(isinstance(selection,dict) and not set(selection)-{'input','service','work','reviewRun','ids'}
             and all(isinstance(selection.get(k),str) for k in ('input','service','work','reviewRun'))
             and (selection['input'],selection['service']) in services
             and S['re'].fullmatch(r'[0-9a-f]{64}',selection['work']) is not None
             and S['re'].fullmatch(r'[0-9a-f]{32}',selection['reviewRun']) is not None,
             'INVALID_EXPLICIT_GLOSSARY_SELECTION')
        alias,service,work,review=(selection[k] for k in ('input','service','work','reviewRun'))
        ids=selection.get('ids',[])
        need(isinstance(ids,list) and len(ids)<=32 and all(isinstance(i,str) and 0<len(i)<=256 for i in ids)
             and len(set(ids))==len(ids),'INVALID_EXPLICIT_GLOSSARY_IDS')
        key=(alias,service,work,review);need(key not in seen,'DUPLICATE_EXPLICIT_GLOSSARY_INPUT');seen.add(key)
        docset,snapshot=services[(alias,service)]
        try:
            need(roots[alias].is_dir() and not roots[alias].is_symlink(),'SELECTED_ROOT_UNAVAILABLE')
            value=read_saved(reader,roots[alias],service,work,review,snapshot,ids);glossary=value['glossary']
            need(glossary['entries'],'NO_SAVED_GLOSSARY',nextAction='Use retained entity candidates; author any missing shared definition within the requested scope. No new model call was started.')
            name=digest(value)[7:]+'.json';path=output/'objects'/name
            if not path.exists():S['write_json'](path,value)
            for entry,binding in zip(glossary['entries'],glossary['entryBindings']):
                records.append({'input':alias,'docset':docset,'service':service,'work':work,'reviewRun':review,
                    'id':'SAVED_GLOSSARY:'+work+':'+review+':'+entry['id'],'kind':'SAVED_GLOSSARY',
                    'localId':entry['id'],'title':entry['label'],'object':name,'entryDigest':binding['entryDigest'],
                    'answerPointer':binding['pointer'],'savedSnapshot':glossary['savedSnapshot'],
                    'comparedSnapshot':snapshot,'answerDigest':glossary['answerDigest'],
                    'packetDigest':glossary['packetDigest'],'reviewDigest':glossary['reviewDigest'],
                    'applicability':value['capturedContext']['status'],
                    'authority':glossary['authority'],'definitionStatus':'CANDIDATE_NOT_SHARED_DEFINITION'})
        except (Stop,S['Stop'],OSError,ValueError,KeyError,TypeError) as error:
            issues.append({'input':alias,'service':service,'work':work,'reviewRun':review,
                'status':error.status if hasattr(error,'status') else 'INVALID_SAVED_GLOSSARY_RESPONSE'})
    value={'schema':INVENTORY,'collection':collection['id'],'collectionDigest':digest(collection),
           'records':records,'complete':not issues,'issues':issues,'counters':reader.counters}
    value['digest']=digest(value);S['write_json'](output/'inventory.json',value)
    result={'status':'SAVED_GLOSSARY_CANDIDATES_COLLECTED','complete':not issues,'entries':len(records),
        'issues':issues,'counters':reader.counters,'sharedDefinitionsEstablished':0}
    S['write_json'](output/'result.json',result);return result


def citations(directory,alias,output):
    inventory=load_inventory(directory);records=[r for r in inventory['records'] if r['input']==alias]
    need(inventory['complete'] and records,'COMPLETE_SAVED_GLOSSARY_INPUT_REQUIRED')
    output=Path(output);need(not output.exists() and '.codeclew' not in output.parts
         and not output.resolve().is_relative_to(Path(directory).resolve()),'INVALID_GLOSSARY_CITATION_OUTPUT')
    output.mkdir();(output/'objects').mkdir()
    manifest={'schema':MANIFEST,'input':alias,'records':[{**r,'key':'r'+str(i+1)} for i,r in enumerate(records)]}
    for record in records:
        path=output/'objects'/record['object']
        if not path.exists():S['write_json'](path,S['read_json'](Path(directory)/'objects'/record['object']))
    manifest['id']=digest(manifest);S['write_json'](output/'citations.json',manifest);tag='g'+manifest['id'][7:19]
    return {'status':'SAVED_GLOSSARY_CITATIONS_SAVED','manifestId':manifest['id'],'input':alias,
        'records':[{'id':S['record_selector'](r),'cite':tag+':'+r['key'],'title':r['title']} for r in manifest['records']]}


def inspect(directory,selectors=()):
    inventory=load_inventory(directory);records={S['record_selector'](r):r for r in inventory['records']}
    need(len(selectors)<=64 and len(set(selectors))==len(selectors) and all(s in records for s in selectors),
         'UNKNOWN_OR_AMBIGUOUS_GLOSSARY_SELECTION')
    if not selectors:
        return {'status':'SAVED_GLOSSARY_CANDIDATES','complete':inventory['complete'],
                'records':[{'id':key,'title':r['title'],'capturedContext':r['applicability']} for key,r in records.items()],
                'issues':inventory['issues']}
    values=[]
    for selector in selectors:
        record=records[selector];original=S['read_json'](Path(directory)/'objects'/record['object'])
        entry=next(e for e in original['glossary']['entries'] if e['id']==record['localId'])
        values.append({'id':selector,'entry':entry,'capturedContext':record['applicability']})
    return {'status':'HISTORICAL_GLOSSARY_NOT_SOURCE_OR_NEW_MEANING_REVIEW','selected':values}


def load_manifest(path,manifest,alias,engine,cache,reader=None):
    need(alias is not None and engine is not None and manifest.get('input')==alias
         and set(manifest)=={'schema','input','records','id'} and manifest['schema']==MANIFEST
         and manifest['id']==digest({k:v for k,v in manifest.items() if k!='id'})
         and isinstance(manifest['records'],list) and 0<len(manifest['records'])<=4096,
         'INVALID_SELECTED_GLOSSARY_MANIFEST')
    tag='g'+manifest['id'][7:19];entries={};clew,root=engine
    groups=defaultdict(list)
    required={'input','docset','service','work','reviewRun','id','kind','localId','title','object','entryDigest',
              'answerPointer','savedSnapshot','comparedSnapshot','answerDigest','packetDigest','reviewDigest',
              'applicability','authority','definitionStatus','key'}
    def cache_key(record):
        return ('SAVED_GLOSSARY',str(clew),str(root),record['work'],record['reviewRun'],record['comparedSnapshot'],record['localId'])
    for record in manifest['records']:
        need(isinstance(record,dict) and set(record)==required and record['input']==alias
             and record['kind']=='SAVED_GLOSSARY' and isinstance(record['key'],str)
             and S['ID'].fullmatch(record['key']) is not None,'INVALID_SAVED_GLOSSARY_CITATION_RECORD')
        if cache_key(record) not in cache:
            groups[(record['service'],record['work'],record['reviewRun'],record['comparedSnapshot'])].append(record)
    for (service,work,review,snapshot),records in groups.items():
        ids=list(dict.fromkeys(r['localId'] for r in records))
        for start in range(0,len(ids),32):
            if reader is None:
                with tempfile.TemporaryDirectory() as output:
                    current=read_saved(S['Reader'](clew,output),root,service,work,review,snapshot,ids[start:start+32])
            else:current=read_saved(reader,root,service,work,review,snapshot,ids[start:start+32])
            for record in records:
                if record['localId'] in ids[start:start+32]:cache[cache_key(record)]=current
    for record in manifest['records']:
        need(S['re'].fullmatch(r'[0-9a-f]{64}\.json',record['object']) is not None
             and not (path.parent/'objects').is_symlink(),'INVALID_SAVED_GLOSSARY_OBJECT_PATH')
        original=S['read_json'](path.parent/'objects'/record['object'])
        need(record['object']==digest(original)[7:]+'.json','SAVED_GLOSSARY_OBJECT_CHANGED')
        current=cache[cache_key(record)];glossary=current['glossary']
        need(original['subject']==current['subject'] and original['savedContextDigest']==current['savedContextDigest']
             and original['glossary']['provenance']==glossary['provenance']
             and all(record[k]==glossary[k] for k in ('savedSnapshot','comparedSnapshot','answerDigest','packetDigest','reviewDigest','authority')),
             'SAVED_GLOSSARY_NATIVE_BINDING_MISMATCH')
        entry=next(e for e in glossary['entries'] if e['id']==record['localId'])
        binding=next(e for e in glossary['entryBindings'] if e['id']==record['localId'])
        need(record['id']=='SAVED_GLOSSARY:'+record['work']+':'+record['reviewRun']+':'+record['localId']
             and binding['id']==record['localId'] and binding['pointer']==record['answerPointer']
             and binding['entryDigest']==record['entryDigest'] and entry['label']==record['title']
             and any(value==entry for value in original['glossary']['entries']), 'SAVED_GLOSSARY_NATIVE_BINDING_MISMATCH')
        handle=tag+':'+record['key'];need(handle not in entries,'DUPLICATE_GLOSSARY_CITATION_KEY')
        entries[handle]={'manifestId':manifest['id'],'basisKind':'SAVED_GLOSSARY','kind':'SAVED_GLOSSARY',
            'evidenceInput':alias,'service':record['service'],'work':record['work'],'reviewRun':record['reviewRun'],
            'snapshot':record['savedSnapshot'],'comparedSnapshot':record['comparedSnapshot'],
            'contextDigest':current['savedContextDigest'],'scope':'HISTORICAL_APPROVED_PACKET',
            'localId':record['localId'],'id':record['id'],'title':record['title'],'authority':record['authority'],
            'basis':{'recordDigest':record['entryDigest'],'pointer':record['answerPointer'],
                     'answerDigest':record['answerDigest'],'packetDigest':record['packetDigest'],
                     'reviewDigest':record['reviewDigest']},'originalEntry':entry,
            'provenance':glossary['provenance'],'capturedContext':current['capturedContext']}
    return manifest,{'kind':'DECLARED'}, {},entries


def checked_records(collection_path,roots_path,clew,directory,output):
    collection=S['collection'](collection_path);roots=S['roots'](roots_path,collection)
    value=load_inventory(directory)
    need(value['collection']==collection['id']
         and value['collectionDigest']==digest(collection),'GLOSSARY_COLLECTION_SCOPE_CHANGED')
    services={(i['id'],s['id']):(i['docset'],s['snapshot']) for i in collection['inputs'] for s in i['services']}
    output=Path(output);output.mkdir();reader=S['Reader'](clew,output)
    found=[];cache={};issues=list(value['issues'])
    for alias in roots:
        records=[r for r in value['records'] if r['input']==alias]
        if not records:continue
        for record in records:
            need(services.get((alias,record['service']))==(record['docset'],record['comparedSnapshot']),
                 'GLOSSARY_COLLECTION_SCOPE_CHANGED')
        manifest={'schema':MANIFEST,'input':alias,'records':[{**r,'key':'r'+str(i+1)} for i,r in enumerate(records)]}
        manifest['id']=digest(manifest)
        _,_,_,entries=load_manifest(Path(directory)/'inventory.json',manifest,alias,(clew,roots[alias]),cache,reader)
        by_key={r['key']:r for r in manifest['records']}
        for handle,entry in entries.items():
            record=by_key[handle.rsplit(':',1)[1]];raw=entry['originalEntry']
            found.append({**record,'projectionDigest':record['entryDigest'],'recordDigest':record['entryDigest'],
                          'snapshot':record['savedSnapshot'],'basis':entry['basis'],'value':raw,
                          'applicability':entry['capturedContext']['status']})
    S['write_json'](output/'result.json',{'status':'SAVED_GLOSSARY_BINDINGS_CHECKED','entries':len(found),
        'inputIssues':issues,'counters':reader.counters})
    return found,reader.counters,issues
