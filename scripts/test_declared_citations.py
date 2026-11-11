"""Typed declared pages share the human pipeline without acquiring SOURCE proof."""
import copy
import json
from pathlib import Path
import runpy
import subprocess
import unittest

ROOT = Path(__file__).resolve().parent.parent
S = runpy.run_path(str(ROOT/'skills/codeclew/scripts/shared_catalog.py'))
M = runpy.run_path(str(ROOT/'skills/codeclew/scripts/page_citations.py'))
R = runpy.run_path(str(ROOT/'skills/codeclew/scripts/reader_sources.py'))
FIXTURE = runpy.run_path(str(ROOT/'scripts/test_shared_catalog.py'))
BASE = {name:getattr(FIXTURE['CollectionTest'],name)
        for name in ('setUp','write','context','page','engine','collect')}
F = runpy.run_path(str(ROOT/'scripts/test_author_citations.py'))
NATIVE_RUN = subprocess.run


class DeclaredCitationsTest(unittest.TestCase):
    setUp = BASE['setUp']
    write = BASE['write']
    context = BASE['context']
    page = BASE['page']
    engine = BASE['engine']
    collect = BASE['collect']

    def prepare(self):
        inventory,_,_=self.collect()
        out = self.root/'declared'; result=S['citations'](inventory,'east',out)
        manifest=out/'citations.json'; identity=result['manifestId']
        config=json.loads(self.local.read_bytes());config['manifests'][identity]='east'
        self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        return manifest,result['records'][0]['cite']

    def compose(self,fragments,label='page',**intent):
        path=self.root/(label+'.json');self.write(path,{'pages':[{'page':'catalog.md','fragments':fragments,**intent}]})
        out=self.root/(label+'-composed');M['compose'](path,out);return out

    def test_declared_only_page_replays_and_has_separate_checked_footer(self):
        manifest,handle=self.prepare()
        composed=self.compose([{'text':'# Catalog\n\n'},
            {'id':'declared-name','text':'Owned declared label.\n','declaredEvidence':[handle]}])
        bound=self.root/'bound';M['bind']([manifest],composed/'markdown',composed/'request.json',bound)
        self.assertEqual(M['check']([manifest],bound/'index.json')['claims'],1)
        mapped=json.loads((bound/'maps/catalog.md.json').read_bytes())
        self.assertEqual(mapped['schema'],M['TYPED_MAP'])
        self.assertNotIn('sources',mapped['claims'][0]);self.assertNotIn('evidence',mapped['claims'][0])
        self.assertEqual(mapped['claims'][0]['declarations'][0]['basisKind'],'DECLARED')
        R['M']['configure_engine'](self.clew,evidence_inputs=self.local)
        bundle=self.root/'footer.json';R['materialize'](bound/'index.json',[manifest],bundle)
        code="import {loadSources,sourceFooter} from "+json.dumps((ROOT/'skills/codeclew/examples/markdown-renderer/sources.mjs').as_uri())+"; const b=await loadSources(process.argv[1]); console.log(sourceFooter(b,'catalog.md','# Catalog\\n\\nOwned declared label.\\n','',null));"
        result=NATIVE_RUN(['node','--input-type=module','-e',code,str(bundle)],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        self.assertIn('Declared information',result.stdout);self.assertNotIn('Code fragments',result.stdout)
        self.assertNotIn(str(self.root),result.stdout)

    def test_declared_handle_cannot_satisfy_source_claim_and_source_cannot_satisfy_declaration(self):
        manifest,handle=self.prepare()
        composed=self.compose([{'id':'behavior','text':'Owned label.','evidence':[handle]}])
        with self.assertRaises(M['Stop']) as error:
            M['bind']([manifest],composed/'markdown',composed/'request.json',self.root/'refused-source')
        self.assertEqual(error.exception.status,'DECLARED_EVIDENCE_CANNOT_BIND_SOURCE_CLAIM')
        source,view,_=F['fixture'](self.root,'source-proof')
        value=json.loads(source.read_bytes());config=json.loads(self.local.read_bytes())
        config['manifests'][value['id']]='east';self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        composed=self.compose([{'id':'declared','text':'Owned label.','declaredEvidence':[view['records'][0]['cite']]}],'wrong-type')
        with self.assertRaises(M['Stop']) as error:
            M['bind']([source],composed/'markdown',composed/'request.json',self.root/'refused-declared')
        self.assertEqual(error.exception.status,'SOURCE_CANNOT_BIND_DECLARED_CLAIM')

    def test_mixed_page_preserves_source_map_locations_and_requires_explicit_declared_rebind(self):
        manifest,handle=self.prepare();source,view,_=F['fixture'](self.root,'source-proof')
        source_id=json.loads(source.read_bytes())['id'];config=json.loads(self.local.read_bytes())
        config['manifests'][source_id]='east';self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        source_handle=view['records'][0]['cite']
        fragments=[{'id':'source','text':'Source [location](cite:'+source_handle+').\n','evidence':[source_handle]},
                   {'id':'declared','text':'Declared owned label.\n','declaredEvidence':[handle]}]
        composed=self.compose(fragments)
        mapping=self.root/'repositories.json';self.write(mapping,{'east/owned':{'repository':'https://example.invalid/owned',
            'urlTemplate':'https://example.invalid/owned/blob/{revision}/{path}#L{startLine}-L{endLine}'}})
        bound=self.root/'mixed';M['bind']([manifest,source],composed/'markdown',composed/'request.json',bound,repositories=mapping)
        self.assertEqual(M['check']([manifest,source],bound/'index.json')['claims'],2)
        body=(bound/'markdown/catalog.md').read_bytes();mapped=json.loads((bound/'maps/catalog.md.json').read_bytes())
        self.assertNotIn(b'cite:',body)
        for claim in mapped['claims']:
            loc=claim['location'];self.assertEqual(body[loc['startByte']:loc['endByte']].decode(),claim['text'])
        other=S['citations'](self.root/'run-1','west',self.root/'west-declared')
        west=self.root/'west-declared/citations.json';config['manifests'][other['manifestId']]='west';self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        updated=copy.deepcopy(fragments);updated[1]['declaredEvidence']=[other['records'][0]['cite']]
        current=self.compose(updated,'updated')
        with self.assertRaises(M['Stop']) as error:
            M['bind']([manifest,source,west],current/'markdown',current/'request.json',self.root/'unapproved-rebind',
                      previous=bound/'index.json',repositories=mapping)
        self.assertEqual(error.exception.status,'EXPLICIT_CLAIM_REMOVAL_OR_REBIND_REQUIRED')
        accepted=self.compose(updated,'explicit-rebind',replace=['declared'])
        next_bound=self.root/'updated-bound'
        M['bind']([manifest,source,west],accepted/'markdown',accepted/'request.json',next_bound,
                  previous=bound/'index.json',repositories=mapping)
        self.assertEqual(M['check']([manifest,source,west],next_bound/'index.json')['claims'],2)
        removed=self.compose(updated[:1],'explicit-removal',remove=['declared'])
        source_only=self.root/'source-only'
        M['bind']([source,west],removed/'markdown',removed/'request.json',source_only,
                  previous=next_bound/'index.json',repositories=mapping)
        self.assertEqual(M['check']([source,west],source_only/'index.json')['claims'],1)
        self.assertEqual(json.loads((source_only/'maps/catalog.md.json').read_bytes())['schema'],
                         'codeclew-authored-sources/1.0')

    def test_wrong_root_and_rehashed_declared_value_cannot_bypass_native_header(self):
        manifest,handle=self.prepare()
        config=json.loads(self.local.read_bytes());config['inputs']['east']['root']=str(self.roots['west'])
        self.write(self.local,config);M['configure_engine'](self.clew,evidence_inputs=self.local)
        with self.assertRaises(M['Stop']) as error:M['C']['load'](manifest)
        self.assertEqual(error.exception.status,'RETAINED_CATALOG_READ_FAILED')
        config['inputs']['east']['root']=str(self.roots['east'])
        value=json.loads(manifest.read_bytes());record=value['records'][0]
        original=json.loads((manifest.parent/record['value']).read_bytes());original['title']='Forged declaration'
        record['basis']['recordDigest']=S['digest'](original)
        record['value']='objects/'+record['basis']['recordDigest'][7:]+'.json'
        self.write(manifest.parent/record['value'],original)
        value['id']=S['digest']({k:v for k,v in value.items() if k!='id'});self.write(manifest,value)
        config['manifests'][value['id']]='east';self.write(self.local,config)
        M['configure_engine'](self.clew,evidence_inputs=self.local)
        with self.assertRaises(M['Stop']) as error:M['C']['load'](manifest)
        self.assertEqual(error.exception.status,'DECLARED_NATIVE_BINDING_MISMATCH')


if __name__=='__main__':unittest.main()
