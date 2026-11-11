"""Confirmed human decisions survive proposals and bounded inventory refreshes."""
import copy
import json
from pathlib import Path
import runpy
import unittest

ROOT=Path(__file__).resolve().parent.parent
S=runpy.run_path(str(ROOT/'skills/codeclew/scripts/shared_catalog.py'))
K=runpy.run_path(str(ROOT/'skills/codeclew/scripts/semantic_catalog.py'),
                 init_globals={'S':S,'Stop':S['Stop'],'digest':S['digest']})
FIXTURE=runpy.run_path(str(ROOT/'scripts/test_shared_catalog.py'))
METHODS={name:getattr(FIXTURE['CollectionTest'],name) for name in ('setUp','write','context','page','engine','collect')}


class SemanticCatalogTest(unittest.TestCase):
    setUp=METHODS['setUp'];write=METHODS['write'];context=METHODS['context']
    page=METHODS['page'];engine=METHODS['engine'];collect=METHODS['collect']

    def prepare(self):
        inventory,_,value=self.collect()
        locations=[{'record':S['record_selector'](r),'target':{'docset':r['docset'],'page':r['service']+'.html','anchor':'integration'}}
                   for r in value['records'] if r['kind']=='EXTERNAL_USE']
        record={'id':'payments','kind':'external','title':'Payments','target':{'docset':'catalog','page':'payments.html'},
                'aliases':[{'input':'east','docset':'east','localId':'provider'}],
                'status':'CONFIRMED','decision':{'origin':'human','rationale':'Owned test decision'},
                'original':{'page':'payments.md','fragments':[{'text':'# Payments\n\n'},
                    {'id':'definition','text':'Owned authored fragment.\n','declaredEvidence':['d-fixture:r1']}]}}
        decisions={'schema':K['INPUT'],'collection':'owned','revision':'r1','baseDigest':None,
                   'records':[record],'locations':locations,'remove':[]}
        path=self.root/'decisions.json';self.write(path,decisions)
        docsets=[{'id':i['docset'],'title':i['docset'],'version':'r1',
                  'baseURL':'https://docs.example.invalid/'+i['docset']+'/',
                  'services':[{'id':s['id'],'title':s['id'],'overview':{'docset':i['docset'],'page':s['id']+'.html'}} for s in i['services']]}
                 for i in self.selected['inputs']]
        docsets.append({'id':'catalog','title':'Catalog','version':'r1','baseURL':'https://docs.example.invalid/catalog/','services':[]})
        registry={'schema':'codeclew-cross-docset-routes/1.1','revision':'r1','docsets':docsets,
                  'externalActors':[],'routes':[]};routes=self.root/'routes.json';self.write(routes,registry)
        self.output_number=0
        return inventory,path,routes,decisions

    def link(self,inventory,path,routes,previous=None):
        self.output_number+=1;output=self.root/('linked-'+str(self.output_number))
        result=K['links'](self.config,self.local,self.clew,inventory,path,routes,output,previous)
        return output,result,json.loads((output/'catalog.json').read_bytes())

    def test_scoped_alias_does_not_merge_same_name_in_other_root_and_original_is_unchanged(self):
        inventory,path,routes,decisions=self.prepare();output,result,saved=self.link(inventory,path,routes)
        self.assertTrue(result['complete']);self.assertEqual(result['records'],1)
        registry=json.loads((output/'routes.json').read_bytes());record=registry['semanticCatalog']['records'][0]
        self.assertEqual({u['docset'] for u in record['uses']},{'east'});self.assertEqual(len(record['uses']),3)
        self.assertTrue(any(s.startswith('west/') for s in result['unmappedCandidates']))
        self.assertEqual(json.loads((output/'originals.catalog.json').read_bytes())['pages'][0],decisions['records'][0]['original'])
        self.assertEqual(result['counters']['captureCalls'],0);self.assertEqual(result['counters']['proseAuthorCalls'],0)
        _,again,_=self.link(inventory,path,routes,output)
        self.assertEqual(again['counters']['originalPagesReused'],1)
        self.assertEqual(again['counters']['originalPagesChanged'],0)

    def test_proposal_preserves_confirmed_mapping_and_new_definition_needs_exact_replace(self):
        inventory,path,routes,decisions=self.prepare();output,_,saved=self.link(inventory,path,routes)
        old=saved['records'][0]['definitionDigest'];proposal=copy.deepcopy(decisions)
        proposal['records'][0]['status']='PROPOSED';proposal['records'][0]['decision']['origin']='model'
        proposal['records'][0]['aliases']=[{'input':'west','docset':'west','localId':'provider'}]
        self.write(path,proposal);_,result,current=self.link(inventory,path,routes,output)
        self.assertTrue(result['complete']);self.assertEqual(result['unappliedProposals'],1)
        self.assertEqual(current['records'][0]['definitionDigest'],old)
        changed=copy.deepcopy(decisions);changed['records'][0]['original']['fragments'][1]['text']='Changed definition.\n'
        self.write(path,changed)
        with self.assertRaises(S['Stop']) as error:self.link(inventory,path,routes,output)
        self.assertEqual(error.exception.status,'EXPLICIT_CANONICAL_REPLACEMENT_REQUIRED')
        changed['records'][0]['replaces']=old;self.write(path,changed)
        _,accepted,current=self.link(inventory,path,routes,output)
        self.assertTrue(accepted['complete']);self.assertNotEqual(current['records'][0]['definitionDigest'],old)

    def test_missing_local_target_is_bounded_and_does_not_delete_confirmed_decision(self):
        inventory,path,routes,decisions=self.prepare();output,_,saved=self.link(inventory,path,routes)
        decisions['locations']=decisions['locations'][1:];self.write(path,decisions)
        _,result,current=self.link(inventory,path,routes,output)
        self.assertFalse(result['complete']);self.assertEqual(result['records'],0)
        self.assertEqual(current['records'][0]['definitionDigest'],saved['records'][0]['definitionDigest'])
        self.assertEqual(result['issues'][0]['status'],'LOCAL_DETAIL_TARGET_MISSING')
        self.assertEqual(current['removed'],[])

    def test_authorized_agent_or_import_can_establish_mapping_without_claiming_human_origin(self):
        inventory,path,routes,decisions=self.prepare()
        for origin in ['agent','model','import']:
            current=copy.deepcopy(decisions);current['records'][0]['decision']['origin']=origin
            self.write(path,current);_,result,saved=self.link(inventory,path,routes)
            self.assertTrue(result['complete'])
            self.assertEqual(saved['records'][0]['definition']['decision']['origin'],origin)
            self.assertEqual(saved['records'][0]['definition']['status'],'CONFIRMED')

    def test_invalid_previous_snapshot_cannot_silently_start_a_fresh_catalog(self):
        _,path,_,_=self.prepare();previous=self.root/'corrupt-previous';previous.mkdir()
        self.write(previous/'catalog.json',{})
        with self.assertRaises(S['Stop']) as error:K['decisions'](path,previous)
        self.assertEqual(error.exception.status,'CATALOG_BASE_DIGEST_MISMATCH')


if __name__=='__main__':unittest.main()
