"""Protocol doubles for explicit historical glossary import/binding, not model quality."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT=Path(__file__).resolve().parent.parent
S=runpy.run_path(str(ROOT/'skills/codeclew/scripts/shared_catalog.py'))
G=runpy.run_path(str(ROOT/'skills/codeclew/scripts/glossary_catalog.py'),
                 init_globals={'S':S,'Stop':S['Stop'],'digest':S['digest']})
M=runpy.run_path(str(ROOT/'skills/codeclew/scripts/page_citations.py'))
R=runpy.run_path(str(ROOT/'skills/codeclew/scripts/reader_sources.py'))
RUN=subprocess.run


class GlossaryCatalogTest(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory();self.addCleanup(self.temp.cleanup);self.root=Path(self.temp.name)
        self.docs=self.root/'east';self.docs.mkdir();self.other=self.root/'wrong';self.other.mkdir()
        self.clew=self.root/'trusted-clew';self.clew.write_text('protocol double')
        self.work='a'*64;self.review='b'*32;self.snapshot='snapshot-1';self.empty=False;self.calls=[]
        self.collection=self.root/'collection.json';self.write(self.collection,{'schema':S['COLLECTION'],'id':'owned','revision':'r1',
            'inputs':[{'id':'east','docset':'east','services':[{'id':'orders','snapshot':self.snapshot}]}]})
        self.local=self.root/'local.json';self.write(self.local,{'schema':'codeclew-evidence-inputs/1.0',
            'inputs':{'east':{'root':str(self.docs)}},'manifests':{}})
        self.selection=self.root/'selection.json';self.write(self.selection,[{'input':'east','service':'orders','work':self.work,'reviewRun':self.review}])
        self.entries=[{'id':'device','label':'Device','kind':'term','definition':{'text':'Owned saved definition.',
             'evidence':['s1'],'uncertainty':'Only the saved packet scope.','glossaryRefs':['device-type']}},
            {'id':'device-type','label':'DeviceDto','kind':'technical_carrier','technicalNames':['example.DeviceDto'],
             'subjectRefs':['original-declaration'],'definition':{'text':'Owned representation definition.',
             'evidence':['s1'],'uncertainty':'No business entity inferred.','glossaryRefs':[]}}]
        self.mock=patch('subprocess.run',side_effect=self.engine);self.mock.start();self.addCleanup(self.mock.stop)

    def write(self,path,value):path.write_text(S['encode'](value))

    def engine(self,args,**kwargs):
        self.calls.append(args);self.assertEqual(args[2:4],['work','answer-context'])
        self.assertIn('--include-glossary',args);self.assertNotIn('find-answer',args);self.assertNotIn('capture',args)
        if Path(args[args.index('--root')+1])!=self.docs:
            return subprocess.CompletedProcess(args,2,b'',b'Unknown selected Work')
        self.assertEqual(args[args.index('--work')+1],self.work);self.assertEqual(args[args.index('--review-run')+1],self.review)
        all=[] if self.empty else copy.deepcopy(self.entries)
        ids=[args[i+1] for i,arg in enumerate(args) if arg=='--glossary-id']
        chosen=[(i,e) for i,e in enumerate(all) if not ids or e['id'] in ids]
        inventory=[{'id':e['id'],'label':e['label'],'kind':e['kind'],'pointer':'/glossary/'+str(i),'entryDigest':S['digest'](e)} for i,e in enumerate(all)]
        answer_digest=S['digest']({'glossary':all});packet_digest=S['digest']({'packet':'owned'})
        glossary={'schema':'codeclew-saved-glossary/1.0','work':self.work,'subject':'service:orders','reviewRun':self.review,
            'savedSnapshot':'saved-snapshot','comparedSnapshot':self.snapshot,'answerSchema':'codeclew-operation-answer/1.2',
            'answerDigest':answer_digest,'packetDigest':packet_digest,'reviewDigest':S['digest']({'verdict':'APPROVE'}),
            'provenance':{'work':self.work,'reviewRun':self.review,'answerDigest':answer_digest,'packetDigest':packet_digest,
                          'meaningReview':'MODEL_APPROVED','sourceContext':'SAVED_SNAPSHOT_NOT_REVERIFIED','runtime':'UNKNOWN'},
            'status':'NO_SAVED_GLOSSARY' if self.empty else 'SAVED_APPROVED_GLOSSARY','inventory':inventory,
            'entries':[e for _,e in chosen],
            'entryBindings':[{k:inventory[i][k] for k in ('id','pointer','entryDigest')} for i,_ in chosen],
            'evidenceBindings':[{'label':'s1','id':'original-source','kind':'SOURCE','recordDigest':'sha256:original-row',
                                 'projection':'ORIGINAL_AUDIT_BINDING_NOT_SOURCE_CONTENT','workReference':'s1','deliveredToAuthor':True}],
            'availableEntries':len(all),'selectedEntries':len(chosen),'omittedEntries':len(all)-len(chosen),
            'authority':'HISTORICAL_MODEL_APPROVAL_AGAINST_SAVED_PACKET_NOT_SOURCE_OR_RUNTIME_PROOF'}
        value={'schema':'codeclew-saved-answer-context/1.1','work':self.work,'subject':'service:orders','reviewRun':self.review,
            'savedSnapshot':'saved-snapshot','comparedSnapshot':self.snapshot,'packetDigest':packet_digest,
            'savedContextDigest':S['digest']({'old':'context'}),'comparedContextDigest':S['digest']({'current':'context'}),
            'meaningReview':'MODEL_APPROVED_AGAINST_SAVED_PACKET','capturedContext':{'status':'CURRENT'},
            'captures':0,'agentInvocations':0,'writes':0,'glossary':glossary}
        return subprocess.CompletedProcess(args,0,S['encode'](value).encode(),b'')

    def prepare(self):
        inventory=self.root/'glossary';result=G['collect'](self.collection,self.local,self.clew,self.selection,inventory)
        output=self.root/'citations';citations=G['citations'](inventory,'east',output)
        config=json.loads(self.local.read_bytes());config['manifests'][citations['manifestId']]='east';self.write(self.local,config)
        return inventory,output/'citations.json',citations['records'][0]['cite'],result

    def test_explicit_import_keeps_original_glossary_kind_qualification_and_packet_bindings(self):
        inventory,manifest,handle,result=self.prepare()
        self.assertTrue(result['complete']);self.assertEqual(result['entries'],2)
        self.assertEqual(result['counters']['glossaryCalls'],1);self.assertEqual(result['sharedDefinitionsEstablished'],0)
        self.assertEqual(result['counters']['captureCalls'],0);self.assertEqual(result['counters']['sourceReadCalls'],0)
        M['configure_engine'](self.clew,evidence_inputs=self.local);before=len(self.calls)
        _,_,_,entries=M['C']['load'](manifest)
        self.assertEqual(len(self.calls)-before,1)  # Both entries share one bounded native call.
        self.assertEqual(entries[handle]['originalEntry'],self.entries[0]);self.assertEqual(entries[handle]['basisKind'],'SAVED_GLOSSARY')
        self.assertEqual(entries[handle]['provenance']['runtime'],'UNKNOWN')
        self.assertEqual(entries[handle]['snapshot'],'saved-snapshot')
        self.assertEqual(entries[handle]['comparedSnapshot'],self.snapshot)
        selected=S['inspect'](inventory,[S['record_selector'](G['load_inventory'](inventory)['records'][0])])
        self.assertEqual(selected['selected'][0]['entry'],self.entries[0])

    def test_saved_glossary_uses_same_page_pipeline_with_distinct_footer_and_no_source_substitution(self):
        _,manifest,handle,_=self.prepare();M['configure_engine'](self.clew,evidence_inputs=self.local)
        original=self.root/'original.json';self.write(original,{'pages':[{'page':'term.md','fragments':[
            {'text':'# Device\n\n'},{'id':'definition','text':'Owned saved definition.\n','declaredEvidence':[handle]}]}]})
        composed=self.root/'composed';M['compose'](original,composed);bound=self.root/'bound'
        M['bind']([manifest],composed/'markdown',composed/'request.json',bound)
        self.assertEqual(M['check']([manifest],bound/'index.json')['claims'],1)
        R['M']['configure_engine'](self.clew,evidence_inputs=self.local);bundle=self.root/'footer.json'
        R['materialize'](bound/'index.json',[manifest],bundle)
        code="import {loadSources,sourceFooter} from "+json.dumps((ROOT/'skills/codeclew/examples/markdown-renderer/sources.mjs').as_uri())+";const b=await loadSources(process.argv[1]);console.log(sourceFooter(b,'term.md','# Device\\n\\nOwned saved definition.\\n','',null));"
        result=RUN(['node','--input-type=module','-e',code,str(bundle)],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr);self.assertIn('Saved glossary',result.stdout)
        self.assertIn('Only the saved packet scope.',result.stdout);self.assertNotIn('Code fragments',result.stdout)
        request=json.loads((composed/'request.json').read_bytes());claim=request['pages'][0]['claims'][0]
        claim['evidence']=claim.pop('declaredEvidence');path=self.root/'source-request.json';self.write(path,request)
        with self.assertRaises(M['Stop']) as error:M['bind']([manifest],composed/'markdown',path,self.root/'refused')
        self.assertEqual(error.exception.status,'DECLARED_EVIDENCE_CANNOT_BIND_SOURCE_CLAIM')

    def test_wrong_root_and_rehashed_modified_definition_refuse_native_replay(self):
        _,manifest,_,_=self.prepare();config=json.loads(self.local.read_bytes())
        config['inputs']['east']['root']=str(self.other);self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        with self.assertRaises(M['Stop']) as error:M['C']['load'](manifest)
        self.assertEqual(error.exception.status,'RETAINED_CATALOG_READ_FAILED')
        config['inputs']['east']['root']=str(self.docs)
        value=json.loads(manifest.read_bytes());record=value['records'][0]
        obj=json.loads((manifest.parent/'objects'/record['object']).read_bytes());obj['glossary']['entries'][0]['definition']['text']='Forged meaning.'
        record['object']=S['digest'](obj)[7:]+'.json';record['entryDigest']=S['digest'](obj['glossary']['entries'][0])
        self.write(manifest.parent/'objects'/record['object'],obj);value['id']=S['digest']({k:v for k,v in value.items() if k!='id'})
        self.write(manifest,value);config['manifests'][value['id']]='east';self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        with self.assertRaises(M['Stop']) as error:M['C']['load'](manifest)
        self.assertEqual(error.exception.status,'SAVED_GLOSSARY_NATIVE_BINDING_MISMATCH')

    def test_missing_or_empty_glossary_is_a_retained_gap_not_a_shared_definition(self):
        self.empty=True;output=self.root/'empty'
        result=G['collect'](self.collection,self.local,self.clew,self.selection,output)
        self.assertFalse(result['complete']);self.assertEqual(result['entries'],0)
        self.assertEqual(result['issues'][0]['status'],'NO_SAVED_GLOSSARY')
        with self.assertRaises(S['Stop']):G['citations'](output,'east',self.root/'empty-citations')

    def test_catalog_alias_requires_exact_glossary_scope_and_never_turns_carrier_into_entity(self):
        inventory,_,_,_=self.prepare()
        rows,counters,issues=G['checked_records'](self.collection,self.local,self.clew,inventory,self.root/'glossary-check')
        self.assertEqual(counters['glossaryCalls'],1);self.assertEqual(issues,[])
        carrier=next(r for r in rows if r['localId']=='device-type')
        self.assertEqual(carrier['kind'],'SAVED_GLOSSARY');self.assertEqual(carrier['value']['kind'],'technical_carrier')
        K=runpy.run_path(str(ROOT/'skills/codeclew/scripts/semantic_catalog.py'),
                         init_globals={'S':S,'Stop':S['Stop'],'digest':S['digest']})
        selector=S['record_selector'](carrier)
        index={('east','east','SAVED_GLOSSARY','device-type'):[(selector,carrier)]}
        alias={'input':'east','docset':'east','localId':'device-type','service':'orders',
               'work':self.work,'reviewRun':self.review}
        self.assertEqual(K['alias_candidates']({'kind':'representation','scope':{'docset':'east','service':'orders'}},alias,index),[selector])
        alias.pop('work');alias.pop('reviewRun')
        with self.assertRaises(S['Stop']):K['alias_candidates']({'kind':'concept'},alias,index)


if __name__=='__main__':unittest.main()
