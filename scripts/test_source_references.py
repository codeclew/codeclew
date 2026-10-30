#!/usr/bin/env python3
"""Native SOURCE-reference acquisition and read-only citation proof controls."""
import argparse
import copy
import hashlib
import json
from pathlib import Path
import runpy
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parent.parent
M = runpy.run_path(str(ROOT/'skills/codeclew/scripts/evidence_packet.py'))
A = runpy.run_path(str(ROOT/'skills/codeclew/scripts/author_citations.py'))
F = runpy.run_path(str(ROOT/'scripts/test_work_bound_source.py'))
C = runpy.run_path(str(ROOT/'scripts/test_citation_evidence.py'))
encode, digest = M['encode'], M['digest']


class SourceReferencesTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(); self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name); self.bad = None; self.commands = []; self.requests = []
        self.source = F['source']('{"openapi":"3.1.0","info":{"title":"π🙂","version":"1"},"paths":{}}\n')
        self.source.update(file='contracts/example.openapi.json',startLine=1,endLine=1,authority='DECLARED_OPENAPI',
            occurrence={'snapshot':'sha256:'+'a'*64,'blob':'b'*40,'startByte':0,'endByte':len(self.source['text'].encode())})
        self.paged = False; self.omitted = False; self.limit = 30

    def native(self, command, **kwargs):
        self.commands.append(command)
        self.assertIn(command[1:4], [['docs','work','read'],['docs','work','read-part']])
        request = json.loads(Path(command[command.index('--input')+1]).read_text()); self.requests.append(request)
        if command[3] == 'read-part':
            raw = self.source['text'].encode(); start = int(request.get('cursor','p:0').split(':')[1])
            end = min(start+self.limit,len(raw))
            while end < len(raw) and raw[end]&0xc0 == 0x80: end -= 1
            value = F['part'](self.source,start,end,'p:'+str(end) if end<len(raw) else None)
            if self.bad == 'part-bytes': value['text'] += '!'
            if self.bad == 'part-digest': value['recordDigest'] = 'bad'
            if self.bad == 'part-incomplete': value['nextCursor'] = None
            if self.bad == 'part-reference': value['reference'] = 's-other'
            if self.bad == 'part-id': value['sourceId'] = 'other'
            if self.bad == 'part-revision': value['source']['revision'] = 'changed'
            if self.bad in ['part-id','part-reference','part-revision','part-incomplete']:
                value['receiptDigest'] = digest({k:v for k,v in value.items() if k!='receiptDigest'})
        else:
            self.assertEqual(request['references'],['s9'])
            coverage = {'kind':'COVERAGE','id':'owned','revision':'rev-a'}
            row = {'kind':'SOURCE','id':self.source['id'],'reference':'s9','record':copy.deepcopy(self.source)}
            omission = {'kind':'SOURCE','id':self.source['id'],'reference':'s9','reason':'ITEM_EXCEEDS_WORK_BYTE_BUDGET'}
            cursor = 'next' if self.paged and 'cursor' not in request else None
            items = [coverage] if self.paged and cursor else [row] if self.paged else [coverage,row]
            omissions = []
            if self.omitted:
                items = [i for i in items if i['kind']!='SOURCE']
                if not cursor: omissions = [omission]
            value = {**F['page'](*items,cursor=cursor,omitted=omissions),'total':2,'contextDigest':'ctx',
                'inputDigest':'input','membershipDigest':'members','authority':'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED'}
            if self.bad in ['work','snapshot','subject']: value[self.bad] = 'foreign'
            if self.bad == 'reference': row['reference'] = 's-other'
            if self.bad == 'id': row['id'] = 'other'
            if self.bad == 'service': row['record']['service'] = 'foreign'
            if self.bad == 'revision': row['record']['revision'] = 'foreign'
            if self.bad == 'page-bytes': row['record']['text'] += '!'
            if self.bad == 'page-digest': row['record']['textDigest'] = 'bad'
            if self.bad == 'total': value['total'] = 3
            if self.bad == 'unresolved-omission': value['omitted'] = [{**omission,'id':'other'}]
            if self.bad == 'not-returned': value['items'] = [coverage]; value['total'] = 1
            if self.bad == 'continuation': value['nextCursor'] = 'repeat'
            value['receiptDigest'] = digest(value)
            if self.bad == 'page-receipt': value['receiptDigest'] = 'bad'
        return subprocess.CompletedProcess(command,0,encode(value).encode(),b'')

    def acquire(self, label='read', selection=None, output_format='author'):
        out = self.root/label; out.mkdir(); selected = self.root/(label+'.selection.json')
        selected.write_text(encode(selection if selection is not None else [{'sourceId':self.source['id'],'sourceReference':'s9'}]))
        args = argparse.Namespace(action='read-sources',clew='/supported/clew',root='/disposable/docs',service='owned',
            snapshot=F['SNAPSHOT'],work='work-a',contains='',selection=selected,output_dir=out,output_format=output_format,
            reuse_read=None,source_union=False)
        with patch('subprocess.run',side_effect=self.native): value,code = M['Packet'](args).run()
        return out,value,code

    def test_real_source_kind_preserves_authority_occurrence_and_checked_claim_binding(self):
        out,value,code = self.acquire(); self.assertEqual(code,0)
        self.assertEqual(value['records'][0]['authority'],'DECLARED_OPENAPI')
        self.assertNotIn('wholeFile',value['records'][0]); self.assertFalse(value['failures'])
        manifest,packet,discovery,entries = A['load'](out/'citations.json')
        outcome = packet['outcomes'][0]; self.assertNotIn('declaration',outcome)
        self.assertFalse(discovery['inventoryEnumerated']); self.assertEqual(outcome['source']['occurrence'],self.source['occurrence'])
        self.assertEqual(packet['texts'][self.source['textDigest']],self.source['text'])
        self.assertGreater(len(outcome['sourceParts']),1)
        md = self.root/'guide.md'; md.write_text('A declared contract is retained.\n')
        claims = self.root/'claims.json'; claims.write_text(encode({'claims':[{'id':'contract','text':md.read_text().strip(),'evidence':list(entries)}]}))
        A['compile_map']([out/'citations.json'],md,claims,self.root/'bound')
        bound = json.loads((self.root/'bound/sources.json').read_text())['claims'][0]['sources'][0]
        self.assertEqual(bound['id'],self.source['id']); self.assertEqual(bound['evidenceKind'],'RETAINED_SOURCE')
        request = self.root/'views.json'; request.write_text(encode([{'file':self.source['file']}]))
        grouped = A['inspect_grouped'](out/'citations.json',request)
        self.assertEqual(grouped['groups'][0]['fragments'][0]['text'],self.source['text'])

    def test_native_paging_and_explicit_size_omission_recover_through_parts(self):
        for omitted in [False,True]:
            self.paged = True; self.omitted = omitted
            out,_,code = self.acquire(str(omitted)); self.assertEqual(code,0)
            A['load'](out/'citations.json')
            self.assertEqual(len(json.loads((out/'packet.json').read_text())['outcomes'][0]['sourceBinding']['pages']),2)

    def test_metadata_projection_retains_same_citations_without_body_delivery(self):
        out,value,code = self.acquire(output_format='metadata'); self.assertEqual(code,0)
        self.assertFalse(value['sourceBodiesDisplayed']); self.assertEqual(value['texts'],{})
        A['load'](out/'citations.json')

    def test_wrong_native_reference_scope_bytes_and_incomplete_delivery_fail_closed(self):
        for bad in ['work','snapshot','subject','reference','id','service','revision','page-bytes','page-digest',
                    'page-receipt','total','unresolved-omission','not-returned','continuation','part-id','part-reference',
                    'part-revision','part-bytes','part-digest','part-incomplete']:
            with self.subTest(bad=bad):
                self.bad = bad; out,value,code = self.acquire(bad)
                self.assertNotEqual(code,0); self.assertTrue(value['failures'])
                with self.assertRaises(A['Stop']): A['load'](out/'citations.json')

    def test_explicit_unique_id_reference_required(self):
        valid = {'sourceId':self.source['id'],'sourceReference':'s9'}
        for index,selection in enumerate([[{}],[{'sourceReference':'s9'}],[{**valid,'file':'invented'}],[valid,valid]]):
            out,value,code = self.acquire('invalid'+str(index),selection)
            self.assertNotEqual(code,0); self.assertTrue(value['failures'])

    def test_changed_selected_source_id_is_not_inferred_from_returned_reference(self):
        out,value,code = self.acquire(selection=[{'sourceId':'other','sourceReference':'s9'}])
        self.assertNotEqual(code,0); self.assertTrue(value['failures'])
        self.assertEqual(len(self.commands),1)

    def test_raw_context_packet_import_is_not_an_acquisition_option(self):
        with self.assertRaises(SystemExit) as error, patch('subprocess.run',side_effect=AssertionError('no native read')):
            M['main'](['read-sources','--clew','clew','--root','root','--service','owned','--snapshot',F['SNAPSHOT'],
                '--work','work-a','--selection','selection.json','--packet','context.json','--output-dir',str(self.root/'forbidden')])
        self.assertEqual(error.exception.code,2); self.assertFalse((self.root/'forbidden').exists())

    def test_rehashed_saved_scope_reference_bytes_and_omissions_cannot_rebind(self):
        for bad in ['work','snapshot','subject','reference','source-id','record','omission','parts','selection']:
            with self.subTest(bad=bad):
                out,_,code = self.acquire('saved-'+bad); self.assertEqual(code,0)
                packet = json.loads((out/'packet.json').read_text()); outcome = packet['outcomes'][0]
                path = out/outcome['sourceBinding']['raw']['stdout']; page = json.loads(path.read_text())
                row = page['items'][1]
                if bad in ['work','snapshot','subject']: page[bad] = 'foreign'
                elif bad == 'reference': row['reference'] = 's-other'
                elif bad == 'source-id': row['id'] = 'other'
                elif bad == 'record': row['record']['text'] += '!'
                elif bad == 'omission': page['omitted'] = [{'kind':'SOURCE','id':'foreign'}]
                elif bad == 'parts': outcome['sourceParts'] = outcome['sourceParts'][:-1]
                elif bad == 'selection': outcome['selection']['sourceReference'] = 's-other'
                page['receiptDigest'] = digest({k:v for k,v in page.items() if k!='receiptDigest'})
                path.write_text(encode(page)); C['refresh'](out,packet)
                with patch('subprocess.run',side_effect=AssertionError('proof validation cannot acquire')):
                    with self.assertRaises(A['Stop']): A['load'](out/'citations.json')


if __name__ == '__main__': unittest.main()
