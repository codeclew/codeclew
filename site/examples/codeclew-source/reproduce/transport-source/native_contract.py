"""Bounded generic-native schema reconstruction, not an OperationAnswer adapter."""
from __future__ import annotations
import copy, hashlib, itertools, json, re
from pathlib import Path
from finite_cli import JobError, _json_loads, _canonical_json_bytes, digest
ROOT=Path(__file__).resolve().parent
MAX_SCHEMA=65536
PROPOSAL='codeclew-documentation-proposal/1.0'
REVIEW='codeclew-documentation-review/1.0'
SECTIONS={'section-overview','section-responsibilities','section-entities','section-ingress','section-egress'}


def template(name):
    raw=(ROOT/f'known-{name}-schema.json').read_bytes()
    expected=json.loads((ROOT/'source-provenance.json').read_text())['templatesRawSha256'][name]
    if hashlib.sha256(raw).hexdigest()!=expected: raise JobError('known native schema template changed')
    return _json_loads(raw)


def page_items(payload):
    return [item for page in payload['evidence']['pages'] for item in page['items']]


def delivered(payload, proof, citable):
    result={item['reference'] for item in page_items(payload) if isinstance(item.get('reference'),str)
            and (not citable or item.get('kind') in {'SOURCE','DEPENDENCY','ENTRYPOINT'})}
    # Exact SOURCE proofs are required for this candidate; source_parts validation
    # in controller binds full text, native snapshot and immutable record digests.
    for part in payload['evidence']['sourceParts']:
        result.add(part['reference'])
    return sorted(result)


def bind_zero(schema,action,remaining):
    if remaining==0:
        schema['oneOf']=[{'$ref':f'#/$defs/{action}Action'}]
        schema['$defs'].pop('expandAction',None); schema['$defs'].pop('selection',None)
        schema['description']=f'Return the complete {action} response. No registered expansion action remains.'
    return schema


def author_schema(payload,remaining,proof):
    incoming=payload['outputSchema']; p=template('proposal')
    p['properties'].pop('retainedEdits'); p['$defs'].pop('retainedEdit')
    p['$defs']['operation']['properties']['summary']['properties']={'text':{
        'maxLength':2048,'pattern':'^[^`<]*$',
        'description':"Nonblank plain prose, at most 2048 UTF-8 bytes (not characters); no backticks or '<'. The host enforces the byte limit."}}
    values=delivered(payload,proof,True)
    evidence={'type':'string','enum':values} if values else False
    p['$defs']['claim']['properties']['evidence']['items']=copy.deepcopy(evidence)
    p['$defs']['visualClaim']['properties']['evidence']['items']=copy.deepcopy(evidence)
    p['$defs']['assertion']['properties']['evidence']=copy.deepcopy(evidence)
    aliases=payload['sequenceGuidance']['participantAliases']['builtInAuthoredAliases']
    if aliases!=sorted({'caller',proof['service']}): raise JobError('unexpected participant aliases')
    description=('For message, return, and declared steps this is required and must be a nonempty known authored participant alias. Built-in aliases for this Work: '
        +json.dumps(aliases,separators=(',',':'))+'. IDs declared in this operation\'s participants are also valid. Use raw service IDs, not renderer-internal service-* IDs; note and group endpoints remain optional.')
    step=p['$defs']['step']; step['properties']['from']['description']=description; step['properties']['to']['description']=description
    step['properties']['interaction']['description']='Declared transitions require a nonempty delivered Work reference to a declared interaction.'
    endpoint={'type':'string','minLength':1}
    step['oneOf']=[{'properties':{'kind':{'enum':['message','return']},'from':endpoint,'to':endpoint},'required':['kind','meaning','from','to']},
        {'properties':{'kind':{'const':'declared'},'from':endpoint,'to':endpoint,'interaction':{'type':'string','minLength':1}},'required':['kind','meaning','from','to','interaction']},
        {'properties':{'kind':{'enum':['note','alt','loop','opt']}},'required':['kind','meaning']}]
    p['$defs']['operation']['properties']['entrypoint']={'type':'string','enum':sorted(proof['targets']),
        'description':'Use a delivered, recorded Work operation reference from this packet, or the exact scenario subject where the runtime supports that special target. Raw operation IDs are not target handles.'}
    # Gap handles are genuine host dynamic overlays, never evidence enums. The
    # bounded model subset exposes only publicly verified section handles.
    gap=incoming['$defs']['proposalSchema']['properties']['gaps']
    props=gap.get('properties',{})
    if not isinstance(props,dict) or len(props)>1024 or any(not isinstance(k,str) or v!={'type':'string','minLength':1} for k,v in props.items()):
        raise JobError('unknown gap schema overlay')
    if not set(proof['targets']).issubset(props): raise JobError('verified section gap handles absent')
    p['properties']['gaps']={'type':'object','maxProperties':1024,'additionalProperties':False,'properties':copy.deepcopy(props)}
    s=template('section-author'); s.pop('$id'); s['title']='Documentation author result'
    s['description']='Return a proposal action containing the inner proposal, or request registered evidence expansion. Never return a bare proposal.'
    p.pop('$id'); p.pop('$schema'); definitions=p.pop('$defs')
    s['$defs'].pop('sectionAction'); s['$defs'].update(definitions); s['$defs']['proposalSchema']=p
    s['$defs']['proposalAction']={'type':'object','additionalProperties':False,'properties':{'action':{'const':'proposal'},'proposal':{'$ref':'#/$defs/proposalSchema'}},'required':['action','proposal']}
    s['oneOf']=[{'$ref':'#/$defs/proposalAction'},{'$ref':'#/$defs/expandAction'}]
    return bind_zero(s,'proposal',remaining)


def reviewer_schema(payload,remaining,proof):
    s=template('section-author'); s.pop('$id'); s['title']='Bound meaning-review result'
    s['description']='Return a review of every supplied claim and operation, or request registered evidence expansion. Coverage arrays contain ID strings. Issue evidence contains delivered Work handles. Runtime validation also enforces UTF-8 byte bounds and nonempty text.'
    s['$defs'].pop('sectionAction'); base=template('review'); p=base['properties']
    claims=sorted(payload['claims']); operations=[o['id'] for o in payload['content']['operations']]
    for key in ['work','proposal','evidenceDigest']: p[key]={'const':payload[key]}
    for key,values in [('assessedClaims',claims),('assessedOperations',operations)]:
        p[key]['minItems']=p[key]['maxItems']=len(values)
        p[key]['items']={'type':'string','minLength':1,'enum':values} if values else False
    p['issues']['items']['properties']['claim']['enum']=claims+[None]
    values=delivered(payload,proof,False)
    p['issues']['items']['properties']['evidence']['items']={'type':'string','minLength':1,'enum':values} if values else False
    review={'type':'object','additionalProperties':False,'properties':p,'required':base['required']}
    s['$defs']['reviewAction']={'type':'object','additionalProperties':False,'properties':{'action':{'const':'review'},'review':review},'required':['action','review']}
    s['oneOf']=[{'$ref':'#/$defs/reviewAction'},{'$ref':'#/$defs/expandAction'}]
    return bind_zero(s,'review',remaining)


def obj(properties): return {'type':'object','additionalProperties':False,'properties':properties,'required':list(properties)}
def arr(items): return {'type':'array','items':items}
def string(values=None,nullable=False):
    s={'type':['string','null'] if nullable else 'string'}
    if values is not None:
        if not values: raise JobError('empty strict enum')
        s['enum']=values
    return s


def project(job,proof):
    payload=job['payload']; role=job['role']; source=payload['outputSchema']; remaining=job['expansionBudget']['remaining']
    expected=author_schema(payload,remaining,proof) if role=='author' else reviewer_schema(payload,remaining,proof)
    if source!=expected: raise JobError('schema differs from known native templates plus admitted host overlays')
    if len(_canonical_json_bytes(source))>MAX_SCHEMA: raise JobError('full native output schema exceeds candidate bound')
    if remaining: return None,None  # Only a local expand, no structured model output.
    if role=='author':
        refs=delivered(payload,proof,True)
        if not refs: raise JobError('no citable evidence for bounded summary author')
        summary=obj({'text':string(),'evidence':arr(string(refs)),'uncertainty':string(nullable=True)})
        operation=obj({'entrypoint':string(sorted(proof['targets'])),'title':string(),'summary':summary,'steps':arr({'type':'string'})})
        # Empty arrays and all byte/cardinality bounds are checked locally and
        # again natively. Optional native fields omitted by this valid subset.
        gap_branches=[]; targets=sorted(proof['targets'])
        for mask in range(1<<len(targets)):
            gap_branches.append(obj({k:string() for i,k in enumerate(targets) if mask&(1<<i)}))
        projected=obj({'action':string(['proposal']),'proposal':obj({'schema':string([PROPOSAL]),'operations':arr(operation),
            'gaps':{'anyOf':gap_branches},'uncertainties':arr(string())})})
    else:
        review=source['$defs']['reviewAction']['properties']['review']['properties']
        claims=sorted(payload['claims']); operations=[o['id'] for o in payload['content']['operations']]
        # Empty coverage arrays remain representable; no empty enum is emitted.
        coverage=lambda values: arr(string(values) if values else string())
        refs=delivered(payload,proof,False)
        issue=obj({'severity':string(['ERROR','LIMITATION']),'claim':string(claims+[None],nullable=True),'reason':string(),
            'evidence':arr(string(refs) if refs else string())})
        projected=obj({'action':string(['review']),'review':obj({'schema':string([REVIEW]),'work':string([payload['work']]),
            'proposal':string([payload['proposal']]),'evidenceDigest':string([payload['evidenceDigest']]),
            'verdict':string(['APPROVE','REJECT','NEEDS_EVIDENCE']),'assessedClaims':coverage(claims),
            'assessedOperations':coverage(operations),'issues':arr(issue),'limitations':arr(string())})})
    pretty=(json.dumps(projected,ensure_ascii=False,indent=2)+'\n').encode()
    if len(pretty)>MAX_SCHEMA: raise JobError('projected strict schema exceeds candidate bound')
    return projected,{'projectionVersion':'codeclew-private-generic-summary-strict/1.0','work':job['work'],'role':role,
        'originalSchemaDigest':digest(source),'projectedSchemaDigest':digest(projected),'projectedBytes':len(pretty),
        'nativeSchemaPreservedInPrompt':True,'scope':'VERIFIED_SERVICE_SECTION_SUMMARIES_ONLY',
        'boundsAuthority':'LOCAL_AND_NATIVE_VALIDATORS_NOT_MODEL_KEYWORD_SUPPORT'}


def validate(value,schema,root=None,depth=0):
    root=schema if root is None else root
    if depth>64: raise JobError('output nesting exceeds bound')
    if schema is False: raise JobError('value prohibited by native schema')
    if schema is True: return
    if '$ref' in schema:
        target=root
        for part in schema['$ref'].removeprefix('#/').split('/'): target=target[part]
        validate(value,target,root,depth+1)
    for union in ['oneOf','anyOf']:
        if union in schema:
            successes=0
            for branch in schema[union]:
                try: validate(value,branch,root,depth+1); successes+=1
                except JobError: pass
            if successes==0 or union=='oneOf' and successes!=1: raise JobError('union output does not match')
    if 'const' in schema and value!=schema['const']: raise JobError('constant mismatch')
    if 'enum' in schema and value not in schema['enum']: raise JobError('enum mismatch')
    types=schema.get('type',[]); types=[types] if isinstance(types,str) else types
    checks={'object':lambda:isinstance(value,dict),'array':lambda:isinstance(value,list),'string':lambda:isinstance(value,str),
            'null':lambda:value is None,'integer':lambda:type(value) is int,'boolean':lambda:type(value) is bool}
    if types and not any(checks[t]() for t in types): raise JobError('output type mismatch')
    if isinstance(value,dict):
        if not set(schema.get('required',[])).issubset(value): raise JobError('missing required output fields')
        props=schema.get('properties',{})
        if schema.get('additionalProperties') is False and not set(value).issubset(props): raise JobError('unknown output field')
        for k,v in value.items():
            if k in props: validate(v,props[k],root,depth+1)
        if 'maxProperties' in schema and len(value)>schema['maxProperties']: raise JobError('too many properties')
    if isinstance(value,list):
        if len(value)<schema.get('minItems',0) or len(value)>schema.get('maxItems',100000): raise JobError('array bounds')
        if schema.get('uniqueItems') and len({_canonical_json_bytes(v) for v in value})!=len(value): raise JobError('duplicate coverage')
        for v in value:
            if 'items' in schema: validate(v,schema['items'],root,depth+1)
    if isinstance(value,str):
        if len(value)<schema.get('minLength',0) or len(value)>schema.get('maxLength',1000000): raise JobError('string bounds')
        if 'pattern' in schema and not re.search(schema['pattern'],value): raise JobError('string pattern')


def validate_output(value,job):
    validate(value,job['_projected_output_schema']); validate(value,job['payload']['outputSchema'])
    if job['role']=='author':
        proposal=value['proposal']; operations=proposal['operations']; targets=job['_proof']['targets']
        if len(operations)>5: raise JobError('summary operation bound')
        seen=set()
        for operation in operations:
            ref=operation['entrypoint']
            if ref in seen or ref in proposal['gaps']: raise JobError('duplicate or conflicting summary target')
            seen.add(ref)
            if operation['steps']: raise JobError('sequence steps outside bounded summary contract')
            text=operation['summary']['text']
            if not text.strip() or len(text.encode())>2048 or '`' in text or '<' in text: raise JobError('native summary prose byte bound')
            if not operation['title'].strip() or len(operation['title'].encode())>512: raise JobError('title byte bound')
            uncertainty=operation['summary']['uncertainty']
            if uncertainty is not None and (not uncertainty.strip() or len(uncertainty.encode())>2048): raise JobError('summary uncertainty byte bound')
        if set(targets)!=seen|set(proposal['gaps']): raise JobError('verified section summaries/gaps not covered')
        for text in list(proposal['gaps'].values())+proposal['uncertainties']:
            if not text.strip() or len(text.encode())>2048: raise JobError('uncertainty/gap byte bound')
    else:
        r=value['review']
        if r['verdict']=='APPROVE' and any(i['severity']=='ERROR' for i in r['issues']): raise JobError('approval contradicts errors')
        if r['verdict']!='APPROVE' and not r['issues']: raise JobError('nonapproval lacks issue')
        for text in r['limitations']+[i['reason'] for i in r['issues']]:
            if not text.strip() or len(text.encode())>2048: raise JobError('review text byte bound')
