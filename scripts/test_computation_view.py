#!/usr/bin/env python3
"""Behavioral checks for a deterministic source-bound Work computation view."""
import copy
import hashlib
from pathlib import Path
import runpy
import unittest
import tempfile
import argparse
import json

ROOT = Path(__file__).resolve().parent.parent
M = runpy.run_path(str(ROOT / 'skills/codeclew/scripts/computation_view.py'))


def fixture():
    text = 'fun example(flag: Boolean) {\r\n    if (flag) send("π🙂") else error("bad")\r\n}\r\n'
    raw = text.encode(); origin = 9
    def span(a, b):
        return {'coordinateDomain': 'ORIGINAL_UTF8_BYTES', 'file': 'src/Example.kt', 'ownerSymbolIdentity': 'callable:example',
                'compilationScope': 'main', 'ownerByteStart': origin, 'ownerByteEnd': origin+len(raw),
                'byteStart': origin+a, 'byteEnd': origin+b, 'fullCompilationSourceDigest': 'sha256:file'}
    at = raw.index(b'if ('); stop = raw.index(b'\r\n}', at)
    nodes = [{'id': 0, 'parent': None, 'role': 'BODY', 'kind': 'KtBlockExpression', 'origin': 'SOURCE_DECLARED', 'sourceSpan': span(0,len(raw))},
             {'id': 1, 'parent': 0, 'role': 'CHILD', 'kind': 'KtIfExpression', 'origin': 'SOURCE_DECLARED', 'sourceSpan': span(at,stop)}]
    condition = raw.index(b'flag)',at)
    nodes.append({'id': 2, 'parent': 1, 'role': 'CONDITION', 'kind': 'KtNameReferenceExpression', 'origin': 'SOURCE_DECLARED', 'sourceSpan': span(condition,condition+4)})
    return {'record': {'id': 'symbol:example', 'symbol': 'example', 'normalized': {'documentation': {
        'sourceStructure': {'schema': 'codeclew-kotlin-source-structure/1.0', 'authority': 'SOURCE_CONTAINMENT_ONLY', 'execution': 'NOT_ASSERTED', 'nodes': nodes, 'gaps': [], 'complete': True, 'omittedScans': 0},
        'events': [{'kind': 'CALL', 'sourceStructureId': 1, 'kafka': {'adapter':'SPRING_KAFKA_LITERAL_TOPIC/1.0', 'topic':'example.changed'}}],
        'boundaries': ['CALLBACK_INVOCATION_ORDER_AND_COUNT_NOT_ESTABLISHED'],
        'semanticInputs': {'calls': [{'targetIdentity':'callable:example', 'resolution':'COMPILER_EXACT'}], 'returns': [{'resolution':'UNKNOWN'}]}}}},
        'source': {'text':text, 'textDigest':'sha256:'+hashlib.sha256(raw).hexdigest(), 'file':'src/Example.kt', 'revision':'abc', 'startLine':2, 'endLine':5,
                   'occurrence': {'startByte':origin, 'endByte':origin+len(raw)}}}


class ComputationViewTest(unittest.TestCase):
    def test_source_once_structure_and_original_metadata(self):
        item = fixture(); before=copy.deepcopy(item)
        text, meta=M['project']([item])
        self.assertEqual(text.count(item['source']['text']),1)
        self.assertIn('CONDITION parent=n1',text)
        self.assertIn('compiler target -> c1',text)
        self.assertIn('declared-send',text); self.assertIn('example.changed',text); self.assertIn('delivery unknown',text)
        self.assertEqual(meta['callables'][0]['typedBoundaries'][0]['eventPointer'],'/normalized/documentation/events/0')
        self.assertEqual(meta['callables'][0]['record']['normalized']['documentation']['semanticInputs']['returns'],[{'resolution':'UNKNOWN'}])
        self.assertFalse(meta['wholeProgramComplete']);self.assertTrue(meta['omissions'])
        self.assertEqual(item,before)

    def test_shared_body_is_a_reference_and_output_is_deterministic(self):
        item=fixture()
        text,metadata=M['project']([item,item])
        self.assertEqual(text.count(item['source']['text']),1)
        self.assertIn('Source: c1',text)
        self.assertEqual((text,metadata),M['project']([item,item]))

    def test_exact_utf8_ranges_and_invalid_containment(self):
        item=fixture(); item['record']['normalized']['documentation']['sourceStructure']['nodes'][2]['sourceSpan']['byteEnd']=10000
        with self.assertRaises(M['Stop']):M['project']([item])
        item=fixture();item['source']['text']+='changed'
        with self.assertRaises(M['Stop']):M['project']([item])
        item=fixture();item['record']['normalized']['documentation']['sourceStructure']['nodes'][2]['parent']=2
        with self.assertRaises(M['Stop']):M['project']([item])

    def test_budget_gap_stays_visible_and_original_gap_is_retained(self):
        item=fixture();structure=item['record']['normalized']['documentation']['sourceStructure']
        structure.update(complete=False,omittedScans=3,gaps=[{'reason':'TRAVERSAL_BUDGET','role':'THEN'}])
        text,meta=M['project']([item])
        self.assertIn('GAP: 3 omitted traversals',text)
        self.assertEqual(meta['callables'][0]['record']['normalized']['documentation']['sourceStructure']['gaps'],structure['gaps'])

    def test_exact_return_target_uses_local_owner_and_unknown_stays_visible(self):
        item=fixture();sem=item['record']['normalized']['documentation']['semanticInputs']
        sem['returns']=[{'resolution':'COMPILER_EXACT','byteStart':10,'byteEnd':16,'nonLocal':False,
                         'targetRegion':{'kind':'NAMED_FUNCTION','ownerSymbolIdentity':'callable:example'}},
                        {'resolution':'UNKNOWN'}]
        text,meta=M['project']([item])
        self.assertIn('target=c1 kind=NAMED_FUNCTION resolution=COMPILER_EXACT',text)
        self.assertIn('target=UNRESOLVED_LOCAL_TARGET kind=UNKNOWN resolution=UNKNOWN',text)
        self.assertEqual(meta['callables'][0]['record']['normalized']['documentation']['semanticInputs'],sem)

    def test_original_owner_source_binding_without_occurrence_is_exact_and_refuses_changed_digest(self):
        item=fixture();source=item['source'];occurrence=source.pop('occurrence')
        source.update(id='source:example',evidenceDigest='sha256:evidence')
        span=item['record']['normalized']['documentation']['sourceStructure']['nodes'][0]['sourceSpan']
        item['record']['normalized']['outlineOwnerSource']={
            'sourceStatus':'SOURCE_RETAINED','sourceId':source['id'],'sourceDigest':source['textDigest'],
            'evidenceDigest':source['evidenceDigest'],'file':source['file'],'startLine':source['startLine'],'endLine':source['endLine'],
            'byteStart':occurrence['startByte'],'byteEnd':occurrence['endByte'],'fullCompilationSourceDigest':span['fullCompilationSourceDigest']}
        text,meta=M['project']([item])
        self.assertIn(source['text'],text)
        self.assertEqual(meta['callables'][0]['sourceCoordinateAuthority'],'COMPILER_BOUND_OUTLINE_OWNER_SOURCE')
        item['record']['normalized']['outlineOwnerSource']['sourceDigest']='sha256:changed'
        with self.assertRaises(M['Stop']):M['project']([item])

    def test_compact_source_keeps_raw_bytes_and_only_adds_bound_markers(self):
        item=fixture();doc=item['record']['normalized']['documentation']
        doc['events'][0]['sourceSpan']=doc['sourceStructure']['nodes'][1]['sourceSpan']
        doc['semanticInputs']['returns']=[]
        doc['semanticInputs']['calls'][0]['byteStart']=item['source']['occurrence']['startByte']+item['source']['text'].encode().index(b'send(')
        text,meta=M['compact_project']([item])
        self.assertNotIn('bytes=',text);self.assertNotIn('SOURCE_DECLARED',text)
        self.assertNotIn('COMPILER_EXACT',text);self.assertNotIn('parent=',text)
        self.assertIn('/*→A*/',text);self.assertIn('/*Kafka*/',text)
        self.assertIn('example.changed',text);self.assertIn('delivery unknown',text)
        raw=item['source']['text'].encode();edits=meta['callables'][0]['displayEdits']
        displayed=raw
        for edit in reversed(edits):
            displayed=displayed[:edit['start']]+edit['replacement'].encode()+displayed[edit['end']:]
        for edit in reversed(edits):
            displayed=displayed[:edit['renderedStart']]+edit['original'].encode()+displayed[edit['renderedEnd']:]
        self.assertEqual(displayed,raw)
        self.assertEqual(meta['callables'][0]['source']['text'],item['source']['text'])
        self.assertEqual(meta['compactAnchors']['A']['source']['textDigest'],item['source']['textDigest'])

    def test_compact_return_targets_the_exact_same_line_callback(self):
        item=fixture();source=item['source'];text='fun example() { val first = { 1 }; run { return@run } }\n';raw=text.encode();base=9
        source.update(text=text,textDigest='sha256:'+hashlib.sha256(raw).hexdigest(),endLine=2,
                      occurrence={'startByte':base,'endByte':base+len(raw)})
        doc=item['record']['normalized']['documentation'];template=doc['sourceStructure']['nodes'][0]['sourceSpan']
        def span(a,b):return {**template,'ownerByteEnd':base+len(raw),'byteStart':base+a,'byteEnd':base+b}
        second=raw.index(b'{ return@run');end=raw.index(b'}',second)+1
        nodes=[{'id':0,'parent':None,'role':'BODY','kind':'KtBlockExpression','origin':'SOURCE_DECLARED','sourceSpan':span(0,len(raw))},
               {'id':1,'parent':0,'role':'CALLBACK','kind':'KtLambdaExpression','origin':'SOURCE_DECLARED','sourceSpan':span(raw.index(b'{ 1 }'),raw.index(b'{ 1 }')+5)},
               {'id':2,'parent':0,'role':'CALLBACK','kind':'KtLambdaExpression','origin':'SOURCE_DECLARED','sourceSpan':span(second,end)}]
        doc['sourceStructure']['nodes']=nodes;doc['events']=[]
        doc['semanticInputs']={'calls':[],'returns':[{'resolution':'COMPILER_EXACT','byteStart':base+raw.index(b'return@run'),
             'byteEnd':base+raw.index(b'return@run')+10,'targetRegion':{'kind':'ANONYMOUS_FUNCTION','byteStart':base+second,'byteEnd':base+end}}]}
        rendered,meta=M['compact_project']([item])
        self.assertIn('first = { 1 }',rendered)
        self.assertIn('run /*a1*/{ /*→a1*/return@run',rendered)
        self.assertEqual(meta['compactAnchors']['a1']['nodeId'],2)

    def test_compact_whole_file_binding_displays_only_the_selected_owner(self):
        item=fixture();original=item['source']['text'];prefix='// start\n';suffix='// end\n'
        item['source'].update(text=prefix+original+suffix,occurrence={'startByte':0,'endByte':len((prefix+original+suffix).encode())},
                              textDigest='sha256:'+hashlib.sha256((prefix+original+suffix).encode()).hexdigest())
        doc=item['record']['normalized']['documentation'];doc['events']=[];doc['semanticInputs']={'calls':[],'returns':[]}
        rendered,meta=M['compact_project']([item])
        self.assertIn(original,rendered);self.assertNotIn('// start',rendered);self.assertNotIn('// end',rendered)
        self.assertEqual(meta['callables'][0]['source']['text'],prefix+original+suffix)
        self.assertEqual(meta['callables'][0]['displayOriginalSha256'],hashlib.sha256(original.encode()).hexdigest())

    def test_compact_missing_bindings_and_traversal_gap_stay_explicit(self):
        item=fixture();doc=item['record']['normalized']['documentation']
        doc['events']=[];del doc['semanticInputs'];doc['sourceStructure'].update(complete=False,omittedScans=1)
        text,meta=M['compact_project']([item])
        self.assertIn('call/return bindings unavailable',text)
        self.assertIn('structure budget gap',text)
        self.assertEqual(meta['callables'][0]['semanticInputsStatus'],'MISSING')

    def test_acquisition_byte_accounting_matches_the_written_author_file(self):
        item=fixture();record=item['record'];source=item['source']
        record.update(kind='SYMBOL',service='example',sourceIds=['source:example'])
        source.update(id='source:example',service='example')
        record['normalized']['documentation']['events']=[]
        record['normalized']['documentation']['semanticInputs']={'calls':[],'returns':[]}
        class Packet:
            def __init__(self,args):self.args=args;self.receipts=[]
            def call(self,*args,**kwargs):return {'items':[{'kind':'COVERAGE','id':'example','revision':'abc'}]},{}
            def validate_page(self,*args):pass
            def dependency(self,identity):return record,{'stdout':'owned-native-fixture.json'}
            def navigation(self,record):return ['source-ref']
        class BoundSource:
            def source(self,*args):return source
        globals_=M['acquire'].__globals__;previous=globals_['Packet'],globals_['BoundSource']
        try:
            globals_['Packet'],globals_['BoundSource']=Packet,BoundSource
            with tempfile.TemporaryDirectory() as name:
                root=Path(name);selection=root/'selection.json';selection.write_text(json.dumps([{'declarationId':record['id']}]))
                args=argparse.Namespace(root='docs-root',service='example',snapshot='snapshot',work='work',selection=selection,output_dir=root)
                measurement=M['acquire'](args)
                actual=(root/'author.txt').read_bytes()
                self.assertEqual(measurement['authorBytes'],len(actual))
                self.assertEqual(measurement['directSourceBytes'],len((root/'direct-source.txt').read_bytes()))
                self.assertGreater(len(actual),len(source['text'].encode()))
                self.assertEqual(json.loads((root/'measurement.json').read_text()),measurement)
        finally:
            globals_['Packet'],globals_['BoundSource']=previous

    def test_missing_semantic_bindings_are_an_explicit_gap(self):
        item=fixture();del item['record']['normalized']['documentation']['semanticInputs']
        text,meta=M['project']([item])
        self.assertIn('SEMANTIC_INPUTS_NOT_RETAINED',text)
        self.assertEqual(meta['callables'][0]['semanticInputsStatus'],'MISSING')
        self.assertNotIn('compiler target ->',text)

    def test_missing_connected_contract_is_not_reconstructed_from_labels(self):
        item=fixture();del item['record']['normalized']['documentation']['sourceStructure']
        with self.assertRaises(M['Stop']):M['project']([item])


if __name__=='__main__':unittest.main()
