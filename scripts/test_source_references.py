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


class SourceBatchTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(); self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name); self.requests = []; self.limit = 20
        self.omit = set(); self.page_size = 20; self.bad = None
        self.sources = {}
        for n in range(1, 10):
            text = '\ufeff// π🙂\r\nif (ready) send();\r\n'
            source = F['source'](text if n != 2 else 'if (ready) send();\r\n')
            source.update(id='source-'+str(n), file='Same.cs', evidenceDigest='sha256:original',
                          authority='DECLARED_OPENAPI' if n == 3 else 'EXACT_SNAPSHOT_TEXT',
                          occurrence={'snapshot':'sealed','blob':'original','startByte':n,'endByte':n+len(source['text'].encode())})
            self.sources['s'+str(n)] = source

    def native(self, command, **kwargs):
        request = json.loads(Path(command[command.index('--input')+1]).read_text())
        self.requests.append((command[3], request))
        if command[3] == 'read-part':
            if self.bad == 'part-error':
                return subprocess.CompletedProcess(command,2,encode({'error':{'message':'source unavailable'}}).encode(),b'')
            ref = request['reference']; source = self.sources[ref]; raw = source['text'].encode()
            start = int(request.get('cursor','p:0').split(':')[1]); end = min(start+self.limit,len(raw))
            while end < len(raw) and raw[end] & 0xc0 == 0x80:end -= 1
            value = F['part'](source,start,end,'p:'+str(end) if end<len(raw) else None,reference=ref)
        else:
            refs = request['references']; self.assertLessEqual(len(refs),8)
            rows = [{'kind':'COVERAGE','id':'owned','revision':'rev-a'}]
            rows += [{'kind':'SOURCE','id':self.sources[r]['id'],'reference':r,
                      'record':copy.deepcopy(self.sources[r])} for r in refs]
            start = int(request.get('cursor','page:0').split(':')[1]); stop = min(start+self.page_size,len(rows))
            items, omitted = [], []
            for row in rows[start:stop]:
                if row.get('reference') in self.omit:
                    omitted.append({k:v for k,v in row.items() if k!='record'} | {'reason':'ITEM_EXCEEDS_WORK_BYTE_BUDGET'})
                else:items.append(row)
            value = {**F['page'](*items,omitted=omitted,cursor='page:'+str(stop) if stop<len(rows) else None),
                     'total':len(rows),'membershipDigest':'members','contextDigest':'context','inputDigest':'input',
                     'authority':'IMMUTABLE_WORK_CAPTURE_NOT_REVERIFIED'}
            source_row = next((r for r in items if r['kind']=='SOURCE'),None)
            if self.bad == 'work':value['work']='foreign'
            if self.bad == 'snapshot':value['snapshot']='foreign'
            if self.bad == 'subject':value['subject']='service:foreign'
            if self.bad == 'total':value['total']+=1
            if self.bad == 'missing':value['items']=[r for r in items if r['kind']=='COVERAGE']
            if self.bad == 'continuation':value['nextCursor']='page:0'
            if self.bad == 'empty-progress':
                value.update(items=[],omitted=[],nextCursor='page:'+str(start+1))
            if self.bad == 'duplicate' and source_row:value['items'].append(copy.deepcopy(source_row))
            if source_row:
                if self.bad == 'reference':source_row['reference']='foreign'
                if self.bad == 'reference-type':source_row['reference']=[]
                if self.bad == 'id':source_row['id']='foreign'
                if self.bad == 'source-id':source_row['record']['id']='foreign'
                if self.bad == 'revision':source_row['record']['revision']='foreign'
                if self.bad == 'text':source_row['record']['text']+='!'
                if self.bad == 'digest':source_row['record']['textDigest']='bad'
                if self.bad == 'partial':source_row['record'].pop('text')
            value['receiptDigest']=digest(value)
            if self.bad == 'receipt':value['receiptDigest']='bad'
        return subprocess.CompletedProcess(command,0,encode(value).encode(),b'')

    def acquire(self, label='read', size=8):
        out=self.root/label;out.mkdir();selection=self.root/(label+'.json')
        selection.write_text(encode([{'sourceId':s['id'],'sourceReference':ref} for ref,s in self.sources.items()]))
        args=argparse.Namespace(action='read-sources',clew='/supported/clew',root='/disposable/docs',service='owned',
            snapshot=F['SNAPSHOT'],work='work-a',contains='',selection=selection,output_dir=out,
            output_format='author',reuse_read=None,source_union=False,source_batch_size=size)
        with patch('subprocess.run',side_effect=self.native):value,code=M['Packet'](args).run()
        return out,value,code

    def test_complete_mixed_batches_preserve_all_overlapping_bindings_and_inspection(self):
        out,value,code=self.acquire();self.assertEqual(code,0)
        self.assertEqual([x[0] for x in self.requests],['read','read'])
        _,packet,_,entries=A['load'](out/'citations.json')
        self.assertEqual(len(entries),9);self.assertEqual(len(packet['outcomes']),9)
        for outcome,source in zip(packet['outcomes'],self.sources.values()):
            self.assertEqual(outcome['source'],{k:v for k,v in source.items() if k!='text'})
            self.assertEqual(packet['texts'][outcome['textKey']],source['text'])
            self.assertEqual(outcome['sourceParts'],[])
            self.assertEqual(outcome['sourceBatch']['kind'],'NATIVE_SOURCE_BATCH_ROW')

    def test_pagination_and_native_size_omission_use_parts_for_only_omitted_member(self):
        self.sources=dict(list(self.sources.items())[:3]);self.page_size=2;self.omit={'s2'}
        out,value,code=self.acquire();self.assertEqual(code,0);A['load'](out/'citations.json')
        part_refs={r['reference'] for verb,r in self.requests if verb=='read-part'}
        self.assertEqual(part_refs,{'s2'})
        packet=json.loads((out/'packet.json').read_text())
        self.assertEqual([o['sourceBatch']['kind'] for o in packet['outcomes']],
                         ['NATIVE_SOURCE_BATCH_ROW','NATIVE_SOURCE_BATCH_OMISSION','NATIVE_SOURCE_BATCH_ROW'])

    def test_batch_and_part_only_control_have_exact_source_and_binding_identity(self):
        self.sources=dict(list(self.sources.items())[:3])
        old,_,code=self.acquire('old',1);self.assertEqual(code,0);old_count=len(self.requests);self.requests=[]
        new,_,code=self.acquire('new');self.assertEqual(code,0);self.assertLess(len(self.requests),old_count)
        before=json.loads((old/'packet.json').read_text());after=json.loads((new/'packet.json').read_text())
        self.assertEqual(before['texts'],after['texts'])
        for a,b in zip(before['outcomes'],after['outcomes']):
            for field in ['source','sourceRecordDigest','selection','textKey']:self.assertEqual(a[field],b[field])
        A['load'](old/'citations.json');A['load'](new/'citations.json')

    def test_failed_omitted_member_keeps_completed_neighbor_without_rereading_it(self):
        self.sources=dict(list(self.sources.items())[:2]);self.omit={'s1'};self.bad='part-error'
        out,_,code=self.acquire();self.assertNotEqual(code,0)
        packet=json.loads((out/'packet.json').read_text())
        self.assertEqual(packet['status'],'SELECTED_SOURCES_INCOMPLETE')
        self.assertEqual([o['status'] for o in packet['outcomes']],['NATIVE_ERROR','RETAINED_SOURCE_RETURNED'])
        self.assertEqual([r['reference'] for verb,r in self.requests if verb=='read-part'],['s1'])
        self.assertEqual(packet['outcomes'][1]['sourceBatch']['kind'],'NATIVE_SOURCE_BATCH_ROW')
        with self.assertRaises(A['Stop']):A['load'](out/'citations.json')

    def test_inconsistent_complete_rows_fail_closed_even_with_rehashed_page(self):
        self.sources=dict(list(self.sources.items())[:2])
        for bad in ['work','snapshot','subject','total','missing','duplicate','reference','reference-type','id',
                    'source-id','revision','text','digest','partial','receipt','continuation','empty-progress']:
            self.bad=bad
            with self.subTest(bad=bad):
                _,value,code=self.acquire(bad);self.assertNotEqual(code,0)
                self.assertFalse(any(verb=='read-part' for verb,_ in self.requests))
                self.requests=[]

    def test_saved_batch_proof_corruption_refuses_after_wrapper_rehash(self):
        self.sources=dict(list(self.sources.items())[:2])
        for bad in ['kind','pointer','record-digest','choices','pages','fake-part','part-command','truncated-pages']:
            self.page_size=1 if bad=='truncated-pages' else 20
            out,_,code=self.acquire(bad);self.assertEqual(code,0)
            packet=json.loads((out/'packet.json').read_text());outcome=packet['outcomes'][0];proof=outcome['sourceBatch']
            if bad=='kind':proof['kind']='SOURCE_PART'
            if bad=='pointer':proof['raw']['pointer']='/items/0'
            if bad=='record-digest':proof['recordDigest']='bad'
            if bad=='choices':proof['choices'][0]['reference']='other'
            if bad=='pages':proof['pages']=[]
            if bad=='truncated-pages':proof['pages'].pop()
            if bad=='fake-part':outcome['sourceParts']=[{'stdout':proof['pages'][0]}]
            C['refresh'](out,packet)
            if bad=='part-command':
                path=out/'evidence-receipts.json';receipts=json.loads(path.read_text())
                next(r for r in receipts if r['stdout']==proof['pages'][0])['command'][3]='read-part'
                path.write_text(encode(receipts))
            with self.subTest(bad=bad),self.assertRaises(A['Stop']):A['load'](out/'citations.json')


if __name__ == '__main__': unittest.main()
