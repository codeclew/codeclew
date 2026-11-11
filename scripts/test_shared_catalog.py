"""Owned five-service collection protocol fixtures; native responses are mocked."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
S=runpy.run_path(str(ROOT/'skills/codeclew/scripts/shared_catalog.py'))


class CollectionTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup)
        self.root=Path(self.temp.name);self.clew=self.root/'trusted-clew';self.clew.write_text('test double')
        self.roots={name:self.root/name for name in ['east','west']}
        for path in self.roots.values():path.mkdir()
        self.selected={'schema':S['COLLECTION'],'id':'owned','revision':'r1','inputs':[
            {'id':'east','docset':'east','services':[{'id':s,'snapshot':'east-s1'} for s in ['orders','billing','warehouse']]},
            {'id':'west','docset':'west','services':[{'id':s,'snapshot':'west-s1'} for s in ['orders','notifier']]}]}
        self.config=self.root/'collection.json';self.write(self.config,self.selected)
        self.local=self.root/'roots.json';self.write(self.local,{'schema':'codeclew-evidence-inputs/1.0',
            'inputs':{name:{'root':str(path)} for name,path in self.roots.items()},'manifests':{}})
        self.contexts={};self.works={};self.fail=None;self.calls=[];self.serial=0
        for alias,services in [('east',['orders','billing','warehouse']),('west',['orders','notifier'])]:
            external={'id':'provider','title':'Payments' if alias=='east' else 'Unrelated provider',
                      'description':'Declared '+alias+' provider','configurationReferences':[{'service':'unselected','path':'hidden.conf'}]}
            uses=[{'id':s+'-use','title':s+' operation','from':{'service':s},'to':{'service':'provider'},
                   'transport':{'kind':'http','method':'POST','path':'/'+s}} for s in services]
            normalized={'externalService':external,'interactions':uses,'authority':'DECLARED_EXTERNAL_SERVICE'}
            dep={'id':'external-service:provider','kind':'EXTERNAL_SERVICE','service':'','symbol':external['title'],
                 'normalized':normalized,'digest':S['digest'](normalized),'sourceIds':[]}
            for service in services:
                self.contexts[(alias,service,'s1')]=self.context(alias,service,alias+'-s1',dep,uses.index(next(u for u in uses if u['from']['service']==service)))
        self.mock=patch('subprocess.run',side_effect=self.engine);self.runner=self.mock.start();self.addCleanup(self.mock.stop)

    def write(self,path,value):path.write_text(S['encode'](value))

    def context(self,alias,service,snapshot,dep,index):
        context=S['digest']({'input':alias,'snapshot':snapshot,'dependency':dep})
        rows=[]
        for kind,pointer in [('EXTERNAL_SYSTEM','/normalized/externalService'),('EXTERNAL_USE',f'/normalized/interactions/{index}')]:
            value=S['pointer'](dep,pointer)
            basis={'snapshot':snapshot,'contextDigest':context,'dependencyId':dep['id'],'dependencyDigest':dep['digest'],
                   'dependencyRecordDigest':S['digest'](dep),'pointer':pointer,'recordDigest':S['digest'](value)}
            rows.append({'kind':'SEMANTIC_INVENTORY','id':kind+':'+value['id'],'record':{
                'kind':kind,'localId':value['id'],'title':value['title'],'scope':'SELECTED_SERVICE',
                'authority':'DECLARED_EXTERNAL_SERVICE','basis':basis,'fullRecordReference':'d1'}})
        scope={'kind':'SEMANTIC_SCOPE','id':'scope','record':{'schema':'codeclew-semantic-inventory-scope/1.0',
            'service':service,'snapshot':snapshot,'contextDigest':context,'items':len(rows),
            'serviceEvidence':{'extractor':'JAVAC' if alias=='east' else 'SOURCE_SYNTAX','coverage':'PARTIAL'},
            'sourceAcquisition':'NONE','currentCatalogueCompared':False}}
        return {'input':alias,'service':service,'snapshot':snapshot,'contextDigest':context,'rows':[scope,*rows],'dependency':dep}

    def page(self,work,context,offset):
        selected=context['rows'][offset:offset+1]
        value={'schema':'codeclew-documentation-work-page/1.0','snapshot':context['snapshot'],
            'work':work,'subject':'service:'+context['service'],'contextDigest':context['contextDigest'],
            'items':selected,'omitted':[],'total':len(context['rows']),
            'membershipDigest':S['digest']([[r['kind'],r['id']] for r in context['rows']]),
            'nextCursor':str(offset+1) if offset+1<len(context['rows']) else None}
        return {**value,'receiptDigest':S['digest'](value)}

    def engine(self,args,**kwargs):
        self.calls.append(args);operation=args[2:4];alias=Path(args[args.index('--root')+1]).name
        self.assertEqual(Path(args[0]),self.clew);self.assertNotIn('check',args);self.assertNotIn('--refresh',args)
        request=json.loads(Path(args[args.index('--input')+1]).read_bytes())
        if operation==['work','prepare']:
            self.assertEqual(request['contextProfile'],'semantic-inventory-v1')
            service=args[args.index('--subject')+1].split(':',1)[1];snapshot=args[args.index('--snapshot')+1]
            version=snapshot.rsplit('-',1)[1];context=self.contexts[(alias,service,version)]
            work=S['digest']({'input':alias,'service':service,'snapshot':snapshot})[7:];self.works[work]=context
            value=self.page(work,context,0)
        elif operation==['work','read']:
            work=args[args.index('--work')+1];context=self.works[work]
            if context['input']!=alias:
                return subprocess.CompletedProcess(args,2,b'',b'Work unavailable in selected root')
            if request.get('query'):
                self.assertEqual(request['query']['expectedSnapshot'],context['snapshot'])
                self.assertEqual(request['query']['expectedContextDigest'],context['contextDigest'])
            value=self.page(work,context,int(request.get('cursor') or '0'))
        else:
            self.assertEqual(operation,['work','read-dependency-part'])
            work=args[args.index('--work')+1];context=self.works[work]
            raw=S['encode'](context['dependency']).rstrip('\n').encode();offset=int(request['cursor'] or '0')
            chunk=raw[offset:offset+160]
            while True:
                try:text=chunk.decode();break
                except UnicodeDecodeError:chunk=chunk[:-1]
            value={'work':work,'snapshot':context['snapshot'],'id':context['dependency']['id'],
                'startByte':offset,'endByte':offset+len(chunk),'totalRecordBytes':len(raw),'text':text,
                'recordDigest':'sha256:'+hashlib.sha256(raw).hexdigest(),
                'fragmentDigest':'sha256:'+hashlib.sha256(chunk).hexdigest(),
                'nextCursor':str(offset+len(chunk)) if offset+len(chunk)<len(raw) else None}
            if self.fail==context['service']:value['startByte']=999
            value['receiptDigest']=S['digest'](value)
        return subprocess.CompletedProcess(args,0,S['encode'](value).encode(),b'')

    def collect(self,previous=None):
        self.serial+=1;out=self.root/f'run-{self.serial}'
        result=S['collect'](self.config,self.local,self.clew,out,previous)
        return out,result,json.loads((out/'inventory.json').read_bytes())

    def test_five_services_two_roots_preserve_local_identity_and_reuse_unchanged_records(self):
        first,result,inventory=self.collect();self.assertTrue(result['complete']);self.assertEqual(result['services'],5)
        self.assertEqual(len(inventory['records']),10)
        providers=[r for r in inventory['records'] if r['kind']=='EXTERNAL_SYSTEM' and r['service']=='orders']
        self.assertEqual({r['input'] for r in providers},{'east','west'})
        self.assertEqual(len({r['recordDigest'] for r in providers}),2)
        self.assertNotIn(str(self.root),S['encode'](inventory))
        second,repeat,repeated=self.collect(first)
        self.assertEqual(repeat['counters']['prepareCalls'],0);self.assertEqual(repeat['counters']['dependencyPartCalls'],0)
        self.assertEqual(repeat['counters']['recordsReused'],10)
        self.assertEqual(repeat['counters']['captureCalls'],0);self.assertEqual(repeat['counters']['sourceReadCalls'],0)
        self.assertEqual(repeat['counters']['sourceMaterializationCalls'],0)
        self.assertEqual(repeated['removed'],[])
        for path in [first/'inventory.json',*sorted((first/'objects').glob('*.json'))]:
            self.assertTrue(path.exists())

    def test_one_changed_use_imports_only_that_value_and_missing_parts_never_mark_deletion(self):
        first,_,old=self.collect();old_hash=hashlib.sha256((first/'inventory.json').read_bytes()).hexdigest()
        for service in ['orders','billing','warehouse']:
            previous=self.contexts[('east',service,'s1')]
            dep=copy.deepcopy(previous['dependency']);dep['normalized']['interactions'][0]['transport']['path']='/updated'
            dep['digest']=S['digest'](dep['normalized'])
            index=next(i for i,u in enumerate(dep['normalized']['interactions']) if u['from']['service']==service)
            self.contexts[('east',service,'s2')]=self.context('east',service,'east-s2',dep,index)
        selected=copy.deepcopy(self.selected)
        for service in selected['inputs'][0]['services']:service['snapshot']='east-s2'
        self.write(self.config,selected)
        _,result,current=self.collect(first)
        self.assertTrue(result['complete']);self.assertEqual(result['counters']['recordsImported'],1)
        self.assertEqual(result['counters']['recordsReused'],9)
        self.assertEqual(hashlib.sha256((first/'inventory.json').read_bytes()).hexdigest(),old_hash)
        self.fail='orders'
        # Force a changed record so failure occurs while reading its parts.
        self.contexts[('east','orders','s2')]['dependency']['normalized']['interactions'][0]['title']='Changed again'
        dep=self.contexts[('east','orders','s2')]['dependency'];dep['digest']=S['digest'](dep['normalized'])
        self.contexts[('east','orders','s2')]=self.context('east','orders','east-s2',dep,0)
        _,failed,partial=self.collect(first)
        self.assertFalse(failed['complete']);self.assertTrue(partial['issues'])
        self.assertFalse(any(r['input']=='east' and r['service']=='orders' for r in partial['removed']))

    def test_check_reloads_owning_roots_and_rejects_rehashed_header_substitution(self):
        first,_,inventory=self.collect()
        result=S['check'](self.config,self.local,self.clew,first,self.root/'check')
        self.assertTrue(result['complete']);self.assertEqual(result['recordsChecked'],10)
        self.assertEqual(result['counters']['prepareCalls'],0)
        self.assertEqual(result['counters']['dependencyPartCalls'],0)
        # A new self-digest does not turn an edited header into native evidence.
        inventory['records'][0]['title']='Forged identity'
        inventory['digest']=S['digest']({k:v for k,v in inventory.items() if k!='digest'})
        self.write(first/'inventory.json',inventory)
        rejected=S['check'](self.config,self.local,self.clew,first,self.root/'changed-check')
        self.assertFalse(rejected['complete'])
        self.assertIn('CATALOG_DECLARED_BINDING_CHANGED',[i['status'] for i in rejected['issues']])
        mapping=json.loads(self.local.read_bytes())
        mapping['inputs']['east']['root']=str(self.roots['west'])
        self.write(self.local,mapping)
        rejected=S['check'](self.config,self.local,self.clew,first,self.root/'wrong-root-check')
        self.assertFalse(rejected['complete'])
        self.assertIn('RETAINED_CATALOG_READ_FAILED',[i['status'] for i in rejected['issues']])

    def test_inspect_shares_equal_bodies_but_preserves_scoped_choices_and_rejects_changed_objects(self):
        first,_,inventory=self.collect()
        view=S['inspect'](first,['east/orders/EXTERNAL_SYSTEM:provider','east/billing/EXTERNAL_SYSTEM:provider'])
        self.assertEqual(len(view['bodies']),1)
        self.assertEqual(len(view['selected']),2)
        self.assertNotIn('configurationReferences',view['bodies'][0]['value'])
        both=S['inspect'](first,['east/orders/EXTERNAL_SYSTEM:provider','west/orders/EXTERNAL_SYSTEM:provider'])
        self.assertEqual(len(both['bodies']),2)
        record=inventory['records'][0]
        self.write(first/'objects'/record['object'],{'id':'provider','title':'Changed'})
        with self.assertRaises(S['Stop']) as error:S['inspect'](first)
        self.assertEqual(error.exception.status,'CACHED_DECLARATION_CHANGED')

    def test_changed_continuation_membership_is_incomplete_and_never_marks_deletion(self):
        first,_,_=self.collect()
        def changed(args,**kwargs):
            result=self.engine(args,**kwargs)
            if args[2:4]==['work','read']:
                request=json.loads(Path(args[args.index('--input')+1]).read_bytes())
                if request.get('cursor'):
                    value=json.loads(result.stdout);value['membershipDigest']='changed'
                    value['receiptDigest']=S['digest']({k:v for k,v in value.items() if k!='receiptDigest'})
                    return subprocess.CompletedProcess(args,0,S['encode'](value).encode(),b'')
            return result
        self.runner.side_effect=changed
        _,result,inventory=self.collect(first)
        self.assertFalse(result['complete']);self.assertEqual(result['records'],0)
        self.assertTrue(all(i['status']=='SEMANTIC_MEMBERSHIP_CHANGED' for i in result['issues']))
        self.assertEqual(inventory['removed'],[])


if __name__=='__main__':unittest.main()
