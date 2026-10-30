#!/usr/bin/env python3
"""Source-bound C# display: reversible markers, joins and scope rejection."""
import copy
import hashlib
from pathlib import Path
import runpy
import unittest

ROOT = Path(__file__).resolve().parent.parent
M = runpy.run_path(str(ROOT/'skills/codeclew/scripts/source_regions.py'))


def fixture():
    text = 'async Task Run() {\r\n    // π🙂\r\n    try { await Read(); } catch (Exception e) when (Ready(e)) { Log(); }\r\n    await Pause();\r\n}'
    raw = text.encode(); base = 19
    def span(value):
        start = raw.index(value.encode()); end = start+len(value.encode())
        return {'file':'Unit.cs','byteStart':base+start,'byteEnd':base+end,'start':start,'end':end,'startLine':1,'endLine':5}
    def node(i, parent, role, kind, value):
        return {'id':i,'parent':parent,'role':role,'kind':kind,'source':span(value)}
    nodes = [node(0,None,'BODY','BLOCK',text[text.index('{'):]),
             node(1,0,'STATEMENT','TRY','try { await Read(); } catch (Exception e) when (Ready(e)) { Log(); }'),
             node(2,1,'TRY_BODY','BLOCK','{ await Read(); }'), node(3,2,'STATEMENT','EXPRESSION','await Read();'),
             node(4,1,'CATCH','CATCH','catch (Exception e) when (Ready(e)) { Log(); }'),
             node(5,4,'CATCH_BODY','BLOCK','{ Log(); }'), node(6,5,'STATEMENT','EXPRESSION','Log();'),
             node(7,0,'STATEMENT','EXPRESSION','await Pause();')]
    sid = 'source'; digest = 'sha256:'+hashlib.sha256(raw).hexdigest()
    site = {'sourceId':sid,'file':'Unit.cs','byteStart':base,'byteEnd':base+len(raw),'sourceDigest':digest,'evidenceDigest':'facts'}
    source = {'id':sid,'service':'neutral','revision':'r1','file':'Unit.cs','text':text,'textDigest':digest,'evidenceDigest':'facts'}
    owner = {'id':'owner','kind':'SYMBOL','service':'neutral','symbol':'internal-signature', 'sourceIds':[sid],
             'normalized':{'name':'Run','scope':'main','outlineOwnerSource':site}}
    g = {'schema':'codeclew-csharp-source-regions/1.0','authority':'ROSLYN_SOURCE_REGIONS','interpretation':'LEXICAL_AND_CONDITIONAL_NORMAL_ONLY','execution':'NOT_ASSERTED',
         'ownerSymbolIdentity':'internal-signature','file':'Unit.cs','nodes':nodes,'sites':[{'region':3,'kind':'AWAIT','source':span('await Read()')},{'region':7,'kind':'AWAIT','source':span('await Pause()')}],'expressions':[]}
    rel = {'normalContinuation':[{'from':i,'to':7} for i in (1,2,4,5)],'abrupt':[],
           'siteContexts':[{'site':0,'region':3,'protectedTry':1,'enclosingRegions':[3,2,1,0]},{'site':1,'region':7,'protectedTry':None,'enclosingRegions':[7,0]}]}
    record = {'id':'regions','kind':'CSHARP_SOURCE_REGIONS','service':'neutral','sourceIds':[sid], 'normalized':{
        'schema':'codeclew-csharp-source-regions-evidence/1.0','ownerId':'owner','ownerSymbolIdentity':'internal-signature','scope':'main','sourceSite':site,
        'file':'Unit.cs','compilerFactBinding':'facts','graph':g,'relationships':rel}}
    return {'record':record,'owner':owner,'source':source}


class SourceRegionsTest(unittest.TestCase):
    def test_reversible_source_with_compact_shared_join_and_await_scope(self):
        item=fixture(); before=copy.deepcopy(item)
        text, meta=M['project']([item])
        self.assertIn('If normal: 1,4 → 7.', text)
        self.assertIn('Await 3: in try 1.',text)
        self.assertIn('Await 7: no enclosing try body.',text)
        self.assertIn('when (Ready(e))',text)
        self.assertNotIn('byteStart',text); self.assertNotIn('internal-signature',text)
        self.assertNotIn('ROSLYN_SOURCE_REGIONS',text)
        displayed=text.split('```csharp\n',1)[1].split('\n```',1)[0].encode()
        for edit in reversed(meta['callables'][0]['displayEdits']):
            displayed=displayed[:edit['displayStart']]+displayed[edit['displayEnd']:]
        self.assertEqual(displayed,item['source']['text'].encode())
        self.assertEqual(item,before)
        self.assertEqual((text,meta),M['project']([item]))

    def test_original_owner_identity_and_utf8_boundaries_are_required(self):
        for field in ('source','scope','owner','range','parent','authority'):
            item=fixture(); n=item['record']['normalized']
            if field=='source': item['source']['text']+='changed'
            elif field=='scope': n['scope']='other'
            elif field=='owner': n['ownerId']='other'
            elif field=='range': n['graph']['nodes'][3]['source']['byteStart']=n['sourceSite']['byteEnd']+1
            elif field=='parent': n['graph']['nodes'][3]['parent']=3
            else: n['graph']['authority']='K2_FIR_CFG'
            with self.subTest(field=field), self.assertRaises(M['Stop']): M['project']([item])

    def test_cleanup_is_local_and_exit_scope_is_visible(self):
        item=fixture(); n=item['record']['normalized']; n['graph']['nodes'][1]['opaque']='FINALLY_COMPLETION_NOT_MODELED'
        n['relationships']['abrupt']=[{'from':6,'kind':'RETURN','targetScope':0,'continuePhase':None,'unknownCleanupBoundaries':[1]}]
        text,_=M['project']([item])
        self.assertIn('6 return → A exit; cleanup unresolved: 1.',text)
        self.assertIn('1: finally completion not modeled.',text)


if __name__=='__main__': unittest.main()
